//! Qoder CN 官方接口：设备流登录、每日活动领取、积分余额与凭证续期。
//!
//! openapi 域的业务请求按客户端形态带齐协议头与设备身份；活动列表由服务端按账号
//! 维度下发，账号未获投放时列表里只有展示型活动，重试或更换设备头都不会凭空产生
//! 可领活动。因此签到结果必须区分「今日已领」「暂不可领」「账号未获活动投放」，
//! 不能一律归因为设备指纹。官网域提供逐包积分明细。
//! 凭证只在本机数据库与请求头中使用，不写入任何日志。

use std::time::Duration;

use base64::Engine as _;
use chrono::Utc;
use reqwest::header::{HeaderMap, HeaderValue};
use reqwest::Client;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::data::models::{Account, CreditExpiry};

pub const OPEN_API_BASE: &str = "https://openapi.qoder.com.cn";
pub const WEB_BASE: &str = "https://qoder.cn";

const CLIENT_TYPE: &str = "10";
const CLIENT_UA: &str = "Qoder";
const COSY_VERSION: &str = "0.4.3";
const MACHINE_OS: &str = "x86_64_win32";

const DEVICE_AUTH_BASE: &str = "https://qoder.cn/device/selectAccounts";
const SSO_LOGIN_BASE: &str = "https://qoder.cn/sso/login/aliyun";
const DEVICE_AUTH_CLIENT_ID: &str = "732aef47-9cf2-46a2-95fe-4cebb5d0d1fa";

const CAMPAIGNS_PATH: &str = "/sash/api/v1/me/campaigns";
const USAGE_PATH: &str = "/sash/api/v2/me/usage";
const BIG_MODEL_PATH: &str = "/api/v2/me/usages/big_model_credits";

/// 客户端原生设备身份文件（相对客户端数据根）。三键由客户端自行生成并维护，
/// 只在请求时读取，不落库、不写日志。
const NATIVE_IDENTITY_FILE: &str = "SharedClientCache/cache/machine_token.json";

#[derive(Debug, Clone, Default, PartialEq)]
pub struct NativeDeviceIdentity {
    pub token: String,
    pub code: String,
    pub machine_type: String,
    pub hostname: String,
}

pub fn parse_native_identity(body: &Value) -> NativeDeviceIdentity {
    NativeDeviceIdentity {
        token: text(body, &["token"]),
        code: text(body, &["code"]),
        machine_type: text(body, &["type"]),
        hostname: text(body, &["hostname", "machineHostname"]),
    }
}

/// 三键齐备才认作真值；文件缺失、格式异常或字段不全一律返回 None，绝不伪造。
/// 设备身份文件属于客户端原生目录，与登录态快照布局无关，逐个候选根探测。
pub fn read_native_identity() -> Option<NativeDeviceIdentity> {
    super::switch::native_identity_roots()
        .iter()
        .find_map(|root| read_native_identity_from(root))
}

pub fn read_native_identity_from(root: &std::path::Path) -> Option<NativeDeviceIdentity> {
    let raw = std::fs::read_to_string(root.join(NATIVE_IDENTITY_FILE)).ok()?;
    let body: Value = serde_json::from_str(&raw).ok()?;
    let id = parse_native_identity(&body);
    (!id.token.is_empty() && !id.code.is_empty() && !id.machine_type.is_empty()).then_some(id)
}

/// 设备头：`Cosy-MachineId` 每账号独立稳定；设备级三键只有读到原生真值才带。
/// 缺头不补假值，伪造值会把请求分进不受信任的风控桶。
pub fn device_headers(
    machine_id: &str,
    native: Option<&NativeDeviceIdentity>,
) -> Vec<(&'static str, String)> {
    let mut out = Vec::new();
    if !machine_id.trim().is_empty() {
        out.push(("Cosy-MachineId", machine_id.trim().to_string()));
    }
    let Some(n) = native else { return out };
    for (key, value) in [
        ("Cosy-MachineToken", n.token.as_str()),
        ("Cosy-MachineCode", n.code.as_str()),
        ("Cosy-MachineType", n.machine_type.as_str()),
    ] {
        if !value.is_empty() {
            out.push((key, value.to_string()));
        }
    }
    let hostname: String = n
        .hostname
        .chars()
        .filter(|c| c.is_ascii_graphic() && *c != ' ')
        .collect();
    if !hostname.is_empty() {
        out.push(("Cosy-MachineHostname", hostname));
    }
    out
}

/// 只描述本端实际带了哪些设备头，不输出任何凭据值，也不代表服务端已验证通过。
fn fingerprint_hint(machine_id: &str, native: Option<&NativeDeviceIdentity>) -> String {
    match (machine_id.trim().is_empty(), native.is_some()) {
        (true, _) => "未带账号设备指纹".to_string(),
        (false, false) => "已带账号指纹，未读到客户端设备身份".to_string(),
        (false, true) => "已带账号指纹与本机设备身份".to_string(),
    }
}

