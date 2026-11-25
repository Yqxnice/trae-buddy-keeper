//! 积分命令：真实积分查询、积分包明细与定时积分查询配置。

use chrono::Utc;
use tauri::State;

use crate::data::models::{
    AccountCredits, CreditExpiry, CreditResult, CreditScheduleSettings, CreditScheduleState,
};
use crate::AppState;

// ===== 真实积分 =====

#[tauri::command]
pub async fn refresh_credits(
    state: State<'_, AppState>,
    id: String,
) -> Result<CreditResult, String> {
    Ok(crate::platform::workbuddy::credits::refresh_account(&state.db, &id).await)
}

#[tauri::command]
pub async fn refresh_all_credits(
    state: State<'_, AppState>,
    platform: Option<String>,
) -> Result<Vec<CreditResult>, String> {
    let mut accounts = state.db.list_accounts().map_err(|e| e.to_string())?;
    if let Some(p) = platform {
        accounts.retain(|a| a.platform == p);
    }
    let mut results = Vec::new();
    for acc in &accounts {
        results
            .push(crate::platform::workbuddy::credits::refresh_account(&state.db, &acc.id).await);
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Ok(results)
}

/// 积分页：当前产品各账号的余额与积分包明细
#[tauri::command]
pub fn get_credit_packages(
    state: State<AppState>,
    platform: String,
) -> Result<Vec<AccountCredits>, String> {
    let now = Utc::now().timestamp();
    let accounts = state.db.list_accounts().map_err(|e| e.to_string())?;
    // 两个平台统一用真实积分（接口落库）
    Ok(accounts
        .into_iter()
        .filter(|a| a.platform == platform)
        .map(|a| {
            let packages: Vec<CreditExpiry> =
                serde_json::from_str(&a.credits_expiries).unwrap_or_default();
            AccountCredits {
                account_id: a.id,
                account_name: a.name,
                platform: a.platform,
                balance: a.credits_balance.unwrap_or(0.0),
                fetched_at: a.credits_fetched_at,
                packages: packages
                    .into_iter()
                    .filter(|e| e.at > now && e.amount > 0.0)
                    .collect(),
            }
        })
        .collect())
}

// ===== 定时积分查询 =====

#[tauri::command]
pub fn get_credit_schedule(state: State<AppState>) -> CreditScheduleState {
    crate::services::credit_schedule::load_state(&state.db)
}

#[tauri::command]
pub fn set_credit_schedule(
    state: State<AppState>,
    settings: CreditScheduleSettings,
) -> Result<(), String> {
    crate::services::credit_schedule::save_settings(&state.db, &settings)
}
