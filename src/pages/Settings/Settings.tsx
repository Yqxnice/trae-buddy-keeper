import { useEffect, useRef, useState } from "react";
import { RefreshCw, FolderSearch } from "lucide-react";
import { PageHeader } from "@/components/PageHeader/PageHeader";
import { Button } from "@/components/Button/Button";
import { Switch } from "@/components/Switch/Switch";
import { Select } from "@/components/Select/Select";
import { Segmented } from "@/components/Segmented/Segmented";
import { TextInput, NumInput } from "@/components/TextInput/TextInput";
import { api, type AppSettings } from "@/lib/api";
import { applyTheme, normalizeTheme } from "@/lib/theme";
import { enable, disable, isEnabled } from "@tauri-apps/plugin-autostart";
import { useUIStore } from "@/stores/ui";
import "./Settings.css";

export function SettingsPage() {
  const pushToast = useUIStore((s) => s.pushToast);
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
      <div className="page-loading">
        <RefreshCw className="spin" size={26} strokeWidth={1.6} />
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
    <div className="settings">
      <PageHeader
        title="设置"
        sub="除「日志保留天数」需手动保存外，其余设置修改后自动保存"
      />

      <div className="gtitle first">外观</div>
      <div className="set-card">
        <div className="set-row">
          <div className="set-info">
            <div className="set-name">外观主题</div>
            <div className="set-desc">跟随系统时实时同步 Windows 深浅色</div>
          </div>
          <Select
            value={theme}
            onChange={changeTheme}
            options={[
              { value: "light", label: "浅色" },
              { value: "dark", label: "深色" },
              { value: "system", label: "跟随系统" },
            ]}
          />
        </div>
        <div className="set-row">
          <div className="set-info">
            <div className="set-name">日志保留天数</div>
            <div className="set-desc">超出后自动清理，点击「保存」生效</div>
          </div>
          <div className="numrow">
            <NumInput
              min={1}
              value={appSettings.log_retention_days}
              onChange={(e) =>
                setAppSettings({
                  ...appSettings,
                  log_retention_days: Math.max(1, Number(e.target.value) || 1),
                })
              }
            />
            <span className="tcap">天</span>
            <Button variant="pri" onClick={saveLogSettings}>
              保存
            </Button>
          </div>
        </div>
      </div>

      <div className="gtitle">通知</div>
      <div className="set-card">
        <div className="set-row">
          <div className="set-info">
            <div className="set-name">通知方式</div>
            <div className="set-desc">签到完成、续期等事件的通知方式</div>
          </div>
          <Segmented
            options={notifOptions}
            value={notifMode}
            onChange={saveNotif}
          />
        </div>
      </div>

      <div className="gtitle">启动</div>
      <div className="set-card">
        <div className="set-row">
          <div className="set-info">
            <div className="set-name">开机启动</div>
            <div className="set-desc">系统启动时自动运行</div>
          </div>
          <Switch checked={autoStart} onChange={() => toggleAutoStart(!autoStart)} />
        </div>
        <div className="set-row">
          <div className="set-info">
            <div className="set-name">启动时最小化到托盘</div>
            <div className="set-desc">应用启动后不显示主窗口</div>
          </div>
          <Switch
            checked={appSettings.minimize_to_tray_on_start}
            onChange={async () => {
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
      </div>

      <div className="gtitle">客户端路径</div>
      <div className="set-card">
        {(
          [
            { key: "trae", label: "TRAE", field: "client_path_trae" as const },
            { key: "workbuddy", label: "WorkBuddy", field: "client_path_workbuddy" as const },
            { key: "codebuddy_cn", label: "CodeBuddy CN", field: "client_path_codebuddy_cn" as const },
          ]
        ).map((c) => (
          <div className="set-row" key={c.key}>
            <div className="set-info">
              <div className="set-name">{c.label}</div>
              <div className="set-desc">未检测到即未安装，可留空或手动输入</div>
            </div>
            <div className="path-row">
              <TextInput
                className="mono"
                value={appSettings[c.field]}
                placeholder="未安装或手动输入路径"
                onChange={(e) => queueSaveApp({ ...appSettings, [c.field]: e.target.value })}
              />
              <Button
                disabled={detecting !== null}
                onClick={() => detectOne(c.key as "trae" | "workbuddy" | "codebuddy_cn")}
              >
                <FolderSearch />
                自动获取
              </Button>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
