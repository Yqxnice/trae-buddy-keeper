//! Trae CN 客户端账号切换：整目录快照「冷切换」。
//!
//! Trae CN 登录态快照冷切换：按白名单整份搬运登录态载体并守护启动结果。
//!
//! 与 WorkBuddy 的单认证文件不同，Trae 登录态散落在 vscdb/storage.json/Cookie 等多处，
//! 只能按固定白名单整体备份/还原。以下几处是实测踩过的坑：
//! 1) 恢复前必须删 `code.lock` 与 `state.vscdb-wal/-shm`——否则客户端启动回放旧账号 WAL；
//! 2) 守护判定只看 storage.json 的 `iCubeAuthInfo://usertag` + `iCubeAuthInfo://icube.cloudide`
//!    凭据键：客户端冷启动会重写该文件，单帧缺失只是瞬态，**持续缺失**才判定为主动登出；
//! 3) 切换成功后必须把 live 的 storage.json 回写 vault；**切换前**也要把出站账号的 live 现场
//!    回写其档案（用客户端日志里的登录用户名确认是同一账号）——客户端运行期间 refresh token
//!    可能已被服务端轮换，不刷新档案的话下次切回来就是过期令牌；
//! 4) 「未能确认登录」不等于「切换失败」：不硬回滚，保留客户端现状，给档案打
//!    `verify_failed_at` 标记并引导重新登录 / 重建档——切进去重新登录是会话失效时唯一的
//!    自救路径，硬回滚会把这条路堵死。
//!
//! 实测补充（2026-11-17）：真正的凭据只存在于 `storage.json` 的 `iCubeAuthInfo://*`（safeStorage
//! 密文）；`state.vscdb` 的 ItemTable 里没有任何令牌，只有模型列表/聊天草稿等用户态数据。
//! 早先「SQLite 是登录令牌真源」的注释不准确，已更正。

use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// 客户端数据根（%APPDATA%\Trae CN）
fn data_root() -> Result<PathBuf, String> {
    let roaming = dirs::data_dir().ok_or("无法定位 %APPDATA%")?;
    Ok(roaming.join("Trae CN"))
}

/// 固定 15 项载体白名单（相对数据根；目录条目整树处理，缓存类子目录跳过）
const WHITELIST: [&str; 15] = [
    "User/globalStorage/storage.json", // 登录态键（safeStorage 密文，令牌真源）+ 设备遥测
    "User/globalStorage/state.vscdb",  // SQLite：用户态配置/聊天数据，不含令牌
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

/// SQLite 运行时边车：建档时一并快照（强杀后可能含未 checkpoint 的数据），
/// 但恢复时**只删不写回**——把旧 WAL 放回 live 会让客户端回放上一账号的事务。
const SQLITE_SIDECARS: [&str; 2] = [
    "User/globalStorage/state.vscdb-wal",
    "User/globalStorage/state.vscdb-shm",
];

/// 设备/密钥绑定类载体：vault 里缺这几项时保留 live 版本，绝不删。
/// `Local State` 存 safeStorage 主密钥，删掉会导致 `iCubeAuthInfo://*` 密文解不开；
/// `machineid` / `aha` 是设备身份，删掉等于换了一台机器。
const KEEP_ON_MISSING: [&str; 4] = ["Local State", "machineid", "aha", "Preferences"];

/// 对称恢复时是否应删除 live 上 vault 没有的项（抽成纯函数便于单测）
fn should_remove_missing(rel: &str) -> bool {
    !KEEP_ON_MISSING.contains(&rel) && !SQLITE_SIDECARS.contains(&rel)
}

/// 与登录态无关的缓存/日志目录
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
/// 守护窗口：留足 Electron 冷启动 + 一次令牌刷新的时间
const GUARD_SECS: u64 = 30;
const GUARD_POLL_MS: u64 = 400;
/// 凭据缺失必须**持续**这么久才认定为客户端主动登出。
/// 客户端启动时会先把 storage.json 重写成不含凭据的中间态，随后再写回；
/// 旧实现采到一帧缺失就回滚，把正常切换误判成「refresh token 失效」。
const CLEAR_CONFIRM: Duration = Duration::from_secs(4);
/// 连续缺失这么久仍等不到凭据写回，才提前收工判失败；否则继续等到守护窗口结束，
/// 给慢机器上的冷启动 + 令牌刷新留足时间（实测出现过 4 秒就放弃、随后客户端才写回凭据）。
const GIVE_UP_MISSING: Duration = Duration::from_secs(12);

/// 切换/建档都要 kill 客户端再整目录覆盖：两者并发会把两个账号的现场混在一起，
/// 而且「后完成的那个」才写「当前已注入」标记，可能让标记和客户端实际登录的账号相反。
/// 进程内互斥，抢不到直接报错让用户稍后重试。
static CLIENT_SWAP_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn acquire_swap_lock() -> Result<std::sync::MutexGuard<'static, ()>, String> {
    CLIENT_SWAP_LOCK
        .try_lock()
        .map_err(|_| "另一个 Trae 切换/建档任务正在进行中，请等它完成后再操作".to_string())
}

