//! Trae CN 客户端账号切换：整目录快照「冷切换」。
//!
//! 实现参考了开源项目 TraeTools。
//!
//! 与 WorkBuddy 的单认证文件不同，Trae 登录态散落在 vscdb/storage.json/Cookie 等多处，
//! 只能按固定白名单整体备份/还原。已吸收参考项目实测的三个坑：
//! 1) 恢复前必须删 `code.lock` 与 `state.vscdb-wal/-shm`——否则客户端启动回放旧账号 WAL；
//! 2) 守护判定只看 storage.json 的 `iCubeAuthInfo://usertag` + `iCubeAuthInfo://icube.cloudide`
//!    凭据键：被客户端清除 = 明确失败（回滚），全程齐全 = 进入；
//! 3) 切换成功后必须把 live 的 storage.json 回写 vault——否则 refresh token 被服务端轮换，
//!    同一账号第二次切换 401。

use serde_json::Value;
use std::path::{Path, PathBuf};

/// 客户端数据根（%APPDATA%\Trae CN）
fn data_root() -> Result<PathBuf, String> {
    let roaming = dirs::data_dir().ok_or("无法定位 %APPDATA%")?;
    Ok(roaming.join("Trae CN"))
}

/// 固定 15 项载体白名单（相对数据根；目录条目整树处理，缓存类子目录跳过）
const WHITELIST: [&str; 15] = [
    "User/globalStorage/storage.json", // 登录态键 + 设备遥测
    "User/globalStorage/state.vscdb",  // SQLite：登录令牌真源
    "User/globalStorage/state.vscdb-wal",
    "User/globalStorage/state.vscdb-shm",
    "User/globalStorage/state.vscdb.backup",
    "machineid",
    "aha",
    "Preferences",
    "Local State",
    "Local Storage/leveldb",
    "Local Storage/config.db",
    "Network",
    "Partitions/trae-webview",
    "Partitions/icube-web-crawler-shared-session-v1.0",
    "Session Storage",
];

/// 与登录态无关的缓存/日志目录（对齐参考实现的默认排除清单）
const EXCLUDE_DIRS: [&str; 16] = [
    "Cache",
    "CachedData",
    "CachedConfigurations",
    "CachedProfilesData",
    "Code Cache",
    "GPUCache",
    "Crashpad",
    "DawnGraphiteCache",
    "DawnWebGPUCache",
    "ShaderCache",
    "Dictionaries",
    "blob_storage",
    "Service Worker",
    "Shared Dictionary",
    "logs",
    "Backups",
];

/// 登录凭据键：两把都存在且非空才视为「已登录」
const AUTH_KEYS: [&str; 2] = ["iCubeAuthInfo://usertag", "iCubeAuthInfo://icube.cloudide"];

const GRACEFUL_WAIT_SECS: u64 = 12;
const FORCE_WAIT_SECS: u64 = 8;
const GUARD_SECS: u64 = 20;

fn vault_dir(data_dir: &Path, account_id: &str) -> PathBuf {
    data_dir.join("vaults").join(account_id)
}

fn storage_path(root: &Path) -> PathBuf {
    root.join("User/globalStorage/storage.json")
}

// ===== 设备号：客户端真实注册过的 Aha 设备号 =====

fn is_device_no(s: &str) -> bool {
    matches!(s.len(), 12..=20) && s.bytes().all(|b| b.is_ascii_digit())
}

/// 从 storage.json 取设备号：键名 `iCubeAuthInfo://icube-dc:<数字>` 即设备号，
/// 无需解密；也兼容「键名固定、号写在值里」的写法。
fn device_id_from_storage(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let obj = v.as_object()?;
    for (k, val) in obj {
        if let Some(rest) = k.strip_prefix("iCubeAuthInfo://icube-dc:") {
            let d = rest.trim();
            if is_device_no(d) {
                return Some(d.to_string());
            }
        } else if k.trim_end_matches(':') == "iCubeAuthInfo://icube-dc" {
            if let Some(d) = val.as_str().map(str::trim) {
                if is_device_no(d) {
                    return Some(d.to_string());
                }
            }
        }
    }
    None
}

/// 该账号 vault 里客户端真实注册过的设备号。
/// 签到只认客户端注册过的号，脚本生成的号会被上游判 9074；
/// vault 里的 storage.json 正是该账号在客户端登录时的那份设备身份。
pub fn vault_real_device_id(data_dir: &Path, account_id: &str) -> Option<String> {
    device_id_from_storage(&storage_path(&vault_dir(data_dir, account_id)))
}

/// 当前 live 客户端注册过的设备号（本机 Trae CN 最近一次登录用的那个）
pub fn live_real_device_id() -> Option<String> {
    let root = data_root().ok()?;
    device_id_from_storage(&storage_path(&root))
}

// ===== 文件树操作 =====

