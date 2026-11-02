//! 实现参考了开源项目 workbuddy-switch、workbuddy-manager、TraeTools。

use std::sync::Arc;

use chrono::{Local, Utc};
use tauri::{Manager, State};
use uuid::Uuid;

pub mod data;
pub mod infra;
pub mod platform;
pub mod services;

use crate::data::db::Database;
use crate::data::models::*;
use crate::infra::notification::Notifier;
use crate::services::checkin::{load_checkin_settings, CheckinService};
use crate::services::scheduler::Scheduler;
use crate::services::token::TokenManager;

pub struct AppState {
    pub db: Arc<Database>,
    pub checkin: Arc<CheckinService>,
    pub token: Arc<TokenManager>,
    pub notifier: Arc<Notifier>,
    pub scheduler: Arc<Scheduler>,
    /// 应用数据目录（vault 快照根目录的父级）
    pub data_dir: std::path::PathBuf,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let app_dir = app.path().app_data_dir().expect("无法获取应用数据目录");
            std::fs::create_dir_all(&app_dir).ok();
            let db_path = app_dir.join("trae-buddy-keeper.db");
            let db = match Database::new(db_path.to_str().unwrap()) {
                Ok(d) => Arc::new(d),
                Err(e) => {
                    eprintln!("数据库初始化失败: {}", e);
                    return Err(e.into());
                }
            };
            // 日志变更广播依赖应用句柄，需在所有写日志操作前注入
            db.set_emitter(app.handle().clone());

            #[cfg(windows)]
            crate::infra::notification::register_windows_app_id(
                &app.config().identifier,
                app.config()
                    .product_name
                    .as_deref()
                    .unwrap_or("TraeBuddy Keeper"),
            );
            let notif_mode = db
                .get_setting("notification_mode")
                .unwrap_or(None)
                .unwrap_or_else(|| "both".to_string());
            let notifier = Arc::new(Notifier::new(app.handle().clone(), notif_mode));
            let checkin = Arc::new(CheckinService::new(db.clone(), app_dir.clone()));
            let token = Arc::new(TokenManager::new(db.clone()));
            let scheduler = Arc::new(Scheduler::new(
                db.clone(),
                checkin.clone(),
                token.clone(),
                notifier.clone(),
            ));

            let state = AppState {
                db: db.clone(),
                checkin: checkin.clone(),
                token: token.clone(),
                notifier: notifier.clone(),
                scheduler: scheduler.clone(),
                data_dir: app_dir.clone(),
            };
            app.manage(state);

            // 首次启动：初始化通用设置并自动探测客户端安装路径
            if db.get_setting("app_settings").unwrap_or(None).is_none() {
                let mut s = AppSettings::default();
                let paths = crate::infra::clients::detect_client_paths();
                s.client_path_trae = paths.trae;
                s.client_path_workbuddy = paths.workbuddy;
                s.client_path_workbuddy_ai = paths.workbuddy_ai;
                s.client_path_codebuddy_cn = paths.codebuddy_cn;
                if let Ok(json) = serde_json::to_string(&s) {
                    let _ = db.set_setting("app_settings", &json);
                }
            }
            let oauth_server = crate::platform::trae::oauth::OAuthServer(std::sync::Arc::new(
                std::sync::Mutex::new(crate::platform::trae::oauth::OAuthState::default()),
            ));
            app.manage(oauth_server.clone());
            // 预热 OAuth 回调服务器，避免点击授权时临时绑定端口的延迟
            crate::platform::trae::oauth::prewarm(app.handle().clone(), oauth_server);

            scheduler.clone().start();

            if let Err(e) = crate::infra::tray::setup_tray(app.handle()) {
                eprintln!("托盘初始化失败: {}", e);
            }