/// Trae 档案目录：`vaults/trae/<account_id>`。
///
/// 旧版把 UUID 目录直接散在 `vaults/` 根下，用户在资源管理器里分不清哪些是
/// Trae 档案（只有 Qoder 有独立的 `qoder_cn/` 子目录）。这里统一收进 `trae/`。
/// 首次遇到旧布局时自动搬移，保证从旧备份恢复的数据仍可用。
fn vault_dir(data_dir: &Path, account_id: &str) -> PathBuf {
    let new_dir = data_dir.join("vaults/trae").join(account_id);
    if new_dir.exists() {
        return new_dir;
    }
    if !crate::data::vault_cleanup::valid_account_id(account_id) {
        return new_dir;
    }
    let legacy = data_dir.join("vaults").join(account_id);
    if !legacy.exists() {
        return new_dir;
    }
    if let Some(parent) = new_dir.parent() {
        if std::fs::create_dir_all(parent).is_ok() && std::fs::rename(&legacy, &new_dir).is_ok() {
            // 旧备份一并搬走，保持 `<dir>.bak` 的轮转语义
            let legacy_bak = PathBuf::from(format!("{}.bak", legacy.display()));
            if legacy_bak.exists() {
                let new_bak = PathBuf::from(format!("{}.bak", new_dir.display()));
                let _ = std::fs::rename(&legacy_bak, &new_bak);
            }
            return new_dir;
        }
    }
    // 搬移失败（如目录被占用）时退回旧路径，至少不丢档案
    legacy
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

/// 恢复一组载体到数据根（切换与回滚共用）：先清 code.lock 与 vscdb 边车再写回。
/// 槽位没有的登录态载体会把 live 对应项删掉（对称恢复），避免上一账号的文件残留串号；
/// 设备绑定类文件与边车例外，理由见 KEEP_ON_MISSING / SQLITE_SIDECARS。
fn restore_entry_set(src_root: &Path, dst_root: &Path) -> Result<(), String> {
    // 单实例锁残留会让新实例误判「已有实例在运行」
    let lock = dst_root.join("code.lock");
    if lock.exists() {
        std::fs::remove_file(&lock).map_err(|e| format!("删除 code.lock 失败: {e}"))?;
    }
    // 边车只删不写回：客户端启动回放旧 WAL 是「切了等于没切」的直接来源
    for side in SQLITE_SIDECARS {
        let p = dst_root.join(side);
        if p.exists() {
            std::fs::remove_file(&p).map_err(|e| format!("删除 {} 失败: {e}", p.display()))?;
        }
    }
    for rel in WHITELIST {
        if SQLITE_SIDECARS.contains(&rel) {
            continue;
        }
        if !src_root.join(rel).exists() {
            if !should_remove_missing(rel) {
                continue;
            }
            let dst = dst_root.join(rel);
            if dst.is_dir() {
                let _ = std::fs::remove_dir_all(&dst);
            } else if dst.is_file() {
                let _ = std::fs::remove_file(&dst);
            }
            continue;
        }
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
        // 注意：不能带 /T（杀进程树）——从 Trae 终端启动的无关子进程（如开发调试中的
        // 本应用 dev 实例）也在树内，会被连坐。Electron 全部进程映像名相同，/IM 已覆盖。
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

// ===== 账号归属核验：读客户端日志里的登录用户名 =====
// `iCubeAuthInfo://usertag` 是客户端自定义的 safeStorage 密文，无法离线解密；而且客户端
// 每次启动/写入都会**重新加密**（同一账号两次快照的密文并不相同，已实测），所以密文比对
// 不能用来判定账号归属。唯一可靠的免费信号是 exthost 日志里周期性打印的
// `refreshUserInfo username:<name>`（`completion.log` 里还有 `userName: <name>`）。
// 读不到日志就跳过回写——宁可档案旧一点，也不能把 A 的登录态写进 B 的档案。

/// 日志里确认登录用户名的标记（取最后一次出现）
const LOGIN_NAME_MARKERS: [&str; 2] = ["refreshUserInfo username:", "userName:"];

/// 单个日志文件里某个标记最后一次出现后的用户名（只取同一行，到空白为止）
fn login_name_in_file(path: &Path, marker: &str) -> Option<String> {
    // 单个日志超过 16MB 视为异常，跳过，避免一次性读入大文件
    if std::fs::metadata(path).ok()?.len() > 16 * 1024 * 1024 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let idx = text.rfind(marker)?;
    // 只在同一行内取值：标记后面为空（登出态）时不能把下一行的词当用户名
    let name = text[idx + marker.len()..]
        .lines()
        .next()?
        .split_whitespace()
        .next()?
        .trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn collect_log_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            collect_log_files(&p, out);
        } else if p
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("log"))
            .unwrap_or(false)
        {
            out.push(p);
        }
    }
}

/// logs 下除会话目录外还有 `aha_log` 之类的杂项目录；会话目录名是 15 位时间戳
/// （`YYYYMMDDTHHMMSS`），按名倒序即时间序。
fn is_session_dir_name(name: &str) -> bool {
    name.len() == 15
        && name.as_bytes()[8] == b'T'
        && name
            .bytes()
            .enumerate()
            .all(|(i, b)| i == 8 || b.is_ascii_digit())
}

/// 从客户端数据根的 `logs/` 推断最近一次登录的用户名。
///
/// 只看最新的会话目录：更早目录里的用户名代表历史账号，拿来比对会把「上一次登录的账号」
/// 当成现场。某台机器上最新的会话目录通常正是刚被杀掉的那一次，因此切换前调用时
/// 读到的就是出站账号。
pub fn login_name_from_logs(logs_root: &Path) -> Option<String> {
    let newest = newest_session_dir(logs_root)?;
    let mut files = Vec::new();
    collect_log_files(&newest, &mut files);
    if files.is_empty() {
        return None;
    }
    // 新的日志文件优先：同一次会话里最后一次写入才代表最终登录态
    files.sort_by_cached_key(|p| {
        std::cmp::Reverse(std::fs::metadata(p).and_then(|m| m.modified()).ok())
    });
    // 按标记权威性分轮：先认 `refreshUserInfo username:`，整轮都没有才退化到 `userName:`
    LOGIN_NAME_MARKERS
        .iter()
        .find_map(|marker| files.iter().find_map(|p| login_name_in_file(p, marker)))
}

/// 最新会话目录里最新的日志写入时间（用于判断日志是否「本会话产生」）
fn newest_session_log_mtime(logs_root: &Path) -> Option<std::time::SystemTime> {
    let newest = newest_session_dir(logs_root)?;
    let mut files = Vec::new();
    collect_log_files(&newest, &mut files);
    files
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok()?.modified().ok())
        .max()
}

fn newest_session_dir(logs_root: &Path) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(logs_root)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    let mut sessions: Vec<PathBuf> = dirs
        .iter()
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .map(is_session_dir_name)
                .unwrap_or(false)
        })
        .cloned()
        .collect();
    if sessions.is_empty() {
        // 命名规则变化时退回按修改时间排序，至少还能工作
        dirs.sort_by_cached_key(|p| {
            std::cmp::Reverse(std::fs::metadata(p).and_then(|m| m.modified()).ok())
        });
        dirs.into_iter().next()
    } else {
        sessions.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
        sessions.into_iter().next()
    }
}

/// 当前客户端登录的用户名（读 `%APPDATA%\Trae CN\logs`）
pub fn live_login_name() -> Option<String> {
    login_name_from_logs(&data_root().ok()?.join("logs"))
}

/// 写入「当前已注入 Trae」标记，同时记下写入时刻。
/// 日志里读到的用户名只有在标记写入之后还有新日志时才可信，否则可能是上一个会话的残留。
pub const ACTIVE_TRAE_AT_KEY: &str = "active_client_trae_at";

