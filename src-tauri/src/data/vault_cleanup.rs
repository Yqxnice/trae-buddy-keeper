//! 删除账户时清理该账号的客户端登录档案（vault）。
//!
//! 背景：档案按平台分层存放——`vaults/trae/<id>`、`vaults/qoder_cn/<id>`。
//! 旧版 Trae 档案直接散在 `vaults/<id>`，兼容清理时仍会扫描该位置。
//! 旧版删除账户只删数据库行，档案目录永久残留——既堆积孤儿目录，又留下
//! 可离线解密的登录态密文。本模块把清理做成可单测的纯逻辑。
//!
//! 安全边界：
//! - 只按精确 account_id 目录操作，绝不触碰 qoder_cn 的 `last` / `last.bak`
//!   （那是全局「切换前现场」，不属于任何账号）。
//! - account_id 必须是单段路径名，拒绝 `..` / 分隔符 / 平台目录名，防止路径穿越。
//! - 目录不存在视为正常（账号可能从未建档），不报错。

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// account_id 必须是单个路径段（数据库里是 UUID；拒绝一切穿越写法与保留名）。
///
/// 保留名是硬安全边界：Windows 文件系统大小写不敏感，若 id 恰为 `qoder_cn`
/// 或 `trae`，旧布局下 `vaults/<id>` 就会指向整个平台档案根目录；若为 `last`
/// 则指向全局回滚槽。任何 `.bak` 结尾的 id 同理会命中别人的备份目录。
pub fn valid_account_id(account_id: &str) -> bool {
    if account_id.is_empty()
        || account_id == "."
        || account_id == ".."
        || account_id.contains('/')
        || account_id.contains('\\')
        || Path::new(account_id).file_name() != Some(OsStr::new(account_id))
    {
        return false;
    }
    let lower = account_id.to_ascii_lowercase();
    !matches!(lower.as_str(), "qoder_cn" | "trae" | "last" | "vaults") && !lower.ends_with(".bak")
}

/// 该账号可能存在的全部 vault 目录（Trae 新/旧布局 + Qoder CN 子目录）。
pub fn account_vault_targets(data_dir: &Path, account_id: &str) -> Vec<PathBuf> {
    vec![
        data_dir.join("vaults/trae").join(account_id),
        data_dir.join("vaults/qoder_cn").join(account_id),
        // 旧布局遗留：清账号时一并收掉，避免再次制造孤儿目录
        data_dir.join("vaults").join(account_id),
    ]
}

fn backup_path(dir: &Path) -> PathBuf {
    PathBuf::from(format!("{}.bak", dir.display()))
}

/// 清理账号档案。
///
/// `keep_backup = true`：把主档轮转为 `<dir>.bak`（旧 .bak 先删，与 qoder 切换轮转同语义）；
/// `keep_backup = false`：主档与其 .bak 一并删除。返回人类可读的处理摘要。
pub fn cleanup_account_vaults(
    data_dir: &Path,
    account_id: &str,
    keep_backup: bool,
) -> Result<Vec<String>, String> {
    if !valid_account_id(account_id) {
        return Err(format!("非法的账户 id：{account_id}"));
    }
    let mut notes = Vec::new();
    for dir in account_vault_targets(data_dir, account_id) {
        let bak = backup_path(&dir);
        if !dir.exists() && !bak.exists() {
            continue;
        }
        if keep_backup {
            if dir.exists() {
                if bak.exists() {
                    std::fs::remove_dir_all(&bak)
                        .map_err(|e| format!("删除旧档案备份 {} 失败：{e}", bak.display()))?;
                }
                std::fs::rename(&dir, &bak)
                    .map_err(|e| format!("轮转档案 {} 失败：{e}", dir.display()))?;
                notes.push(format!("档案已保留为备份 {}", bak.display()));
            } else if bak.exists() {
                notes.push(format!("档案备份已存在 {}", bak.display()));
            }
        } else {
            if dir.exists() {
                std::fs::remove_dir_all(&dir)
                    .map_err(|e| format!("删除档案 {} 失败：{e}", dir.display()))?;
                notes.push(format!("已删除档案 {}", dir.display()));
            }
            if bak.exists() {
                std::fs::remove_dir_all(&bak)
                    .map_err(|e| format!("删除档案备份 {} 失败：{e}", bak.display()))?;
                notes.push(format!("已删除档案备份 {}", bak.display()));
            }
        }
    }
    Ok(notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "keeper_vault_cleanup_{name}_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn seed(dir: &Path, payload: &str) {
        std::fs::create_dir_all(dir.join("User/globalStorage")).unwrap();
        std::fs::write(dir.join("User/globalStorage/storage.json"), payload).unwrap();
        std::fs::write(dir.join("meta.json"), "{}").unwrap();
    }

    #[test]
    fn rejects_path_traversal_ids() {
        for id in ["", ".", "..", "../other", "a/b", "a\\b"] {
            assert!(!valid_account_id(id), "应拒绝 {id}");
        }
        assert!(valid_account_id("82ca0d50-9b24-492c-840d-ed3d08162709"));
    }

    #[test]
    fn rejects_reserved_ids_that_would_hit_shared_slots() {
        for id in [
            "qoder_cn",
            "QODER_CN",
            "trae",
            "TRAE",
            "last",
            "LAST",
            "acc-1.bak",
            "last.BAK",
            "vaults",
        ] {
            assert!(!valid_account_id(id), "保留名应拒绝 {id}");
        }
    }

    #[test]
    fn targets_cover_both_platforms_and_legacy_layout() {
        let d = Path::new("C:/data");
        let t = account_vault_targets(d, "acc-1");
        assert_eq!(t[0], d.join("vaults/trae/acc-1"));
        assert_eq!(t[1], d.join("vaults/qoder_cn/acc-1"));
        assert_eq!(t[2], d.join("vaults/acc-1"));
    }

    #[test]
    fn delete_removes_main_and_backup_but_not_last_slot() {
        let d = tmp("delete");
        let trae = d.join("vaults/trae/acc-1");
        let legacy = d.join("vaults/acc-1");
        let qoder = d.join("vaults/qoder_cn/acc-1");
        let last = d.join("vaults/qoder_cn/last");
        seed(&trae, "trae");
        seed(&legacy, "legacy trae");
        seed(&qoder, "qoder");
        seed(&qoder.with_extension("bak"), "old");
        seed(&last, "global rollback");
        seed(&last.with_extension("bak"), "global rollback bak");

        let notes = cleanup_account_vaults(&d, "acc-1", false).unwrap();
        assert_eq!(notes.len(), 4, "{notes:?}");
        assert!(!trae.exists());
        assert!(!legacy.exists());
        assert!(!qoder.exists());
        assert!(!qoder.with_extension("bak").exists());
        assert!(last.exists(), "last 槽不能被删");
        assert!(last.with_extension("bak").exists(), "last.bak 不能被删");
    }

    #[test]
    fn keep_backup_rotates_and_replaces_previous_bak() {
        let d = tmp("bak");
        let trae = d.join("vaults/trae/acc-1");
        seed(&trae, "new");
        seed(&trae.with_extension("bak"), "older");

        let notes = cleanup_account_vaults(&d, "acc-1", true).unwrap();
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(!trae.exists());
        let bak = trae.with_extension("bak");
        assert_eq!(
            std::fs::read_to_string(bak.join("User/globalStorage/storage.json")).unwrap(),
            "new"
        );
    }

    #[test]
    fn missing_vaults_are_a_noop() {
        let d = tmp("noop");
        assert!(cleanup_account_vaults(&d, "never-archived", false)
            .unwrap()
            .is_empty());
    }
}
