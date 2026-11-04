use chrono::{Datelike, Local, Utc};
use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::data::db::Database;
use crate::data::models::*;
use crate::platform::trae::api as trae_api;
use crate::platform::workbuddy::api as workbuddy_api;
use crate::services::account_lock;

/// 9074/9095 语义：9074=该设备号被上游限流/拉黑，9095=该设备今日已签到
const TRAE_MASTER_RISK_CODE: &str = "code=9074";
/// 9074 时换设备号的总尝试次数（含第一次原号）
const TRAE_9074_MAX_ATTEMPTS: usize = 3;

/// 签到批次锁异常占用上限（秒）：超过此值仍被占用，视为持有者已异常
/// （panic 未走到 Drop、任务被丢弃、进程挂起等），允许带 warning 强制回收一次。
const BATCH_STALE_SECS: i64 = 7200;
/// 并发批次排队等待上限（秒）：撞车批次先等正在运行的批次结束，超时后带
/// warning 强制接管继续执行，宁可短暂重叠也不让账号当天漏签。
const BATCH_WAIT_MAX_SECS: u64 = 180;
/// 排队等待的轮询间隔（毫秒）
const BATCH_WAIT_POLL_MS: u64 = 2000;

/// 设备号形态：12~20 位纯数字（客户端注册过的 Aha 设备号，15/16 位常见但非规则）
fn is_aha_device(s: &str) -> bool {
    matches!(s.len(), 12..=20) && s.bytes().all(|b| b.is_ascii_digit())
}

/// 签到触发来源：仅用于日志诊断，区分自动与人工触发
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckinSource {
    /// 界面按钮（单账号 / 批量）
    Manual,
    /// 托盘菜单「一键签到」
    Tray,
    /// 定时任务到点批量
    Scheduled,
    /// 应用启动后的一次性补签
    Startup,
    /// 定时补签巡检
    CatchUp,
}

impl CheckinSource {
    /// IPC 传入的来源字符串 → 枚举（未知值回退手动）
    pub fn from_opt(source: Option<&str>) -> Self {
        match source.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            Some("tray") => Self::Tray,
            Some("scheduled") => Self::Scheduled,
            Some("startup") => Self::Startup,
            Some("catchup") => Self::CatchUp,
            _ => Self::Manual,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Manual => "手动",
            Self::Tray => "托盘",
            Self::Scheduled => "定时任务",
            Self::Startup => "启动补签",
            Self::CatchUp => "补签",
        }
    }
}

/// 批次锁持有凭证：只有持有者本人能释放，被强制回收后旧凭证自动失效
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BatchToken(u64);

struct BatchLease {
    token: u64,
    started_at: i64,
}

/// 签到批次互斥锁（只保护极短的临界区，因此用同步 Mutex，不是 async 锁）。
///
/// 释放按 token 幂等：只有当前持有者的 token 能清空占用；一旦被强制回收，
/// 旧持有者的 Drop 因 token 不匹配变成 no-op，不会误释放新持有者。
pub struct CheckinBatchLock {
    lease: Mutex<Option<BatchLease>>,
    next_token: AtomicU64,
}

impl CheckinBatchLock {
    fn new() -> Self {
        Self {
            lease: Mutex::new(None),
            next_token: AtomicU64::new(1),
        }
    }

    fn lock_lease(&self) -> std::sync::MutexGuard<'_, Option<BatchLease>> {
        // 锁内没有会在 panic 时留下不一致状态的操作，毒锁按正常值恢复
        self.lease.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// 立即尝试获取；已有持有者时返回 None（不等待）
    fn try_acquire(&self) -> Option<BatchToken> {
        let mut lease = self.lock_lease();
        if lease.is_some() {
            return None;
        }
        let token = BatchToken(self.next_token.fetch_add(1, Ordering::SeqCst));
        *lease = Some(BatchLease {
            token: token.0,
            started_at: Utc::now().timestamp(),
        });
        Some(token)
    }

    /// 强制回收当前占用（自愈）；返回被回收的占用时长（秒）
    fn force_reclaim(&self) -> Option<i64> {
        let mut lease = self.lock_lease();
        lease
            .take()
            .map(|l| Utc::now().timestamp().saturating_sub(l.started_at))
    }

