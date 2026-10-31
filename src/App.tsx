import { useState, useEffect, type ReactNode } from "react";
import {
  LayoutDashboard,
  Users,
  Settings,
  Minus,
  Square,
  Copy,
  X,
  Coins,
  CalendarCheck,
  BarChart3,
  Clock,
  ScrollText,
} from "lucide-react";
import { Dashboard } from "@/pages/Dashboard/Dashboard";
import { AccountsPage } from "@/pages/Accounts/Accounts";
import { CreditsPage } from "@/pages/Credits/Credits";
import { CheckinPage } from "@/pages/Checkin/Checkin";
import { TokenStatsPage } from "@/pages/TokenStats/TokenStats";
import { LogsPage } from "@/pages/Logs/Logs";
import { SchedulerPage } from "@/pages/Scheduler/Scheduler";
import { SettingsPage } from "@/pages/Settings/Settings";
import { ToastContainer } from "@/components/Toast/Toast";
import { ConfirmHost } from "@/components/ConfirmDialog/ConfirmDialog";
import { SelectorBar } from "@/components/SelectorBar/SelectorBar";
import { SideNav, type NavItem } from "@/components/SideNav/SideNav";
import { api } from "@/lib/api";
import { useCheckinStore } from "@/stores/checkin";
import { useUIStore } from "@/stores/ui";
import { platformLabel, checkinDayStart } from "@/lib/utils";
import { applyTheme, normalizeTheme } from "@/lib/theme";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./App.css";

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

// 浏览器预览环境没有 Tauri internals，兜底空实现避免白屏（Tauri 内运行不受影响）
const appWindow = (() => {
  try {
    return getCurrentWindow();
  } catch {
    return {
      isMaximized: async () => false,
      onResized: async () => () => {},
      minimize: () => {},
      toggleMaximize: () => {},
      close: () => {},
    } as unknown as typeof getCurrentWindow extends () => infer T ? T : never;
  }
})();

/** 自定义标题栏：品牌标识 + 产品下划线切换 + 原生窗口控制 */
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
    <header className="titlebar" data-tauri-drag-region>
      {/* 应用标识 */}
      <div className="tb-brand" data-tauri-drag-region>
        <span className="tb-mark" />
        <span>AI 账户管家</span>
      </div>

      {/* 产品切换：一个产品对应一组功能 */}
      <SelectorBar
        options={PLATFORMS.map((p) => ({ value: p, label: platformLabel(p) }))}
        value={platform}
        onChange={onPlatformChange}
      />

      {/* 窗口控制 */}
      <div className="tb-ctl">
        <button onClick={() => appWindow.minimize()} title="最小化">
          <Minus size={11} strokeWidth={1.6} />
        </button>
        <button
          onClick={() => appWindow.toggleMaximize()}
          title={maximized ? "还原" : "最大化"}
        >
          {maximized ? (
            <Copy size={11} strokeWidth={1.6} />
          ) : (
            <Square size={10} strokeWidth={1.6} />
          )}
        </button>
        <button className="close" onClick={() => appWindow.close()} title="关闭">
          <X size={13} strokeWidth={1.6} />
        </button>
      </div>
    </header>
  );
}

