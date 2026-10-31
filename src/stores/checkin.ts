import { create } from "zustand";
import { api, type CheckinSettings } from "@/lib/api";

/**
 * 定时签到设置：跨页共享状态（Scheduler 写 / Dashboard 读）。
 * store 即已提交到后端的设置，保存成功后所有订阅页自动刷新。
 */
interface CheckinStore {
  settings: CheckinSettings | null;
  /** 从后端拉取设置（未加载时由订阅方调用） */
  load: () => Promise<void>;
  /** 本地编辑草稿（未保存，不落库） */
  update: (patch: Partial<CheckinSettings>) => void;
  /** 持久化当前设置到后端 */
  save: () => Promise<void>;
}

export const useCheckinStore = create<CheckinStore>((set, get) => ({
  settings: null,
  load: async () => {
    set({ settings: await api.getCheckinSettings() });
  },
  update: (patch) =>
    set((s) => (s.settings ? { settings: { ...s.settings, ...patch } } : s)),
  save: async () => {
    const { settings } = get();
    if (settings) await api.setCheckinSettings(settings);
  },
}));
