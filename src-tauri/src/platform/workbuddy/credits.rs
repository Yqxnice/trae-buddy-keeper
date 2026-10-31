//! WorkBuddy 真实积分查询：直连腾讯 billing「用户资源包」接口。
//!
//! 口径对齐 workbuddy-manager（server/services/tencent.py fetch_credits）：
//! - 国内版路径固定 `/v2/billing/meter/get-user-resource`，ProductCode=p_tcaca，
//!   Status=[0,3]，到期时间窗为「本机此刻 ~ +101 年」的粗过滤；
//! - 单套餐剩余：CycleCapacitySize>0 时只看 Cycle 三字段（remain 钳进 [0,size]，
//!   再用 CycleCapacityUsed 反修正），否则回退 CapacityRemain；
//! - 逐套餐先钳负值再累加；
//! - 到期时刻 CycleEndTime 是 UTC+8 墙钟，显式按 +08:00 解析；
//! - 只保留仍有余额的套餐到期项，按到期时间升序。

use chrono::{Local, NaiveDateTime, Utc};
use serde_json::Value;
use std::sync::Arc;

use crate::data::db::Database;
use crate::data::models::{Account, CreditExpiry, CreditResult};
use crate::platform::workbuddy::api::cookie_header;

const WB_BASE: &str = "https://www.codebuddy.cn";
/// 企业配额接口（对齐 workbuddy-switch issue #167）：企业账号不走个人版资源接口
const ENTERPRISE_USAGE_PATH: &str = "/v2/billing/meter/get-enterprise-user-usage";
/// 企业 ID 补拉接口：type 兜底判定为企业版但账号缺 ID 时使用（单次尝试）
const ENTERPRISE_ACCOUNTS_PATH: &str = "/console/accounts";

/// 查询账号实时积分余额与各套餐到期时间。返回 (余额, 到期列表)。
pub async fn fetch_credits(acc: &Account) -> Result<(f64, Vec<CreditExpiry>), String> {
    let token_mode = acc.cookies.trim().is_empty() && !acc.access_token.trim().is_empty();
    if !token_mode && acc.cookies.trim().is_empty() {
        return Err("该账号无可用凭证".to_string());
    }

    let now = Local::now();
    let end = now + chrono::Duration::days(365 * 101);
    let body = serde_json::json!({
        "PageNumber": 1,
        "PageSize": 100,
        "ProductCode": "p_tcaca",
        "Status": [0, 3],
        "PackageEndTimeRangeBegin": now.format("%Y-%m-%d %H:%M:%S").to_string(),
        "PackageEndTimeRangeEnd": end.format("%Y-%m-%d %H:%M:%S").to_string(),
    });

    let client = reqwest::Client::new();
    // token 账号走 /v2 + Bearer；cookie 账号走网页 Cookie 通道（与签到同形）
    let req = if token_mode {
        client
            .post(format!("{WB_BASE}/v2/billing/meter/get-user-resource"))
            .bearer_auth(acc.access_token.trim())
    } else {
        let cookie = cookie_header(&acc.cookies).map_err(|e| e.to_string())?;
        client
            .post(format!("{WB_BASE}/billing/meter/get-user-resource"))
            .header("Cookie", cookie)
    };
    let resp = req
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("Accept-Language", "zh-CN")
        .header("X-Requested-With", "XMLHttpRequest")
        .header("X-CodeBuddy-Request", "1")
        .header("Origin", WB_BASE)
        .header("Referer", format!("{WB_BASE}/profile/usage"))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;

    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("凭证已过期(401)，请重新登录".to_string());
    }
    let v: Value = resp.json().await.map_err(|e| format!("响应解析失败: {e}"))?;
    let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
    if code != 0 {
        let msg = v
            .get("msg")
            .or_else(|| v.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("");
        return Err(format!("查询失败 code={code} {msg}"));
    }
    let data = v.get("data").cloned().unwrap_or(Value::Null);
    let accounts = extract_resource_accounts(&data).ok_or("响应结构无法识别")?;

    let mut total = 0.0_f64;
    let mut expiries = Vec::new();
    for item in accounts {
        // 逐套餐钳负值再累加（与上游同序）
        let remain = package_remain(item).max(0.0);
        total += remain;
        if remain > 0.0 {
            if let Some(at) = package_expiry(item) {
                expiries.push(CreditExpiry {
                    at,
                    amount: round_credits(remain),
                });
            }
        }
    }
    expiries.sort_by_key(|e| e.at);
    Ok((round_credits(total), expiries))
}

