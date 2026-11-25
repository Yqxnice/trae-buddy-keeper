use serde::Serialize;

/// 客户端的本地安装路径（未安装为空字符串）
#[derive(Debug, Clone, Serialize, Default)]
pub struct ClientPaths {
    pub trae: String,
    pub workbuddy: String,
    pub workbuddy_ai: String,
    pub codebuddy_cn: String,
    pub qoder_cn: String,
}

/// 按 DisplayName 匹配卸载项：TRAE/TRAE CN、WorkBuddy（排除 Manager/Switch 等第三方）、
/// WorkBuddy AI（国际版客户端，须先于 WorkBuddy 判定）、CodeBuddy CN/CodeBuddy、
/// Qoder CN（排除 Qoder IDE / Qoder Work 等其它产品线）
fn classify(display_name: &str) -> Option<&'static str> {
    let n = display_name.trim();
    let lower = n.to_lowercase();
    // 本项目安装版 DisplayName「TraeBuddy Keeper」同样以 trae 开头，必须先排除，
    // 否则自动获取会把自身安装路径当成 Trae 客户端，后续存档/切换会误杀自己
    if lower.starts_with("traebuddy") || lower.starts_with("trae buddy") {
        return None;
    }
    if lower.starts_with("trae") {
        return Some("trae");
    }
    if lower.starts_with("workbuddy ai") {
        return Some("workbuddy_ai");
    }
    // 排除第三方管理工具（其名称含 manager / switch），官方客户端名不含该词
    if lower.starts_with("workbuddy") && !lower.contains("manager") && !lower.contains("switch") {
        return Some("workbuddy");
    }
    if lower.starts_with("codebuddy") {
        return Some("codebuddy_cn");
    }
    if lower.starts_with("qoder cn") {
        return Some("qoder_cn");
    }
    if lower.starts_with("qoder") {
        // 其它 Qoder 产品线不是 CN 客户端；早期按前缀宽泛匹配会把它们的
        // 安装路径写进 Qoder CN 设置，导致切换与进程检测全部指向错误客户端
        if lower.contains("ide") || lower.contains("work") || lower.contains("cli") {
            return None;
        }
        return Some("qoder_cn");
    }
    None
}

/// 探测结果兜底：exe 文件名与本应用自身相同则视为误配，一律跳过
fn is_self_exe(path: &str) -> bool {
    let name = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_lowercase());
    let self_name = std::env::current_exe().ok().and_then(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_lowercase())
    });
    match (name, self_name) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

fn valid_exe(path: &str) -> Option<String> {
    // DisplayIcon 可能是 "C:\x\app.exe,0" 形式，去掉图标索引
    let p = path
        .trim()
        .trim_matches('"')
        .split(',')
        .next()
        .unwrap_or("")
        .trim()
        .to_string();
    if p.to_lowercase().ends_with(".exe") && std::path::Path::new(&p).exists() {
        Some(p)
    } else {
        None
    }
}

/// DisplayIcon 指向 .ico 时（Qoder CN 安装版写的是 uninstallerIcon.ico），
/// 在其所在目录找主程序：优先与安装目录同名的 exe，排除卸载/安装器。
fn exe_from_icon_dir(icon: &str) -> Option<String> {
    let raw = icon
        .trim()
        .trim_matches('"')
        .split(',')
        .next()
        .unwrap_or("")
        .trim();
    if raw.is_empty() {
        return None;
    }
    let dir = std::path::Path::new(raw).parent()?;
    let mut candidates: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let full = path.to_string_lossy().to_string();
            let lower = full.to_lowercase();
            let is_exe = path
                .extension()
                .is_some_and(|x| x.to_string_lossy().eq_ignore_ascii_case("exe"));
            (is_exe
                && !lower.contains("unins")
                && !lower.contains("setup")
                && !lower.contains("installer"))
            .then_some(full)
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let dir_name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !dir_name.is_empty() {
        let wanted = format!("{dir_name}.exe");
        if let Some(found) = candidates
            .iter()
            .find(|s| s.to_lowercase().ends_with(&wanted))
        {
            return Some(found.clone());
        }
    }
    candidates.sort();
    candidates.into_iter().next()
}

