use anyhow::{anyhow, Result};
use reqwest::Client;
use serde_json::Value;

use crate::data::models::Account;

pub(crate) const TRAE_BASE: &str = "https://api.trae.cn";
const TRAE_ORIGIN: &str = "https://www.trae.cn";
// 与网页授权登录一致的 SOLO 客户端标识（刷新按 ClientID 匹配，错配会被拒）
const SOLO_CLIENT_ID: &str = "en1oxy7wnw8j9n";

// ===== 签到（增长页 /trae/api/v2/ug/…）必须伪装成 TRAE 客户端 =====
// 上游会识别脚本：非客户端 UA（如 curl/Bun/裸 Mozilla）或缺少设备头会被判 9074
// 「当前参与用户太多」，与请求频率无关，所以必须对齐客户端实际发送的头。
const UG_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) TRAE-SOLO-CN/0.1.69 Chrome/120.0.6099.291 Electron/28.2.10 Safari/537.36";
/// 客户端签到请求体固定带 req_source；空体（"{}"）会被服务端直接拒成 9074
const UG_REQ_BODY: &str = "{\"req_source\": 1}";
const SOLO_APP_VERSION: &str = "0.1.69";
const SOLO_CLIENT_VERSION: &str = "0.1.64";
const DEVICE_BRAND: &str = "ASUS TUF Gaming A15 FA507RM_FA507RM";
const DEVICE_OS_VERSION: &str = "10.0.22631";
/// 设备绑定换证路径：签出的 JWT 与该设备绑定，签到才放行
const DEVICE_EXCHANGE_PATH: &str = "/trae/api/v3/oauth/ExchangeToken";

/// 增长页请求头：对齐客户端签到实际发送的那几项（无 Origin/Referer）。
/// 设备号只能出现一个小写的 `x-device-id`——同时发大小写两份会被判异常客户端。
fn ug_authed(
    client: &Client,
    url: &str,
    access_token: &str,
    device_id: &str,
) -> reqwest::RequestBuilder {
    client
        .post(url)
        // 上游认 Cloud-IDE-JWT 方案，Bearer 会被判为未认证（code=1001）
        .header("Authorization", format!("Cloud-IDE-JWT {}", access_token))
        .header("Content-Type", "application/json")
        .header("User-Agent", UG_UA)
        .header("X-User-Region", "CN")
        .header("x-device-id", device_id)
        .header("x-device-brand", DEVICE_BRAND)
        .header("x-device-type", "Windows")
        .header("x-device-cpu", "AMD")
        .header("x-os-version", DEVICE_OS_VERSION)
        .header("x-app-version", SOLO_APP_VERSION)
}

pub(crate) fn authed(
    client: &Client,
    url: &str,
    access_token: &str,
    device_id: &str,
) -> reqwest::RequestBuilder {
    let mut req = client
        .post(url)
        // 上游认 Cloud-IDE-JWT 方案，Bearer 会被判为未认证（code=1001）
        .header("Authorization", format!("Cloud-IDE-JWT {}", access_token))
        .header("Content-Type", "application/json")
        .header("User-Agent", "Mozilla/5.0")
        .header("Origin", TRAE_ORIGIN)
        .header("Referer", format!("{}/", TRAE_ORIGIN))
        // 网页端业务接口固定区域头，与参考实现一致
        .header("X-User-Region", "cn");
    if !device_id.is_empty() {
        req = req.header("x-device-id", device_id);
    }
    req
}

