use chrono::{Local, Timelike};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::data::db::Database;
use crate::data::models::CheckinResult;
use crate::infra::notification::Notifier;
use crate::services::checkin::{
    load_checkin_settings, CheckinService, CheckinSource,
};
use crate::services::credit_schedule;
use crate::services::token::TokenManager;

/// 统一续期巡检间隔：启动时先跑一轮，之后每 6 小时一轮
const RENEW_SWEEP_SECS: i64 = 6 * 3600;

pub struct Scheduler {
    db: Arc<Database>,
    checkin: Arc<CheckinService>,
    token: Arc<TokenManager>,
    notifier: Arc<Notifier>,
    stop: Arc<Mutex<bool>>,
}

impl Scheduler {
    pub fn new(
        db: Arc<Database>,
        checkin: Arc<CheckinService>,
        token: Arc<TokenManager>,
        notifier: Arc<Notifier>,
    ) -> Self {
        Scheduler {
            db,
            checkin,
            token,
            notifier,
            stop: Arc::new(Mutex::new(false)),
        }
    }

    pub async fn stop(&self) {
        let mut s = self.stop.lock().await;
        *s = true;
    }

    pub fn start(self: Arc<Self>) {
        tauri::async_runtime::spawn(async move {
            self.run().await;
        });
    }