fn is_excluded_dir(name: &str) -> bool {
    EXCLUDE_DIRS.iter().any(|d| d.eq_ignore_ascii_case(name))
}

/// 递归枚举 root 下全部文件（跳过缓存/日志类目录）
fn enumerate_tree(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(root) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                if is_excluded_dir(name) {
                    continue;
                }
            }
            enumerate_tree(&p, out);
        } else if p.is_file() {
            out.push(p);
        }
    }
}

/// 把 src_root/rel 覆盖写到 dst_root/rel：目录先整删目标再整树复制；文件先删再拷。
/// 任一侧源不存在则跳过。
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
        // 对称删除：SQLite 主库不能留旧文件被追加
        if dst.exists() {
            let _ = std::fs::remove_file(&dst);
        }
        let _ = std::fs::copy(&src, &dst);
    }
}

/// 恢复一组载体到数据根（切换与回滚共用）：先清 code.lock 与 vscdb 边车再写回
fn restore_entry_set(src_root: &Path, dst_root: &Path) -> Result<(), String> {
    // 单实例锁残留会让新实例误判「已有实例在运行」
    let lock = dst_root.join("code.lock");
    if lock.exists() {
        std::fs::remove_file(&lock).map_err(|e| format!("删除 code.lock 失败: {e}"))?;
    }
    // 边车不删，客户端启动回放旧账号 WAL——「切了等于没切」的直接来源
    for side in [
        "User/globalStorage/state.vscdb-wal",
        "User/globalStorage/state.vscdb-shm",
    ] {
        let p = dst_root.join(side);
        if p.exists() {
            std::fs::remove_file(&p).map_err(|e| format!("删除 {} 失败: {e}", p.display()))?;
        }
    }
    for rel in WHITELIST {
        overwrite_entry(src_root, dst_root, rel);
    }
    Ok(())
}

// ===== 进程控制 =====

