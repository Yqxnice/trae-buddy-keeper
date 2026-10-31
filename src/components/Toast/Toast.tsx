import { useEffect } from "react";
import { CheckCircle2, XCircle, Info, X, AlertTriangle } from "lucide-react";
import { useUIStore } from "@/stores/ui";
import "./Toast.css";

export type ToastType = "success" | "error" | "info" | "warning";

export interface ToastItem {
  id: string;
  title: string;
  body: string;
  type: ToastType;
}

const ICONS: Record<ToastType, typeof Info> = {
  success: CheckCircle2,
  error: XCircle,
  warning: AlertTriangle,
  info: Info,
};

/** 右上角通知堆栈：4 秒自动消退，可手动关闭；通知数据来自全局 UI store */
export function ToastContainer() {
  const toasts = useUIStore((s) => s.toasts);
  return (
    <div className="toast-stack">
      {toasts.map((t) => (
        <ToastCard key={t.id} toast={t} />
      ))}
    </div>
  );
}

function ToastCard({ toast }: { toast: ToastItem }) {
  const dismissToast = useUIStore((s) => s.dismissToast);

  useEffect(() => {
    const timer = setTimeout(() => dismissToast(toast.id), 4000);
    return () => clearTimeout(timer);
  }, [toast.id, dismissToast]);

  const Icon = ICONS[toast.type] ?? Info;
  return (
    <div className={`toast t-${toast.type}`}>
      <Icon className="t-ico" size={17} />
      <div className="t-main">
        <p className="t-title">{toast.title}</p>
        <p className="t-body">{toast.body}</p>
      </div>
      <button className="t-x" onClick={() => dismissToast(toast.id)}>
        <X size={13} />
      </button>
    </div>
  );
}
