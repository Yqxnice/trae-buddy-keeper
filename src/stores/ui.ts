import { create } from "zustand";
import type { ToastItem, ToastType } from "@/components/Toast/Toast";

/**
 * 全局 UI 状态：当前产品（跨页共享）+ 通知堆栈（任意位置推送）。
 * 页面直接订阅 store，无需 App 逐层透传 props。
 */
interface UIStore {
  /** 当前产品（trae / workbuddy），Titlebar 写、各页面读 */
  platform: string;
  setPlatform: (p: string) => void;
  /** 右上角通知堆栈 */
  toasts: ToastItem[];
  pushToast: (title: string, body: string, type?: ToastType) => void;
  dismissToast: (id: string) => void;
}

export const useUIStore = create<UIStore>((set) => ({
  platform: "trae",
  setPlatform: (platform) => set({ platform }),
  toasts: [],
  pushToast: (title, body, type = "info") =>
    set((s) => ({
      toasts: [
        ...s.toasts,
        { id: Math.random().toString(36).slice(2), title, body, type },
      ],
    })),
  dismissToast: (id) =>
    set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),
}));