/// openapi 域固定协议头 + 账号指纹 + 本机设备真值。
fn client_for(machine_id: &str, native: Option<&NativeDeviceIdentity>) -> Result<Client, String> {
    let mut headers = HeaderMap::new();
    for (key, value) in [
        ("Accept", "application/json"),
        ("User-Agent", CLIENT_UA),
        ("Cosy-ClientType", CLIENT_TYPE),
        ("Cosy-Version", COSY_VERSION),
        ("Cosy-MachineOs", MACHINE_OS),
        ("Content-Type", "application/json"),
    ] {
        headers.insert(
            key,
            HeaderValue::from_str(value).map_err(|e| format!("构造请求头失败: {e}"))?,
        );
    }
    for (key, value) in device_headers(machine_id, native) {
        // 单个头含非法字符时跳过，不让整个客户端构造失败
        if let Ok(v) = HeaderValue::from_str(&value) {
            headers.insert(key, v);
        }
    }
    Client::builder()
        .default_headers(headers)
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| format!("构造 HTTP 客户端失败: {e}"))
}

fn client(machine_id: &str) -> Result<Client, String> {
    client_for(machine_id, read_native_identity().as_ref())
}

/// 按候选键取值，兼容 camelCase / snake_case 与常见的响应信封。
fn pick<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    if let Some(data) = v.get("data") {
        if let Some(found) = pick_shallow(data, keys) {
            return Some(found);
        }
    }
    pick_shallow(v, keys)
}

fn pick_shallow<'a>(v: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|k| v.get(*k)).filter(|x| !x.is_null())
}