    /// 释放：token 不匹配说明已被强制回收（或已释放过），直接忽略
    fn release(&self, token: BatchToken) {
        let mut lease = self.lock_lease();
        if lease.as_ref().map(|l| l.token) == Some(token.0) {
            *lease = None;
        }
    }

    /// 当前占用已持续秒数；未占用返回 None
    pub fn held_for(&self) -> Option<i64> {
        let lease = self.lock_lease();
        lease
            .as_ref()
            .map(|l| Utc::now().timestamp().saturating_sub(l.started_at))
    }

    /// 是否有签到批次正在执行（供积分定时查询让位）
    pub fn is_active(&self) -> bool {
        self.lock_lease().is_some()
    }
}

/// 批次锁 RAII 守卫：Drop 时自动释放，panic 展开路径也能回收
pub struct CheckinBatchGuard {
    lock: Arc<CheckinBatchLock>,
    token: BatchToken,
}

impl Drop for CheckinBatchGuard {
    fn drop(&mut self) {
        self.lock.release(self.token);
    }
}

/// 平台范围匹配：勾选 workbuddy 时同时覆盖 codebuddy_cn（同一账号体系）
pub(crate) fn platform_in_scope(platform: &str, scope: &[String]) -> bool {
    scope
        .iter()
        .any(|p| p == platform || (p == "workbuddy" && platform == "codebuddy_cn"))
}