/// 签到状态：(今日已签到, 活动是否开启, 累计签到积分(响应携带时))
pub async fn checkin_status(
    client: &Client,
    access_token: &str,
    device_id: &str,
) -> Result<(bool, bool, Option<i64>)> {
    let url = format!("{}/trae/api/v2/ug/checkin_credits/status", TRAE_BASE);
    let resp = ug_authed(client, &url, access_token, device_id)
        .body(UG_REQ_BODY)
        .send()
        .await?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("TRAE status 请求失败: {} {}", status, text));
    }
    let v: Value = serde_json::from_str(&text).unwrap_or_default();
    // 业务码非 0 视为失败（如 1001 未认证）
    let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
    if code != 0 {
        let msg = v
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("未知错误");
        return Err(anyhow!("code={} msg={}", code, msg));
    }
    // checked_in / enable 在顶层；兼容旧的 data.claimed 形态
    let claimed = v
        .get("checked_in")
        .and_then(|c| c.as_bool())
        .or_else(|| {
            v.get("data")
                .and_then(|d| d.get("claimed"))
                .and_then(|c| c.as_bool())
        })
        .unwrap_or(false);
    let enabled = v.get("enable").and_then(|e| e.as_bool()).unwrap_or(true);
    // 累计签到积分：深度扫描 credit 类数值字段（响应形态未公开，做多形态兜底）
    let credits = scan_credit_number(&v, 0);
    Ok((claimed, enabled, credits))
}

/// 深度扫描签到响应 JSON 中的积分数字：字段名含 credit/point/reward/grant/bonus/amount
/// 的数值（含数字字符串），跳过 code/时间戳/上限等已知非奖励字段；未命中再深入子对象。
/// claim 响应携带的通常是「本次奖励」，status 响应携带的通常是「累计值」。
fn scan_credit_number(v: &Value, depth: usize) -> Option<i64> {
    if depth > 5 {
        return None;
    }
    const SKIPS: [&str; 8] = ["code", "message", "expire", "time", "date", "limit", "status", "id"];
    const HINTS: [&str; 6] = ["credit", "point", "reward", "grant", "bonus", "amount"];
    match v {
        Value::Object(m) => {
            for (k, val) in m {
                let kl = k.to_lowercase();
                if SKIPS.iter().any(|s| kl.contains(s)) {
                    continue;
                }
                if HINTS.iter().any(|h| kl.contains(h)) {
                    let n = match val {
                        Value::Number(n) => n.as_i64(),
                        Value::String(s) => s.trim().parse::<i64>().ok(),
                        Value::Object(_) => scan_credit_number(val, depth + 1),
                        _ => None,
                    };
                    if n.is_some() {
                        return n;
                    }
                }
            }
            for val in m.values() {
                if val.is_object() {
                    let n = scan_credit_number(val, depth + 1);
                    if n.is_some() {
                        return n;
                    }
                }
            }
            None
        }
        _ => None,
    }
}

pub async fn checkin_claim(client: &Client, access_token: &str, device_id: &str) -> Result<i64> {
    let url = format!("{}/trae/api/v2/ug/checkin_credits/claim", TRAE_BASE);
    let resp = ug_authed(client, &url, access_token, device_id)
        .body(UG_REQ_BODY)
        .send()
        .await?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("TRAE claim 请求失败: {} {}", status, text));
    }
    let v: Value = serde_json::from_str(&text).unwrap_or_default();
    let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
    // 9004 参数错误 / 9095 该设备今日已签：都表示「本设备今天签过了」，不是失败
    if code == 9004 || code == 9095 {
        return Err(anyhow!("TRAE claim 今日已签: code={}", code));
    }
    if code != 0 {
        let msg = v
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("未知错误")
            .to_string();
        return Err(anyhow!("TRAE claim 业务错误: code={} msg={}", code, msg));
    }
    // 积分数额：响应形态未公开，深度扫描多形态兜底（data.credits / data.points / 数字字符串等）
    let points = scan_credit_number(&v, 0).unwrap_or(0);
    Ok(points)
}

/// claim 遇到上游瞬时拒绝（10001、HTTP 5xx）时的原地退避序列。
/// 9074「参与用户太多」不在此列：它是设备/UA 识别未通过，换设备号硬试无效，
/// 只能靠客户端 UA + 设备绑定 JWT 从根上避免（见 ug_authed / bound_exchange）。
const CLAIM_RETRY_DELAYS_SECS: [u64; 3] = [5, 15, 60];

fn claim_retryable(err: &anyhow::Error) -> bool {
    let msg = err.to_string();
    msg.contains("code=10001")
        || msg.contains(" 50")
            && (msg.contains(" 500")
                || msg.contains(" 502")
                || msg.contains(" 503")
                || msg.contains(" 504"))
}

