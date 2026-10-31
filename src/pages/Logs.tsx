import { useEffect, useState } from "react";
import { RefreshCw, Trash2, ScrollText } from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { api, type AppLog } from "@/lib/api";
import { confirmDialog } from "@/components/ConfirmDialog";
import { formatDate } from "@/lib/utils";

type ToastType = "success" | "error" | "info";

const levelBadge = (level: string) => {
  switch (level) {
    case "success":
      return <Badge variant="success">成功</Badge>;
    case "error":
      return <Badge variant="destructive">失败</Badge>;
    default:
      return <Badge variant="secondary">信息</Badge>;
  }
};

export function LogsPage({
  pushToast,
}: {
  pushToast: (t: string, b: string, type?: ToastType) => void;
}) {
  const [logs, setLogs] = useState<AppLog[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);

  // 全屏 loading 仅首次挂载；手动刷新原地更新并 toast 结果
  const load = async (notify = false) => {
    if (notify) setRefreshing(true);
    try {
      setLogs(await api.getLogs(300));
      if (notify) pushToast("刷新成功", "日志已更新", "success");
    } catch (e: any) {
      pushToast(notify ? "刷新失败" : "日志加载失败", e?.toString(), "error");
    } finally {
      setLoading(false);
      if (notify) setRefreshing(false);
    }
  };

  useEffect(() => {
    load();
  }, []);

  const handleClear = async () => {
    if (
      !(await confirmDialog({
        title: "清空系统日志",
        message: "确定清空全部系统日志？此操作不可恢复。",
        confirmText: "清空",
        danger: true,
      }))
    )
      return;
    try {
      await api.clearLogs();
      setLogs([]);
      pushToast("成功", "日志已清空", "success");
    } catch (e: any) {
      pushToast("清空失败", e?.toString(), "error");
    }
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
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">系统日志</h1>
          <p className="text-sm text-muted-foreground">
            签到、积分查询、账户变更等操作记录（保留最近 7 天）
          </p>
        </div>
        <div className="flex items-center gap-2">
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
          <Button variant="outline" size="sm" onClick={handleClear}>
            <Trash2 className="h-4 w-4 mr-2" />
            清空
          </Button>
        </div>
      </div>

      <Card>
        <CardContent className="p-0">
          {logs.length === 0 ? (
            <div className="py-16 text-center text-muted-foreground">
              <ScrollText className="h-8 w-8 mx-auto mb-2 opacity-50" />
              暂无日志
            </div>
          ) : (
            <table className="w-full text-sm">
              <thead>
                <tr className="border-b text-muted-foreground">
                  <th className="text-left font-medium px-4 py-3 w-44">时间</th>
                  <th className="text-left font-medium px-4 py-3 w-20">级别</th>
                  <th className="text-left font-medium px-4 py-3 w-24">类型</th>
                  <th className="text-left font-medium px-4 py-3">内容</th>
                </tr>
              </thead>
              <tbody>
                {logs.map((log) => (
                  <tr key={log.id} className="border-b last:border-0">
                    <td className="px-4 py-2.5 text-muted-foreground whitespace-nowrap">
                      {formatDate(log.created_at)}
                    </td>
                    <td className="px-4 py-2.5">{levelBadge(log.level)}</td>
                    <td className="px-4 py-2.5">{log.action}</td>
                    <td className="px-4 py-2.5 text-muted-foreground break-all">
                      {log.message}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
