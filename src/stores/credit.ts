import { create } from "zustand";
import { api, type CreditScheduleSettings, type CreditScheduleState } from "@/lib/api";

/**
 * 定时积分查询设置：跨页共享状态。
 * 定时任务页的开关/间隔/平台范围修改后立即落库，并同步本地状态。
 */
interface CreditScheduleStore {
  data: CreditScheduleState | null;
  /** 从后端拉取配置与最近一次执行摘要 */
  load: () => Promise<void>;
  /** 本地即时同步（不落库） */
  update: (patch: Partial<CreditScheduleSettings>) => void;
  /** 持久化当前配置，并回读最新执行摘要 */
  save: () => Promise<void>;
}

export const useCreditScheduleStore = create<CreditScheduleStore>((set, get) => ({
  data: null,
  load: async () => {
    set({ data: await api.getCreditSchedule() });
  },
  update: (patch) =>
    set((s) =>
      s.data
        ? { data: { ...s.data, settings: { ...s.data.settings, ...patch } } }
        : s
    ),
  save: async () => {
    const { data } = get();
    if (!data) return;
    await api.setCreditSchedule(data.settings);
    // 回读：后端可能已按新间隔触发一次查询，同步最新执行摘要
    set({ data: await api.getCreditSchedule() });
  },
}));
