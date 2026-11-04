//! 定时积分查询：按用户设定的间隔批量刷新所有账号的真实积分。
//!
//! 复用现有积分查询实现（TRAE pay/web_user_ent_usage、WorkBuddy billing，含国际版基址分流），
//! 每次执行都直连上游、不读取本地快照，等价于前端手动刷新（force=true）的语义。
//! 与定时签到并列、互不影响；签到优先：积分查询使用账号级互斥，检测到签到批量
//! 运行中会主动让位，不占用签到批次的互斥锁。

use std::sync::Arc;
use std::time::Duration;

use crate::data::db::Database;
use crate::data::models::{CreditScheduleRun, CreditScheduleSettings, CreditScheduleState};
use crate::platform::workbuddy::credits;
use crate::services::account_lock;
use crate::services::checkin::{platform_in_scope, CheckinBatchLock};

const SETTINGS_KEY: &str = "credit_schedule_settings";
/// 最近一次执行结果（前端展示）
const RUN_KEY: &str = "credit_schedule_run";
/// 最近一次「尝试执行」时刻：先落库再查询，异常/失败也不会在下一秒重试风暴
const ATTEMPT_KEY: &str = "credit_schedule_last_attempt";

/// 读取配置：区分「未设置」与「读取/解析失败」。
/// 失败时由调用方负责降级并留痕，不能当作「功能关闭」静默吞掉。
pub fn load_settings(db: &Database) -> Result<CreditScheduleSettings, String> {
    match db.get_setting(SETTINGS_KEY).map_err(|e| e.to_string())? {
        Some(s) => serde_json::from_str(&s).map_err(|e| format!("配置解析失败: {}", e)),
        None => Ok(CreditScheduleSettings::default()),
    }
}

pub fn save_settings(db: &Database, settings: &CreditScheduleSettings) -> Result<(), String> {
    let json = serde_json::to_string(settings).map_err(|e| e.to_string())?;
    db.set_setting(SETTINGS_KEY, &json)
        .map_err(|e| e.to_string())
}

pub fn load_run(db: &Database) -> CreditScheduleRun {
    match db.get_setting(RUN_KEY) {
        Ok(Some(s)) => match serde_json::from_str(&s) {
            Ok(run) => run,
            Err(e) => {
                // 执行摘要仅用于展示，解析失败按空记录降级，但要留痕
                eprintln!("定时积分查询执行摘要解析失败，按空记录展示：{}", e);
                CreditScheduleRun::default()
            }
        },
        Ok(None) => CreditScheduleRun::default(),
        Err(e) => {
            eprintln!("定时积分查询执行摘要读取失败，按空记录展示：{}", e);
            CreditScheduleRun::default()
        }
    }
}

pub fn load_state(db: &Database) -> CreditScheduleState {
    let settings = load_settings(db).unwrap_or_else(|e| {
        // 前端拉取配置失败时也必须留痕，并明确降级为默认（关闭）
        eprintln!("定时积分查询配置读取失败，降级为默认关闭：{}", e);
        db.log(
            "error",
            "积分定时查询",
            &format!("配置读取失败，按默认关闭展示：{}", e),
        );
        CreditScheduleSettings::default()
    });
    CreditScheduleState {
        settings,
        last_run: load_run(db),
    }
}

/// 上次执行（含未跑完的尝试）时刻；0 表示从未执行过。
/// 读失败返回 Err，调用方需用内存值兜底且记录错误，不能当成「从未执行」放宽判定。
pub fn last_attempt_at(db: &Database) -> Result<i64, String> {
    match db.get_setting(ATTEMPT_KEY).map_err(|e| e.to_string())? {
        Some(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Ok(0);
            }
            trimmed
                .parse::<i64>()
                .map_err(|e| format!("上次尝试时刻解析失败: {}", e))
        }
        None => Ok(0),
    }
}

pub fn mark_attempt(db: &Database, at: i64) -> Result<(), String> {
    db.set_setting(ATTEMPT_KEY, &at.to_string())
        .map_err(|e| e.to_string())
}

