import { invoke } from "@tauri-apps/api/core";

export interface CreditExpiry {
  at: number;
  amount: number;
  /** 积分包名称（TRAE 回填；WorkBuddy 为空） */
  name?: string;
}

export interface AccountInfo {
  id: string;
  platform: string;
  name: string;
  email: string;
  avatar: string;
  token_expires_at: number;
  refresh_token_expires_at: number;
  last_checkin_time: number;
  checkin_status: string;
  fail_count: number;
  cooldown_until: number;
  created_at: number;
  credits_balance: number | null;
  credits_expiries: CreditExpiry[];
  credits_fetched_at: number;
  /** 账号域名（WorkBuddy 国内/国际按此后缀区分：含 workbuddy.ai 即国际版） */
  domain?: string;
  /** 当前账户已注入的客户端（trae / workbuddy / codebuddy_cn） */
  clients?: string[];
}

export interface CreditResult {
  account_id: string;
  account_name: string;
  success: boolean;
  balance: number | null;
  expiries: CreditExpiry[];
  message: string;
}

// 完整 Account（含凭据）：credits_expiries 后端存的是 JSON 字符串
export interface Account extends Omit<AccountInfo, "credits_expiries"> {
  access_token: string;
  refresh_token: string;
  cookies: string;
  updated_at: number;
  credits_expiries: string;
}

export interface AccountCredits {
  account_id: string;
  account_name: string;
  platform: string;
  balance: number;
  fetched_at: number;
  packages: CreditExpiry[];
}

export interface CheckinResult {
  account_id: string;
  account_name: string;
  platform: string;
  success: boolean;
  /** 自动跳过（今日已签到 / 凭证已失效） */
  skipped: boolean;
  points: number;
  message: string;
}

export interface CheckinRecord {
  id: string;
  account_id: string;
  platform: string;
  status: string;
  points: number;
  message: string;
  created_at: number;
}

export interface PointsDashboard {
  total_points: number;
  today_new: number;
  trend: { date: string; points: number }[];
  expiring_packages: {
    account_name: string;
    platform: string;
    points: number;
    expires_at: number;
    days_left: number;
  }[];
}

export interface AppLog {
  id: string;
  level: string;
  action: string;
  message: string;
  created_at: number;
}

export interface CheckinSettings {
  enabled: boolean;
  hour: number;
  minute: number;
  cooldown_minutes: number;
  max_fail_count: number;
  interval_seconds: number;
  /** 定时签到平台范围；空数组 = 全部平台 */
  platforms: string[];
}

/** 定时积分查询配置：与定时签到并列 */
export interface CreditScheduleSettings {
  enabled: boolean;
  /** 查询间隔（分钟），默认 360（6 小时） */
  interval_minutes: number;
  /** 平台范围（trae / workbuddy）；空数组 = 全部平台 */
  platforms: string[];
}

/** 最近一次定时积分查询的执行摘要 */
export interface CreditScheduleRun {
  at: number;
  success: number;
  failed: number;
  message: string;
}

export interface CreditScheduleState {
  settings: CreditScheduleSettings;
  last_run: CreditScheduleRun;
}

