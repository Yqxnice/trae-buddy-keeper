import { useState, useEffect, useCallback, type ReactNode } from "react";
import {
  LayoutDashboard,
  Users,
  Settings,
  Sparkles,
  Coins,
  CalendarCheck,
  Clock,
  ScrollText,
  BarChart3,
  Minus,
  Square,
  Copy,
  X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { Dashboard } from "@/pages/Dashboard";
import { AccountsPage } from "@/pages/Accounts";
import { CreditsPage } from "@/pages/Credits";
import { CheckinPage } from "@/pages/Checkin";
import { TokenStatsPage } from "@/pages/TokenStats";
import { LogsPage } from "@/pages/Logs";
import { SchedulerPage } from "@/pages/Scheduler";
import { SettingsPage } from "@/pages/Settings";
import { ToastContainer, type ToastItem, type ToastType } from "@/components/ui/toast";
import { ConfirmHost } from "@/components/ConfirmDialog";
import { api } from "@/lib/api";
import { platformLabel } from "@/lib/utils";
import { applyTheme, normalizeTheme } from "@/lib/theme";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

type Tab =
  | "dashboard"
  | "accounts"
  | "credits"
  | "checkin"
  | "token-stats"
  | "scheduler"
  | "logs"
  | "settings";

const PLATFORMS = ["trae", "workbuddy"] as const;

const appWindow = getCurrentWindow();

/** 自定义标题栏：应用标识 + 产品切换 + 窗口控制 */
function Titlebar({
  platform,
  onPlatformChange,
}: {
  platform: string;
  onPlatformChange: (p: string) => void;
}) {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    appWindow.isMaximized().then(setMaximized).catch(() => {});
    appWindow
      .onResized(() => {
        appWindow.isMaximized().then(setMaximized).catch(() => {});
      })
      .then((f) => (unlisten = f));
    return () => unlisten?.();
  }, []);

  return (
    <header
      data-tauri-drag-region
      className="h-11 shrink-0 flex items-center border-b bg-background select-none"
    >
      {/* 应用标识 */}
      <div data-tauri-drag-region className="flex items-center gap-2 pl-4 pr-3">
        <div className="h-6 w-6 rounded-md bg-primary flex items-center justify-center">
          <Sparkles className="h-3.5 w-3.5 text-primary-foreground" />
        </div>
        <span className="font-semibold text-sm">AI 账户管家</span>
      </div>

      {/* 产品切换：一个产品对应一组功能 */}
      <div data-tauri-drag-region className="flex-1 flex items-center">
        <div className="flex items-center gap-1 rounded-full bg-muted p-0.5">
          {PLATFORMS.map((p) => (
            <button
              key={p}
              onClick={() => onPlatformChange(p)}
              className={cn(
                "px-4 py-1 rounded-full text-xs font-medium transition-colors",
                platform === p
                  ? "bg-primary text-primary-foreground shadow-sm"
                  : "text-muted-foreground hover:text-foreground"
              )}
            >
              {platformLabel(p)}
            </button>
          ))}
        </div>
      </div>

      {/* 窗口控制 */}
      <div className="flex items-center h-full">
        <button
          onClick={() => appWindow.minimize()}
          className="h-full px-4 inline-flex items-center justify-center text-muted-foreground hover:bg-accent hover:text-foreground transition-colors"
          title="最小化"
        >
          <Minus className="h-4 w-4" />
        </button>
        <button
          onClick={() => appWindow.toggleMaximize()}
          className="h-full px-4 inline-flex items-center justify-center text-muted-foreground hover:bg-accent hover:text-foreground transition-colors"
          title={maximized ? "还原" : "最大化"}
        >
          {maximized ? <Copy className="h-3.5 w-3.5" /> : <Square className="h-3.5 w-3.5" />}
        </button>
        <button
          onClick={() => appWindow.close()}
          className="h-full px-4 inline-flex items-center justify-center text-muted-foreground hover:bg-destructive hover:text-destructive-foreground transition-colors"
          title="关闭"
        >
          <X className="h-4 w-4" />
        </button>
      </div>
    </header>
  );
}

