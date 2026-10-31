import { useEffect, useRef } from "react";
import "./SelectorBar.css";

interface SelectorBarProps {
  options: { value: string; label: string }[];
  value: string;
  onChange: (v: string) => void;
}

/** 标题栏产品切换：Windows 惯用法下划线 SelectorBar，指示条 220ms 滑移 */
export function SelectorBar({ options, value, onChange }: SelectorBarProps) {
  const indRef = useRef<HTMLSpanElement>(null);

  // 挂载 / 切换 / 尺寸变化时，把指示条滑到当前项下方
  useEffect(() => {
    const ind = indRef.current;
    if (!ind) return;
    const move = () => {
      const cur = ind.parentElement?.querySelector<HTMLElement>(".tb-tab.on");
      if (!cur) return;
      ind.style.left = `${cur.offsetLeft}px`;
      ind.style.width = `${cur.offsetWidth}px`;
    };
    move();
    window.addEventListener("resize", move);
    // 字体加载完成后可能改变宽度，补一次校准
    const t = setTimeout(move, 200);
    return () => {
      window.removeEventListener("resize", move);
      clearTimeout(t);
    };
  }, [value, options.length]);

  return (
    <div className="tb-tabs" role="tablist">
      {options.map((o) => (
        <button
          key={o.value}
          role="tab"
          aria-selected={value === o.value}
          className={`tb-tab${value === o.value ? " on" : ""}`}
          onClick={() => onChange(o.value)}
        >
          {o.label}
        </button>
      ))}
      <span className="tb-ind" ref={indRef} />
    </div>
  );
}