fn text(v: &Value, keys: &[&str]) -> String {
    pick(v, keys)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn number(v: Option<&Value>) -> Option<f64> {
    let v = v?;
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn integer(v: Option<&Value>) -> Option<i64> {
    let v = v?;
    match v {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

/// 毫秒时间戳转秒；0 / 缺失返回 None。
fn ms_to_secs(ms: Option<i64>) -> Option<i64> {
    ms.filter(|m| *m > 0).map(|m| m / 1000)
}

/// 上游的 expires_in 既有毫秒也有秒：大于 30 天秒数时按毫秒解释。
pub fn normalize_expires_in(value: i64) -> i64 {
    if value > 2_592_000 {
        value / 1000
    } else {
        value
    }
}

fn expiry(at: i64, amount: f64, name: &str) -> CreditExpiry {
    CreditExpiry {
        at,
        amount,
        name: name.to_string(),
    }
}

// ===== 设备流登录 =====

/// 一次授权会话；verifier 只保存在后端内存，前端仅持有 nonce 作为轮询句柄。
/// `machine_id` 会写进授权页参数，服务端据此建立该凭证的设备分桶，必须随账号
/// 一起落库并在后续所有业务请求里原样带回。
pub struct DeviceFlow {
    pub nonce: String,
    pub verifier: String,
    pub auth_url: String,
    pub machine_id: String,
}

pub struct QoderTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    pub refresh_expires_at: i64,
    pub uid: String,
}

/// 生成 PKCE 授权会话。SSO 登录页的回调参数必须整体百分号编码，且只带一次
/// directLogin；显式写入 :443 端口会被回调白名单拒绝。
pub fn new_device_flow(with_client_id: bool) -> DeviceFlow {
    let verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let nonce = Uuid::new_v4().to_string();
    let machine_id = Uuid::new_v4().to_string();
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));
    let mut device_url = format!(
        "{DEVICE_AUTH_BASE}?challenge={challenge}&challenge_method=S256&nonce={nonce}&machine_id={machine_id}&directLogin=true"
    );
    if with_client_id {
        device_url.push_str(&format!("&client_id={DEVICE_AUTH_CLIENT_ID}"));
    }
    let auth_url = format!(
        "{SSO_LOGIN_BASE}?oauth_callback={}",
        urlencoding::encode(&device_url)
    );
    DeviceFlow {
        nonce,
        verifier,
        auth_url,
        machine_id,
    }
}

pub fn poll_url(flow: &DeviceFlow) -> String {
    format!(
        "{OPEN_API_BASE}/api/v1/deviceToken/poll?nonce={}&verifier={}&challenge_method=S256",
        flow.nonce,
        urlencoding::encode(&flow.verifier)
    )
}

/// 单次轮询授权结果：HTTP 404 表示用户尚未完成登录。
pub async fn poll_once(flow: &DeviceFlow) -> Result<(u16, Option<Value>), String> {
    let http = client(&flow.machine_id)?;
    let resp = http
        .get(poll_url(flow))
        .send()
        .await
        .map_err(|e| format!("授权轮询失败: {e}"))?;
    let status = resp.status().as_u16();
    let body = resp.json::<Value>().await.ok();
    Ok((status, body))
}

/// 解析轮询成功响应；nonce 必须回验，防止把别的授权会话的凭证写入本机。
pub fn parse_poll(body: &Value, expected_nonce: &str) -> Option<QoderTokens> {
    if text(body, &["nonce"]) != expected_nonce {
        return None;
    }
    let access_token = text(body, &["token", "accessToken", "access_token"]);
    if access_token.is_empty() {
        return None;
    }
    let now = Utc::now().timestamp();
    let expires_at = integer(pick(body, &["expires_in", "expiresIn"]))
        .map(|v| now + normalize_expires_in(v))
        .unwrap_or(now + 30 * 86400);
    let refresh_expires_at = integer(pick(
        body,
        &["refresh_token_expires_in", "refreshTokenExpiresIn"],
    ))
    .map(|v| now + normalize_expires_in(v))
    .unwrap_or(now + 360 * 86400);
    Some(QoderTokens {
        access_token,
        refresh_token: text(body, &["refresh_token", "refreshToken"]),
        expires_at,
        refresh_expires_at,
        uid: text(body, &["user_id", "userId", "uid"]),
    })
}

/// 账户信息；失败时返回空值，不阻断登录入库。
pub struct QoderProfile {
    pub uid: String,
    pub nickname: String,
}

pub async fn fetch_userinfo(token: &str, machine_id: &str) -> QoderProfile {
    let empty = || QoderProfile {
        uid: String::new(),
        nickname: String::new(),
    };
    let Ok(http) = client(machine_id) else {
        return empty();
    };
    let Ok(resp) = http
        .post(format!("{OPEN_API_BASE}/api/v1/userinfo"))
        .bearer_auth(token)
        .json(&json!({}))
        .send()
        .await
    else {
        return empty();
    };
    if resp.status() != reqwest::StatusCode::OK {
        return empty();
    }
    let Ok(body) = resp.json::<Value>().await else {
        return empty();
    };
    QoderProfile {
        uid: text(&body, &["uid", "userId", "user_id", "id"]),
        nickname: text(&body, &["nickname", "nickName", "name", "displayName"]),
    }
}

// ===== 每日活动签到 =====

#[derive(Debug, Clone, PartialEq)]
pub struct Campaign {
    pub id: String,
    pub amount: f64,
    pub claimable: bool,
    pub claimed: bool,
    pub credit_reward: bool,
    pub start_at: i64,
    pub end_at: i64,
}

/// 活动列表解析：状态大小写宽容，VIEW_DETAILS 之类的展示型活动不参与领取。
pub fn parse_campaigns(body: &Value) -> Vec<Campaign> {
    let list = body
        .get("campaigns")
        .and_then(Value::as_array)
        .or_else(|| body.as_array())
        .cloned()
        .unwrap_or_default();
    list.iter()
        .map(|c| {
            let status = text(c, &["claimStatus", "claim_status"]).to_ascii_uppercase();
            let benefit = c.get("benefit");
            let kind = benefit
                .map(|b| text(b, &["kind", "type"]))
                .unwrap_or_default()
                .to_ascii_uppercase();
            let action = text(c, &["actionType", "action_type"]).to_ascii_uppercase();
            Campaign {
                id: text(c, &["campaignId", "campaign_id", "id"]),
                amount: number(
                    benefit
                        .and_then(|b| pick_shallow(b, &["amount", "value"]))
                        .or_else(|| pick_shallow(c, &["amount"])),
                )
                .unwrap_or(0.0),
                claimable: status == "CLAIMABLE",
                claimed: status == "CLAIMED",
                credit_reward: (kind.is_empty() || kind == "CREDITS")
                    && (action.is_empty() || action.contains("CLAIM")),
                start_at: integer(pick_shallow(c, &["startAt", "start_at"])).unwrap_or(0),
                end_at: integer(pick_shallow(c, &["endAt", "end_at"])).unwrap_or(0),
            }
        })
        .filter(|c| !c.id.is_empty())
        .collect()
}

/// 列表里存在「当前窗口内已领取的积分活动」即视为今日已签。
pub fn has_claimed_today(campaigns: &[Campaign], now: i64) -> bool {
    campaigns.iter().any(|c| {
        c.claimed
            && c.credit_reward
            && (c.start_at == 0 || c.start_at <= now)
            && (c.end_at == 0 || now <= c.end_at)
    })
}

/// 服务端按账号维度未下发积分活动时使用；上层据此不累计失败数、不进冷却。
pub const NO_CAMPAIGN_PREFIX: &str = "NO_CAMPAIGN:";

/// 活动列表顶层标记：上游认为当前是否存在可领活动。
pub fn campaign_claimable_flag(body: &Value) -> bool {
    pick(body, &["claimable"])
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoClaimable {
    /// 当前窗口内有已领取的积分活动
    ClaimedToday,
    /// 账号名下没有积分活动，只有展示型内容
    UpstreamNoCampaign,
    /// 上游声明有可领活动，但本地解析未识别（可能新增了状态值）
    ParseMismatch,
    /// 积分活动已下发，但当前状态不是可领
    NotYetClaimable,
}

/// 无可领活动时的精确归因；顶层 `claimable=true` 一律优先判为解析未识别，
/// 避免把新状态值或字段变更误报成「账号没有活动资格」。
pub fn classify_no_claimable(
    campaigns: &[Campaign],
    upstream_claimable: bool,
    now: i64,
) -> NoClaimable {
    if has_claimed_today(campaigns, now) {
        return NoClaimable::ClaimedToday;
    }
    if upstream_claimable {
        return NoClaimable::ParseMismatch;
    }
    if campaigns.iter().any(|c| c.credit_reward) {
        NoClaimable::NotYetClaimable
    } else {
        NoClaimable::UpstreamNoCampaign
    }
}

async fn get_campaigns(
    http: &Client,
    token: &str,
    hint: &str,
) -> Result<(Vec<Campaign>, bool), String> {
    let resp = http
        .get(format!("{OPEN_API_BASE}{CAMPAIGNS_PATH}"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| format!("活动列表请求失败: {e}"))?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err("登录态已失效，请重新授权".to_string());
    }
    // 风控前置拦截通常表现为 503：设备分桶未对齐时不会给出业务错误码
    if status == reqwest::StatusCode::SERVICE_UNAVAILABLE {
        return Err(format!("活动列表被风控前置拦截（HTTP 503；{hint}）"));
    }
    if !status.is_success() {
        return Err(format!("活动列表返回 HTTP {status}（{hint}）"));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("活动列表解析失败: {e}"))?;
    Ok((parse_campaigns(&body), campaign_claimable_flag(&body)))
}

/// 领取当轮全部可领活动；服务端 claim 幂等，重复领取返回 replayed / CLAIMED。
pub async fn do_checkin(acc: &Account) -> Result<(bool, i64, String), String> {
    if acc.access_token.trim().is_empty() {
        return Err("该账号无 access_token".to_string());
    }
    let native = read_native_identity();
    let http = client_for(&acc.machine_id, native.as_ref())?;
    let hint = fingerprint_hint(&acc.machine_id, native.as_ref());
    let (campaigns, upstream_claimable) = get_campaigns(&http, &acc.access_token, &hint).await?;
    // 空列表多见于登录态失效或前置拦截，此处保留本端设备头状态便于排查
    if campaigns.is_empty() {
        return Err(if upstream_claimable {
            "服务端显示有活动，但本地未能解析活动列表，请更新应用后重试".to_string()
        } else {
            format!("未获取到活动列表（{hint}；也可能活动未开放或登录态已失效）")
        });
    }
    let claimable: Vec<Campaign> = campaigns
        .iter()
        .filter(|c| c.claimable && c.credit_reward)
        .cloned()
        .collect();
    if claimable.is_empty() {
        return match classify_no_claimable(&campaigns, upstream_claimable, Utc::now().timestamp())
        {
            NoClaimable::ClaimedToday => Ok((true, 0, "今日活动已领取".to_string())),
            NoClaimable::UpstreamNoCampaign => Err(format!(
                "{NO_CAMPAIGN_PREFIX}该账号当前没有服务端下发的每日积分活动（活动列表仅有展示型内容，与设备指纹无关）。请确认官方客户端活动页是否显示「每天领 100 Credits」；若官方客户端也不显示，说明该账号暂无活动资格，重复重试或重新授权都无法领取"
            )),
            NoClaimable::ParseMismatch => Err(
                "服务端显示有可领活动，但本地未能识别其状态，请更新应用后重试".to_string(),
            ),
            NoClaimable::NotYetClaimable => Err(
                "积分活动已下发但当前不可领取（非可领状态），可稍后再试或核对官方活动页"
                    .to_string(),
            ),
        };
    }

    let mut points = 0.0;
    let mut done = 0usize;
    let mut messages = Vec::new();
    for (index, campaign) in claimable.iter().enumerate() {
        if !campaign
            .id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
        {
            messages.push("活动编号含非法字符，已跳过".to_string());
            continue;
        }
        if index > 0 {
            tokio::time::sleep(Duration::from_millis(1200)).await;
        }
        let url = format!("{OPEN_API_BASE}{CAMPAIGNS_PATH}/{}/claim", campaign.id);
        let resp = http
            .post(&url)
            .bearer_auth(&acc.access_token)
            .json(&json!({}))
            .send()
            .await;
        let resp = match resp {
            Ok(r) => r,
            Err(e) => {
                // 响应丢失时请求可能已到达服务端，复查列表确认是否已领
                if let Ok((list, _)) = get_campaigns(&http, &acc.access_token, &hint).await {
                    if has_claimed_today(&list, Utc::now().timestamp()) {
                        points += campaign.amount;
                        done += 1;
                        continue;
                    }
                }
                messages.push(format!("领取失败: {e}"));
                continue;
            }
        };
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err("登录态已失效，请重新授权".to_string());
        }
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        let outcome = text(&body, &["status", "claimStatus"]).to_ascii_uppercase();
        let replayed = pick(&body, &["replayed"])
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if outcome == "BLOCKED" {
            messages.push(format!("活动被风控前置拦截（{hint}），请稍后再试"));
            continue;
        }
        if !status.is_success() && outcome != "CLAIMED" {
            messages.push(format!("领取返回 HTTP {status}"));
            continue;
        }
        let amount = number(pick(&body, &["amount"]))
            .or_else(|| {
                body.get("benefit")
                    .and_then(|b| number(pick_shallow(b, &["amount"])))
            })
            .unwrap_or(campaign.amount);
        points += amount;
        done += 1;
        if replayed {
            messages.push("活动已领取过，服务端幂等回放".to_string());
        }
    }
    if done == 0 {
        return Err(messages
            .first()
            .cloned()
            .unwrap_or_else(|| "本次没有成功领取的活动".to_string()));
    }
    let suffix = if messages.is_empty() {
        String::new()
    } else {
        format!("（{}）", messages.join("；"))
    };
    Ok((
        true,
        points.round() as i64,
        format!("成功领取 {done} 个活动{suffix}"),
    ))
}

/// 授权后回查服务端真实签到状态，避免刚在官方客户端领过活动却显示未签到。
pub async fn checkin_status(acc: &Account) -> Result<bool, String> {
    if acc.access_token.trim().is_empty() {
        return Ok(false);
    }
    let http = client(&acc.machine_id)?;
    let hint = fingerprint_hint(&acc.machine_id, read_native_identity().as_ref());
    let (campaigns, _) = get_campaigns(&http, &acc.access_token, &hint).await?;
    Ok(has_claimed_today(&campaigns, Utc::now().timestamp()))
}

// ===== 积分余额 =====

fn quota_remaining(quota: Option<&Value>) -> Option<f64> {
    let q = quota?;
    number(pick_shallow(q, &["remaining", "remaining_value"])).or_else(|| {
        let total = number(pick_shallow(q, &["total", "limit_value"]))?;
        let used = number(pick_shallow(q, &["used", "used_value"])).unwrap_or(0.0);
        Some((total - used).max(0.0))
    })
}

/// 聚合用量响应 → (余额, 逐包明细, 订阅周期到期秒)。
pub fn parse_usage(body: &Value) -> (Option<f64>, Vec<CreditExpiry>, i64) {
    let root = body.get("qoderUsage").unwrap_or(body);
    let plan = quota_remaining(root.get("userQuota"));
    let addon = quota_remaining(root.get("addOnQuota"));
    let plan_expire =
        ms_to_secs(integer(pick_shallow(root, &["expiresAt", "expires_at"]))).unwrap_or(0);
    let balance = match (plan, addon) {
        (Some(p), Some(a)) => Some(p + a),
        (Some(p), None) | (None, Some(p)) => Some(p),
        _ => number(pick(
            body,
            &[
                "totalRemaining",
                "remainingCredits",
                "remaining_credits",
                "remaining",
                "balance",
            ],
        )),
    };

    let mut packages = Vec::new();
    if let Some(a) = addon.filter(|v| *v > 0.0) {
        packages.push(expiry(plan_expire, a, "加赠积分"));
    }
    let detail = root
        .get("dedicated_resource_packages")
        .or_else(|| root.get("dedicatedResourcePackages"))
        .and_then(Value::as_array);
    if let Some(list) = detail {
        for d in list {
            let status = text(d, &["status"])
                .trim_start_matches("QUOTA_DETAIL_STATUS_")
                .to_ascii_uppercase();
            if !status.is_empty() && status != "ACTIVE" {
                continue;
            }
            let remaining = number(pick_shallow(d, &["remaining", "remaining_value"]))
                .or_else(|| {
                    let total = number(pick_shallow(d, &["total", "limit_value"]))?;
                    let used = number(pick_shallow(d, &["used", "used_value"])).unwrap_or(0.0);
                    Some((total - used).max(0.0))
                })
                .unwrap_or(0.0);
            if remaining <= 0.0 {
                continue;
            }
            let at = ms_to_secs(integer(pick_shallow(
                d,
                &["expires_at", "expiresAt", "expire_at"],
            )))
            .unwrap_or(plan_expire);
            let name = text(d, &["name", "displayName"]);
            packages.push(expiry(
                at,
                remaining,
                if name.is_empty() { "资源包" } else { &name },
            ));
        }
    }
    (balance, packages, plan_expire)
}

/// 官网逐包明细 → (总余额, 逐包)；已用完 / 非激活的包不进明细。
pub fn parse_big_model(body: &Value) -> (Option<f64>, Vec<CreditExpiry>) {
    let summary = |key: &str| body.get(key).and_then(|v| v.get("quota_summary"));
    let total = number(
        summary("total_quota").and_then(|s| pick_shallow(s, &["remaining_value", "remaining"])),
    );
    let plan_expire =
        ms_to_secs(integer(pick(body, &["nextResetAt", "next_reset_at"]))).unwrap_or(0);
    let mut packages = Vec::new();
    if let Some(list) = body
        .get("total_quota")
        .and_then(|v| v.get("quota_detail"))
        .and_then(Value::as_array)
    {
        for d in list {
            let status = text(d, &["status"])
                .trim_start_matches("QUOTA_DETAIL_STATUS_")
                .to_ascii_uppercase();
            if !status.is_empty() && status != "ACTIVE" {
                continue;
            }
            if d.get("is_active").and_then(Value::as_bool) == Some(false) {
                continue;
            }
            let amount = number(pick_shallow(d, &["remaining_value", "remaining"])).unwrap_or(0.0);
            if amount <= 0.0 {
                continue;
            }
            let at = ms_to_secs(integer(pick_shallow(d, &["expires_at", "expiresAt"])))
                .unwrap_or(plan_expire);
            let source = text(d, &["source"]);
            let name = text(d, &["name"]);
            let label = if !name.is_empty() {
                name
            } else if source == "PLAN" {
                "订阅配额".to_string()
            } else {
                "资源包".to_string()
            };
            packages.push(expiry(at, amount, &label));
        }
    }
    (total, packages)
}

pub async fn fetch_credits(acc: &Account) -> Result<(f64, Vec<CreditExpiry>), String> {
    if acc.access_token.trim().is_empty() {
        return Err("该账号无 access_token".to_string());
    }
    let http = client(&acc.machine_id)?;
    let resp = http
        .get(format!("{OPEN_API_BASE}{USAGE_PATH}"))
        .bearer_auth(&acc.access_token)
        .send()
        .await
        .map_err(|e| format!("积分查询失败: {e}"))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("登录态已失效，请重新授权".to_string());
    }
    if !resp.status().is_success() {
        return Err(format!("积分查询返回 HTTP {}", resp.status()));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("积分响应解析失败: {e}"))?;
    let (balance, mut packages, plan_expire) = parse_usage(&body);

    // 官网域的逐包明细是增强数据：拿不到时保留聚合口径，不影响余额。
    let big = http
        .get(format!("{WEB_BASE}{BIG_MODEL_PATH}"))
        .bearer_auth(&acc.access_token)
        .send()
        .await;
    if let Ok(r) = big {
        if r.status().is_success() {
            if let Ok(v) = r.json::<Value>().await {
                let (total, detail) = parse_big_model(&v);
                if !detail.is_empty() {
                    packages = detail;
                }
                if let Some(t) = total.or(balance) {
                    return Ok((t, packages));
                }
            }
        }
    }
    let _ = plan_expire;
    balance
        .map(|b| (b, packages))
        .ok_or_else(|| "积分响应缺少余额字段".to_string())
}

