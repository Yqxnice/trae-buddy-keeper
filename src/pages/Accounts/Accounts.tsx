import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  Play,
  RefreshCw,
  RotateCcw,
  Globe,
} from "lucide-react";
import { PageHeader } from "@/components/PageHeader/PageHeader";
import { Button } from "@/components/Button/Button";
import { Chip } from "@/components/Chip/Chip";
import { Avatar } from "@/components/Avatar/Avatar";
import { LedgerRow, RowAction } from "@/components/LedgerRow/LedgerRow";
import { RowMenu, type MenuItem } from "@/components/RowMenu/RowMenu";
import { Dialog } from "@/components/Dialog/Dialog";
import { TextInput, TextArea } from "@/components/TextInput/TextInput";
import { api, type AccountInfo, type TraeVaultInfo, type WbVariant } from "@/lib/api";
import traeLogo from "@/assets/logos/trae-color.svg";
import workbuddyLogo from "@/assets/logos/workbuddy-color.svg";
import codebuddyLogo from "@/assets/logos/codebuddy-color.svg";
import qoderLogo from "@/assets/logos/qoder-color.svg";
import { confirmDialog, confirmDialogChecked } from "@/components/ConfirmDialog/ConfirmDialog";
import { formatDate, formatRelative, platformLabel, checkinChipInfo, checkinDayStart } from "@/lib/utils";
import { useUIStore } from "@/stores/ui";
import "./Accounts.css";

// 客户端切换器素材：卡片次行 SelectorBar（图标 + 名称，点击即切换）
const CLIENT_APPS: Record<string, { src: string; label: string; short: string }> = {
  trae: { src: traeLogo, label: "Trae 客户端", short: "Trae" },
  workbuddy: { src: workbuddyLogo, label: "WorkBuddy 客户端", short: "WorkBuddy" },
  codebuddy_cn: { src: codebuddyLogo, label: "CodeBuddy CN 客户端", short: "CodeBuddy CN" },
  qoder_cn: { src: qoderLogo, label: "Qoder CN 客户端", short: "Qoder CN" },
};

