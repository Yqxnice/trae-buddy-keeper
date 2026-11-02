use chrono::{Datelike, Local, Utc};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::data::db::Database;
use crate::data::models::*;
use crate::platform::trae::api as trae_api;
use crate::platform::workbuddy::api as workbuddy_api;

/// 9074/9095 语义：9074=该设备号被上游限流/拉黑，9095=该设备今日已签到
const TRAE_MASTER_RISK_CODE: &str = "code=9074";
/// 9074 时换设备号的总尝试次数（含第一次原号）
const TRAE_9074_MAX_ATTEMPTS: usize = 3;

/// 设备号形态：12~20 位纯数字（客户端注册过的 Aha 设备号，15/16 位常见但非规则）
fn is_aha_device(s: &str) -> bool {
    matches!(s.len(), 12..=20) && s.bytes().all(|b| b.is_ascii_digit())
}

/// 平台范围匹配：勾选 workbuddy 时同时覆盖 codebuddy_cn（同一账号体系）
fn platform_in_scope(platform: &str, scope: &[String]) -> bool {
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
    running: Arc<Mutex<bool>>,
    /// 应用数据目录（读账号 vault 里客户端真实设备号用）
    data_dir: std::path::PathBuf,
}

impl CheckinService {
    pub fn new(db: Arc<Database>, data_dir: std::path::PathBuf) -> Self {
        CheckinService {
            db,
            running: Arc::new(Mutex::new(false)),
            data_dir,
        }
    }

    pub async fn is_running(&self) -> bool {
        *self.running.lock().await
    }

    // 单账户签到（含冷却与隔离判断）
    pub async fn checkin_one(&self, account_id: &str) -> Result<CheckinResult, String> {
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
        // 3) 今日已签到（6 点边界）
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
            &format!("[{}] {}：{}", acc.platform, acc.name, message),
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
            // 签到后刷新真实积分（失败不影响签到结果）
            let _ = crate::platform::workbuddy::credits::refresh_account(&self.db, &acc.id).await;
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

    // 批量签到（可按平台过滤）
    /// 批量签到；platforms 为 None 时签到全部平台，否则仅签到范围内的平台
    pub async fn checkin_all(
        &self,
        platforms: Option<&[String]>,
    ) -> Result<Vec<CheckinResult>, String> {
        let mut lock = self.running.lock().await;
        if *lock {
            return Err("已有签到任务在执行中".to_string());
        }
        *lock = true;
        drop(lock);

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
                .checkin_one(&acc.id)
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

        let mut lock = self.running.lock().await;
        *lock = false;
        Ok(results)
    }

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

    // 超时补签到：检查今天是否有账户未签到
    /// 补签当日未签到的账号；platforms 为 None 时覆盖全部平台
    pub async fn retry_missed(
        &self,
        platforms: Option<&[String]>,
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
            if !checked_in_today(acc.last_checkin_time)
                && acc.cooldown_until <= now
                && !is_expired(acc, now)
            {
                if let Ok(r) = self.checkin_one(&acc.id).await {
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

// 6 点边界：本地时间今日 6 点之后签到才算今天
fn checked_in_today(last_checkin: i64) -> bool {
    if last_checkin == 0 {
        return false;
    }
    // 今日本地 6 点的时间戳
    let today_six = Local::now()
        .date_naive()
        .and_hms_opt(6, 0, 0)
        .unwrap()
        .and_local_timezone(Local)
        .single()
        .map(|dt| dt.timestamp())
        .unwrap_or(0);
    last_checkin >= today_six
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
