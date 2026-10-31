use anyhow::{anyhow, Result};
use reqwest::Client;
use serde_json::Value;

use crate::data::models::Account;

const WB_BASE: &str = "https://www.codebuddy.cn";

pub fn cookie_header(cookies_json: &str) -> Result<String> {
    // cookies 存为 JSON 数组: [{name, value}, ...]
    let arr: Value = serde_json::from_str(cookies_json)
        .map_err(|e| anyhow!("cookie 解析失败: {}", e))?;
    let cookies = arr
        .as_array()
        .ok_or_else(|| anyhow!("cookie 格式非数组"))?;
    let parts: Vec<String> = cookies
        .iter()
        .filter_map(|c| {
            let name = c.get("name").and_then(|n| n.as_str())?;
            let value = c.get("value").and_then(|v| v.as_str())?;
            Some(format!("{}={}", name, value))
        })
        .collect();
    Ok(parts.join("; "))
}

pub async fn do_checkin(acc: &Account) -> Result<(bool, i64, String)> {
    let client = Client::new();
    // 浏览器登录拿到的 token 账号（无 cookie）：走 /v2 + Bearer 通道；
    // 手动粘贴 cookie 的账号：走网页 Cookie 通道
    let token_mode = acc.cookies.trim().is_empty() && !acc.access_token.trim().is_empty();
    let req = if token_mode {
        client
            .post(format!("{}/v2/billing/meter/daily-checkin", WB_BASE))
            .header("Content-Type", "application/json")
            .bearer_auth(acc.access_token.trim())
            .header("Origin", WB_BASE)
            .header("Referer", &format!("{}/", WB_BASE))
            .body("{}")
    } else {
        let cookie = cookie_header(&acc.cookies)?;
        if cookie.is_empty() {
            return Ok((false, 0, "无可用 cookie".to_string()));
        }
        client
            .post(format!("{}/billing/meter/daily-checkin", WB_BASE))
            .header("Content-Type", "application/json")
            .header("Cookie", &cookie)
            .header("Origin", WB_BASE)
            .header("Referer", &format!("{}/profile/usage", WB_BASE))
            .body("{}")
    };
    let resp = req.send().await?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Ok((false, 0, "凭证已过期(401)，请重新登录".to_string()));
    }
    let v: Value = serde_json::from_str(&text).unwrap_or_default();
    let msg = v
        .get("msg")
        .or_else(|| v.get("message"))
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_string();
    // 「今天已签到」以 HTTP 4xx + code=10001 返回，属正常幂等，视为成功
    if msg.contains("已签到") || msg.contains("已领取") {
        return Ok((true, 0, msg));
    }
    if !status.is_success() {
        return Ok((false, 0, format!("签到请求失败: {} {}", status, text)));
    }
    let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
    if code != 0 {
        return Ok((false, 0, format!("签到失败: {}", msg)));
    }
    let points = 100; // WorkBuddy 签到固定 100 积分
    Ok((true, points, format!("签到成功 +{}", points)))
}

pub async fn verify_cookies(cookies_json: &str) -> Result<bool> {
    let client = Client::new();
    let cookie = cookie_header(cookies_json)?;
    let url = format!("{}/billing/meter/check-gift-claimed", WB_BASE);
    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Cookie", &cookie)
        .header("Origin", WB_BASE)
        .header("Referer", &format!("{}/profile/usage", WB_BASE))
        .body("{}")
        .send()
        .await?;
    Ok(resp.status().is_success())
}

/// 用 refreshToken 换新 accessToken（对齐参考项目 issue #40 口径）：
/// POST {chat}/v2/plugin/auth/token/refresh，refreshToken 走 X-Refresh-Token 头，
/// 带 X-Auth-Refresh-Source: plugin；响应可能是裸 {accessToken,...} 或包一层 data。
/// expiresIn 缺省时沿用旧到期时间（与上游 preserveExpiry 同口径）。
/// 返回 (new_access, new_refresh, expires_at)。
pub async fn refresh_access_token(acc: &Account) -> Result<(String, String, i64)> {
    let refresh = acc.refresh_token.trim();
    if refresh.is_empty() {
        return Err(anyhow!("该账号没有 refreshToken，只能重新扫码或登录"));
    }
    let client = Client::new();
    // 与登录同 host（www.codebuddy.cn）：platform=workbuddy 的 token 用 copilot.tencent.com
    // 刷新可能拿不到同档位会话，刷新端点与登录接口保持同口径
    let resp = client
        .post("https://www.codebuddy.cn/v2/plugin/auth/token/refresh")
        .header("Accept", "application/json, text/plain, */*")
        .header("X-Requested-With", "XMLHttpRequest")
        .header("User-Agent", "CLI/2.63.2 CodeBuddy/2.63.2")
        .header("Origin", "https://www.codebuddy.cn")
        .header("Referer", "https://www.codebuddy.cn/")
        // 刷新带的是旧 accessToken（上游同样用刷新前的值构造头）
        .bearer_auth(acc.access_token.trim())
        .header("X-Refresh-Token", refresh)
        .header("X-Auth-Refresh-Source", "plugin")
        .send()
        .await
        .map_err(|e| anyhow!("刷新请求失败: {}", e))?;

    if resp.status().as_u16() >= 400 {
        return Err(anyhow!(
            "刷新失败（HTTP {}）——若反复失败说明登录态已失效，需重新扫码或登录",
            resp.status()
        ));
    }
    let v: Value = resp.json().await.map_err(|e| anyhow!("刷新响应不是 JSON: {}", e))?;
    let body = match v.get("data") {
        Some(d) if d.is_object() => d.clone(),
        _ => v.clone(),
    };
    let new_access = body
        .get("accessToken")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    if new_access.is_empty() {
        return Err(anyhow!("刷新响应里没有 accessToken —— 需重新扫码或登录"));
    }
    let new_refresh = body
        .get("refreshToken")
        .and_then(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(refresh)
        .to_string();
    // expiresIn 缺省时保留旧到期时间，不乱写
    let expires_at = match body.get("expiresIn").and_then(|e| e.as_i64()) {
        Some(secs) if secs > 0 && secs < 10 * 365 * 86400 => {
            chrono::Utc::now().timestamp() + secs
        }
        _ => acc.token_expires_at,
    };
    Ok((new_access, new_refresh, expires_at))
}
