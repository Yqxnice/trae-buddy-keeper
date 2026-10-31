import { useEffect, useRef, useState } from "react";
import { MoreHorizontal } from "lucide-react";
import "./RowMenu.css";

export interface MenuItem {
  label?: string;
  /** 分隔线 */
  sep?: boolean;
  title?: string;
  danger?: boolean;
  disabled?: boolean;
  onClick?: () => void;
}

/** 行内溢出菜单：主动词之外的次级操作收纳到「···」浮层（点击外部 / Esc 关闭） */
export function RowMenu({ items, label = "更多操作" }: { items: MenuItem[]; label?: string }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    window.addEventListener("pointerdown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  return (
    <span className="rmenu" ref={ref}>
      <button
        className={`ract rmenu-trigger${open ? " on" : ""}`}
        title={label}
        aria-label={label}
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        <MoreHorizontal size={15} strokeWidth={2} />
      </button>
      {open && (
        <div className="rmenu-panel" role="menu">
          {items.map((it, i) =>
            it.sep ? (
              <div className="rmenu-sep" key={i} />
            ) : (
              <button
                key={i}
                role="menuitem"
                className={`rmenu-item${it.danger ? " danger" : ""}`}
                title={it.title ?? it.label}
                disabled={it.disabled}
                onClick={() => {
                  setOpen(false);
                  it.onClick?.();
                }}
              >
                {it.label}
              </button>
            )
          )}
        </div>
      )}
    </span>
  );
}
