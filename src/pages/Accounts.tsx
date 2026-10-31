import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  Trash2,
  Play,
  RefreshCw,
  Globe,
  Upload,
  Edit3,
  Pencil,
  ArrowLeftRight,
  Coins,
  Save,
  Laptop,
} from "lucide-react";
import { Card, CardContent } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogClose,
} from "@/components/ui/dialog";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import { api, type AccountInfo } from "@/lib/api";
import { confirmDialog } from "@/components/ConfirmDialog";
import { formatDate, formatRelative, platformLabel, checkinDayStart } from "@/lib/utils";

type ToastType = "success" | "error" | "info";

export function AccountsPage({
  platform,
  pushToast,
}: {
  platform: string;
  pushToast: (t: string, b: string, type?: ToastType) => void;
}) {
  const [accounts, setAccounts] = useState<AccountInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [addOpen, setAddOpen] = useState(false);
  const [addTab, setAddTab] = useState("manual");
  const [batchRunning, setBatchRunning] = useState(false);
  const [renewRunning, setRenewRunning] = useState(false);

  // 手动添加表单（平台由当前面板标签决定）
  const [manual, setManual] = useState({
    name: "",
    email: "",
    access_token: "",
    refresh_token: "",
    cookies: "",
  });
  const [importJson, setImportJson] = useState("");
  const [checkingId, setCheckingId] = useState<string | null>(null);
  const [switchingId, setSwitchingId] = useState<string | null>(null);
  const [switchingCnId, setSwitchingCnId] = useState<string | null>(null);
  const [archivingId, setArchivingId] = useState<string | null>(null);
  const [creditLoadingId, setCreditLoadingId] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);

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

  // WorkBuddy 网页登录状态（state 轮询）
  const [wbWaiting, setWbWaiting] = useState(false);
  const [wbLoginState, setWbLoginState] = useState("");

  // 全屏 loading 仅首次挂载生效；后续刷新原地更新列表，避免整页拆建抖动
  // notify=true（手动刷新）时成功/失败均 toast；操作后的自动重载保持静默
  const load = async (notify = false) => {
    if (notify) setRefreshing(true);
    try {
      const list = await api.listAccounts();
      setAccounts(list);
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
  };

  useEffect(() => {
    load();
  }, [platform]);

  const filtered = accounts.filter((a) => a.platform === platform);

  // 三种添加方式：手动 / 导入 / 网页授权，各自直达对应面板
  const openAdd = (tab: string) => {
    setAddTab(tab);
    setAddOpen(true);
  };

  const handleManualAdd = async () => {
    if (!manual.access_token && platform === "trae") {
      pushToast("提示", "TRAE 账户需要 access_token", "error");
      return;
    }
    if (!manual.cookies && platform === "workbuddy") {
      pushToast("提示", "WorkBuddy 账户需要 cookies", "error");
      return;
    }
    try {
      await api.addAccountManual({ ...manual, platform });
      pushToast("成功", "账户已添加", "success");
      setAddOpen(false);
      setManual({
        name: "",
        email: "",
        access_token: "",
        refresh_token: "",
        cookies: "",
      });
      load();
    } catch (e: any) {
      pushToast("添加失败", e?.toString(), "error");
    }
  };

  const handleImport = async () => {
    // JSON 中未显式指定 platform 时，按当前面板标签注入
    let json = importJson;
    try {
      const parsed = JSON.parse(importJson);
      const items = (Array.isArray(parsed) ? parsed : [parsed]).map((it) => ({
        platform,
        ...it,
      }));
      json = JSON.stringify(Array.isArray(parsed) ? items : items[0]);
    } catch {
      pushToast("导入失败", "JSON 解析失败，请检查格式", "error");
      return;
    }
    try {
      const msg = await api.addAccountImport(json);
      pushToast("导入成功", msg, "success");
      setImportJson("");
      setAddOpen(false);
      load();
    } catch (e: any) {
      pushToast("导入失败", e?.toString(), "error");
    }
  };

  const finishOauth = async (callbackUrl: string, name?: string) => {
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
  };

  // 授权回调事件：浏览器完成授权后自动完成登录
  useEffect(() => {
    const unlisten = listen<string>("oauth-callback", (e) => {
      finishOauth(e.payload, oauthName || undefined);
    });
    return () => {
      unlisten.then((f) => f());
    };
  }, [oauthName]);

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
    setAddOpen(false);
  };

  const handleWorkbuddyLogin = async () => {
    try {
      const r = await api.startWorkbuddyLogin();
      setWbLoginState(r.state);
      setWbWaiting(true);
      pushToast("提示", "已在浏览器打开 WorkBuddy 登录页，完成后将自动添加", "info");
    } catch (e: any) {
      pushToast("启动登录失败", e?.toString(), "error");
    }
  };

  // WorkBuddy 登录轮询：每 2 秒查询一次，成功即入库并关闭弹窗
  useEffect(() => {
    if (!wbWaiting || !wbLoginState) return;
    const timer = setInterval(async () => {
      try {
        const r = await api.pollWorkbuddyLogin(wbLoginState);
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
  }, [wbWaiting, wbLoginState]);

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
    if (
      !(await confirmDialog({
        title: "删除账户",
        message: "确定删除该账户？相关签到记录也会被清除。",
        confirmText: "删除",
        danger: true,
      }))
    )
      return;
    try {
      await api.deleteAccount(id);
      pushToast("成功", "账户已删除", "success");
      load();
    } catch (e: any) {
      pushToast("删除失败", e?.toString(), "error");
    }
  };

  const handleSwitch = async (id: string) => {
    // WorkBuddy/Trae：真实切换客户端登录账号（会关闭并重启客户端，耗时较长）
    if (platform === "workbuddy" || platform === "trae") {
      setSwitchingId(id);
      try {
        // Trae：运行中切库会写花客户端数据，先检测并明确告知会自动关闭客户端
        if (platform === "trae" && (await api.traeClientRunning())) {
          pushToast("正在关闭客户端", "检测到 Trae CN 正在运行，切换前会先关闭它", "info");
        }
        const msg =
          platform === "workbuddy"
            ? await api.switchWorkbuddyClient(id)
            : await api.switchTraeClient(id);
        pushToast("切换成功", msg, "success");
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

  // Trae：把当前客户端登录态快照到该账号（建档），切换的前提
  const handleArchive = async (id: string) => {
    setArchivingId(id);
    try {
      const msg = await api.backupTraeVault(id);
      pushToast("建档成功", msg, "success");
    } catch (e: any) {
      pushToast("建档失败", e?.toString(), "error");
    } finally {
      setArchivingId(null);
    }
  };

  const statusBadge = (status: string, failCount: number, lastCheckin: number) => {
    switch (status) {
      case "success":
        // success 状态会跨天保留，只有落在今日 6 点之后才算「今日已签到」
        return lastCheckin >= checkinDayStart() ? (
          <Badge variant="success">已签到</Badge>
        ) : (
          <Badge variant="secondary">未签到</Badge>
        );
      case "failed":
        return <Badge variant="destructive">失败 {failCount} 次</Badge>;
      case "cooling":
        return <Badge variant="cooling">冷却中</Badge>;
      default:
        return <Badge variant="secondary">未签到</Badge>;
    }
  };

  // 一键签到：只签当前产品平台
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
    } catch (e: any) {
      pushToast("批量签到失败", e?.toString(), "error");
    } finally {
      setBatchRunning(false);
    }
  };

  // 一键续期：对即将到期的凭证自动续期
  const handleRenewAll = async () => {
    setRenewRunning(true);
    try {
      const msgs = await api.autoRenewAll();
      const fails = msgs.filter((m) => m.includes("失败"));
      pushToast(
        "一键续期完成",
        fails.length > 0 ? `${msgs.length - fails.length} 个成功，${fails.length} 个失败` : "全部成功或无需续期",
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

  // 真实积分：余额 + 最近一笔到期角标（悬停看全部明细）；两个平台都支持
  const creditBadge = (acc: AccountInfo) => {
    if (acc.credits_balance == null) {
      return <span className="text-xs text-muted-foreground">积分未查询</span>;
    }
    const now = Date.now() / 1000;
    const valid = (acc.credits_expiries || []).filter(
      (e) => e.at > now && e.amount > 0
    );
    const nearest = valid[0];
    const detail = valid
      .map(
        (e) =>
          `${new Date(e.at * 1000).toLocaleDateString()} 到期 ${e.amount.toLocaleString()}`
      )
      .join("\n");
    let badge = null;
    if (nearest) {
      const days = Math.floor((nearest.at - now) / 86400);
      const variant =
        days < 1 ? "destructive" : days < 7 ? "warning" : "secondary";
      badge = (
        <Badge variant={variant}>
          {nearest.amount.toLocaleString()} 额度 ·{" "}
          {days < 1 ? "今日到期" : `${days} 天`}
        </Badge>
      );
    }
    return (
      <span
        className="flex items-center gap-2"
        title={detail ? `积分包明细：\n${detail}` : "无到期中的积分包"}
      >
        <span className="text-xs font-medium text-amber-600">
          积分 {acc.credits_balance.toLocaleString()}
        </span>
        {badge}
      </span>
    );
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
          <h1 className="text-2xl font-bold">{platformLabel(platform)} 账号管理</h1>
          <p className="text-sm text-muted-foreground">
            共 {filtered.length} 个账号
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            disabled={batchRunning}
            onClick={handleCheckinAll}
          >
            <Play className="h-4 w-4 mr-1" />
            {batchRunning ? "签到中…" : "一键签到"}
          </Button>
          <Button
            variant="outline"
            size="sm"
            disabled={renewRunning}
            onClick={handleRenewAll}
          >
            <RefreshCw className="h-4 w-4 mr-1" />
            {renewRunning ? "续期中…" : "一键续期"}
          </Button>
          <Button variant="outline" size="sm" onClick={() => openAdd("manual")}>
            <Edit3 className="h-4 w-4 mr-1" />
            手动添加
          </Button>
          <Button variant="outline" size="sm" onClick={() => openAdd("import")}>
            <Upload className="h-4 w-4 mr-1" />
            导入
          </Button>
          <Button size="sm" onClick={() => openAdd("login")}>
            <Globe className="h-4 w-4 mr-1" />
            网页授权
          </Button>
          <Button
            variant="ghost"
            size="icon"
            onClick={() => load(true)}
            disabled={refreshing}
            title="刷新"
          >
            <RefreshCw
              className={`h-4 w-4 ${refreshing ? "animate-spin" : ""}`}
            />
          </Button>
        </div>
      </div>

      <div className="grid gap-4">
        {filtered.length === 0 ? (
          <Card>
            <CardContent className="py-16 text-center text-muted-foreground">
              暂无{platformLabel(platform)}账号，点击右上角「网页授权」快速添加
            </CardContent>
          </Card>
        ) : (
          filtered.map((acc) => (
            <Card key={acc.id}>
              <CardContent className="p-4 flex items-center gap-4">
                {/* 头像：登录链路抓到 URL 就显示图片，否则回退首字母圆标 */}
                {acc.avatar ? (
                  <img
                    src={acc.avatar}
                    alt={acc.name}
                    className="h-10 w-10 rounded-full object-cover border shrink-0"
                    onError={(e) => {
                      (e.target as HTMLImageElement).style.display = "none";
                    }}
                  />
                ) : (
                  <div className="h-10 w-10 rounded-full bg-primary/10 text-primary flex items-center justify-center font-semibold shrink-0 border">
                    {(acc.name || "?").charAt(0).toUpperCase()}
                  </div>
                )}
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 mb-1">
                    <span className="font-semibold">{acc.name || "未命名"}</span>
                    {acc.email && (
                      <span className="text-sm text-muted-foreground">{acc.email}</span>
                    )}
                    {statusBadge(acc.checkin_status, acc.fail_count, acc.last_checkin_time)}
                    {creditBadge(acc)}
                  </div>
                  <div className="flex gap-4 text-xs text-muted-foreground">
                    <span>最后签到：{formatRelative(acc.last_checkin_time)}</span>
                    <span>
                      Token 到期：{acc.token_expires_at ? formatDate(acc.token_expires_at) : "—"}
                    </span>
                    {acc.cooldown_until > Date.now() / 1000 && (
                      <span className="text-blue-500">
                        冷却至：{formatDate(acc.cooldown_until)}
                      </span>
                    )}
                  </div>
                </div>
                <div className="flex items-center gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => handleRefreshCredits(acc.id)}
                    disabled={creditLoadingId === acc.id}
                    title="查询真实积分"
                  >
                    {creditLoadingId === acc.id ? (
                      <RefreshCw className="h-4 w-4 animate-spin" />
                    ) : (
                      <Coins className="h-4 w-4" />
                    )}
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => handleCheckin(acc.id)}
                    disabled={checkingId === acc.id}
                  >
                    {checkingId === acc.id ? (
                      <RefreshCw className="h-4 w-4 animate-spin" />
                    ) : (
                      <Play className="h-4 w-4" />
                    )}
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => handleRenew(acc.id)}
                    title="手动续期"
                  >
                    <RefreshCw className="h-4 w-4" />
                  </Button>
                  {platform === "trae" && (
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => handleArchive(acc.id)}
                      disabled={archivingId === acc.id}
                      title="把当前 Trae 客户端登录态存档到此账户（需先关闭客户端）"
                    >
                      {archivingId === acc.id ? (
                        <RefreshCw className="h-4 w-4 animate-spin" />
                      ) : (
                        <Save className="h-4 w-4" />
                      )}
                    </Button>
                  )}
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => handleSwitch(acc.id)}
                    disabled={switchingId === acc.id}
                    title={
                      platform === "workbuddy"
                        ? "切换 WorkBuddy 客户端到此账户（将重启客户端）"
                        : platform === "trae"
                          ? "切换 Trae 客户端到此账户（将重启客户端，需先存档）"
                          : "切换到此账户"
                    }
                  >
                    {switchingId === acc.id ? (
                      <RefreshCw className="h-4 w-4 animate-spin" />
                    ) : (
                      <ArrowLeftRight className="h-4 w-4" />
                    )}
                  </Button>
                  {platform === "workbuddy" && (
                    <Button
                      variant="outline"
                      size="sm"
                      onClick={() => handleSwitchCn(acc.id)}
                      disabled={switchingCnId === acc.id}
                      title="切换 CodeBuddy CN 到此账户（将重启客户端）"
                    >
                      {switchingCnId === acc.id ? (
                        <RefreshCw className="h-4 w-4 animate-spin" />
                      ) : (
                        <Laptop className="h-4 w-4" />
                      )}
                    </Button>
                  )}
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => openEdit(acc.id)}
                    title="编辑账户"
                  >
                    <Pencil className="h-4 w-4" />
                  </Button>
                  <Button
                    variant="ghost"
                    size="icon"
                    onClick={() => handleDelete(acc.id)}
                  >
                    <Trash2 className="h-4 w-4 text-destructive" />
                  </Button>
                </div>
              </CardContent>
            </Card>
          ))
        )}
      </div>

      {/* 编辑账号弹窗 */}
      <Dialog open={editOpen} onOpenChange={setEditOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>编辑账号</DialogTitle>
            <DialogDescription>修改名称、邮箱与登录凭据</DialogDescription>
          </DialogHeader>
          <DialogClose onClose={() => setEditOpen(false)} />
          <div className="space-y-3 pt-2">
            <div>
              <label className="text-sm font-medium">名称</label>
              <Input
                value={editForm.name}
                onChange={(e) => setEditForm({ ...editForm, name: e.target.value })}
                placeholder="账户备注名"
              />
            </div>
            <div>
              <label className="text-sm font-medium">accessToken</label>
              <Textarea
                value={editForm.access_token}
                onChange={(e) =>
                  setEditForm({ ...editForm, access_token: e.target.value })
                }
                rows={3}
                className="font-mono text-xs"
              />
            </div>
            <div>
              <label className="text-sm font-medium">refreshToken</label>
              <Textarea
                value={editForm.refresh_token}
                onChange={(e) =>
                  setEditForm({ ...editForm, refresh_token: e.target.value })
                }
                rows={3}
                className="font-mono text-xs"
              />
            </div>
            <div className="flex gap-2 pt-1">
              <Button
                variant="outline"
                className="flex-1"
                onClick={() => setEditOpen(false)}
              >
                取消
              </Button>
              <Button
                className="flex-1"
                onClick={handleEditSave}
                disabled={editSaving}
              >
                {editSaving ? (
                  <RefreshCw className="h-4 w-4 mr-2 animate-spin" />
                ) : null}
                保存
              </Button>
            </div>
          </div>
        </DialogContent>
      </Dialog>

      {/* 添加账号弹窗：平台由当前产品决定 */}
      <Dialog open={addOpen} onOpenChange={(open) => (open ? setAddOpen(true) : closeAddDialog())}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>添加 {platformLabel(platform)} 账号</DialogTitle>
            <DialogDescription>
              支持网页授权（推荐）、手动添加、凭据导入
            </DialogDescription>
          </DialogHeader>
          <DialogClose onClose={closeAddDialog} />
          <Tabs value={addTab} onValueChange={setAddTab}>
            <TabsList className="grid grid-cols-3 w-full">
              <TabsTrigger value="manual">
                <Edit3 className="h-4 w-4 mr-1" /> 手动
              </TabsTrigger>
              <TabsTrigger value="import">
                <Upload className="h-4 w-4 mr-1" /> 导入
              </TabsTrigger>
              <TabsTrigger value="login">
                <Globe className="h-4 w-4 mr-1" /> 网页授权
              </TabsTrigger>
            </TabsList>

            <TabsContent value="manual" className="space-y-3 pt-4">
              <div>
                <label className="text-sm font-medium">名称</label>
                <Input
                  value={manual.name}
                  onChange={(e) => setManual({ ...manual, name: e.target.value })}
                  placeholder="账户备注名"
                />
              </div>
              <div>
                <label className="text-sm font-medium">邮箱（可选）</label>
                <Input
                  value={manual.email}
                  onChange={(e) => setManual({ ...manual, email: e.target.value })}
                  placeholder="email@example.com"
                />
              </div>
              {platform === "trae" ? (
                <>
                  <div>
                    <label className="text-sm font-medium">Access Token</label>
                    <Input
                      value={manual.access_token}
                      onChange={(e) =>
                        setManual({ ...manual, access_token: e.target.value })
                      }
                      placeholder="Bearer token"
                    />
                  </div>
                  <div>
                    <label className="text-sm font-medium">Refresh Token（可选）</label>
                    <Input
                      value={manual.refresh_token}
                      onChange={(e) =>
                        setManual({ ...manual, refresh_token: e.target.value })
                      }
                      placeholder="用于自动续期"
                    />
                  </div>
                </>
              ) : (
                <div>
                  <label className="text-sm font-medium">Cookies（JSON 数组）</label>
                  <Textarea
                    value={manual.cookies}
                    onChange={(e) => setManual({ ...manual, cookies: e.target.value })}
                    placeholder='[{"name":"xxx","value":"yyy"}]'
                    rows={4}
                  />
                </div>
              )}
              <Button className="w-full" onClick={handleManualAdd}>
                添加账户
              </Button>
            </TabsContent>

            <TabsContent value="import" className="space-y-3 pt-4">
              <div>
                <label className="text-sm font-medium">凭据 JSON</label>
                <Textarea
                  value={importJson}
                  onChange={(e) => setImportJson(e.target.value)}
                  placeholder={
                    platform === "trae"
                      ? '{"name":"我的账户","access_token":"...","refresh_token":"..."}'
                      : '{"name":"我的账户","cookies":[{"name":"xxx","value":"yyy"}]}'
                  }
                  rows={8}
                />
              </div>
              <p className="text-xs text-muted-foreground">
                支持单个对象或数组，默认导入为 {platformLabel(platform)}{" "}
                账户（JSON 中显式指定的 platform 字段优先）。
              </p>
              <Button className="w-full" onClick={handleImport}>
                导入
              </Button>
            </TabsContent>

            <TabsContent value="login" className="space-y-3 pt-4">
              {platform === "trae" ? (
                oauthWaiting ? (
                  <>
                    <div className="flex items-center gap-2 text-sm text-muted-foreground">
                      <RefreshCw className="h-4 w-4 animate-spin" />
                      正在等待浏览器完成授权，回调将自动完成登录…
                    </div>
                    <div>
                      <label className="text-sm font-medium">回调 URL（自动填充失败时手动粘贴）</label>
                      <Textarea
                        value={oauthCallback}
                        onChange={(e) => setOauthCallback(e.target.value)}
                        placeholder="http://127.0.0.1:18923/authorize?refreshToken=..."
                        rows={3}
                      />
                    </div>
                    <p className="text-xs text-muted-foreground">
                      若长时间停留在「认证中」（常见于 18923
                      端口被占用），请取消后改用「手动」标签页添加。
                    </p>
                    <div className="flex gap-2">
                      <Button
                        variant="outline"
                        className="flex-1"
                        onClick={handleCancelOauth}
                      >
                        取消
                      </Button>
                      <Button
                        className="flex-1"
                        disabled={!oauthCallback.trim()}
                        onClick={() => finishOauth(oauthCallback.trim(), oauthName || undefined)}
                      >
                        完成登录
                      </Button>
                    </div>
                  </>
                ) : (
                  <>
                    <p className="text-sm text-muted-foreground">
                      点击下方按钮在系统浏览器中打开 TRAE
                      授权页，授权完成后自动捕获登录凭证并添加账户，无需手动复制 token。
                    </p>
                    <div>
                      <label className="text-sm font-medium">账号备注名（可选）</label>
                      <Input
                        value={oauthName}
                        onChange={(e) => setOauthName(e.target.value)}
                        placeholder="留空则使用 TRAE 用户名"
                      />
                    </div>
                    <Button className="w-full" onClick={handleTraeOauth}>
                      <Globe className="h-4 w-4 mr-2" /> 打开浏览器授权登录
                    </Button>
                  </>
                )
              ) : wbWaiting ? (
                <>
                  <p className="text-sm text-muted-foreground">
                    等待浏览器登录完成…（扫码或账号授权后会自动添加账户）
                  </p>
                  <Button
                    variant="outline"
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
                  <p className="text-sm text-muted-foreground">
                    点击下方按钮在浏览器中打开 WorkBuddy
                    登录页，完成登录后自动添加账户。
                  </p>
                  <Button className="w-full" onClick={handleWorkbuddyLogin}>
                    <Globe className="h-4 w-4 mr-2" /> 打开浏览器登录 WorkBuddy
                  </Button>
                </>
              )}
            </TabsContent>
          </Tabs>
        </DialogContent>
      </Dialog>
    </div>
  );
}