export default function App() {
  const [tab, setTab] = useState<Tab>("dashboard");
  const [platform, setPlatform] = useState<string>("trae");
  const [toasts, setToasts] = useState<ToastItem[]>([]);

  // 引用稳定：常驻页面不会因为父级重渲染而反复触发自身副作用
  const pushToast = useCallback(
    (title: string, body: string, type: ToastType = "info") => {
      const id = Math.random().toString(36).slice(2);
      setToasts((t) => [...t, { id, title, body, type }]);
    },
    []
  );

  const dismissToast = (id: string) => {
    setToasts((t) => t.filter((x) => x.id !== id));
  };

  // 监听后端 toast 事件
  useEffect(() => {
    const unlisten = listen<{ title: string; body: string }>("app-toast", (e) => {
      pushToast(e.payload.title, e.payload.body, "info");
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  // 启动时按已保存设置应用外观主题（默认浅色）
  useEffect(() => {
    api
      .getAppSettings()
      .then((s) => applyTheme(normalizeTheme(s.theme)))
      .catch(() => applyTheme("light"));
  }, []);

  // 监听托盘签到事件（托盘为全局操作，签到所有产品）
  useEffect(() => {
    const unlisten = listen("tray-checkin", async () => {
      try {
        const results = await api.checkinAll();
        const success = results.filter((r) => r.success).length;
        pushToast(
          "批量签到完成",
          `成功 ${success} / ${results.length}`,
          success === results.length ? "success" : "warning"
        );
      } catch (e: any) {
        pushToast("签到失败", e?.toString() || "未知错误", "error");
      }
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  // keep-alive：访问过的页面保留挂载（隐藏而非卸载），切走后正在执行的
  // 签到 / 刷新积分 / 续期等任务继续在后台跑，切回来仍能看到进度与结果
  const [visited, setVisited] = useState<Tab[]>(["dashboard"]);
  useEffect(() => {
    setVisited((v) => (v.includes(tab) ? v : [...v, tab]));
  }, [tab]);

  // TRAE 没有可统计的 token 用量数据源（本地对话库加密、服务端只有 credits 口径），
  // 因此该产品下不展示 Token 统计入口
  const isVisible = (id: Tab) => !(platform === "trae" && id === "token-stats");

  const navItems: { id: Tab; label: string; icon: any }[] = [
    { id: "dashboard", label: "总览", icon: LayoutDashboard },
    { id: "accounts", label: "账号", icon: Users },
    { id: "credits", label: "积分", icon: Coins },
    { id: "checkin", label: "签到", icon: CalendarCheck },
    ...(platform === "trae"
      ? []
      : [{ id: "token-stats" as Tab, label: "Token 统计", icon: BarChart3 }]),
  ];

  // 切到 TRAE 时若停留在 Token 统计，退回总览，避免停留在已隐藏的页面
  const handlePlatformChange = (p: string) => {
    setPlatform(p);
    if (p === "trae") {
      setTab((current) => (current === "token-stats" ? "dashboard" : current));
    }
  };

  // 各页面常驻：由外层控制显示/隐藏，组件不因切换而重建
  const pages: { id: Tab; node: ReactNode }[] = [
    { id: "dashboard", node: <Dashboard platform={platform} pushToast={pushToast} /> },
    { id: "accounts", node: <AccountsPage platform={platform} pushToast={pushToast} /> },
    { id: "credits", node: <CreditsPage platform={platform} pushToast={pushToast} /> },
    { id: "checkin", node: <CheckinPage platform={platform} pushToast={pushToast} /> },
    { id: "token-stats", node: <TokenStatsPage pushToast={pushToast} /> },
    { id: "scheduler", node: <SchedulerPage pushToast={pushToast} /> },
    { id: "logs", node: <LogsPage pushToast={pushToast} /> },
    { id: "settings", node: <SettingsPage pushToast={pushToast} /> },
  ];

  return (
    <div className="flex flex-col h-screen bg-muted/30">
      <Titlebar platform={platform} onPlatformChange={handlePlatformChange} />

      <div className="flex flex-1 overflow-hidden">
        {/* 侧边栏：当前产品下的功能导航 */}
        <aside className="w-52 border-r bg-background flex flex-col">
          <div className="px-4 pt-4 pb-2 text-xs text-muted-foreground">
            当前产品：{platformLabel(platform)}
          </div>
          <nav className="flex-1 px-3 space-y-1">
            {navItems.map((item) => (
              <button
                key={item.id}
                onClick={() => setTab(item.id)}
                className={cn(
                  "w-full flex items-center gap-3 px-3 py-2 rounded-md text-sm font-medium transition-colors",
                  tab === item.id
                    ? "bg-accent text-accent-foreground"
                    : "text-muted-foreground hover:bg-accent/50"
                )}
              >
                <item.icon className="h-4 w-4" />
                {item.label}
              </button>
            ))}
          </nav>
          <div className="p-3 space-y-1 border-t">
            <button
              onClick={() => setTab("scheduler")}
              className={cn(
                "w-full flex items-center gap-3 px-3 py-2 rounded-md text-sm font-medium transition-colors",
                tab === "scheduler"
                  ? "bg-accent text-accent-foreground"
                  : "text-muted-foreground hover:bg-accent/50"
              )}
            >
              <Clock className="h-4 w-4" />
              定时任务
            </button>
            <button
              onClick={() => setTab("logs")}
              className={cn(
                "w-full flex items-center gap-3 px-3 py-2 rounded-md text-sm font-medium transition-colors",
                tab === "logs"
                  ? "bg-accent text-accent-foreground"
                  : "text-muted-foreground hover:bg-accent/50"
              )}
            >
              <ScrollText className="h-4 w-4" />
              系统日志
            </button>
            <button
              onClick={() => setTab("settings")}
              className={cn(
                "w-full flex items-center gap-3 px-3 py-2 rounded-md text-sm font-medium transition-colors",
                tab === "settings"
                  ? "bg-accent text-accent-foreground"
                  : "text-muted-foreground hover:bg-accent/50"
              )}
            >
              <Settings className="h-4 w-4" />
              设置
            </button>
          </div>
        </aside>

        {/* 主内容：访问过的页面保留挂载，非当前页用 hidden 隐藏 */}
        <main className="flex-1 overflow-auto">
          {pages.map((page) =>
            isVisible(page.id) && (tab === page.id || visited.includes(page.id)) ? (
              <div key={page.id} className={tab === page.id ? "h-full" : "hidden"}>
                {page.node}
              </div>
            ) : null
          )}
        </main>
      </div>

      <ToastContainer toasts={toasts} onDismiss={dismissToast} />
      <ConfirmHost />
    </div>
  );
}
