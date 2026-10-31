use serde::Serialize;

/// 三个客户端的本地安装路径（未安装为空字符串）
#[derive(Debug, Clone, Serialize, Default)]
pub struct ClientPaths {
    pub trae: String,
    pub workbuddy: String,
    pub codebuddy_cn: String,
}

/// 按 DisplayName 匹配卸载项：TRAE/TRAE CN、WorkBuddy（排除 Manager）、CodeBuddy CN/CodeBuddy
fn classify(display_name: &str) -> Option<&'static str> {
    let n = display_name.trim();
    let lower = n.to_lowercase();
    if lower.starts_with("trae") {
        return Some("trae");
    }
    if lower.starts_with("workbuddy") && !lower.contains("manager") {
        return Some("workbuddy");
    }
    if lower.starts_with("codebuddy") {
        return Some("codebuddy_cn");
    }
    None
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
        let Some(client) = classify(name) else { continue };
        let done = match client {
            "trae" => !out.trae.is_empty(),
            "workbuddy" => !out.workbuddy.is_empty(),
            _ => !out.codebuddy_cn.is_empty(),
        };
        if done {
            continue;
        }
        let icon = item
            .get("DisplayIcon")
            .and_then(|s| s.as_str())
            .unwrap_or("");
        let found = valid_exe(icon).or_else(|| {
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
            match client {
                "trae" => out.trae = path,
                "workbuddy" => out.workbuddy = path,
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
