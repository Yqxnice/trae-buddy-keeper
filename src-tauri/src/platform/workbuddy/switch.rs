//! WorkBuddy 客户端账号切换：备份 → 关进程 → 写认证文件 → 删退出标记 → 启动。
//!
//! 方案对齐 workbuddy-switch（auth_file.rs / switch.rs）：
//! 官方登录态集中在单个 JSON 认证文件（四段：account/auth/accounts/allAccounts），
//! 切换 = 原子写入目标账号凭据 + 写后校验 + 清理 `<file>.logged-out` 退出标记
//! （标记存在时官方 App 一律视为未登录）。写入失败自动从备份还原，不带病启动。

use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

use crate::data::models::Account;

/// 国内版认证文件名（与官方一致；国际版为 workbuddy-desktop-ai.info，本项目不涉及）
const AUTH_FILE_NAME: &str = "workbuddy-desktop.info";
/// CodeBuddy 国内版产品域
const DOMAIN: &str = "www.codebuddy.cn";
/// Windows 进程镜像名
const PROCESS_IMAGE: &str = "WorkBuddy.exe";
/// 关闭进程总预算：先优雅 12s，再强杀 8s（对齐 TraeTools 三级关闭口径）
const GRACEFUL_WAIT_SECS: u64 = 12;
const FORCE_WAIT_SECS: u64 = 8;

/// 官方认证文件路径（三平台口径与 workbuddy-switch 一致）
pub fn auth_file_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("无法定位用户主目录")?;
    #[cfg(target_os = "windows")]
    let dir = home.join("AppData/Local/CodeBuddyExtension/Data/Public/auth");
    #[cfg(target_os = "macos")]
    let dir = home.join("Library/Application Support/CodeBuddyExtension/Data/Public/auth");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let dir = home.join(".local/share/CodeBuddyExtension/Data/Public/auth");
    Ok(dir.join(AUTH_FILE_NAME))
}

/// 官方退出标记：在完整文件名后追加后缀（不是替换扩展名）
fn logout_marker_path(auth_path: &Path) -> PathBuf {
    let mut p = auth_path.as_os_str().to_os_string();
    p.push(".logged-out");
    PathBuf::from(p)
}

/// 切换前备份当前认证文件到 auth 目录下的 backups/，返回备份路径（无文件则不备份）
fn backup_auth_file(auth_path: &Path) -> Option<PathBuf> {
    if !auth_path.exists() {
        return None;
    }
    let dir = auth_path.parent()?.join("backups");
    std::fs::create_dir_all(&dir).ok()?;
    let ts = chrono::Utc::now().format("%Y%m%d-%H%M%S");
    let dest = dir.join(format!("workbuddy-desktop.{ts}.info"));
    std::fs::copy(auth_path, &dest).ok()?;
    Some(dest)
}

/// 从账号构造官方 account 字段（缺省值与 workbuddy-switch build_account_obj 对齐）
fn build_account_obj(acc: &Account) -> Value {
    let mut obj = Map::new();
    obj.insert("uid".to_string(), json!(acc.uid));
    obj.insert("nickname".to_string(), json!(acc.name));
    let defaults = json!({
        "type": "personal",
        "accountType": "",
        "idp": "",
        "oneidAccountId": "",
        "areaInfoComplete": false,
        "isCurrentOneIdEnterprise": false,
        "isCurrentOneIdPersonal": false,
        "isFirstLogin": false,
        "isCreator": false,
        "isAdmin": false,
        "uin": "",
        "phoneNumber": "",
        "lastLogin": true,
        "pluginEnabled": true,
        "deployStatus": {"statusCode": 0, "statusMsg": "", "detailMsg": ""},
        "sso": {"domain": "", "domainModifiedTimes": 0},
    });
    for (k, v) in defaults.as_object().unwrap() {
        obj.insert(k.clone(), v.clone());
    }
    Value::Object(obj)
}

/// 从账号构造官方 auth 字段（expiresAt 为毫秒时间戳；库内存秒，这里换算）
fn build_auth_obj(acc: &Account) -> Value {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut obj = Map::new();
    obj.insert("accessToken".to_string(), json!(acc.access_token));
    obj.insert("refreshToken".to_string(), json!(acc.refresh_token));
    obj.insert("tokenType".to_string(), json!("Bearer"));
    // 优先用登录响应落库的 domain（真实桌面会话常见 www.workbuddy.cn），缺失回退国内版产品域
    let domain = if acc.domain.is_empty() {
        DOMAIN
    } else {
        acc.domain.as_str()
    };
    obj.insert("domain".to_string(), json!(domain));
    obj.insert("lastRefreshTime".to_string(), json!(now_ms));
    obj.insert(
        "scope".to_string(),
        json!("openid profile offline_access email"),
    );
    if acc.token_expires_at > 0 {
        let exp_ms = acc.token_expires_at * 1000;
        obj.insert("expiresAt".to_string(), json!(exp_ms));
        obj.insert(
            "expiresIn".to_string(),
            json!(((exp_ms - now_ms) / 1000).max(0)),
        );
        let refresh_ms = if acc.refresh_token_expires_at > 0 {
            acc.refresh_token_expires_at * 1000
        } else {
            exp_ms
        };
        obj.insert("refreshExpiresAt".to_string(), json!(refresh_ms));
        obj.insert(
            "refreshExpiresIn".to_string(),
            json!(((refresh_ms - now_ms) / 1000).max(0)),
        );
    } else {
        obj.insert("expiresIn".to_string(), json!(0));
        obj.insert("refreshExpiresIn".to_string(), json!(0));
    }
    obj.insert("notBeforePolicy".to_string(), json!(0));
    obj.insert("sessionState".to_string(), json!(""));
    Value::Object(obj)
}

