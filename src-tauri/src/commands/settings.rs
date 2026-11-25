//! 通用设置命令：外观 / 通知 / 启动、客户端路径、系统日志、数据备份与恢复。

use tauri::State;

use crate::data::models::{AppLog, AppSettings};
use crate::AppState;
use crate::PROJECT_REPO_URL;

// ===== 系统日志 =====

#[tauri::command]
pub fn get_logs(state: State<AppState>, limit: Option<i64>) -> Result<Vec<AppLog>, String> {
    state
        .db
        .list_logs(limit.unwrap_or(200))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn clear_logs(state: State<AppState>) -> Result<(), String> {
    state.db.clear_logs().map_err(|e| e.to_string())
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
pub async fn backup_data(state: State<'_, AppState>) -> Result<Option<String>, String> {
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
pub async fn import_data(state: State<'_, AppState>) -> Result<Option<String>, String> {
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
        let mut note = "数据库已恢复，建议重启应用；若备份来自其他 Windows 用户/设备，账户凭证无法解密，需重新网页授权登录".to_string();
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
pub fn get_app_settings(state: State<AppState>) -> Result<AppSettings, String> {
    let mut settings = state.db.load_app_settings();
    // 历史误配自愈：Qoder CN 路径曾被自动探测填成其它 Qoder 产品线的 exe，
    // 打开设置时用一次探测覆盖，避免切换/进程检测继续指向错误客户端
    let configured = settings.client_path_qoder_cn.trim();
    if !configured.is_empty() && !crate::platform::qoder_cn::switch::is_qoder_cn_exe(configured) {
        settings.client_path_qoder_cn = crate::infra::clients::detect_client_paths().qoder_cn;
        let json = serde_json::to_string(&settings).map_err(|e| e.to_string())?;
        state
            .db
            .set_setting("app_settings", &json)
            .map_err(|e| e.to_string())?;
    }
    Ok(settings)
}

#[tauri::command]
pub fn get_data_dir(state: State<AppState>) -> String {
    state.data_dir.to_string_lossy().to_string()
}

#[tauri::command]
pub fn open_data_dir(state: State<AppState>) -> Result<(), String> {
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

/// 打开项目仓库（设置页命令与托盘菜单共用同一份逻辑与地址）
pub(crate) fn open_project_repo_url() -> Result<(), String> {
    open::that_detached(PROJECT_REPO_URL).map_err(|e| format!("打开仓库链接失败: {e}"))
}

#[tauri::command]
pub fn open_project_repo() -> Result<(), String> {
    open_project_repo_url()
}

#[tauri::command]
pub fn set_app_settings(state: State<AppState>, settings: AppSettings) -> Result<(), String> {
    let json = serde_json::to_string(&settings).map_err(|e| e.to_string())?;
    state
        .db
        .set_setting("app_settings", &json)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn detect_client_paths() -> crate::infra::clients::ClientPaths {
    crate::infra::clients::detect_client_paths()
}

#[tauri::command]
pub async fn get_notification_mode(state: State<'_, AppState>) -> Result<String, String> {
    Ok(state.notifier.get_mode().await)
}

#[tauri::command]
pub async fn set_notification_mode(state: State<'_, AppState>, mode: String) -> Result<(), String> {
    state.notifier.set_mode(&mode).await;
    state
        .db
        .set_setting("notification_mode", &mode)
        .map_err(|e| e.to_string())
}