export default function App() {
  const [tab, setTab] = useState<Tab>("dashboard");
  // 当前产品与通知堆栈：来自全局 UI store（Titlebar 写 / 各页面读）
  const platform = useUIStore((s) => s.platform);

  // 状态脊柱微点：账号异常（警告）/ 今日未签（品牌绿）
  const [navDots, setNavDots] = useState<{ accounts?: "warn" | null; checkin?: "brand" | null }>({});

  // 监听后端 toast 事件
  useEffect(() => {
    const unlisten = listen<{ title: string; body: string }>("app-toast", (e) => {
      useUIStore.getState().pushToast(e.payload.title, e.payload.body, "info");
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

  // 定时签到设置为跨页共享状态（总览读 / 定时任务写），启动时加载一次
  useEffect(() => {
    useCheckinStore.getState().load().catch(() => {});
  }, []);

  // 监听托盘签到事件（托盘为全局操作，签到所有产品）
  useEffect(() => {
    const unlisten = listen("tray-checkin", async () => {
      try {
        const results = await api.checkinAll();
        const success = results.filter((r) => r.success).length;
        useUIStore
          .getState()
          .pushToast(
            "批量签到完成",
            `成功 ${success} / ${results.length}`,
            success === results.length ? "success" : "warning"
          );
      } catch (e: any) {
        useUIStore.getState().pushToast("签到失败", e?.toString() || "未知错误", "error");
      }
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  // 状态脊柱：静默拉取账号清单，标记异常（失败/冷却/凭证过期）与今日未签；
  // 60s 轮询一次，签到 / 切换等操作后微点自动跟随真实状态
  useEffect(() => {
    let alive = true;
    const refresh = () =>
      api
        .listAccounts()
        .then((list) => {
          if (!alive) return;
          const mine = list.filter((a) => a.platform === platform);
          const now = Date.now() / 1000;
          const boundary = checkinDayStart();
          const hasTrouble = mine.some(
            (a) =>
              a.checkin_status === "failed" ||
              a.checkin_status === "cooling" ||
              (a.token_expires_at > 0 && a.token_expires_at < now)
          );
          const hasUnsigned = mine.some((a) => a.last_checkin_time < boundary);
          setNavDots({
            accounts: hasTrouble ? "warn" : null,
            checkin: hasUnsigned ? "brand" : null,
          });
        })
        .catch(() => {});
    refresh();
    const timer = setInterval(refresh, 60_000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [platform]);

  // keep-alive：访问过的页面保留挂载（隐藏而非卸载），切走后正在执行的
  // 签到 / 刷新积分 / 续期等任务继续在后台跑，切回来仍能看到进度与结果
  const [visited, setVisited] = useState<Tab[]>(["dashboard"]);
  useEffect(() => {
    setVisited((v) => (v.includes(tab) ? v : [...v, tab]));
  }, [tab]);

  // TRAE 没有可统计的 token 用量数据源（本地对话库加密、服务端只有 credits 口径），
  // 因此该产品下不展示 Token 统计入口
  const isVisible = (id: Tab) => !(platform === "trae" && id === "token-stats");

  const navItems: NavItem[] = [
    { id: "dashboard", label: "总览", icon: LayoutDashboard },
    { id: "accounts", label: "账号", icon: Users, dot: navDots.accounts ?? null },
    { id: "credits", label: "积分", icon: Coins },
    { id: "checkin", label: "签到", icon: CalendarCheck, dot: navDots.checkin ?? null },
    ...(platform === "trae"
      ? []
      : [{ id: "token-stats" as Tab, label: "Token 统计", icon: BarChart3 }]),
  ];

  const footItems: NavItem[] = [
    { id: "scheduler", label: "定时任务", icon: Clock },
    { id: "logs", label: "系统日志", icon: ScrollText },
    { id: "settings", label: "设置", icon: Settings },
  ];

  // 切到 TRAE 时若停留在 Token 统计，退回总览，避免停留在已隐藏的页面
  const handlePlatformChange = (p: string) => {
    useUIStore.getState().setPlatform(p);
    if (p === "trae") {
      setTab((current) => (current === "token-stats" ? "dashboard" : current));
    }
  };

  // 各页面常驻：由外层控制显示/隐藏，组件不因切换而重建
  const pages: { id: Tab; node: ReactNode }[] = [
    { id: "dashboard", node: <Dashboard /> },
    { id: "accounts", node: <AccountsPage /> },
    { id: "credits", node: <CreditsPage /> },
    { id: "checkin", node: <CheckinPage /> },
    { id: "token-stats", node: <TokenStatsPage /> },
    { id: "scheduler", node: <SchedulerPage /> },
    { id: "logs", node: <LogsPage /> },
    { id: "settings", node: <SettingsPage /> },
  ];

  return (
    <div className="app-shell">
      <Titlebar platform={platform} onPlatformChange={handlePlatformChange} />

      <div className="app-body">
        {/* 状态脊柱：当前产品下的功能导航 */}
        <SideNav
          items={navItems}
          footItems={footItems}
          value={tab}
          onChange={(id) => setTab(id as Tab)}
        />

        {/* 主内容：访问过的页面保留挂载，非当前页用 hidden 隐藏 */}
        <main className="main scroll-area">
          {pages.map((page) =>
            isVisible(page.id) && (tab === page.id || visited.includes(page.id)) ? (
              <div
                key={page.id}
                className={`view-page${tab === page.id ? " anim-viewin" : ""}`}
                hidden={tab !== page.id}
              >
                {page.node}
              </div>
            ) : null
          )}
        </main>
      </div>

      <ToastContainer />
      <ConfirmHost />
    </div>
  );
}
