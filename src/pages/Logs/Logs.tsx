import { useEffect, useState } from "react";
import { RefreshCw, Trash2 } from "lucide-react";
import { PageHeader } from "@/components/PageHeader/PageHeader";
import { Button } from "@/components/Button/Button";
import { Segmented } from "@/components/Segmented/Segmented";
import { api, type AppLog } from "@/lib/api";
import { confirmDialog } from "@/components/ConfirmDialog/ConfirmDialog";
import { formatDate } from "@/lib/utils";
import { useUIStore } from "@/stores/ui";
import "./Logs.css";

type LvFilter = "all" | "success" | "error";

const LV_LABEL: Record<string, { cls: string; text: string }> = {
  success: { cls: "lv-ok", text: "成功" },
  error: { cls: "lv-err", text: "失败" },
  info: { cls: "lv-info", text: "信息" },
};

export function LogsPage() {
  const pushToast = useUIStore((s) => s.pushToast);
  const [logs, setLogs] = useState<AppLog[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [lv, setLv] = useState<LvFilter>("all");

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
      <div className="page-loading">
        <RefreshCw className="spin" size={26} strokeWidth={1.6} />
      </div>
    );
  }

  const shown = lv === "all" ? logs : logs.filter((l) => l.level === lv);

  return (
    <div className="logs">
      <PageHeader
        title="系统日志"
        sub="签到结果 · 积分查询 · 账户增删 · Token 续期 · 网页授权"
      />

      <div className="tools">
        <Segmented<LvFilter>
          options={[
            { value: "all", label: "全部" },
            { value: "success", label: "成功" },
            { value: "error", label: "失败" },
          ]}
          value={lv}
          onChange={setLv}
        />
        <span className="spacer" />
        <Button onClick={() => load(true)} disabled={refreshing}>
          <RefreshCw className={refreshing ? "spin" : ""} />
          刷新
        </Button>
        <Button variant="danger" onClick={handleClear}>
          <Trash2 />
          清空
        </Button>
      </div>

      {shown.length === 0 ? (
        <div className="logs-empty">暂无日志</div>
      ) : (
        <div className="loglist">
          {shown.map((log) => {
            const lvInfo = LV_LABEL[log.level] ?? LV_LABEL.info;
            return (
              <div className="log-line" key={log.id}>
                <span className="log-time">{formatDate(log.created_at)}</span>
                <span className={`lv ${lvInfo.cls}`}>{lvInfo.text}</span>
                <span className="cat">{log.action}</span>
                <span className="log-msg">{log.message}</span>
              </div>
            );
          })}
        </div>
      )}

      <div className="quiet">
        <span>保留最近 7 天，超出自动清理 · 保留时间可在设置中调整</span>
      </div>
    </div>
  );
}