/// 客户端是否在运行（按 exe 文件名判定）
pub fn client_running(image: &str) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    std::process::Command::new("tasklist")
        .args(["/FI", &format!("IMAGENAME eq {image}"), "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains(image.trim_end_matches(".exe")))
        .unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
pub fn client_running(image: &str) -> bool {
    std::process::Command::new("pgrep")
        .args(["-f", image.trim_end_matches(".exe")])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn client_image(exe: &str) -> String {
    Path::new(exe)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Trae CN.exe")
        .to_string()
}

/// 按安装路径判断客户端是否在运行（切换前先检测用）
pub fn is_running_for_exe(exe: &str) -> bool {
    if exe.is_empty() {
        return false;
    }
    client_running(&client_image(exe))
}

fn kill_client(image: &str) -> Result<(), String> {
    if !client_running(image) {
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let _ = std::process::Command::new("taskkill")
            .args(["/IM", image, "/T"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        if wait_exit(image, GRACEFUL_WAIT_SECS) {
            return Ok(());
        }
        let _ = std::process::Command::new("taskkill")
            .args(["/IM", image, "/T", "/F"])
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
    Err("无法关闭 Trae CN 进程，请手动结束后重试".to_string())
}

fn wait_exit(image: &str, timeout_secs: u64) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    loop {
        if !client_running(image) {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
}

fn launch_client(exe: &str) -> Result<(), String> {
    if exe.is_empty() || !Path::new(exe).exists() {
        return Err("未找到 Trae CN 安装路径，请到设置页配置客户端路径".to_string());
    }
    std::process::Command::new(exe)
        .spawn()
        .map_err(|e| format!("启动 Trae CN 失败: {e}"))?;
    Ok(())
}

// ===== 守护：读 storage.json 凭据键 =====

/// Some(true)=凭据齐全；Some(false)=凭据缺失（客户端已主动登出）；None=文件被占用/解析中（瞬态）
fn has_auth_creds(storage: &Path) -> Option<bool> {
    let text = std::fs::read_to_string(storage).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?; // 解析失败=正在改写，瞬态
    let obj = v.as_object()?;
    Some(AUTH_KEYS.iter().all(|k| {
        obj.get(*k)
            .map(|val| match val {
                Value::String(s) => !s.is_empty(),
                Value::Null => false,
                _ => true,
            })
            .unwrap_or(false)
    }))
}

// ===== 对外：建档 / 查询 / 切换 =====

/// 查询账号建档信息：(建档时间 epoch 秒, 载体文件数)；未建档返回 None
pub fn vault_info(data_dir: &Path, account_id: &str) -> Option<(i64, usize)> {
    let meta_path = vault_dir(data_dir, account_id).join("meta.json");
    let text = std::fs::read_to_string(meta_path).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let created = v.get("created_at")?.as_i64()?;
    let entries = v.get("entries")?.as_u64()? as usize;
    Some((created, entries))
}

/// 建档：把当前 live 登录态快照到账号 vault。要求客户端已关闭（运行中拷 SQLite 必不自洽）
pub fn backup_vault(data_dir: &Path, account_id: &str, image: &str) -> Result<String, String> {
    if client_running(image) {
        return Err("建档前请先关闭 Trae CN 客户端（运行中快照会不一致）".to_string());
    }
    let root = data_root()?;
    if !root.exists() {
        return Err(format!("未找到 Trae CN 数据目录：{}", root.display()));
    }
    let vault = vault_dir(data_dir, account_id);
    std::fs::create_dir_all(&vault).map_err(|e| e.to_string())?;

    let mut count = 0usize;
    for rel in WHITELIST {
        let src = root.join(rel);
        if !src.exists() {
            continue; // 白名单条目必须保留但允许缺失
        }
        overwrite_entry(&root, &vault, rel);
        if src.is_dir() {
            let mut files = Vec::new();
            enumerate_tree(&src, &mut files);
            count += files.len();
        } else {
            count += 1;
        }
    }
    if count == 0 {
        return Err("数据目录下没有任何载体文件，Trae CN 可能从未登录过".to_string());
    }
    let meta = serde_json::json!({
        "account_id": account_id,
        "created_at": chrono::Utc::now().timestamp(),
        "entries": count,
    });
    std::fs::write(
        vault.join("meta.json"),
        serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(format!("建档完成，共 {count} 个载体文件"))
}

/// 切换编排：关进程 → 当前 live 快照到临时回滚目录 → 写回目标 vault → 拉起 → 守护判定。
/// 守护失败自动回滚并重启客户端；成功后回写 storage.json 保鲜 refresh token。
pub fn switch_to_vault(data_dir: &Path, account_id: &str, exe: &str) -> Result<String, String> {
    let root = data_root()?;
    let vault = vault_dir(data_dir, account_id);
    if !vault.join("meta.json").exists() {
        return Err("该账号尚未建档：先让客户端登录该账号，关闭客户端后点「存档」".to_string());
    }
    let image = Path::new(exe)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Trae CN.exe")
        .to_string();

    kill_client(&image)?;

    // 1) 当前 live → 临时回滚目录
    let rollback = std::env::temp_dir().join(format!(
        "ai_manager_rollback_{}",
        uuid::Uuid::new_v4().simple()
    ));
    for rel in WHITELIST {
        overwrite_entry(&root, &rollback, rel);
    }

    // 2) 写回目标账号；失败即回滚
    if let Err(e) = restore_entry_set(&vault, &root) {
        let _ = restore_entry_set(&rollback, &root);
        let _ = std::fs::remove_dir_all(&rollback);
        let _ = launch_client(exe);
        return Err(format!("写回目标账号失败: {e}（已回滚并尝试重启客户端）"));
    }

    // 3) 拉起；失败即回滚
    if let Err(e) = launch_client(exe) {
        let _ = restore_entry_set(&rollback, &root);
        let _ = std::fs::remove_dir_all(&rollback);
        return Err(format!("{e}（已回滚）"));
    }

    // 4) 守护：只看凭据键——被清除=明确失败回滚；全程齐全=真进入
    let storage = storage_path(&root);
    let mut running_seen = false;
    let mut creds_seen = false;
    let mut auth_cleared = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(GUARD_SECS);
    while std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(400));
        if !client_running(&image) {
            continue;
        }
        running_seen = true;
        match has_auth_creds(&storage) {
            Some(false) => {
                auth_cleared = true;
                break;
            }
            Some(true) => creds_seen = true,
            None => {} // 瞬态，下轮再看
        }
    }

    if !running_seen || auth_cleared {
        // 明确失败：回滚到切换前的 live 并重启
        kill_client(&image).ok();
        let _ = restore_entry_set(&rollback, &root);
        let _ = launch_client(exe);
        let _ = std::fs::remove_dir_all(&rollback);
        let why = if auth_cleared {
            "客户端主动清除了登录凭据（该账号 refresh token 可能已失效，请重新建档）"
        } else {
            "客户端进程在守护期内未起来"
        };
        return Err(format!("切换失败：{why}。已回滚到原账号并重启客户端"));
    }

    let _ = std::fs::remove_dir_all(&rollback);

    if creds_seen {
        // 5) 成功后回写 storage.json 到 vault（保鲜轮换后的 refresh token）
        let dst = vault.join("User/globalStorage/storage.json");
        if let Some(parent) = dst.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let synced = std::fs::copy(&storage, &dst).is_ok();
        Ok(if synced {
            "已切换并重启 Trae CN（守护确认登录凭据齐全）".to_string()
        } else {
            "已切换并重启 Trae CN；但 storage.json 回写建档失败，建议重新建档一次".to_string()
        })
    } else {
        // 凭据始终读不到（可能被占用）：不回滚、不回写，交给用户确认
        Ok("已重启 Trae CN，但守护期内未能确认登录态，请检查客户端是否已进入目标账号".to_string())
    }
}
