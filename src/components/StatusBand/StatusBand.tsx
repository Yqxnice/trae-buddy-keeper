import type { ReactNode } from "react";
import "./StatusBand.css";

/** 今日状态带：指挥甲板第一对象，品牌描边锁定视线 */
export function StatusBand({ children }: { children: ReactNode }) {
  return <div className="band">{children}</div>;
}

/** 状态带大数字项 */
export function BandBig({
  label,
  value,
  unit,
  sub,
}: {
  label: string;
  value: ReactNode;
  /** 跟在大数字后的小字（如 / 总数） */
  sub?: ReactNode;
  /** 小单位（如 积分） */
  unit?: string;
}) {
  return (
    <div>
      <div className="bl">{label}</div>
      <div className="bv">
        {value}
        {sub && <span className="bv-sub">{sub}</span>}
        {unit && <span className="bv-unit">{unit}</span>}
      </div>
    </div>
  );
}

/** 状态带次级数字项 */
export function BandItem({
  label,
  children,
}: {
  label: string;
  children: ReactNode;
}) {
  return (
    <div>
      <div className="bl">{label}</div>
      <div className="bv2">{children}</div>
    </div>
  );
}
