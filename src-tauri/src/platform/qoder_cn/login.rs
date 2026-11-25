//! Qoder CN 网页授权：PKCE 设备流，前端持有 nonce 轮询、后端自持 verifier。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use chrono::Utc;
use serde::Serialize;
use tauri::State;
use uuid::Uuid;

use super::api::{fetch_userinfo, new_device_flow, parse_poll, poll_once, DeviceFlow};
use crate::data::models::{Account, CheckinStatus};
use crate::AppState;

/// 进行中的授权会话。verifier 不下发前端，避免凭证经渲染进程传递。
fn flows() -> &'static Mutex<HashMap<String, DeviceFlow>> {
    static FLOWS: OnceLock<Mutex<HashMap<String, DeviceFlow>>> = OnceLock::new();
    FLOWS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QoderLoginStart {
    pub state: String,
    pub auth_url: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QoderLoginPoll {
    /// waiting：用户尚未完成登录；ready：已成功并入库
    pub status: String,
    pub name: String,
}

/// 第一步：生成授权会话并在系统浏览器打开登录页。
/// compat=true 时省略 client_id，用于授权页提示「参数无效」的兼容路径。
#[tauri::command]
pub async fn start_qoder_login(compat: Option<bool>) -> Result<QoderLoginStart, String> {
    let flow = new_device_flow(!compat.unwrap_or(false));
    let auth_url = flow.auth_url.clone();
    let state = flow.nonce.clone();
    flows()
        .lock()
        .map_err(|e| format!("授权会话锁异常: {e}"))?
        .insert(state.clone(), flow);
    open::that_detached(&auth_url).map_err(|e| format!("打开浏览器失败: {e}"))?;
    Ok(QoderLoginStart { state, auth_url })
}

/// 取消授权：清掉内存中的会话，不产生任何副作用。
#[tauri::command]
pub fn stop_qoder_login(login_state: String) {
    if let Ok(mut map) = flows().lock() {
        map.remove(&login_state);
    }
}

/// 第二步：轮询授权结果；成功后回填昵称并直接入库。
#[tauri::command]
pub async fn poll_qoder_login(
    app_state: State<'_, AppState>,
    login_state: String,
) -> Result<QoderLoginPoll, String> {
    let flow = flows()
        .lock()
        .map_err(|e| format!("授权会话锁异常: {e}"))?
        .get(&login_state)
        .map(|f| DeviceFlow {
            nonce: f.nonce.clone(),
            verifier: f.verifier.clone(),
            auth_url: f.auth_url.clone(),
            machine_id: f.machine_id.clone(),
        })
        .ok_or_else(|| "授权会话已过期，请重新发起登录".to_string())?;
    let waiting = || QoderLoginPoll {
        status: "waiting".to_string(),
        name: String::new(),
    };

    let (status, body) = poll_once(&flow).await?;
    // 404 是官方约定的「等待授权」响应，不是错误
    if status == 404 || status == 0 {
        return Ok(waiting());
    }
    let Some(body) = body else {
        return Ok(waiting());
    };
    if status != 200 {
        return Ok(waiting());
    }
    let nonce_matches = body
        .get("nonce")
        .and_then(|v| v.as_str())
        .map(|n| n == flow.nonce)
        .unwrap_or(false);
    let Some(tokens) = parse_poll(&body, &flow.nonce) else {
        if nonce_matches {
            return Err("授权响应缺少凭证，请重新发起登录".to_string());
        }
        return Err("授权响应与本次会话不匹配，已拒绝入库".to_string());
    };

    let profile = fetch_userinfo(&tokens.access_token, &flow.machine_id).await;
    let uid = if tokens.uid.is_empty() {
        profile.uid.clone()
    } else {
        tokens.uid.clone()
    };
    let now = Utc::now().timestamp();
    let name = if !profile.nickname.is_empty() {
        profile.nickname
    } else if !uid.is_empty() {
        format!("qoder-{}", &uid[..uid.len().min(8)])
    } else {
        format!("qoder-{}", now % 10000)
    };
    let acc = Account {
        id: Uuid::new_v4().to_string(),
        platform: "qoder_cn".to_string(),
        name: name.clone(),
        email: String::new(),
        avatar: String::new(),
        uid,
        domain: "qoder.cn".to_string(),
        enterprise_id: String::new(),
        account_type: String::new(),
        device_id: String::new(),
        device_key: String::new(),
        // 授权页里带去服务端的设备号才是该凭证信任分桶的真值，不能用 nonce 代替
        machine_id: flow.machine_id.clone(),
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        cookies: String::new(),
        token_expires_at: tokens.expires_at,
        refresh_token_expires_at: tokens.refresh_expires_at,
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
    app_state
        .db
        .insert_account(&acc)
        .map_err(|e| e.to_string())?;
    if let Ok(mut map) = flows().lock() {
        map.remove(&login_state);
    }
    app_state
        .db
        .log("success", "登录", &format!("Qoder CN 网页授权成功：{name}"));
    // 授权前可能已在官方客户端领过活动：回查真实状态，避免新账号误显示未签到
    app_state.checkin.sync_checkin_status(&acc).await;
    Ok(QoderLoginPoll {
        status: "ready".to_string(),
        name,
    })
}
