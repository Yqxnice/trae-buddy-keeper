import * as React from "react";
import { CheckCircle2, XCircle, Info, X, AlertTriangle } from "lucide-react";
import { cn } from "@/lib/utils";

export type ToastType = "success" | "error" | "info" | "warning";

export interface ToastItem {
  id: string;
  title: string;
  body: string;
  type: ToastType;
}

export function ToastContainer({
  toasts,
  onDismiss,
}: {
  toasts: ToastItem[];
  onDismiss: (id: string) => void;
}) {
  return (
    <div className="fixed top-4 right-4 z-[100] flex flex-col gap-2 w-80">
      {toasts.map((t) => (
        <Toast key={t.id} toast={t} onDismiss={onDismiss} />
      ))}
    </div>
  );
}

function Toast({ toast, onDismiss }: { toast: ToastItem; onDismiss: (id: string) => void }) {
  React.useEffect(() => {
    const timer = setTimeout(() => onDismiss(toast.id), 4000);
    return () => clearTimeout(timer);
  }, [toast.id, onDismiss]);

  const Icon =
    toast.type === "success"
      ? CheckCircle2
      : toast.type === "error"
      ? XCircle
      : toast.type === "warning"
      ? AlertTriangle
      : Info;
  const color =
    toast.type === "success"
      ? "text-green-500"
      : toast.type === "error"
      ? "text-red-500"
      : toast.type === "warning"
      ? "text-yellow-500"
      : "text-blue-500";

  return (
    <div className="flex items-start gap-3 rounded-lg border bg-background p-4 shadow-lg">
      <Icon className={cn("h-5 w-5 shrink-0 mt-0.5", color)} />
      <div className="flex-1 min-w-0">
        <p className="text-sm font-medium">{toast.title}</p>
        <p className="text-xs text-muted-foreground mt-0.5 break-words">{toast.body}</p>
      </div>
      <button
        onClick={() => onDismiss(toast.id)}
        className="shrink-0 opacity-50 hover:opacity-100"
      >
        <X className="h-4 w-4" />
      </button>
    </div>
  );
}
