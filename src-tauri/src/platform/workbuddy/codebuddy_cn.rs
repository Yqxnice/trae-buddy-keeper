//! CodeBuddy CN（IDE）账号切换。
//!
//! 实现参考了开源项目 workbuddy-switch。
//!
//! 机制与 WorkBuddy 桌面端完全不同：CodeBuddy CN 是 VS Code 系 IDE，登录态保存在
//! `%APPDATA%\CodeBuddy CN\User\globalStorage\state.vscdb` 的 Safe Storage secret 行
//! （Chromium/Electron Safe Storage），而不是独立认证文件。
//!
//! 切换 = 关闭进程 → 把会话 JSON 加密写入该 secret 行 → 重启。
//! Windows 加密：`Local State` 的 `os_crypt.encrypted_key`（base64，去 "DPAPI" 前缀）
//! 经 DPAPI 解出 32 字节 AES key，AES-256-GCM 加密，密文格式 `v10` + 12 字节 nonce + 密文；
//! 落库值为文本 `{"type":"Buffer","data":[...]}`。

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::data::models::Account;

/// CodeBuddy 系扩展共用的 marketplace 扩展 id（secret key 的 extensionId）
const SECRET_EXTENSION_ID: &str = "tencent-cloud.coding-copilot";
/// CodeBuddy CN 的 secret key（国际版为 planning-genie.new.accessToken，不带 cn 后缀）
const SECRET_KEY: &str = "planning-genie.new.accessTokencn";
/// Windows 进程镜像名（taskkill 用）
const PROCESS_IMAGE: &str = "CodeBuddy CN.exe";
const V10_PREFIX: &[u8] = b"v10";

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Safe Storage secret 行键：secret://{"extensionId":"...","key":"..."}（紧凑 JSON）
fn secret_item_key() -> String {
    format!(
        "secret://{}",
        json!({"extensionId": SECRET_EXTENSION_ID, "key": SECRET_KEY})
    )
}

/// CodeBuddy CN 数据目录（%APPDATA%\CodeBuddy CN）
pub fn data_dir() -> Option<PathBuf> {
    dirs::data_dir().map(|d| d.join("CodeBuddy CN"))
}

/// state.vscdb 路径：按参考项目候选顺序解析，均不存在时用首选路径（并建好父目录）
fn resolve_state_db_path() -> Result<PathBuf, String> {
    let root = data_dir().ok_or("无法定位 CodeBuddy CN 数据目录")?;
    let candidates = [
        root.join("User").join("globalStorage").join("state.vscdb"),
        root.join("globalStorage").join("state.vscdb"),
        root.join("state.vscdb"),
    ];
    if let Some(p) = candidates.iter().find(|p| p.exists()) {
        return Ok(p.clone());
    }
    let preferred = candidates[0].clone();
    if let Some(parent) = preferred.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建 globalStorage 失败: {e}"))?;
    }
    Ok(preferred)
}

/// Local State 位于数据根目录（db 路径上三级）
fn local_state_path(db_path: &Path) -> Result<PathBuf, String> {
    let root = db_path
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .ok_or_else(|| format!("无法从 db 路径推断数据目录: {}", db_path.display()))?;
    let path = root.join("Local State");
    if path.exists() {
        Ok(path)
    } else {
        Err(format!("未找到 Local State: {}", path.display()))
    }
}

// ---------- Windows 加密（DPAPI + AES-256-GCM v10） ----------

#[cfg(target_os = "windows")]
fn dpapi_decrypt(encrypted: &[u8]) -> Result<Vec<u8>, String> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};
    unsafe {
        let mut data_in = CRYPT_INTEGER_BLOB {
            cbData: encrypted.len() as u32,
            pbData: encrypted.as_ptr() as *mut u8,
        };
        let mut data_out = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        CryptUnprotectData(&mut data_in, None, None, None, None, 0, &mut data_out)
            .map_err(|e| format!("DPAPI CryptUnprotectData 失败: {e}"))?;
        if data_out.pbData.is_null() || data_out.cbData == 0 {
            return Err("DPAPI 返回空数据".to_string());
        }
        let result = std::slice::from_raw_parts(data_out.pbData, data_out.cbData as usize).to_vec();
        let _ = LocalFree(HLOCAL(data_out.pbData as _));
        Ok(result)
    }
}

/// 从 Local State 读取 os_crypt.encrypted_key，DPAPI 解出 32 字节 AES key
#[cfg(target_os = "windows")]
fn encryption_key(db_path: &Path) -> Result<Vec<u8>, String> {
    use base64::Engine as _;
    let text = std::fs::read_to_string(local_state_path(db_path)?)
        .map_err(|e| format!("读取 Local State 失败: {e}"))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("解析 Local State 失败: {e}"))?;
    let b64 = v["os_crypt"]["encrypted_key"]
        .as_str()
        .ok_or("Local State 缺少 os_crypt.encrypted_key")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64)
        .map_err(|e| format!("encrypted_key base64 解码失败: {e}"))?;
    if bytes.len() < 6 || &bytes[..5] != b"DPAPI" {
        return Err("encrypted_key 前缀不是 DPAPI".to_string());
    }
    dpapi_decrypt(&bytes[5..])
}

/// v10 加密：b"v10" + 12 字节随机 nonce + AES-256-GCM 密文（对齐参考项目）
#[cfg(target_os = "windows")]
fn encrypt_v10(key: &[u8], plaintext: &[u8]) -> Result<Vec<u8>, String> {
    use aes_gcm::aead::{Aead, AeadCore, OsRng};
    use aes_gcm::{Aes256Gcm, KeyInit};
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| format!("AES key 无效: {e}"))?;
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| format!("AES-GCM 加密失败: {e}"))?;
    let mut out = Vec::with_capacity(V10_PREFIX.len() + nonce.len() + ciphertext.len());
    out.extend_from_slice(V10_PREFIX);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

