//! 账户命令：列表、详情、编辑、删除与活跃账户标记。

use tauri::State;

use crate::data::models::{Account, AccountInfo};
use crate::data::vault_cleanup::cleanup_account_vaults;
use crate::AppState;

#[tauri::command]
pub fn list_accounts(state: State<AppState>) -> Result<Vec<AccountInfo>, String> {
    let accounts = state.db.list_accounts().map_err(|e| e.to_string())?;
    // 用户在 Trae 里手动退出再登录别的账号时，「当前已注入」标记会漂移
    // （界面显示 A、客户端实际是 B）。以客户端日志里的登录用户名为准校正一次。
    let _ = crate::platform::trae::switch::correct_active_marker(&state.db, &accounts);
    let mut infos: Vec<AccountInfo> = accounts.into_iter().map(AccountInfo::from).collect();
    // 每个客户端当前注入的账户 id → 前端在卡片头像上渲染客户端品牌角标
    for (key, client) in [
        ("active_client_trae", "trae"),
        ("active_client_workbuddy", "workbuddy"),
        ("active_client_codebuddy_cn", "codebuddy_cn"),
        ("active_client_qoder_cn", "qoder_cn"),
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
pub fn get_account_detail(state: State<AppState>, id: String) -> Result<Option<Account>, String> {
    state.db.get_account(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_account(
    state: State<AppState>,
    id: String,
    keep_vault_backup: bool,
) -> Result<String, String> {
    let account = state.db.get_account(&id).ok().flatten();
    let name = account.as_ref().map(|a| a.name.clone()).unwrap_or_default();

    // 先清 vault 再删数据库行：清理失败就中止删除，避免留下「账号已删、档案成孤儿」。
    let notes = cleanup_account_vaults(&state.data_dir, &id, keep_vault_backup)
        .map_err(|e| format!("清理登录档案失败，账户未删除：{e}"))?;

    state.db.delete_account(&id).map_err(|e| e.to_string())?;
    let detail = if notes.is_empty() {
        "无登录档案需要清理".to_string()
    } else {
        notes.join("；")
    };
    state
        .db
        .log("info", "账户", &format!("删除账户 {}（{detail}）", name));
    Ok(format!("账户已删除；{detail}"))
}

#[tauri::command]
pub fn update_account(
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

// ===== 账户切换 =====

#[tauri::command]
pub fn switch_account(state: State<AppState>, id: String) -> Result<String, String> {
    // 标记当前活跃账户（前端据此高亮）
    state
        .db
        .set_setting("active_account", &id)
        .map_err(|e| e.to_string())?;
    Ok(format!("已切换到账户 {}", id))
}