pub fn mark_active_account(db: &crate::data::db::Database, account_id: &str) {
    let _ = db.set_setting("active_client_trae", account_id);
    let _ = db.set_setting(
        ACTIVE_TRAE_AT_KEY,
        &chrono::Utc::now().timestamp().to_string(),
    );
}

/// 用客户端日志里的登录用户名校正「当前已注入 Trae」标记。
///
/// 用户在 Trae 里手动退出再登录别的账号时，这个标记会漂移：界面显示 A、客户端实际登录 B，
/// 切换前的出站回写也会因此认错账号。名字能唯一匹配某个 Trae 账户时以日志为准。
///
/// 防误判：刚切换完时新会话的日志目录可能还没建出来，此时最新会话仍是出站账号那次；
/// 因此要求最新会话在「标记写入时刻」之后仍有日志写入，否则保持原标记不动。
/// 读不到用户名时同样不动。
pub fn correct_active_marker(
    db: &crate::data::db::Database,
    accounts: &[crate::data::models::Account],
) -> Option<String> {
    let logs_root = data_root().ok()?.join("logs");
    let live_name = login_name_from_logs(&logs_root)?;
    if let Some(at) = db
        .get_setting(ACTIVE_TRAE_AT_KEY)
        .ok()
        .flatten()
        .and_then(|v| v.parse::<u64>().ok())
    {
        let since = std::time::UNIX_EPOCH + Duration::from_secs(at);
        if newest_session_log_mtime(&logs_root).is_none_or(|m| m <= since) {
            return None;
        }
    }
    let mut hits = accounts
        .iter()
        .filter(|a| a.platform == "trae" && a.name.trim() == live_name.trim());
    let hit = hits.next()?;
    if hits.next().is_some() {
        return None; // 同名账户不唯一，无法确定是哪一个
    }
    let marked = db.get_setting("active_client_trae").ok().flatten();
    if marked.as_deref() != Some(hit.id.as_str()) {
        mark_active_account(db, &hit.id);
        db.log(
            "info",
            "切换",
            &format!("校正「Trae 当前已注入」标记：客户端日志显示当前登录「{live_name}」"),
        );
    }
    Some(hit.id.clone())
}

// ===== 守护状态机（纯逻辑，便于单测） =====

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CredSample {
    /// 两把凭据键齐全且非空
    Present,
    /// storage.json 是合法 JSON，但凭据键缺失/被清空——客户端写出了登出态
    Missing,
    /// 文件被占用、JSON 写到一半、还不是对象——启动期瞬态
    Unreadable,
}

#[derive(Debug, Default)]
struct GuardState {
    running_seen: bool,
    present: usize,
    missing: usize,
    unreadable: usize,
    ever_present: bool,
    last_present: bool,
    /// 连续 Present 采样数：恢复时凭据本来就齐全，必须等客户端自己改写后才算数
    present_streak: usize,
    /// 出现过 Missing/Unreadable：说明文件被客户端改写过（不是我们恢复进去的那份）
    saw_turbulence: bool,
    missing_since: Option<Instant>,
    /// 凭据缺失已持续 CLEAR_CONFIRM，判定为客户端主动登出
    cleared: bool,
}

impl GuardState {
    fn observe(&mut self, s: CredSample, now: Instant) {
        match s {
            CredSample::Present => {
                self.present += 1;
                self.present_streak += 1;
                self.ever_present = true;
                self.last_present = true;
                // 中间态自行恢复 → 撤销「持续缺失」计数，不误杀正常切换
                self.missing_since = None;
                self.cleared = false;
            }
            CredSample::Missing => {
                self.missing += 1;
                self.present_streak = 0;
                self.last_present = false;
                self.saw_turbulence = true;
                let since = *self.missing_since.get_or_insert(now);
                if now.duration_since(since) >= CLEAR_CONFIRM {
                    self.cleared = true;
                }
            }
            CredSample::Unreadable => {
                self.unreadable += 1;
                self.present_streak = 0;
                // 文件正在改写不算「客户端清空了凭据」，不累计连续缺失时长；
                // 但也不能据此宣称登录态正常——否则会把写了一半的 storage.json 回写进档案。
                self.saw_turbulence = true;
                self.missing_since = None;
                self.last_present = false;
            }
        }
    }

    /// 客户端自己写出过稳定凭据：恢复进去的凭据先齐全 → 启动中被清空/改写 → 随后又写回。
    /// 恢复写入的文件不会凭空变化，出现这个序列只可能是客户端完成了登录/刷新，是成功强信号。
    fn client_wrote_credentials(&self) -> bool {
        self.saw_turbulence && self.present_streak >= 2 && self.last_present
    }

    /// 客户端进程短暂消失的一帧：本轮没有采样，连续缺失计时必须重新起算。
    ///
    /// 守护轮询 400ms，但 Electron 冷启动期间 `client_running` 可能连续多帧为假，
    /// 循环直接 `continue`。若不重置 `missing_since`，两帧相隔数秒的缺失会被
    /// `now - since >= CLEAR_CONFIRM` 误判成「持续清除 4 秒」——表现为采样只有
    /// 「缺失 2」却报客户端登出（与真实刷新令牌失效混淆）。
    fn note_client_absent(&mut self) {
        self.missing_since = None;
        self.cleared = false;
    }

