//! WorkBuddy 登录：腾讯插件授权协议（state 轮询模式）。
//!
//! 实现参考了开源项目 workbuddy-manager、workbuddy-switch。
//!
//! 流程：`POST /v2/plugin/auth/state` 获取 state 与授权链接 → 呼出系统浏览器打开
//! 授权页（扫码或账号登录）→ 前端轮询 `poll_workbuddy_login` →
//! `GET /v2/plugin/auth/token?state=` 返回 accessToken/refreshToken →
//! 再查 `GET /v2/plugin/login/account?state=` 取昵称入库。
//! 协议细节：统一信封 {code, msg, data}，code != 0 即业务未完成（含等待登录）。

use chrono::Utc;
use serde::Serialize;
use serde_json::Value;
use tauri::State;
use uuid::Uuid;

use crate::data::models::{Account, CheckinStatus};
use crate::AppState;

/// 登录接口基址（国内版；platform=workbuddy 换取桌面端会话 token，
/// 用 platform=CLI 拿到的 token 只能调 billing/签到接口，写进桌面客户端会被拒登）
const CHAT_BASE: &str = "https://www.codebuddy.cn";
const UA: &str = "CLI/2.63.2 CodeBuddy/2.63.2";
const ORIGIN: &str = "https://www.codebuddy.cn";

/// 时间戳归一化为秒：官方 expiresAt 字段可能给毫秒
fn norm_ts_secs(v: Option<&Value>) -> Option<i64> {
    let n = v?.as_i64()?;
    Some(if n > 1_000_000_000_000 { n / 1000 } else { n })
}

fn http() -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("Accept", "application/json, text/plain, */*".parse().unwrap());
    headers.insert("X-Requested-With", "XMLHttpRequest".parse().unwrap());
    headers.insert("User-Agent", UA.parse().unwrap());
    headers.insert("Origin", ORIGIN.parse().unwrap());
    headers.insert("Referer", format!("{ORIGIN}/").parse().unwrap());
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap_or_default()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbuddyLoginStart {
    pub state: String,
    pub auth_url: String,
}

/// 第一步：获取登录 state + 授权链接，并呼出系统浏览器
#[tauri::command]
pub async fn start_workbuddy_login() -> Result<WorkbuddyLoginStart, String> {
    let resp = http()
        .post(format!(
            "{CHAT_BASE}/v2/plugin/auth/state?platform=workbuddy"
        ))
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|e| format!("连不上腾讯登录接口: {e}（请检查网络/代理）"))?;
    let v: Value = resp.json().await.map_err(|e| format!("响应解析失败: {e}"))?;
    let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
    let data = v.get("data").cloned().unwrap_or(Value::Null);
    if code != 0 {
        let msg = v.get("msg").and_then(|m| m.as_str()).unwrap_or("");
        return Err(format!("获取授权链接失败 code={code} {msg}"));
    }
    let state = data
        .get("state")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let auth_url = data
        .get("authUrl")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    if state.is_empty() || auth_url.is_empty() {
        return Err("授权链接响应缺少 state/authUrl".to_string());
    }
    open::that_detached(&auth_url).map_err(|e| format!("打开浏览器失败: {e}"))?;
    Ok(WorkbuddyLoginStart { state, auth_url })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkbuddyLoginPoll {
    /// waiting：用户尚未完成登录；ready：已成功并入库
    pub status: String,
    pub name: String,
}

/// 第二步：轮询登录结果；成功后直接入库并返回 ready
#[tauri::command]
pub async fn poll_workbuddy_login(
    app_state: State<'_, AppState>,
    login_state: String,
) -> Result<WorkbuddyLoginPoll, String> {
    let waiting = || WorkbuddyLoginPoll {
        status: "waiting".to_string(),
        name: String::new(),
    };

    let resp = http()
        .get(format!("{CHAT_BASE}/v2/plugin/auth/token"))
        .query(&[("state", login_state.as_str())])
        .send()
        .await
        .map_err(|e| format!("连不上腾讯登录接口: {e}"))?;
    let v: Value = resp.json().await.map_err(|e| format!("响应解析失败: {e}"))?;
    let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
    let data = v.get("data").cloned().unwrap_or(Value::Null);
    let access_token = data
        .get("accessToken")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    // code != 0 即用户还没扫码/确认，继续等
    if code != 0 || access_token.is_empty() {
        return Ok(waiting());
    }
    let refresh_token = data
        .get("refreshToken")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let now = Utc::now().timestamp();
    let expires_at = norm_ts_secs(data.get("expiresAt").or_else(|| data.get("expires_at")))
        .unwrap_or_else(|| {
            now + data
                .get("expiresIn")
                .and_then(|s| s.as_i64())
                .unwrap_or(3600)
        });
    let refresh_expires_at =
        norm_ts_secs(data.get("refreshExpiresAt").or_else(|| data.get("refresh_expires_at")))
            .unwrap_or(now + 30 * 86400);

    // 档位校验：响应 domain 必须是国内版产品域，否则写进客户端会一直登录失效
    let domain = data
        .get("domain")
        .and_then(|d| d.as_str())
        .unwrap_or("")
        .to_string();
    if !domain.is_empty() && domain != "www.codebuddy.cn" && domain != "www.workbuddy.cn" {
        return Err(format!(
            "登录响应的 domain（{domain}）不是国内版，已拒绝入库"
        ));
    }

    // 第三步：取账号信息（昵称 / uid）
    let acct = http()
        .get(format!("{CHAT_BASE}/v2/plugin/login/account"))
        .query(&[("state", login_state.as_str())])
        .bearer_auth(&access_token)
        .send()
        .await
        .map_err(|e| format!("获取账号信息失败: {e}"))?;
    let av: Value = acct.json().await.map_err(|e| format!("账号信息解析失败: {e}"))?;
    let adata = av.get("data").cloned().unwrap_or(Value::Null);
    let uid = adata
        .get("uid")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let nickname = adata
        .get("nickname")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    // 头像：接口可能返回 avatar / avatarUrl / avatar_url / portrait 等键
    let avatar = ["avatar", "avatarUrl", "avatar_url", "portrait", "headImageUrl"]
        .iter()
        .filter_map(|k| adata.get(*k).and_then(|s| s.as_str()))
        .find(|s| !s.is_empty())
        .unwrap_or("")
        .to_string();
    // 企业账号字段：积分查询按 enterpriseId / type=ultimate|exclusive 分流企业接口
    let enterprise_id = adata
        .get("enterpriseId")
        .or_else(|| adata.get("enterprise_id"))
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let account_type = adata
        .get("type")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();

    let name = if !nickname.is_empty() {
        nickname
    } else if !uid.is_empty() {
        format!("wb-{uid}")
    } else {
        format!("workbuddy-{}", now % 10000)
    };
    let acc = Account {
        id: Uuid::new_v4().to_string(),
        platform: "workbuddy".to_string(),
        name: name.clone(),
        email: String::new(),
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
        cookies: String::new(),
        token_expires_at: expires_at,
        refresh_token_expires_at: refresh_expires_at,
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
    app_state.db.insert_account(&acc).map_err(|e| e.to_string())?;
    app_state.db.log(
        "success",
        "登录",
        &format!("WorkBuddy 网页授权成功：{}", name),
    );
    Ok(WorkbuddyLoginPoll {
        status: "ready".to_string(),
        name,
    })
}