/// device_id：签到按设备维度幂等（did_checked_in），用调用方持有的稳定设备 id
pub async fn do_checkin(acc: &Account, device_id: &str) -> Result<(bool, i64, String)> {
    let client = Client::new();
    // 先查状态；记录领取前累计积分（如响应提供），领取后做差可得本次奖励
    let before_credits;
    match checkin_status(&client, &acc.access_token, device_id).await {
        Ok((claimed, enabled, credits)) => {
            before_credits = credits;
            if claimed {
                return Ok((true, 0, "今日已签到".to_string()));
            }
            if !enabled {
                return Ok((false, 0, "签到活动未开启".to_string()));
            }
        }
        Err(e) => {
            return Ok((false, 0, format!("查询签到状态失败: {}", e)));
        }
    }
    // 领取：瞬时失败按退避序列原地重试，避免错过当天
    let mut result = checkin_claim(&client, &acc.access_token, device_id).await;
    for delay in CLAIM_RETRY_DELAYS_SECS {
        match &result {
            Err(e) if claim_retryable(e) => {
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
                result = checkin_claim(&client, &acc.access_token, device_id).await;
            }
            _ => break,
        }
    }
    match result {
        Ok(points) => {
            // claim 的 code=0 可能是假成功：当日已签的账号对任意设备号都会回 0。
            // 只有用同一设备号回查 checked_in 翻转，才算真正领到。
            match checkin_status(&client, &acc.access_token, device_id).await {
                Ok((true, _, after_credits)) => {
                    // claim 响应未携带奖励数额时，用领取前后累计积分差值兜底
                    // （免费账号每日 100、会员 200 由上游判定，这里如实显示）
                    let earned = if points > 0 {
                        points
                    } else {
                        (after_credits.unwrap_or(0) - before_credits.unwrap_or(0)).max(0)
                    };
                    Ok((
                        true,
                        earned,
                        if earned > 0 {
                            format!("签到成功 +{}", earned)
                        } else {
                            "签到成功".to_string()
                        },
                    ))
                }
                Ok((false, _, _)) => Ok((
                    false,
                    0,
                    "领取接口返回成功，但同设备号回查未确认，稍后自动重试".to_string(),
                )),
                // 回查自身失败时不否定领取结果
                Err(_) => Ok((true, points, "签到成功".to_string())),
            }
        }
        Err(e) if e.to_string().contains("今日已签") => Ok((true, 0, "今日已签到".to_string())),
        Err(e) => Ok((false, 0, format!("签到失败: {}", e))),
    }
}

/// 查询资格包过期记录（/trae/api/v2/pay/expired_ents，只读）。
/// 返回 (到期秒, 包总额, 包名)；临期与已过期都在列表里，调用方自行取舍。
/// 仅用于给积分包明细补充名称，失败不影响余额查询主流程。
async fn fetch_expired_ents(
    client: &Client,
    access_token: &str,
) -> Result<Vec<(i64, f64, String)>, String> {
    let url = format!("{TRAE_BASE}/trae/api/v2/pay/expired_ents");
    let resp = authed(client, &url, access_token, "")
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!("查询失败: {status}"));
    }
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("响应解析失败: {e}"))?;
    let mut out = Vec::new();
    if let Some(list) = v.get("expired_ent_list").and_then(Value::as_array) {
        for e in list {
            let name = e
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let limit = e
                .get("credits_limit")
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let ms = e
                .get("expire_time_ms")
                .and_then(Value::as_f64)
                .unwrap_or(0.0) as i64;
            let at = if ms > 1_000_000_000_000 { ms / 1000 } else { ms };
            if at > 0 && (!name.is_empty() || limit > 0.0) {
                out.push((at, limit, name));
            }
        }
    }
    Ok(out)
}

