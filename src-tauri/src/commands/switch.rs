//! 客户端切换命令：登录态注入、Trae / Qoder CN 档案切换与人工回退。

use tauri::State;

use crate::data::db::Database;
use crate::AppState;

/// WorkBuddy 客户端真实切换：写官方认证文件并重启客户端
#[tauri::command]
pub async fn switch_workbuddy_client(
    state: State<'_, AppState>,
    id: String,
) -> Result<String, String> {
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
pub async fn switch_codebuddy_cn_client(
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
pub async fn backup_trae_vault(
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
    let acc_id = acc.id.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        crate::platform::trae::switch::backup_vault(&data_dir, &id, &image, close_running)
    })
    .await
    .map_err(|e| format!("建档任务执行失败: {e}"))?;
    match &result {
        Ok(msg) => {
            db.log("success", "建档", &format!("Trae 账号「{name}」{msg}"));
            // 建档的前提就是「客户端当前登录着这个账号」，因此该账号就是客户端里的现场，
            // 顺手校正「当前已注入」标记，后续切换才能把出站 live 安全回写到正确档案。
            crate::platform::trae::switch::mark_active_account(&db, &acc_id);
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

/// 查询 Trae 账号建档信息（含「待验证」标记）；未建档返回 None
#[tauri::command]
pub fn trae_vault_info(
    state: State<AppState>,
    id: String,
) -> Option<crate::platform::trae::switch::VaultInfo> {
    crate::platform::trae::switch::vault_info(&state.data_dir, &id)
}

/// Trae 客户端当前是否在运行（切换前先检测，运行中切库会写花客户端数据）
#[tauri::command]
pub fn trae_client_running(state: State<AppState>) -> bool {
    let exe = resolve_trae_exe(&state.db);
    crate::platform::trae::switch::is_running_for_exe(&exe)
}

/// Trae 客户端真实切换：整目录快照冷切换（含守护与回滚）
#[tauri::command]
pub async fn switch_trae_client(state: State<'_, AppState>, id: String) -> Result<String, String> {
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
    // 出站账号：优先认客户端日志里实际登录的账号（手动改登会让标记漂移），
    // 其次才是「当前已注入客户端」标记。切换前会把 live 最新登录态回写这个档案
    // （令牌在会话期间可能已轮换）。
    let current_account_id = {
        let accounts = state.db.list_accounts().unwrap_or_default();
        crate::platform::trae::switch::correct_active_marker(&state.db, &accounts)
            .or_else(|| state.db.get_setting("active_client_trae").ok().flatten())
            .filter(|v| !v.is_empty() && v != &id)
    };
    let current_account_name = current_account_id
        .as_ref()
        .and_then(|cid| state.db.get_account(cid).ok().flatten())
        .map(|a| a.name);
    // 先检测客户端：运行中直接改文件会写花它的数据，必须等它完全退出再切
    if crate::platform::trae::switch::is_running_for_exe(&exe) {
        state.db.log(
            "info",
            "切换",
            &format!("检测到 Trae CN 正在运行，先关闭客户端再切换到「{}」", name),
        );
    }
    // 切换前用数据库里的 access_token 做一次服务端会话预检（fail-open，最多 5 秒）：
    // 只有明确 401/1001 才附加警示，网络异常一律放行；判死也照常切换，因为
    // 「切进该账号重新登录」正是会话被吊销时唯一的恢复手段。
    let precheck_warning = crate::platform::trae::api::probe_session(&acc)
        .await
        .warning();
    // 关进程/拷贝载体/守护轮询都是阻塞操作，放阻塞线程池
    let result = tauri::async_runtime::spawn_blocking(move || {
        crate::platform::trae::switch::switch_to_vault(
            &data_dir,
            &acc_id,
            &exe,
            current_account_id.as_deref(),
            current_account_name.as_deref(),
        )
    })
    .await
    .map_err(|e| format!("切换任务执行失败: {e}"))?;
    let with_precheck = |mut text: String| {
        if let Some(w) = &precheck_warning {
            text.push_str("\n\n");
            text.push_str(w);
        }
        text
    };
    match &result {
        Ok(msg) => {
            let warned = msg.starts_with("SWITCH_WARN:");
            // 把守护返回的细节（采样统计 / 状态不稳定提示）与预检警示一并写进系统日志
            let out = with_precheck(msg.clone());
            db.log(
                if warned { "warning" } else { "success" },
                "切换",
                &format!("Trae 客户端切换到「{name}」：{out}"),
            );
            let _ = db.set_setting("active_account", &id);
            // 未确认登录时不要把「当前已注入客户端」标记指向目标账号，
            // 否则列表会高亮一个客户端里实际没登进去的账号。
            if !warned {
                crate::platform::trae::switch::mark_active_account(&db, &id);
            }
            Ok(out)
        }
        Err(e) => {
            let out = with_precheck(e.clone());
            db.log(
                "error",
                "切换",
                &format!("Trae 切换到「{name}」失败：{out}"),
            );
            Err(out)
        }
    }
}

// ===== Qoder CN 客户端切换 =====

/// Qoder CN 客户端当前是否在运行（建档前必须确认已完全退出）
#[tauri::command]
pub fn qoder_client_running(state: State<AppState>) -> bool {
    let exe = crate::platform::qoder_cn::switch::resolve_exe(&state.db);
    crate::platform::qoder_cn::switch::is_running_for_exe(&exe)
}

/// 查询 Qoder CN 账号建档信息；未建档返回 None
#[tauri::command]
pub fn qoder_vault_info(
    state: State<AppState>,
    id: String,
) -> Option<crate::platform::trae::switch::VaultInfo> {
    crate::platform::qoder_cn::switch::vault_info(&state.data_dir, &id)
}

/// 建档：把当前客户端登录现场整份快照到账号目录
#[tauri::command]
pub async fn backup_qoder_vault(
    state: State<'_, AppState>,
    id: String,
    close_running: bool,
) -> Result<String, String> {
    let acc = state
        .db
        .get_account(&id)
        .map_err(|e| e.to_string())?
        .ok_or("账户不存在")?;
    if acc.platform != "qoder_cn" {
        return Err("仅 Qoder CN 账户需要建档".to_string());
    }
    let exe = crate::platform::qoder_cn::switch::resolve_exe(&state.db);
    let data_dir = state.data_dir.clone();
    let db = state.db.clone();
    let name = acc.name.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        crate::platform::qoder_cn::switch::backup_vault(&data_dir, &id, &exe, close_running)
    })
    .await
    .map_err(|e| format!("建档任务执行失败: {e}"))?;
    match &result {
        Ok(msg) => {
            db.log("success", "建档", &format!("Qoder CN 账号「{name}」{msg}"));
            Ok(msg.clone())
        }
        Err(e) => {
            db.log(
                "error",
                "建档",
                &format!("Qoder CN 账号「{name}」建档失败：{e}"),
            );
            Err(e.clone())
        }
    }
}

/// 切换：关进程 → 归档当前现场 → 写回目标档案 → 拉起客户端 → 登录态核验
#[tauri::command]
pub async fn switch_qoder_client(state: State<'_, AppState>, id: String) -> Result<String, String> {
    let acc = state
        .db
        .get_account(&id)
        .map_err(|e| e.to_string())?
        .ok_or("账户不存在")?;
    if acc.platform != "qoder_cn" {
        return Err("仅 Qoder CN 账户支持客户端切换".to_string());
    }
    let exe = crate::platform::qoder_cn::switch::resolve_exe(&state.db);
    let data_dir = state.data_dir.clone();
    let db = state.db.clone();
    let name = acc.name.clone();
    // 切换前的现场归属：只认「当前已注入客户端」的标记，避免把别的账号档案覆盖掉。
    let current_account_id = db
        .get_setting("active_client_qoder_cn")
        .ok()
        .flatten()
        .filter(|v| !v.is_empty() && v != &id);
    let other_uids = db
        .list_accounts()
        .unwrap_or_default()
        .into_iter()
        .filter(|a| a.platform == "qoder_cn" && a.id != id && !a.uid.is_empty())
        .map(|a| a.uid)
        .collect();
    let ctx = crate::platform::qoder_cn::switch::SwitchContext {
        current_account_id,
        target_uid: acc.uid.clone(),
        other_uids,
    };
    let result = tauri::async_runtime::spawn_blocking(move || {
        crate::platform::qoder_cn::switch::switch_to_vault(&data_dir, &id, &exe, &ctx)
    })
    .await
    .map_err(|e| format!("切换任务执行失败: {e}"))?;
    match &result {
        Ok(msg) => {
            // 未确认登录时不把「当前已注入客户端」标记指向目标账号，
            // 否则列表会高亮一个客户端里实际没登进去的账号。
            let warned = msg.starts_with("SWITCH_WARN:");
            db.log(
                if warned { "warning" } else { "success" },
                "切换",
                &format!("Qoder CN 客户端切换到「{name}」：{msg}"),
            );
            let _ = db.set_setting("active_account", &acc.id);
            if !warned {
                let _ = db.set_setting("active_client_qoder_cn", &acc.id);
            }
            Ok(msg.clone())
        }
        Err(e) => {
            db.log(
                "error",
                "切换",
                &format!("Qoder CN 切换到「{name}」失败：{e}"),
            );
            Err(e.clone())
        }
    }
}

/// 人工兜底：把上一次切换前的现场（last 槽）恢复到客户端。
#[tauri::command]
pub async fn restore_qoder_last(state: State<'_, AppState>) -> Result<String, String> {
    let exe = crate::platform::qoder_cn::switch::resolve_exe(&state.db);
    let data_dir = state.data_dir.clone();
    let db = state.db.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        crate::platform::qoder_cn::switch::restore_last_vault(&data_dir, &exe)
    })
    .await
    .map_err(|e| format!("恢复任务执行失败: {e}"))?;
    match &result {
        Ok(msg) => {
            db.log("success", "恢复", &format!("Qoder CN {msg}"));
            Ok(msg.clone())
        }
        Err(e) => {
            db.log(
                "error",
                "恢复",
                &format!("Qoder CN 恢复切换前现场失败：{e}"),
            );
            Err(e.clone())
        }
    }
}