            if let Some(win) = app.get_webview_window("main") {
                // 启动时最小化到托盘（窗口默认不显示，按需显示）
                if !db.load_app_settings().minimize_to_tray_on_start {
                    let _ = win.show();
                    let _ = win.set_focus();
                }
                let win_clone = win.clone();
                win.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = win_clone.hide();
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_accounts,
            add_account_manual,
            add_account_import,
            delete_account,
            update_account,
            get_checkin_records,
            checkin_one,
            checkin_all,
            get_points_dashboard,
            get_checkin_settings,
            set_checkin_settings,
            get_notification_mode,
            set_notification_mode,
            renew_token,
            auto_renew_all,
            crate::platform::trae::oauth::start_trae_oauth,
            crate::platform::trae::oauth::stop_trae_oauth,
            crate::platform::trae::oauth::complete_trae_oauth,
            crate::platform::workbuddy::login::start_workbuddy_login,
            crate::platform::workbuddy::login::poll_workbuddy_login,
            switch_account,
            switch_workbuddy_client,
            switch_codebuddy_cn_client,
            switch_trae_client,
            trae_client_running,
            backup_trae_vault,
            trae_vault_info,
            get_account_detail,
            refresh_credits,
            refresh_all_credits,
            get_credit_packages,
            get_logs,
            clear_logs,
            get_app_settings,
            get_data_dir,
            open_data_dir,
            backup_data,
            import_data,
            set_app_settings,
            detect_client_paths,
            get_token_statistics,
        ])
        .run(tauri::generate_context!())
        .expect("运行 Tauri 应用失败");
}

// ===== 账户命令 =====

#[tauri::command]
fn list_accounts(state: State<AppState>) -> Result<Vec<AccountInfo>, String> {
    let accounts = state.db.list_accounts().map_err(|e| e.to_string())?;
    let mut infos: Vec<AccountInfo> = accounts.into_iter().map(AccountInfo::from).collect();
    // 每个客户端当前注入的账户 id → 前端在卡片头像上渲染客户端品牌角标
    for (key, client) in [
        ("active_client_trae", "trae"),
        ("active_client_workbuddy", "workbuddy"),
        ("active_client_codebuddy_cn", "codebuddy_cn"),
    ] {
        if let Ok(Some(id)) = state.db.get_setting(key) {
            for info in infos.iter_mut() {
                if info.id == id {
                    info.clients.push(client.to_string());
                }
            }
        }
    }
    Ok(infos)
}

#[tauri::command]
fn get_account_detail(state: State<AppState>, id: String) -> Result<Option<Account>, String> {
    state.db.get_account(&id).map_err(|e| e.to_string())
}

#[tauri::command]
fn add_account_manual(
    state: State<AppState>,
    platform: String,
    name: String,
    email: String,
    access_token: String,
    refresh_token: String,
    cookies: String,
) -> Result<String, String> {
    let now = Utc::now().timestamp();
    let acc = Account {
        id: Uuid::new_v4().to_string(),
        platform: platform.clone(),
        name: if name.is_empty() {
            format!("{}-{}", platform, now % 10000)
        } else {
            name
        },
        email,
        avatar: String::new(),
        uid: String::new(),
        domain: String::new(),
        enterprise_id: String::new(),
        account_type: String::new(),
        device_id: String::new(),
        device_key: String::new(),
        machine_id: String::new(),
        access_token,
        refresh_token,
        cookies,
        token_expires_at: now + 7 * 86400,
        refresh_token_expires_at: now + 30 * 86400,
        last_checkin_time: 0,
        checkin_status: CheckinStatus::Idle.as_str().to_string(),
        fail_count: 0,
        cooldown_until: 0,
        created_at: now,
        updated_at: now,
        credits_balance: None,
        credits_expiries: String::new(),
        credits_fetched_at: 0,
    };
    state.db.insert_account(&acc).map_err(|e| e.to_string())?;
    state.db.log(
        "info",
        "账户",
        &format!("手动添加 [{}] {}", platform, acc.name),
    );
    Ok(acc.id)
}