    fn samples(&self) -> String {
        format!(
            "齐全 {} / 缺失 {} / 读取中 {}",
            self.present, self.missing, self.unreadable
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GuardVerdict {
    /// 凭据曾齐全后被持续清除，或启动后始终没写入：无法确认，打标并引导重建档
    ConfirmedLogout,
    /// 守护期内进程没起来：唯一仍硬回滚的情形
    ClientNeverStarted,
    /// 凭据出现过但结束时状态不稳定：不回滚、不回写，交给用户确认
    CredentialsUnstable,
    /// 结束时凭据齐全，可回写 vault
    LoggedIn,
}

fn guard_verdict(s: &GuardState) -> GuardVerdict {
    if !s.running_seen {
        return GuardVerdict::ClientNeverStarted;
    }
    if s.cleared {
        return GuardVerdict::ConfirmedLogout;
    }
    if s.ever_present && s.last_present {
        return GuardVerdict::LoggedIn;
    }
    GuardVerdict::CredentialsUnstable
}

// ===== 对外：建档 / 查询 / 切换 =====

/// 历史坏配置兜底：客户端路径可能被误配成本应用自身（自动获取曾把
/// 「TraeBuddy Keeper」安装项当成 Trae 命中），此时绝不能对自身检测/关闭
fn ensure_not_self(image: &str) -> Result<(), String> {
    let self_name = std::env::current_exe().ok().and_then(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_lowercase())
    });
    if let Some(self_name) = self_name {
        if image.to_lowercase() == self_name {
            return Err(
                "客户端路径配置错误：指向了本应用自身，请到设置页重新自动获取或手动修正 Trae 客户端路径"
                    .to_string(),
            );
        }
    }
    Ok(())
}

/// 建档元数据（前端据此显示「档案待验证」徽标并提供重建档引导）
#[derive(Debug, Clone, Serialize)]
pub struct VaultInfo {
    pub created_at: i64,
    pub entries: usize,
    /// 最近一次切换未能确认登录身份的 epoch 秒；None = 档案已验证
    pub verify_failed_at: Option<i64>,
}

/// 查询账号建档信息；未建档或 meta 损坏返回 None
pub fn vault_info(data_dir: &Path, account_id: &str) -> Option<VaultInfo> {
    let text = std::fs::read_to_string(vault_dir(data_dir, account_id).join("meta.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    Some(VaultInfo {
        created_at: v.get("created_at")?.as_i64()?,
        entries: v.get("entries")?.as_u64()? as usize,
        // 旧档案没有该字段 = 未标记，保持兼容
        verify_failed_at: v.get("verify_failed_at").and_then(Value::as_i64),
    })
}

/// 切换验证结果写回档案：verified=true 清标，false 打标。
/// 只改这一个字段，保留 account_id / created_at / entries 等原有内容。
pub fn mark_vault_verify(data_dir: &Path, account_id: &str, verified: bool) -> Result<(), String> {
    let path = vault_dir(data_dir, account_id).join("meta.json");
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let obj = v.as_object_mut().ok_or("meta.json 不是 JSON 对象")?;
    if verified {
        obj.remove("verify_failed_at");
    } else {
        obj.insert(
            "verify_failed_at".to_string(),
            Value::from(chrono::Utc::now().timestamp()),
        );
    }
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())
}