/// 不同层级的信封包装，尽量把套餐数组取出来（Response/Data/data 逐层下钻）
fn extract_resource_accounts(mut cur: &Value) -> Option<&Vec<Value>> {
    for _ in 0..4 {
        match cur {
            Value::Object(map) => {
                if let Some(Value::Array(arr)) = map.get("Accounts") {
                    return Some(arr);
                }
                let next = ["Response", "Data", "data", "response"]
                    .iter()
                    .filter_map(|k| map.get(*k))
                    .find(|v| v.is_object() || v.is_array());
                match next {
                    Some(v) => cur = v,
                    None => return None,
                }
            }
            Value::Array(arr) => return Some(arr),
            _ => return None,
        }
    }
    None
}

fn num(item: &Value, key: &str) -> f64 {
    item.get(key).and_then(|v| v.as_f64()).unwrap_or(0.0)
}

fn round_credits(v: f64) -> f64 {
    if v.fract() == 0.0 {
        v.trunc()
    } else {
        (v * 100.0).round() / 100.0
    }
}

/// 单套餐剩余额度（对齐上游 packageRemainUsed）
fn package_remain(item: &Value) -> f64 {
    let size = num(item, "CycleCapacitySize");
    if size > 0.0 {
        let mut remain = num(item, "CycleCapacityRemain").clamp(0.0, size);
        let mut used = size - remain;
        let cycle_used = num(item, "CycleCapacityUsed");
        if cycle_used > used {
            used = cycle_used;
            if size >= used {
                remain = size - used;
            }
        }
        remain
    } else {
        num(item, "CapacityRemain")
    }
}

/// 套餐到期时刻（epoch 秒）。CycleEndTime 是 UTC+8 墙钟，缺失/解析失败返回 None
fn package_expiry(item: &Value) -> Option<i64> {
    let raw = item.get("CycleEndTime")?.as_str()?.trim().to_string();
    if raw.is_empty() {
        return None;
    }
    let naive = NaiveDateTime::parse_from_str(&raw, "%Y-%m-%d %H:%M:%S").ok()?;
    // 按 UTC 解释再减 8 小时 = UTC+8 墙钟对应的真实 epoch
    Some(naive.and_utc().timestamp() - 8 * 3600)
}

/// 企业账号判定（对齐参考项目 issue #167）：有 enterpriseId，或 type ∈ {ultimate, exclusive}
fn is_enterprise(acc: &Account) -> bool {
    !acc.enterprise_id.trim().is_empty()
        || matches!(acc.account_type.trim(), "ultimate" | "exclusive")
}

struct EnterpriseUsage {
    /// 总量；`-1` 表示不限量
    limit_num: f64,
    used: f64,
    /// 周期重置时间（毫秒时间戳）
    cycle_reset_at: Option<i64>,
}

fn first_number(v: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().filter_map(|k| v.get(*k)).find_map(|x| x.as_f64())
}

/// 解析企业配额响应：兼容 {data:{data:{…}}} / {data:{…}} / 字段位于顶层三种包装，
/// 字段 camelCase / snake_case 双命名兼容（对齐参考项目 parse_enterprise_usage）
fn parse_enterprise_usage(response: &Value) -> Option<EnterpriseUsage> {
    let data = response.get("data").unwrap_or(&Value::Null);
    let nested = data.get("data").unwrap_or(&Value::Null);
    for payload in [nested, data, response] {
        if !payload.is_object() {
            continue;
        }
        let Some(limit_num) = first_number(payload, &["limitNum", "limit_num"]) else {
            continue;
        };
        if !limit_num.is_finite() {
            continue;
        }
        let used = first_number(payload, &["credit", "used_num"])
            .filter(|v| v.is_finite())
            .unwrap_or(0.0)
            .max(0.0);
        let cycle_reset_at = ["cycleResetTime", "CycleResetTime"]
            .iter()
            .filter_map(|k| payload.get(*k))
            .find_map(|x| x.as_i64().or_else(|| x.as_str().and_then(|s| s.parse().ok())));
        return Some(EnterpriseUsage {
            limit_num,
            used,
            cycle_reset_at,
        });
    }
    None
}

