//! TRAE OAuth 授权登录：本地回环回调服务 + 令牌入库
//!
//! 流程：启动 127.0.0.1:18923 回调服务器 → 打开系统浏览器访问 TRAE 授权页 →
//! 授权完成后浏览器跳回 /authorize?refreshToken=...&accessToken=...，服务器捕获完整
//! 回调 URL 并通过 `oauth-callback` 事件推给前端，前端调用 `complete_trae_oauth` 入库。
//!
//! 注意：TRAE 官网授权页会跨源探测 127.0.0.1:18923 是否在线（原本用于检测桌面客户端），
//! 端口无监听时页面会一直停留在「认证中」，因此服务器必须对无凭证的探测请求应答 200 + CORS。

use std::sync::Mutex;

use axum::{
    http::{header, HeaderValue, Uri},
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use base64::Engine;
use chrono::Utc;
use rand::RngCore;
use serde_json::Value;
use sha2::Digest;
use tauri::{AppHandle, Emitter, State};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::data::models::{Account, CheckinStatus};
use crate::AppState;

const OAUTH_CLIENT_ID: &str = "en1oxy7wnw8j9n";
const OAUTH_CLIENT_SECRET: &str = "-";
const OAUTH_REDIRECT_URI: &str = "http://127.0.0.1:18923/authorize";
const OAUTH_CALLBACK_PORT: u16 = 18923;

// 与真实 TRAE SOLO 客户端一致的常量（参考实现向量钉住）
const IDE_VERSION: &str = "0.1.61";
const DEVICE_BRAND: &str = "83DG";
const OS_VERSION: &str = "Windows 11 Pro";
const DEVICE_NAME: &str = "DESKTOP-CPASOLO";
const PLATFORM_CODE: &str = "SOLO_PC";
const EXCHANGE_PATH: &str = "/trae/api/v3/oauth/ExchangeToken";
const EXCHANGE_ORIGINS: [&str; 2] = ["https://api.trae.cn", "https://api.trae.com.cn"];

/// 回调服务器运行句柄（全局单例，幂等启动）
#[derive(Clone)]
pub struct OAuthServer(pub std::sync::Arc<Mutex<OAuthState>>);

#[derive(Default)]
pub struct OAuthState {
    handle: Option<OAuthCallbackHandle>,
    /// 当轮登录上下文（PKCE verifier + 设备指纹），授权码换证时使用
    ctx: Option<LoginCtx>,
}

#[derive(Clone)]
pub struct LoginCtx {
    code_verifier: String,
    machine_id: String,
    device_id: String,
    device_public_pem: String,
    /// 设备私钥（32 字节 hex）：换证签 DeviceProof 用，必须随账号持久化
    device_key: String,
}

pub struct OAuthCallbackHandle {
    shutdown_txs: Vec<oneshot::Sender<()>>,
    join_handles: Vec<JoinHandle<()>>,
}

impl OAuthCallbackHandle {
    fn stop(&mut self) {
        for tx in self.shutdown_txs.drain(..) {
            let _ = tx.send(());
        }
        for h in self.join_handles.drain(..) {
            h.abort();
        }
    }
}

impl Drop for OAuthCallbackHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

// ==================== 授权 URL 生成 ====================

/// PKCE：48 随机字节 → base64url(无填充)；challenge = SHA-256(verifier) 同编码
fn pkce_pair() -> (String, String) {
    let mut bytes = [0u8; 48];
    rand::thread_rng().fill_bytes(&mut bytes);
    let verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(sha2::Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// 生成设备密钥对：返回 (私钥 32 字节 hex, SPKI PEM 公钥)。
/// DeviceInfo.DevicePublicKey 为空会被判设备绑定拒绝（业务码 20405）；
/// 私钥要落库，后续换证（DeviceProof）必须用同一把钥匙签名。
fn device_keypair() -> Result<(String, String), String> {
    use p256::elliptic_curve::pkcs8::EncodePublicKey;
    for _ in 0..16 {
        let mut bytes = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut bytes);
        let Ok(key) = p256::ecdsa::SigningKey::from_slice(&bytes) else {
            continue; // 标量落在 [1, n-1] 之外，重取
        };
        let der = key
            .verifying_key()
            .to_public_key_der()
            .map_err(|e| format!("导出设备公钥失败: {e}"))?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(der.as_bytes());
        let mut pem = String::with_capacity(encoded.len() + 64);
        pem.push_str("-----BEGIN PUBLIC KEY-----\n");
        for chunk in encoded.as_bytes().chunks(64) {
            pem.push_str(std::str::from_utf8(chunk).unwrap());
            pem.push('\n');
        }
        pem.push_str("-----END PUBLIC KEY-----\n");
        let secret_hex: String = bytes.iter().map(|b| format!("{:02x}", b)).collect();
        return Ok((secret_hex, pem));
    }
    Err("生成设备密钥对失败".to_string())
}

/// 生成当轮登录上下文 + 与真实 TRAE SOLO 客户端完全一致的授权 URL
fn build_login(ctx: &LoginCtx, code_challenge: &str) -> String {
    let login_trace_id = Uuid::new_v4().to_string();
    let os_version_enc = urlencoding::encode(OS_VERSION).replace("%20", "+");
    format!(
        "https://www.trae.cn/authorization?login_version=1&auth_from=solo&login_channel=native_ide\
         &plugin_version=1.0.0&auth_type=local&client_id={}&redirect=0&login_trace_id={}\
         &auth_callback_url={}&machine_id={}&device_id={}&x_device_id={}&x_machine_id={}\
         &x_device_brand={}&x_device_type=windows&x_os_version={}&x_env=prod\
         &x_app_version={}&x_app_type=trae&code_challenge={}&code_challenge_method=S256\
         &hide_saas_login=true",
        OAUTH_CLIENT_ID,
        login_trace_id,
        OAUTH_REDIRECT_URI,
        ctx.machine_id,
        ctx.device_id,
        ctx.device_id,
        ctx.machine_id,
        DEVICE_BRAND,
        os_version_enc,
        IDE_VERSION,
        code_challenge,
    )
}

// ==================== 本地回调服务器 ====================

fn with_cors(body: impl IntoResponse) -> Response {
    let mut resp = body.into_response();
    let headers = resp.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("*"),
    );
    // Chrome Private Network Access：https 页面探测本地服务必须携带该响应头
    headers.insert(
        header::HeaderName::from_static("access-control-allow-private-network"),
        HeaderValue::from_static("true"),
    );
    resp
}

/// 查询串是否携带凭证参数（区分「授权回调」与「在线探测」）
fn query_has_token(query: &str) -> bool {
    [
        "refreshToken",
        "refresh_token",
        "accessToken",
        "access_token",
        "authCodeInfo=",
        "authCode=",
        "code=",
        "error=",
        "isRedirect=",
    ]
    .iter()
    .any(|k| query.contains(k))
}

const SUCCESS_HTML: &str = r#"<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<title>登录成功</title>
<style>
  body { font-family: system-ui, sans-serif; background: #f8fafc; color: #0f172a;
         display: flex; align-items: center; justify-content: center; height: 100vh; margin: 0; }
  .card { text-align: center; padding: 40px 48px; background: #fff; border-radius: 12px;
          box-shadow: 0 4px 16px rgba(0,0,0,.08); }
  .icon { font-size: 40px; }
  h1 { font-size: 18px; margin: 12px 0 8px; }
  p { font-size: 14px; color: #64748b; margin: 0; }
</style>
</head>
<body>
<div class="card">
  <div class="icon">&#9989;</div>
  <h1>授权成功</h1>
  <p>已捕获登录凭证，请返回「TraeBuddy Keeper」。</p>
</div>
</body>
</html>"#;

async fn authorize_handler(uri: Uri, app: AppHandle) -> Response {
    let query = uri.query().unwrap_or("");
    let keys: Vec<&str> = query
        .split('&')
        .filter_map(|p| p.split('=').next())
        .filter(|k| !k.is_empty())
        .collect();
    eprintln!("[oauth] GET /authorize 参数键: {:?}", keys);
    if !query_has_token(query) {
        // 官网授权页的在线探测请求
        return with_cors("ok");
    }
    let full_url = format!(
        "http://127.0.0.1:{}/authorize?{}",
        OAUTH_CALLBACK_PORT, query
    );
    eprintln!("[oauth] 捕获到授权回调，已推送给前端");
    let _ = app.emit("oauth-callback", full_url);
    with_cors(Html(SUCCESS_HTML))
}

async fn probe_fallback(req: axum::extract::Request) -> Response {
    eprintln!(
        "[oauth] 探测请求 {} {} upgrade={:?}",
        req.method(),
        req.uri(),
        req.headers()
            .get(header::UPGRADE)
            .and_then(|v| v.to_str().ok())
    );
    with_cors("ok")
}

/// 确保回调服务器已启动（幂等；已启动则立即返回）
pub async fn ensure_server_running(app: &AppHandle, server: &OAuthServer) -> Result<(), String> {
    {
        let guard = server.0.lock().unwrap_or_else(|e| e.into_inner());
        if guard.handle.is_some() {
            return Ok(());
        }
    }
    let app_for_router = app.clone();
    let make_router = move || {
        let app_for_handler = app_for_router.clone();
        Router::new()
            .route(
                "/authorize",
                get(move |uri: Uri| authorize_handler(uri, app_for_handler.clone()))
                    .options(|| async { with_cors("") }),
            )
            .fallback(probe_fallback)
    };

    let mut handle = OAuthCallbackHandle {
        shutdown_txs: Vec::new(),
        join_handles: Vec::new(),
    };
    // 监听 IPv4/IPv6 回环
    let addrs = [
        (format!("127.0.0.1:{}", OAUTH_CALLBACK_PORT), true),
        (format!("[::1]:{}", OAUTH_CALLBACK_PORT), false),
    ];
    for (addr, required) in addrs {
        match TcpListener::bind(&addr).await {
            Ok(listener) => {
                eprintln!("[oauth] 回调服务器已监听 {}", addr);
                let (tx, rx) = oneshot::channel::<()>();
                let server_fut =
                    axum::serve(listener, make_router()).with_graceful_shutdown(async move {
                        let _ = rx.await;
                    });
                handle.shutdown_txs.push(tx);
                handle.join_handles.push(tokio::spawn(async move {
                    let _ = server_fut.await;
                }));
            }
            Err(e) if required => {
                return Err(format!(
                    "本地回调端口 {} 绑定失败: {}（端口被占用时请改用「手动添加」）",
                    OAUTH_CALLBACK_PORT, e
                ));
            }
            Err(_) => {}
        }
    }

    let mut guard = server.0.lock().unwrap_or_else(|e| e.into_inner());
    guard.handle = Some(handle);
    Ok(())
}

/// 应用启动时预热回调服务器（失败仅记录日志，不阻塞启动）
pub fn prewarm(app: AppHandle, server: OAuthServer) {
    tauri::async_runtime::spawn(async move {
        if let Err(e) = ensure_server_running(&app, &server).await {
            eprintln!("OAuth 回调服务预热失败: {}", e);
        }
    });
}

/// 确保回调服务器运行中，并立即在系统浏览器打开 TRAE 授权页
#[tauri::command]
pub async fn start_trae_oauth(
    app: AppHandle,
    server: State<'_, OAuthServer>,
) -> Result<String, String> {
    ensure_server_running(&app, &server).await?;
    let (code_verifier, code_challenge) = pkce_pair();
    let (device_key, device_public_pem) = device_keypair()?;
    let ctx = LoginCtx {
        code_verifier,
        machine_id: Uuid::new_v4().to_string(),
        device_id: {
            // 16 位数字且首位非 0（Aha 设备号形态，签到风控认这个格式）
            use rand::Rng;
            let mut rng = rand::thread_rng();
            let mut id = String::with_capacity(16);
            id.push(char::from_digit(rng.gen_range(1..=9), 10).unwrap());
            for _ in 0..15 {
                id.push(char::from_digit(rng.gen_range(0..10), 10).unwrap());
            }
            id
        },
        device_public_pem,
        device_key,
    };
    let url = build_login(&ctx, &code_challenge);
    {
        let mut guard = server.0.lock().unwrap_or_else(|e| e.into_inner());
        guard.ctx = Some(ctx);
    }
    // 浏览器打开走独立线程，不阻塞命令返回
    let url_for_open = url.clone();
    std::thread::spawn(move || {
        let _ = open::that_detached(&url_for_open);
    });
    Ok(url)
}

/// 停止本地回调服务器
#[tauri::command]
pub fn stop_trae_oauth(server: State<'_, OAuthServer>) -> Result<(), String> {
    let mut guard = server.0.lock().unwrap_or_else(|e| e.into_inner());
    guard.handle.take();
    guard.ctx.take();
    Ok(())
}

// ==================== 回调解析与入库 ====================

struct CallbackInfo {
    refresh_token: Option<String>,
    access_token: Option<String>,
    /// 授权码（authCodeInfo JSON 或 authCode 键）
    auth_code: Option<String>,
    /// 回调带回的登录站点（host 键），换证候选地址派生用
    login_host: Option<String>,
    user_name: Option<String>,
    avatar: Option<String>,
}

/// 从 JSON 文本（可能被多包一层引号 / 双重 URL 编码）里按候选键取字符串
fn json_pick(raw: &str, keys: &[&str], depth: usize) -> Option<String> {
    if depth > 2 {
        return None;
    }
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parsed: Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => {
            // 可能被二次 URL 编码：再解一层试
            let twice = urlencoding::decode(trimmed).ok()?.to_string();
            if twice == trimmed {
                return None;
            }
            return json_pick(&twice, keys, depth + 1);
        }
    };
    if let Some(inner) = parsed.as_str() {
        return json_pick(inner, keys, depth + 1);
    }
    let obj = parsed.as_object()?;
    keys.iter()
        .find_map(|k| obj.get(*k).and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 解析回调 URL 查询参数
fn parse_callback(callback_url: &str) -> Result<CallbackInfo, String> {
    let query_str = callback_url
        .split('?')
        .nth(1)
        .ok_or_else(|| "回调 URL 中缺少查询参数".to_string())?;
    let mut params = std::collections::HashMap::new();
    for pair in query_str.split('&') {
        let mut kv = pair.splitn(2, '=');
        let key = kv.next().unwrap_or("").to_string();
        let value = kv.next().unwrap_or("").to_string();
        let decoded = urlencoding::decode(&value)
            .map(|c| c.to_string())
            .unwrap_or(value);
        params.insert(key, decoded);
    }
    let refresh_token = params
        .get("refreshToken")
        .or_else(|| params.get("refresh_token"))
        .cloned();
    let access_token = params
        .get("accessToken")
        .or_else(|| params.get("access_token"))
        .cloned();
    // authCode：独立键，或 authCodeInfo JSON（{"authCode":"AC-..."}）
    let auth_code = params
        .get("authCode")
        .or_else(|| params.get("code"))
        .cloned()
        .or_else(|| {
            params
                .get("authCodeInfo")
                .and_then(|raw| json_pick(raw, &["authCode", "AuthCode", "auth_code", "code"], 0))
        });
    let login_host = params
        .get("host")
        .or_else(|| params.get("loginHost"))
        .or_else(|| params.get("login_host"))
        .cloned();
    if refresh_token.is_none() && access_token.is_none() && auth_code.is_none() {
        if let Some(err) = params.get("error").or_else(|| params.get("errorCode")) {
            return Err(format!("授权失败: {err}"));
        }
        if params
            .get("isRedirect")
            .map(|v| v == "false")
            .unwrap_or(false)
        {
            return Err("授权未完成（isRedirect=false）：授权被取消或设备绑定被拒".to_string());
        }
        // 诊断：记录 authCodeInfo 原始值（截断），便于确认内部结构
        if let Some(raw) = params.get("authCodeInfo") {
            let preview: String = raw.chars().take(150).collect();
            eprintln!("[oauth] authCodeInfo 原始值(截断): {}", preview);
        }
        let keys: Vec<&String> = params.keys().collect();
        eprintln!("[oauth] 回调未解析出授权材料，实际参数键: {:?}", keys);
        return Err(format!(
            "回调 URL 中缺少授权材料（refreshToken/authCode），实际参数: {keys:?}"
        ));
    }
    let user_name = params
        .get("userName")
        .or_else(|| params.get("user_name"))
        .or_else(|| params.get("nickname"))
        .cloned()
        .or_else(|| {
            params.get("userInfo").and_then(|raw| {
                json_pick(
                    raw,
                    &[
                        "ScreenName",
                        "screenName",
                        "Nickname",
                        "nickname",
                        "Name",
                        "name",
                        "displayName",
                    ],
                    0,
                )
            })
        });
    let avatar = params
        .get("avatar")
        .or_else(|| params.get("avatarUrl"))
        .cloned()
        .or_else(|| {
            params.get("userInfo").and_then(|raw| {
                json_pick(
                    raw,
                    &[
                        "Avatar",
                        "avatar",
                        "AvatarUrl",
                        "avatarUrl",
                        "avatar_url",
                        "Portrait",
                        "portrait",
                    ],
                    0,
                )
            })
        });
    Ok(CallbackInfo {
        refresh_token,
        access_token,
        auth_code,
        login_host,
        user_name,
        avatar,
    })
}

/// 用 refresh_token 换取 access_token
async fn exchange_token(refresh_token: &str) -> Result<(String, Option<String>), String> {
    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.trae.com.cn/cloudide/api/v3/trae/oauth/ExchangeToken")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "ClientID": OAUTH_CLIENT_ID,
            "RefreshToken": refresh_token,
            "ClientSecret": OAUTH_CLIENT_SECRET,
            "UserID": ""
        }))
        .send()
        .await
        .map_err(|e| format!("ExchangeToken 请求失败: {}", e))?;
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("ExchangeToken 响应解析失败: {}", e))?;
    let code = body.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
    if code != 0 {
        let msg = body
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("未知错误");
        return Err(format!("ExchangeToken 失败 (code={}): {}", code, msg));
    }
    let data = body.get("data").ok_or("ExchangeToken 响应缺少 data")?;
    let access = data
        .get("access_token")
        .or_else(|| data.get("token"))
        .and_then(|t| t.as_str())
        .ok_or("ExchangeToken 响应缺少 access_token")?
        .to_string();
    let new_refresh = data
        .get("refresh_token")
        .and_then(|t| t.as_str())
        .map(|s| s.to_string());
    Ok((access, new_refresh))
}

