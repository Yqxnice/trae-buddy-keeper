import { useEffect, useState } from "react";
import {
  TrendingUp,
  Calendar,
  Coins,
  AlertTriangle,
  RefreshCw,
} from "lucide-react";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { api, type PointsDashboard } from "@/lib/api";
import { formatDate, platformLabel } from "@/lib/utils";
import {
  Area,
  AreaChart,
  CartesianGrid,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";

type ToastType = "success" | "error" | "info";

export function Dashboard({
  platform,
  pushToast,
}: {
  platform: string;
  pushToast: (t: string, b: string, type?: ToastType) => void;
}) {
  const [data, setData] = useState<PointsDashboard | null>(null);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);

  // 全屏 loading 仅首次挂载；手动刷新原地更新数据并 toast 结果
  const load = async (notify = false) => {
    if (notify) setRefreshing(true);
    try {
      const d = await api.getPointsDashboard(platform);
      setData(d);
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
      <div className="flex items-center justify-center h-full">
        <RefreshCw className="h-8 w-8 animate-spin text-muted-foreground" />
      </div>
    );
  }

  return (
    <div className="p-6 space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">{platformLabel(platform)} 总览</h1>
          <p className="text-sm text-muted-foreground">当前产品账号积分汇总</p>
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={() => load(true)}
          disabled={refreshing}
        >
          <RefreshCw
            className={`h-4 w-4 mr-2 ${refreshing ? "animate-spin" : ""}`}
          />
          刷新
        </Button>
      </div>

      {/* 统计卡片 */}
      <div className="grid grid-cols-3 gap-4">
        <Card>
          <CardHeader className="pb-2">
            <CardDescription className="flex items-center gap-2">
              <Coins className="h-4 w-4" /> 累计积分
            </CardDescription>
          </CardHeader>
          <CardContent>
            <div className="text-3xl font-bold">{data.total_points.toLocaleString()}</div>
          </CardContent>
        </Card>
        <Card>
          <CardHeader className="pb-2">
            <CardDescription className="flex items-center gap-2">
              <TrendingUp className="h-4 w-4" /> 今日新增
            </CardDescription>
          </CardHeader>
          <CardContent>
            <div className="text-3xl font-bold text-green-600">
              +{data.today_new.toLocaleString()}
            </div>
          </CardContent>
        </Card>
        <Card>
          <CardHeader className="pb-2">
            <CardDescription className="flex items-center gap-2">
              <AlertTriangle className="h-4 w-4" /> 即将到期
            </CardDescription>
          </CardHeader>
          <CardContent>
            <div className="text-3xl font-bold text-orange-500">
              {data.expiring_packages.length}
              <span className="text-sm font-normal text-muted-foreground ml-1">个积分包</span>
            </div>
          </CardContent>
        </Card>
      </div>

      {/* 7 日趋势 */}
      <Card>
        <CardHeader>
          <CardTitle className="text-base flex items-center gap-2">
            <Calendar className="h-4 w-4" /> 近 7 日积分趋势
          </CardTitle>
        </CardHeader>
        <CardContent>
          <div className="h-64">
            <ResponsiveContainer width="100%" height="100%">
              <AreaChart data={data.trend}>
                <defs>
                  <linearGradient id="colorPoints" x1="0" y1="0" x2="0" y2="1">
                    <stop offset="5%" stopColor="#3b82f6" stopOpacity={0.3} />
                    <stop offset="95%" stopColor="#3b82f6" stopOpacity={0} />
                  </linearGradient>
                </defs>
                <CartesianGrid strokeDasharray="3 3" stroke="#e5e7eb" />
                <XAxis dataKey="date" tick={{ fontSize: 12 }} />
                <YAxis tick={{ fontSize: 12 }} />
                <Tooltip />
                <Area
                  type="monotone"
                  dataKey="points"
                  stroke="#3b82f6"
                  strokeWidth={2}
                  fill="url(#colorPoints)"
                />
              </AreaChart>
            </ResponsiveContainer>
          </div>
        </CardContent>
      </Card>

      {/* 到期提醒 */}
      {data.expiring_packages.length > 0 && (
        <Card>
          <CardHeader>
            <CardTitle className="text-base flex items-center gap-2">
              <AlertTriangle className="h-4 w-4 text-orange-500" /> 积分包到期提醒（7 日内）
            </CardTitle>
          </CardHeader>
          <CardContent>
            <div className="space-y-2">
              {data.expiring_packages.map((p, i) => (
                <div
                  key={i}
                  className="flex items-center justify-between rounded-lg border p-3"
                >
                  <div className="flex items-center gap-3">
                    <span className="font-medium">{p.account_name}</span>
                    <span className="text-sm text-muted-foreground">
                      {p.points.toLocaleString()} 积分
                    </span>
                  </div>
                  <div className="flex items-center gap-3">
                    <Badge variant={p.days_left <= 1 ? "destructive" : "warning"}>
                      {p.days_left <= 0 ? "今日到期" : `${p.days_left} 天后到期`}
                    </Badge>
                    <span className="text-xs text-muted-foreground">
                      {formatDate(p.expires_at)}
                    </span>
                  </div>
                </div>
              ))}
            </div>
          </CardContent>
        </Card>
      )}
    </div>
  );
}