fn save_run(db: &Database, run: &CreditScheduleRun) {
    match serde_json::to_string(run) {
        Ok(json) => {
            if let Err(e) = db.set_setting(RUN_KEY, &json) {
                eprintln!("定时积分查询执行摘要写入失败：{}", e);
                db.log(
                    "error",
                    "积分定时查询",
                    &format!("执行摘要写入失败：{}", e),
                );
            }
        }
        Err(e) => eprintln!("定时积分查询执行摘要序列化失败：{}", e),
    }
}

/// 账号是否具备可用的查询凭证（无凭证的账号不发请求、也不计失败）
fn has_credentials(acc: &crate::data::models::Account) -> bool {
    !acc.access_token.trim().is_empty() || !acc.cookies.trim().is_empty()
}

/// 批量查询范围内账号的积分：单账号失败不中断，失败逐条写日志（来源「积分定时查询」），
/// 结束时写一条汇总日志并落库执行摘要。scope 为空表示全部平台。
pub async fn run_batch(
    db: &Arc<Database>,
    scope: &[String],
    at: i64,
    batch_lock: Arc<CheckinBatchLock>,
) -> CreditScheduleRun {
    // 读失败必须走错误路径，不能与「确实没有账号」混为一谈
    let accounts = match db.list_accounts() {
        Ok(list) => list,
        Err(e) => {
            let message = format!("账号列表读取失败：{}", e);
            db.log(
                "error",
                "积分定时查询",
                &format!("账号列表读取失败，本次查询未执行：{}", e),
            );
            let run = CreditScheduleRun {
                at,
                success: 0,
                failed: 0,
                message,
            };
            save_run(db, &run);
            return run;
        }
    };
    let accounts: Vec<_> = accounts
        .into_iter()
        .filter(|a| scope.is_empty() || platform_in_scope(&a.platform, scope))
        .filter(has_credentials)
        .collect();

    let (mut success, mut failed, mut skipped) = (0u32, 0u32, 0u32);
    // 签到优先：每处理一个账号都检查签到批量是否已开始，一旦开始立即让位
    let mut yielded: Option<String> = None;
    for (idx, acc) in accounts.iter().enumerate() {
        if batch_lock.is_active() {
            yielded = Some(format!("签到任务已开始，已提前让位（已处理 {}/{} 个）", idx, accounts.len()));
            break;
        }
        // 账号级互斥：该账号正在签到/其它请求时跳过，绝不并发发起第二次请求
        let Some(guard) = account_lock::try_acquire_account(&acc.id) else {
            skipped += 1;
            continue;
        };
        let r = credits::refresh_account_with_action_guarded(db, &acc.id, "积分定时查询", guard).await;
        if r.success {
            success += 1;
        } else {
            failed += 1;
        }
        // 账号之间错开请求，避免同一 IP 上密集调用
        if idx + 1 < accounts.len() {
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    let run = if accounts.is_empty() {
        db.log(
            "info",
            "积分定时查询",
            "范围内无可查询凭证的账号，跳过本次查询",
        );
        CreditScheduleRun {
            at,
            success: 0,
            failed: 0,
            message: "无可查询账号".to_string(),
        }
    } else {
        let mut message = format!(
            "成功 {} 个，失败 {} 个（共 {} 个）",
            success,
            failed,
            accounts.len()
        );
        if skipped > 0 {
            message.push_str(&format!("，跳过 {} 个（账号正在签到）", skipped));
        }
        if let Some(reason) = yielded.as_deref() {
            message.push_str(&format!("；{}", reason));
        }
        let level = if yielded.is_some() {
            "info"
        } else if failed == 0 && skipped == 0 {
            "success"
        } else if success == 0 && skipped == 0 {
            "error"
        } else {
            "info"
        };
        db.log(level, "积分定时查询", &message);
        CreditScheduleRun {
            at,
            success,
            failed,
            message,
        }
    };
    save_run(db, &run);
    run
}