/// TRAE 真实积分：POST /trae/api/v2/pay/web_user_ent_usage（只读）。
/// 返回 (总余额, 到期列表)。积分包：credits_limit - usage.credits_amount 为剩余，
/// entitlement_base_info.end_time 为到期（epoch 秒）。
pub async fn fetch_credits(
    acc: &Account,
) -> Result<(f64, Vec<crate::data::models::CreditExpiry>), String> {
    if acc.access_token.trim().is_empty() {
        return Err("该账号无 access_token".to_string());
    }
    let client = Client::new();
    let url = format!("{}/trae/api/v2/pay/web_user_ent_usage", TRAE_BASE);
    let resp = authed(&client, &url, acc.access_token.trim(), "")
        .json(&serde_json::json!({ "require_usage": true }))
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err("凭证已过期(401)，请重新授权".to_string());
    }
    if !status.is_success() {
        return Err(format!("查询失败: {} {}", status, text));
    }
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("响应解析失败: {e}"))?;
    let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
    if code != 0 {
        let msg = v.get("message").and_then(|m| m.as_str()).unwrap_or("");
        return Err(format!("查询失败 code={code} {msg}"));
    }

    let num = |v: &Value, key: &str| v.get(key).and_then(|x| x.as_f64()).unwrap_or(0.0);
    // 临期包名称映射（到期秒 → 包名）；查询失败只影响名称展示，不阻断余额主流程
    let names: std::collections::HashMap<i64, String> =
        match fetch_expired_ents(&client, acc.access_token.trim()).await {
            Ok(list) => list
                .into_iter()
                .map(|(at, _, name)| (at, name))
                .collect(),
            Err(_) => Default::default(),
        };
    let mut total = 0.0_f64;
    let mut expiries = Vec::new();
    let mut packs_empty = false;
    if let Some(packs) = v
        .get("user_entitlement_pack_list")
        .and_then(|p| p.as_array())
    {
        packs_empty = packs.is_empty();
        for p in packs {
            let base = p
                .get("entitlement_base_info")
                .cloned()
                .unwrap_or(Value::Null);
            let limit = base
                .get("product_extra")
                .and_then(|e| e.get("package_extra"))
                .and_then(|e| e.get("quota"))
                .and_then(|q| q.get("credits_limit"))
                .and_then(|c| c.as_f64())
                .or_else(|| {
                    base.get("quota")
                        .and_then(|q| q.get("credits_limit"))
                        .and_then(|c| c.as_f64())
                })
                .unwrap_or(0.0);
            let used = p
                .get("usage")
                .and_then(|u| u.get("credits_amount"))
                .and_then(|c| c.as_f64())
                .unwrap_or(0.0);
            let remain = (limit - used).max(0.0);
            total += remain;
            // end_time 是 epoch 秒；毫秒量级（>1e12）则直接保留
            let end = num(&base, "end_time") as i64;
            let at = if end > 1_000_000_000_000 {
                end / 1000
            } else {
                end
            };
            if remain > 0.0 && at > 0 {
                expiries.push(crate::data::models::CreditExpiry {
                    name: names.get(&at).cloned().unwrap_or_default(),
                    at,
                    amount: remain,
                });
            }
        }
    }
    // 包为空时回退到汇总字段
    if total == 0.0 {
        if let Some(s) = v.get("usage_summary") {
            total = num(s, "total_amount") - num(s, "consumed_amount");
        }
    }
    // 防清零：包列表为空且汇总也拿不到数值，多半是未登录/风控的异常结构，
    // 报错让调用方保留库中旧余额，而不是把界面余额写成 0
    if packs_empty && total == 0.0 {
        return Err("积分包列表为空，可能会话失效".into());
    }
    expiries.sort_by_key(|e| e.at);
    Ok((total.max(0.0), expiries))
}

// ===== 换证：设备绑定（DeviceProof）优先，legacy /cloudide 兜底 =====

