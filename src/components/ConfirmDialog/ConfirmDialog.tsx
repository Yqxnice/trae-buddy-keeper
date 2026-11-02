import { useEffect, useState } from "react";
import { Dialog } from "@/components/Dialog/Dialog";
import { Button } from "@/components/Button/Button";
import "./ConfirmDialog.css";

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

/** 常驻宿主：App 挂载一次即可，全局 Promise 式调用 */
export function ConfirmHost() {
  const [state, setState] = useState<State | null>(null);
  // 在 effect 中注册全局入口（渲染期直接赋值会被 react-hooks/purity 拦截）
  useEffect(() => {
    openConfirm = setState;
    return () => {
      openConfirm = null;
    };
  }, []);

  const close = (ok: boolean) => {
    state?.resolve(ok);
    setState(null);
  };

  return (
    <Dialog
      open={!!state}
      onClose={() => close(false)}
      title={state?.title || "确认操作"}
      width={380}
    >
      <p className="cfrm-msg">{state?.message}</p>
      <div className="cfrm-acts">
        <Button onClick={() => close(false)}>取消</Button>
        <Button
          variant={state?.danger ? "danger" : "pri"}
          onClick={() => close(true)}
        >
          {state?.confirmText || "确定"}
        </Button>
      </div>
    </Dialog>
  );
}
