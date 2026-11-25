//! Qoder CN 登录态快照：按白名单整份搬运客户端数据目录，不改写客户端凭证。
//!
//! 客户端存在两代互不相通的目录布局，切换前必须先识别，否则会把登录态写进
//! 客户端根本不读的目录，表现为「切换成功但客户端账号没变」：
//! - 桌面 App：`%APPDATA%\<dataDirectoryName>`（当前为 `com.qodercn.app.stable`），
//!   登录态在根级 `auth.v1.dat`（`v10` + AES-256-GCM，密钥取自 `Local State`
//!   的 `os_crypt.encrypted_key`，该密钥由 DPAPI 保护），账号缓存在 leveldb / Network；
//! - 旧版 IDE fork：`%APPDATA%\QoderCN` 或 `%APPDATA%\Qoder`，登录态在
//!   `User/globalStorage/state.vscdb`，恢复前要清掉 SQLite 边车。
//!
//! 目录名优先从安装目录 `resources/product.json#dataDirectoryName` 读取，避免写死。
//! 切换前把静止现场存成持久 `last` 槽并条件回写当前账号档案；覆盖档案前轮转 `.bak`。
//! 启动后用 `auth.v1.dat` 里的真实 uid 判定：只有确认登录到目标账号才算成功，
//! 其余情况一律返回警示且不把「当前已注入客户端」标记指向目标账号。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rusqlite::types::ValueRef;
use rusqlite::{Connection, OpenFlags};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::data::db::Database;
use crate::platform::trae::switch::VaultInfo;

/// 旧版 IDE fork 的登录态载体（VS Code 系布局）。
const IDE_WHITELIST: &[&str] = &[
    "User/globalStorage/storage.json",
    "User/globalStorage/state.vscdb",
    "User/globalStorage/state.vscdb-wal",
    "User/globalStorage/state.vscdb-shm",
    "User/globalStorage/state.vscdb.backup",
    "machineid",
    "Preferences",
    "Local State",
    "Local Storage/leveldb",
    "Network",
    "Session Storage",
    "SharedClientCache/cache/id",
    "SharedClientCache/cache/machine_token.json",
    "SharedClientCache/cache/client.json",
    "SharedClientCache/cache/status.json",
];

/// 桌面 App 的登录态载体。`main.sqlite` 是业务库（会话 / 设置），不随账号搬运，
/// 否则切换会把用户聊天记录一并回滚；登录身份的权威只在 `auth.v1.dat`。
const APP_WHITELIST: &[&str] = &[
    "auth.v1.dat",
    "auth-profile-overlays.v1.dat",
    "auth.machine-id",
    "Local State",
    "Preferences",
    "Local Storage/leveldb",
    "Session Storage",
    "Network",
    "SharedStorage",
    "channel-activation.v1.json",
    "first-launch-onboarding.v1.json",
    "notification-preferences.v1.json",
];

const IDE_SIDECARS: &[&str] = &[
    "User/globalStorage/state.vscdb-wal",
    "User/globalStorage/state.vscdb-shm",
    "code.lock",
];
const APP_SIDECARS: &[&str] = &[];

/// 删掉这些文件会让密文解不开或丢失设备绑定，档案缺项时保留 live。
const IDE_KEEP: &[&str] = &[
    "Local State",
    "machineid",
    "Preferences",
    "SharedClientCache/cache/id",
];
const APP_KEEP: &[&str] = &["Local State", "auth.machine-id", "Preferences"];

const IDE_LOGIN: &str = "User/globalStorage/state.vscdb";
const APP_LOGIN: &str = "auth.v1.dat";
const IDE_MACHINE: &str = "machineid";
const APP_MACHINE: &str = "auth.machine-id";
const DEFAULT_APP_DIR: &str = "com.qodercn.app.stable";
const LAST_SLOT: &str = "last";
const GRACEFUL_WAIT_SECS: u64 = 8;
const FORCE_WAIT_SECS: u64 = 6;
const VERIFY_WINDOW_SECS: u64 = 20;
const VERIFY_POLL_MS: u64 = 500;
/// 连续 4 次采样都读到目标身份（约 2 秒）才提前收尾，避开客户端启动瞬态。
const VERIFY_STABLE_SAMPLES: u32 = 4;
/// 曾经出现过凭据又连续 6 次采样为空（约 3 秒）才判定客户端真的清除了登录态。
const VERIFY_CLEAR_SAMPLES: u32 = 6;

/// 切换时由命令层注入的上下文：避免切换模块直接依赖数据库连接。
#[derive(Debug, Default)]
pub struct SwitchContext {
    /// 切换前标记在客户端里的账号；用于把静止现场回写进它的档案
    pub current_account_id: Option<String>,
    /// 目标账号的平台 uid
    pub target_uid: String,
    /// 账号池内其它 Qoder 账号的 uid，用于识别串台
    pub other_uids: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Layout {
    DesktopApp,
    IdeFork,
}

/// 一次切换 / 建档所使用的数据目录与载体集合。
#[derive(Debug)]
pub struct LayoutSpec {
    pub layout: Layout,
    pub root: PathBuf,
    pub whitelist: &'static [&'static str],
    pub sidecars: &'static [&'static str],
    pub keep: &'static [&'static str],
    pub auth_file: &'static str,
    pub machine_file: &'static str,
}

fn app_spec(root: PathBuf) -> LayoutSpec {
    LayoutSpec {
        layout: Layout::DesktopApp,
        root,
        whitelist: APP_WHITELIST,
        sidecars: APP_SIDECARS,
        keep: APP_KEEP,
        auth_file: APP_LOGIN,
        machine_file: APP_MACHINE,
    }
}

fn ide_spec(root: PathBuf) -> LayoutSpec {
    LayoutSpec {
        layout: Layout::IdeFork,
        root,
        whitelist: IDE_WHITELIST,
        sidecars: IDE_SIDECARS,
        keep: IDE_KEEP,
        auth_file: IDE_LOGIN,
        machine_file: IDE_MACHINE,
    }
}