#[tauri::command]
fn add_account_import(state: State<AppState>, json: String) -> Result<String, String> {
    // 支持导入一个或多个账户的 JSON
    let v: serde_json::Value =
        serde_json::from_str(&json).map_err(|e| format!("JSON 解析失败: {}", e))?;
    let arr = if v.is_array() {
        v.as_array().cloned().unwrap()
    } else {
        vec![v]
    };
    let now = Utc::now().timestamp();
    let mut count = 0;
    for item in arr {
        let platform = item
            .get("platform")
            .and_then(|p| p.as_str())
            .unwrap_or("trae")
            .to_string();
        let name = item
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or("")
            .to_string();
        let email = item
            .get("email")
            .and_then(|e| e.as_str())
            .unwrap_or("")
            .to_string();
        let access_token = item
            .get("access_token")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        let refresh_token = item
            .get("refresh_token")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string();
        let cookies = item
            .get("cookies")
            .map(|c| c.to_string())
            .unwrap_or_else(|| "[]".to_string());
        let avatar = item
            .get("avatar")
            .and_then(|a| a.as_str())
            .unwrap_or("")
            .to_string();
        let uid = item
            .get("uid")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string();
        let domain = item
            .get("domain")
            .and_then(|d| d.as_str())
            .unwrap_or("")
            .to_string();
        // 兼容从外部工具导出的企业账号字段
        let enterprise_id = item
            .get("enterprise_id")
            .or_else(|| item.get("enterpriseId"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let account_type = item
            .get("account_type")
            .or_else(|| item.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let acc = Account {
            id: Uuid::new_v4().to_string(),
            platform,
            name,
            email,
            avatar,
            uid,
            domain,
            enterprise_id,
            account_type,
            device_id: String::new(),
            device_key: String::new(),
            machine_id: String::new(),
            access_token,
            refresh_token,
            cookies,
            token_expires_at: now + 7 * 86400,
            refresh_token_expires_at: now + 30 * 86400,
            last_checkin_time: 0,
            checkin_status: CheckinStatus::Idle.as_str().to_string(),
            fail_count: 0,
            cooldown_until: 0,
            created_at: now,
            updated_at: now,
            credits_balance: None,
            credits_expiries: String::new(),
            credits_fetched_at: 0,
        };
        state.db.insert_account(&acc).map_err(|e| e.to_string())?;
        count += 1;
    }
    state
        .db
        .log("info", "账户", &format!("导入 {} 个账户", count));
    Ok(format!("导入成功 {} 个账户", count))
}

#[tauri::command]
fn delete_account(state: State<AppState>, id: String) -> Result<(), String> {
    let name = state
        .db
        .get_account(&id)
        .ok()
        .flatten()
        .map(|a| a.name)
        .unwrap_or_default();
    state.db.delete_account(&id).map_err(|e| e.to_string())?;
    state.db.log("info", "账户", &format!("删除账户 {}", name));
    Ok(())
}

#[tauri::command]
fn update_account(
    state: State<AppState>,
    id: String,
    name: String,
    email: String,
    access_token: String,
    refresh_token: String,
    cookies: String,
) -> Result<(), String> {
    state
        .db
        .update_account_info(&id, &name, &email, &access_token, &refresh_token, &cookies)
        .map_err(|e| e.to_string())?;
    state.db.log("info", "账户", &format!("编辑账户 {}", name));
    Ok(())
}

// ===== 系统日志 =====

#[tauri::command]
fn get_logs(state: State<AppState>, limit: Option<i64>) -> Result<Vec<AppLog>, String> {
    state
        .db
        .list_logs(limit.unwrap_or(200))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn clear_logs(state: State<AppState>) -> Result<(), String> {
    state.db.clear_logs().map_err(|e| e.to_string())
}

// ===== 签到命令 =====

#[tauri::command]
async fn checkin_one(state: State<'_, AppState>, id: String) -> Result<CheckinResult, String> {
    state.checkin.checkin_one(&id).await
}

#[tauri::command]
async fn checkin_all(
    state: State<'_, AppState>,
    platform: Option<String>,
) -> Result<Vec<CheckinResult>, String> {
    let results = state
        .checkin
        .checkin_all(platform.map(|p| vec![p]).as_deref())
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
fn get_checkin_records(
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
fn get_points_dashboard(
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

// ===== 真实积分 =====

#[tauri::command]
async fn refresh_credits(state: State<'_, AppState>, id: String) -> Result<CreditResult, String> {
    Ok(crate::platform::workbuddy::credits::refresh_account(&state.db, &id).await)
}

#[tauri::command]
async fn refresh_all_credits(
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
fn get_credit_packages(
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

// ===== 设置 =====

#[tauri::command]
fn get_checkin_settings(state: State<AppState>) -> Result<CheckinSettings, String> {
    Ok(load_checkin_settings(&state.db))
}

#[tauri::command]
fn set_checkin_settings(state: State<AppState>, settings: CheckinSettings) -> Result<(), String> {
    let json = serde_json::to_string(&settings).map_err(|e| e.to_string())?;
    state
        .db
        .set_setting("checkin_settings", &json)
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_notification_mode(state: State<'_, AppState>) -> Result<String, String> {
    Ok(state.notifier.get_mode().await)
}

#[tauri::command]
async fn set_notification_mode(state: State<'_, AppState>, mode: String) -> Result<(), String> {
    state.notifier.set_mode(&mode).await;
    state
        .db
        .set_setting("notification_mode", &mode)
        .map_err(|e| e.to_string())
}

// ===== 数据备份与恢复 =====

/// 递归复制目录（覆盖式合并），统计成功/失败文件数（被占用文件跳过）
fn copy_dir_recursive(
    src: &std::path::Path,
    dest: &std::path::Path,
    copied: &mut usize,
    failed: &mut usize,
) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    for entry in std::fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        let to = dest.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &to, copied, failed)?;
        } else {
            match std::fs::copy(entry.path(), &to) {
                Ok(_) => *copied += 1,
                Err(_) => *failed += 1,
            }
        }
    }
    Ok(())
}

#[tauri::command]
async fn backup_data(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let db = state.db.clone();
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<Option<String>, String> {
        // 弹窗选择备份保存位置；关闭弹窗视为取消
        let Some(dest_root) = rfd::FileDialog::new()
            .set_title("选择备份保存位置")
            .pick_folder()
        else {
            return Ok(None);
        };
        let name = format!(
            "TraeBuddyKeeper-Backup-{}",
            chrono::Local::now().format("%Y%m%d-%H%M%S")
        );
        let dest = dest_root.join(name);
        std::fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
        // 数据库一致性快照
        db.backup_to(&dest.join("trae-buddy-keeper.db"))
            .map_err(|e| e.to_string())?;
        let mut note = dest.to_string_lossy().to_string();
        // 授权会话目录（存在才复制）
        let vaults = data_dir.join("vaults");
        if vaults.exists() {
            let (mut copied, mut failed) = (0usize, 0usize);
            copy_dir_recursive(&vaults, &dest.join("vaults"), &mut copied, &mut failed)?;
            if failed > 0 {
                note = format!("{}（{} 个会话文件被占用未复制）", note, failed);
            }
        }
        db.log("info", "备份", &format!("数据已备份至 {}", dest.display()));
        Ok(Some(note))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn import_data(state: State<'_, AppState>) -> Result<Option<String>, String> {
    let db = state.db.clone();
    let data_dir = state.data_dir.clone();
    tauri::async_runtime::spawn_blocking(move || -> Result<Option<String>, String> {
        let Some(src_root) = rfd::FileDialog::new()
            .set_title("选择备份文件夹（需包含 trae-buddy-keeper.db）")
            .pick_folder()
        else {
            return Ok(None);
        };
        let mut src_db = src_root.join("trae-buddy-keeper.db");
        if !src_db.exists() {
            // 兼容更名前的旧备份（数据库文件名为 ai-manager.db）
            let legacy = src_root.join("ai-manager.db");
            if !legacy.exists() {
                return Err("所选文件夹不包含 trae-buddy-keeper.db，不是有效备份".to_string());
            }
            src_db = legacy;
        }
        db.restore_from(&src_db).map_err(|e| e.to_string())?;
        let mut note = "数据库已恢复，建议重启应用".to_string();
        let src_vaults = src_root.join("vaults");
        if src_vaults.exists() {
            let (mut copied, mut failed) = (0usize, 0usize);
            copy_dir_recursive(&src_vaults, &data_dir.join("vaults"), &mut copied, &mut failed)?;
            note = format!("数据库与 {} 个会话文件已恢复，建议重启应用", copied);
            if failed > 0 {
                note.push_str(&format!("（{} 个文件被占用跳过）", failed));
            }
        }
        db.log("info", "导入", &format!("从 {} 恢复数据", src_root.display()));
        Ok(Some(note))
    })
    .await
    .map_err(|e| e.to_string())?
}

// ===== 通用设置 =====

#[tauri::command]
fn get_app_settings(state: State<AppState>) -> Result<AppSettings, String> {
    Ok(state.db.load_app_settings())
}

#[tauri::command]
fn get_data_dir(state: State<AppState>) -> String {
    state.data_dir.to_string_lossy().to_string()
}

#[tauri::command]
fn open_data_dir(state: State<AppState>) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&state.data_dir)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&state.data_dir)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&state.data_dir)
            .spawn()
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

#[tauri::command]
fn set_app_settings(state: State<AppState>, settings: AppSettings) -> Result<(), String> {
    let json = serde_json::to_string(&settings).map_err(|e| e.to_string())?;
    state
        .db
        .set_setting("app_settings", &json)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn detect_client_paths() -> crate::infra::clients::ClientPaths {
    crate::infra::clients::detect_client_paths()
}

// ===== Token 续期 =====

#[tauri::command]
async fn renew_token(state: State<'_, AppState>, id: String) -> Result<String, String> {
    let name = state
        .db
        .get_account(&id)
        .ok()
        .flatten()
        .map(|a| a.name)
        .unwrap_or_else(|| id.clone());
    match state.token.renew(&id).await {
        Ok(Some(msg)) => {
            state
                .db
                .log("success", "续期", &format!("{}：{}", name, msg));
            Ok(msg)
        }
        Ok(None) => Ok("无需续期".to_string()),
        Err(e) => {
            state.db.log("error", "续期", &format!("{}：{}", name, e));
            Err(e.to_string())
        }
    }
}

#[tauri::command]
async fn auto_renew_all(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let accounts = state.db.list_accounts().map_err(|e| e.to_string())?;
    let mut msgs = Vec::new();
    for acc in accounts {
        match state.token.auto_renew_if_needed(&acc.id).await {
            Ok(Some(msg)) => {
                state
                    .db
                    .log("success", "续期", &format!("{}：{}", acc.name, msg));
                msgs.push(format!("{}: {}", acc.name, msg));
            }
            Ok(None) => {}
            Err(e) => {
                state
                    .db
                    .log("error", "续期", &format!("{}：{}", acc.name, e));
                msgs.push(format!("{}: {}", acc.name, e));
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    Ok(msgs)
}

// ===== 账户切换 =====

#[tauri::command]
fn switch_account(state: State<AppState>, id: String) -> Result<String, String> {
    // 标记当前活跃账户（前端据此高亮）
    state
        .db
        .set_setting("active_account", &id)
        .map_err(|e| e.to_string())?;
    Ok(format!("已切换到账户 {}", id))
}

/// WorkBuddy 客户端真实切换：写官方认证文件并重启客户端（对齐参考实现方案）
#[tauri::command]
async fn switch_workbuddy_client(state: State<'_, AppState>, id: String) -> Result<String, String> {
    let acc = state
        .db
        .get_account(&id)
        .map_err(|e| e.to_string())?
        .ok_or("账户不存在")?;
    if acc.platform != "workbuddy" {
        return Err("仅 WorkBuddy 账户支持客户端切换".to_string());
    }
    if acc.access_token.is_empty() {
        return Err("该账户缺少 access_token，请重新登录后再切换".to_string());
    }
    // 客户端路径：设置页配置优先，其次注册表探测；国际版账号用 WorkBuddy AI 客户端。
    // 先解析再关进程，避免关了才发现没路径
    let exe = {
        let settings = state.db.load_app_settings();
        let detected = crate::infra::clients::detect_client_paths();
        match crate::platform::workbuddy::variant::variant_of(&acc) {
            crate::platform::workbuddy::variant::WbVariant::Ai => {
                let configured = settings.client_path_workbuddy_ai;
                if !configured.is_empty() {
                    configured
                } else {
                    detected.workbuddy_ai
                }
            }
            crate::platform::workbuddy::variant::WbVariant::Cn => {
                let configured = settings.client_path_workbuddy;
                if !configured.is_empty() {
                    configured
                } else {
                    detected.workbuddy
                }
            }
        }
    };
    if exe.is_empty() {
        return Err("未找到 WorkBuddy 安装路径，请到设置页配置客户端路径".to_string());
    }
    let db = state.db.clone();
    let name = acc.name.clone();
    // 关进程/写文件为阻塞操作，放到阻塞线程池，避免卡住异步运行时
    let result = tauri::async_runtime::spawn_blocking(move || {
        crate::platform::workbuddy::switch::switch_to_client(&acc, &exe)
    })
    .await
    .map_err(|e| format!("切换任务执行失败: {e}"))?;
    match &result {
        Ok(msg) => {
            db.log(
                "success",
                "切换",
                &format!("WorkBuddy 客户端已切换到「{name}」"),
            );
            // 与活跃账户高亮语义保持一致
            let _ = db.set_setting("active_account", &id);
            let _ = db.set_setting("active_client_workbuddy", &id);
            Ok(msg.clone())
        }
        Err(e) => {
            db.log(
                "error",
                "切换",
                &format!("WorkBuddy 切换到「{name}」失败：{e}"),
            );
            Err(e.clone())
        }
    }
}

#[tauri::command]
async fn switch_codebuddy_cn_client(
    state: State<'_, AppState>,
    id: String,
) -> Result<String, String> {
    let acc = state
        .db
        .get_account(&id)
        .map_err(|e| e.to_string())?
        .ok_or("账户不存在")?;
    if acc.platform != "workbuddy" {
        return Err("仅 WorkBuddy/CodeBuddy 账户支持切换到 CodeBuddy CN".to_string());
    }
    if acc.access_token.is_empty() {
        return Err("该账户缺少 access_token，请重新登录后再切换".to_string());
    }
    // 客户端路径：设置页配置优先，其次注册表探测；先解析再关进程
    let exe = {
        let configured = state.db.load_app_settings().client_path_codebuddy_cn;
        if !configured.is_empty() {
            configured
        } else {
            crate::infra::clients::detect_client_paths().codebuddy_cn
        }
    };
    if exe.is_empty() {
        return Err("未找到 CodeBuddy CN 安装路径，请到设置页配置客户端路径".to_string());
    }
    let db = state.db.clone();
    let name = acc.name.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        crate::platform::workbuddy::codebuddy_cn::switch_to_client(&acc, &exe)
    })
    .await
    .map_err(|e| format!("切换任务执行失败: {e}"))?;
    match &result {
        Ok(msg) => {
            db.log(
                "success",
                "切换",
                &format!("CodeBuddy CN 已切换到「{name}」"),
            );
            let _ = db.set_setting("active_client_codebuddy_cn", &id);
            Ok(msg.clone())
        }
        Err(e) => {
            db.log(
                "error",
                "切换",
                &format!("CodeBuddy CN 切换到「{name}」失败：{e}"),
            );
            Err(e.clone())
        }
    }
}

/// 解析 Trae CN 客户端 exe 路径（设置页配置优先，其次注册表探测）
fn resolve_trae_exe(db: &Database) -> String {
    let configured = db.load_app_settings().client_path_trae;
    if !configured.is_empty() {
        configured
    } else {
        crate::infra::clients::detect_client_paths().trae
    }
}

/// Trae 账号建档：把当前客户端登录态快照到该账号的 vault（要求客户端已关闭）。
/// close_running=true 时自动关闭仍在运行的客户端后再建档（前端已弹窗征得确认）。
#[tauri::command]
async fn backup_trae_vault(
    state: State<'_, AppState>,
    id: String,
    close_running: bool,
) -> Result<String, String> {
    let acc = state
        .db
        .get_account(&id)
        .map_err(|e| e.to_string())?
        .ok_or("账户不存在")?;
    if acc.platform != "trae" {
        return Err("仅 Trae 账户需要建档".to_string());
    }
    let exe = resolve_trae_exe(&state.db);
    let image = std::path::Path::new(&exe)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Trae CN.exe")
        .to_string();
    let data_dir = state.data_dir.clone();
    let db = state.db.clone();
    let name = acc.name.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        crate::platform::trae::switch::backup_vault(&data_dir, &id, &image, close_running)
    })
    .await
    .map_err(|e| format!("建档任务执行失败: {e}"))?;
    match &result {
        Ok(msg) => {
            db.log("success", "建档", &format!("Trae 账号「{name}」{msg}"));
            Ok(msg.clone())
        }
        Err(e) => {
            db.log(
                "error",
                "建档",
                &format!("Trae 账号「{name}」建档失败：{e}"),
            );
            Err(e.clone())
        }
    }
}

/// 查询 Trae 账号建档信息：(建档时间, 载体文件数)；未建档返回 None
#[tauri::command]
fn trae_vault_info(state: State<AppState>, id: String) -> Option<(i64, usize)> {
    crate::platform::trae::switch::vault_info(&state.data_dir, &id)
}

/// Trae 客户端当前是否在运行（切换前先检测，运行中切库会写花客户端数据）
#[tauri::command]
fn trae_client_running(state: State<AppState>) -> bool {
    let exe = resolve_trae_exe(&state.db);
    crate::platform::trae::switch::is_running_for_exe(&exe)
}

/// Trae 客户端真实切换：整目录快照冷切换（对齐参考实现方案，含守护与回滚）
#[tauri::command]
async fn switch_trae_client(state: State<'_, AppState>, id: String) -> Result<String, String> {
    let acc = state
        .db
        .get_account(&id)
        .map_err(|e| e.to_string())?
        .ok_or("账户不存在")?;
    if acc.platform != "trae" {
        return Err("仅 Trae 账户支持该切换".to_string());
    }
    // 先解析路径再关进程，避免关了才发现没路径
    let exe = resolve_trae_exe(&state.db);
    if exe.is_empty() {
        return Err("未找到 Trae CN 安装路径，请到设置页配置客户端路径".to_string());
    }
    let data_dir = state.data_dir.clone();
    let db = state.db.clone();
    let name = acc.name.clone();
    let acc_id = acc.id.clone();
    // 先检测客户端：运行中直接改文件会写花它的数据，必须等它完全退出再切
    if crate::platform::trae::switch::is_running_for_exe(&exe) {
        state.db.log(
            "info",
            "切换",
            &format!("检测到 Trae CN 正在运行，先关闭客户端再切换到「{}」", name),
        );
    }
    // 关进程/拷贝载体/守护轮询都是阻塞操作，放阻塞线程池
    let result = tauri::async_runtime::spawn_blocking(move || {
        crate::platform::trae::switch::switch_to_vault(&data_dir, &acc_id, &exe)
    })
    .await
    .map_err(|e| format!("切换任务执行失败: {e}"))?;
    match &result {
        Ok(msg) => {
            db.log("success", "切换", &format!("Trae 客户端已切换到「{name}」"));
            let _ = db.set_setting("active_account", &id);
            let _ = db.set_setting("active_client_trae", &id);
            Ok(msg.clone())
        }
        Err(e) => {
            db.log("error", "切换", &format!("Trae 切换到「{name}」失败：{e}"));
            Err(e.clone())
        }
    }
}

// ===== Token 统计 =====

/// 扫描本地日志的 Token 用量统计（WorkBuddy 国内/国际版、CodeBuddy CLI、CodeBuddy IDE）。
///
/// 纯磁盘扫描，不放异步运行时：用 `spawn_blocking` 避免阻塞主线程。
/// `days` 只接受 7/30/90，其它值或不传视为全量。
/// 另含 TRAE 云端用量源（第四个 source，按 TRAE 账号拉取服务端接口）。
#[tauri::command]
async fn get_token_statistics(
    state: State<'_, AppState>,
    days: Option<i64>,
    force: Option<bool>,
) -> Result<serde_json::Value, String> {
    // 结果缓存：本地日志扫描 + TRAE 云端请求开销大，10 分钟内直接返回缓存；
    // 前端手动刷新传 force=true 跳过缓存强制重算
    const TTL_SECS: i64 = 600;
    let force = force.unwrap_or(false);
    let cache_key = format!("token_stats_cache_{}", days.unwrap_or(0));
    if !force {
        if let Ok(Some(json)) = state.db.get_setting(&cache_key) {
            if let Ok(entry) = serde_json::from_str::<serde_json::Value>(&json) {
                let cached_at = entry
                    .get("cached_at")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
                if chrono::Utc::now().timestamp() - cached_at < TTL_SECS {
                    if let Some(stats) = entry.get("stats") {
                        return Ok(stats.clone());
                    }
                }
            }
        }
    }

    let mut stats =
        tauri::async_runtime::spawn_blocking(move || crate::platform::token_stats::get_statistics(days))
            .await
            .map_err(|error| format!("扫描 Token 统计失败: {error}"))?;

    // ---- TRAE 云端用量源（云端型，无本地日志）----
    let accounts: Vec<_> = state
        .db
        .list_accounts()
        .map_err(|e| e.to_string())?
        .into_iter()
        .filter(|a| a.platform == "trae" && !a.access_token.is_empty())
        .collect();
    let mut items: Vec<(String, serde_json::Value)> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    for (idx, acc) in accounts.iter().enumerate() {
        // 临期/过期先自动续期；失败不阻断，用现有 token 尽力拉取
        let _ = state.token.auto_renew_if_needed(&acc.id).await;
        let acc = state
            .db
            .get_account(&acc.id)
            .ok()
            .flatten()
            .unwrap_or_else(|| acc.clone());
        match crate::platform::trae::usage::fetch_all(&acc.access_token, days).await {
            Ok(sessions) => {
                items.extend(
                    sessions
                        .into_iter()
                        .map(|s| (acc.name.clone(), s.to_stats_record())),
                );
            }
            Err(e) => errors.push(format!("{}：{e}", acc.name)),
        }
        // 多账号之间错开请求，降低调用密度
        if idx + 1 < accounts.len() {
            tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        }
    }
    let notice = if accounts.is_empty() {
        Some("未添加 TRAE 账号，添加并授权后可统计云端用量".to_string())
    } else if days.is_none() {
        Some("云端用量最多回溯 90 天".to_string())
    } else {
        None
    };
    let errors = if errors.is_empty() {
        None
    } else {
        Some(errors.join("；"))
    };
    let source = crate::platform::token_stats::trae_cloud_source(
        items,
        accounts.len(),
        notice.as_deref(),
        errors.as_deref(),
    );
    if let Some(arr) = stats
        .get_mut("sources")
        .and_then(serde_json::Value::as_array_mut)
    {
        arr.push(source);
    }
    // 写入缓存（失败不阻断返回）
    let entry = serde_json::json!({
        "cached_at": chrono::Utc::now().timestamp(),
        "stats": stats,
    });
    let _ = state.db.set_setting(&cache_key, &entry.to_string());
    Ok(stats)
}
