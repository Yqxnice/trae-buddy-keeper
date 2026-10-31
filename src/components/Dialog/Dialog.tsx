import { useEffect, type ReactNode } from "react";
import { X } from "lucide-react";
import "./Dialog.css";

interface DialogProps {
  open: boolean;
  onClose: () => void;
  title: string;
  description?: string;
  /** 宽度上限，默认 460px */
  width?: number;
  children: ReactNode;
}

/** 应用内弹窗骨架：遮罩点击与 Esc 均触发 onClose */
export function Dialog({ open, onClose, title, description, width = 460, children }: DialogProps) {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;
  return (
    <div className="dlg-mask" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dlg" style={{ maxWidth: width }} role="dialog" aria-modal="true" aria-label={title}>
        <div className="dlg-head">
          <div>
            <div className="dlg-title">{title}</div>
            {description && <div className="dlg-desc">{description}</div>}
          </div>
          <button className="dlg-x" onClick={onClose} title="关闭">
            <X size={15} />
          </button>
        </div>
        <div className="dlg-body">{children}</div>
      </div>
    </div>
  );
}
