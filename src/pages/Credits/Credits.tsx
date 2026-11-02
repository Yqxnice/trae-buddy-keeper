import { useCallback, useEffect, useState } from "react";
import { RefreshCw, Package } from "lucide-react";
import { PageHeader } from "@/components/PageHeader/PageHeader";
import { Button } from "@/components/Button/Button";
import { Avatar } from "@/components/Avatar/Avatar";
import { Meter } from "@/components/Meter/Meter";
import { api, type AccountCredits } from "@/lib/api";
import { formatDate, platformLabel } from "@/lib/utils";
import { useUIStore } from "@/stores/ui";
import "./Credits.css";

function daysLeft(at: number) {
  return Math.floor((at - Date.now() / 1000) / 86400);
}

export function CreditsPage() {
  const platform = useUIStore((s) => s.platform);
  const pushToast = useUIStore((s) => s.pushToast);
  const [data, setData] = useState<AccountCredits[]>([]);
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);

  // 全屏 loading 仅首次挂载生效
  const load = useCallback(async () => {
    try {
      setData(await api.getCreditPackages(platform));
    } catch (e: any) {
      pushToast("加载失败", e?.toString(), "error");
    } finally {
      setLoading(false);
    }
  }, [platform, pushToast]);

  useEffect(() => {
    load();
  }, [load]);

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

  if (loading) {
    return (
      <div className="page-loading">
        <RefreshCw className="spin" size={26} strokeWidth={1.6} />
      </div>
    );
  }

  const total = data.reduce((s, a) => s + a.balance, 0);
  const pkgCount = data.reduce((s, a) => s + a.packages.length, 0);

  return (
    <div className="credits">
      <PageHeader
        title="积分"
        sub={
          platform === "workbuddy"
            ? "各账号实时余额与积分包到期明细（数据来自腾讯 billing 接口）"
            : "各账号实时余额与积分包到期明细（数据来自 TRAE 权益接口）"
        }
        right={
          <Button onClick={handleRefresh} disabled={refreshing}>
            <RefreshCw className={refreshing ? "spin" : ""} />
            刷新
          </Button>
        }
      />

      {/* 汇总条 */}
      <div className="total-card">
        <span className="tv">{total.toLocaleString()}</span>
        <span className="tcap">积分总余额</span>
        <span className="tcap sep">·</span>
        <span className="tcap">
          {pkgCount} 个有效积分包
        </span>
        <span className="spacer" />
        <span className="tcap">
          <Package size={12} style={{ verticalAlign: -2 }} /> 共 {data.length} 个账号
        </span>
      </div>

      {/* 各账号积分包卡片 */}
      {data.length === 0 ? (
        <div className="credits-empty">暂无{platformLabel(platform)}账号</div>
      ) : (
        data.map((acc) => {
          // 余量条比例基准：该账号最大积分包，纯视觉编码包间相对大小
          const maxPkg = Math.max(...acc.packages.map((p) => p.amount), 1);
          return (
            <div className="pkg-card" key={acc.account_id}>
              <div className="pkg-head">
                <Avatar name={acc.account_name || "未命名"} size="sm" />
                <span className="rname">{acc.account_name || "未命名"}</span>
                {acc.fetched_at > 0 && (
                  <span className="rdev">查询于 {formatDate(acc.fetched_at)}</span>
                )}
                <span className="pkg-balance">
                  {acc.balance.toLocaleString()}
                  <span className="unit">积分</span>
                </span>
              </div>
              {acc.packages.length === 0 ? (
                <p className="credits-none">无有效积分包</p>
              ) : (
                acc.packages.map((p, i) => {
                  const days = daysLeft(p.at);
                  return (
                    <div className="pkg-row" key={i}>
                      <span className="pkg-label" title={p.name || undefined}>
                        {p.name || "积分包"}
                      </span>
                      <Meter small value={(p.amount / maxPkg) * 100} />
                      <span className="pkg-val">{p.amount.toLocaleString()} 积分</span>
                      <span className={`pkg-exp${days < 1 ? " bad" : days < 7 ? " warn" : ""}`}>
                        {days < 1 ? "今日到期" : `${days} 天后到期`} · {formatDate(p.at)}
                      </span>
                    </div>
                  );
                })
              )}
            </div>
          );
        })
      )}
    </div>
  );
}