/// 解析换证/续期响应：{"Result":{"Token","RefreshToken","TokenExpireAt",...}}
fn parse_exchange_response(body: &str) -> Result<(String, String), String> {
    let parsed: Value =
        serde_json::from_str(body).map_err(|e| format!("换证响应不是合法 JSON: {e}"))?;
    for key in ["Result", "result"] {
        if let Some(obj) = parsed.get(key).and_then(Value::as_object) {
            let token = ["Token", "token", "AccessToken", "accessToken"]
                .iter()
                .find_map(|k| obj.get(*k).and_then(Value::as_str))
                .unwrap_or("")
                .to_string();
            let refresh = ["RefreshToken", "refreshToken", "refresh_token"]
                .iter()
                .find_map(|k| obj.get(*k).and_then(Value::as_str))
                .unwrap_or("")
                .to_string();
            if token.is_empty() && refresh.is_empty() {
                return Err("换证响应里没有令牌".to_string());
            }
            // 只有 refreshToken 回来时用它当 accessToken（参考实现同样兜底）
            let access = if token.is_empty() {
                refresh.clone()
            } else {
                token
            };
            return Ok((access, refresh));
        }
    }
    Err("换证响应缺少 Result 字段".to_string())
}

/// 授权码换证：PKCE + 设备指纹，依次尝试候选 origin（顺序即语义）
async fn exchange_auth_code(
    ctx: &LoginCtx,
    auth_code: &str,
    login_host: Option<&str>,
) -> Result<(String, String), String> {
    let body = serde_json::json!({
        "ClientID": OAUTH_CLIENT_ID,
        "AuthCode": auth_code,
        "CodeVerifier": ctx.code_verifier,
        "IDEVersion": IDE_VERSION,
        "DeviceInfo": {
            "DeviceID": ctx.device_id,
            "MachineID": ctx.machine_id,
            "PlatformCode": PLATFORM_CODE,
            "DeviceType": "PC",
            "DeviceName": DEVICE_NAME,
            "DeviceModel": DEVICE_BRAND,
            "ClientVersion": IDE_VERSION,
            "DevicePublicKey": ctx.device_public_pem,
            "DeviceBrand": "Microsoft",
            "DeviceCPU": "",
            "OSInfo": "windows",
            "OSVersion": OS_VERSION,
        }
    });
    // 候选 origin：官方两把账号 API 钉在最前，loginHost 派生的垫后
    let mut origins: Vec<String> = EXCHANGE_ORIGINS.iter().map(|s| s.to_string()).collect();
    if let Some(host) = login_host {
        let host = host
            .trim()
            .trim_start_matches("http://")
            .trim_start_matches("https://");
        let host = host.split(['/', '?']).next().unwrap_or("");
        if !host.is_empty() {
            origins.push(format!("https://{host}"));
            if let Some(rest) = host.strip_prefix("www.") {
                origins.push(format!("https://api.{rest}"));
            }
        }
    }
    origins.dedup();

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;
    let mut errors = Vec::new();
    for origin in &origins {
        let url = format!("{origin}{EXCHANGE_PATH}");
        eprintln!("[oauth] 授权码换证尝试 {url}");
        let result = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("x-cloudide-token", "")
            .header("User-Agent", "Trae/1.0.0 antigravity-cockpit-tools")
            .json(&body)
            .send()
            .await;
        match result {
            Ok(resp) => {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                if status.as_u16() >= 400 {
                    errors.push(format!("{url} => HTTP {status}"));
                    continue;
                }
                match parse_exchange_response(&text) {
                    Ok(pair) => {
                        eprintln!("[oauth] 授权码换证成功");
                        return Ok(pair);
                    }
                    Err(reason) => errors.push(format!("{url} => {reason}")),
                }
            }
            Err(e) => errors.push(format!("{url} => {e}")),
        }
    }
    Err(format!("授权码换证失败：{}", errors.join("；")))
}

