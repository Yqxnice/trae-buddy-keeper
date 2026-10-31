import { useEffect, useMemo, useState } from "react";
import {
  RefreshCw,
  BarChart3,
  ArrowDownToLine,
  ArrowUpFromLine,
  Database,
  HardDrive,
  MessagesSquare,
  Zap,
} from "lucide-react";
import {
  Bar,
  BarChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Tabs, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { api, type TokenStatistics, type TokenStatsSource } from "@/lib/api";
import { cn } from "@/lib/utils";

type ToastType = "success" | "error" | "info";

type SourceKey = "workbuddy" | "workbuddy-ai" | "codebuddy-ide";
type RangeKey = 0 | 7 | 30 | 90;

const SOURCE_LABELS: Record<SourceKey, string> = {
  workbuddy: "WorkBuddy",
  "workbuddy-ai": "WorkBuddy 国际版",
  "codebuddy-ide": "CodeBuddy IDE",
};

const RANGE_OPTIONS: { key: RangeKey; label: string }[] = [
  { key: 0, label: "全部" },
  { key: 7, label: "近 7 天" },
  { key: 30, label: "近 30 天" },
  { key: 90, label: "近 90 天" },
];

function fmt(n: number): string {
  return n.toLocaleString("zh-CN");
}

/** 把 daily（按 total 倒序）改成按日期升序，供趋势图使用 */
function dailyForChart(source: TokenStatsSource) {
  return [...source.daily]
    .sort((a, b) => a.key.localeCompare(b.key))
    .map((d) => ({
      date: d.key.slice(5), // MM-DD
      input: d.input,
      output: d.output,
      cacheWrite: d.cacheWrite,
      total: d.total,
    }));
}

interface SummaryCardProps {
  label: string;
  value: string;
  sub?: string;
  icon: React.ComponentType<{ className?: string }>;
  tone?: "default" | "input" | "output" | "cache";
}

function SummaryCard({ label, value, sub, icon: Icon, tone = "default" }: SummaryCardProps) {
  const toneClass =
    tone === "input"
      ? "text-blue-600"
      : tone === "output"
      ? "text-emerald-600"
      : tone === "cache"
      ? "text-amber-600"
      : "text-foreground";
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardDescription className="flex items-center gap-2">
          <Icon className={cn("h-4 w-4", toneClass)} />
          {label}
        </CardDescription>
      </CardHeader>
      <CardContent>
        <div className={cn("text-2xl font-bold", toneClass)}>{value}</div>
        {sub && <div className="text-xs text-muted-foreground mt-1">{sub}</div>}
      </CardContent>
    </Card>
  );
}

