//! 签到命令：单账号 / 批量签到、签到记录、积分看板与签到设置。

use chrono::Local;
use tauri::State;

use crate::data::models::{CheckinRecord, CheckinResult, CheckinSettings, PointsDashboard};
use crate::services::checkin::{load_checkin_settings, CheckinSource};
use crate::AppState;

// ===== 签到命令 =====

#[tauri::command]
pub async fn checkin_one(state: State<'_, AppState>, id: String) -> Result<CheckinResult, String> {
    state.checkin.checkin_one(&id).await
}

#[tauri::command]
pub async fn checkin_all(
    state: State<'_, AppState>,
    platform: Option<String>,
    source: Option<String>,
) -> Result<Vec<CheckinResult>, String> {
    // 来源仅用于日志诊断：托盘传 "tray"，界面按钮不传（默认手动）
    let src = CheckinSource::from_opt(source.as_deref());
    let results = state
        .checkin
        .checkin_all_from(platform.map(|p| vec![p]).as_deref(), src)
        .await?;
    let skipped = results.iter().filter(|r| r.skipped).count();
    let success = results.iter().filter(|r| r.success && !r.skipped).count();
    let failed = results.len() - success - skipped;
    state
        .notifier
        .notify(
            "批量签到完成",
            &format!(
                "成功 {} 个，跳过 {} 个，失败 {} 个",
                success, skipped, failed
            ),
        )
        .await;
    Ok(results)
}

#[tauri::command]
pub fn get_checkin_records(
    state: State<AppState>,
    account_id: Option<String>,
    limit: Option<i64>,
) -> Result<Vec<CheckinRecord>, String> {
    state
        .db
        .list_checkin_records(account_id.as_deref(), limit.unwrap_or(50))
        .map_err(|e| e.to_string())
}

// ===== 积分看板 =====

#[tauri::command]
pub fn get_points_dashboard(
    state: State<AppState>,
    platform: String,
) -> Result<PointsDashboard, String> {
    let today = format!("{}", Local::now().format("%Y-%m-%d"));
    let today_new = state
        .db
        .get_today_points(&today, &platform)
        .map_err(|e| e.to_string())?;
    let trend = state
        .db
        .get_trend(7, &platform)
        .map_err(|e| e.to_string())?;
    // 真实积分（接口落库）；TRAE 签到流水仅用于趋势图
    if platform == "workbuddy" || platform == "trae" {
        let total = state
            .db
            .get_real_total_credits(&platform)
            .map_err(|e| e.to_string())?;
        let expiring = state
            .db
            .get_real_expiring_packages(7, &platform)
            .map_err(|e| e.to_string())?;
        return Ok(PointsDashboard {
            total_points: total as i64,
            today_new,
            trend,
            expiring_packages: expiring,
        });
    }
    let total = state
        .db
        .get_total_points(&platform)
        .map_err(|e| e.to_string())?;
    let expiring = state
        .db
        .get_expiring_packages(7, &platform)
        .map_err(|e| e.to_string())?;
    Ok(PointsDashboard {
        total_points: total,
        today_new,
        trend,
        expiring_packages: expiring,
    })
}

// ===== 设置 =====

#[tauri::command]
pub fn get_checkin_settings(state: State<AppState>) -> Result<CheckinSettings, String> {
    Ok(load_checkin_settings(&state.db))
}

#[tauri::command]
pub fn set_checkin_settings(
    state: State<AppState>,
    settings: CheckinSettings,
) -> Result<(), String> {
    let json = serde_json::to_string(&settings).map_err(|e| e.to_string())?;
    state
        .db
        .set_setting("checkin_settings", &json)
        .map_err(|e| e.to_string())
}
