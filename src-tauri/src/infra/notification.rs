use std::sync::Arc;
use tauri::{AppHandle, Emitter};
#[cfg(not(windows))]
use tauri_plugin_notification::NotificationExt;

use crate::data::models::NotificationSettings;

pub struct Notifier {
    app: AppHandle,
    mode: Arc<tokio::sync::Mutex<String>>,
}

impl Notifier {
    pub fn new(app: AppHandle, mode: String) -> Self {
        Notifier {
            app,
            mode: Arc::new(tokio::sync::Mutex::new(mode)),
        }
    }

    pub async fn set_mode(&self, mode: &str) {
        let mut m = self.mode.lock().await;
        *m = mode.to_string();
    }

    pub async fn get_mode(&self) -> String {
        self.mode.lock().await.clone()
    }

    pub async fn notify(&self, title: &str, body: &str) {
        let mode = self.get_mode().await;
        match mode.as_str() {
            "none" => {}
            "toast" => {
                self.send_toast(title, body).await;
            }
            "system" => {
                self.send_system(title, body);
            }
            "both" => {
                self.send_toast(title, body).await;
                self.send_system(title, body);
            }
            _ => {
                self.send_toast(title, body).await;
            }
        }
    }

    /// 签到数据变更信号（定时签到/超时补签完成）：
    /// 与通知模式无关，始终发前端事件驱动总览等页面刷新
    pub fn emit_checkin_done(&self) {
        let _ = self.app.emit("checkin-done", ());
    }

    async fn send_toast(&self, title: &str, body: &str) {
        let payload = serde_json::json!({
            "title": title,
            "body": body,
            "ts": chrono::Utc::now().timestamp(),
        });
        let _ = self.app.emit("app-toast", payload);
    }

    #[cfg(not(windows))]
    fn send_system(&self, title: &str, body: &str) {
        let _ = self
            .app
            .notification()
            .builder()
            .title(title)
            .body(body)
            .show();
    }

    /// Windows 下自带 AppUserModelID 发送 toast。
    /// 官方插件在开发/绿色包模式下会退化成 PowerShell 身份（或直接不显示），
    /// 这里通过注册表登记 AUMID 后由 notify-rust 直接发送，保证开发环境也能弹出。
    #[cfg(windows)]
    fn send_system(&self, title: &str, body: &str) {
        let app_id = self.app.config().identifier.clone();
        let title = title.to_string();
        let body = body.to_string();
        tauri::async_runtime::spawn_blocking(move || {
            if let Err(e) = notify_rust::Notification::new()
                .app_id(&app_id)
                .summary(&title)
                .body(&body)
                .show()
            {
                eprintln!("系统通知发送失败: {}", e);
            }
        });
    }
}

/// 在 Windows 注册应用的 AppUserModelID（写入 HKCU，无需安装包/开始菜单快捷方式），
/// 否则未安装的桌面应用无法弹出系统 toast。
#[cfg(windows)]
pub fn register_windows_app_id(app_id: &str, display_name: &str) {
    use std::os::windows::process::CommandExt;
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;

    let wide: Vec<u16> = app_id.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let _ = SetCurrentProcessExplicitAppUserModelID(PCWSTR(wide.as_ptr()));
    }

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let key = format!("HKCU\\Software\\Classes\\AppUserModelId\\{}", app_id);
    let _ = std::process::Command::new("reg")
        .args([
            "add",
            &key,
            "/v",
            "DisplayName",
            "/t",
            "REG_SZ",
            "/d",
            display_name,
            "/f",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}

pub fn load_notification_settings() -> NotificationSettings {
    NotificationSettings {
        mode: "both".to_string(),
    }
}