/// 从安装目录的 `resources/product.json` 读取真实数据目录名；读不到返回 None。
pub fn parse_data_dir_name(body: &Value) -> Option<String> {
    let name = body
        .get("dataDirectoryName")
        .and_then(Value::as_str)
        .or_else(|| body.get("dataFolderName").and_then(Value::as_str))?
        .trim();
    if name.is_empty() || name.contains('/') || name.contains('\\') || name.contains("..") {
        return None;
    }
    Some(name.to_string())
}

fn product_data_dir_name() -> Option<String> {
    let mut exes = Vec::new();
    let detected = crate::infra::clients::detect_client_paths().qoder_cn;
    if is_qoder_cn_exe(&detected) {
        exes.push(PathBuf::from(detected));
    }
    exes.extend(exe_candidates());
    for exe in exes {
        if !exe.exists() {
            continue;
        }
        let Some(parent) = exe.parent() else { continue };
        let text = std::fs::read_to_string(parent.join("resources/product.json")).ok()?;
        let body: Value = serde_json::from_str(&text).ok()?;
        if let Some(name) = parse_data_dir_name(&body) {
            return Some(name);
        }
    }
    None
}

/// 识别当前安装的客户端布局：桌面 App 目录优先，旧版 IDE fork 仅作兼容回退。
pub fn detect_layout() -> Result<LayoutSpec, String> {
    let base = dirs::data_dir().ok_or_else(|| "无法定位应用数据目录".to_string())?;
    let mut names = Vec::new();
    if let Some(name) = product_data_dir_name() {
        names.push(name);
    }
    names.push(DEFAULT_APP_DIR.to_string());
    for name in names {
        let root = base.join(name);
        if root.is_dir() {
            return Ok(app_spec(root));
        }
    }
    for name in ["QoderCN", "Qoder"] {
        let root = base.join(name);
        if root.is_dir() {
            return Ok(ide_spec(root));
        }
    }
    // 客户端尚未启动过：仍按当前桌面 App 布局返回，建档时会给出「请先登录」提示。
    Ok(app_spec(base.join(DEFAULT_APP_DIR)))
}

/// 客户端数据根（当前布局）；设备身份读取不要用它，改用 `native_identity_roots`。
pub fn data_root() -> Result<PathBuf, String> {
    Ok(detect_layout()?.root)
}

/// 设备级身份文件的候选根：两代客户端都可能有，逐个探测而不绑定登录态布局。
pub fn native_identity_roots() -> Vec<PathBuf> {
    let Some(base) = dirs::data_dir() else {
        return Vec::new();
    };
    ["Qoder", "QoderCN", DEFAULT_APP_DIR]
        .iter()
        .map(|name| base.join(name))
        .collect()
}

/// 默认安装路径候选：只认 Qoder CN 本体的两种主程序名，
/// 其它 Qoder 产品线（Qoder IDE / Qoder Work）与设置页配置不在此混用。
const QODER_CN_EXE_NAMES: [&str; 2] = ["Qoder CN.exe", "Qoder CN IDE.exe"];

/// 自动探测/历史配置的结果必须是 Qoder CN 本体，避免启动到别的 Qoder 客户端。
pub fn is_qoder_cn_exe(path: &str) -> bool {
    let name = Path::new(path.trim())
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    name.starts_with("qoder cn") || name.starts_with("qoder_cn") || name.starts_with("qodercn")
}

pub fn exe_candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let dir_names = ["Qoder CN IDE", "Qoder CN"];
    if let Some(local) = dirs::data_local_dir() {
        for dir_name in dir_names {
            let ide = local.join("Programs").join(dir_name);
            for exe in QODER_CN_EXE_NAMES {
                out.push(ide.join(exe));
            }
            let versions = ide.join(".qoder-versions");
            if let Ok(entries) = std::fs::read_dir(versions) {
                for entry in entries.flatten() {
                    for exe in QODER_CN_EXE_NAMES {
                        let p = entry.path().join(exe);
                        if p.is_file() {
                            out.push(p);
                        }
                    }
                }
            }
        }
    }
    if let Ok(program_files) = std::env::var("ProgramFiles") {
        for dir_name in dir_names {
            for exe in QODER_CN_EXE_NAMES {
                out.push(PathBuf::from(&program_files).join(dir_name).join(exe));
            }
        }
    }
    out
}

pub fn resolve_exe(db: &Database) -> String {
    let configured = db.load_app_settings().client_path_qoder_cn;
    // 历史误配：自动探测曾把其它 Qoder 产品线的 exe 写进设置，一律不再采用
    if is_qoder_cn_exe(&configured) && Path::new(configured.trim()).exists() {
        return configured;
    }
    let detected = crate::infra::clients::detect_client_paths().qoder_cn;
    if is_qoder_cn_exe(&detected) && Path::new(detected.trim()).exists() {
        return detected;
    }
    exe_candidates()
        .into_iter()
        .find(|p| p.exists())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn vault_root(data_dir: &Path) -> PathBuf {
    data_dir.join("vaults/qoder_cn")
}

fn vault_dir(data_dir: &Path, account_id: &str) -> PathBuf {
    vault_root(data_dir).join(account_id)
}

fn last_dir(data_dir: &Path) -> PathBuf {
    vault_root(data_dir).join(LAST_SLOT)
}

fn bak_dir(dir: &Path) -> PathBuf {
    PathBuf::from(format!("{}.bak", dir.display()))
}

fn client_image(exe: &str) -> String {
    if exe.trim().is_empty() {
        return QODER_CN_EXE_NAMES[0].to_string();
    }
    Path::new(exe)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| QODER_CN_EXE_NAMES[0].to_string())
}

#[cfg(target_os = "windows")]
fn is_running(image: &str) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    std::process::Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {image}"), "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| {
            let s = String::from_utf8_lossy(&o.stdout).to_ascii_lowercase();
            s.contains(&image.to_ascii_lowercase())
        })
        .unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
