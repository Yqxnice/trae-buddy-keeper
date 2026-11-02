use chrono::{Timelike, Utc};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::data::db::Database;
use crate::infra::notification::Notifier;
use crate::services::checkin::{load_checkin_settings, CheckinService};
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
        // 0 表示启动后立刻巡检一轮
        let mut last_renew_at = 0i64;
        loop {
            {
                let stop = self.stop.lock().await;
                if *stop {
                    break;
                }
            }
            let now = Utc::now();
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
            if settings.enabled {
                let now_min = now.hour() * 60 + now.minute();
                let target_min = settings.hour * 60 + settings.minute;
                // 到达设定时间（分钟匹配）
                if now_min == target_min && now.second() < 5 {
                    // 检查今天是否已批量签到过
                    let last_batch = self
                        .db
                        .get_setting("last_batch_checkin_date")
                        .unwrap_or(None)
                        .unwrap_or_default();
                    let today = format!("{}", now.format("%Y-%m-%d"));
                    if last_batch != today {
                        let results = self
                            .checkin
                            .checkin_all(scope.as_deref())
                            .await
                            .unwrap_or_default();
                        self.notifier.emit_checkin_done();
                        let skipped = results.iter().filter(|r| r.skipped).count();
                        let success = results.iter().filter(|r| r.success && !r.skipped).count();
                        let failed = results.len() - success - skipped;
                        self.notifier
                            .notify(
                                "签到完成",
                                &format!(
                                    "成功 {} 个，跳过 {} 个，失败 {} 个",
                                    success, skipped, failed
                                ),
                            )
                            .await;
                        let _ = self.db.set_setting("last_batch_checkin_date", &today);
                    }
                }
                // 超时补签：每 10 分钟检查一次未签到的（同样受平台范围约束）
                if now.minute() % 10 == 0 && now.second() < 5 {
                    match self.checkin.retry_missed(scope.as_deref()).await {
                        Ok(results) if !results.is_empty() => self.notifier.emit_checkin_done(),
                        _ => {}
                    }
                }
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
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
