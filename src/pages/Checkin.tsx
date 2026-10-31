import { useEffect, useMemo, useState } from "react";
import { Play, RefreshCw, CircleCheck, CircleSlash, CircleX } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { api, type AccountInfo, type CheckinResult } from "@/lib/api";
import { formatDate, platformLabel, checkinDayStart } from "@/lib/utils";

type ToastType = "success" | "error" | "info";

export function CheckinPage({
  platform,
  pushToast,
}: {
  platform: string;
  pushToast: (t: string, b: string, type?: ToastType) => void;
}) {
  const [accounts, setAccounts] = useState<AccountInfo[]>([]);
  const [todayPoints, setTodayPoints] = useState<Record<string, number>>({});
  const [loading, setLoading] = useState(true);
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
      <div className="flex items-center justify-center h-full">
        <RefreshCw className="h-8 w-8 animate-spin text-muted-foreground" />
      </div>
    );
  }

  return (
    <div className="p-6 space-y-6">
      <div>
        <h1 className="text-2xl font-bold">{platformLabel(platform)} 签到</h1>
        <p className="text-sm text-muted-foreground">
          多账号每日签到 · 支持手动执行与结果查看
        </p>
      </div>

      {/* 签到范围 */}
      <Card>
        <CardContent className="p-4 space-y-4">
          <div className="flex items-center justify-between flex-wrap gap-3">
            <div className="space-y-3">
              <div className="flex items-center gap-2">
                <span className="text-sm font-medium mr-2">签到范围</span>
                <Button
                  variant={scope === "all" ? "default" : "outline"}
                  size="sm"
                  onClick={() => setScope("all")}
                >
                  全部账号
                </Button>
                <Button
                  variant={scope === "manual" ? "default" : "outline"}
                  size="sm"
                  onClick={() => setScope("manual")}
                >
                  手动勾选
                </Button>
              </div>
              <div className="flex items-center gap-4 text-sm">
                <label className="flex items-center gap-1.5 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={skipSigned}
                    onChange={(e) => setSkipSigned(e.target.checked)}
                    className="accent-primary"
                  />
                  跳过今日已签
                </label>
                <label className="flex items-center gap-1.5 cursor-pointer">
                  <input
                    type="checkbox"
                    checked={skipExpired}
                    onChange={(e) => setSkipExpired(e.target.checked)}
                    className="accent-primary"
                  />
                  跳过失效账号
                </label>
              </div>
            </div>
            <div className="flex items-center gap-3">
              <span className="text-sm text-muted-foreground">
                目标：{targets.length} 个账号
              </span>
              <Button onClick={run} disabled={running || targets.length === 0}>
                {running ? (
                  <RefreshCw className="h-4 w-4 mr-2 animate-spin" />
                ) : (
                  <Play className="h-4 w-4 mr-2" />
                )}
                {running ? "签到中..." : "开始签到"}
              </Button>
            </div>
          </div>
        </CardContent>
      </Card>

      {/* 账号列表 */}
      <div>
        <p className="text-sm text-muted-foreground mb-2">
          参与签到的账号（{scope === "all" ? filtered.length : selected.size}）
        </p>
        <Card>
          <CardContent className="p-0">
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b text-muted-foreground">
                  <th className="text-left font-medium px-4 py-3">账号</th>
                  <th className="text-right font-medium px-4 py-3">凭证状态</th>
                  <th className="text-right font-medium px-4 py-3">今日</th>
                  <th className="text-right font-medium px-4 py-3">签到积分</th>
                  <th className="text-right font-medium px-4 py-3">结果</th>
                </tr>
              </thead>
              <tbody>
                {filtered.length === 0 ? (
                  <tr>
                    <td colSpan={5} className="text-center py-12 text-muted-foreground">
                      暂无{platformLabel(platform)}账号
                    </td>
                  </tr>
                ) : (
                  filtered.map((acc) => {
                    const r = results[acc.id];
                    const signed = isSigned(acc);
                    const expired = isExpired(acc);
                    const inTargets = targets.some((t) => t.id === acc.id);
                    return (
                      <tr
                        key={acc.id}
                        className={`border-b last:border-0 ${inTargets ? "" : "opacity-50"}`}
                      >
                        <td className="px-4 py-3">
                          <div className="flex items-center gap-2">
                            {scope === "manual" && (
                              <input
                                type="checkbox"
                                checked={selected.has(acc.id)}
                                onChange={() => toggle(acc.id)}
                                className="accent-primary"
                              />
                            )}
                            <div>
                              <div className="font-medium">{acc.name || "未命名"}</div>
                              <div className="text-xs text-muted-foreground">
                                {acc.id.slice(0, 8)}
                              </div>
                            </div>
                          </div>
                        </td>
                        <td className="px-4 py-3 text-right">
                          {acc.token_expires_at ? (
                            <span
                              className={
                                expired ? "text-destructive" : "text-muted-foreground"
                              }
                            >
                              {expired ? "已失效 · " : ""}
                              {formatDate(acc.token_expires_at)} 到期
                            </span>
                          ) : (
                            <span className="text-muted-foreground">—</span>
                          )}
                        </td>
                        <td className="px-4 py-3 text-right">
                          {signed ? (
                            <Badge variant="success">已签</Badge>
                          ) : (
                            <Badge variant="secondary">未签</Badge>
                          )}
                        </td>
                        <td className="px-4 py-3 text-right font-medium">
                          {todayPoints[acc.id] ?? 0}
                        </td>
                        <td className="px-4 py-3 text-right">
                          {r ? (
                            <span
                              className={`inline-flex items-center gap-1 text-xs ${
                                r.skipped
                                  ? "text-muted-foreground"
                                  : r.success
                                  ? "text-green-600"
                                  : "text-destructive"
                              }`}
                              title={r.message}
                            >
                              {r.skipped ? (
                                <CircleSlash className="h-3.5 w-3.5" />
                              ) : r.success ? (
                                <CircleCheck className="h-3.5 w-3.5" />
                              ) : (
                                <CircleX className="h-3.5 w-3.5" />
                              )}
                              {r.message}
                            </span>
                          ) : (
                            <span className="text-xs text-muted-foreground">—</span>
                          )}
                        </td>
                      </tr>
                    );
                  })
                )}
              </tbody>
            </table>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