// ===== 凭证续期 =====

/// 刷新令牌前缀决定端点：作业令牌走 jobToken，设备流令牌走 deviceToken。
pub fn refresh_endpoint_for(refresh_token: &str) -> &'static str {
    if refresh_token.starts_with("jrt-") {
        "/api/v1/jobToken/refresh"
    } else {
        "/api/v1/deviceToken/refresh"
    }
}

pub async fn refresh_access_token(acc: &Account) -> Result<(String, String, i64), String> {
    let refresh = acc.refresh_token.trim();
    if refresh.is_empty() {
        return Err("该账号没有 refresh_token，只能重新授权".to_string());
    }
    let http = client(&acc.machine_id)?;
    let resp = http
        .post(format!("{OPEN_API_BASE}{}", refresh_endpoint_for(refresh)))
        .json(&json!({ "refresh_token": refresh }))
        .send()
        .await
        .map_err(|e| format!("续期请求失败: {e}"))?;
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Err("刷新凭证已失效，请重新授权".to_string());
    }
    if !status.is_success() {
        return Err(format!("续期返回 HTTP {status}"));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("续期响应解析失败: {e}"))?;
    let access = text(
        &body,
        &[
            "accessToken",
            "access_token",
            "token",
            "device_token",
            "deviceToken",
        ],
    );
    if access.is_empty() {
        return Err("续期响应缺少新凭证".to_string());
    }
    let new_refresh = text(&body, &["refreshToken", "refresh_token"]);
    let now = Utc::now().timestamp();
    let expires_at = integer(pick(&body, &["expiresIn", "expires_in", "expires_in_ms"]))
        .map(|v| now + normalize_expires_in(v))
        .unwrap_or(now + 86400);
    Ok((
        access,
        if new_refresh.is_empty() {
            refresh.to_string()
        } else {
            new_refresh
        },
        expires_at,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn device_flow_binds_pkce_and_encodes_callback() {
        let flow = new_device_flow(true);
        assert_eq!(flow.verifier.len(), 64);
        assert!(flow.auth_url.starts_with(SSO_LOGIN_BASE));
        let (_, encoded) = flow.auth_url.split_once("?oauth_callback=").unwrap();
        let device_url = urlencoding::decode(encoded).unwrap().into_owned();
        assert!(!device_url.contains(":443"));
        assert_eq!(device_url.matches("directLogin").count(), 1);
        assert!(device_url.contains(&format!("nonce={}", flow.nonce)));
        // 授权页里的设备号必须和落库、后续业务请求用的是同一个值
        assert!(device_url.contains(&format!("machine_id={}", flow.machine_id)));
        assert_eq!(flow.machine_id.len(), 36);
        assert_ne!(flow.machine_id, flow.nonce);
        assert!(device_url.contains(&format!("client_id={DEVICE_AUTH_CLIENT_ID}")));
        let expect = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(Sha256::digest(flow.verifier.as_bytes()));
        assert!(device_url.contains(&format!("challenge={expect}")));
    }

    #[test]
    fn compat_flow_omits_client_id() {
        let flow = new_device_flow(false);
        let (_, encoded) = flow.auth_url.split_once("?oauth_callback=").unwrap();
        let device_url = urlencoding::decode(encoded).unwrap().into_owned();
        assert!(!device_url.contains("client_id="));
    }

    #[test]
    fn poll_parses_tokens_and_verifies_nonce() {
        let body = json!({
            "token": "dt-x", "user_id": "u1", "nonce": "n-1",
            "refresh_token": "rt-y", "expires_in": 2591999999i64,
            "refresh_token_expires_in": 31103999999i64
        });
        let tokens = parse_poll(&body, "n-1").expect("应解析成功");
        assert_eq!(tokens.access_token, "dt-x");
        assert_eq!(tokens.uid, "u1");
        let now = Utc::now().timestamp();
        assert!((29..=31).contains(&((tokens.expires_at - now) / 86400)));
        assert!((350..=370).contains(&((tokens.refresh_expires_at - now) / 86400)));
        assert!(parse_poll(&body, "other").is_none(), "nonce 不匹配必须拒绝");
        assert!(parse_poll(&json!({"errorCode": "NotFound"}), "n-1").is_none());
    }

    #[test]
    fn campaigns_filter_claimable_and_ignore_view_only() {
        let body = json!({"campaigns": [
            {"campaignId": "a-1", "claimStatus": "claimable", "actionType": "CLAIM_BENEFIT",
             "benefit": {"kind": "CREDITS", "amount": 100}},
            {"campaignId": "b-2", "claimStatus": "CLAIMED", "actionType": "VIEW_DETAILS",
             "benefit": {"kind": "CREDITS", "amount": 50}},
            {"campaignId": "c-3", "claimStatus": "CLAIMED", "actionType": "CLAIM_BENEFIT",
             "benefit": {"kind": "CREDITS", "amount": 100},
             "startAt": 100, "endAt": 9999999999i64}
        ]});
        let list = parse_campaigns(&body);
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].amount, 100.0);
        assert!(list[0].claimable);
        assert!(!list[1].credit_reward, "展示型活动不算积分领取");
        assert!(has_claimed_today(&list, 200));
        assert!(!has_claimed_today(&list, 50), "窗口外不算今日已领");
    }

    #[test]
    fn no_claimable_is_classified_by_upstream_flag() {
        // 账号未获投放：列表里只有展示型活动，顶层 claimable=false
        let no_campaign = json!({
            "claimable": false,
            "campaigns": [{
                "campaignId": "v-1", "claimStatus": "CLAIMED",
                "actionType": "VIEW_DETAILS", "startAt": 100, "endAt": 9999999999i64
            }]
        });
        let list = parse_campaigns(&no_campaign);
        assert!(!campaign_claimable_flag(&no_campaign));
        assert_eq!(
            classify_no_claimable(&list, false, 200),
            NoClaimable::UpstreamNoCampaign,
            "只有展示型活动不得误报成设备指纹问题"
        );

        // 上游声明有可领但本地解析不出，必须报解析未识别而不是账号无资格
        let mismatch = json!({
            "claimable": true,
            "campaigns": [{
                "campaignId": "a-1", "claimStatus": "NEW_STATUS",
                "actionType": "CLAIM_BENEFIT", "benefit": {"kind": "CREDITS", "amount": 100}
            }]
        });
        let list = parse_campaigns(&mismatch);
        assert!(campaign_claimable_flag(&mismatch));
        assert_eq!(
            classify_no_claimable(&list, true, 200),
            NoClaimable::ParseMismatch
        );

        // 今日已领仍按成功跳过处理
        let claimed = json!({
            "claimable": false,
            "campaigns": [{
                "campaignId": "a-1", "claimStatus": "CLAIMED", "actionType": "CLAIM_BENEFIT",
                "benefit": {"kind": "CREDITS", "amount": 100},
                "startAt": 100, "endAt": 9999999999i64
            }]
        });
        let list = parse_campaigns(&claimed);
        assert_eq!(
            classify_no_claimable(&list, false, 200),
            NoClaimable::ClaimedToday
        );

        // 积分活动已下发但状态不可领
        let pending = json!({
            "claimable": false,
            "campaigns": [{
                "campaignId": "a-1", "claimStatus": "EXPIRED", "actionType": "CLAIM_BENEFIT",
                "benefit": {"kind": "CREDITS", "amount": 100}
            }]
        });
        let list = parse_campaigns(&pending);
        assert_eq!(
            classify_no_claimable(&list, false, 200),
            NoClaimable::NotYetClaimable
        );
    }

    #[test]
    fn usage_balance_and_packages() {
        let body = json!({"qoderUsage": {
            "userQuota": {"total": 300, "used": 100, "remaining": 200},
            "addOnQuota": {"total": 150, "used": 50, "remaining": 100},
            "expiresAt": 1791673619906i64,
            "dedicated_resource_packages": [
                {"total": 50, "remaining": 30, "expires_at": 1791673619906i64,
                 "status": "QUOTA_DETAIL_STATUS_ACTIVE", "name": "签到包"},
                {"total": 50, "remaining": 0, "status": "ACTIVE", "name": "已用完"},
                {"total": 50, "remaining": 40, "status": "QUOTA_DETAIL_STATUS_SUSPENDED"}
            ]
        }});
        let (balance, packages, plan_expire) = parse_usage(&body);
        assert_eq!(balance, Some(300.0));
        assert_eq!(plan_expire, 1791673619);
        assert_eq!(packages.len(), 2, "聚合加赠 + 一个有效资源包");
        assert_eq!(packages[1].name, "签到包");
        assert_eq!(packages[1].amount, 30.0);
    }

    #[test]
    fn big_model_skips_used_and_inactive_packages() {
        let body = json!({
            "nextResetAt": 1791673619906i64,
            "total_quota": {"quota_summary": {"remaining_value": 420},
                "quota_detail": [
                    {"remaining_value": 20, "expires_at": 1791673619906i64, "source": "PLAN", "status": "ACTIVE"},
                    {"remaining_value": 0, "source": "RESOURCE_PACKAGE", "status": "ACTIVE"},
                    {"remaining_value": 30, "status": "QUOTA_DETAIL_STATUS_SUSPENDED"}
                ]}
        });
        let (total, packages) = parse_big_model(&body);
        assert_eq!(total, Some(420.0));
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].name, "订阅配额");
        assert_eq!(packages[0].at, 1791673619);
    }

    #[test]
    fn refresh_endpoint_splits_by_prefix() {
        assert_eq!(refresh_endpoint_for("jrt-abc"), "/api/v1/jobToken/refresh");
        assert_eq!(
            refresh_endpoint_for("rt-abc"),
            "/api/v1/deviceToken/refresh"
        );
        assert_eq!(normalize_expires_in(2591999999), 2591999);
        assert_eq!(normalize_expires_in(3600), 3600);
    }

    #[test]
    fn native_identity_parses_three_keys_and_hostname() {
        let body = json!({
            "code": "e0b843fe69617592d9", "hostname": "Xu",
            "id": "66343531-3762-442d-a230-38773a39372d",
            "os": "x86_64_windows", "token": "native-token",
            "type": "257c1de2141b72c8c1"
        });
        let id = parse_native_identity(&body);
        assert_eq!(id.token, "native-token");
        assert_eq!(id.code, "e0b843fe69617592d9");
        assert_eq!(id.machine_type, "257c1de2141b72c8c1");
        assert_eq!(id.hostname, "Xu");
        // 三键不齐时调用方必须丢弃，不能拿残缺值冒充真值
        let partial = parse_native_identity(&json!({"code": "c", "token": "t"}));
        assert!(partial.machine_type.is_empty());
        assert!(parse_native_identity(&json!({})).token.is_empty());
    }

    #[test]
    fn native_identity_reader_requires_complete_truth() {
        let dir = std::env::temp_dir().join(format!("am-qoder-native-{}", Uuid::new_v4()));
        let cache = dir.join("SharedClientCache/cache");
        std::fs::create_dir_all(&cache).unwrap();
        let file = cache.join("machine_token.json");
        assert!(
            read_native_identity_from(&dir).is_none(),
            "文件不存在时不得伪造"
        );
        std::fs::write(&file, r#"{"code":"c","token":"t"}"#).unwrap();
        assert!(
            read_native_identity_from(&dir).is_none(),
            "三键不全时不得伪造"
        );
        std::fs::write(
            &file,
            r#"{"code":"c","token":"t","type":"m","hostname":"Xu"}"#,
        )
        .unwrap();
        let id = read_native_identity_from(&dir).expect("三键齐备应读到真值");
        assert_eq!(id.code, "c");
        assert_eq!(id.machine_type, "m");
        assert_eq!(id.hostname, "Xu");
        std::fs::write(&file, b"not json").unwrap();
        assert!(
            read_native_identity_from(&dir).is_none(),
            "格式异常时不得伪造"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn device_headers_never_fake_missing_identity() {
        let only_account = device_headers("mid-1", None);
        assert_eq!(only_account.len(), 1);
        assert_eq!(only_account[0].0, "Cosy-MachineId");
        assert_eq!(only_account[0].1, "mid-1");
        // 空账号指纹时一个设备头都不发，绝不生成假值
        assert!(device_headers("  ", None).is_empty());
        assert!(device_headers("", None).is_empty());
    }

    #[test]
    fn device_headers_carry_native_triplet_and_sanitize_hostname() {
        let native = NativeDeviceIdentity {
            token: "native-token".into(),
            code: "code-1".into(),
            machine_type: "type-1".into(),
            hostname: "Xu \u{0007}PC".into(),
        };
        let headers = device_headers("mid-1", Some(&native));
        assert_eq!(headers.len(), 5);
        assert!(headers
            .iter()
            .any(|(k, v)| *k == "Cosy-MachineId" && v == "mid-1"));
        assert!(headers
            .iter()
            .any(|(k, v)| *k == "Cosy-MachineToken" && v == "native-token"));
        assert!(headers
            .iter()
            .any(|(k, v)| *k == "Cosy-MachineCode" && v == "code-1"));
        assert!(headers
            .iter()
            .any(|(k, v)| *k == "Cosy-MachineType" && v == "type-1"));
        assert!(headers
            .iter()
            .any(|(k, v)| *k == "Cosy-MachineHostname" && v == "XuPC"));
        // 账号维度的值不被设备级真值覆盖或重复
        assert_eq!(
            headers
                .iter()
                .filter(|(k, _)| *k == "Cosy-MachineId")
                .count(),
            1
        );
        assert!(!headers
            .iter()
            .any(|(k, v)| *k == "Cosy-MachineId" && v == "native-token"));
    }

    #[test]
    fn fingerprint_hint_describes_state_without_leaking_secrets() {
        let native = NativeDeviceIdentity {
            token: "secret-token".into(),
            code: "c".into(),
            machine_type: "t".into(),
            hostname: "Xu".into(),
        };
        assert_eq!(fingerprint_hint("", None), "未带账号设备指纹");
        assert!(fingerprint_hint("mid-1", None).contains("未读到客户端设备身份"));
        assert_eq!(
            fingerprint_hint("mid-1", Some(&native)),
            "已带账号指纹与本机设备身份",
            "只描述本端带了什么，不得声称服务端已验证通过"
        );
        assert!(!fingerprint_hint("mid-1", Some(&native)).contains("secret"));
    }
}