    async fn run(&self) {
        // 0 表示启动后立刻巡检一轮续期
        let mut last_renew_at = 0i64;
        // 启动补签只判定一次（跳过原因会落一条日志）
        let mut startup_checked = false;
        // 补签巡检去重：同一分钟只跑一轮，避免 5 秒窗口内重复进入
        let mut last_retry_minute: Option<i64> = None;
        // 定时积分查询：启动后的「首次是否执行」只判定并解释一次
        let mut credit_boot_noted = false;
        // 定时积分查询：上次尝试时刻的内存副本，数据库写入失败时仍能阻止重复整批
        let mut credit_last_attempt = 0i64;
        // 定时积分查询：上一次已记录的错误，仅在错误内容变化时重复写日志
        let mut credit_err: Option<String> = None;
        loop {
            {
                let stop = self.stop.lock().await;
                if *stop {
                    break;
                }
            }
            // 签到时间由用户在本地时区设置，必须用本地时间比较；续期用时间戳不受影响
            let now = Local::now();
            // 统一续期：临近到期（24h 内，含已过期）的账号自动续期，
            // 与签到开关无关，保证签到侧「失效即跳过」不会让账号永久失活
            if now.timestamp() - last_renew_at >= RENEW_SWEEP_SECS {
                last_renew_at = now.timestamp();
                self.renew_expiring().await;
            }
            let settings = load_checkin_settings(&self.db);
            // 平台范围：空 = 全部平台，否则仅签到勾选的平台
            let scope: Option<Vec<String>> = if settings.platforms.is_empty() {
                None
            } else {
                Some(settings.platforms.clone())
            };
            let now_min = now.hour() as i64 * 60 + now.minute() as i64;
            let target_min = settings.hour as i64 * 60 + settings.minute as i64;
            let today = now.format("%Y-%m-%d").to_string();
            let last_batch = self
                .db
                .get_setting("last_batch_checkin_date")
                .unwrap_or(None)
                .unwrap_or_default();
            let in_scheduled_window = now_min == target_min && now.second() < 5;

            // 启动补签：仅在「已过当日配置签到时间」且当日定时批量未完成时补一轮，
            // 绝不因为应用启动就无条件发起签到；被跳过时留一条可诊断的日志
            if !startup_checked {
                startup_checked = true;
                if !settings.enabled {
                    self.db.log("info", "签到", "定时签到未开启，跳过启动补签");
                } else if now_min < target_min {
                    self.db.log(
                        "info",
                        "签到",
                        &format!(
                            "当前 {:02}:{:02} 未到定时签到时间 {:02}:{:02}，跳过启动补签",
                            now.hour(),
                            now.minute(),
                            settings.hour,
                            settings.minute
                        ),
                    );
                } else if in_scheduled_window {
                    // 恰好处在定时窗口内，交给本轮定时任务执行，避免重复
                } else if last_batch == today {
                    self.db.log("info", "签到", "今日定时签到已完成，跳过启动补签");
                } else {
                    match self
                        .checkin
                        .retry_missed(scope.as_deref(), CheckinSource::Startup)
                        .await
                    {
                        Ok(results) => {
                            self.report("启动补签完成", &results).await;
                            if !results.is_empty() {
                                self.notifier.emit_checkin_done();
                            }
                            if Self::is_batch_completed(&results) {
                                let _ = self.db.set_setting("last_batch_checkin_date", &today);
                            } else {
                                self.db.log(
                                    "error",
                                    "签到",
                                    "启动补签没有任何账号成功，不标记今日已完成，等待补签巡检重试",
                                );
                            }
                        }
                        Err(e) => {
                            self.db
                                .log("error", "签到", &format!("启动补签未执行：{}", e));
                        }
                    }
                }
            }

            if settings.enabled {
                // 定时签到：本地时间分钟匹配，单日只执行一次
                if in_scheduled_window && last_batch != today {
                    match self
                        .checkin
                        .checkin_all_from(scope.as_deref(), CheckinSource::Scheduled)
                        .await
                    {
                        Ok(results) => {
                            self.report("签到完成", &results).await;
                            self.notifier.emit_checkin_done();
                            if Self::is_batch_completed(&results) {
                                let _ = self.db.set_setting("last_batch_checkin_date", &today);
                            } else {
                                // 账号级失败被折叠为 success=false 的普通结果，这里必须自己判断：
                                // 全部失败时不能写「今日已完成」，否则重启后的启动补签会被错误跳过
                                self.db.log(
                                    "error",
                                    "签到",
                                    "定时签到没有任何账号成功，不标记今日已完成，等待补签巡检重试",
                                );
                            }
                        }
                        Err(e) => {
                            // 锁冲突等失败不写当天标记，避免被误判为「今日已完成」
                            self.db
                                .log("error", "签到", &format!("定时签到未执行：{}", e));
                        }
                    }
                }
                // 补签巡检：只允许在当日配置时间之后，且同一分钟只跑一轮
                let after = Local::now();
                let after_min = after.hour() as i64 * 60 + after.minute() as i64;
                let after_minute_index = after.timestamp() / 60;
                if after_min >= target_min
                    && after.minute() % 10 == 0
                    && after.second() < 5
                    && last_retry_minute != Some(after_minute_index)
                {
                    last_retry_minute = Some(after_minute_index);
                    match self
                        .checkin
                        .retry_missed(scope.as_deref(), CheckinSource::CatchUp)
                        .await
                    {
                        Ok(results) => {
                            if !results.is_empty() {
                                self.notifier.emit_checkin_done();
                            }
                        }
                        Err(e) => {
                            // 任务锁被占用等情况不能静默丢：记日志并说明下一轮会重试
                            self.db.log(
                                "info",
                                "签到",
                                &format!("补签巡检未执行（将在下个巡检点重试）：{}", e),
                            );
                        }
                    }
                }
            }
            // 定时积分查询：与签到互不影响，独立按间隔触发
            self.maybe_run_credit_schedule(
                &mut credit_boot_noted,
                &mut credit_last_attempt,
                &mut credit_err,
            )
            .await;
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    }

    /// 定时积分查询：以「上次执行时间」为基准，间隔到期才触发。
    /// 只在本循环内做触发与状态标记，整批查询交给独立任务执行；
    /// 积分查询不再占用签到批次互斥锁，签到到点可直接执行，不会被积分查询阻塞。
    async fn maybe_run_credit_schedule(
        &self,
        boot_noted: &mut bool,
        last_attempt_mem: &mut i64,
        last_err: &mut Option<String>,
    ) {
        let cfg = match credit_schedule::load_settings(&self.db) {
            Ok(cfg) => {
                if last_err.take().is_some() {
                    self.db.log("info", "积分定时查询", "配置/记录读取已恢复，继续按配置执行");
                }
                cfg
            }
            Err(e) => {
                // 读失败降级为「本轮不执行」，但不能静默：只在错误内容变化时写一次，避免每秒刷屏
                if last_err.as_deref() != Some(e.as_str()) {
                    eprintln!("定时积分查询配置读取失败，降级为不执行：{}", e);
                    self.db.log(
                        "error",
                        "积分定时查询",
                        &format!("配置读取失败，降级为不执行本轮查询：{}", e),
                    );
                    *last_err = Some(e);
                }
                return;
            }
        };
        if !cfg.enabled {
            return;
        }
        let now_local = Local::now();
        let now = now_local.timestamp();
        // 与定时签到错开：签到配置的执行分钟内不发起积分查询，
        // 前后 2 分钟内也不启动新批次，避免长时间批次占着任务锁、挤掉签到窗口
        let checkin_cfg = load_checkin_settings(&self.db);
        let now_min = now_local.hour() as i64 * 60 + now_local.minute() as i64;
        let checkin_min = checkin_cfg.hour as i64 * 60 + checkin_cfg.minute as i64;
        if checkin_cfg.enabled {
            let diff = (now_min - checkin_min).rem_euclid(1440);
            if diff.min(1440 - diff) <= 2 {
                return;
            }
        }
        let interval = cfg.interval_minutes.max(1) as i64 * 60;
        let db_last = match credit_schedule::last_attempt_at(&self.db) {
            Ok(v) => v,
            Err(e) => {
                // 读失败时用内存值兜底，绝不把故障当成「从未执行」而放宽判定
                if last_err.as_deref() != Some(e.as_str()) {
                    eprintln!("定时积分查询上次尝试时刻读取失败，使用内存值兜底：{}", e);
                    self.db.log(
                        "error",
                        "积分定时查询",
                        &format!("上次尝试时刻读取失败，按内存值判定间隔：{}", e),
                    );
                    *last_err = Some(e);
                }
                *last_attempt_mem
            }
        };
        // 数据库值可能与内存值不同：取较大者，保证写失败时本进程内不会重复整批
        let last = db_last.max(*last_attempt_mem);
        let due = last == 0 || now - last >= interval;
        // 启动后的首次判定只做一次并解释原因，避免用户疑惑「刚启动就查积分」
        if !*boot_noted {
            *boot_noted = true;
            if due {
                self.db.log(
                    "info",
                    "积分定时查询",
                    if last == 0 {
                        "尚无执行记录，启动后先执行一次"
                    } else {
                        "距上次查询已超过设定间隔，启动后补查一次"
                    },
                );
            }
        }
        // 从未执行过视为到期；已有记录时按间隔判定
        if !due {
            return;
        }
        // 不占用签到批次互斥锁：积分查询与签到可以并行处理不同账号，
        // 同一账号由账号级互斥保护；签到批量一旦开始，批次侧会主动让位
        // 先记「尝试时刻」再查询：即使本轮失败/异常也不会在下一秒反复重试；
        // 写库失败时用内存值兜底，避免写失败导致每轮都重新发起整批请求
        *last_attempt_mem = now;
        if let Err(e) = credit_schedule::mark_attempt(&self.db, now) {
            eprintln!("定时积分查询上次尝试时刻写入失败，已用内存值兜底：{}", e);
            self.db.log(
                "error",
                "积分定时查询",
                &format!("上次尝试时刻写入失败，已用内存值兜底避免重复查询：{}", e),
            );
        }
        // 整批查询放到独立任务执行：本循环只负责触发与状态标记，不会被耗时批次阻塞
        let db = self.db.clone();
        let scope = cfg.platforms.clone();
        let batch_lock = self.checkin.batch_lock();
        tauri::async_runtime::spawn(async move {
            credit_schedule::run_batch(&db, &scope, now, batch_lock).await;
        });
    }

    /// 批量结果是否构成「当天已完成」：确实有账号签到成功，或全部账号都返回已签到/跳过。
    fn is_batch_completed(results: &[CheckinResult]) -> bool {
        let success = results.iter().filter(|r| r.success && !r.skipped).count();
        success > 0 || results.iter().all(|r| r.skipped)
    }

    /// 批量/补签结果通知（没有账号参与时不打扰用户）
    async fn report(&self, title: &str, results: &[CheckinResult]) {
        if results.is_empty() {
            return;
        }
        let skipped = results.iter().filter(|r| r.skipped).count();
        let success = results.iter().filter(|r| r.success && !r.skipped).count();
        let failed = results.len() - success - skipped;
        self.notifier
            .notify(
                title,
                &format!(
                    "成功 {} 个，跳过 {} 个，失败 {} 个",
                    success, skipped, failed
                ),
            )
            .await;
    }

    /// 统一续期巡检：对所有账号尝试 auto_renew_if_needed
    /// （24h 内到期或已过期才真正续期，其余直接跳过；单账号另有 5 分钟频率限制）
    async fn renew_expiring(&self) {
        let accounts = self.db.list_accounts().unwrap_or_default();
        let mut renewed = 0;
        for acc in accounts {
            match self.token.auto_renew_if_needed(&acc.id).await {
                Ok(Some(msg)) => {
                    renewed += 1;
                    self.db.log(
                        "success",
                        "续期",
                        &format!("[{}] {}：{}", acc.platform, acc.name, msg),
                    );
                }
                Ok(None) => {}
                Err(e) => {
                    self.db.log(
                        "error",
                        "续期",
                        &format!("[{}] {}：{}", acc.platform, acc.name, e),
                    );
                }
            }
        }
        if renewed > 0 {
            self.notifier
                .notify("自动续期", &format!("{} 个账号已自动续期", renewed))
                .await;
        }
    }
}
