//! TRAE 云端用量统计（数据源，供 token_stats 作为第四个 source 聚合）。
//!
//! 逆向自网页版 dashboard#usage 页面（参考 TraeTools 实现）：
//! `POST /trae/api/v1/pay/query_user_usage_group_by_session`，认证与网页业务
//! 接口一致（Cloud-IDE-JWT + Origin/Referer，无需设备头）。响应的
//! `user_usage_group_by_sessions[]` 是按会话聚合的用量，token 四项
//! （入 / 出 / 缓存读 / 缓存写）在 `extra_info` 中。

use anyhow::{anyhow, Result};
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;

use super::api::{authed, TRAE_BASE};

/// 每页条数与最大翻页数（与参考实现一致，上限 400 条会话防失控）。
const PAGE_SIZE: i64 = 20;
const MAX_PAGES: i64 = 20;

/// 一次会话的云端用量记录。
#[derive(Debug, Clone)]
pub struct UsageSession {
    pub session_id: String,
    /// 服务端 Unix 秒（会话产生时间）。
    pub usage_time: i64,
    pub model_name: String,
    /// Max / 空。
    pub mode: String,
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    /// 用户首条消息预览（会话展示标题）。
    pub preview: String,
}

impl UsageSession {
    /// 聚合成 token_stats 的合成记录：timestamp(ms) + 顶层 usage + providerData.model，
    /// 字段名对齐 token_stats 既有解码键，复用整套聚合管线。
    pub fn to_stats_record(&self) -> Value {
        let model = if self.mode.is_empty() {
            self.model_name.clone()
        } else {
            format!("{}（{}）", self.model_name, self.mode)
        };
        json!({
            "sessionId": self.session_id,
            "title": self.preview,
            "timestamp": self.usage_time.saturating_mul(1000),
            "providerData": { "model": model },
            "usage": {
                "input_tokens": self.input,
                "output_tokens": self.output,
                "cache_read_input_tokens": self.cache_read,
                "cache_write_input_tokens": self.cache_write,
            },
        })
    }
}

fn lenient_i64(v: &Value) -> i64 {
    v.as_i64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        .unwrap_or(0)
}

fn parse_session(el: &Value) -> UsageSession {
    let s = |k: &str| {
        el.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let extra = el.get("extra_info");
    let num = |k: &str| extra.map(|e| e.get(k).map(lenient_i64).unwrap_or(0)).unwrap_or(0);
    UsageSession {
        session_id: s("session_id"),
        usage_time: el.get("usage_time").map(lenient_i64).unwrap_or(0),
        model_name: s("model_name"),
        mode: s("mode"),
        input: num("input_token"),
        output: num("output_token"),
        cache_read: num("cache_read_token"),
        cache_write: num("cache_write_token"),
        preview: s("user_input_preview"),
    }
}

/// 拉取一页「按会话聚合的用量」，返回 (total, 本页会话)。
async fn fetch_page(
    client: &Client,
    access_token: &str,
    start: i64,
    end: i64,
    page: i64,
) -> Result<(i64, Vec<UsageSession>)> {
    let url = format!("{TRAE_BASE}/trae/api/v1/pay/query_user_usage_group_by_session");
    let body = json!({
        "start_time": start,
        "end_time": end,
        "page_size": PAGE_SIZE,
        "page_num": page,
        "usage_type": [7],
    });
    let resp = authed(client, &url, access_token, "")
        .json(&body)
        .send()
        .await?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(anyhow!("凭证已过期(401)，请重新授权"));
    }
    if !status.is_success() {
        return Err(anyhow!("请求失败: {status} {}", &text[..text.len().min(200)]));
    }
    let v: Value = serde_json::from_str(&text).map_err(|e| anyhow!("响应解析失败: {e}"))?;
    let code = v.get("code").and_then(Value::as_i64).unwrap_or(0);
    if code != 0 {
        let msg = v.get("message").and_then(Value::as_str).unwrap_or("未知错误");
        if code == 1001 {
            return Err(anyhow!("凭证无效(code=1001)，请重新授权"));
        }
        return Err(anyhow!("业务错误 code={code} {msg}"));
    }
    let total = v.get("total").map(lenient_i64).unwrap_or(0);
    let items = v
        .get("user_usage_group_by_sessions")
        .and_then(Value::as_array)
        .map(|arr| arr.iter().map(parse_session).collect())
        .unwrap_or_default();
    Ok((total, items))
}

/// 整段拉取时间范围内的全部会话（自动翻页）。
/// `days` 为 None（「全部」范围）时云端最多回溯 90 天。
pub async fn fetch_all(access_token: &str, days: Option<i64>) -> Result<Vec<UsageSession>> {
    let days = days.unwrap_or(90).clamp(1, 90);
    let end = chrono::Utc::now().timestamp();
    let start = end - days * 86_400;
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|e| anyhow!("HTTP 客户端初始化失败: {e}"))?;

    let mut all: Vec<UsageSession> = Vec::new();
    let mut total = 1i64;
    let mut page = 1i64;
    while page <= MAX_PAGES && (all.len() as i64) < total {
        let (t, items) = fetch_page(&client, access_token, start, end, page).await?;
        total = t;
        if items.is_empty() {
            break;
        }
        let last_page = (items.len() as i64) < PAGE_SIZE;
        all.extend(items);
        if last_page {
            break;
        }
        page += 1;
    }
    Ok(all)
}
