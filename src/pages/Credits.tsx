import { useEffect, useState } from "react";
import { Coins, RefreshCw, Package, Clock } from "lucide-react";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { api, type AccountCredits } from "@/lib/api";
import { formatDate, platformLabel } from "@/lib/utils";

type ToastType = "success" | "error" | "info";

function daysLeft(at: number) {
  return Math.floor((at - Date.now() / 1000) / 86400);
}

export function CreditsPage({
  platform,
  pushToast,
}: {
  platform: string;
  pushToast: (t: string, b: string, type?: ToastType) => void;
}) {
  const [data, setData] = useState<AccountCredits[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);

  // 全屏 loading 仅首次挂载生效
  const load = async () => {
    try {
      setData(await api.getCreditPackages(platform));
    } catch (e: any) {
      pushToast("加载失败", e?.toString(), "error");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, [platform]);

  // 两个平台都有真实积分接口；原地更新数据，成功/失败均 toast
  const handleRefresh = async () => {
    setRefreshing(true);
    try {
      const results = await api.refreshAllCredits(platform);
      const failed = results.filter((r) => !r.success);
      if (failed.length > 0) {
        pushToast(
          "部分账号查询失败",
          failed.map((f) => `${f.account_name}: ${f.message}`).join("；"),
          "error"
        );
      } else {
        pushToast("刷新成功", "积分数据已更新", "success");
      }
      setData(await api.getCreditPackages(platform));
    } catch (e: any) {
      pushToast("刷新失败", e?.toString(), "error");
    } finally {
      setRefreshing(false);
    }
  };

  const total = data.reduce((s, a) => s + a.balance, 0);
  const pkgCount = data.reduce((s, a) => s + a.packages.length, 0);

  if (loading) {
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
          <h1 className="text-2xl font-bold">{platformLabel(platform)} 积分</h1>
          <p className="text-sm text-muted-foreground">
            {platform === "workbuddy"
              ? "各账号实时余额与积分包到期明细（数据来自腾讯 billing 接口）"
              : "各账号实时余额与积分包到期明细（数据来自 TRAE 权益接口）"}
          </p>
        </div>
        <Button variant="outline" size="sm" onClick={handleRefresh} disabled={refreshing}>
          <RefreshCw className={`h-4 w-4 mr-2 ${refreshing ? "animate-spin" : ""}`} />
          刷新
        </Button>
      </div>

      {/* 汇总 */}
      <div className="grid grid-cols-2 gap-4">
        <Card>
          <CardHeader className="pb-2">
            <CardDescription className="flex items-center gap-2">
              <Coins className="h-4 w-4" /> 总余额
            </CardDescription>
          </CardHeader>
          <CardContent>
            <div className="text-3xl font-bold">{total.toLocaleString()}</div>
          </CardContent>
        </Card>
        <Card>
          <CardHeader className="pb-2">
            <CardDescription className="flex items-center gap-2">
              <Package className="h-4 w-4" /> 有效积分包
            </CardDescription>
          </CardHeader>
          <CardContent>
            <div className="text-3xl font-bold">
              {pkgCount}
              <span className="text-sm font-normal text-muted-foreground ml-1">个</span>
            </div>
          </CardContent>
        </Card>
      </div>

      {/* 各账号积分包 */}
      {data.length === 0 ? (
        <Card>
          <CardContent className="py-16 text-center text-muted-foreground">
            暂无{platformLabel(platform)}账号
          </CardContent>
        </Card>
      ) : (
        data.map((acc) => (
          <Card key={acc.account_id}>
            <CardHeader className="pb-3">
              <div className="flex items-center justify-between">
                <CardTitle className="text-base">{acc.account_name || "未命名"}</CardTitle>
                <div className="flex items-center gap-3">
                  {acc.fetched_at > 0 && (
                    <span className="text-xs text-muted-foreground">
                      查询于 {formatDate(acc.fetched_at)}
                    </span>
                  )}
                  <span className="text-lg font-bold text-amber-600">
                    {acc.balance.toLocaleString()}
                    <span className="text-xs font-normal text-muted-foreground ml-1">积分</span>
                  </span>
                </div>
              </div>
            </CardHeader>
            <CardContent>
              {acc.packages.length === 0 ? (
                <p className="text-sm text-muted-foreground">无有效积分包</p>
              ) : (
                <div className="space-y-2">
                  {acc.packages.map((p, i) => {
                    const days = daysLeft(p.at);
                    return (
                      <div
                        key={i}
                        className="flex items-center justify-between rounded-lg border p-3"
                      >
                        <div className="flex items-center gap-3">
                          <Package className="h-4 w-4 text-muted-foreground" />
                          <span className="font-medium">
                            {p.amount.toLocaleString()} 积分
                          </span>
                        </div>
                        <div className="flex items-center gap-3">
                          <Badge
                            variant={
                              days < 1 ? "destructive" : days < 7 ? "warning" : "secondary"
                            }
                          >
                            <Clock className="h-3 w-3 mr-1" />
                            {days < 1 ? "今日到期" : `${days} 天后到期`}
                          </Badge>
                          <span className="text-xs text-muted-foreground">
                            {formatDate(p.at)}
                          </span>
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}
            </CardContent>
          </Card>
        ))
      )}
    </div>
  );
}
