import { useEffect, useState } from "react";
import { Dialog } from "@/components/Dialog/Dialog";
import { Button } from "@/components/Button/Button";
import "./ConfirmDialog.css";

interface Options {
  title?: string;
  message: string;
  confirmText?: string;
  danger?: boolean;
  /** 可选的勾选提示（如删除账户时「保留档案备份」）；不传则不渲染勾选框 */
  checkLabel?: string;
  checkDefault?: boolean;
}

interface State extends Options {
  resolve: (ok: boolean, checked: boolean) => void;
}

let openConfirm: ((s: State) => void) | null = null;

/** 应用内确认弹窗（替代原生 confirm）。用法：if (await confirmDialog({ message: "..." })) {...} */
export function confirmDialog(opts: Options): Promise<boolean> {
  return new Promise((resolve) => {
    openConfirm?.({ ...opts, resolve: (ok) => resolve(ok) });
  });
}

/** 带勾选框的确认弹窗，返回 [是否确认, 是否勾选]。 */
export function confirmDialogChecked(
  opts: Options,
): Promise<[boolean, boolean]> {
  return new Promise((resolve) => {
    openConfirm?.({
      ...opts,
      resolve: (ok, checked) => resolve([ok, checked]),
    });
  });
}

/** 常驻宿主：App 挂载一次即可，全局 Promise 式调用 */
export function ConfirmHost() {
  const [state, setState] = useState<State | null>(null);
  const [checked, setChecked] = useState(false);
  // 在 effect 中注册全局入口（渲染期直接赋值会被 react-hooks/purity 拦截）
  useEffect(() => {
    openConfirm = (s) => {
      setChecked(s.checkDefault ?? false);
      setState(s);
    };
    return () => {
      openConfirm = null;
    };
  }, []);

  const close = (ok: boolean) => {
    state?.resolve(ok, checked);
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
      {state?.checkLabel && (
        <label className="cfrm-check">
          <input
            type="checkbox"
            checked={checked}
            onChange={(e) => setChecked(e.target.checked)}
          />
          {state.checkLabel}
        </label>
      )}
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