/// 建档：把当前 live 登录态快照到账号 vault。要求客户端已关闭（运行中拷 SQLite 必不自洽）。
/// 客户端运行中且 close_running=false 时返回 CLIENT_RUNNING: 前缀错误（前端据此弹确认）；
/// close_running=true 则先温和关闭（WM_CLOSE），超时再强杀，与切换流程同一套 kill_client。
pub fn backup_vault(
    data_dir: &Path,
    account_id: &str,
    image: &str,
    close_running: bool,
) -> Result<String, String> {
    let _swap = acquire_swap_lock()?;
    ensure_not_self(image)?;
    if client_running(image) {
        if !close_running {
            return Err(
                "CLIENT_RUNNING:Trae 客户端仍在后台运行（点窗口 × 可能只是驻留后台，并未退出进程）"
                    .to_string(),
            );
        }
        kill_client(image)?;
        // 双重确认：kill_client 成功后进程必然已退出
        if client_running(image) {
            return Err("客户端进程未能完全退出，请手动结束后重试".to_string());
        }
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
    // 重新建档即视为新档案：不带「待验证」标记（旧档若有也会被覆盖清掉）
    let meta = serde_json::json!({
        "account_id": account_id,
        "created_at": chrono::Utc::now().timestamp(),
        "entries": count,
        "verify_failed_at": null,
    });
    std::fs::write(
        vault.join("meta.json"),
        serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(format!("建档完成，共 {count} 个载体文件"))
}

/// 用 live 现场重建档案：先写临时目录，成功后再把旧档轮转为 `.bak` 并换入，
/// 任一环节失败都不会破坏原有档案。
fn save_vault_from_live(root: &Path, vault: &Path, account_id: &str) -> Result<usize, String> {
    let staging = PathBuf::from(format!("{}.staging", vault.display()));
    if staging.exists() {
        std::fs::remove_dir_all(&staging).map_err(|e| format!("清理临时档案失败: {e}"))?;
    }
    std::fs::create_dir_all(&staging).map_err(|e| format!("创建临时档案失败: {e}"))?;

    let cleanup = |msg: String| {
        let _ = std::fs::remove_dir_all(&staging);
        msg
    };
    let mut count = 0usize;
    for rel in WHITELIST {
        let src = root.join(rel);
        if !src.exists() {
            continue;
        }
        overwrite_entry(root, &staging, rel);
        if src.is_dir() {
            let mut files = Vec::new();
            enumerate_tree(&src, &mut files);
            count += files.len();
        } else {
            count += 1;
        }
    }
    if count == 0 {
        return Err(cleanup("live 数据目录下没有可归档的载体文件".to_string()));
    }
    // 拷贝过程中的读取失败会被 overwrite_entry 静默跳过，换入前必须再验一次凭据，
    // 否则残缺的临时档会把好档案顶掉。
    if !matches!(has_auth_creds(&storage_path(&staging)), Some(true)) {
        return Err(cleanup(
            "临时档案缺少登录凭据（拷贝不完整），已放弃回写".to_string(),
        ));
    }
    let meta = serde_json::json!({
        "account_id": account_id,
        "created_at": chrono::Utc::now().timestamp(),
        "entries": count,
        "verify_failed_at": null,
    });
    if let Err(e) = std::fs::write(
        staging.join("meta.json"),
        serde_json::to_string_pretty(&meta).map_err(|e| e.to_string())?,
    ) {
        return Err(cleanup(format!("写入元数据失败: {e}")));
    }

    let bak = PathBuf::from(format!("{}.bak", vault.display()));
    if bak.exists() {
        let _ = std::fs::remove_dir_all(&bak);
    }
    let had_old = vault.exists();
    if had_old {
        std::fs::rename(vault, &bak).map_err(|e| cleanup(format!("轮转旧档案失败: {e}")))?;
    }
    if let Err(e) = std::fs::rename(&staging, vault) {
        if had_old {
            let _ = std::fs::rename(&bak, vault);
        }
        return Err(cleanup(format!("换入新档案失败: {e}")));
    }
    Ok(count)
}

/// 切换前把当前 live 登录态回写到「出站账号」档案。
/// live 必须确实登录着、且与出站档案是同一账号，否则跳过——宁可档案旧，
/// 也不能把 A 的登录态写进 B 的档案。账号归属靠客户端日志里的登录用户名比对。
fn archive_outgoing(
    data_dir: &Path,
    live_root: &Path,
    account_id: &str,
    expected_name: Option<&str>,
) -> Result<String, String> {
    let vault = vault_dir(data_dir, account_id);
    if !vault.join("meta.json").exists() {
        return Err("出站账号未建档".to_string());
    }
    if !matches!(has_auth_creds(&storage_path(live_root)), Some(true)) {
        return Err("当前客户端未检测到登录凭据，跳过回写以免污染档案".to_string());
    }
    let expected = expected_name
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or("无法确定出站账号名，跳过回写以免污染档案")?;
    match login_name_from_logs(&live_root.join("logs")) {
        Some(live) if live == expected => {}
        Some(live) => {
            return Err(format!(
                "当前客户端登录的是「{live}」，与出站账号「{expected}」不一致，跳过回写"
            ))
        }
        None => return Err("读不到客户端日志里的登录用户名，跳过回写以免污染档案".to_string()),
    }
    let count = save_vault_from_live(live_root, &vault, account_id)?;
    let _ = mark_vault_verify(data_dir, account_id, true);
    Ok(format!(
        "已把切换前现场回写到账号 {account_id} 的档案（{count} 个载体文件）"
    ))
}

/// 切换编排：关进程 → 当前 live 快照到临时回滚目录 → 写回目标 vault → 拉起 → 守护判定。
/// 只有「客户端没起来」才硬回滚；守护未能确认登录时保留客户端现状、给档案打标并返回
/// `SWITCH_WARN:` 前缀的提示，交给用户重新登录 / 重建档。成功后回写 storage.json 保鲜。
pub fn switch_to_vault(
    data_dir: &Path,
    account_id: &str,
    exe: &str,
    current_account_id: Option<&str>,
    current_account_name: Option<&str>,
) -> Result<String, String> {
    let _swap = acquire_swap_lock()?;
    let root = data_root()?;
    let vault = vault_dir(data_dir, account_id);
    if !vault.join("meta.json").exists() {
        return Err("该账号尚未建档：先让客户端登录该账号，关闭客户端后点「存档」".to_string());
    }
    // 切换会先杀客户端、再整目录覆盖，代价很大。先确认档案本身带凭据，
    // 坏档案直接拒绝，别让用户正常使用的客户端白白被重启一次。
    if !matches!(has_auth_creds(&storage_path(&vault)), Some(true)) {
        return Err(
            "该账号档案缺少登录凭据（storage.json 中 iCubeAuthInfo 键缺失或为空）：请让 Trae 客户端登录该账号，完全退出后重新「建档」"
                .to_string(),
        );
    }
    let image = Path::new(exe)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("Trae CN.exe")
        .to_string();
    ensure_not_self(&image)?;

    kill_client(&image)?;

    // 0) 出站账号：客户端运行期间 refresh token 可能已被轮换，切换前必须把
    //    最新 live 现场回写档案，否则下次切回来会拿着过期令牌被服务端清凭据。
    //    写入前核对日志里的登录用户名，账号对不上就跳过，绝不污染别人的档案。
    let mut notes: Vec<String> = Vec::new();
    if let Some(current) = current_account_id
        .map(str::trim)
        .filter(|c| !c.is_empty() && *c != account_id)
    {
        match archive_outgoing(data_dir, &root, current, current_account_name) {
            Ok(msg) => notes.push(msg),
            Err(e) => notes.push(format!("未回写出站账号档案：{e}")),
        }
    }
    let with_notes = |msg: String| {
        if notes.is_empty() {
            msg
        } else {
            format!("{msg}；{}", notes.join("；"))
        }
    };

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
        return Err(with_notes(format!(
            "写回目标账号失败: {e}（已回滚并尝试重启客户端）"
        )));
    }

    // 3) 拉起；失败即回滚
    if let Err(e) = launch_client(exe) {
        let _ = restore_entry_set(&rollback, &root);
        let _ = std::fs::remove_dir_all(&rollback);
        return Err(with_notes(format!("{e}（已回滚）")));
    }

    // 4) 守护：凭据缺失需持续 CLEAR_CONFIRM 才算客户端真的登出。
    //    客户端冷启动会先把 storage.json 改写成不含凭据的中间态再写回，
    //    旧实现单帧缺失即回滚，是「切到 Xu 失败」这类偶发误判的直接原因。
    let storage = storage_path(&root);
    let mut guard = GuardState::default();
    let deadline = Instant::now() + Duration::from_secs(GUARD_SECS);
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(GUARD_POLL_MS));
        if !client_running(&image) {
            // 进程短暂消失时没有采样，重置连续缺失计时，避免把冷启动期间的
            // 两帧间隔当作「持续清除 4 秒」而误判切换失败。
            guard.note_client_absent();
            continue;
        }
        guard.running_seen = true;
        let sample = match has_auth_creds(&storage) {
            Some(true) => CredSample::Present,
            Some(false) => CredSample::Missing,
            None => CredSample::Unreadable,
        };
        guard.observe(sample, Instant::now());
        // 客户端自己写回了稳定凭据 → 成功，立刻收工
        if guard.client_wrote_credentials() {
            break;
        }
        // 连续缺失足够久且没有任何写回迹象 → 失败，提前收工，不必干等满窗口
        if let Some(since) = guard.missing_since {
            if since.elapsed() >= GIVE_UP_MISSING {
                break;
            }
        }
    }

    let verdict = guard_verdict(&guard);
    match verdict {
        GuardVerdict::ClientNeverStarted => {
            // 客户端根本没起来：留在目标档案下也用不了，回滚并重启原账号
            kill_client(&image).ok();
            let _ = restore_entry_set(&rollback, &root);
            let _ = launch_client(exe);
            let _ = std::fs::remove_dir_all(&rollback);
            let _ = mark_vault_verify(data_dir, account_id, false);
            return Err(with_notes(format!(
                "切换失败：客户端进程在守护期内未起来（采样：{}）。已回滚到原账号并重启客户端",
                guard.samples()
            )));
        }
        GuardVerdict::ConfirmedLogout | GuardVerdict::CredentialsUnstable => {
            // 「不确认」不等于「失败」：不杀进程、不覆盖回滚。
            // 会话被服务端轮换时，唯一自救路径就是在这个客户端里重新登录，
            // 硬回滚到原账号会把这条路堵死，这里保持 fail-open。
            let _ = std::fs::remove_dir_all(&rollback);
            let _ = mark_vault_verify(data_dir, account_id, false);
            let why = match verdict {
                GuardVerdict::ConfirmedLogout if guard.ever_present =>
                    "客户端启动后清除了登录凭据，且等待期间没有再写回（可能是档案里的 refresh token 已失效——例如同一设备手动登录过别的账号；也可能是客户端冷启动尚未完成登录）".to_string(),
                GuardVerdict::ConfirmedLogout =>
                    "客户端启动后始终未写入登录凭据（档案可能已过期）".to_string(),
                _ => "守护结束时凭据状态不稳定".to_string(),
            };
            return Ok(with_notes(format!(
                "SWITCH_WARN:已重启 Trae CN，但未能确认登录到目标账号：{why}（采样：{}）。客户端保持当前状态、未回滚——请看一下 Trae 里现在显示的是哪个账号：如果已经是目标账号，点一次「存档」刷新档案即可消除待验证标记；如果不是，直接在 Trae 登录该账号，再回来点「存档」重新建档。",
                guard.samples()
            )));
        }
        GuardVerdict::LoggedIn => {}
    }

    let _ = std::fs::remove_dir_all(&rollback);
    // 守护确认齐全 → 清掉历史「待验证」标记
    let _ = mark_vault_verify(data_dir, account_id, true);

    // 5) 成功后回写 storage.json 到 vault（保鲜轮换后的 refresh token）。
    //    令牌只在这一份文件里；state.vscdb 只是用户态数据，运行中 copy 反而可能拿到撕裂快照。
    let dst = vault.join("User/globalStorage/storage.json");
    if let Some(parent) = dst.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let synced = std::fs::copy(&storage, &dst).is_ok();
    Ok(with_notes(if synced {
        "已切换并重启 Trae CN（守护确认登录凭据齐全）".to_string()
    } else {
        "已切换并重启 Trae CN；但 storage.json 回写建档失败，建议重新建档一次".to_string()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(base: Instant, secs: u64) -> Instant {
        base + Duration::from_secs(secs)
    }

    fn running(mut g: GuardState) -> GuardState {
        g.running_seen = true;
        g
    }

    #[test]
    fn vault_dir_migrates_legacy_layout_once() {
        let base = std::env::temp_dir().join(format!("tvault_{}", uuid::Uuid::new_v4().simple()));
        let legacy = base.join("vaults/acc-1");
        let legacy_bak = base.join("vaults/acc-1.bak");
        for dir in [&legacy, &legacy_bak] {
            std::fs::create_dir_all(dir.join("User/globalStorage")).unwrap();
            std::fs::write(dir.join("User/globalStorage/storage.json"), "payload").unwrap();
        }

        let got = vault_dir(&base, "acc-1");
        assert_eq!(got, base.join("vaults/trae/acc-1"));
        assert_eq!(
            std::fs::read_to_string(got.join("User/globalStorage/storage.json")).unwrap(),
            "payload"
        );
        assert!(!legacy.exists(), "旧目录应被搬走");
        assert!(
            base.join("vaults/trae/acc-1.bak").exists(),
            "旧备份一并搬走"
        );
        // 再调用一次仍返回新路径（不再尝试迁移）
        assert_eq!(vault_dir(&base, "acc-1"), got);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn startup_transient_clear_recovers_without_rollback() {
        // 复现「切到 Xu 失败」：客户端启动先写出不含凭据的中间态，2 秒后写回。
        // 旧实现在第一帧就 break 并回滚，这里必须判定为成功。
        let t0 = Instant::now();
        let mut g = running(GuardState::default());
        g.observe(CredSample::Missing, t(t0, 0));
        g.observe(CredSample::Missing, t(t0, 1));
        g.observe(CredSample::Present, t(t0, 2));
        g.observe(CredSample::Present, t(t0, 3));
        assert!(!g.cleared, "中间态恢复后不能判定为客户端登出");
        assert_eq!(guard_verdict(&g), GuardVerdict::LoggedIn);
        assert_eq!(g.present, 2);
        assert_eq!(g.missing, 2);
    }

    #[test]
    fn client_rewrites_credentials_after_long_clear_is_success() {
        // 线上复现：恢复进去的凭据先齐全，客户端启动后清空超过 CLEAR_CONFIRM，
        // 之后才把凭据写回来。旧循环一看到 cleared 就 break，误报切换失败。
        let t0 = Instant::now();
        let mut g = running(GuardState::default());
        g.observe(CredSample::Present, t(t0, 0));
        for s in 1..6 {
            g.observe(CredSample::Missing, t(t0, s));
        }
        assert!(g.cleared, "连续缺失 4 秒应判定为 cleared");
        g.observe(CredSample::Present, t(t0, 6));
        assert!(!g.client_wrote_credentials(), "单帧写回还不够稳定");
        g.observe(CredSample::Present, t(t0, 7));
        assert!(g.client_wrote_credentials(), "客户端自己写回凭据 = 成功信号");
        assert_eq!(guard_verdict(&g), GuardVerdict::LoggedIn);
    }

    #[test]
    fn untouched_restored_credentials_are_not_the_client_signal() {
        // 恢复进去后就一直齐全：说明客户端还没改写文件，不能据此提前宣告成功
        let t0 = Instant::now();
        let mut g = running(GuardState::default());
        for s in 0..5 {
            g.observe(CredSample::Present, t(t0, s));
        }
        assert!(!g.client_wrote_credentials());
        assert_eq!(guard_verdict(&g), GuardVerdict::LoggedIn);
    }

    #[test]
    fn sustained_missing_is_a_real_logout() {
        let t0 = Instant::now();
        let mut g = running(GuardState::default());
        for s in 0..6 {
            g.observe(CredSample::Missing, t(t0, s));
        }
        assert!(g.cleared);
        assert_eq!(guard_verdict(&g), GuardVerdict::ConfirmedLogout);
    }

    #[test]
    fn present_then_sustained_missing_is_a_real_logout() {
        // 恢复进去时凭据齐全，客户端校验 refresh token 失败后持续清除 → 必须回滚
        let t0 = Instant::now();
        let mut g = running(GuardState::default());
        g.observe(CredSample::Present, t(t0, 0));
        for s in 1..7 {
            g.observe(CredSample::Missing, t(t0, s));
        }
        assert!(g.ever_present);
        assert!(g.cleared);
        assert_eq!(guard_verdict(&g), GuardVerdict::ConfirmedLogout);
    }

    #[test]
    fn unreadable_samples_do_not_accumulate_the_clear_streak() {
        let t0 = Instant::now();
        let mut g = running(GuardState::default());
        g.observe(CredSample::Missing, t(t0, 0));
        g.observe(CredSample::Missing, t(t0, 1));
        g.observe(CredSample::Unreadable, t(t0, 2)); // 文件写到一半，重置计时
        g.observe(CredSample::Missing, t(t0, 3));
        g.observe(CredSample::Missing, t(t0, 5));
        assert!(!g.cleared, "只连续缺失 2 秒，不能判定登出");
        g.observe(CredSample::Missing, t(t0, 8));
        assert!(g.cleared);
    }

    #[test]
    fn process_gap_does_not_accumulate_the_clear_streak() {
        // 复现真实采样：400ms 轮询，客户端冷启动期间进程短暂消失数秒。
        // 只有 2 帧「缺失」且中间隔着进程空窗，绝不能误判为持续清除 4 秒。
        let t0 = Instant::now();
        let mut g = running(GuardState::default());
        g.observe(CredSample::Present, t(t0, 0));
        g.observe(CredSample::Missing, t(t0, 1));
        g.note_client_absent(); // 进程消失 5 秒，期间没有采样
        g.observe(CredSample::Missing, t(t0, 6));
        assert!(!g.cleared, "进程空窗后的缺失必须重新起算");
        assert_eq!(g.missing, 2);
        assert_eq!(guard_verdict(&g), GuardVerdict::CredentialsUnstable);
        // 之后真的连续清除才判失败
        for s in 7..12 {
            g.observe(CredSample::Missing, t(t0, s));
        }
        assert!(g.cleared);
        assert_eq!(guard_verdict(&g), GuardVerdict::ConfirmedLogout);
    }

    #[test]
    fn unreadable_tail_never_overwrites_the_vault() {
        // 最后一次采样读不到文件时不能宣称成功并回写，否则会把半成品写进档案
        let t0 = Instant::now();
        let mut g = running(GuardState::default());
        g.observe(CredSample::Present, t(t0, 0));
        g.observe(CredSample::Unreadable, t(t0, 1));
        assert_eq!(guard_verdict(&g), GuardVerdict::CredentialsUnstable);
    }

    #[test]
    fn client_never_started_is_a_hard_failure() {
        let g = GuardState::default();
        assert_eq!(guard_verdict(&g), GuardVerdict::ClientNeverStarted);
    }

    #[test]
    fn has_auth_creds_distinguishes_present_missing_and_unreadable() {
        let dir = std::env::temp_dir().join(format!("tbk_guard_{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("storage.json");
        std::fs::write(
            &p,
            r#"{"iCubeAuthInfo://usertag":"abc","iCubeAuthInfo://icube.cloudide":"def"}"#,
        )
        .unwrap();
        assert_eq!(has_auth_creds(&p), Some(true));
        std::fs::write(
            &p,
            r#"{"iCubeAuthInfo://usertag":"","iCubeAuthInfo://icube.cloudide":"def"}"#,
        )
        .unwrap();
        assert_eq!(has_auth_creds(&p), Some(false));
        std::fs::write(&p, r#"{"other":1}"#).unwrap();
        assert_eq!(has_auth_creds(&p), Some(false));
        std::fs::write(&p, r#"{"broken""#).unwrap();
        assert_eq!(has_auth_creds(&p), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn vault_verify_flag_roundtrip_and_legacy_compat() {
        let dir = std::env::temp_dir().join(format!("tbk_vault_{}", uuid::Uuid::new_v4().simple()));
        let vault = vault_dir(&dir, "acc");
        std::fs::create_dir_all(&vault).unwrap();
        let legacy = r#"{"account_id":"acc","created_at":1700000000,"entries":42}"#;
        std::fs::write(vault.join("meta.json"), legacy).unwrap();

        let info = vault_info(&dir, "acc").unwrap();
        assert_eq!(info.created_at, 1700000000);
        assert_eq!(info.entries, 42);
        assert!(
            info.verify_failed_at.is_none(),
            "旧档案无该字段应视为未标记"
        );

        mark_vault_verify(&dir, "acc", false).unwrap();
        let failed = vault_info(&dir, "acc").unwrap();
        assert!(failed.verify_failed_at.is_some());
        assert_eq!(failed.entries, 42, "打标不能丢原有字段");

        mark_vault_verify(&dir, "acc", true).unwrap();
        assert!(vault_info(&dir, "acc").unwrap().verify_failed_at.is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn symmetric_restore_protects_bindings_and_drops_sidecars() {
        for rel in KEEP_ON_MISSING {
            assert!(!should_remove_missing(rel), "设备绑定 {rel} 不能删");
        }
        for rel in SQLITE_SIDECARS {
            assert!(
                WHITELIST.contains(&rel),
                "边车 {rel} 必须仍建档，只是恢复时不写回"
            );
            assert!(!should_remove_missing(rel), "边车 {rel} 不进对称删除分支");
        }
        assert!(should_remove_missing("User/globalStorage/storage.json"));
        assert!(should_remove_missing("Network"));

        let base =
            std::env::temp_dir().join(format!("tbk_restore_{}", uuid::Uuid::new_v4().simple()));
        let vault = base.join("vault");
        let live = base.join("live");
        let write = |root: &Path, rel: &str, text: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        write(&vault, "User/globalStorage/storage.json", "new");
        write(&live, "User/globalStorage/storage.json", "stale");
        write(&live, "User/globalStorage/state.vscdb-wal", "old wal");
        write(&live, "Local State", "safeStorage key");
        write(&live, "Network/Cookies", "old cookies");
        std::fs::write(live.join("code.lock"), "lock").unwrap();

        restore_entry_set(&vault, &live).unwrap();
        assert_eq!(
            std::fs::read_to_string(live.join("User/globalStorage/storage.json")).unwrap(),
            "new"
        );
        assert!(!live.join("User/globalStorage/state.vscdb-wal").exists());
        assert!(!live.join("code.lock").exists());
        assert!(
            live.join("Local State").exists(),
            "vault 缺设备密钥时必须保留 live 版本"
        );
        assert!(
            !live.join("Network/Cookies").exists(),
            "vault 缺登录态载体时必须对称删除，避免串号"
        );
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn login_name_from_logs_uses_newest_session() {
        let base =
            std::env::temp_dir().join(format!("tbk_logname_{}", uuid::Uuid::new_v4().simple()));
        let write = |session: &str, rel: &str, line: &str| {
            let p = base.join("logs").join(session).join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, line).unwrap();
        };
        write(
            "20251125T100000",
            "window1/exthost/trae.ai-code-completion/Trae AI Code Client.log",
            "refreshUserInfo username:祸 region:cn\n",
        );
        // 更早会话里的用户名不代表现场，必须被忽略
        write(
            "20251125T181500",
            "window1/exthost/trae.ai-code-completion/Trae AI Code Client.log",
            "refreshUserInfo username:祸 region:cn\nrefreshUserInfo username:君焕 region:cn\n",
        );
        assert_eq!(
            login_name_from_logs(&base.join("logs")).as_deref(),
            Some("君焕"),
            "同一会话取最后一次用户名，且只看最新会话"
        );
        // `logs` 下还有 `aha_log` 这类非会话目录，必须被忽略
        let other = base.join("logs/aha_log/aha.log");
        std::fs::create_dir_all(other.parent().unwrap()).unwrap();
        std::fs::write(&other, "refreshUserInfo username:张三 region:cn\n").unwrap();
        assert_eq!(
            login_name_from_logs(&base.join("logs")).as_deref(),
            Some("君焕"),
            "非时间戳目录不能顶替最新会话"
        );
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn login_name_from_logs_reads_completion_log_format() {
        let base =
            std::env::temp_dir().join(format!("tbk_logname2_{}", uuid::Uuid::new_v4().simple()));
        let p = base
            .join("logs/20251125T181500/window1/exthost/trae.ai-code-completion/completion.log");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "[info] userInfo userName: 君焕\n").unwrap();
        assert_eq!(
            login_name_from_logs(&base.join("logs")).as_deref(),
            Some("君焕")
        );
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn save_vault_from_live_rotates_previous_archive_to_bak() {
        let base =
            std::env::temp_dir().join(format!("tbk_archive_{}", uuid::Uuid::new_v4().simple()));
        let live = base.join("live");
        let vault = vault_dir(&base, "acc");
        let write = |root: &Path, rel: &str, text: &str| {
            let p = root.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        write(&vault, "User/globalStorage/storage.json", "old-login");
        write(
            &live,
            "User/globalStorage/storage.json",
            r#"{"iCubeAuthInfo://usertag":"tag-a","iCubeAuthInfo://icube.cloudide":"cl-a"}"#,
        );
        write(&live, "machineid", "mid");
        write(&live, "Local State", "keys");

        let count = save_vault_from_live(&live, &vault, "acc").unwrap();
        assert_eq!(count, 3);
        assert_eq!(
            std::fs::read_to_string(vault.join("User/globalStorage/storage.json")).unwrap(),
            r#"{"iCubeAuthInfo://usertag":"tag-a","iCubeAuthInfo://icube.cloudide":"cl-a"}"#
        );
        let bak = PathBuf::from(format!("{}.bak", vault.display()));
        assert_eq!(
            std::fs::read_to_string(bak.join("User/globalStorage/storage.json")).unwrap(),
            "old-login",
            "旧档案要完整轮转到 .bak，新档写入失败时才有退路"
        );
        let meta: Value =
            serde_json::from_str(&std::fs::read_to_string(vault.join("meta.json")).unwrap())
                .unwrap();
        assert_eq!(meta["account_id"], "acc");
        assert!(meta["verify_failed_at"].is_null());
        assert!(!PathBuf::from(format!("{}.staging", vault.display())).exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn archive_outgoing_refuses_logged_out_live() {
        let base =
            std::env::temp_dir().join(format!("tbk_outgoing_{}", uuid::Uuid::new_v4().simple()));
        let live = base.join("live");
        let vault = vault_dir(&base, "acc");
        std::fs::create_dir_all(&vault).unwrap();
        std::fs::write(vault.join("meta.json"), r#"{"account_id":"acc"}"#).unwrap();
        std::fs::write(vault.join("User/globalStorage/storage.json"), "").ok();
        let p = live.join("User/globalStorage/storage.json");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, r#"{"other":1}"#).unwrap();

        let err = archive_outgoing(&base, &live, "acc", Some("祸")).unwrap_err();
        assert!(err.contains("登录凭据"), "登出态必须拒绝回写: {err}");
        assert!(
            !PathBuf::from(format!("{}.bak", vault.display())).exists(),
            "被拒绝时不能动原档案"
        );
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn archive_outgoing_skips_when_log_name_differs() {
        let base = std::env::temp_dir().join(format!(
            "tbk_outgoing_name_{}",
            uuid::Uuid::new_v4().simple()
        ));
        let live = base.join("live");
        let vault = vault_dir(&base, "acc");
        std::fs::create_dir_all(&vault).unwrap();
        std::fs::write(vault.join("meta.json"), r#"{"account_id":"acc"}"#).unwrap();
        let storage = live.join("User/globalStorage/storage.json");
        std::fs::create_dir_all(storage.parent().unwrap()).unwrap();
        std::fs::write(
            &storage,
            r#"{"iCubeAuthInfo://usertag":"tag","iCubeAuthInfo://icube.cloudide":"cl"}"#,
        )
        .unwrap();
        let log = live.join(
            "logs/20251125T181500/window1/exthost/trae.ai-code-completion/Trae AI Code Client.log",
        );
        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        std::fs::write(&log, "refreshUserInfo username:君焕 region:cn\n").unwrap();

        // live 实际登录君焕，却要回写到「祸」的档案 → 必须拒绝
        let err = archive_outgoing(&base, &live, "acc", Some("祸")).unwrap_err();
        assert!(err.contains("不一致"), "账号不一致必须跳过: {err}");
        assert!(
            !PathBuf::from(format!("{}.bak", vault.display())).exists(),
            "被拒绝时不能动原档案"
        );

        // 读不到日志（无用户名）时同样不能猜，必须跳过
        std::fs::remove_file(&log).unwrap();
        let err = archive_outgoing(&base, &live, "acc", Some("君焕")).unwrap_err();
        assert!(err.contains("日志"), "读不到用户名时必须跳过: {err}");
        std::fs::remove_dir_all(&base).unwrap();
    }
}
