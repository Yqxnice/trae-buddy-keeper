import { useCallback, useEffect, useRef, useState } from "react";
import { RefreshCw, FolderSearch, FolderOpen, Download, RotateCcw, HardDriveDownload, HardDriveUpload } from "lucide-react";
import { PageHeader } from "@/components/PageHeader/PageHeader";
import { Button } from "@/components/Button/Button";
import { Switch } from "@/components/Switch/Switch";
import { Select } from "@/components/Select/Select";
import { Segmented } from "@/components/Segmented/Segmented";
import { TextInput, NumInput } from "@/components/TextInput/TextInput";
import { Meter } from "@/components/Meter/Meter";
import { api, type AppSettings } from "@/lib/api";
import { useCheckinStore } from "@/stores/checkin";
import { updater, type UpdatePhase } from "@/lib/updater";
import { applyTheme, normalizeTheme } from "@/lib/theme";
import { enable, disable, isEnabled } from "@tauri-apps/plugin-autostart";
import { useUIStore } from "@/stores/ui";
import { platformLabel } from "@/lib/utils";
import "./Settings.css";

/** 定时签到可选的平台范围（CodeBuddy CN 与 WorkBuddy 同账号体系，随 WorkBuddy 一起签到） */
const SCHED_PLATFORM_KEYS = ["trae", "workbuddy"] as const;