/** 横向条形分布（模型 / 项目） */
function DistributionList({
  title,
  items,
  color,
}: {
  title: string;
  items: { key: string; total: number }[];
  color: string;
}) {
  const max = items.length > 0 ? Math.max(...items.map((i) => i.total), 1) : 1;
  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-base">{title}</CardTitle>
      </CardHeader>
      <CardContent>
        {items.length === 0 ? (
          <p className="text-sm text-muted-foreground">暂无数据</p>
        ) : (
          <div className="space-y-2.5">
            {items.slice(0, 10).map((item) => (
              <div key={item.key}>
                <div className="flex items-center justify-between text-sm mb-1">
                  <span className="truncate max-w-[60%] font-medium" title={item.key}>
                    {item.key}
                  </span>
                  <span className="text-muted-foreground tabular-nums">{fmt(item.total)}</span>
                </div>
                <div className="h-2 rounded-full bg-muted overflow-hidden">
                  <div
                    className="h-full rounded-full transition-all"
                    style={{ width: `${(item.total / max) * 100}%`, backgroundColor: color }}
                  />
                </div>
              </div>
            ))}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

export function TokenStatsPage({
  pushToast,
}: {
  pushToast: (t: string, b: string, type?: ToastType) => void;
}) {
  const [data, setData] = useState<TokenStatistics | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [source, setSource] = useState<SourceKey>("workbuddy");
  const [range, setRange] = useState<RangeKey>(0);

  const load = async (days: RangeKey) => {
    try {
      setRefreshing(true);
      const result = await api.getTokenStatistics(days === 0 ? undefined : days);
      setData(result);
    } catch (e: any) {
      pushToast("加载失败", e?.toString() || "未知错误", "error");
    } finally {
      setLoading(false);
      setRefreshing(false);
    }
  };

  useEffect(() => {
    load(range);
  }, [range]);

  const current: TokenStatsSource | undefined = useMemo(() => {
    return data?.sources.find((s) => s.source === source);
  }, [data, source]);

  const summary = current?.summary;
  const chartData = current ? dailyForChart(current) : [];

  return (
    <div className="p-6 space-y-6">
      {/* 头部 */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold flex items-center gap-2">
            <BarChart3 className="h-6 w-6" />
            Token 统计
          </h1>
          <p className="text-sm text-muted-foreground mt-1">
            扫描本地日志聚合，数据不上传；国内版与国际版独立统计
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={() => load(range)} disabled={refreshing}>
          <RefreshCw className={cn("h-4 w-4 mr-2", refreshing && "animate-spin")} />
          刷新
        </Button>
      </div>

      {/* 数据源 + 时间范围 */}
      <div className="flex flex-wrap items-center gap-4">
        <Tabs value={source} onValueChange={(v: string) => setSource(v as SourceKey)}>
          <TabsList>
            {(Object.keys(SOURCE_LABELS) as SourceKey[]).map((key) => (
              <TabsTrigger key={key} value={key}>
                {SOURCE_LABELS[key]}
              </TabsTrigger>
            ))}
          </TabsList>
        </Tabs>
        <div className="flex items-center gap-1 rounded-full bg-muted p-0.5">
          {RANGE_OPTIONS.map((opt) => (
            <button
              key={opt.key}
              onClick={() => setRange(opt.key)}
              className={cn(
                "px-3 py-1 rounded-full text-xs font-medium transition-colors",
                range === opt.key
                  ? "bg-primary text-primary-foreground shadow-sm"
                  : "text-muted-foreground hover:text-foreground"
              )}
            >
              {opt.label}
            </button>
          ))}
        </div>
        {current && (
          <span className="text-xs text-muted-foreground">
            扫描 {current.filesScanned} 个文件
            {current.parseErrors > 0 && (
              <span className="text-yellow-600"> · {current.parseErrors} 行解析失败</span>
            )}
          </span>
        )}
      </div>

      {loading ? (
        <div className="flex items-center justify-center h-64">
          <RefreshCw className="h-8 w-8 animate-spin text-muted-foreground" />
        </div>
      ) : !current ? (
        <Card>
          <CardContent className="py-16 text-center text-muted-foreground">
            暂无 {SOURCE_LABELS[source]} 的日志数据
          </CardContent>
        </Card>
      ) : (
        <>
          {/* 概览卡片 */}
          <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-6 gap-4">
            <SummaryCard
              label="总 Token"
              value={fmt(summary!.total)}
              sub={`${fmt(summary!.records)} 次请求`}
              icon={Zap}
            />
            <SummaryCard
              label="输入"
              value={fmt(summary!.input)}
              sub={`未命中 ${fmt(summary!.uncachedInput)}`}
              icon={ArrowDownToLine}
              tone="input"
            />
            <SummaryCard
              label="输出"
              value={fmt(summary!.output)}
              icon={ArrowUpFromLine}
              tone="output"
            />
            <SummaryCard
              label="缓存命中"
              value={fmt(summary!.cacheRead)}
              sub={
                summary!.cacheHitRate != null
                  ? `命中率 ${(summary!.cacheHitRate * 100).toFixed(1)}%`
                  : undefined
              }
              icon={Database}
              tone="cache"
            />
            <SummaryCard
              label="缓存写入"
              value={fmt(summary!.cacheWrite)}
              icon={HardDrive}
              tone="cache"
            />
            <SummaryCard
              label="请求数"
              value={fmt(summary!.records)}
              icon={MessagesSquare}
            />
          </div>

          {/* 每日趋势 */}
          <Card>
            <CardHeader className="pb-3">
              <CardTitle className="text-base">每日 Token 用量</CardTitle>
              <CardDescription>按日期聚合的输入 / 输出 / 缓存写入</CardDescription>
            </CardHeader>
            <CardContent>
              {chartData.length === 0 ? (
                <p className="text-sm text-muted-foreground py-8 text-center">暂无数据</p>
              ) : (
                <div className="h-64">
                  <ResponsiveContainer width="100%" height="100%">
                    <BarChart data={chartData} margin={{ top: 10, right: 10, left: 0, bottom: 0 }}>
                      <CartesianGrid strokeDasharray="3 3" vertical={false} className="stroke-muted" />
                      <XAxis dataKey="date" tick={{ fontSize: 12 }} className="text-muted-foreground" />
                      <YAxis tick={{ fontSize: 12 }} className="text-muted-foreground" />
                      <Tooltip
                        formatter={(value: number, name: string) => [fmt(value), name]}
                        labelFormatter={(label) => `日期: ${label}`}
                      />
                      <Bar dataKey="input" name="输入" stackId="a" fill="#3b82f6" />
                      <Bar dataKey="output" name="输出" stackId="a" fill="#10b981" />
                      <Bar dataKey="cacheWrite" name="缓存写入" stackId="a" fill="#f59e0b" />
                    </BarChart>
                  </ResponsiveContainer>
                </div>
              )}
            </CardContent>
          </Card>

          {/* 模型 / 项目分布 */}
          <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
            <DistributionList
              title="按模型分布"
              items={current.models.map((m) => ({ key: m.key, total: m.total }))}
              color="#3b82f6"
            />
            <DistributionList
              title="按项目分布"
              items={current.projects.map((p) => ({ key: p.key, total: p.total }))}
              color="#10b981"
            />
          </div>

          {/* 会话排行 */}
          <Card>
            <CardHeader className="pb-3">
              <CardTitle className="text-base">会话排行（Top 15）</CardTitle>
            </CardHeader>
            <CardContent>
              {current.sessions.length === 0 ? (
                <p className="text-sm text-muted-foreground">暂无会话数据</p>
              ) : (
                <div className="overflow-x-auto">
                  <table className="w-full text-sm">
                    <thead>
                      <tr className="border-b text-left text-muted-foreground">
                        <th className="py-2 pr-4 font-medium">会话</th>
                        <th className="py-2 pr-4 font-medium">项目</th>
                        <th className="py-2 pr-4 font-medium text-right">输入</th>
                        <th className="py-2 pr-4 font-medium text-right">输出</th>
                        <th className="py-2 pr-4 font-medium text-right">请求</th>
                        <th className="py-2 font-medium text-right">合计</th>
                      </tr>
                    </thead>
                    <tbody>
                      {current.sessions.slice(0, 15).map((s) => (
                        <tr key={s.key} className="border-b last:border-0 hover:bg-accent/30">
                          <td className="py-2 pr-4">
                            <div className="font-medium truncate max-w-[240px]" title={s.key}>
                              {s.title || s.sessionId}
                            </div>
                          </td>
                          <td className="py-2 pr-4 text-muted-foreground">{s.project}</td>
                          <td className="py-2 pr-4 text-right tabular-nums">{fmt(s.input)}</td>
                          <td className="py-2 pr-4 text-right tabular-nums">{fmt(s.output)}</td>
                          <td className="py-2 pr-4 text-right tabular-nums">{fmt(s.records)}</td>
                          <td className="py-2 text-right font-semibold tabular-nums">
                            {fmt(s.total)}
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </CardContent>
          </Card>
        </>
      )}
    </div>
  );
}
