import "./Chip.css";

export type ChipTone = "ok" | "run" | "warn" | "bad" | "idle";

interface ChipProps {
  tone: ChipTone;
  /** 是否显示前导状态点 */
  dot?: boolean;
  /** 呼吸动画：缺省时 run 态自动开启（执行中语义），其余不呼吸 */
  pulse?: boolean;
  children: React.ReactNode;
}

/** 状态章：账本行 / 日志 / 签到结果共用的圆角胶囊 */
export function Chip({ tone, dot = false, pulse, children }: ChipProps) {
  const willPulse = pulse ?? (tone === "run" && dot);
  return (
    <span className={`chip chip-${tone}`}>
      {dot && <i className={`dot${willPulse ? " pulse" : ""}`} />}
      {children}
    </span>
  );
}