export function SettingsPage() {
  const pushToast = useUIStore((s) => s.pushToast);
  const [appSettings, setAppSettings] = useState<AppSettings | null>(null);
  const [notifMode, setNotifMode] = useState("both");
  const [autoStart, setAutoStart] = useState(false);
  const [theme, setTheme] = useState("light");
  const [detecting, setDetecting] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [dataDir, setDataDir] = useState("");

  // —— 自动更新状态机：idle → checking → available → downloading → ready ——
  const [currentVer, setCurrentVer] = useState("");
  const [phase, setPhase] = useState<UpdatePhase>("idle");
  const [newVer, setNewVer] = useState("");
  const [progress, setProgress] = useState(0);
  const [errMsg, setErrMsg] = useState("");

  useEffect(() => {
    updater.currentVersion().then(setCurrentVer).catch(() => {});
  }, []);

  const checkUpdate = async () => {
    setPhase("checking");
    setErrMsg("");
    try {
      const v = await updater.check();
      if (v) {
        setNewVer(v);
        setPhase("available");
      } else {
        setPhase("latest");
      }
    } catch (e: any) {
      setErrMsg(`检查失败：${e?.toString() || "未知错误"}`);
      setPhase("error");
    }
  };

  const installUpdate = async () => {
    setProgress(0);
    setPhase("downloading");
    try {
      await updater.downloadAndInstall(setProgress);
      setPhase("ready");
    } catch (e: any) {
      setErrMsg(`下载失败：${e?.toString() || "未知错误"}`);
      setPhase("error");
    }
  };

  const relaunchApp = async () => {
    try {
      await updater.relaunch();
    } catch (e: any) {
      pushToast("重启失败", e?.toString(), "error");
    }
  };

  // —— 定时签到平台范围（checkin store，App 启动时已加载）——
  const schedPlatforms = useCheckinStore((s) => s.settings?.platforms);
  const updateCheckin = useCheckinStore((s) => s.update);
  const saveCheckin = useCheckinStore((s) => s.save);
  // 空数组语义为「全部平台」，UI 上按全选展示，任何勾选变化都会落为显式列表
  const scopeCurrent =
    schedPlatforms && schedPlatforms.length > 0 ? schedPlatforms : SCHED_PLATFORM_KEYS;

  const toggleSchedPlatform = async (key: string, on: boolean) => {
    if (!on && scopeCurrent.length <= 1) {
      pushToast("无法关闭", "至少保留一个平台参与定时签到", "warning");
      return;
    }
    const next = on
      ? Array.from(new Set([...scopeCurrent, key]))
      : scopeCurrent.filter((p) => p !== key);
    updateCheckin({ platforms: next });
    try {
      await saveCheckin();
      pushToast("已保存", "定时签到平台范围已更新", "success");
    } catch (e: any) {
      pushToast("保存失败", e?.toString(), "error");
    }
  };

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [m, a] = await Promise.all([
        api.getNotificationMode(),
        api.getAppSettings(),
      ]);
      setNotifMode(m);
      setAppSettings(a);
      setTheme(normalizeTheme(a.theme));
      api.getDataDir().then(setDataDir).catch(() => {});
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
  }, [pushToast]);

  useEffect(() => {
    load();
  }, [load]);

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

  // 在系统资源管理器中打开数据目录
  const openDataFolder = async () => {
    try {
      await api.openDataDir();
    } catch (e: any) {
      pushToast("打开失败", e?.toString(), "error");
    }
  };

  // —— 数据备份与恢复：弹窗选择位置由后端原生对话框完成，返回 null 表示取消 ——
  const [backupBusy, setBackupBusy] = useState(false);
  const runDataOp = async (op: () => Promise<string | null>, doneLabel: string) => {
    setBackupBusy(true);
    try {
      const note = await op();
      if (note) pushToast(doneLabel, note, "success");
    } catch (e: any) {
      pushToast("操作失败", e?.toString(), "error");
    } finally {
      setBackupBusy(false);
    }
  };
  const doBackup = () => runDataOp(api.backupData, "备份完成");
  const doImport = () => runDataOp(api.importData, "导入完成");

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

  const detectOne = async (
    key: "trae" | "workbuddy" | "workbuddy_ai" | "codebuddy_cn"
  ) => {
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
            : key === "workbuddy_ai"
            ? "client_path_workbuddy_ai"
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

      <div className="gtitle">定时签到</div>
      <div className="set-card">
        <div className="set-row">
          <div className="set-info">
            <div className="set-name">平台范围</div>
            <div className="set-desc">仅勾选的平台参与每日自动签到与超时补签</div>
          </div>
        </div>
        {SCHED_PLATFORM_KEYS.map((key) => (
          <div className="set-row" key={key}>
            <div className="set-info">
              <div className="set-name">{platformLabel(key)}</div>
              <div className="set-desc">
                {key === "workbuddy"
                  ? "该平台账号参与定时签到（含 CodeBuddy CN）"
                  : "该平台账号是否参与定时自动签到"}
              </div>
            </div>
            <Switch
              checked={scopeCurrent.includes(key)}
              disabled={schedPlatforms === undefined}
              onChange={(v) => toggleSchedPlatform(key, v)}
            />
          </div>
        ))}
      </div>

      <div className="gtitle">客户端路径</div>
      <div className="set-card">
        {(
          [
            { key: "trae", label: "TRAE", field: "client_path_trae" as const },
            { key: "workbuddy", label: "WorkBuddy", field: "client_path_workbuddy" as const },
            {
              key: "workbuddy_ai",
              label: "WorkBuddy AI（国际版）",
              field: "client_path_workbuddy_ai" as const,
            },
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
                onClick={() =>
                  detectOne(c.key as "trae" | "workbuddy" | "workbuddy_ai" | "codebuddy_cn")
                }
              >
                <FolderSearch />
                自动获取
              </Button>
            </div>
          </div>
        ))}
      </div>

      <div className="gtitle">备份与恢复</div>
      <div className="set-card">
        <div className="set-row">
          <div className="set-info">
            <div className="set-name">备份数据</div>
            <div className="set-desc">导出数据库与授权会话，弹窗选择保存位置</div>
          </div>
          <Button disabled={backupBusy} onClick={doBackup}>
            <HardDriveDownload />
            备份数据
          </Button>
        </div>
        <div className="set-row">
          <div className="set-info">
            <div className="set-name">导入数据</div>
            <div className="set-desc">从备份文件夹恢复并覆盖现有数据，建议恢复后重启</div>
          </div>
          <Button disabled={backupBusy} onClick={doImport}>
            <HardDriveUpload />
            导入数据
          </Button>
        </div>
      </div>

      <div className="gtitle">关于与更新</div>
      <div className="set-card">
        <div className="set-row">
          <div className="set-info">
            <div className="set-name">存储位置</div>
            <div className="set-desc data-dir">{dataDir || "…"}</div>
          </div>
          <Button onClick={openDataFolder}>
            <FolderOpen />
            打开文件夹
          </Button>
        </div>
        <div className="set-row">
          <div className="set-info">
            <div className="set-name">
              当前版本{currentVer ? ` v${currentVer}` : ""}
              {phase === "available" ? ` · 发现新版本 v${newVer}` : ""}
            </div>
            <div className="set-desc">
              {phase === "idle" && "应用启动时会自动检查更新，也可在此手动检查"}
              {phase === "checking" && "正在检查更新…"}
              {phase === "available" && "下载完成后重启应用即可完成升级"}
              {phase === "downloading" && `正在下载更新… ${progress}%`}
              {phase === "ready" && "更新包已安装，重启应用后生效"}
              {phase === "latest" && "当前已是最新版本"}
              {phase === "error" && errMsg}
            </div>
            {phase === "downloading" && <Meter value={progress} small />}
          </div>
          {phase === "ready" ? (
            <Button variant="pri" onClick={relaunchApp}>
              <RotateCcw />
              重启应用
            </Button>
          ) : phase === "available" ? (
            <Button variant="pri" onClick={installUpdate}>
              <Download />
              下载并安装
            </Button>
          ) : (
            <Button
              disabled={phase === "checking" || phase === "downloading"}
              onClick={checkUpdate}
            >
              <RefreshCw className={phase === "checking" ? "spin" : ""} />
              检查更新
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}