/// 原子写：先写同目录临时文件，再替换目标（Windows 上 rename 不能覆盖，需先删）
fn atomic_write(path: &Path, content: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("info.tmp");
    std::fs::write(&tmp, content)?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// 把账号写入官方认证文件：合并 allAccounts（按 uid 去重）→ 原子写 → 写后校验 → 清退出标记
fn write_account_to_auth_file(acc: &Account, auth_path: &Path) -> Result<(), String> {
    if let Some(parent) = auth_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let existing: Value = std::fs::read_to_string(auth_path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}));
    let all_accounts = existing
        .get("allAccounts")
        .or_else(|| existing.get("accounts"))
        .filter(|v| v.is_array())
        .cloned()
        .unwrap_or_else(|| json!([]));

    let account_obj = build_account_obj(acc);
    let auth_obj = build_auth_obj(acc);

    // 目标 uid 非空时按 uid/id 去重；为空时保留全部存量（无从判重）
    let mut all: Vec<Value> = all_accounts.as_array().cloned().unwrap_or_default();
    if !acc.uid.is_empty() {
        all.retain(|a| {
            let primary = a
                .get("uid")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .or_else(|| {
                    a.get("id")
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                })
                .unwrap_or("");
            primary != acc.uid
        });
    }
    all.push(account_obj.clone());

    let session = json!({
        "account": account_obj,
        "auth": auth_obj,
        "accounts": all,
        "allAccounts": all,
    });
    let content = serde_json::to_string_pretty(&session).map_err(|e| e.to_string())?;
    atomic_write(auth_path, &content).map_err(|e| format!("写入认证文件失败: {e}"))?;

    // 写后校验：重读比对 accessToken，不一致视为未写入目标账号
    let written: Value = std::fs::read_to_string(auth_path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .ok_or("认证文件写后读取失败")?;
    let written_token = written
        .get("auth")
        .and_then(|a| a.get("accessToken"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if written_token != acc.access_token {
        return Err("认证文件写后校验失败，未写入目标账号".to_string());
    }

    // 官方 App 优先读退出标记：标记存在时即使凭据齐全也视为未登录，必须在校验成功后清理
    let marker = logout_marker_path(auth_path);
    match std::fs::remove_file(&marker) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => {
            return Err(format!(
                "清理 WorkBuddy 退出标记失败（{}）：{e}",
                marker.display()
            ))
        }
    }
    Ok(())
}

// ===== 进程控制 =====

#[cfg(target_os = "windows")]
fn workbuddy_running() -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let out = std::process::Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {PROCESS_IMAGE}"), "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout).contains("WorkBuddy"),
        Err(_) => false,
    }
}

#[cfg(not(target_os = "windows"))]
fn workbuddy_running() -> bool {
    std::process::Command::new("pgrep")
        .args(["-x", "WorkBuddy"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// 关闭 WorkBuddy：先温和结束，超时后强杀；仍有存活则报错（带残留进程写文件会切换无效）
fn close_workbuddy() -> Result<(), String> {
    if !workbuddy_running() {
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        // 1) 温和结束（不带 /F，给客户端自行落盘的机会）
        let _ = std::process::Command::new("taskkill")
            .args(["/IM", PROCESS_IMAGE, "/T"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        if wait_exit(GRACEFUL_WAIT_SECS) {
            return Ok(());
        }
        // 2) 进程树强杀
        let _ = std::process::Command::new("taskkill")
            .args(["/IM", PROCESS_IMAGE, "/T", "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        if wait_exit(FORCE_WAIT_SECS) {
            return Ok(());
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("pkill")
            .args(["-x", "WorkBuddy"])
            .output();
        if wait_exit(GRACEFUL_WAIT_SECS) {
            return Ok(());
        }
        let _ = std::process::Command::new("pkill")
            .args(["-9", "-x", "WorkBuddy"])
            .output();
        if wait_exit(FORCE_WAIT_SECS) {
            return Ok(());
        }
    }
    Err("无法关闭 WorkBuddy 进程，请手动结束后重试".to_string())
}

fn wait_exit(timeout_secs: u64) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    loop {
        if !workbuddy_running() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

/// 拉起客户端
fn launch_workbuddy(exe_path: &str) -> Result<(), String> {
    if exe_path.is_empty() || !Path::new(exe_path).exists() {
        return Err("未找到 WorkBuddy 安装路径，请到设置页配置或重新探测".to_string());
    }
    std::process::Command::new(exe_path)
        .spawn()
        .map_err(|e| format!("启动 WorkBuddy 失败: {e}"))?;
    Ok(())
}

/// 一键切换编排：备份 → 关进程 → 写认证 → 启动。
/// 写认证失败时从备份还原并尽量拉起客户端，不带病前进。
pub fn switch_to_client(acc: &Account, exe_path: &str) -> Result<String, String> {
    let auth_path = auth_file_path()?;
    let backup = backup_auth_file(&auth_path);

    close_workbuddy()?;

    if let Err(e) = write_account_to_auth_file(acc, &auth_path) {
        // 原子写正常不会留半文件，但 rename 前目标已被删除的极端情况存在：尽力还原
        if let Some(bak) = &backup {
            let _ = std::fs::copy(bak, &auth_path);
        }
        let _ = launch_workbuddy(exe_path);
        return Err(format!("{e}（已还原原认证文件并尝试重启 WorkBuddy）"));
    }

    launch_workbuddy(exe_path)?;
    Ok(match backup {
        Some(p) => format!("已切换并重启 WorkBuddy（原认证已备份到 {}）", p.display()),
        None => "已切换并重启 WorkBuddy".to_string(),
    })
}