/// 完成 OAuth 登录：解析回调 URL → （必要时换 token）→ 写入账户
#[tauri::command]
pub async fn complete_trae_oauth(
    state: State<'_, AppState>,
    server: State<'_, OAuthServer>,
    callback_url: String,
    name: Option<String>,
) -> Result<String, String> {
    let info = parse_callback(&callback_url)?;
    let (access_token, refresh_token) = match (info.access_token, info.refresh_token) {
        (Some(at), Some(rt)) => (at, rt),
        (Some(at), None) => (at.clone(), at),
        (None, Some(rt)) => {
            let (at, nrt) = exchange_token(&rt).await?;
            (at, nrt.unwrap_or(rt))
        }
        (None, None) => {
            // 授权码模式：需要当轮 PKCE verifier 与设备指纹
            let code = info.auth_code.clone().ok_or("回调 URL 中缺少授权材料")?;
            let ctx = {
                let guard = server.0.lock().unwrap_or_else(|e| e.into_inner());
                guard.ctx.clone()
            }
            .ok_or("登录会话已丢失，请重新发起授权登录")?;
            exchange_auth_code(&ctx, &code, info.login_host.as_deref()).await?
        }
    };

    let now = Utc::now().timestamp();

    // 当轮登录的设备身份（设备号/私钥/机器码）：换证签发的 JWT 与之绑定，
    // 签到与后续续期都必须沿用，必须一并落库
    let login_identity = {
        let guard = server.0.lock().unwrap_or_else(|e| e.into_inner());
        guard.ctx.as_ref().map(|ctx| {
            (
                ctx.device_id.clone(),
                ctx.device_key.clone(),
                ctx.machine_id.clone(),
            )
        })
    };

    // 同一 token 已存在则更新而不是重复添加
    let accounts = state.db.list_accounts().map_err(|e| e.to_string())?;
    if let Some(existing) = accounts
        .iter()
        .find(|a| a.platform == "trae" && a.refresh_token == refresh_token)
    {
        state
            .db
            .update_account_tokens(
                &existing.id,
                &access_token,
                &refresh_token,
                now + 7 * 86400,
                now + 30 * 86400,
            )
            .map_err(|e| e.to_string())?;
        // 重新授权会换一套设备身份，旧设备绑定的 JWT 已失效，必须同步覆盖
        if let Some((device_id, device_key, machine_id)) = &login_identity {
            let _ = state.db.update_account_device_identity(
                &existing.id,
                device_id,
                device_key,
                machine_id,
            );
        }
        // 重新授权前可能在官方客户端签过到：回查真实状态，避免误显示未签到
        if let Ok(Some(fresh)) = state.db.get_account(&existing.id) {
            state.checkin.sync_checkin_status(&fresh).await;
        }
        stop_server_inner(&server);
        return Ok(format!("账户「{}」凭据已更新", existing.name));
    }

    let (login_device_id, login_device_key, login_machine_id) =
        login_identity.unwrap_or_else(|| (String::new(), String::new(), String::new()));

    let display_name = name
        .filter(|n| !n.trim().is_empty())
        .or(info.user_name)
        .unwrap_or_else(|| format!("trae-{}", now % 10000));
    let acc = Account {
        id: Uuid::new_v4().to_string(),
        platform: "trae".to_string(),
        name: display_name.clone(),
        email: String::new(),
        uid: String::new(),
        domain: String::new(),
        enterprise_id: String::new(),
        account_type: String::new(),
        device_id: login_device_id,
        device_key: login_device_key,
        machine_id: login_machine_id,
        avatar: info.avatar.unwrap_or_default(),
        access_token,
        refresh_token,
        cookies: String::new(),
        token_expires_at: now + 7 * 86400,
        refresh_token_expires_at: now + 30 * 86400,
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
    state.db.insert_account(&acc).map_err(|e| e.to_string())?;
    state.db.log(
        "success",
        "登录",
        &format!("TRAE 网页授权成功：{}", display_name),
    );
    // 授权前可能在官方客户端签过到：回查真实状态，避免新账号误显示未签到
    state.checkin.sync_checkin_status(&acc).await;
    stop_server_inner(&server);
    Ok(format!("账户「{}」已添加", display_name))
}

fn stop_server_inner(server: &State<'_, OAuthServer>) {
    let mut guard = server.0.lock().unwrap_or_else(|e| e.into_inner());
    guard.handle.take();
    guard.ctx.take();
}
