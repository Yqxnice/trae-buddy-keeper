import { useEffect, useMemo, useState } from "react";
import { Play, RefreshCw } from "lucide-react";
import { PageHeader } from "@/components/PageHeader/PageHeader";
import { Button } from "@/components/Button/Button";
import { Segmented } from "@/components/Segmented/Segmented";
import { Avatar } from "@/components/Avatar/Avatar";
import { Chip } from "@/components/Chip/Chip";
import { api, type AccountInfo, type CheckinResult } from "@/lib/api";
import { formatDate, platformLabel, checkinDayStart } from "@/lib/utils";
import { useUIStore } from "@/stores/ui";
import "./Checkin.css";

export function CheckinPage() {
  const platform = useUIStore((s) => s.platform);
  const pushToast = useUIStore((s) => s.pushToast);
  const [accounts, setAccounts] = useState<AccountInfo[]>([]);
  const [todayPoints, setTodayPoints] = useState<Record<string, number>>({});
  const [loading, setLoading] = useState(true);
  const [checkingId, setCheckingId] = useState<string | null>(null);
  const [running, setRunning] = useState(false);

  const [scope, setScope] = useState<"all" | "manual">("all");
  const [skipSigned, setSkipSigned] = useState(true);
  const [skipExpired, setSkipExpired] = useState(true);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [results, setResults] = useState<Record<string, CheckinResult>>({});

  // 全屏 loading 仅首次挂载；签到完成后原地更新，避免整页拆建抖动
  const load = async () => {
    try {
      const [accs, records] = await Promise.all([
        api.listAccounts(),
        api.getCheckinRecords(undefined, 200),
      ]);
      setAccounts(accs);
      // 今日各账号已得积分（按签到成功记录聚合）
      const boundary = checkinDayStart();
      const map: Record<string, number> = {};
      for (const r of records) {
        if (r.platform === platform && r.status === "success" && r.created_at >= boundary) {
          map[r.account_id] = (map[r.account_id] || 0) + r.points;
        }
      }
      setTodayPoints(map);
    } catch (e: any) {
      pushToast("加载失败", e?.toString(), "error");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    // 切产品只重置选中项：results 里可能还有正在跑或已完成的签到结果，不能清掉
    setSelected(new Set());
    load();
  }, [platform]);

  const filtered = accounts.filter((a) => a.platform === platform);
  const now = Date.now() / 1000;
  const boundary = checkinDayStart();
  const isSigned = (a: AccountInfo) => a.last_checkin_time >= boundary;
  const isExpired = (a: AccountInfo) =>
    a.token_expires_at > 0 && a.token_expires_at < now;

  // 参与签到的目标账号
  const targets = useMemo(() => {
    let list = scope === "all" ? filtered : filtered.filter((a) => selected.has(a.id));
    if (skipSigned) list = list.filter((a) => !isSigned(a));
    if (skipExpired) list = list.filter((a) => !isExpired(a));
    return list;
  }, [scope, selected, skipSigned, skipExpired, filtered]);

  const toggle = (id: string) => {
    setSelected((s) => {
      const next = new Set(s);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  };

  const run = async () => {
    if (running || targets.length === 0) return;
    setRunning(true);
    setResults({});
    let ok = 0;
    let skipped = 0;
    for (const acc of targets) {
      setCheckingId(acc.id);
      try {
        const r = await api.checkinOne(acc.id);
        setResults((prev) => ({ ...prev, [acc.id]: r }));
        if (r.skipped) skipped++;
        else if (r.success) ok++;
      } catch (e: any) {
        setResults((prev) => ({
          ...prev,
          [acc.id]: {
            account_id: acc.id,
            account_name: acc.name,
            platform,
            success: false,
            skipped: false,
            points: 0,
            message: e?.toString() || "签到异常",
          },
        }));
      }
    }
    setCheckingId(null);
    pushToast(
      "签到完成",
      `成功 ${ok}，跳过 ${skipped}，共 ${targets.length}`,
      ok === targets.length - skipped ? "success" : "error"
    );
    setRunning(false);
    load();
  };

  if (loading) {
    return (
      <div className="page-loading">
        <RefreshCw className="spin" size={26} strokeWidth={1.6} />
      </div>
    );
  }

  return (
    <div className="checkin">
      <PageHeader
        title="签到"
        sub={`${platformLabel(platform)} · 执行签到并查看逐账号结果`}
      />

      {/* 工具行：范围 + 跳过策略 + 开始 */}
      <div className="tools">
        <Segmented
          options={[
            { value: "all", label: "全部账号" },
            { value: "manual", label: "手动勾选" },
          ]}
          value={scope}
          onChange={setScope}
        />
        <label className="chk">
          <input
            type="checkbox"
            checked={skipSigned}
            onChange={(e) => setSkipSigned(e.target.checked)}
          />
          跳过今日已签
        </label>
        <label className="chk">
          <input
            type="checkbox"
            checked={skipExpired}
            onChange={(e) => setSkipExpired(e.target.checked)}
          />
          跳过失效账号
        </label>
        <span className="spacer" />
        <span className="chk-count">目标：{targets.length} 个账号</span>
        <Button variant="pri" onClick={run} disabled={running || targets.length === 0}>
          {running ? <RefreshCw className="spin" /> : <Play />}
          {running ? "签到中..." : "开始签到"}
        </Button>
      </div>

      {/* 逐账号结果行 */}
      <div className="rows">
        {filtered.length === 0 ? (
          <div className="checkin-empty">暂无{platformLabel(platform)}账号</div>
        ) : (
          filtered.map((acc) => {
            const r = results[acc.id];
            const expired = isExpired(acc);
            const inTargets = targets.some((t) => t.id === acc.id);
            return (
              <div
                key={acc.id}
                className={`log-row${inTargets ? "" : " off"}`}
              >
                {scope === "manual" && (
                  <input
                    type="checkbox"
                    className="chk-box"
                    checked={selected.has(acc.id)}
                    onChange={() => toggle(acc.id)}
                  />
                )}
                <Avatar name={acc.name} src={acc.avatar} size="sm" />
                <div className="ck-id">
                  <div className="log-name">{acc.name || "未命名"}</div>
                  <div className="log-note mono">{acc.id.slice(0, 8)}</div>
                </div>
                <span className="log-note ck-cred">
                  {expired ? "已失效 · " : ""}
                  {acc.token_expires_at ? `${formatDate(acc.token_expires_at)} 到期` : "—"}
                </span>
                <span className="rval ck-points">{todayPoints[acc.id] ?? 0} 积分</span>
                {r ? (
                  <Chip
                    tone={r.skipped ? "idle" : r.success ? "ok" : "bad"}
                    dot
                  >
                    <span title={r.message}>{r.message}</span>
                  </Chip>
                ) : checkingId === acc.id ? (
                  <Chip tone="run" dot>
                    签到中…
                  </Chip>
                ) : (
                  <Chip tone={isSigned(acc) ? "ok" : "idle"} dot>
                    {isSigned(acc) ? "已签" : "未签"}
                  </Chip>
                )}
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}