/// 由种子派生 16 位设备号：同一种子恒定；换号时换盐即可得到新号
fn derived_device_id(seed: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(seed.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{:02x}", b)).collect();
    let n = u128::from_str_radix(&hex[..30], 16).unwrap_or(0);
    format!(
        "{}",
        1_000_000_000_000_000u128 + n % 9_000_000_000_000_000u128
    )
}

pub struct CheckinService {
    db: Arc<Database>,
    batch_lock: Arc<CheckinBatchLock>,
    /// 应用数据目录（读账号 vault 里客户端真实设备号用）
    data_dir: std::path::PathBuf,
}

impl CheckinService {
    pub fn new(db: Arc<Database>, data_dir: std::path::PathBuf) -> Self {
        CheckinService {
            db,
            batch_lock: Arc::new(CheckinBatchLock::new()),
            data_dir,
        }
    }

    /// 批次锁句柄：供定时积分查询判断「签到是否正在进行」并让位
    pub fn batch_lock(&self) -> Arc<CheckinBatchLock> {
        self.batch_lock.clone()
    }

    // 单账户签到（手动入口，来源记为手动）
    pub async fn checkin_one(&self, account_id: &str) -> Result<CheckinResult, String> {
        self.checkin_one_from(account_id, CheckinSource::Manual)
            .await
    }

    // 单账户签到（含冷却与隔离判断）；source 仅用于日志区分触发来源
    pub async fn checkin_one_from(
        &self,
        account_id: &str,
        source: CheckinSource,
    ) -> Result<CheckinResult, String> {
        // 账号级互斥：与积分查询共用，保证同一账号同一时刻只有一个上游请求。
        // 签到优先——即使积分查询正在请求该账号，也只需等它这一条请求结束。
        // 守卫持有到函数结束（包含落库与签到后积分刷新），保证状态对其他批次可见
        let account_guard = account_lock::acquire_account(account_id).await;
        let acc = self
            .db
            .get_account(account_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| "账户不存在".to_string())?;

        let now = Utc::now().timestamp();
        // 0) WorkBuddy 国际版没有签到接口：入口直接跳过，不发请求、不落签到记录，
        //    也不计入「今天是否全部已签到」的待签集合
        if acc.platform == "workbuddy"
            && crate::platform::workbuddy::variant::variant_of(&acc)
                == crate::platform::workbuddy::variant::WbVariant::Ai
        {
            return Ok(CheckinResult {
                account_id: acc.id.clone(),
                account_name: acc.name.clone(),
                platform: acc.platform.clone(),
                success: false,
                skipped: true,
                points: 0,
                message: "WorkBuddy 国际版无签到，已跳过".to_string(),
            });
        }
        // 统一自动跳过：所有入口（手动单个/批量勾选/定时/补签）都在这里收敛，
        // 命中后不再请求上游，也不会记失败
        // 1) 凭证已失效
        if is_expired(&acc, now) {
            return Ok(CheckinResult {
                account_id: acc.id.clone(),
                account_name: acc.name.clone(),
                platform: acc.platform.clone(),
                success: false,
                skipped: true,
                points: 0,
                message: "凭证已失效，已自动跳过".to_string(),
            });
        }
        // 2) 冷却中
        if acc.cooldown_until > now {
            return Ok(CheckinResult {
                account_id: acc.id.clone(),
                account_name: acc.name.clone(),
                platform: acc.platform.clone(),
                success: false,
                skipped: false,
                points: 0,
                message: format!("冷却中，剩余 {} 秒", acc.cooldown_until - now),
            });
        }
        // 3) 今日已签到（本地自然日边界）
        if checked_in_today(acc.last_checkin_time) {
            let _ = self.db.update_account_checkin(
                &acc.id,
                CheckinStatus::Success.as_str(),
                acc.last_checkin_time,
                0,
                0,
            );
            return Ok(CheckinResult {
                account_id: acc.id.clone(),
                account_name: acc.name.clone(),
                platform: acc.platform.clone(),
                success: true,
                skipped: true,
                points: 0,
                message: "今日已签到，已自动跳过".to_string(),
            });
        }

        let (success, points, message) = match acc.platform.as_str() {
            "trae" => self.trae_checkin_with_reauth(&acc).await,
            "workbuddy" => workbuddy_api::do_checkin(&acc)
                .await
                .unwrap_or_else(|e| (false, 0, e.to_string())),
            _ => (false, 0, "未知平台".to_string()),
        };

        let now_ts = Utc::now().timestamp();
        let rec = CheckinRecord {
            id: Uuid::new_v4().to_string(),
            account_id: acc.id.clone(),
            platform: acc.platform.clone(),
            status: if success { "success" } else { "failed" }.to_string(),
            points,
            message: message.clone(),
            created_at: now_ts,
        };
        let _ = self.db.insert_checkin_record(&rec);
        self.db.log(
            if success { "success" } else { "error" },
            "签到",
            &format!(
                "[{}] {}：{}（{}）",
                acc.platform,
                acc.name,
                message,
                source.label()
            ),
        );

        if success {
            let _ = self.db.update_account_checkin(
                &acc.id,
                CheckinStatus::Success.as_str(),
                now_ts,
                0,
                0,
            );
            // 写入积分历史（仅作获得流水/趋势；WorkBuddy 真实余额与到期以
            // billing 接口为准，这里不再假设 31 天有效期）
            if points > 0 {
                let today = today_str();
                let expiry = if acc.platform == "workbuddy" {
                    0 // 未知，真实到期见 accounts.credits_expiries
                } else {
                    now_ts + 31 * 86400 // TRAE 签到积分 31 天有效期
                };
                let ph = PointsHistory {
                    id: Uuid::new_v4().to_string(),
                    account_id: acc.id.clone(),
                    date: today,
                    points,
                    source: "checkin".to_string(),
                    expires_at: expiry,
                    created_at: now_ts,
                };
                let _ = self.db.insert_points_history(&ph);
            }
            // 签到后刷新真实积分（失败不影响签到结果）：复用已持有的账号锁，
            // 否则会被自身的 try 锁挡掉，每次签到都写一条「已跳过积分查询」
            let _ = crate::platform::workbuddy::credits::refresh_account_with_action_guarded(
                &self.db,
                &acc.id,
                "积分查询",
                account_guard,
            )
            .await;
        } else {
            let mut new_fail = acc.fail_count + 1;
            let settings = load_checkin_settings(&self.db);
            // 9074 是服务端按时间窗口排队限流：当场连发只会延长惩罚窗口，
            // 直接按「已达上限」进冷却，把重试交给下一轮定时任务
            if acc.platform == "trae" && message.contains(TRAE_MASTER_RISK_CODE) {
                new_fail = new_fail.max(settings.max_fail_count as i32);
            }
            let cooldown_until = if new_fail >= settings.max_fail_count as i32 {
                now_ts + settings.cooldown_minutes as i64 * 60
            } else {
                0
            };
            let status = if cooldown_until > 0 {
                CheckinStatus::Cooling.as_str()
            } else {
                CheckinStatus::Failed.as_str()
            };
            let _ = self.db.update_account_checkin(
                &acc.id,
                status,
                acc.last_checkin_time,
                new_fail,
                cooldown_until,
            );
        }

        Ok(CheckinResult {
            account_id: acc.id.clone(),
            account_name: acc.name.clone(),
            platform: acc.platform.clone(),
            success,
            skipped: false,
            points,
            message,
        })
    }

    /// 授权登录后回查服务端真实签到状态：用户可能刚在官方客户端里签过到，
    /// 本地无记录会误显示「未签到」。查询到已签时直接把账号落为今日已签；
    /// 查询失败静默忽略（不打断登录流程，也不写失败日志），后续签到尝试会自愈。
    pub async fn sync_checkin_status(&self, acc: &Account) {
        let now = Utc::now().timestamp();
        // 国际版无签到体系；凭证失效的账号查了也查不动
        if acc.platform == "workbuddy"
            && crate::platform::workbuddy::variant::variant_of(acc)
                == crate::platform::workbuddy::variant::WbVariant::Ai
        {
            return;
        }
        if is_expired(acc, now) {
            return;
        }
        let signed = match acc.platform.as_str() {
            "trae" => {
                let device_id = self.trae_device_id(acc);
                let client = reqwest::Client::new();
                trae_api::checkin_status(&client, &acc.access_token, &device_id)
                    .await
                    .map(|(claimed, _enabled, _)| claimed)
                    .unwrap_or(false)
            }
            "workbuddy" => workbuddy_api::checkin_status(acc).await.unwrap_or(false),
            _ => return,
        };
        if signed {
            let _ = self.db.update_account_checkin(
                &acc.id,
                CheckinStatus::Success.as_str(),
                now,
                0,
                0,
            );
        }
    }

    /// 批量签到（手动入口，来源记为手动）
    pub async fn checkin_all(
        &self,
        platforms: Option<&[String]>,
    ) -> Result<Vec<CheckinResult>, String> {
        self.checkin_all_from(platforms, CheckinSource::Manual).await
    }

    /// 批量签到；platforms 为 None 时签到全部平台，否则仅签到范围内的平台
    pub async fn checkin_all_from(
        &self,
        platforms: Option<&[String]>,
        source: CheckinSource,
    ) -> Result<Vec<CheckinResult>, String> {
        // RAII：正常返回或 panic 展开都会释放批次锁
        let _guard = self.acquire_batch_guard(source).await;
        self.do_checkin_all(platforms, source).await
    }

    async fn do_checkin_all(
        &self,
        platforms: Option<&[String]>,
        source: CheckinSource,
    ) -> Result<Vec<CheckinResult>, String> {
        let mut accounts = self.db.list_accounts().map_err(|e| e.to_string())?;
        // 国际版无签到接口，不进批量签到队列
        accounts.retain(|a| {
            a.platform != "workbuddy"
                || crate::platform::workbuddy::variant::variant_of(a)
                    != crate::platform::workbuddy::variant::WbVariant::Ai
        });
        if let Some(ps) = platforms {
            accounts.retain(|a| platform_in_scope(&a.platform, ps));
        }
        let settings = load_checkin_settings(&self.db);
        let mut results = Vec::new();

        for acc in &accounts {
            let r = self
                .checkin_one_from(&acc.id, source)
                .await
                .unwrap_or_else(|e| CheckinResult {
                    account_id: acc.id.clone(),
                    account_name: acc.name.clone(),
                    platform: acc.platform.clone(),
                    success: false,
                    skipped: false,
                    points: 0,
                    message: e,
                });
            let skipped = r.skipped;
            results.push(r);
            // 自动跳过的账号没有发请求，不必再等间隔
            if skipped {
                continue;
            }
            // 签到间隔：TRAE 增长页对同 IP 连发很敏感，账号之间拉开 12~25s；
            // 其它平台仍用设置里的间隔
            let gap = if acc.platform == "trae" {
                use rand::Rng;
                12 + rand::thread_rng().gen_range(0..14)
            } else {
                settings.interval_seconds as u64
            };
            tokio::time::sleep(std::time::Duration::from_secs(gap)).await;
        }

        Ok(results)
    }

    /// 抢占签到批次（有限排队 + 超时接管 + 异常自愈）：
    /// - 无占用：立即获取，返回 RAII 守卫（Drop 释放，panic 展开也能回收）；
    /// - 有占用且未超过异常上限：每 2 秒轮询，最多等待 BATCH_WAIT_MAX_SECS，
    ///   让撞车批次先等正在运行的批次结束，避免并发重复请求；
    /// - 占用超过 BATCH_STALE_SECS（异常路径未释放/任务丢失）：warning 后强制回收一次；
    /// - 等待超过上限：warning 后强制接管继续执行，宁可短暂重叠也不让账号当天漏签。
    ///   重叠期间由账号级互斥 + 每次签到前重新读库（checked_in_today）保证不会
    ///   并发请求同一账号，已签过的账号会被自动跳过。
    ///
    /// 永不因「已有批次在跑」直接失败：这样定时/启动补签/巡检/手动都不会被跳过。
    async fn acquire_batch_guard(&self, source: CheckinSource) -> CheckinBatchGuard {
        let started = std::time::Instant::now();
        let wait_limit = std::time::Duration::from_secs(BATCH_WAIT_MAX_SECS);
        let mut wait_noted = false;
        loop {
            if let Some(token) = self.batch_lock.try_acquire() {
                return CheckinBatchGuard {
                    lock: self.batch_lock.clone(),
                    token,
                };
            }
            // 自愈：占用时间明显超过一次批量的合理上限，说明持有者异常
            if let Some(held) = self.batch_lock.held_for() {
                if held > BATCH_STALE_SECS {
                    if let Some(held) = self.batch_lock.force_reclaim() {
                        self.db.log(
                            "warning",
                            "签到",
                            &format!(
                                "签到批次锁异常占用 {} 秒（上限 {} 秒），已强制回收",
                                held, BATCH_STALE_SECS
                            ),
                        );
                        continue;
                    }
                }
            }
            if started.elapsed() >= wait_limit {
                // 有限等待已到上限：接管继续执行（账号级互斥保证不重复请求）
                if let Some(held) = self.batch_lock.force_reclaim() {
                    self.db.log(
                        "warning",
                        "签到",
                        &format!(
                            "[{}] 等待已有签到批次 {} 秒仍未结束，已强制接管继续执行",
                            source.label(),
                            held
                        ),
                    );
                }
                continue;
            }
            if !wait_noted {
                wait_noted = true;
                self.db.log(
                    "info",
                    "签到",
                    &format!(
                        "[{}] 已有签到批次在执行（已持续 {} 秒），本批次排队等待（最多 {} 秒）",
                        source.label(),
                        self.batch_lock.held_for().unwrap_or(0),
                        BATCH_WAIT_MAX_SECS
                    ),
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(BATCH_WAIT_POLL_MS)).await;
        }
    }

    // 签到批次互斥只约束签到自身的批量/补签路径；积分查询改用账号级互斥，
    // 并通过 batch_lock().is_active() 检测签到批量后主动让位。

    /// TRAE 签到设备号：优先库里已存的固定号；缺失时才从账号 vault 的客户端真实号补齐，
    /// 再不行按账号种子派生。9074 后会把失败号轮换掉（见 trae_checkin）。
    fn trae_device_id(&self, acc: &Account) -> String {
        let others: HashSet<String> = self
            .db
            .list_accounts()
            .unwrap_or_default()
            .into_iter()
            .filter(|a| a.id != acc.id)
            .map(|a| a.device_id.trim().to_string())
            .filter(|d| !d.is_empty())
            .collect();
        // 1) 库里已存的号（上一次签到成功/轮换后的号，保持稳定）
        let current = acc.device_id.trim();
        if !current.is_empty() && !others.contains(current) && is_aha_device(current) {
            return current.to_string();
        }
        // 2) vault 里客户端真实注册过的设备号
        if let Some(real) =
            crate::platform::trae::switch::vault_real_device_id(&self.data_dir, &acc.id)
        {
            if is_aha_device(&real) && !others.contains(&real) {
                let _ = self.db.update_account_device_id(&acc.id, &real);
                return real;
            }
        }
        // 3) 按账号种子派生
        let seed_uid = if acc.uid.trim().is_empty() {
            acc.id.as_str()
        } else {
            acc.uid.trim()
        };
        let id = derived_device_id(&format!("trae-cn-device\0{}\0{}", current, seed_uid));
        let _ = self.db.update_account_device_id(&acc.id, &id);
        id
    }

    /// 换一个没被其它账号占用、本轮也没试过的全新设备号
    fn fresh_device_id(
        &self,
        acc: &Account,
        prev: &str,
        attempt: usize,
        tried: &HashSet<String>,
    ) -> String {
        let others: HashSet<String> = self
            .db
            .list_accounts()
            .unwrap_or_default()
            .into_iter()
            .filter(|a| a.id != acc.id)
            .map(|a| a.device_id.trim().to_string())
            .filter(|d| !d.is_empty())
            .collect();
        let mut salt = format!("{}\0{}", prev, attempt);
        let mut cand = derived_device_id(&format!("trae-cn-device\0{}\0{}", acc.id, salt));
        for _ in 0..50 {
            if !others.contains(&cand) && !tried.contains(&cand) {
                return cand;
            }
            salt.push('x');
            cand = derived_device_id(&format!("trae-cn-device\0{}\0{}", acc.id, salt));
        }
        cand
    }

    /// TRAE 签到：先用账号自己的设备号。若被 9074 拒绝——服务端是按「设备号」拉黑的，
    /// 同一个号会一直 9074——就换全新号重试（有上限），成功后把新号落库复用。
    async fn trae_checkin(&self, acc: &Account) -> (bool, i64, String) {
        let primary = self.trae_device_id(acc);
        let mut tried: HashSet<String> = HashSet::new();
        let mut device_id = primary.clone();
        let mut last: (bool, i64, String) = (false, 0, String::new());
        for attempt in 0..TRAE_9074_MAX_ATTEMPTS {
            let r = trae_api::do_checkin(acc, &device_id)
                .await
                .unwrap_or_else(|e| (false, 0, e.to_string()));
            if r.0 {
                if device_id != primary {
                    let _ = self.db.update_account_device_id(&acc.id, &device_id);
                }
                return r;
            }
            // 非 9074 的错误换号也没用，直接返回
            if !r.2.contains(TRAE_MASTER_RISK_CODE) {
                return r;
            }
            last = r;
            if attempt + 1 == TRAE_9074_MAX_ATTEMPTS {
                break;
            }
            tried.insert(device_id.clone());
            device_id = self.fresh_device_id(acc, &device_id, attempt, &tried);
            // 换号之间留一点间隔，避免连续请求放大风控
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        }
        // 全部失败：把最后一个号落库，下一轮从它再换，避免永远卡在同一个被拉黑的号
        let _ = self.db.update_account_device_id(&acc.id, &device_id);
        let hint = format!(
            "（9074 是本设备号被上游限流：已自动更换设备号重试 {} 次仍未放行，已进入冷却，由定时任务自动补签）",
            TRAE_9074_MAX_ATTEMPTS
        );
        (false, 0, format!("{}{}", last.2, hint))
    }

    /// TRAE 签到：access_token 失效（code=1001 认证失败）时，
    /// 若有 refresh_token 先按设备绑定方式续期再重试一次
    async fn trae_checkin_with_reauth(&self, acc: &Account) -> (bool, i64, String) {
        let r = self.trae_checkin(acc).await;
        let auth_failed = r.2.contains("code=1001") || r.2.contains("401");
        if r.0 || !auth_failed {
            return r;
        }
        if acc.refresh_token.trim().is_empty() {
            return (
                false,
                0,
                format!("{}（凭证已失效且无 refresh_token，请重新授权）", r.2),
            );
        }
        match trae_api::refresh_access_token(acc).await {
            Ok((new_access, new_refresh, expires_at, device_bound)) => {
                let _ = self.db.update_account_tokens(
                    &acc.id,
                    &new_access,
                    &new_refresh,
                    expires_at,
                    acc.refresh_token_expires_at,
                );
                self.db.log(
                    if device_bound { "info" } else { "error" },
                    "续期",
                    &format!(
                        "{}：签到前凭证失效，已自动续期（{}）并重试",
                        acc.name,
                        if device_bound {
                            "设备绑定"
                        } else {
                            "未绑定设备，签到可能被 9074 拒绝"
                        }
                    ),
                );
                // 重新读库：拿到续期后的 token 与最新设备号
                let retry_acc = match self.db.get_account(&acc.id).ok().flatten() {
                    Some(mut fresh) => {
                        fresh.access_token = new_access;
                        fresh
                    }
                    None => {
                        let mut fallback = acc.clone();
                        fallback.access_token = new_access;
                        fallback
                    }
                };
                self.trae_checkin(&retry_acc).await
            }
            Err(e) => (
                false,
                0,
                format!("{}（自动续期失败: {}，请重新授权）", r.2, e),
            ),
        }
    }

    /// 补签当日未签到的账号；platforms 为 None 时覆盖全部平台
    pub async fn retry_missed(
        &self,
        platforms: Option<&[String]>,
        source: CheckinSource,
    ) -> Result<Vec<CheckinResult>, String> {
        // RAII：正常返回或 panic 展开都会释放批次锁
        let _guard = self.acquire_batch_guard(source).await;
        self.do_retry_missed(platforms, source).await
    }

    async fn do_retry_missed(
        &self,
        platforms: Option<&[String]>,
        source: CheckinSource,
    ) -> Result<Vec<CheckinResult>, String> {
        let accounts = self.db.list_accounts().map_err(|e| e.to_string())?;
        let accounts: Vec<_> = accounts
            .into_iter()
            .filter(|a| {
                // 国际版无签到接口，不参与补签
                if a.platform == "workbuddy"
                    && crate::platform::workbuddy::variant::variant_of(a)
                        == crate::platform::workbuddy::variant::WbVariant::Ai
                {
                    return false;
                }
                platforms
                    .map(|ps| platform_in_scope(&a.platform, ps))
                    .unwrap_or(true)
            })
            .collect();
        let now = Utc::now().timestamp();
        let mut results = Vec::new();
        let settings = load_checkin_settings(&self.db);
        for acc in &accounts {
            // 今日已成功签到的账号不再发起请求：这是「一天只签一次」的硬门槛
            if !checked_in_today(acc.last_checkin_time)
                && acc.cooldown_until <= now
                && !is_expired(acc, now)
            {
                if let Ok(r) = self.checkin_one_from(&acc.id, source).await {
                    let skipped = r.skipped;
                    results.push(r);
                    if skipped {
                        continue;
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(
                    settings.interval_seconds as u64,
                ))
                .await;
            }
        }
        Ok(results)
    }
}

// 凭证已失效（与界面「已失效」一致）：token 到期时间已过
fn is_expired(acc: &Account, now: i64) -> bool {
    acc.token_expires_at > 0 && acc.token_expires_at < now
}

// 签到日边界：本地自然日 0 点。上游额度/签到按自然日重置，若跨 0 点仍用
// 其它边界（如 6 点）会把 0~6 点已成功的签到判成「昨日未签」，导致边界巡检重复请求。
fn checked_in_today(last_checkin: i64) -> bool {
    if last_checkin == 0 {
        return false;
    }
    let now = Local::now();
    // 用「当天本地日期 + 当前时区偏移」反算日界，而不是把本地 00:00 交给时区解析：
    // 夏令时切换可能让本地 00:00 不存在或有歧义，single() 会返回 None 并退化成 0，
    // 从而把所有账号都误判为「今日已签到」。这里任何时区都能得到确定的时间戳。
    let offset_secs = now.offset().local_minus_utc() as i64;
    let today_start = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .expect("00:00:00 必定是合法时间")
        .and_utc()
        .timestamp()
        - offset_secs;
    last_checkin >= today_start
}

fn today_str() -> String {
    let now = Local::now();
    format!("{:04}-{:02}-{:02}", now.year(), now.month(), now.day())
}

pub fn load_checkin_settings(db: &Database) -> CheckinSettings {
    let json = db.get_setting("checkin_settings").unwrap_or(None);
    match json {
        Some(s) => {
            let mut cfg: CheckinSettings = serde_json::from_str(&s).unwrap_or_default();
            // 历史配置兼容：codebuddy_cn 已归入 workbuddy 范围（同一账号体系）
            if cfg.platforms.iter().any(|p| p == "codebuddy_cn") {
                cfg.platforms.retain(|p| p != "codebuddy_cn");
                if !cfg.platforms.iter().any(|p| p == "workbuddy") {
                    cfg.platforms.push("workbuddy".to_string());
                }
            }
            cfg
        }
        None => CheckinSettings::default(),
    }
}