fn is_running(image: &str) -> bool {
    std::process::Command::new("pgrep")
        .args(["-f", image.trim_end_matches(".exe")])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn is_running_for_exe(exe: &str) -> bool {
    if exe.trim().is_empty() {
        // 路径未知时只判 Qoder CN 本体的两种进程名，不碰其它 Qoder 产品线
        return QODER_CN_EXE_NAMES.iter().any(|name| is_running(name));
    }
    is_running(&client_image(exe))
}

fn wait_exit(image: &str, timeout_secs: u64) -> bool {
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    while Instant::now() < deadline {
        if !is_running(image) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    !is_running(image)
}

fn kill_client(image: &str) -> Result<(), String> {
    if !is_running(image) {
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let _ = std::process::Command::new("taskkill")
            .args(["/IM", image])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        if wait_exit(image, GRACEFUL_WAIT_SECS) {
            return Ok(());
        }
        let _ = std::process::Command::new("taskkill")
            .args(["/IM", image, "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        if wait_exit(image, FORCE_WAIT_SECS) {
            return Ok(());
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = std::process::Command::new("pkill")
            .args(["-f", image.trim_end_matches(".exe")])
            .output();
        if wait_exit(image, GRACEFUL_WAIT_SECS) {
            return Ok(());
        }
        let _ = std::process::Command::new("pkill")
            .args(["-9", "-f", image.trim_end_matches(".exe")])
            .output();
        if wait_exit(image, FORCE_WAIT_SECS) {
            return Ok(());
        }
    }
    Err("无法关闭 Qoder CN 进程，请手动结束后重试".to_string())
}

fn launch_client(exe: &str) -> Result<(), String> {
    if exe.is_empty() || !Path::new(exe).exists() {
        return Err("未找到 Qoder CN 安装路径，请到设置页配置客户端路径".to_string());
    }
    std::process::Command::new(exe)
        .spawn()
        .map_err(|e| format!("启动 Qoder CN 失败: {e}"))?;
    Ok(())
}

fn enumerate_tree(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_dir() {
            enumerate_tree(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn overwrite_entry(src_root: &Path, dst_root: &Path, rel: &str) {
    let src = src_root.join(rel);
    let dst = dst_root.join(rel);
    if src.is_dir() {
        if dst.exists() {
            let _ = std::fs::remove_dir_all(&dst);
        }
        let mut files = Vec::new();
        enumerate_tree(&src, &mut files);
        for f in files {
            if let Ok(rel_f) = f.strip_prefix(&src) {
                let target = dst.join(rel_f);
                if let Some(parent) = target.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let _ = std::fs::copy(&f, &target);
            }
        }
    } else if src.is_file() {
        if let Some(parent) = dst.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if dst.exists() {
            let _ = std::fs::remove_file(&dst);
        }
        let _ = std::fs::copy(&src, &dst);
    }
}

/// 对称恢复：档案有的项覆盖，缺的项从 live 删除（设备绑定类除外）。
fn restore_entry_set(src_root: &Path, dst_root: &Path, spec: &LayoutSpec) -> Result<(), String> {
    for &side in spec.sidecars {
        let p = dst_root.join(side);
        if p.exists() {
            std::fs::remove_file(&p).map_err(|e| format!("删除 {} 失败: {e}", p.display()))?;
        }
    }
    for &rel in spec.whitelist {
        let src = src_root.join(rel);
        let dst = dst_root.join(rel);
        if src.exists() {
            overwrite_entry(src_root, dst_root, rel);
        } else if !spec.keep.contains(&rel) {
            if dst.is_dir() {
                let _ = std::fs::remove_dir_all(&dst);
            } else if dst.exists() {
                let _ = std::fs::remove_file(&dst);
            }
        }
    }
    Ok(())
}

fn snapshot_entry(src_root: &Path, dst_root: &Path, rel: &str) -> bool {
    let src = src_root.join(rel);
    overwrite_entry(src_root, dst_root, rel);
    src.exists()
}

fn write_meta(
    dir: &Path,
    slot: &str,
    count: usize,
    layout: Layout,
    uid: Option<&str>,
) -> Result<(), String> {
    let meta = json!({
        "account_id": slot,
        "created_at": chrono::Utc::now().timestamp(),
        "entries": count,
        "verify_failed_at": Value::Null,
        "layout": match layout {
            Layout::DesktopApp => "desktop_app",
            Layout::IdeFork => "ide_fork",
        },
        "uid": uid.unwrap_or(""),
    });
    std::fs::write(
        dir.join("meta.json"),
        serde_json::to_string_pretty(&meta).unwrap_or_default(),
    )
    .map_err(|e| format!("写入档案元数据失败: {e}"))
}

/// 覆盖前把上一份档案挪到 `.bak`，避免拷贝中断永久丢失旧登录态。
fn rotate_backup(dir: &Path) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }
    let bak = bak_dir(dir);
    if bak.exists() {
        std::fs::remove_dir_all(&bak).map_err(|e| format!("清理旧回退档失败: {e}"))?;
    }
    if let Some(parent) = bak.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建档案目录失败: {e}"))?;
    }
    std::fs::rename(dir, &bak).map_err(|e| format!("轮转旧档案失败: {e}"))
}

/// 把静止的客户端现场整份快照到指定槽位，覆盖前轮转 `.bak`。
fn save_vault(root: &Path, dest: &Path, slot: &str, spec: &LayoutSpec) -> Result<usize, String> {
    rotate_backup(dest)?;
    std::fs::create_dir_all(dest).map_err(|e| format!("创建档案目录失败: {e}"))?;
    let mut count = 0usize;
    for &rel in spec.whitelist {
        if snapshot_entry(root, dest, rel) {
            count += 1;
        }
    }
    let uid = if spec.layout == Layout::DesktopApp {
        read_auth_uid(root)
    } else {
        None
    };
    write_meta(dest, slot, count, spec.layout, uid.as_deref())?;
    Ok(count)
}

fn vault_valid(dir: &Path, spec: &LayoutSpec) -> bool {
    dir.join("meta.json").is_file() && dir.join(spec.auth_file).is_file()
}

fn resolve_vault(data_dir: &Path, account_id: &str, spec: &LayoutSpec) -> Result<PathBuf, String> {
    let main = vault_dir(data_dir, account_id);
    if vault_valid(&main, spec) {
        return Ok(main);
    }
    let bak = bak_dir(&main);
    if vault_valid(&bak, spec) {
        return Ok(bak);
    }
    if !main.join("meta.json").exists() && !bak.join("meta.json").exists() {
        return Err("该账号尚未建档：先让客户端登录该账号，退出后点「存档」".to_string());
    }
    Err(format!(
        "档案与当前客户端不匹配（缺少 {}）：旧版客户端目录的档案不能用于当前 Qoder CN。请在客户端登录该账号、完全退出后点「存档」重新建档",
        spec.auth_file
    ))
}

/// 账号档案里的客户端真实设备号；主档损坏时回退 `.bak`。
/// 设备号读取与「当前客户端是哪一代布局」无关：两代文件名都试，避免旧档案失效。
/// 同一台机器上多个账号建档得到的是同一个值，调用方必须做账号间去重。
pub fn vault_machine_id(data_dir: &Path, account_id: &str) -> Option<String> {
    let main = vault_dir(data_dir, account_id);
    let dir = [main.clone(), bak_dir(&main)].into_iter().find(|d| {
        d.join("meta.json").is_file()
            && (d.join(APP_LOGIN).is_file() || d.join(IDE_LOGIN).is_file())
    })?;
    let raw = [APP_MACHINE, IDE_MACHINE]
        .iter()
        .find_map(|name| std::fs::read_to_string(dir.join(name)).ok())?;
    let s = raw.trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// 查询档案元数据；旧档缺少验证字段时视为未打标。
pub fn vault_info(data_dir: &Path, account_id: &str) -> Option<VaultInfo> {
    let text = std::fs::read_to_string(vault_dir(data_dir, account_id).join("meta.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    Some(VaultInfo {
        created_at: v.get("created_at")?.as_i64()?,
        entries: v.get("entries")?.as_u64()? as usize,
        verify_failed_at: v
            .get("verify_failed_at")
            .and_then(Value::as_i64)
            .filter(|t| *t > 0),
    })
}

pub fn mark_vault_verify(data_dir: &Path, account_id: &str, verified: bool) -> Result<(), String> {
    let path = vault_dir(data_dir, account_id).join("meta.json");
    let text = std::fs::read_to_string(&path).unwrap_or_default();
    let mut v: Value = serde_json::from_str(&text).unwrap_or_else(|_| json!({}));
    if verified {
        v["verify_failed_at"] = Value::Null;
    } else {
        v["verify_failed_at"] = json!(chrono::Utc::now().timestamp());
    }
    std::fs::write(&path, serde_json::to_string_pretty(&v).unwrap_or_default())
        .map_err(|e| format!("更新档案元数据失败: {e}"))
}

/// 解出桌面 App `Local State` 里的 AES 密钥（DPAPI 保护），仅用于本机只读核验。
fn app_aes_key(root: &Path) -> Option<Vec<u8>> {
    use base64::Engine;
    let text = std::fs::read_to_string(root.join("Local State")).ok()?;
    let body: Value = serde_json::from_str(&text).ok()?;
    let b64 = body.get("os_crypt")?.get("encrypted_key")?.as_str()?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    if bytes.len() < 6 || &bytes[..5] != b"DPAPI" {
        return None;
    }
    crate::data::crypto::unprotect_bytes(&bytes[5..]).ok()
}

/// `v10` = 3 字节前缀 + 12 字节 nonce + AES-256-GCM 密文（末尾 16 字节 tag）。
fn decrypt_auth_blob(blob: &[u8], key: &[u8]) -> Option<String> {
    use aes_gcm::aead::{generic_array::GenericArray, Aead, KeyInit};
    use aes_gcm::Aes256Gcm;
    const V10: &[u8] = b"v10";
    if blob.len() < V10.len() + 12 + 16 || &blob[..V10.len()] != V10 {
        return None;
    }
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    let nonce = GenericArray::from_slice(&blob[V10.len()..V10.len() + 12]);
    let plain = cipher.decrypt(nonce, &blob[V10.len() + 12..]).ok()?;
    let body: Value = serde_json::from_slice(&plain).ok()?;
    let uid = body.get("user")?.get("id")?.as_str()?;
    (!uid.trim().is_empty()).then(|| uid.trim().to_string())
}

/// 只读解出客户端当前登录账号的 uid。解密失败一律返回 None，绝不猜测或伪造。
fn read_auth_uid(root: &Path) -> Option<String> {
    let blob = std::fs::read(root.join(APP_LOGIN)).ok()?;
    let key = app_aes_key(root)?;
    decrypt_auth_blob(&blob, &key)
}

fn auth_present(root: &Path) -> bool {
    std::fs::metadata(root.join(APP_LOGIN))
        .map(|m| m.len() > 3)
        .unwrap_or(false)
}

/// 只读扫一遍旧版 IDE 登录态数据库：统计认证类键，并收集短文本里的身份线索。
/// 打不开（运行中被锁 / 布局不同构）返回 None，调用方一律按「无证据」处理。
struct LoginSignal {
    cred_entries: usize,
    text: String,
}

fn read_login_signal(root: &Path) -> Option<LoginSignal> {
    let path = root.join(IDE_LOGIN);
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(&path, flags).ok()?;
    conn.busy_timeout(Duration::from_millis(300)).ok()?;
    let mut stmt = conn.prepare("SELECT key, value FROM ItemTable").ok()?;
    let mut rows = stmt.query([]).ok()?;
    let mut cred_entries = 0usize;
    let mut text = String::new();
    while let Some(row) = rows.next().ok().flatten() {
        let key = row.get::<_, String>(0).unwrap_or_default();
        let bytes: &[u8] = match row.get_ref(1) {
            Ok(ValueRef::Text(t)) => t,
            Ok(ValueRef::Blob(b)) => b,
            _ => &[],
        };
        let lower = key.to_ascii_lowercase();
        if !bytes.is_empty()
            && (lower.contains("auth") || lower.contains("userinfo") || lower.contains("secret://"))
        {
            cred_entries += 1;
        }
        if bytes.len() < 4096 {
            if let Ok(s) = std::str::from_utf8(bytes) {
                text.push_str(s);
                text.push('\n');
            }
        }
    }
    Some(LoginSignal { cred_entries, text })
}

fn live_has_login(root: &Path, spec: &LayoutSpec) -> Option<bool> {
    if spec.layout == Layout::DesktopApp {
        return Some(auth_present(root));
    }
    read_login_signal(root).map(|s| s.cred_entries > 0)
}

/// 切换前归档当前现场：last 槽无条件保留；回写当前账号档案要过登录态守卫，
/// 避免把「用户已手动登出」的现场覆盖掉一份完好的档案。
fn archive_current(
    data_dir: &Path,
    spec: &LayoutSpec,
    ctx: &SwitchContext,
    target: &str,
) -> String {
    let root = &spec.root;
    let mut notes = Vec::new();
    match save_vault(root, &last_dir(data_dir), LAST_SLOT, spec) {
        Ok(n) => notes.push(format!("切换前现场已存为 last 槽（{n} 项）")),
        Err(e) => notes.push(format!("last 槽保存失败：{e}")),
    }
    let Some(current) = ctx
        .current_account_id
        .as_deref()
        .filter(|v| !v.is_empty() && *v != target)
    else {
        return notes.join("；");
    };
    let dir = vault_dir(data_dir, current);
    if !vault_valid(&dir, spec) {
        return notes.join("；");
    }
    match live_has_login(root, spec) {
        Some(true) => match save_vault(root, &dir, current, spec) {
            Ok(n) => notes.push(format!(
                "已把当前登录态回写到账号 {current} 的档案（{n} 项）"
            )),
            Err(e) => notes.push(format!("回写当前账号档案失败：{e}")),
        },
        Some(false) => {
            notes.push("当前客户端未检测到登录态，已跳过回写账号档案，避免污染".to_string())
        }
        None => notes.push("无法读取当前登录态，已跳过回写账号档案".to_string()),
    }
    notes.join("；")
}

#[derive(Debug, PartialEq, Eq)]
enum VerifyVerdict {
    Confirmed,
    Alive,
    Indeterminate,
    Mismatch(String),
}

/// 桌面 App 布局：用凭据里的真实 uid 判定，不凭「文件写回 / 进程起来了」宣称成功。
fn verify_desktop(root: &Path, ctx: &SwitchContext) -> VerifyVerdict {
    let deadline = Instant::now() + Duration::from_secs(VERIFY_WINDOW_SECS);
    let mut ever_present = false;
    let mut missing = 0u32;
    let mut matched = 0u32;
    let mut last_uid: Option<String> = None;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(VERIFY_POLL_MS));
        if auth_present(root) {
            ever_present = true;
            missing = 0;
            if let Some(uid) = read_auth_uid(root) {
                if !ctx.target_uid.is_empty() && uid == ctx.target_uid {
                    matched += 1;
                    last_uid = Some(uid);
                    if matched >= VERIFY_STABLE_SAMPLES {
                        break;
                    }
                } else {
                    matched = 0;
                    last_uid = Some(uid);
                }
            }
        } else if ever_present {
            matched = 0;
            missing += 1;
            if missing >= VERIFY_CLEAR_SAMPLES {
                break;
            }
        }
    }
    if ever_present && missing >= VERIFY_CLEAR_SAMPLES {
        return VerifyVerdict::Mismatch(
            "客户端启动后清除了登录凭据（多为 refresh token 已被服务端轮换/失效）".to_string(),
        );
    }
    if ctx.target_uid.is_empty() {
        return if ever_present {
            VerifyVerdict::Alive
        } else {
            VerifyVerdict::Indeterminate
        };
    }
    match last_uid {
        Some(uid) if uid == ctx.target_uid => VerifyVerdict::Confirmed,
        Some(uid) => {
            VerifyVerdict::Mismatch(format!("客户端当前登录账号（uid {uid}）与目标账号不一致"))
        }
        None if ever_present => VerifyVerdict::Alive,
        None => VerifyVerdict::Indeterminate,
    }
}

/// 旧版 IDE 布局：只读扫描登录态数据库，明确读到别的 uid / 凭据被清除才判异常。
fn verify_ide(root: &Path, ctx: &SwitchContext) -> VerifyVerdict {
    let deadline = Instant::now() + Duration::from_secs(VERIFY_WINDOW_SECS);
    let mut ever_present = false;
    let mut stable = 0u32;
    let mut missing = 0u32;
    let mut unreadable = 0u32;
    let mut target_seen = false;
    let mut other_uid: Option<String> = None;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(VERIFY_POLL_MS));
        let Some(signal) = read_login_signal(root) else {
            unreadable += 1;
            if unreadable >= VERIFY_CLEAR_SAMPLES {
                break;
            }
            continue;
        };
        unreadable = 0;
        if signal.cred_entries > 0 {
            ever_present = true;
            missing = 0;
            stable += 1;
            if !ctx.target_uid.is_empty() && signal.text.contains(&ctx.target_uid) {
                target_seen = true;
            }
            if !target_seen {
                for uid in &ctx.other_uids {
                    if !uid.is_empty() && signal.text.contains(uid) {
                        other_uid = Some(uid.clone());
                    }
                }
            }
            if other_uid.is_some() || target_seen || stable >= VERIFY_STABLE_SAMPLES {
                break;
            }
        } else if ever_present {
            stable = 0;
            missing += 1;
            if missing >= VERIFY_CLEAR_SAMPLES {
                break;
            }
        }
    }
    if let Some(uid) = other_uid {
        return VerifyVerdict::Mismatch(format!("读取到的登录身份（uid {uid}）与目标账号不一致"));
    }
    if ever_present && missing >= VERIFY_CLEAR_SAMPLES {
        return VerifyVerdict::Mismatch(
            "客户端启动后清除了登录凭据（多为 refresh token 已被服务端轮换/失效）".to_string(),
        );
    }
    if target_seen {
        return VerifyVerdict::Confirmed;
    }
    if ever_present {
        return VerifyVerdict::Alive;
    }
    VerifyVerdict::Indeterminate
}

fn verify_login(spec: &LayoutSpec, ctx: &SwitchContext) -> VerifyVerdict {
    if spec.layout == Layout::DesktopApp {
        verify_desktop(&spec.root, ctx)
    } else {
        verify_ide(&spec.root, ctx)
    }
}

/// 只有确认登录到目标账号才返回 `confirmed=true`；其余情况一律警示，
/// 避免「写回成功」被误当成「客户端已切换到该账号」。
fn switch_message(verdict: VerifyVerdict, note: &str) -> (String, bool) {
    let suffix = if note.is_empty() {
        String::new()
    } else {
        format!("\n\n{note}")
    };
    let tail = "客户端保持当前状态、未回滚——请确认客户端里显示的是哪个账号；若不是目标账号，直接登录该账号，然后回这里点「存档」重新建档，或点「恢复切换前现场」回退。";
    match verdict {
        VerifyVerdict::Confirmed => (
            format!("已切换并重启 Qoder CN，已确认客户端登录凭据属于目标账号。{suffix}"),
            true,
        ),
        VerifyVerdict::Alive => (
            format!("SWITCH_WARN:已重启 Qoder CN，但读不出客户端当前登录账号，不能确认切换成功。{tail}{suffix}"),
            false,
        ),
        VerifyVerdict::Indeterminate => (
            format!("SWITCH_WARN:已重启 Qoder CN，但读取不到客户端登录凭据，不能确认切换成功。{tail}{suffix}"),
            false,
        ),
        VerifyVerdict::Mismatch(why) => (
            format!("SWITCH_WARN:已重启 Qoder CN，切换结果与目标账号不符：{why}。{tail}{suffix}"),
            false,
        ),
    }
}

/// 建档：把当前客户端登录现场整份快照到账号目录。
pub fn backup_vault(
    data_dir: &Path,
    account_id: &str,
    exe: &str,
    close_running: bool,
) -> Result<String, String> {
    let spec = detect_layout()?;
    let root = &spec.root;
    let image = client_image(exe);
    if is_running(&image) {
        if !close_running {
            return Err("CLIENT_RUNNING: Qoder CN 仍在运行，请先完全退出客户端".to_string());
        }
        kill_client(&image)?;
    }
    if !root.is_dir() {
        return Err(format!(
            "未找到 Qoder CN 客户端数据目录（{}），请先登录一次客户端",
            root.display()
        ));
    }
    if !root.join(spec.auth_file).is_file() {
        return Err(format!(
            "当前客户端没有登录态数据（缺少 {}），请先在 Qoder CN 登录该账号再建档",
            spec.auth_file
        ));
    }
    let count = save_vault(root, &vault_dir(data_dir, account_id), account_id, &spec)?;
    Ok(format!("建档成功，已保存 {count} 项登录态载体"))
}

/// 把 last 槽（上一次切换前的现场）恢复到客户端，用于人工兜底回退。
pub fn restore_last_vault(data_dir: &Path, exe: &str) -> Result<String, String> {
    let spec = detect_layout()?;
    let last = last_dir(data_dir);
    if !vault_valid(&last, &spec) {
        return Err("没有可恢复的切换前现场：last 槽缺失或不含登录态数据".to_string());
    }
    kill_client(&client_image(exe))?;
    if let Err(e) = restore_entry_set(&last, &spec.root, &spec) {
        return Err(format!("恢复切换前现场失败：{e}"));
    }
    if exe.is_empty() || !Path::new(exe).exists() {
        return Ok("已恢复切换前现场；未找到客户端路径，请手动启动 Qoder CN".to_string());
    }
    launch_client(exe)?;
    Ok("已恢复切换前现场并重启 Qoder CN，请在客户端确认账号".to_string())
}

/// 切换：关进程 → 归档当前现场 → 写回目标档案 → 拉起客户端 → 登录身份核验。
/// 只在客户端根本没起来时回滚；其他情况保留现场并如实告知，留重新登录的自救出口。
pub fn switch_to_vault(
    data_dir: &Path,
    account_id: &str,
    exe: &str,
    ctx: &SwitchContext,
) -> Result<String, String> {
    let spec = detect_layout()?;
    let root = &spec.root;
    let vault = resolve_vault(data_dir, account_id, &spec)?;
    let image = client_image(exe);
    kill_client(&image)?;

    let archive_note = archive_current(data_dir, &spec, ctx, account_id);
    let rollback =
        std::env::temp_dir().join(format!("ai_manager_qoder_rollback_{}", Uuid::new_v4()));
    std::fs::create_dir_all(&rollback).map_err(|e| format!("创建回滚目录失败: {e}"))?;
    for &rel in spec.whitelist {
        snapshot_entry(root, &rollback, rel);
    }
    // 立即回滚只认本次切换前的临时快照；last 槽留给用户手动回退，避免把
    // 一份可能不完整的持久槽二次覆盖到刚恢复好的现场上。
    let rollback_to_live = |reason: &str| {
        let _ = restore_entry_set(&rollback, root, &spec);
        let _ = std::fs::remove_dir_all(&rollback);
        let _ = launch_client(exe);
        format!("{reason}；已回滚到原账号并尝试重启客户端")
    };
    if let Err(e) = restore_entry_set(&vault, root, &spec) {
        return Err(rollback_to_live(&format!("写入登录态失败：{e}")));
    }
    if let Err(e) = launch_client(exe) {
        return Err(rollback_to_live(&e));
    }

    // 进程起来了只是切换的前提；真实登录身份交给启动后的核验，不凭文件写回宣称成功。
    let started = {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if is_running(&image) {
                break true;
            }
            if Instant::now() >= deadline {
                break false;
            }
            std::thread::sleep(Duration::from_millis(400));
        }
    };
    if !started {
        kill_client(&image).ok();
        let _ = mark_vault_verify(data_dir, account_id, false);
        return Err(rollback_to_live("切换失败：客户端进程未能启动"));
    }
    let _ = std::fs::remove_dir_all(&rollback);

    let verdict = verify_login(&spec, ctx);
    let (message, confirmed) = switch_message(verdict, &archive_note);
    let _ = mark_vault_verify(data_dir, account_id, confirmed);
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!("tbk_qoder_test_{}", Uuid::new_v4()))
    }

    fn seed_login_db(root: &Path, key: &str, value: &str) {
        let path = root.join(IDE_LOGIN);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS ItemTable (key TEXT, value BLOB)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO ItemTable (key, value) VALUES (?1, ?2)",
            rusqlite::params![key, value.as_bytes()],
        )
        .unwrap();
    }

    fn ide(root: PathBuf) -> LayoutSpec {
        ide_spec(root)
    }

    fn app(root: PathBuf) -> LayoutSpec {
        app_spec(root)
    }

    fn encrypt_auth_for_test(key: &[u8], uid: &str) -> Vec<u8> {
        use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
        use aes_gcm::Aes256Gcm;
        let cipher = Aes256Gcm::new_from_slice(key).unwrap();
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let plain = json!({"schemaVersion": 1, "user": {"id": uid}});
        let ct = cipher
            .encrypt(&nonce, plain.to_string().as_bytes())
            .unwrap();
        let mut out = b"v10".to_vec();
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        out
    }

    #[test]
    fn keep_on_missing_protects_device_binding_in_both_layouts() {
        for rel in ["Local State", "Preferences"] {
            assert!(IDE_KEEP.contains(&rel));
            assert!(APP_KEEP.contains(&rel));
        }
        assert!(!IDE_KEEP.contains(&IDE_LOGIN));
        assert!(!APP_KEEP.contains(&APP_LOGIN));
    }

    #[test]
    fn app_layout_targets_real_credential_carriers() {
        assert!(APP_WHITELIST.contains(&APP_LOGIN));
        assert!(APP_WHITELIST.contains(&"Local State"));
        assert!(APP_WHITELIST.contains(&"Local Storage/leveldb"));
        assert!(!APP_WHITELIST.contains(&IDE_LOGIN));
        assert!(!APP_WHITELIST.contains(&"main.sqlite"));
        assert_eq!(app_spec(PathBuf::from("x")).layout, Layout::DesktopApp);
        assert_eq!(ide_spec(PathBuf::from("x")).layout, Layout::IdeFork);
    }

    #[test]
    fn parse_data_dir_name_accepts_product_json_and_rejects_paths() {
        assert_eq!(
            parse_data_dir_name(&json!({"dataDirectoryName": "com.qodercn.app.stable"})),
            Some("com.qodercn.app.stable".to_string())
        );
        assert_eq!(
            parse_data_dir_name(&json!({"dataFolderName": ".qoder"})),
            Some(".qoder".to_string())
        );
        assert_eq!(
            parse_data_dir_name(&json!({"dataDirectoryName": "..\\evil"})),
            None
        );
        assert_eq!(parse_data_dir_name(&json!({})), None);
    }

    #[test]
    fn symmetric_restore_copies_present_and_removes_missing() {
        let base = temp_dir();
        let src = base.join("vault");
        let dst = base.join("live");
        let _ = std::fs::create_dir_all(src.join("User/globalStorage"));
        let _ = std::fs::create_dir_all(dst.join("User/globalStorage"));
        std::fs::write(src.join(IDE_LOGIN), b"new").unwrap();
        std::fs::write(dst.join(IDE_LOGIN), b"old").unwrap();
        std::fs::write(dst.join("User/globalStorage/storage.json"), b"stale").unwrap();
        std::fs::write(dst.join("Local State"), b"keep").unwrap();
        std::fs::write(dst.join("code.lock"), b"lock").unwrap();
        let spec = ide(dst.clone());
        restore_entry_set(&src, &dst, &spec).unwrap();
        assert_eq!(std::fs::read(dst.join(IDE_LOGIN)).unwrap(), b"new");
        assert!(!dst.join("User/globalStorage/storage.json").exists());
        assert!(dst.join("Local State").exists(), "设备绑定文件必须保留");
        assert!(!dst.join("code.lock").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn app_restore_writes_auth_blob_and_keeps_local_state() {
        let base = temp_dir();
        let src = base.join("vault");
        let dst = base.join("live");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::write(src.join(APP_LOGIN), b"v10cipher").unwrap();
        std::fs::write(dst.join(APP_LOGIN), b"old").unwrap();
        std::fs::write(dst.join("Local State"), b"key").unwrap();
        let spec = app(dst.clone());
        restore_entry_set(&src, &dst, &spec).unwrap();
        assert_eq!(std::fs::read(dst.join(APP_LOGIN)).unwrap(), b"v10cipher");
        assert!(dst.join("Local State").exists());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn rotate_backup_preserves_previous_snapshot() {
        let base = temp_dir();
        let dir = base.join("acc");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("meta.json"), b"old").unwrap();
        rotate_backup(&dir).unwrap();
        assert!(!dir.exists());
        assert_eq!(
            std::fs::read(bak_dir(&dir).join("meta.json")).unwrap(),
            b"old"
        );
        // 第二次轮转不会抛错，旧的 .bak 被清掉
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("meta.json"), b"new").unwrap();
        rotate_backup(&dir).unwrap();
        assert_eq!(
            std::fs::read(bak_dir(&dir).join("meta.json")).unwrap(),
            b"new"
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn vault_valid_requires_the_auth_file_of_the_active_layout() {
        let base = temp_dir();
        let dir = base.join("acc");
        std::fs::create_dir_all(dir.join(IDE_LOGIN).parent().unwrap()).unwrap();
        std::fs::write(dir.join("meta.json"), b"{}").unwrap();
        std::fs::write(dir.join(IDE_LOGIN), b"db").unwrap();
        let ide_layout = ide(PathBuf::from("x"));
        let app_layout = app(PathBuf::from("x"));
        assert!(vault_valid(&dir, &ide_layout));
        assert!(
            !vault_valid(&dir, &app_layout),
            "旧版档案不能当成 App 档案使用"
        );
        std::fs::write(dir.join(APP_LOGIN), b"v10").unwrap();
        assert!(vault_valid(&dir, &app_layout));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn save_vault_writes_meta_and_falls_back_to_bak() {
        let base = temp_dir();
        let live = base.join("live");
        seed_login_db(&live, "secret://aicoding.auth.userInfo", "cipher");
        std::fs::write(live.join("machineid"), b"mid").unwrap();
        let data = base.join("data");
        let slot = vault_dir(&data, "acc-1");
        let spec = ide(live.clone());
        let count = save_vault(&live, &slot, "acc-1", &spec).unwrap();
        assert!(count >= 2);
        assert!(vault_valid(&slot, &spec));
        assert_eq!(vault_info(&data, "acc-1").unwrap().entries, count);
        // 主档失效时，.bak 仍可被解析为恢复源
        rotate_backup(&slot).unwrap();
        assert_eq!(
            resolve_vault(&data, "acc-1", &spec).unwrap(),
            bak_dir(&slot)
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn archive_current_always_keeps_last_slot() {
        let base = temp_dir();
        let live = base.join("live");
        seed_login_db(&live, "secret://aicoding.auth.userInfo", "cipher");
        let data = base.join("data");
        let ctx = SwitchContext::default();
        let spec = ide(live.clone());
        let note = archive_current(&data, &spec, &ctx, "target");
        assert!(note.contains("last 槽"));
        assert!(vault_valid(&last_dir(&data), &spec));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn archive_current_skips_writeback_when_live_has_no_login() {
        let base = temp_dir();
        let live = base.join("live");
        seed_login_db(&live, "chat.draft", "hello");
        let data = base.join("data");
        let slot = vault_dir(&data, "current");
        let spec = ide(live.clone());
        save_vault(&live, &slot, "current", &spec).unwrap();
        let before = std::fs::read(slot.join("meta.json")).unwrap();
        let ctx = SwitchContext {
            current_account_id: Some("current".to_string()),
            target_uid: String::new(),
            other_uids: Vec::new(),
        };
        let note = archive_current(&data, &spec, &ctx, "target");
        assert!(note.contains("跳过回写"));
        // last 槽轮转不影响 current 主档
        assert_eq!(std::fs::read(slot.join("meta.json")).unwrap(), before);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn login_signal_counts_auth_keys_and_exposes_uid() {
        let base = temp_dir();
        let live = base.join("live");
        seed_login_db(&live, "secret://aicoding.auth.userInfo", "uid=8801");
        let signal = read_login_signal(&live).unwrap();
        assert_eq!(signal.cred_entries, 1);
        assert!(signal.text.contains("8801"));
        let spec = ide(live.clone());
        assert_eq!(live_has_login(&live, &spec), Some(true));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn decrypt_auth_blob_reads_uid_and_rejects_bad_key() {
        let key = [7u8; 32];
        let blob = encrypt_auth_for_test(&key, "01a1e921-8c4c-774f-8f8c-31d7c5586baf");
        assert_eq!(
            decrypt_auth_blob(&blob, &key),
            Some("01a1e921-8c4c-774f-8f8c-31d7c5586baf".to_string())
        );
        assert_eq!(decrypt_auth_blob(&blob, &[8u8; 32]), None);
        assert_eq!(decrypt_auth_blob(b"v10", &key), None);
    }

    #[test]
    fn verify_login_confirms_target_uid_early() {
        let base = temp_dir();
        let live = base.join("live");
        seed_login_db(&live, "secret://aicoding.auth.userInfo", "userId=8801");
        let ctx = SwitchContext {
            current_account_id: None,
            target_uid: "8801".to_string(),
            other_uids: vec!["9902".to_string()],
        };
        let spec = ide(live.clone());
        assert_eq!(verify_login(&spec, &ctx), VerifyVerdict::Confirmed);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn verify_login_detects_other_uid() {
        let base = temp_dir();
        let live = base.join("live");
        seed_login_db(&live, "secret://aicoding.auth.userInfo", "userId=9902");
        let ctx = SwitchContext {
            current_account_id: None,
            target_uid: "8801".to_string(),
            other_uids: vec!["9902".to_string()],
        };
        let spec = ide(live.clone());
        assert!(matches!(
            verify_login(&spec, &ctx),
            VerifyVerdict::Mismatch(_)
        ));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn only_confirmed_counts_as_success() {
        for verdict in [
            VerifyVerdict::Alive,
            VerifyVerdict::Indeterminate,
            VerifyVerdict::Mismatch("身份不符".to_string()),
        ] {
            let (msg, confirmed) = switch_message(verdict, "");
            assert!(!confirmed);
            assert!(msg.starts_with("SWITCH_WARN:"), "未确认绝不能报成功: {msg}");
        }
        let (msg, confirmed) = switch_message(VerifyVerdict::Confirmed, "");
        assert!(confirmed);
        assert!(!msg.starts_with("SWITCH_WARN:"));
    }

    #[test]
    fn exe_candidates_only_include_cn_binary() {
        let candidates = exe_candidates();
        assert!(!candidates.is_empty());
        for p in candidates {
            let name = p
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            assert!(
                QODER_CN_EXE_NAMES.contains(&name.as_str()),
                "候选路径必须是 Qoder CN 本体: {name}"
            );
        }
    }

    #[test]
    fn qoder_cn_exe_guard_rejects_other_product_lines() {
        assert!(is_qoder_cn_exe(r"D:\Cstools\Qoder CN\Qoder CN.exe"));
        assert!(is_qoder_cn_exe("Qoder CN IDE.exe"));
        assert!(!is_qoder_cn_exe(r"D:\Cstools\Qoder IDE\Qoder IDE.exe"));
        assert!(!is_qoder_cn_exe("Qoder Work.exe"));
        assert!(!is_qoder_cn_exe(""));
    }
}