#[cfg(not(target_os = "windows"))]
fn encryption_key(_db_path: &Path) -> Result<Vec<u8>, String> {
    Err("当前仅支持 Windows 注入 CodeBuddy CN".to_string())
}

#[cfg(not(target_os = "windows"))]
fn encrypt_v10(_key: &[u8], _plaintext: &[u8]) -> Result<Vec<u8>, String> {
    Err("当前仅支持 Windows 注入 CodeBuddy CN".to_string())
}

// ---------- 会话 JSON（对齐参考实现） ----------

/// 复合 token：`<uid>+<access_token>`；无 uid 时退化为裸 token
fn format_token(acc: &Account) -> String {
    if acc.uid.is_empty() {
        acc.access_token.clone()
    } else {
        format!("{}+{}", acc.uid, acc.access_token)
    }
}

fn build_session_json(acc: &Account) -> String {
    let domain = if acc.domain.trim().is_empty() {
        "www.codebuddy.cn"
    } else {
        acc.domain.trim()
    };
    let token = format_token(acc);
    json!({
        "id": "Tencent-Cloud.genie-ide-cn",
        "token": token,
        "refreshToken": acc.refresh_token,
        "expiresAt": acc.token_expires_at,
        "domain": domain,
        "accessToken": token,
        "converted": true,
        "account": {
            "id": acc.uid,
            "uid": acc.uid,
            "label": acc.name,
            "nickname": acc.name,
            "enterpriseId": acc.enterprise_id,
            "enterpriseName": "",
            "pluginEnabled": true,
            "lastLogin": true
        },
        "auth": {
            "accessToken": acc.access_token,
            "refreshToken": acc.refresh_token,
            "tokenType": "Bearer",
            "domain": domain,
            "expiresAt": acc.token_expires_at,
            "expiresIn": 0,
            "refreshExpiresIn": 0,
            "refreshExpiresAt": 0,
            "lastRefreshTime": now_ms()
        }
    })
    .to_string()
}

// ---------- 注入 ----------

/// 加密写入 CodeBuddy CN 登录态 secret；返回 db 路径
fn inject_session(plaintext: &str) -> Result<PathBuf, String> {
    let db_path = resolve_state_db_path()?;
    let key = encryption_key(&db_path)?;
    let encrypted = encrypt_v10(&key, plaintext.as_bytes())?;
    let buffer_str = json!({"type": "Buffer", "data": encrypted}).to_string();

    let conn = rusqlite::Connection::open(&db_path)
        .map_err(|e| format!("打开 state.vscdb 失败: {e}"))?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS ItemTable (key TEXT PRIMARY KEY, value TEXT)",
        [],
    )
    .map_err(|e| format!("初始化 ItemTable 失败: {e}"))?;
    let db_key = secret_item_key();
    conn.execute(
        "INSERT OR REPLACE INTO ItemTable (key, value) VALUES (?, ?)",
        rusqlite::params![db_key, buffer_str],
    )
    .map_err(|e| format!("写入 state.vscdb 失败: {e}"))?;

    // 写后校验：行存在且为 Buffer JSON
    let written: String = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = ?",
            [db_key.as_str()],
            |r| r.get(0),
        )
        .map_err(|e| format!("写后校验失败: {e}"))?;
    let parsed: Value =
        serde_json::from_str(&written).map_err(|_| "写后校验：值不是 JSON".to_string())?;
    let len = parsed["data"].as_array().map(|a| a.len()).unwrap_or(0);
    if len < V10_PREFIX.len() + 12 {
        return Err("写后校验：Buffer 数据异常".to_string());
    }
    Ok(db_path)
}

// ---------- 进程控制（对齐同目录 switch.rs 的方式） ----------

#[cfg(target_os = "windows")]
fn client_running() -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    std::process::Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {PROCESS_IMAGE}")])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(PROCESS_IMAGE))
        .unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
fn client_running() -> bool {
    false
}

fn wait_exit(timeout_secs: u64) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    while std::time::Instant::now() < deadline {
        if !client_running() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    false
}

/// 先优雅关闭（/T），超时后强制（/T /F）
#[cfg(target_os = "windows")]
fn close_client() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    for attempt in 0..2 {
        let mut args = vec!["/IM", PROCESS_IMAGE, "/T"];
        if attempt == 1 {
            args.push("/F");
        }
        let _ = std::process::Command::new("taskkill")
            .args(&args)
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        if wait_exit(20) {
            return Ok(());
        }
    }
    Err(format!("{PROCESS_IMAGE} 进程无法关闭，请手动结束后重试"))
}

#[cfg(not(target_os = "windows"))]
fn close_client() -> Result<(), String> {
    Err("当前仅支持 Windows".to_string())
}

fn launch(exe_path: &str) -> Result<(), String> {
    std::process::Command::new(exe_path)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("启动 {exe_path} 失败: {e}"))
}

// ---------- 切换入口 ----------

/// 切换 CodeBuddy CN 到指定账号：关进程 → 注入登录态 → 重启。
/// 账号与 WorkBuddy 国内版同体系（access_token/refresh_token 通用）。
pub fn switch_to_client(acc: &Account, exe_path: &str) -> Result<String, String> {
    if acc.access_token.trim().is_empty() {
        return Err("该账号没有 access_token，请重新登录".to_string());
    }
    if !PathBuf::from(exe_path).exists() {
        return Err(format!("CodeBuddy CN 可执行文件不存在: {exe_path}"));
    }
    close_client()?;
    let session = build_session_json(acc);
    let db_path = inject_session(&session)?;
    launch(exe_path)?;
    Ok(format!("已写入 {}", db_path.display()))
}