/// type 兜底判定为企业版但账号缺 enterpriseId 时，从 /console/accounts 补拉一次（GET，不刷新 token）
async fn resolve_enterprise_id(acc: &Account) -> Option<String> {
    if acc.access_token.trim().is_empty() {
        return None;
    }
    let resp = reqwest::Client::new()
        .get(format!("{WB_BASE}{ENTERPRISE_ACCOUNTS_PATH}"))
        .bearer_auth(acc.access_token.trim())
        .header("Accept", "application/json")
        .header("X-CodeBuddy-Request", "1")
        .header("Origin", WB_BASE)
        .header("Referer", format!("{WB_BASE}/console"))
        .send()
        .await
        .ok()?;
    let v: Value = resp.json().await.ok()?;
    if v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1) != 0 {
        return None;
    }
    let arr = v
        .pointer("/data/accounts")
        .or_else(|| v.pointer("/data/Accounts"))?
        .as_array()?;
    let first = arr.first()?;
    first
        .get("enterpriseId")
        .or_else(|| first.get("enterprise_id"))
        .and_then(|s| s.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 企业配额查询：注入 X-Enterprise-Id / X-Tenant-Id 请求头，POST 空 body；
/// 不限量返回 (None, []) 不落 0/0 快照（与参考项目同口径，避免统计页误显示余额 0）
async fn fetch_enterprise_credits(
    db: &Arc<Database>,
    acc: &Account,
) -> Result<(Option<f64>, Vec<CreditExpiry>, String), String> {
    if acc.access_token.trim().is_empty() {
        return Err("企业积分查询需要 access_token，请重新登录".to_string());
    }
    let mut eid = acc.enterprise_id.trim().to_string();
    if eid.is_empty() {
        eid = resolve_enterprise_id(acc)
            .await
            .ok_or("企业账号缺少 enterpriseId，/console/accounts 补拉失败")?;
        let _ = db.set_enterprise_id(&acc.id, &eid);
    }
    let resp = reqwest::Client::new()
        .post(format!("{WB_BASE}{ENTERPRISE_USAGE_PATH}"))
        .bearer_auth(acc.access_token.trim())
        .header("X-Enterprise-Id", &eid)
        .header("X-Tenant-Id", &eid)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("X-CodeBuddy-Request", "1")
        .header("Origin", WB_BASE)
        .header("Referer", format!("{WB_BASE}/profile/usage"))
        .json(&serde_json::json!({}))
        .send()
        .await
        .map_err(|e| format!("请求失败: {e}"))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err("凭证已过期(401)，请重新登录".to_string());
    }
    let v: Value = resp.json().await.map_err(|e| format!("响应解析失败: {e}"))?;
    let code = v.get("code").and_then(|c| c.as_i64()).unwrap_or(-1);
    if code != 0 {
        let msg = v
            .get("msg")
            .or_else(|| v.get("message"))
            .and_then(|m| m.as_str())
            .unwrap_or("");
        return Err(format!("查询失败 code={code} {msg}"));
    }
    let usage = parse_enterprise_usage(&v).ok_or("企业配额数据无法解析")?;
    if usage.limit_num == -1.0 {
        return Ok((None, vec![], "企业配额：不限量".to_string()));
    }
    let remaining = round_credits((usage.limit_num - usage.used).max(0.0));
    let expiries = match usage.cycle_reset_at {
        Some(ms) if remaining > 0.0 => vec![CreditExpiry {
            at: ms / 1000,
            amount: remaining,
        }],
        _ => vec![],
    };
    Ok((Some(remaining), expiries, "查询成功".to_string()))
}

/// 刷新单个账号积分并落库
pub async fn refresh_account(db: &Arc<Database>, account_id: &str) -> CreditResult {
    let acc = match db.get_account(account_id) {
        Ok(Some(a)) => a,
        _ => {
            return CreditResult {
                account_id: account_id.to_string(),
                account_name: String::new(),
                success: false,
                balance: None,
                expiries: vec![],
                message: "账户不存在".to_string(),
            }
        }
    };
    // 企业分流（issue #167）：workbuddy 企业账号走企业配额接口，不回落个人版
    let result = match acc.platform.as_str() {
        "trae" => crate::platform::trae::api::fetch_credits(&acc)
            .await
            .map(|(b, e)| (Some(b), e, "查询成功".to_string())),
        _ if is_enterprise(&acc) => fetch_enterprise_credits(db, &acc).await,
        _ => fetch_credits(&acc)
            .await
            .map(|(b, e)| (Some(b), e, "查询成功".to_string())),
    };
    match result {
        Ok((balance, expiries, message)) => {
            let json = serde_json::to_string(&expiries).unwrap_or_else(|_| "[]".to_string());
            let _ = db.update_account_credits(&acc.id, balance, &json, Utc::now().timestamp());
            CreditResult {
                account_id: acc.id.clone(),
                account_name: acc.name.clone(),
                success: true,
                balance,
                expiries,
                message,
            }
        }
        Err(e) => {
            db.log("error", "积分查询", &format!("{}：{}", acc.name, e));
            CreditResult {
                account_id: acc.id.clone(),
                account_name: acc.name.clone(),
                success: false,
                balance: acc.credits_balance,
                expiries: vec![],
                message: e,
            }
        }
    }
}
