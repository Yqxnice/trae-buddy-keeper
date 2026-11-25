//! Token 命令：凭证续期与 Token 用量统计。

use tauri::State;

use crate::AppState;

// ===== Token 续期 =====

/// 手动续期单个账号：不受 24h 到期窗口限制，仅有单账号 5 分钟限频。
#[tauri::command]
pub async fn renew_token(state: State<'_, AppState>, id: String) -> Result<String, String> {
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

/// 手动批量续期：不受自动巡检的 24h 到期窗口限制，任意账号随时可刷新。
/// 仍受 TokenManager 的单账号 5 分钟限频保护，避免误触与上游风控。
/// 结果用统一的「续期成功 / 续期失败」文案，便于前端准确统计成功与失败数。
#[tauri::command]
pub async fn auto_renew_all(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let accounts = state.db.list_accounts().map_err(|e| e.to_string())?;
    let mut msgs = Vec::new();
    for acc in accounts {
        match state.token.renew(&acc.id).await {
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
                msgs.push(format!("{}: 续期失败（{}）", acc.name, e));
            }
        }
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
    Ok(msgs)
}

// ===== Token 统计 =====

/// 扫描本地日志的 Token 用量统计（WorkBuddy 国内/国际版、CodeBuddy CLI、CodeBuddy IDE）。
///
/// 纯磁盘扫描，不放异步运行时：用 `spawn_blocking` 避免阻塞主线程。
/// `days` 只接受 7/30/90，其它值或不传视为全量。
/// 另含 TRAE 云端用量源（第四个 source，按 TRAE 账号拉取服务端接口）。
#[tauri::command]
pub async fn get_token_statistics(
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
                let cached_at = entry.get("cached_at").and_then(|v| v.as_i64()).unwrap_or(0);
                if chrono::Utc::now().timestamp() - cached_at < TTL_SECS {
                    if let Some(stats) = entry.get("stats") {
                        return Ok(stats.clone());
                    }
                }
            }
        }
    }

    let mut stats = tauri::async_runtime::spawn_blocking(move || {
        crate::platform::token_stats::get_statistics(days)
    })
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
