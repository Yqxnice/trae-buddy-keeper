//! 多平台 AI 客户端账号管理：授权登录、签到、积分、客户端切换与用量统计。

use std::sync::Arc;

use tauri::{Emitter, Manager};

pub mod commands;
pub mod data;
pub mod infra;
pub mod platform;
pub mod services;

use crate::data::db::Database;
use crate::data::models::AppSettings;
use crate::infra::notification::Notifier;
use crate::services::checkin::CheckinService;
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

/// 项目仓库地址（与 README 及更新端点保持一致）
pub const PROJECT_REPO_URL: &str = "https://github.com/Yqxnice/trae-buddy-keeper";

/// 托盘菜单与设置页共用：重新导出，保持 tray.rs 的 `crate::open_project_repo_url()` 调用不变。
pub(crate) use commands::settings::open_project_repo_url;

/// 托盘/单实例/启动共用的主窗口呼出：还原最小化 → 主显示器居中 → 显示 → 聚焦。
/// 居中放在 show 之前，避免窗口先在旧位置闪一下再跳到屏幕中央。
pub(crate) fn show_main_window(app: &tauri::AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let _ = win.unminimize();
    center_on_primary_monitor(&win);
    let _ = win.show();
    let _ = win.set_focus();
}

/// 按主显示器居中（多显示器时固定回主屏，而不是停留在副屏边缘）。
/// 主显示器信息拿不到时退回窗口自身的 center。
fn center_on_primary_monitor(win: &tauri::WebviewWindow) {
    let Some(monitor) = win.primary_monitor().ok().flatten() else {
        let _ = win.center();
        return;
    };
    let Ok(size) = win.outer_size() else {
        return;
    };
    let origin = monitor.position();
    let area = monitor.size();
    // outer_size / position / size 均为物理像素，直接做整数运算
    let x = origin.x + (area.width as i32 - size.width as i32) / 2;
    let y = origin.y + (area.height as i32 - size.height as i32) / 2;
    let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 单实例插件必须最先注册：它在插件 setup 阶段（早于主窗口创建、托盘与定时任务
        // 初始化）就建立互斥量，第二实例在该阶段即静默退出，不会再执行 setup 中的任何初始化
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            // 自启动拉起的第二实例（开机自启与手动点击撞车）只让位、不抢焦点
            if argv.iter().any(|arg| arg == "--autostart") {
                return;
            }
            // 手动再次启动：恢复并聚焦已在运行的实例（含最小化到托盘/隐藏的情况），并重新居中
            show_main_window(app);
            // 启动参数转发给已有实例（前端可按需监听，不监听也不影响）
            let _ = app.emit("single-instance-args", argv);
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            // 自启动项带上标记参数，供单实例回调区分「手动启动」与「开机自启」
            Some(vec!["--autostart"]),
        ))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let app_dir = app.path().app_data_dir().expect("无法获取应用数据目录");
            std::fs::create_dir_all(&app_dir).ok();
            // 兼容旧版本注册的自启项（不含 --autostart）：自启处于开启状态时后台重写一次注册项，
            // 让后续自启动进程能被单实例回调识别为静默启动；用户在任务管理器里禁用自启时不动
            {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    use tauri_plugin_autostart::ManagerExt as _;
                    if matches!(handle.autolaunch().is_enabled(), Ok(true)) {
                        let _ = handle.autolaunch().enable();
                    }
                });
            }
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
                    show_main_window(app.handle());
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
            commands::accounts::list_accounts,
            commands::accounts::get_account_detail,
            commands::accounts::delete_account,
            commands::accounts::update_account,
            commands::accounts::switch_account,
            commands::checkin::checkin_one,
            commands::checkin::checkin_all,
            commands::checkin::get_checkin_records,
            commands::checkin::get_points_dashboard,
            commands::checkin::get_checkin_settings,
            commands::checkin::set_checkin_settings,
            commands::credits::refresh_credits,
            commands::credits::refresh_all_credits,
            commands::credits::get_credit_packages,
            commands::credits::get_credit_schedule,
            commands::credits::set_credit_schedule,
            commands::settings::get_notification_mode,
            commands::settings::set_notification_mode,
            commands::settings::get_logs,
            commands::settings::clear_logs,
            commands::settings::backup_data,
            commands::settings::import_data,
            commands::settings::get_app_settings,
            commands::settings::get_data_dir,
            commands::settings::open_data_dir,
            commands::settings::open_project_repo,
            commands::settings::set_app_settings,
            commands::settings::detect_client_paths,
            commands::switch::switch_workbuddy_client,
            commands::switch::switch_codebuddy_cn_client,
            commands::switch::switch_trae_client,
            commands::switch::trae_client_running,
            commands::switch::backup_trae_vault,
            commands::switch::trae_vault_info,
            commands::switch::switch_qoder_client,
            commands::switch::qoder_client_running,
            commands::switch::backup_qoder_vault,
            commands::switch::qoder_vault_info,
            commands::switch::restore_qoder_last,
            commands::token::renew_token,
            commands::token::auto_renew_all,
            commands::token::get_token_statistics,
            crate::platform::trae::oauth::start_trae_oauth,
            crate::platform::trae::oauth::stop_trae_oauth,
            crate::platform::trae::oauth::complete_trae_oauth,
            crate::platform::workbuddy::login::start_workbuddy_login,
            crate::platform::workbuddy::login::poll_workbuddy_login,
            crate::platform::qoder_cn::login::start_qoder_login,
            crate::platform::qoder_cn::login::stop_qoder_login,
            crate::platform::qoder_cn::login::poll_qoder_login,
        ])
        .run(tauri::generate_context!())
        .expect("运行 Tauri 应用失败");
}