export const api = {
  listAccounts: () => invoke<AccountInfo[]>("list_accounts"),
  getAccountDetail: (id: string) => invoke<Account | null>("get_account_detail", { id }),
  updateAccount: (p: {
    id: string;
    name: string;
    email: string;
    access_token: string;
    refresh_token: string;
    cookies: string;
  }) => invoke<void>("update_account", p),
  addAccountManual: (p: {
    platform: string;
    name: string;
    email: string;
    access_token: string;
    refresh_token: string;
    cookies: string;
  }) => invoke<string>("add_account_manual", p),
  addAccountImport: (json: string) => invoke<string>("add_account_import", { json }),
  deleteAccount: (id: string) => invoke<void>("delete_account", { id }),
  checkinOne: (id: string) => invoke<CheckinResult>("checkin_one", { id }),
  /** source 仅用于后端日志标注触发来源：托盘传 "tray"，界面按钮不传 */
  checkinAll: (platform?: string, source?: "tray" | "scheduled" | "catchup") =>
    invoke<CheckinResult[]>("checkin_all", {
      platform: platform ?? null,
      source: source ?? null,
    }),
  getCheckinRecords: (accountId?: string, limit = 50) =>
    invoke<CheckinRecord[]>("get_checkin_records", { accountId, limit }),
  getPointsDashboard: (platform: string) =>
    invoke<PointsDashboard>("get_points_dashboard", { platform }),
  getCheckinSettings: () => invoke<CheckinSettings>("get_checkin_settings"),
  setCheckinSettings: (s: CheckinSettings) => invoke<void>("set_checkin_settings", { settings: s }),
  getCreditSchedule: () => invoke<CreditScheduleState>("get_credit_schedule"),
  setCreditSchedule: (s: CreditScheduleSettings) =>
    invoke<void>("set_credit_schedule", { settings: s }),
  getNotificationMode: () => invoke<string>("get_notification_mode"),
  setNotificationMode: (mode: string) => invoke<void>("set_notification_mode", { mode }),
  renewToken: (id: string) => invoke<string>("renew_token", { id }),
  autoRenewAll: () => invoke<string[]>("auto_renew_all"),
  startTraeOauth: () => invoke<string>("start_trae_oauth"),
  stopTraeOauth: () => invoke<void>("stop_trae_oauth"),
  completeTraeOauth: (callbackUrl: string, name?: string) =>
    invoke<string>("complete_trae_oauth", { callbackUrl, name: name ?? null }),
  startWorkbuddyLogin: (variant: WbVariant = "cn") =>
    invoke<{ state: string; authUrl: string }>("start_workbuddy_login", { variant }),
  pollWorkbuddyLogin: (loginState: string, variant: WbVariant = "cn") =>
    invoke<{ status: string; name: string }>("poll_workbuddy_login", { loginState, variant }),
  switchAccount: (id: string) => invoke<string>("switch_account", { id }),
  switchWorkbuddyClient: (id: string) =>
    invoke<string>("switch_workbuddy_client", { id }),
  switchCodebuddyCnClient: (id: string) =>
    invoke<string>("switch_codebuddy_cn_client", { id }),
  switchTraeClient: (id: string) =>
    invoke<string>("switch_trae_client", { id }),
  traeClientRunning: () => invoke<boolean>("trae_client_running"),
  backupTraeVault: (id: string, closeRunning = false) =>
    invoke<string>("backup_trae_vault", { id, closeRunning }),
  traeVaultInfo: (id: string) =>
    invoke<[number, number] | null>("trae_vault_info", { id }),
  refreshCredits: (id: string) => invoke<CreditResult>("refresh_credits", { id }),
  refreshAllCredits: (platform?: string) =>
    invoke<CreditResult[]>("refresh_all_credits", { platform: platform ?? null }),
  getCreditPackages: (platform: string) =>
    invoke<AccountCredits[]>("get_credit_packages", { platform }),
  getLogs: (limit = 200) => invoke<AppLog[]>("get_logs", { limit }),
  clearLogs: () => invoke<void>("clear_logs"),

  // ===== 通用设置 =====
  getAppSettings: () => invoke<AppSettings>("get_app_settings"),
  getDataDir: () => invoke<string>("get_data_dir"),
  openDataDir: () => invoke<void>("open_data_dir"),
  openProjectRepo: () => invoke<void>("open_project_repo"),
  backupData: () => invoke<string | null>("backup_data"),
  importData: () => invoke<string | null>("import_data"),
  setAppSettings: (settings: AppSettings) =>
    invoke<void>("set_app_settings", { settings }),
  detectClientPaths: () => invoke<ClientPaths>("detect_client_paths"),

  // ===== Token 统计 =====
  getTokenStatistics: (days?: number, force = false) =>
    invoke<TokenStatistics>("get_token_statistics", { days: days ?? null, force }),
};

export interface AppSettings {
  log_retention_days: number;
  minimize_to_tray_on_start: boolean;
  /** 外观主题：light | dark | system */
  theme: string;
  client_path_trae: string;
  client_path_workbuddy: string;
  client_path_workbuddy_ai: string;
  client_path_codebuddy_cn: string;
}

export interface ClientPaths {
  trae: string;
  workbuddy: string;
  workbuddy_ai: string;
  codebuddy_cn: string;
}

/** WorkBuddy 档位：国内版 cn / 国际版 ai */
export type WbVariant = "cn" | "ai";

// ===== Token 统计 =====

export interface TokenStatsTotals {
  total: number;
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  uncachedInput: number;
  records: number;
  cacheHitRate: number | null;
}

export interface TokenStatsGroup extends TokenStatsTotals {
  key: string;
  title?: string | null;
  project?: string;
  sessionId?: string;
}

export interface TokenStatsSource {
  source: string;
  summary: TokenStatsTotals;
  models: TokenStatsGroup[];
  projects: TokenStatsGroup[];
  sessions: TokenStatsGroup[];
  daily: TokenStatsGroup[];
  dailyByModel: Record<string, TokenStatsGroup[]>;
  hours: TokenStatsGroup[];
  filesScanned: number;
  parseErrors: number;
  coverageStartAt: number | null;
  coverageEndAt: number | null;
  /** 云端源（trae）专用：中性提示（未添加账号 / 回溯上限） */
  notice?: string | null;
  /** 云端源（trae）专用：按账号拉取失败的汇总 */
  errors?: string | null;
}

export interface TokenStatistics {
  generatedAt: number;
  rangeDays?: number | null;
  sources: TokenStatsSource[];
}
