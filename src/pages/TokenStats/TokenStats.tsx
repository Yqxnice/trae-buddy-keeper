import { useEffect, useMemo, useState, type CSSProperties } from "react";
import { RefreshCw } from "lucide-react";
import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  LabelList,
  Pie,
  PieChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { PageHeader } from "@/components/PageHeader/PageHeader";
import { Button } from "@/components/Button/Button";
import { Segmented } from "@/components/Segmented/Segmented";
import { api, type TokenStatistics, type TokenStatsSource } from "@/lib/api";
import { useUIStore } from "@/stores/ui";
import "./TokenStats.css";

type SourceKey = "workbuddy" | "workbuddy-ai" | "codebuddy-ide";
type RangeKey = "0" | "7" | "30" | "90";

const SOURCE_LABELS: Record<SourceKey, string> = {
  workbuddy: "WorkBuddy",
  "workbuddy-ai": "WorkBuddy 国际版",
  "codebuddy-ide": "CodeBuddy IDE",
};

const RANGE_OPTIONS: { value: RangeKey; label: string }[] = [
  { value: "0", label: "全部" },
  { value: "7", label: "近 7 天" },
  { value: "30", label: "近 30 天" },
  { value: "90", label: "近 90 天" },
];

/** 趋势图三个系列：顺序即堆叠顺序（输入在底、输出在顶） */
const TREND_SERIES = [
  { key: "input", name: "输入", cls: "in" },
  { key: "output", name: "输出", cls: "out" },
  { key: "cacheWrite", name: "缓存写入", cls: "cache" },
] as const;

const fmt = (n: number): string => n.toLocaleString("zh-CN");
const fmtAxis = (n: number): string =>
  n >= 1e6 ? `${(n / 1e6).toFixed(1)}M` : n >= 1e3 ? `${(n / 1e3).toFixed(0)}k` : `${n}`;
const pct = (v: number, total: number): string =>
  total > 0 ? `${((v / total) * 100).toFixed(1)}%` : "—";

const toMs = (t: number): number => (t < 1e12 ? t * 1000 : t);
const fmtDay = (t: number): string =>
  new Date(toMs(t)).toLocaleDateString("zh-CN", { month: "2-digit", day: "2-digit" });

const shortName = (s: string, n = 10): string => (s.length > n ? `${s.slice(0, n)}…` : s);

/* ==================== 自定义 Tooltip ==================== */

function TrendTooltip({
  active,
  payload,
  label,
}: {
  active?: boolean;
  payload?: { dataKey?: string | number; value?: number | string }[];
  label?: string;
}) {
  if (!active || !payload?.length) return null;
  const total = payload.reduce((s, p) => s + Number(p.value ?? 0), 0);
  return (
    <div className="ts-tip">
      <div className="ts-tip-date">{label}</div>
      {payload.map((p) => {
        const s = TREND_SERIES.find((x) => x.key === p.dataKey);
        return (
          <div className="ts-tip-row" key={String(p.dataKey)}>
            <i className={`ts-dot seg-${s?.cls}`} />
            <span>{s?.name}</span>
            <b>{fmt(Number(p.value))}</b>
          </div>
        );
      })}
      <div className="ts-tip-row ts-tip-total">
        <span>合计</span>
        <b>{fmt(total)}</b>
      </div>
    </div>
  );
}

function HourTooltip({
  active,
  payload,
  label,
}: {
  active?: boolean;
  payload?: { value?: number | string }[];
  label?: string | number;
}) {
  if (!active || !payload?.length) return null;
  return (
    <div className="ts-tip">
      <div className="ts-tip-date">{Number(label)}:00 – {Number(label) + 1}:00</div>
      <div className="ts-tip-row">
        <i className="ts-dot seg-in" />
        <span>Token 用量</span>
        <b>{fmt(Number(payload[0].value))}</b>
      </div>
    </div>
  );
}

function ProjTooltip({
  active,
  payload,
}: {
  active?: boolean;
  payload?: { value?: number | string; payload?: { name?: string } }[];
}) {
  if (!active || !payload?.length) return null;
  return (
    <div className="ts-tip">
      <div className="ts-tip-date">{payload[0].payload?.name}</div>
      <div className="ts-tip-row">
        <i className="ts-dot seg-in" />
        <span>Token 用量</span>
        <b>{fmt(Number(payload[0].value))}</b>
      </div>
    </div>
  );
}

function PieTooltip({
  active,
  payload,
  total,
}: {
  active?: boolean;
  payload?: { value?: number | string; name?: string }[];
  total?: number;
}) {
  if (!active || !payload?.length) return null;
  const v = Number(payload[0].value ?? 0);
  const pct = total ? ((v / total) * 100).toFixed(1) : "0.0";
  return (
    <div className="ts-tip">
      <div className="ts-tip-date">{payload[0].name}</div>
      <div className="ts-tip-row">
        <span>用量</span>
        <b>{fmt(v)}</b>
      </div>
      <div className="ts-tip-row">
        <span>占比</span>
        <b>{pct}%</b>
      </div>
    </div>
  );
}

