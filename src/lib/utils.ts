export function formatDate(ts: number): string {
  if (!ts) return "—";
  return new Date(ts * 1000).toLocaleString("zh-CN");
}

export function formatRelative(ts: number): string {
  if (!ts) return "从未";
  const diff = Date.now() - ts * 1000;
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return "刚刚";
  if (mins < 60) return `${mins} 分钟前`;
  const hours = Math.floor(mins / 60);
  if (hours < 24) return `${hours} 小时前`;
  const days = Math.floor(hours / 24);
  return `${days} 天前`;
}

export function platformLabel(p: string): string {
  if (p === "trae") return "TRAE";
  if (p === "codebuddy_cn") return "CodeBuddy CN";
  return "WorkBuddy";
}

// 签到日边界：本地时间每天 06:00（与后端 checked_in_today 一致）
export function checkinDayStart(): number {
  const now = new Date();
  return new Date(now.getFullYear(), now.getMonth(), now.getDate(), 6).getTime() / 1000;
}

// 状态章取值（与 Chip 组件五变体对应）
export type ChipTone = "ok" | "run" | "warn" | "bad" | "idle";

/** 签到状态 → 状态章（success 跨天保留，仅今日 6 点后算「今日已签到」） */
export function checkinChipInfo(acc: {
  checkin_status: string;
  fail_count: number;
  last_checkin_time: number;
}): { tone: ChipTone; label: string } {
  const boundary = checkinDayStart();
  switch (acc.checkin_status) {
    case "success":
      return acc.last_checkin_time >= boundary
        ? { tone: "ok", label: "已签到" }
        : { tone: "idle", label: "未签到" };
    case "failed":
      return { tone: "bad", label: `失败 ${acc.fail_count} 次` };
    case "cooling":
      return { tone: "warn", label: "冷却中" };
    default:
      return { tone: "idle", label: "未签到" };
  }
}
