use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckinStatus {
    Idle,
    Success,
    Failed,
    Cooling,
}

impl CheckinStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CheckinStatus::Idle => "idle",
            CheckinStatus::Success => "success",
            CheckinStatus::Failed => "failed",
            CheckinStatus::Cooling => "cooling",
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub platform: String,
    pub name: String,
    pub email: String,
    /// 平台侧用户 ID（WorkBuddy 切换写认证文件用；旧数据/Trae 为空）
    pub uid: String,
    /// 登录响应返回的产品域（如 www.codebuddy.cn / www.workbuddy.cn）；切换写认证文件用
    pub domain: String,
    /// 企业账号 ID（企业版积分接口分流用；个人版为空）
    pub enterprise_id: String,
    /// 平台账号类型（ultimate/exclusive 兜底判定为企业版；未知为空）
    pub account_type: String,
    /// TRAE 设备号（16 位数字，登录时生成并写进授权页；签到与换证都必须沿用它）
    pub device_id: String,
    /// TRAE 设备密钥（P-256 私钥的 32 字节 hex）：换证时用它签 DeviceProof，
    /// 签出的 JWT 才与设备绑定；未绑定设备的 JWT 会被签到接口判 9074
    pub device_key: String,
    /// TRAE 登录时的机器码（换证 DeviceInfo.MachineID 用）
    pub machine_id: String,
    pub avatar: String,
    pub access_token: String,
    pub refresh_token: String,
    pub cookies: String,
    pub token_expires_at: i64,
    pub refresh_token_expires_at: i64,
    pub last_checkin_time: i64,
    pub checkin_status: String,
    pub fail_count: i32,
    pub cooldown_until: i64,
    pub created_at: i64,
    pub updated_at: i64,
    // 真实积分（腾讯 billing 接口查询结果）；None 表示尚未查询
    pub credits_balance: Option<f64>,
    pub credits_expiries: String, // JSON: [{at, amount}]
    pub credits_fetched_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreditExpiry {
    pub at: i64,
    pub amount: f64,
    /// 积分包名称（TRAE 用 expired_ents 按到期时间匹配回填；WorkBuddy 为空）
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountCredits {
    pub account_id: String,
    pub account_name: String,
    pub platform: String,
    pub balance: f64,
    pub fetched_at: i64,
    pub packages: Vec<CreditExpiry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreditResult {
    pub account_id: String,
    pub account_name: String,
    pub success: bool,
    pub balance: Option<f64>,
    pub expiries: Vec<CreditExpiry>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountInfo {
    pub id: String,
    pub platform: String,
    pub name: String,
    pub email: String,
    pub avatar: String,
    pub token_expires_at: i64,
    pub refresh_token_expires_at: i64,
    pub last_checkin_time: i64,
    pub checkin_status: String,
    pub fail_count: i32,
    pub cooldown_until: i64,
    pub created_at: i64,
    pub credits_balance: Option<f64>,
    pub credits_expiries: Vec<CreditExpiry>,
    pub credits_fetched_at: i64,
    pub account_type: String,
    /// 当前账户已注入的客户端（trae / workbuddy / codebuddy_cn），由 list_accounts 依据 active_client_* 设置填充
    #[serde(default)]
    pub clients: Vec<String>,
}

impl From<Account> for AccountInfo {
    fn from(a: Account) -> Self {
        let expiries: Vec<CreditExpiry> =
            serde_json::from_str(&a.credits_expiries).unwrap_or_default();
        AccountInfo {
            id: a.id,
            platform: a.platform,
            name: a.name,
            email: a.email,
            avatar: a.avatar,
            token_expires_at: a.token_expires_at,
            refresh_token_expires_at: a.refresh_token_expires_at,
            last_checkin_time: a.last_checkin_time,
            checkin_status: a.checkin_status,
            fail_count: a.fail_count,
            cooldown_until: a.cooldown_until,
            created_at: a.created_at,
            credits_balance: a.credits_balance,
            credits_expiries: expiries,
            credits_fetched_at: a.credits_fetched_at,
            account_type: a.account_type,
            clients: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckinRecord {
    pub id: String,
    pub account_id: String,
    pub platform: String,
    pub status: String,
    pub points: i64,
    pub message: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PointsHistory {
    pub id: String,
    pub account_id: String,
    pub date: String,
    pub points: i64,
    pub source: String,
    pub expires_at: i64,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PointsDashboard {
    pub total_points: i64,
    pub today_new: i64,
    pub trend: Vec<DailyPoints>,
    pub expiring_packages: Vec<ExpiringPackage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailyPoints {
    pub date: String,
    pub points: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpiringPackage {
    pub account_name: String,
    pub platform: String,
    pub points: f64,
    pub expires_at: i64,
    pub days_left: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckinResult {
    pub account_id: String,
    pub account_name: String,
    pub platform: String,
    pub success: bool,
    /// 自动跳过（今日已签到 / 凭证已失效 / 冷却中），未请求上游
    pub skipped: bool,
    pub points: i64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppLog {
    pub id: String,
    pub level: String, // info | success | error
    pub action: String,
    pub message: String,
    pub created_at: i64,
}

fn default_theme() -> String {
    "light".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    pub log_retention_days: i64,
    pub minimize_to_tray_on_start: bool,
    #[serde(default = "default_theme")]
    pub theme: String, // light | dark | system
    pub client_path_trae: String,
    pub client_path_workbuddy: String,
    #[serde(default)]
    pub client_path_workbuddy_ai: String,
    pub client_path_codebuddy_cn: String,
}

impl Default for AppSettings {
    fn default() -> Self {
        AppSettings {
            log_retention_days: 7,
            minimize_to_tray_on_start: false,
            theme: "light".to_string(),
            client_path_trae: String::new(),
            client_path_workbuddy: String::new(),
            client_path_workbuddy_ai: String::new(),
            client_path_codebuddy_cn: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckinSettings {
    pub enabled: bool,
    pub hour: u32,
    pub minute: u32,
    pub cooldown_minutes: u32,
    pub max_fail_count: u32,
    pub interval_seconds: u32,
    /// 定时签到平台范围（trae / workbuddy / codebuddy_cn）；空 = 全部平台
    #[serde(default)]
    pub platforms: Vec<String>,
}

impl Default for CheckinSettings {
    fn default() -> Self {
        CheckinSettings {
            enabled: true,
            hour: 9,
            minute: 0,
            cooldown_minutes: 30,
            max_fail_count: 3,
            interval_seconds: 3,
            platforms: Vec::new(),
        }
    }
}