/* ==================== 页面 ==================== */

export function TokenStatsPage() {
  const pushToast = useUIStore((s) => s.pushToast);
  const [data, setData] = useState<TokenStatistics | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [source, setSource] = useState<SourceKey>("workbuddy");
  const [range, setRange] = useState<RangeKey>("0");

  const load = async (days: RangeKey) => {
    try {
      setRefreshing(true);
      const n = Number(days);
      const result = await api.getTokenStatistics(n === 0 ? undefined : n);
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

  /** 每日趋势（按日期升序） */
  const chartData = useMemo(() => {
    if (!current) return [];
    return [...current.daily]
      .sort((a, b) => a.key.localeCompare(b.key))
      .map((d) => ({
        date: d.key.slice(5), // MM-DD
        input: d.input,
        output: d.output,
        cacheWrite: d.cacheWrite,
      }));
  }, [current]);

  /** 24 小时分布（补齐空小时） */
  const hoursData = useMemo(() => {
    if (!current) return [];
    const map = new Map<number, number>();
    for (const h of current.hours) {
      const hr = Number(h.key);
      if (!Number.isNaN(hr)) map.set(hr, (map.get(hr) ?? 0) + h.total);
    }
    return Array.from({ length: 24 }, (_, i) => ({ hour: i, total: map.get(i) ?? 0 }));
  }, [current]);
  const hoursEmpty = hoursData.every((h) => h.total === 0);

  /** 模型环形图：Top5 + 其他 */
  const modelData = useMemo(() => {
    if (!current) return [];
    const list = current.models.map((m) => ({ name: m.key, value: m.total }));
    if (list.length <= 6) return list;
    const rest = list.slice(5).reduce((s, x) => s + x.value, 0);
    return [...list.slice(0, 5), { name: "其他", value: rest }];
  }, [current]);
  const pieTotal = modelData.reduce((s, x) => s + x.value, 0);

  /** 项目分布：Top 8 横向条形 */
  const projData = useMemo(() => {
    if (!current) return [];
    return current.projects.slice(0, 8).map((p) => ({ name: p.key, value: p.total }));
  }, [current]);

  const sessions = current ? current.sessions.slice(0, 15) : [];

  const coverage =
    current?.coverageStartAt && current?.coverageEndAt
      ? `${fmtDay(current.coverageStartAt)} ~ ${fmtDay(current.coverageEndAt)}`
      : null;

  return (
    <div className="tstats">
      <PageHeader
        title="Token 统计"
        sub="扫描本地日志聚合，数据不上传；国内版与国际版独立统计"
        right={
          <Button onClick={() => load(range)} disabled={refreshing}>
            <RefreshCw className={refreshing ? "spin" : ""} />
            刷新
          </Button>
        }
      />

      {/* 数据源 + 时间范围 */}
      <div className="tools">
        <Segmented
          options={(Object.keys(SOURCE_LABELS) as SourceKey[]).map((k) => ({
            value: k,
            label: SOURCE_LABELS[k],
          }))}
          value={source}
          onChange={(v) => setSource(v as SourceKey)}
        />
        <Segmented options={RANGE_OPTIONS} value={range} onChange={setRange} />
      </div>

      {loading ? (
        <div className="page-loading">
          <RefreshCw className="spin" size={26} strokeWidth={1.6} />
        </div>
      ) : !current ? (
        <div className="tstats-empty">暂无 {SOURCE_LABELS[source]} 的日志数据</div>
      ) : (
        <>
          {/* 概览主带：主指标 + KPI 分组 + 命中率圆环 */}
          <section className="ts-hero">
            <div className="ts-hero-main">
              <div className="ts-hero-label">总 Token 用量</div>
              <div className="ts-hero-total">{fmt(summary!.total)}</div>
              <div className="ts-hero-meta">
                <span>请求数 {fmt(summary!.records)} 次</span>
                {coverage && <span>覆盖 {coverage}</span>}
                <span>扫描 {current.filesScanned} 个文件</span>
                {current.parseErrors > 0 && (
                  <span className="tstats-err">{current.parseErrors} 行解析失败</span>
                )}
              </div>
            </div>
            <div className="ts-hero-kpis">
              <div className="ts-kpi">
                <div className="ts-kpi-l">
                  <i className="ts-dot seg-in" />
                  输入
                </div>
                <div className="ts-kpi-v">{fmt(summary!.input)}</div>
                <div className="ts-kpi-s">未命中 {fmt(summary!.uncachedInput)}</div>
              </div>
              <div className="ts-kpi">
                <div className="ts-kpi-l">
                  <i className="ts-dot seg-out" />
                  输出
                </div>
                <div className="ts-kpi-v">{fmt(summary!.output)}</div>
                <div className="ts-kpi-s">占比 {pct(summary!.output, summary!.total)}</div>
              </div>
              <div className="ts-kpi">
                <div className="ts-kpi-l">
                  <i className="ts-dot seg-ok" />
                  缓存命中
                </div>
                <div className="ts-kpi-v">{fmt(summary!.cacheRead)}</div>
                <div className="ts-kpi-s">占比 {pct(summary!.cacheRead, summary!.total)}</div>
              </div>
              <div className="ts-kpi">
                <div className="ts-kpi-l">
                  <i className="ts-dot seg-cache" />
                  缓存写入
                </div>
                <div className="ts-kpi-v">{fmt(summary!.cacheWrite)}</div>
                <div className="ts-kpi-s">占比 {pct(summary!.cacheWrite, summary!.total)}</div>
              </div>
            </div>
            {summary!.cacheHitRate != null && (
              <div
                className="ts-ring"
                style={{ "--p": summary!.cacheHitRate * 100 } as CSSProperties}
              >
                <div className="ts-ring-in">
                  <b>{(summary!.cacheHitRate * 100).toFixed(1)}%</b>
                  <span>缓存命中率</span>
                </div>
              </div>
            )}
          </section>

          {/* 每日趋势：堆叠面积图 */}
          <section className="ts-panel">
            <header className="ts-panel-h">
              <span className="ts-panel-t">每日 Token 用量</span>
              <div className="ts-legend">
                {TREND_SERIES.map((s) => (
                  <span key={s.key}>
                    <i className={`ts-dot seg-${s.cls}`} />
                    {s.name}
                  </span>
                ))}
              </div>
            </header>
            {chartData.length === 0 ? (
              <div className="tstats-empty ts-empty-flat">暂无数据</div>
            ) : (
              <div className="ts-chart">
                <ResponsiveContainer width="100%" height="100%">
                  <AreaChart data={chartData} margin={{ top: 8, right: 8, left: 0, bottom: 0 }}>
                    <defs>
                      <linearGradient id="ts-g-in" x1="0" y1="0" x2="0" y2="1">
                        <stop offset="0%" style={{ stopColor: "var(--brand)", stopOpacity: 0.55 }} />
                        <stop offset="100%" style={{ stopColor: "var(--brand)", stopOpacity: 0.05 }} />
                      </linearGradient>
                      <linearGradient id="ts-g-out" x1="0" y1="0" x2="0" y2="1">
                        <stop offset="0%" style={{ stopColor: "var(--warn)", stopOpacity: 0.55 }} />
                        <stop offset="100%" style={{ stopColor: "var(--warn)", stopOpacity: 0.05 }} />
                      </linearGradient>
                      <linearGradient id="ts-g-cache" x1="0" y1="0" x2="0" y2="1">
                        <stop offset="0%" style={{ stopColor: "var(--muted)", stopOpacity: 0.35 }} />
                        <stop offset="100%" style={{ stopColor: "var(--muted)", stopOpacity: 0.03 }} />
                      </linearGradient>
                    </defs>
                    <CartesianGrid strokeDasharray="3 6" vertical={false} />
                    <XAxis dataKey="date" tickLine={false} axisLine={false} minTickGap={28} />
                    <YAxis width={44} tickLine={false} axisLine={false} tickFormatter={fmtAxis} />
                    <Tooltip content={<TrendTooltip />} />
                    <Area
                      dataKey="input"
                      stackId="1"
                      stroke="none"
                      fill="url(#ts-g-in)"
                      className="ts-a-in"
                    />
                    <Area
                      dataKey="cacheWrite"
                      stackId="1"
                      stroke="none"
                      fill="url(#ts-g-cache)"
                      className="ts-a-cache"
                    />
                    <Area
                      dataKey="output"
                      stackId="1"
                      stroke="none"
                      fill="url(#ts-g-out)"
                      className="ts-a-out"
                    />
                  </AreaChart>
                </ResponsiveContainer>
              </div>
            )}
          </section>

          {/* 模型分布 + 项目分布 */}
          <div className="ts-grid2">
            <section className="ts-panel">
              <header className="ts-panel-h">
                <span className="ts-panel-t">按模型分布</span>
              </header>
              {modelData.length === 0 ? (
                <div className="tstats-empty ts-empty-flat">暂无数据</div>
              ) : (
                <div className="ts-donut">
                  <div className="ts-donut-chart">
                    <ResponsiveContainer width="100%" height="100%">
                      <PieChart>
                        <Pie
                          data={modelData}
                          dataKey="value"
                          nameKey="name"
                          innerRadius="64%"
                          outerRadius="92%"
                          paddingAngle={2}
                          cornerRadius={4}
                          startAngle={90}
                          endAngle={-270}
                          stroke="none"
                        >
                          {modelData.map((_, i) => (
                            <Cell key={i} className={`ts-pc-${i % 6}`} />
                          ))}
                        </Pie>
                        <Tooltip content={<PieTooltip total={pieTotal} />} />
                      </PieChart>
                    </ResponsiveContainer>
                    <div className="ts-donut-center">
                      <b>{fmtAxis(pieTotal)}</b>
                      <span>总 Token</span>
                    </div>
                  </div>
                  <div className="ts-donut-legend">
                    {modelData.map((m, i) => (
                      <div className="ts-dl-row" key={m.name} title={m.name}>
                        <i className={`ts-dot ts-pc-${i % 6}`} />
                        <span className="ts-dl-name">{shortName(m.name, 16)}</span>
                        <span className="ts-dl-val">{fmt(m.value)}</span>
                        <span className="ts-dl-pct">
                          {pieTotal ? ((m.value / pieTotal) * 100).toFixed(1) : "0.0"}%
                        </span>
                      </div>
                    ))}
                  </div>
                </div>
              )}
            </section>

            <section className="ts-panel">
              <header className="ts-panel-h">
                <span className="ts-panel-t">按项目分布</span>
              </header>
              {projData.length === 0 ? (
                <div className="tstats-empty ts-empty-flat">暂无数据</div>
              ) : (
                <div className="ts-chart-sm">
                  <ResponsiveContainer width="100%" height="100%">
                    <BarChart
                      data={projData}
                      layout="vertical"
                      margin={{ top: 4, right: 52, left: 0, bottom: 0 }}
                    >
                      <XAxis type="number" hide />
                      <YAxis
                        type="category"
                        dataKey="name"
                        width={104}
                        tickLine={false}
                        axisLine={false}
                        tickFormatter={(v: string) => shortName(v)}
                      />
                      <Tooltip content={<ProjTooltip />} cursor={{ fill: "transparent" }} />
                      <Bar dataKey="value" className="ts-pbar" radius={[0, 3, 3, 0]} barSize={13}>
                        <LabelList dataKey="value" position="right" formatter={(v) => fmtAxis(Number(v))} />
                      </Bar>
                    </BarChart>
                  </ResponsiveContainer>
                </div>
              )}
            </section>
          </div>

          {/* 24 小时分布 */}
          <section className="ts-panel">
            <header className="ts-panel-h">
              <span className="ts-panel-t">24 小时用量分布</span>
            </header>
            {hoursEmpty ? (
              <div className="tstats-empty ts-empty-flat">暂无数据</div>
            ) : (
              <div className="ts-chart-sm">
                <ResponsiveContainer width="100%" height="100%">
                  <BarChart data={hoursData} margin={{ top: 4, right: 8, left: 0, bottom: 0 }}>
                    <CartesianGrid strokeDasharray="3 6" vertical={false} />
                    <XAxis
                      dataKey="hour"
                      tickLine={false}
                      axisLine={false}
                      interval={3}
                      tickFormatter={(h: number) => `${h}时`}
                    />
                    <YAxis width={40} tickLine={false} axisLine={false} tickFormatter={fmtAxis} />
                    <Tooltip content={<HourTooltip />} cursor={{ fill: "transparent" }} />
                    <Bar dataKey="total" className="ts-hbar" radius={[3, 3, 0, 0]} />
                  </BarChart>
                </ResponsiveContainer>
              </div>
            )}
          </section>

          {/* 会话排行 */}
          <section className="ts-panel">
            <header className="ts-panel-h">
              <span className="ts-panel-t">会话排行（Top 15）</span>
            </header>
            {sessions.length === 0 ? (
              <div className="tstats-empty ts-empty-flat">暂无会话数据</div>
            ) : (
              <div className="ts-sess">
                <div className="ts-sess-h">
                  <span className="sess-rank">#</span>
                  <span className="sess-name">会话</span>
                  <span className="sess-proj">项目</span>
                  <span className="sess-num">输入</span>
                  <span className="sess-num">输出</span>
                  <span className="sess-num">请求</span>
                  <span className="sess-total">总计</span>
                </div>
                {sessions.map((s, i) => (
                  <div className="ts-sess-r" key={s.key} title={s.key}>
                    <span className="sess-rank">{String(i + 1).padStart(2, "0")}</span>
                    <span className="sess-name">{s.title || s.sessionId}</span>
                    <span className="sess-proj">{s.project || "—"}</span>
                    <span className="sess-num">{fmt(s.input)}</span>
                    <span className="sess-num">{fmt(s.output)}</span>
                    <span className="sess-num">{fmt(s.records)}</span>
                    <span className="sess-total">{fmt(s.total)}</span>
                  </div>
                ))}
              </div>
            )}
          </section>
        </>
      )}
    </div>
  );
}
