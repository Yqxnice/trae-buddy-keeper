import { useEffect, useState } from "react";
import { RefreshCw, Save, RotateCcw } from "lucide-react";
import { PageHeader } from "@/components/PageHeader/PageHeader";
import { Button } from "@/components/Button/Button";
import { Switch } from "@/components/Switch/Switch";
import { NumInput } from "@/components/TextInput/TextInput";
import { useCheckinStore } from "@/stores/checkin";
import { useUIStore } from "@/stores/ui";
import { platformLabel } from "@/lib/utils";
import "./Scheduler.css";

export function SchedulerPage() {
  const pushToast = useUIStore((s) => s.pushToast);
  const settings = useCheckinStore((s) => s.settings);
  const load = useCheckinStore((s) => s.load);
  const update = useCheckinStore((s) => s.update);
  const save = useCheckinStore((s) => s.save);
  const [loading, setLoading] = useState(true);

  // store 在 App 启动时已加载；此页兜底拉取，保证直接进入本页也有数据
  useEffect(() => {
    (settings ? Promise.resolve() : load())
      .catch((e: any) => pushToast("加载失败", e?.toString(), "error"))
      .finally(() => setLoading(false));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleSave = async () => {
    if (!settings) return;
    try {
      await save();
      pushToast("已保存", "定时任务设置已更新", "success");
    } catch (e: any) {
      pushToast("保存失败", e?.toString(), "error");
    }
  };

  if (loading || !settings) {
    return (
      <div className="page-loading">
        <RefreshCw className="spin" size={26} strokeWidth={1.6} />
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
