import { useEffect, useState } from "react";
import { RefreshCw } from "lucide-react";
import { PageHeader } from "@/components/PageHeader/PageHeader";
import { StatusBand, BandBig, BandItem } from "@/components/StatusBand/StatusBand";
import { LedgerRow } from "@/components/LedgerRow/LedgerRow";
import { Avatar } from "@/components/Avatar/Avatar";
import { Chip } from "@/components/Chip/Chip";
import { Button } from "@/components/Button/Button";
import { api, type PointsDashboard, type AccountInfo } from "@/lib/api";
import { useCheckinStore } from "@/stores/checkin";
import { useUIStore } from "@/stores/ui";
import { formatDate, formatRelative, platformLabel, checkinChipInfo } from "@/lib/utils";
import "./Dashboard.css";

export function Dashboard() {
  const platform = useUIStore((s) => s.platform);
  const pushToast = useUIStore((s) => s.pushToast);
  const [data, setData] = useState<PointsDashboard | null>(null);
  const [accounts, setAccounts] = useState<AccountInfo[]>([]);
  // 定时签到设置：来自共享 store（Scheduler 保存后此处自动刷新）
  const sched = useCheckinStore((s) => s.settings);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);

  // 全屏 loading 仅首次挂载；手动刷新原地更新数据并 toast 结果
  const load = async (notify = false) => {
    if (notify) setRefreshing(true);
    try {
      const [d, list] = await Promise.all([
        api.getPointsDashboard(platform),
        api.listAccounts(),
      ]);
      setData(d);
      setAccounts(list);
      if (notify) pushToast("刷新成功", "数据已更新", "success");
    } catch (e: any) {
      pushToast(notify ? "刷新失败" : "加载失败", e?.toString(), "error");
    } finally {
      setLoading(false);
      if (notify) setRefreshing(false);
    }
  };

  useEffect(() => {
    load();
  }, [platform]);

  if (loading || !data) {
    return (
      <div className="page-loading">
        <RefreshCw className="spin" size={26} strokeWidth={1.6} />
      </div>
    );
  }

  const mine = accounts.filter((a) => a.platform === platform);
  const boundary = new Date();
  boundary.setHours(6, 0, 0, 0);
  const boundaryTs = boundary.getTime() / 1000;
  const done = mine.filter((a) => a.last_checkin_time >= boundaryTs).length;

  // 总余额：当前产品所有账号余额求和（未查询的按 0 计）
  const balance = mine.reduce((s, a) => s + (a.credits_balance ?? 0), 0);

  // 积分缓存：最近一次成功查询时间
  const latestFetch = mine.reduce((m, a) => Math.max(m, a.credits_fetched_at || 0), 0);

  // 下次自动执行：定时签到开启时显示执行时刻
  const nextRun =
    sched && sched.enabled
      ? `${String(sched.hour).padStart(2, "0")}:${String(sched.minute).padStart(2, "0")}`
      : "未启用";

  // 7 日趋势最大值，供微柱归一
  const trendMax = Math.max(...data.trend.map((t) => t.points), 1);

  return (
    <div className="dash">
      <PageHeader
        title="总览"
        sub={`${platformLabel(platform)} · 今日签到与积分概况`}
        right={
          <Button onClick={() => load(true)} disabled={refreshing}>
            <RefreshCw className={refreshing ? "spin" : ""} />
            刷新
          </Button>
        }
      />

      {/* 指挥甲板：今日状态带 */}
      <StatusBand>
        <BandBig label="今日签到" value={done} sub={` / ${mine.length}`} />
        <BandItem label="总余额">
          {balance.toLocaleString()}
          <span className="unit">积分</span>
        </BandItem>
        <BandItem label="今日新增">
          +{data.today_new.toLocaleString()}
          <span className="unit">积分</span>
        </BandItem>
        <BandItem label="下次自动执行">{nextRun}</BandItem>
        <BandItem label="积分缓存">{latestFetch ? formatRelative(latestFetch) : "—"}</BandItem>
      </StatusBand>

      {/* 账本行（只读概览） */}
      <div className="rows">
        {mine.length === 0 ? (
          <div className="dash-empty">暂无{platformLabel(platform)}账号</div>
        ) : (
          mine.map((acc) => {
            const info = checkinChipInfo(acc);
            return (
              <LedgerRow
                key={acc.id}
                avatar={<Avatar name={acc.name} src={acc.avatar} />}
                name={acc.name || "未命名"}
                meta={acc.email || undefined}
                chip={<Chip tone={info.tone} dot>{info.label}</Chip>}
              >
                <span className="dash-note">
                  最后签到 {formatRelative(acc.last_checkin_time)} · Token 到期{" "}
                  {acc.token_expires_at ? formatDate(acc.token_expires_at) : "—"}
                </span>
                {acc.credits_balance != null && (
                  <span className="rval">{acc.credits_balance.toLocaleString()} 积分</span>
                )}
              </LedgerRow>
            );
          })
        )}
      </div>

      {/* 7 日趋势微柱 */}
      {data.trend.length > 0 && (
        <>
          <div className="gtitle">近 7 日积分趋势</div>
          <div className="trend-bars">
            {data.trend.map((t) => (
              <div key={t.date} className="trend-col" title={`${t.date} · ${t.points} 积分`}>
                <i style={{ height: `${Math.max(6, (t.points / trendMax) * 100)}%` }} />
                <span>{t.date.slice(5)}</span>
              </div>
            ))}
          </div>
        </>
      )}

      {/* 到期提醒 */}
      {data.expiring_packages.length > 0 && (
        <>
          <div className="gtitle">积分包到期提醒（7 日内）</div>
          <div className="rows">
            {data.expiring_packages.map((p, i) => (
              <div className="row" key={i}>
                <span className="rname">{p.account_name}</span>
                <div className="rgrow">
                  <span className="dash-note">{p.points.toLocaleString()} 积分</span>
                  <span className="rval">{formatDate(p.expires_at)}</span>
                </div>
                <Chip tone={p.days_left <= 1 ? "bad" : "warn"}>
                  {p.days_left <= 0 ? "今日到期" : `${p.days_left} 天后到期`}
                </Chip>
              </div>
            ))}
          </div>
        </>
      )}

      {/* 静默信息条 */}
      <div className="quiet">
        <span>
          {sched?.enabled
            ? `定时任务已启用 · 每日 ${nextRun} 自动签到 · 失败账号进入 ${sched.cooldown_minutes} 分钟冷却后自动补签`
            : "定时任务未启用 · 可在「定时任务」页开启每日自动签到"}
        </span>
      </div>
    </div>
  );
}