// 操作门控：签到幂等跳过由前后端共同守住；手动续期不设到期窗口，仅后端保留 5 分钟限频
function checkinBlocked(acc: AccountInfo): "cooling" | "signed" | "expired" | null {
  if (acc.cooldown_until > Date.now() / 1000) return "cooling";
  if (acc.checkin_status === "success" && acc.last_checkin_time >= checkinDayStart()) return "signed";
  // 凭证已失效：后端签到必跳过（checkin.rs 防线 #2），引导先续期
  if (tokenExpired(acc)) return "expired";
  return null;
}
function tokenExpired(acc: AccountInfo): boolean {
  return acc.token_expires_at > 0 && acc.token_expires_at < Date.now() / 1000;
}
export function AccountsPage() {
  const platform = useUIStore((s) => s.platform);
  const pushToast = useUIStore((s) => s.pushToast);
  const [accounts, setAccounts] = useState<AccountInfo[]>([]);
  const [loading, setLoading] = useState(true);
  // 当前时间戳（秒）：列表渲染中用于到期/冷却的相对展示，挂载时取一次即可
  const [nowSec, setNowSec] = useState(() => Date.now() / 1000);
  const [addOpen, setAddOpen] = useState(false);
  const [batchRunning, setBatchRunning] = useState(false);
  const [renewRunning, setRenewRunning] = useState(false);
  const [restoring, setRestoring] = useState(false);

  const [checkingId, setCheckingId] = useState<string | null>(null);
  const [switchingId, setSwitchingId] = useState<string | null>(null);
  const [switchingCnId, setSwitchingCnId] = useState<string | null>(null);
  const [archivingId, setArchivingId] = useState<string | null>(null);
  const [creditLoadingId, setCreditLoadingId] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  // Trae 档案验证状态：切换未确认登录时后端打标，列表显示「档案待验证」徽标
  const [vaultFlags, setVaultFlags] = useState<Record<string, TraeVaultInfo | null>>({});

  // 编辑弹窗
  const [editOpen, setEditOpen] = useState(false);
  const [editSaving, setEditSaving] = useState(false);
  const [editForm, setEditForm] = useState({
    id: "",
    name: "",
    email: "",
    access_token: "",
    refresh_token: "",
    cookies: "",
  });

  // OAuth 授权登录状态（TRAE）
  const [oauthWaiting, setOauthWaiting] = useState(false);
  const [oauthCallback, setOauthCallback] = useState("");
  const [oauthName, setOauthName] = useState("");

  // WorkBuddy 网页登录状态（state 轮询）；wbVariant 为登录目标档位
  const [wbWaiting, setWbWaiting] = useState(false);
  const [wbLoginState, setWbLoginState] = useState("");
  const [wbVariant, setWbVariant] = useState<WbVariant>("cn");

  // Qoder CN 设备流登录状态；compat 为授权页兼容模式
  const [qoderWaiting, setQoderWaiting] = useState(false);
  const [qoderLoginState, setQoderLoginState] = useState("");
  const [qoderCompat, setQoderCompat] = useState(false);

  // 全屏 loading 仅首次挂载生效；后续刷新原地更新列表，避免整页拆建抖动
  // notify=true（手动刷新）时成功/失败均 toast；操作后的自动重载保持静默
  const refreshVaultFlags = useCallback(
    async (list: AccountInfo[]) => {
      if (platform !== "trae" && platform !== "qoder_cn") return;
      const isQoder = platform === "qoder_cn";
      const entries = await Promise.all(
        list
          .filter((a) => a.platform === platform)
          .map(async (a) =>
            [
              a.id,
              await (isQoder
                ? api.qoderVaultInfo(a.id)
                : api.traeVaultInfo(a.id)
              ).catch(() => null),
            ] as const
          )
      );
      const next: Record<string, TraeVaultInfo | null> = {};
      for (const [id, info] of entries) next[id] = info;
      setVaultFlags(next);
    },
    [platform]
  );

  const load = useCallback(
    async (notify = false) => {
      if (notify) setRefreshing(true);
      try {
        const list = await api.listAccounts();
        setAccounts(list);
        void refreshVaultFlags(list);
        setNowSec(Date.now() / 1000);
        if (notify) pushToast("刷新成功", "账号列表已更新", "success");
        // 后台刷新过期的真实积分（60s 内查过则跳过），完成后静默更新列表
        const now = Date.now() / 1000;
        const stale = list.some(
          (a) => a.platform === platform && now - (a.credits_fetched_at || 0) > 60
        );
        if (stale) {
          api
            .refreshAllCredits(platform)
            .then(() => api.listAccounts())
            .then(setAccounts)
            .catch(() => {});
        }
      } catch (e: any) {
        pushToast(notify ? "刷新失败" : "加载失败", e?.toString(), "error");
      } finally {
        setLoading(false);
        if (notify) setRefreshing(false);
      }
    },
    [platform, pushToast, refreshVaultFlags]
  );

  useEffect(() => {
    load();
  }, [load]);

  const filtered = accounts.filter((a) => a.platform === platform);

  // 唯一添加方式：网页授权登录；打开弹窗时清理旧等待态
  const openAdd = () => {
    if (oauthWaiting) handleCancelOauth();
    setWbWaiting(false);
    setWbLoginState("");
    if (qoderWaiting && qoderLoginState) void api.stopQoderLogin(qoderLoginState);
    setQoderWaiting(false);
    setQoderLoginState("");
    setAddOpen(true);
  };

  const finishOauth = useCallback(
    async (callbackUrl: string, name?: string) => {
      try {
        const msg = await api.completeTraeOauth(callbackUrl, name);
        pushToast("授权成功", msg, "success");
        setOauthWaiting(false);
        setOauthCallback("");
        setOauthName("");
        setAddOpen(false);
        load();
      } catch (e: any) {
        pushToast("授权失败", e?.toString(), "error");
      }
    },
    [pushToast, load]
  );

  // 授权回调事件：浏览器完成授权后自动完成登录
  useEffect(() => {
    const unlisten = listen<string>("oauth-callback", (e) => {
      finishOauth(e.payload, oauthName || undefined);
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [oauthName, finishOauth]);

  const handleTraeOauth = async () => {
    // 乐观切换为等待状态，服务器启动与浏览器打开在后台进行
    setOauthWaiting(true);
    try {
      await api.startTraeOauth();
      pushToast("提示", "已在浏览器打开 TRAE 授权页，完成后将自动登录", "info");
    } catch (e: any) {
      setOauthWaiting(false);
      pushToast("启动授权失败", e?.toString(), "error");
    }
  };

  const handleCancelOauth = async () => {
    setOauthWaiting(false);
    setOauthCallback("");
    try {
      await api.stopTraeOauth();
    } catch {
      // 忽略
    }
  };

  const closeAddDialog = () => {
    if (oauthWaiting) {
      handleCancelOauth();
    }
    setWbWaiting(false);
    setWbLoginState("");
    if (qoderWaiting && qoderLoginState) void api.stopQoderLogin(qoderLoginState);
    setQoderWaiting(false);
    setQoderLoginState("");
    setAddOpen(false);
  };

  const handleWorkbuddyLogin = async () => {
    try {
      const r = await api.startWorkbuddyLogin(wbVariant);
      setWbLoginState(r.state);
      setWbWaiting(true);
      pushToast(
        "提示",
        wbVariant === "ai"
          ? "已在浏览器打开 WorkBuddy 国际版登录页，完成后将自动添加"
          : "已在浏览器打开 WorkBuddy 登录页，完成后将自动添加",
        "info"
      );
    } catch (e: any) {
      pushToast("启动登录失败", e?.toString(), "error");
    }
  };

  // WorkBuddy 登录轮询：每 2 秒查询一次，成功即入库并关闭弹窗
  useEffect(() => {
    if (!wbWaiting || !wbLoginState) return;
    const timer = setInterval(async () => {
      try {
        const r = await api.pollWorkbuddyLogin(wbLoginState, wbVariant);
        if (r.status === "ready") {
          setWbWaiting(false);
          setWbLoginState("");
          pushToast("登录成功", `已添加账户 ${r.name}`, "success");
          setAddOpen(false);
          load();
        }
      } catch (e: any) {
        setWbWaiting(false);
        setWbLoginState("");
        pushToast("登录失败", e?.toString(), "error");
      }
    }, 2000);
    return () => clearInterval(timer);
  }, [wbWaiting, wbLoginState, wbVariant, load, pushToast]);

  const handleQoderLogin = async () => {
    try {
      const r = await api.startQoderLogin(qoderCompat);
      setQoderLoginState(r.state);
      setQoderWaiting(true);
      pushToast("提示", "已在浏览器打开 Qoder CN 登录页，完成后将自动添加", "info");
    } catch (e: any) {
      pushToast("启动登录失败", e?.toString(), "error");
    }
  };

  // Qoder CN 设备流轮询：每 2 秒查询一次授权结果，成功即入库并关闭弹窗
  useEffect(() => {
    if (!qoderWaiting || !qoderLoginState) return;
    const timer = setInterval(async () => {
      try {
        const r = await api.pollQoderLogin(qoderLoginState);
        if (r.status === "ready") {
          setQoderWaiting(false);
          setQoderLoginState("");
          pushToast("登录成功", `已添加账户 ${r.name}`, "success");
          setAddOpen(false);
          load();
        }
      } catch (e: any) {
        setQoderWaiting(false);
        setQoderLoginState("");
        pushToast("登录失败", e?.toString(), "error");
      }
    }, 2000);
    return () => clearInterval(timer);
  }, [qoderWaiting, qoderLoginState, load, pushToast]);

  /** 账号是否为 WorkBuddy 国际版（domain 后缀判定，与后端 variant_of 同口径） */
  const isAiAccount = (acc: AccountInfo) =>
    !!acc.domain && acc.domain.trim().toLowerCase().endsWith(".workbuddy.ai");

  const handleCheckin = async (id: string) => {
    setCheckingId(id);
    try {
      const r = await api.checkinOne(id);
      pushToast(
        r.success ? "签到成功" : "签到失败",
        r.message,
        r.success ? "success" : "error"
      );
      load();
      useUIStore.getState().bumpCheckin();
    } catch (e: any) {
      pushToast("签到失败", e?.toString(), "error");
    } finally {
      setCheckingId(null);
    }
  };

  const handleRenew = async (id: string) => {
    try {
      const msg = await api.renewToken(id);
      pushToast("续期结果", msg, "success");
      load();
    } catch (e: any) {
      pushToast("续期失败", e?.toString(), "error");
    }
  };

  const openEdit = async (id: string) => {
    try {
      const acc = await api.getAccountDetail(id);
      if (!acc) {
        pushToast("编辑失败", "账户不存在", "error");
        return;
      }
      setEditForm({
        id: acc.id,
        name: acc.name,
        email: acc.email,
        access_token: acc.access_token,
        refresh_token: acc.refresh_token,
        cookies: acc.cookies,
      });
      setEditOpen(true);
    } catch (e: any) {
      pushToast("加载失败", e?.toString(), "error");
    }
  };

  const handleEditSave = async () => {
    setEditSaving(true);
    try {
      await api.updateAccount(editForm);
      pushToast("已保存", `账户「${editForm.name}」已更新`, "success");
      setEditOpen(false);
      load();
    } catch (e: any) {
      pushToast("保存失败", e?.toString(), "error");
    } finally {
      setEditSaving(false);
    }
  };

  const handleDelete = async (id: string) => {
    const [ok, keepBackup] = await confirmDialogChecked({
      title: "删除账户",
      message:
        "确定删除该账户？相关签到记录、积分记录会被清除。\n该账户保存的客户端登录档案也会一并清理（Trae / Qoder CN）。",
      checkLabel: "保留登录档案备份（重命名为 .bak，可手动恢复）",
      checkDefault: true,
      confirmText: "删除",
      danger: true,
    });
    if (!ok) return;
    try {
      const msg = await api.deleteAccount(id, keepBackup);
      pushToast("账户已删除", msg, "success");
      load();
    } catch (e: any) {
      pushToast("删除失败", e?.toString(), "error");
    }
  };

  const handleSwitch = async (id: string) => {
    // 真实切换客户端登录账号（会关闭并重启客户端，耗时较长）
    if (platform === "workbuddy" || platform === "trae" || platform === "qoder_cn") {
      setSwitchingId(id);
      try {
        // 运行中切库会写花客户端数据，先检测并明确告知会自动关闭客户端
        if (platform === "trae" && (await api.traeClientRunning())) {
          pushToast("正在关闭客户端", "检测到 Trae CN 正在运行，切换前会先关闭它", "info");
        }
        if (platform === "qoder_cn" && (await api.qoderClientRunning())) {
          pushToast("正在关闭客户端", "检测到 Qoder CN 正在运行，切换前会先关闭它", "info");
        }
        const msg =
          platform === "workbuddy"
            ? await api.switchWorkbuddyClient(id)
            : platform === "qoder_cn"
              ? await api.switchQoderClient(id)
              : await api.switchTraeClient(id);
        // 后端「不确认 ≠ 失败」：保留客户端现状并打标，这里提供一键重建档引导
        if (
          (platform === "trae" || platform === "qoder_cn") &&
          msg.startsWith("SWITCH_WARN:")
        ) {
          const body = msg.slice("SWITCH_WARN:".length);
          const client = platformLabel(platform);
          pushToast("切换结果待确认", body, "warning");
          const ok = await confirmDialog({
            title: "现在重新建档？",
            message: `本次切换未能确认客户端已登录到目标账号。\n\n${body}\n\n客户端未回滚，你可以先在 ${client} 里登录该账号，再点「重新建档」把新的登录态保存进档案（建档会先关闭客户端）。若想回到切换前，可点「恢复切换前现场」。`,
            confirmText: "重新建档",
          });
          if (ok) await handleArchive(id, true);
          void load();
          return;
        }
        pushToast("切换成功", msg, "success");
        void load();
      } catch (e: any) {
        pushToast("切换失败", e?.toString(), "error");
      } finally {
        setSwitchingId(null);
      }
      return;
    }
    try {
      await api.switchAccount(id);
      pushToast("已切换", "当前活跃账户已更新", "success");
    } catch (e: any) {
      pushToast("切换失败", e?.toString(), "error");
    }
  };

  // WorkBuddy 账号也可切到 CodeBuddy CN（同一账号体系，注入 IDE 登录态后重启客户端）
  const handleSwitchCn = async (id: string) => {
    setSwitchingCnId(id);
    try {
      const msg = await api.switchCodebuddyCnClient(id);
      pushToast("切换成功", msg, "success");
    } catch (e: any) {
      pushToast("切换失败", e?.toString(), "error");
    } finally {
      setSwitchingCnId(null);
    }
  };

  // Qoder CN：把上一次切换前的现场写回客户端（人工兜底回退）
  const handleRestoreLast = async () => {
    const ok = await confirmDialog({
      title: "恢复切换前现场",
      message:
        "这会关闭 Qoder CN 客户端，把上一次切换前的登录现场写回并重启。\n\n仅在上次切换结果不对时使用；当前客户端里的登录账号会被覆盖。",
      confirmText: "恢复并重启",
    });
    if (!ok) return;
    setRestoring(true);
    try {
      const msg = await api.restoreQoderLast();
      pushToast("恢复成功", msg, "success");
      void load();
    } catch (e: any) {
      pushToast("恢复失败", e?.toString(), "error");
    } finally {
      setRestoring(false);
    }
  };

  // Trae / Qoder CN：把当前客户端登录态快照到该账号（建档），切换的前提
  const handleArchive = async (id: string, closeRunning = false) => {
    setArchivingId(id);
    try {
      const msg =
        platform === "qoder_cn"
          ? await api.backupQoderVault(id, closeRunning)
          : await api.backupTraeVault(id, closeRunning);
      pushToast("建档成功", msg, "success");
      // 重新建档会清掉「待验证」标记，刷新徽标状态
      void load();
    } catch (e: any) {
      const msg = String(e ?? "");
      // 客户端仍在后台运行：征得用户确认后自动关闭（温和→强杀）再建档
      if (msg.startsWith("CLIENT_RUNNING:") && !closeRunning) {
        const ok = await confirmDialog({
          title: "客户端仍在运行",
          message:
            `检测到 ${platformLabel(platform)} 客户端仍在后台运行（点窗口 × 可能只是驻留后台，并未退出进程）。\n\n注意：关闭会退出全部 ${platformLabel(platform)} 窗口——如果你正在改代码，未保存的内容可能丢失，建议先保存或改完再建档。\n\n是否关闭客户端并继续建档？`,
          confirmText: "关闭并建档",
        });
        setArchivingId(null);
        if (ok) await handleArchive(id, true);
        return;
      }
      pushToast(
        "建档失败",
        msg.replace("CLIENT_RUNNING:", ""),
        "error",
      );
    } finally {
      setArchivingId(null);
    }
  };

  // 客户端图标即切换入口：按产品决定常驻哪几个应用图标
  const handleAppSwitch = (client: string, id: string) => {
    if (client === "codebuddy_cn") handleSwitchCn(id);
    else handleSwitch(id);
  };

  const handleCheckinAll = async () => {
    setBatchRunning(true);
    try {
      const results = await api.checkinAll(platform);
      const ok = results.filter((r) => r.success).length;
      pushToast(
        "批量签到完成",
        `成功 ${ok} 个，失败 ${results.length - ok} 个`,
        ok === results.length ? "success" : "info"
      );
      setAccounts(await api.listAccounts());
      useUIStore.getState().bumpCheckin();
    } catch (e: any) {
      pushToast("批量签到失败", e?.toString(), "error");
    } finally {
      setBatchRunning(false);
    }
  };

  // 一键续期：手动批量刷新全部账号，不受自动巡检的 24h 窗口限制
  const handleRenewAll = async () => {
    setRenewRunning(true);
    try {
      const msgs = await api.autoRenewAll();
      const fails = msgs.filter((m) => m.includes("续期失败"));
      pushToast(
        "一键续期完成",
        fails.length > 0
          ? `${msgs.length - fails.length} 个成功，${fails.length} 个失败`
          : msgs.length > 0
            ? `已续期 ${msgs.length} 个账号`
            : "当前没有账号",
        fails.length > 0 ? "info" : "success"
      );
      setAccounts(await api.listAccounts());
    } catch (e: any) {
      pushToast("一键续期失败", e?.toString(), "error");
    } finally {
      setRenewRunning(false);
    }
  };

  const handleRefreshCredits = async (id: string) => {
    setCreditLoadingId(id);
    try {
      const r = await api.refreshCredits(id);
      if (!r.success) {
        pushToast("积分查询失败", r.message, "error");
      }
      setAccounts(await api.listAccounts());
    } catch (e: any) {
      pushToast("积分查询失败", e?.toString(), "error");
    } finally {
      setCreditLoadingId(null);
    }
  };

  if (loading) {
    return (
      <div className="page-loading">
        <RefreshCw className="spin" size={26} strokeWidth={1.6} />
      </div>
    );
  }

  // 工具行门控：无 可签账号 / 无可续账号 时禁用批量动作（与后端跳过逻辑一致）
  const checkableCount = filtered.filter(
    (a) =>
      !checkinBlocked(a) &&
      !tokenExpired(a) &&
      !(platform === "workbuddy" && isAiAccount(a))
  ).length;
  const renewEligible = filtered.length > 0;

  return (
    <div className="accs">
      <PageHeader
        title="账号"
        sub={`${platformLabel(platform)} · 共 ${filtered.length} 个账号 · 账本行：状态签 + 积分余量 + 行内操作`}
      />

      {/* 工具行 */}
      <div className="tools">
        <Button variant="pri" onClick={openAdd}>
          <Globe /> 网页授权
        </Button>
        <span className="spacer" />
        {platform === "qoder_cn" && (
          <Button
            variant="ghost"
            disabled={restoring}
            title="把上一次切换前的客户端登录现场写回并重启"
            onClick={handleRestoreLast}
          >
            <RotateCcw /> {restoring ? "恢复中…" : "恢复切换前现场"}
          </Button>
        )}
        <Button
          variant="ghost"
          disabled={batchRunning || checkableCount === 0}
          title={checkableCount === 0 ? "所有账号均已签到或暂不可签（冷却中 / 凭证失效）" : undefined}
          onClick={handleCheckinAll}
        >
          <Play /> {batchRunning ? "签到中…" : "一键签到"}
        </Button>
        <Button
          variant="ghost"
          disabled={renewRunning || !renewEligible}
          title={!renewEligible ? "当前没有账号" : "手动批量续期全部账号（不受 24h 窗口限制，单个账号 5 分钟内不可重复）"}
          onClick={handleRenewAll}
        >
          <RefreshCw className={renewRunning ? "spin" : ""} /> {renewRunning ? "续期中…" : "一键续期"}
        </Button>
        <Button
          variant="ghost"
          onClick={() => load(true)}
          disabled={refreshing}
          title="刷新"
        >
          <RefreshCw className={refreshing ? "spin" : ""} />
        </Button>
      </div>

      {/* 账本行 */}
      <div className="rows">
        {filtered.length === 0 ? (
          <div className="accs-empty">
            暂无{platformLabel(platform)}账号，点击上方「网页授权」快速添加
          </div>
        ) : (
          filtered.map((acc) => {
            const info = checkinChipInfo(acc);
            // 真实积分：右缘数值列（余额 + 最近一笔到期，悬停看全部明细）
            const now = nowSec;
            const valid = (acc.credits_expiries || []).filter((e) => e.at > now && e.amount > 0);
            let creditSub = "无到期中的积分包";
            let creditTitle = creditSub;
            if (valid[0]) {
              const days = Math.floor((valid[0].at - now) / 86400);
              creditSub = `最近到期 ${days < 1 ? "今日" : `${days} 天后`}`;
              creditTitle = valid
                .map(
                  (e) =>
                    `${new Date(e.at * 1000).toLocaleDateString()} 到期 ${e.amount.toLocaleString()}`
                )
                .join("\n");
            }
            const creditLoading = creditLoadingId === acc.id;
            // 常驻客户端切换图标：当前注入的客户端高亮（clients 由后端依据 active_client_* 填充）
            const apps =
              platform === "trae"
                ? ["trae"]
                : platform === "qoder_cn"
                  ? ["qoder_cn"]
                  : ["workbuddy", "codebuddy_cn"];
            // 切换/建档都要关掉客户端再覆盖数据目录，同一时间只允许一个进行：
            // 并发点击会让两个任务互相抢客户端，最后写「当前已注入」标记的那个
            // 未必是客户端里实际登录的账号。
            const clientOpRunning =
              switchingId !== null || switchingCnId !== null || archivingId !== null;
            // 行内按钮门控：今日已签/冷却中不可再签；手动续期不设到期窗口
            const blocked = checkinBlocked(acc);
            // 后端守护未确认登录时打标；列表里直接提示档案可能需要重建
            const vaultBad =
              (platform === "trae" || platform === "qoder_cn") &&
              Boolean(vaultFlags[acc.id]?.verify_failed_at);
            const isAi = platform === "workbuddy" && isAiAccount(acc);
            const menu: MenuItem[] = [
              { label: "编辑", onClick: () => openEdit(acc.id) },
              { sep: true },
              { label: "删除账户", danger: true, onClick: () => handleDelete(acc.id) },
            ];
            return (
              <LedgerRow
                key={acc.id}
                stacked
                avatar={<Avatar name={acc.name} src={acc.avatar} />}
                name={acc.name || "未命名"}
                meta={
                  isAi
                    ? [acc.email, "国际版"].filter(Boolean).join(" · ") || undefined
                    : acc.email || undefined
                }
                chip={<Chip tone={info.tone} dot>{info.label}</Chip>}
                extra={
                  <div className="accs-clients">
                    {apps.map((key) => {
                      const app = CLIENT_APPS[key];
                      const active = acc.clients?.includes(key);
                      return (
                        <button
                          key={key}
                          className={`accs-client${active ? " on" : ""}`}
                          title={
                            active
                              ? `当前登录在${app.label} · 点击切换到此客户端`
                              : `切换${app.label}（将重启客户端）`
                          }
                          disabled={clientOpRunning}
                          onClick={() => handleAppSwitch(key, acc.id)}
                        >
                          <img src={app.src} alt="" />
                          <span>{app.short}</span>
                        </button>
                      );
                    })}
                  </div>
                }
                actions={
                  <>
                    <RowAction
                      label="签到"
                      title={
                        isAi
                          ? "WorkBuddy 国际版没有签到接口"
                          : blocked === "cooling"
                          ? `冷却中 · 至 ${formatDate(acc.cooldown_until)}`
                          : blocked === "signed"
                            ? "今日已签到，明日 00:00 后可再签"
                            : blocked === "expired"
                              ? "凭证已失效，请先续期"
                              : "立即签到"
                      }
                      disabled={isAi || blocked != null || checkingId === acc.id}
                      onClick={() => handleCheckin(acc.id)}
                    />
                    <RowAction
                      label="查积分"
                      title="查询真实积分"
                      disabled={creditLoading}
                      onClick={() => handleRefreshCredits(acc.id)}
                    />
                    <RowAction
                      label="续期"
                      title={
                        acc.token_expires_at > 0 && acc.token_expires_at < nowSec
                          ? "凭证已过期，立即续期"
                          : acc.token_expires_at > 0
                            ? `手动续期（剩余 ${Math.ceil((acc.token_expires_at - nowSec) / 86400)} 天，不受 24h 窗口限制；5 分钟内不可重复）`
                            : "手动续期（无有效期信息也可尝试；5 分钟内不可重复）"
                      }
                      onClick={() => handleRenew(acc.id)}
                    />
                    {(platform === "trae" || platform === "qoder_cn") && (
                      <RowAction
                        label="存档"
                        title={`把当前 ${platformLabel(platform)} 客户端登录态存档到此账户（需先关闭客户端）`}
                        disabled={clientOpRunning}
                        onClick={() => handleArchive(acc.id)}
                      />
                    )}
                    <RowMenu items={menu} />
                  </>
                }
              >
                <div className="accs-sub">
                  <span className="accs-note">
                    最后签到 {formatRelative(acc.last_checkin_time)} · Token 到期{" "}
                    {acc.token_expires_at ? formatDate(acc.token_expires_at) : "—"}
                    {acc.cooldown_until > nowSec &&
                      ` · 冷却至 ${formatDate(acc.cooldown_until)}`}
                    {vaultBad && (
                      <span
                        className="accs-vault-bad"
                        title="上次切换未能确认客户端登录到该账号，建议重新登录并重新建档"
                      >
                        {" · 档案待验证"}
                      </span>
                    )}
                  </span>
                  {acc.credits_balance != null ? (
                    <span className="accs-credit" title={`积分包明细：\n${creditTitle}`}>
                      <span className="accs-credit-val">
                        {acc.credits_balance.toLocaleString()}
                        <i>积分</i>
                      </span>
                      <span className="accs-credit-sub">
                        {creditLoading ? "查询中…" : creditSub}
                      </span>
                    </span>
                  ) : (
                    <span className="accs-credit">
                      <span className="accs-credit-sub">
                        {creditLoading ? "查询中…" : "积分未查询"}
                      </span>
                    </span>
                  )}
                </div>
              </LedgerRow>
            );
          })
        )}
      </div>

      {/* 编辑账号弹窗 */}
      <Dialog
        open={editOpen}
        onClose={() => setEditOpen(false)}
        title="编辑账号"
        description="修改名称、邮箱与登录凭据"
      >
        <div className="fld">
          <label>名称</label>
          <TextInput
            value={editForm.name}
            onChange={(e) => setEditForm({ ...editForm, name: e.target.value })}
            placeholder="账户备注名"
          />
        </div>
        <div className="fld">
          <label>邮箱（可选）</label>
          <TextInput
            value={editForm.email}
            onChange={(e) => setEditForm({ ...editForm, email: e.target.value })}
            placeholder="email@example.com"
          />
        </div>
        <div className="fld">
          <label>accessToken</label>
          <TextArea
            rows={3}
            value={editForm.access_token}
            onChange={(e) => setEditForm({ ...editForm, access_token: e.target.value })}
          />
        </div>
        <div className="fld">
          <label>refreshToken</label>
          <TextArea
            rows={3}
            value={editForm.refresh_token}
            onChange={(e) => setEditForm({ ...editForm, refresh_token: e.target.value })}
          />
        </div>
        <div className="dlg-acts">
          <Button onClick={() => setEditOpen(false)}>取消</Button>
          <Button variant="pri" onClick={handleEditSave} disabled={editSaving}>
            {editSaving ? <RefreshCw className="spin" /> : null}
            保存
          </Button>
        </div>
      </Dialog>

      {/* 添加账号弹窗：网页授权登录，平台由当前产品决定 */}
      <Dialog
        open={addOpen}
        onClose={closeAddDialog}
        title={`添加 ${platformLabel(platform)} 账号`}
        description="在浏览器完成登录后自动捕获凭据"
      >
        <div className="accs-tabpane">
          {platform === "trae" ? (
            oauthWaiting ? (
              <>
                <div className="accs-waiting">
                  <RefreshCw className="spin" size={14} />
                  正在等待浏览器完成授权，回调将自动完成登录…
                </div>
                <div className="fld">
                  <label>回调 URL（自动填充失败时手动粘贴）</label>
                  <TextArea
                    value={oauthCallback}
                    onChange={(e) => setOauthCallback(e.target.value)}
                    placeholder="http://127.0.0.1:18923/authorize?refreshToken=..."
                    rows={3}
                  />
                </div>
                <p className="hint">
                  若长时间停留在「认证中」（常见于 18923
                  端口被占用），请取消后重试，或将回调 URL 粘贴到上方输入框完成登录。
                </p>
                <div className="dlg-acts">
                  <Button onClick={handleCancelOauth}>取消</Button>
                  <Button
                    variant="pri"
                    disabled={!oauthCallback.trim()}
                    onClick={() => finishOauth(oauthCallback.trim(), oauthName || undefined)}
                  >
                    完成登录
                  </Button>
                </div>
              </>
            ) : (
              <>
                <p className="hint">
                  点击下方按钮在系统浏览器中打开 TRAE
                  授权页，授权完成后自动捕获登录凭证并添加账户，无需手动复制 token。
                </p>
                <div className="fld">
                  <label>账号备注名（可选）</label>
                  <TextInput
                    value={oauthName}
                    onChange={(e) => setOauthName(e.target.value)}
                    placeholder="留空则使用 TRAE 用户名"
                  />
                </div>
                <Button variant="pri" className="w-full" onClick={handleTraeOauth}>
                  <Globe /> 打开浏览器授权登录
                </Button>
              </>
            )
          ) : platform === "qoder_cn" ? (
            qoderWaiting ? (
              <>
                <div className="accs-waiting">
                  <RefreshCw className="spin" size={14} />
                  正在轮询 Qoder CN 授权结果，登录成功后自动添加账户…
                </div>
                <Button
                  className="w-full"
                  onClick={() => {
                    void api.stopQoderLogin(qoderLoginState);
                    setQoderWaiting(false);
                    setQoderLoginState("");
                  }}
                >
                  取消
                </Button>
              </>
            ) : (
              <>
                <p className="hint">
                  点击下方按钮在浏览器中打开 Qoder CN 登录页，完成登录后自动添加账户。
                  若授权页提示「参数无效」，可切换到兼容模式后重试。
                </p>
                <div className="fld">
                  <label>授权模式</label>
                  <div className="wb-variant-row">
                    <Button
                      variant={!qoderCompat ? "pri" : "default"}
                      className="flex1"
                      onClick={() => setQoderCompat(false)}
                    >
                      标准
                    </Button>
                    <Button
                      variant={qoderCompat ? "pri" : "default"}
                      className="flex1"
                      onClick={() => setQoderCompat(true)}
                    >
                      兼容
                    </Button>
                  </div>
                </div>
                <Button variant="pri" className="w-full" onClick={handleQoderLogin}>
                  <Globe /> 打开浏览器授权登录
                </Button>
              </>
            )
          ) : wbWaiting ? (
            <>
              <p className="hint">
                等待浏览器登录完成…（扫码或账号授权后会自动添加账户）
              </p>
              <Button
                className="w-full"
                onClick={() => {
                  setWbWaiting(false);
                  setWbLoginState("");
                }}
              >
                取消
              </Button>
            </>
          ) : (
            <>
              <p className="hint">
                点击下方按钮在浏览器中打开 WorkBuddy
                登录页，完成登录后自动添加账户。国际版账号无签到接口，仅支持切换客户端与用量统计。
              </p>
              <div className="fld">
                <label>WorkBuddy 版本</label>
                <div className="wb-variant-row">
                  <Button
                    variant={wbVariant === "cn" ? "pri" : "default"}
                    className="flex1"
                    onClick={() => setWbVariant("cn")}
                  >
                    国内版
                  </Button>
                  <Button
                    variant={wbVariant === "ai" ? "pri" : "default"}
                    className="flex1"
                    onClick={() => setWbVariant("ai")}
                  >
                    国际版
                  </Button>
                </div>
              </div>
              <Button variant="pri" className="w-full" onClick={handleWorkbuddyLogin}>
                <Globe /> 打开浏览器登录 WorkBuddy
              </Button>
            </>
          )}
        </div>
      </Dialog>
    </div>
  );
}