/// 解析换证响应（两种契约都是 Result.{Token, RefreshToken, TokenExpireAt}）
fn parse_exchange_result(text: &str, fallback_refresh: &str) -> Result<(String, String, i64)> {
    let v: Value = serde_json::from_str(text).unwrap_or_default();
    let result = v
        .get("Result")
        .or_else(|| v.get("result"))
        .ok_or_else(|| anyhow!("换证响应缺少 Result: {}", text))?;
    let access = ["Token", "token", "AccessToken", "accessToken"]
        .iter()
        .find_map(|k| result.get(*k).and_then(|t| t.as_str()))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("换证响应缺少 Token: {}", text))?
        .to_string();
    let refresh = ["RefreshToken", "refreshToken", "refresh_token"]
        .iter()
        .find_map(|k| result.get(*k).and_then(|t| t.as_str()))
        .filter(|s| !s.is_empty())
        .unwrap_or(fallback_refresh)
        .to_string();
    // TokenExpireAt: 数字(毫秒)或日期字符串
    let expires_at = match result.get("TokenExpireAt") {
        Some(Value::Number(n)) => n.as_i64().map(|ms| ms / 1000).unwrap_or(0),
        Some(Value::String(s)) => chrono::DateTime::parse_from_rfc3339(s)
            .map(|d| d.timestamp())
            .unwrap_or(0),
        _ => 0,
    };
    let expires_at = if expires_at > 0 {
        expires_at
    } else {
        chrono::Utc::now().timestamp() + 7 * 86400
    };
    Ok((access, refresh, expires_at))
}

fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let t = s.trim();
    if t.len() % 2 != 0 || t.is_empty() {
        return None;
    }
    (0..t.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&t[i..i + 2], 16).ok())
        .collect()
}

/// 由设备私钥（32 字节 hex）导出 SPKI PEM 公钥（DeviceInfo.DevicePublicKey）
fn public_key_pem_from_secret(secret_hex: &str) -> Result<String> {
    use base64::Engine;
    use p256::elliptic_curve::pkcs8::EncodePublicKey;
    let bytes = hex_decode(secret_hex).ok_or_else(|| anyhow!("设备密钥格式非法"))?;
    let key = p256::ecdsa::SigningKey::from_slice(&bytes)
        .map_err(|e| anyhow!("设备密钥无法还原: {e}"))?;
    let der = key
        .verifying_key()
        .to_public_key_der()
        .map_err(|e| anyhow!("导出设备公钥失败: {e}"))?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(der.as_bytes());
    let mut pem = String::with_capacity(encoded.len() + 64);
    pem.push_str("-----BEGIN PUBLIC KEY-----\n");
    for chunk in encoded.as_bytes().chunks(64) {
        pem.push_str(std::str::from_utf8(chunk).map_err(|e| anyhow!("{e}"))?);
        pem.push('\n');
    }
    pem.push_str("-----END PUBLIC KEY-----\n");
    Ok(pem)
}

/// DeviceProof：设备私钥的 ECDSA-SHA256 签名（DER、base64），
/// 签名原文为「POST / 路径 / ClientID / refresh_token / 时间戳 / 随机数」逐行拼接。
/// 换证带签名才会把 JWT 绑到这台设备；不带签名的 JWT 签到必被 9074 拒绝。
fn device_proof(secret_hex: &str, path: &str, client_id: &str, refresh: &str) -> Result<Value> {
    use base64::Engine;
    use p256::ecdsa::signature::Signer;
    let bytes = hex_decode(secret_hex).ok_or_else(|| anyhow!("设备密钥格式非法"))?;
    let key = p256::ecdsa::SigningKey::from_slice(&bytes)
        .map_err(|e| anyhow!("设备密钥无法还原: {e}"))?;
    let ts = chrono::Utc::now().timestamp();
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let text = format!(
        "POST\n{}\n{}\n{}\n{}\n{}",
        path, client_id, refresh, ts, nonce
    );
    let sig: p256::ecdsa::Signature = key.sign(text.as_bytes());
    let der = sig.to_der();
    Ok(serde_json::json!({
        "Signature": base64::engine::general_purpose::STANDARD.encode(der.as_bytes()),
        "Timestamp": ts,
        "Nonce": nonce,
    }))
}

/// 换证「ClientID × 产品线」组合链（对齐参考实现）：
/// ① 桌面端 IDE 线 ② SOLO ClientID + IDE_PC ③ SOLO ClientID + SOLO_PC。
/// 10101（refresh token 不属于该客户端）/ 20403（设备不匹配）时换下一组再试。
const EXCHANGE_COMBOS: [(&str, &str, &str); 3] = [
    ("ono9krqynydwx5", "IDE_PC", "1.107.1"),
    (SOLO_CLIENT_ID, "IDE_PC", SOLO_CLIENT_VERSION),
    (SOLO_CLIENT_ID, "SOLO_PC", SOLO_CLIENT_VERSION),
];

