//! WorkBuddy 档位（国内版 / 国际版）的单一事实来源。
//!
//! 档位差异（API 域名、认证文件名、OAuth platform、能力声明）只允许从这里取，
//! 其它模块不得再出现档位相关字面量。缺省档位为 `Cn`，历史数据零迁移：
//! 账号档位由落库的 `domain` 后缀判定（`.workbuddy.ai` = 国际版）。
//! 实现参考了开源项目 workbuddy-switch 的 variant.rs。

use crate::data::models::Account;

/// 国际版 domain 判据：该后缀视为国际版（只认后缀，避免相似域名误判）
const AI_DOMAIN_SUFFIX: &str = ".workbuddy.ai";

/// WorkBuddy 档位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WbVariant {
    /// 国内版。
    Cn,
    /// 国际版。
    Ai,
}

/// 从账号判定档位：`domain` 后缀 `.workbuddy.ai` 即国际版，其余（含空域名）回落国内版。
pub fn variant_of(acc: &Account) -> WbVariant {
    if has_ai_domain_suffix(&acc.domain) {
        WbVariant::Ai
    } else {
        WbVariant::Cn
    }
}

/// 解析前端传入的档位字符串；未知/空一律回落国内版（向后兼容）。
pub fn parse_variant(raw: Option<&str>) -> WbVariant {
    match raw.map(str::trim).unwrap_or_default().to_ascii_lowercase().as_str() {
        "ai" | "intl" | "global" => WbVariant::Ai,
        _ => WbVariant::Cn,
    }
}

/// 域名是否为国际版域（只认后缀，避免相似域名被误判）。
pub fn has_ai_domain_suffix(domain: &str) -> bool {
    domain.trim().to_ascii_lowercase().ends_with(AI_DOMAIN_SUFFIX)
}

impl WbVariant {
    /// API 基址（登录 / 刷新 / billing / 积分同源）。
    pub fn api_endpoint(self) -> &'static str {
        match self {
            Self::Cn => "https://www.codebuddy.cn",
            Self::Ai => "https://www.workbuddy.ai",
        }
    }

    /// OAuth `platform` 参数：国际版桌面会话必须用 `workbuddy-ai`，
    /// 用国内版 platform 拿到的 token 写进国际版客户端会被拒登。
    pub fn oauth_platform(self) -> &'static str {
        match self {
            Self::Cn => "workbuddy",
            Self::Ai => "workbuddy-ai",
        }
    }

    /// 官方登录态文件名（两档位同目录、不同文件）。
    pub fn auth_file_name(self) -> &'static str {
        match self {
            Self::Cn => "workbuddy-desktop.info",
            Self::Ai => "workbuddy-desktop-ai.info",
        }
    }

    /// Windows 客户端进程镜像名。
    pub fn process_image(self) -> &'static str {
        match self {
            Self::Cn => "WorkBuddy.exe",
            Self::Ai => "WorkBuddyAI.exe",
        }
    }

    /// 该档位是否支持签到。国际版没有签到接口：签到链路（手动/批量/定时/补签）
    /// 在发起任何请求前一律跳过国际版账号，且不产生签到记录。
    pub fn supports_checkin(self) -> bool {
        matches!(self, Self::Cn)
    }

    /// 域名是否与本档位相符（登录响应校验；空域名无法判定，视为相符）。
    pub fn matches_domain(self, domain: &str) -> bool {
        let domain = domain.trim();
        if domain.is_empty() {
            return true;
        }
        match self {
            Self::Ai => has_ai_domain_suffix(domain),
            Self::Cn => {
                (domain == "www.codebuddy.cn" || domain == "www.workbuddy.cn")
                    && !has_ai_domain_suffix(domain)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acc(domain: &str) -> Account {
        Account {
            domain: domain.to_string(),
            ..Account::default()
        }
    }

    #[test]
    fn variant_from_domain_suffix() {
        assert_eq!(variant_of(&acc("www.workbuddy.ai")), WbVariant::Ai);
        assert_eq!(variant_of(&acc("www.codebuddy.cn")), WbVariant::Cn);
        assert_eq!(variant_of(&acc("www.workbuddy.ai.evil.com")), WbVariant::Cn);
        assert_eq!(variant_of(&acc("")), WbVariant::Cn);
    }

    #[test]
    fn domain_match_guards_cross_variant() {
        assert!(WbVariant::Ai.matches_domain("www.workbuddy.ai"));
        assert!(!WbVariant::Ai.matches_domain("www.codebuddy.cn"));
        assert!(WbVariant::Cn.matches_domain("www.workbuddy.cn"));
        assert!(!WbVariant::Cn.matches_domain("www.workbuddy.ai"));
        assert!(WbVariant::Ai.matches_domain(""));
    }

    #[test]
    fn endpoints_and_files_differ() {
        assert_eq!(WbVariant::Cn.api_endpoint(), "https://www.codebuddy.cn");
        assert_eq!(WbVariant::Ai.api_endpoint(), "https://www.workbuddy.ai");
        assert_eq!(WbVariant::Cn.oauth_platform(), "workbuddy");
        assert_eq!(WbVariant::Ai.oauth_platform(), "workbuddy-ai");
        assert_eq!(
            WbVariant::Ai.auth_file_name(),
            "workbuddy-desktop-ai.info"
        );
        assert_eq!(WbVariant::Cn.process_image(), "WorkBuddy.exe");
        assert_eq!(WbVariant::Ai.process_image(), "WorkBuddyAI.exe");
        assert!(!WbVariant::Ai.supports_checkin());
    }
}
