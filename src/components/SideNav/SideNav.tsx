import type { ComponentType, ReactNode } from "react";
import "./SideNav.css";

export interface NavItem {
  id: string;
  label: string;
  icon: ComponentType<{ size?: number | string; strokeWidth?: number | string }>;
  /** 右缘状态微点颜色（状态脊柱：切页前先知情） */
  dot?: "warn" | "brand" | null;
}

/** 状态脊柱侧边栏：主导航 + 底部系统区（定时任务 / 日志 / 设置） */
export function SideNav({
  items,
  footItems,
  value,
  onChange,
}: {
  items: NavItem[];
  footItems: NavItem[];
  value: string;
  onChange: (id: string) => void;
}) {
  const render = (list: NavItem[]) =>
    list.map((item) => (
      <button
        key={item.id}
        className={`nav-i${value === item.id ? " on" : ""}`}
        onClick={() => onChange(item.id)}
      >
        <item.icon size={15} strokeWidth={1.7} />
        <span className="nlabel">{item.label}</span>
        {item.dot && <i className={`ndot ${item.dot}`} />}
      </button>
    ));

  return (
    <aside className="rail">
      <nav className="rail-main">{render(items)}</nav>
      <div className="rail-foot">{render(footItems)}</div>
    </aside>
  );
}

export type { ReactNode };
