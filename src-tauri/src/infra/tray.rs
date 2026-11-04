use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter,
};

pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
    let checkin = MenuItem::with_id(app, "checkin", "一键签到", true, None::<&str>)?;
    // 项目地址排在菜单末尾，并以分隔线与「退出」隔开
    let repo = MenuItem::with_id(app, "repo", "项目地址", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &checkin, &repo, &sep, &quit])?;

    let _tray = TrayIconBuilder::with_id("main-tray")
        .tooltip("TraeBuddy Keeper")
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                crate::show_main_window(app);
            }
            "checkin" => {
                let _ = app.emit("tray-checkin", ());
            }
            "repo" => {
                // 与设置页「打开仓库」同一逻辑；失败不静默，写 stderr
                if let Err(e) = crate::open_project_repo_url() {
                    eprintln!("托盘打开项目地址失败: {e}");
                }
            }
            "quit" => {
                app.exit(0);
            }
            _ => {}
        })
        // 左键单击保持无动作（菜单仅右键弹出），因此双击不会与单击竞争：
        // 直接使用 Windows 原生 WM_LBUTTONDBLCLK 映射出的 DoubleClick 事件，无需自建计时判定
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } = event
            {
                crate::show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}
