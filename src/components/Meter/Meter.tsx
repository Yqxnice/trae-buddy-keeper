import "./Meter.css";

/** 余量条：scaleX 生长动效，值域 0-100；small 用于积分包 / 分布等紧凑场景 */
export function Meter({ value, small = false }: { value: number; small?: boolean }) {
  const pct = Math.max(0, Math.min(100, Math.round(value)));
  return (
    <div
      className={`meter${small ? " sm" : ""}`}
      role="progressbar"
      aria-valuenow={pct}
      aria-valuemin={0}
      aria-valuemax={100}
    >
      <i style={{ width: `${pct}%` }} />
    </div>
  );
}