/// 设备绑定换证：POST api.trae.cn/trae/api/v3/oauth/ExchangeToken
/// body 带 DeviceInfo（设备号/机器码/公钥）+ DeviceProof 签名。
/// 入参 DeviceInfo 的设备号必须与登录时的一致，否则上游回 20403「Token device not match」。
async fn bound_exchange(acc: &Account) -> Result<(String, String, i64)> {
    if acc.device_key.trim().is_empty() {
        return Err(anyhow!("账号缺少设备密钥（旧版登录），需重新授权"));
    }
    if acc.device_id.trim().is_empty() {
        return Err(anyhow!("账号缺少设备号，需重新授权"));
    }
    let public_pem = public_key_pem_from_secret(&acc.device_key)?;
    let machine_id = if acc.machine_id.trim().is_empty() {
        acc.device_id.trim()
    } else {
        acc.machine_id.trim()
    };
    let client = Client::new();
    let url = format!("{}{}", TRAE_BASE, DEVICE_EXCHANGE_PATH);
    let mut last_err = String::new();
    for (client_id, platform_code, client_version) in EXCHANGE_COMBOS {
        let proof = device_proof(
            acc.device_key.trim(),
            DEVICE_EXCHANGE_PATH,
            client_id,
            acc.refresh_token.trim(),
        )?;
        let body = serde_json::json!({
            "ClientID": client_id,
            "ClientSecret": "",
            "RefreshToken": acc.refresh_token.trim(),
            "DeviceInfo": {
                "DeviceID": acc.device_id.trim(),
                "MachineID": machine_id,
                "PlatformCode": platform_code,
                "DeviceType": "PC",
                "DeviceName": "",
                "DeviceModel": "",
                "ClientVersion": client_version,
                "DevicePublicKey": public_pem,
                "DeviceBrand": DEVICE_BRAND,
                "DeviceCPU": "AMD",
                "OSInfo": "Windows",
                "OSVersion": DEVICE_OS_VERSION,
            },
            "DeviceProof": proof,
            "IDEVersion": client_version,
        });
        let resp = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("x-cloudide-token", "")
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            return parse_exchange_result(&text, acc.refresh_token.trim());
        }
        last_err = format!("{} {}", status, text);
        // 只有「客户端不匹配 / 设备不匹配」值得换组合，其它错误直接返回
        if !(text.contains("10101") || text.contains("20403")) {
            break;
        }
    }
    Err(anyhow!("设备绑定换证失败: {}", last_err))
}

/// legacy 换证（/cloudide/…）：不消费设备密钥，但签出的 JWT 不绑定设备，
/// 签到会被判 9074。仅作为续期兜底，保证 token 不失效。
async fn legacy_exchange(refresh_token: &str) -> Result<(String, String, i64)> {
    let client = Client::new();
    // 与网页授权换证同 host（oauth 登录验证过可用）
    let url = "https://api.trae.com.cn/cloudide/api/v3/trae/oauth/ExchangeToken";
    let resp = client
        .post(url)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "ClientID": SOLO_CLIENT_ID,
            "ClientSecret": "-",
            "RefreshToken": refresh_token,
            "UserID": ""
        }))
        .send()
        .await?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!("刷新 token 失败: {} {}", status, text));
    }
    parse_exchange_result(&text, refresh_token)
}

/// 用 refresh_token 换新 access_token：先试设备绑定换证（签到才可用），
/// 失败再退回 legacy 换证。返回 (access, refresh, expires_at, 是否设备绑定)
pub async fn refresh_access_token(acc: &Account) -> Result<(String, String, i64, bool)> {
    match bound_exchange(acc).await {
        Ok((a, r, e)) => Ok((a, r, e, true)),
        Err(bound_err) => {
            let (a, r, e) = legacy_exchange(acc.refresh_token.trim())
                .await
                .map_err(|legacy_err| anyhow!("{}；{}", bound_err, legacy_err))?;
            Ok((a, r, e, false))
        }
    }
}