/// 从注册表卸载项读取客户端安装路径（支持自定义安装目录，如 D 盘）
#[cfg(windows)]
fn detect_from_registry() -> ClientPaths {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let mut out = ClientPaths::default();
    // DisplayIcon 一般是 exe 全路径；PowerShell 输出 JSON 便于解析
    let script = r#"Get-ItemProperty "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*","HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*","HKLM:\Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\*" -ErrorAction SilentlyContinue | Select-Object DisplayName,DisplayIcon,InstallLocation | ConvertTo-Json -Compress"#;
    let output = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let Ok(output) = output else {
        return out;
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
        return out;
    };
    // 单条时 ConvertTo-Json 返回对象而非数组
    let items: Vec<&serde_json::Value> = match &v {
        serde_json::Value::Array(arr) => arr.iter().collect(),
        serde_json::Value::Object(_) => vec![&v],
        _ => return out,
    };
    for item in items {
        let name = item
            .get("DisplayName")
            .and_then(|s| s.as_str())
            .unwrap_or("");
        let Some(client) = classify(name) else {
            continue;
        };
        let done = match client {
            "trae" => !out.trae.is_empty(),
            "workbuddy" => !out.workbuddy.is_empty(),
            "workbuddy_ai" => !out.workbuddy_ai.is_empty(),
            "qoder_cn" => !out.qoder_cn.is_empty(),
            _ => !out.codebuddy_cn.is_empty(),
        };
        if done {
            continue;
        }
        let icon = item
            .get("DisplayIcon")
            .and_then(|s| s.as_str())
            .unwrap_or("");
        let found = valid_exe(icon)
            .or_else(|| {
                // Qoder CN 安装版的 DisplayIcon 是 .ico，从同目录推主程序
                if client == "qoder_cn" {
                    exe_from_icon_dir(icon)
                } else {
                    None
                }
            })
            .or_else(|| {
                // InstallLocation + 目录下第一个 exe（排除卸载程序）
                let dir = item
                    .get("InstallLocation")
                    .and_then(|s| s.as_str())
                    .unwrap_or("")
                    .trim()
                    .trim_matches('"');
                if dir.is_empty() {
                    return None;
                }
                let rd = std::fs::read_dir(dir).ok()?;
                for e in rd.flatten() {
                    let p = e.path();
                    if p.extension().is_some_and(|x| x == "exe") {
                        let s = p.to_string_lossy().to_string();
                        if !s.to_lowercase().contains("unins") {
                            return Some(s);
                        }
                    }
                }
                None
            });
        if let Some(path) = found {
            // 兜底：即使 DisplayName 绕过了分类排除，也不允许把自身当作客户端
            if is_self_exe(&path) {
                continue;
            }
            match client {
                "trae" => out.trae = path,
                "workbuddy" => out.workbuddy = path,
                "workbuddy_ai" => out.workbuddy_ai = path,
                "qoder_cn" => out.qoder_cn = path,
                _ => out.codebuddy_cn = path,
            }
        }
    }
    out
}

#[cfg(not(windows))]
fn detect_from_registry() -> ClientPaths {
    ClientPaths::default()
}

/// 仅从注册表卸载项探测。找不到即视为未安装，留空。
pub fn detect_client_paths() -> ClientPaths {
    detect_from_registry()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qoder_cn_is_not_mixed_with_other_product_lines() {
        assert_eq!(classify("Qoder CN 0.5.13"), Some("qoder_cn"));
        assert_eq!(classify("Qoder CN IDE"), Some("qoder_cn"));
        assert_eq!(classify("Qoder IDE (User)"), None);
        assert_eq!(classify("Qoder Work"), None);
        assert_eq!(classify("Qoder CLI"), None);
        // 旧安装形态可能只叫 Qoder，仍归到 CN
        assert_eq!(classify("Qoder"), Some("qoder_cn"));
    }

    #[test]
    fn self_and_third_party_names_are_excluded() {
        assert_eq!(classify("TraeBuddy Keeper"), None);
        assert_eq!(classify("WorkBuddy Manager"), None);
        assert_eq!(classify("CodeBuddy CN"), Some("codebuddy_cn"));
    }

    #[test]
    fn icon_dir_picks_main_exe_not_uninstaller() {
        let dir = std::env::temp_dir().join("tbk_clients_icon_test/Qoder CN");
        let _ = std::fs::create_dir_all(&dir);
        let main = dir.join("Qoder CN.exe");
        let unins = dir.join("Uninstall Qoder CN.exe");
        std::fs::write(&main, b"").unwrap();
        std::fs::write(&unins, b"").unwrap();
        let icon = dir.join("uninstallerIcon.ico");
        std::fs::write(&icon, b"").unwrap();
        let found = exe_from_icon_dir(&icon.to_string_lossy());
        assert_eq!(found.as_deref(), Some(main.to_string_lossy().as_ref()));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
