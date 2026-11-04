import { useEffect, useState } from "react";
import { RefreshCw, Save, RotateCcw } from "lucide-react";
import { PageHeader } from "@/components/PageHeader/PageHeader";
import { Button } from "@/components/Button/Button";
import { Switch } from "@/components/Switch/Switch";
import { Select } from "@/components/Select/Select";
import { NumInput } from "@/components/TextInput/TextInput";
import { useCheckinStore } from "@/stores/checkin";
import { useCreditScheduleStore } from "@/stores/credit";
import { useUIStore } from "@/stores/ui";
import { formatRelative, platformLabel } from "@/lib/utils";
import type { CreditScheduleSettings } from "@/lib/api";
import "./Scheduler.css";

/** 积分查询常用间隔档位（分钟） */
const CREDIT_INTERVAL_OPTIONS = [
  { value: "30", label: "30 分钟" },
  { value: "60", label: "1 小时" },
  { value: "180", label: "3 小时" },
  { value: "360", label: "6 小时" },
  { value: "720", label: "12 小时" },
  { value: "1440", label: "24 小时" },
];

/** 积分查询可选的平台范围（WorkBuddy 范围含 CodeBuddy CN） */
const CREDIT_SCOPE_KEYS = ["trae", "workbuddy"] as const;

export function SchedulerPage() {
  const pushToast = useUIStore((s) => s.pushToast);
  const settings = useCheckinStore((s) => s.settings);
  const load = useCheckinStore((s) => s.load);
  const update = useCheckinStore((s) => s.update);
  const save = useCheckinStore((s) => s.save);
  const credit = useCreditScheduleStore((s) => s.data);
  const loadCredit = useCreditScheduleStore((s) => s.load);
  const updateCredit = useCreditScheduleStore((s) => s.update);
  const saveCredit = useCreditScheduleStore((s) => s.save);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);

  // store 在 App 启动时已加载；此页兜底拉取，保证直接进入本页也有数据
  const reload = async () => {
    setLoading(true);
    setLoadError(null);
    try {
      await Promise.all([
        settings ? Promise.resolve() : load(),
        credit ? Promise.resolve() : loadCredit(),
      ]);
    } catch (e: any) {
      const msg = e?.toString() || "未知错误";
      setLoadError(msg);
      pushToast("加载失败", msg, "error");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 后台定时任务在页面打开期间可能执行：定期回读最近一次执行摘要
  useEffect(() => {
    const timer = setInterval(() => {
      loadCredit().catch(() => {});
    }, 60_000);
    return () => clearInterval(timer);
  }, [loadCredit]);

  // 修改即保存（与本项目其余设置项一致，不额外加保存按钮）
  const patchCredit = async (patch: Partial<CreditScheduleSettings>) => {
    updateCredit(patch);
    try {
      await saveCredit();
    } catch (e: any) {
      pushToast("保存失败", e?.toString(), "error");
    }
  };

  const handleSave = async () => {
    if (!settings) return;
    try {
      await save();
      pushToast("已保存", "定时任务设置已更新", "success");
    } catch (e: any) {
      pushToast("保存失败", e?.toString(), "error");
    }
  };

  if (loading) {
    return (
      <div className="page-loading">
        <RefreshCw className="spin" size={26} strokeWidth={1.6} />
      </div>
    );
  }

  // 加载失败：给出明确提示与重试入口，不要一直停在骨架屏
  if (!settings || !credit) {
    return (
      <div className="sched">
        <PageHeader
          title="定时任务"
          sub="每日自动签到与失败补签 · 托盘「立即签到」不受影响"
        />
        <div className="sched-card">
          <div className="sched-grid">
            <span className="sched-hint">
              定时任务配置加载失败{loadError ? `：${loadError}` : "，请重试"}
            </span>
            <Button onClick={() => void reload()}>重新加载</Button>
          </div>
        </div>
      </div>
    );
  }

  // 下次执行时刻：今天已过执行点则顺延到明天
  const hh = String(settings.hour).padStart(2, "0");
  const mm = String(settings.minute).padStart(2, "0");
  const now = new Date();
  const next = new Date(now.getFullYear(), now.getMonth(), now.getDate(), settings.hour, settings.minute);
  if (next <= now) next.setDate(next.getDate() + 1);
  const nextLabel = settings.enabled
    ? `${next.getMonth() + 1}/${next.getDate()} ${hh}:${mm}`
    : "未启用";

  // 平台范围：空数组 = 全部平台（与后端语义一致）
  const scopeLabel =
    settings.platforms.length === 0
      ? "全部产品"
      : settings.platforms.map(platformLabel).join(" / ");

  // 积分查询平台范围：空数组同样表示全部平台，UI 上按全选展示
  const creditScope: string[] =
    credit.settings.platforms.length > 0
      ? credit.settings.platforms
      : [...CREDIT_SCOPE_KEYS];
  const toggleCreditPlatform = (key: string, on: boolean) => {
    if (!on && creditScope.length <= 1) {
      pushToast("无法关闭", "至少保留一个平台参与定时积分查询", "warning");
      return;
    }
    const next = on
      ? Array.from(new Set([...creditScope, key]))
      : creditScope.filter((p) => p !== key);
    patchCredit({ platforms: next });
  };
  const lastRunLabel = credit.last_run.at
    ? formatRelative(credit.last_run.at)
    : "尚未执行";

  return (
    <div className="sched">
      <PageHeader
        title="定时任务"
        sub="每日自动签到与失败补签 · 托盘「立即签到」不受影响"
      />

      {/* 主卡：开关 + 执行时间 + 下次执行 */}
      <div className="sched-card">
        <div className="sched-grid">
          <div className="sched-ctl">
            <Switch
              checked={settings.enabled}
              onChange={(v) => update({ enabled: v })}
              label="启用定时签到"
            />
            <div>
              <div className="st-name">每日自动签到</div>
              <div className="st-desc">关闭后仅可手动或托盘触发</div>
            </div>
          </div>
          <span className="divider-v" />
          <div>
            <div className="bl">执行时间</div>
            <div className="numrow">
              <NumInput
                min={0}
                max={23}
                value={settings.hour}
                onChange={(e) => update({ hour: Number(e.target.value) })}
              />
              <span className="st-colon">:</span>
              <NumInput
                min={0}
                max={59}
                value={settings.minute}
                onChange={(e) => update({ minute: Number(e.target.value) })}
              />
            </div>
          </div>
          <span className="divider-v" />
          <div>
            <div className="bl">平台范围</div>
            <div className="bv2">{scopeLabel}</div>
          </div>
          <span className="divider-v" />
          <div>
            <div className="bl">下次执行</div>
            <div className="bv2">{nextLabel}</div>
          </div>
        </div>
      </div>

      {/* 定时积分查询：与签到任务并列、互不影响 */}
      <div className="gtitle">定时积分查询</div>
      <div className="sched-card">
        <div className="sched-grid">
          <div className="sched-ctl">
            <Switch
              checked={credit.settings.enabled}
              onChange={(v) => patchCredit({ enabled: v })}
              label="启用定时积分查询"
            />
            <div>
              <div className="st-name">自动查询积分</div>
              <div className="st-desc">按间隔直连官方接口刷新全部账号</div>
            </div>
          </div>
          <span className="divider-v" />
          <div>
            <div className="bl">查询间隔</div>
            <Select
              value={String(credit.settings.interval_minutes)}
              onChange={(v) => patchCredit({ interval_minutes: Number(v) })}
              options={CREDIT_INTERVAL_OPTIONS}
            />
          </div>
          <span className="divider-v" />
          <div>
            <div className="bl">平台范围</div>
            <div className="sched-scope">
              {CREDIT_SCOPE_KEYS.map((key) => (
                <span className="scope-item" key={key}>
                  <span className="scope-name">{platformLabel(key)}</span>
                  <Switch
                    checked={creditScope.includes(key)}
                    onChange={(v) => toggleCreditPlatform(key, v)}
                    label={platformLabel(key)}
                  />
                </span>
              ))}
            </div>
          </div>
          <span className="divider-v" />
          <div>
            <div className="bl">最近执行</div>
            <div className="bv2">{lastRunLabel}</div>
            <div className="st-desc">
              {credit.last_run.at
                ? credit.last_run.message
                : "启用后按间隔自动查询"}
            </div>
          </div>
        </div>
        <p className="sched-hint">
          定时查询强制刷新真实积分（不走页面 60 秒本地缓存），结果写回现有缓存；与定时签到互不影响，同一时刻只允许一个批量任务执行。
        </p>
      </div>

      {/* 失败处理参数 */}
      <div className="gtitle">失败处理</div>
      <div className="sched-card">
        <div className="sched-grid">
          <div>
            <div className="bl">签到间隔（秒）</div>
            <NumInput
              min={1}
              value={settings.interval_seconds}
              onChange={(e) => update({ interval_seconds: Number(e.target.value) })}
            />
          </div>
          <span className="divider-v" />
          <div>
            <div className="bl">最大失败次数</div>
            <NumInput
              min={1}
              value={settings.max_fail_count}
              onChange={(e) => update({ max_fail_count: Number(e.target.value) })}
            />
          </div>
          <span className="divider-v" />
          <div>
            <div className="bl">冷却时长（分）</div>
            <NumInput
              min={1}
              value={settings.cooldown_minutes}
              onChange={(e) => update({ cooldown_minutes: Number(e.target.value) })}
            />
          </div>
          <span className="spacer" />
          <Button variant="pri" onClick={handleSave}>
            <Save /> 保存设置
          </Button>
        </div>
        <p className="sched-hint">
          失败达到最大次数后，账户进入冷却，冷却期间跳过签到。当前签到范围：{scopeLabel}
          ，可在「设置 → 定时签到」调整。
        </p>
      </div>

      {/* 随应用运行自动执行的任务 */}
      <div className="gtitle">自动巡检</div>
      <div className="sched-card">
        <div className="sched-item">
          <RotateCcw size={15} strokeWidth={1.7} className="sched-ico" />
          <div>
            <div className="st-name">超时补签</div>
            <div className="st-desc">
              每 10 分钟检查一次，对当日未签到且不在冷却、未失效的账号自动补签。
            </div>
          </div>
        </div>
        <div className="sched-item">
          <RefreshCw size={15} strokeWidth={1.7} className="sched-ico" />
          <div>
            <div className="st-name">自动续期巡检</div>
            <div className="st-desc">
              启动时立即执行一次，之后每 6 小时一轮；token 在 24
              小时内到期（含已过期）的账号自动续期。
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
