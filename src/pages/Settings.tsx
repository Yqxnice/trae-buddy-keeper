import { useEffect, useRef, useState } from "react";
import { Bell, Power, RefreshCw, Monitor, Save, SlidersHorizontal, FolderSearch } from "lucide-react";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { api, type AppSettings } from "@/lib/api";
import { applyTheme, normalizeTheme } from "@/lib/theme";
import { enable, disable, isEnabled } from "@tauri-apps/plugin-autostart";

type ToastType = "success" | "error" | "info";

export function SettingsPage({
  pushToast,
}: {
  pushToast: (t: string, b: string, type?: ToastType) => void;
}) {
  const [appSettings, setAppSettings] = useState<AppSettings | null>(null);
  const [notifMode, setNotifMode] = useState("both");
  const [autoStart, setAutoStart] = useState(false);
  const [theme, setTheme] = useState("light");
  const [detecting, setDetecting] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  const load = async () => {
    setLoading(true);
    try {
      const [m, a] = await Promise.all([
        api.getNotificationMode(),
        api.getAppSettings(),
      ]);
      setNotifMode(m);
      setAppSettings(a);
      setTheme(normalizeTheme(a.theme));
      try {
        setAutoStart(await isEnabled());
      } catch {
        setAutoStart(false);
      }
    } catch (e: any) {
      pushToast("加载失败", e?.toString(), "error");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, []);

  const saveNotif = async (mode: string) => {
    setNotifMode(mode);
    try {
      await api.setNotificationMode(mode);
      pushToast("已保存", "通知方式已更新", "success");
    } catch (e: any) {
      pushToast("保存失败", e?.toString(), "error");
    }
  };

  // 切换外观：立即应用并持久化
  const changeTheme = (mode: string) => {
    setTheme(mode);
    applyTheme(mode);
    if (!appSettings) return;
    saveAppNow({ ...appSettings, theme: mode });
    pushToast("已保存", "外观已更新", "success");
  };

  const toggleAutoStart = async (v: boolean) => {
    try {
      if (v) {
        await enable();
      } else {
        await disable();
      }
      setAutoStart(v);
      pushToast("已保存", v ? "已开启开机启动" : "已关闭开机启动", "success");
    } catch (e: any) {
      pushToast("设置失败", e?.toString(), "error");
    }
  };

  // 日志保存时间：手动保存，不做即时保存
  const saveLogSettings = async () => {
    if (!appSettings) return;
    try {
      await api.setAppSettings(appSettings);
      pushToast("已保存", "日志设置已更新", "success");
    } catch (e: any) {
      pushToast("保存失败", e?.toString(), "error");
    }
  };

  // AppSettings 自动保存：输入防抖静默落盘，失败才提示
  const pendingSave = useRef<number | null>(null);

  const persistApp = async (next: AppSettings) => {
    try {
      await api.setAppSettings(next);
    } catch (e: any) {
      pushToast("保存失败", e?.toString(), "error");
    }
  };

  const queueSaveApp = (next: AppSettings) => {
    setAppSettings(next);
    if (pendingSave.current) clearTimeout(pendingSave.current);
    pendingSave.current = window.setTimeout(() => persistApp(next), 600);
  };

  const saveAppNow = (next: AppSettings) => {
    setAppSettings(next);
    if (pendingSave.current) {
      clearTimeout(pendingSave.current);
      pendingSave.current = null;
    }
    persistApp(next);
  };

  const detectOne = async (key: "trae" | "workbuddy" | "codebuddy_cn") => {
    setDetecting(key);
    try {
      const paths = await api.detectClientPaths();
      const found = paths[key];
      if (found && appSettings) {
        const field =
          key === "trae"
            ? "client_path_trae"
            : key === "workbuddy"
            ? "client_path_workbuddy"
            : "client_path_codebuddy_cn";
        const next = { ...appSettings, [field]: found };
        saveAppNow(next);
        pushToast("获取成功", found, "success");
      } else {
        pushToast("未找到", "未检测到该客户端，可能未安装", "info");
      }
    } catch (e: any) {
      pushToast("获取失败", e?.toString(), "error");
    } finally {
      setDetecting(null);
    }
  };

  if (loading || !appSettings) {
    return (
      <div className="flex items-center justify-center h-full">
        <RefreshCw className="h-8 w-8 animate-spin text-muted-foreground" />
      </div>
    );
  }

  const notifOptions = [
    { value: "toast", label: "应用内 Toast" },
    { value: "system", label: "系统通知" },
    { value: "both", label: "系统通知 + Toast" },
    { value: "none", label: "不通知" },
  ];

  return (
    <div className="p-6 space-y-6 max-w-2xl">
      <div>
        <h1 className="text-2xl font-bold">设置</h1>
        <p className="text-sm text-muted-foreground">配置通用、客户端、通知与启动行为，修改后自动保存</p>
      </div>

      {/* 通用设置 */}
      <Card>
        <CardHeader>
          <CardTitle className="text-base flex items-center gap-2">
            <SlidersHorizontal className="h-4 w-4" /> 通用
          </CardTitle>
          <CardDescription>外观主题与日志保留时间</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div>
            <label className="text-sm font-medium">外观</label>
            <select
              value={theme}
              onChange={(e) => changeTheme(e.target.value)}
              className="mt-1 flex h-10 w-full rounded-md border border-input bg-background px-3 py-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 ring-offset-background"
            >
              <option value="light">浅色</option>
              <option value="dark">深色</option>
              <option value="system">跟随系统</option>
            </select>
          </div>
          <div>
            <label className="text-sm font-medium">日志保存时间（天）</label>
            <div className="flex gap-2 mt-1">
              <Input
                type="number"
                min={1}
                value={appSettings.log_retention_days}
                onChange={(e) =>
                  setAppSettings({
                    ...appSettings,
                    log_retention_days: Math.max(1, Number(e.target.value) || 1),
                  })
                }
              />
              <Button variant="outline" className="shrink-0" onClick={saveLogSettings}>
                <Save className="h-4 w-4 mr-2" /> 保存
              </Button>
            </div>
          </div>
        </CardContent>
      </Card>

      {/* 客户端地址 */}
      <Card>
        <CardHeader>
          <CardTitle className="text-base flex items-center gap-2">
            <Monitor className="h-4 w-4" /> 客户端地址
          </CardTitle>
          <CardDescription>
            首次打开应用时已自动探测；未检测到即未安装，可留空
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {(
            [
              { key: "trae", label: "TRAE", field: "client_path_trae" as const },
              { key: "workbuddy", label: "WorkBuddy", field: "client_path_workbuddy" as const },
              { key: "codebuddy_cn", label: "CodeBuddy CN", field: "client_path_codebuddy_cn" as const },
            ]
          ).map((c) => (
            <div key={c.key}>
              <label className="text-sm font-medium">{c.label}</label>
              <div className="flex gap-2 mt-1">
                <Input
                  value={appSettings[c.field]}
                  placeholder="未安装或手动输入路径"
                  onChange={(e) =>
                    queueSaveApp({ ...appSettings, [c.field]: e.target.value })
                  }
                />
                <Button
                  variant="outline"
                  size="sm"
                  className="shrink-0"
                  disabled={detecting !== null}
                  onClick={() => detectOne(c.key as "trae" | "workbuddy" | "codebuddy_cn")}
                >
                  {detecting === c.key ? (
                    <RefreshCw className="h-4 w-4 mr-1 animate-spin" />
                  ) : (
                    <FolderSearch className="h-4 w-4 mr-1" />
                  )}
                  自动获取
                </Button>
              </div>
            </div>
          ))}
        </CardContent>
      </Card>

      {/* 通知设置 */}
      <Card>
        <CardHeader>
          <CardTitle className="text-base flex items-center gap-2">
            <Bell className="h-4 w-4" /> 通知方式
          </CardTitle>
          <CardDescription>签到完成、续期等事件的通知方式</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="grid grid-cols-2 gap-3">
            {notifOptions.map((opt) => (
              <button
                key={opt.value}
                onClick={() => saveNotif(opt.value)}
                className={`rounded-lg border p-3 text-left text-sm font-medium transition-colors ${
                  notifMode === opt.value
                    ? "border-primary bg-accent"
                    : "hover:bg-accent/50"
                }`}
              >
                {opt.label}
              </button>
            ))}
          </div>
        </CardContent>
      </Card>

      {/* 启动设置 */}
      <Card>
        <CardHeader>
          <CardTitle className="text-base flex items-center gap-2">
            <Power className="h-4 w-4" /> 启动行为
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-center justify-between">
            <div>
              <p className="font-medium">开机启动</p>
              <p className="text-sm text-muted-foreground">系统启动时自动运行</p>
            </div>
            <Switch checked={autoStart} onCheckedChange={() => toggleAutoStart(!autoStart)} />
          </div>
          <div className="flex items-center justify-between">
            <div>
              <p className="font-medium">启动时最小化到托盘</p>
              <p className="text-sm text-muted-foreground">应用启动后不显示主窗口</p>
            </div>
            <Switch
              checked={appSettings.minimize_to_tray_on_start}
              onCheckedChange={async () => {
                const next = {
                  ...appSettings,
                  minimize_to_tray_on_start: !appSettings.minimize_to_tray_on_start,
                };
                setAppSettings(next);
                try {
                  await api.setAppSettings(next);
                  pushToast("已保存", "启动行为已更新", "success");
                } catch (e: any) {
                  pushToast("保存失败", e?.toString(), "error");
                }
              }}
            />
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
