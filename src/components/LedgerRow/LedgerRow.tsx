import type { ReactNode } from "react";
import "./LedgerRow.css";

interface LedgerRowProps {
  /** 左缘头像区（Avatar 组件） */
  avatar: ReactNode;
  /** 主名称 */
  name: string;
  /** 等宽小字（设备号 / 邮箱等） */
  meta?: string;
  /** 中部弹性区：文字信息 / 余量条 / 数值 */
  children?: ReactNode;
  /** 右缘状态章（Chip 组件） */
  chip?: ReactNode;
  /** 中部附加区（双行卡片：紧跟状态章之后，填充首行空档，如客户端切换器） */
  extra?: ReactNode;
  /** 右缘数值列（双行卡片用：余量 / 指标等，右对齐） */
  aside?: ReactNode;
  /** 行内操作区（ract 小按钮组） */
  actions?: ReactNode;
  /** 双行卡片：首行头像+名称/邮箱+状态章+数值列+操作，次行备注（children） */
  stacked?: boolean;
}

/** 账本行：状态章 + 弹性中区 + 行内操作，无左缘色条 */
export function LedgerRow({ avatar, name, meta, children, chip, extra, aside, actions, stacked }: LedgerRowProps) {
  if (stacked) {
    return (
      <div className="row row-stacked">
        <div className="rowline">
          {avatar}
          <div className="rid">
            <span className="rname">{name}</span>
            {meta && <span className="rdev">{meta}</span>}
          </div>
          {chip && <span className="rchip">{chip}</span>}
          {extra}
          {aside && <span className="raside">{aside}</span>}
          {actions && <span className="racts">{actions}</span>}
        </div>
        {children && <div className="rsub">{children}</div>}
      </div>
    );
  }
  return (
    <div className="row">
      {avatar}
      <span className="rname">{name}</span>
      {meta && <span className="rdev">{meta}</span>}
      {children && <div className="rgrow">{children}</div>}
      {chip}
      {actions && <span className="racts">{actions}</span>}
    </div>
  );
}

/** 行内小文字按钮（账本行操作区用） */
export function RowAction({
  label,
  title,
  danger = false,
  disabled = false,
  onClick,
}: {
  label: string;
  title?: string;
  danger?: boolean;
  disabled?: boolean;
  onClick?: () => void;
}) {
  return (
    <button
      className={`ract${danger ? " danger" : ""}`}
      title={title ?? label}
      disabled={disabled}
      onClick={onClick}
    >
      {label}
    </button>
  );
}
