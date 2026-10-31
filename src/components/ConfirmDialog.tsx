import { useState, type ReactNode } from "react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";

interface Options {
  title?: string;
  message: string;
  confirmText?: string;
  danger?: boolean;
}

interface State extends Options {
  resolve: (ok: boolean) => void;
}

let openConfirm: ((s: State) => void) | null = null;

/** 应用内确认弹窗（替代原生 confirm）。用法：if (await confirmDialog({ message: "..." })) {...} */
export function confirmDialog(opts: Options): Promise<boolean> {
  return new Promise((resolve) => {
    openConfirm?.({ ...opts, resolve });
  });
}

export function ConfirmHost() {
  const [state, setState] = useState<State | null>(null);
  openConfirm = setState;

  const close = (ok: boolean) => {
    state?.resolve(ok);
    setState(null);
  };

  return (
    <Dialog open={!!state} onOpenChange={(open) => !open && close(false)}>
      <DialogContent className="max-w-sm">
        <DialogHeader>
          <DialogTitle>{state?.title || "确认操作"}</DialogTitle>
        </DialogHeader>
        <p className="text-sm text-muted-foreground whitespace-pre-line">
          {state?.message as ReactNode}
        </p>
        <div className="flex justify-end gap-2 pt-2">
          <Button variant="outline" size="sm" onClick={() => close(false)}>
            取消
          </Button>
          <Button
            variant={state?.danger ? "destructive" : "default"}
            size="sm"
            onClick={() => close(true)}
          >
            {state?.confirmText || "确定"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}
