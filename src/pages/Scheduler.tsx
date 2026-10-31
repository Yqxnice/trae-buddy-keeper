import { useEffect, useState } from "react";
import { Clock, RefreshCw, Save, Repeat, RotateCcw } from "lucide-react";
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
import { api, type CheckinSettings } from "@/lib/api";

type ToastType = "success" | "error" | "info";

export function SchedulerPage({
  pushToast,
}: {
  pushToast: (t: string, b: string, type?: ToastType) => void;
}) {
  const [settings, setSettings] = useState<CheckinSettings | null>(null);
  const [loading, setLoading] = useState(true);

  const load = async () => {
    setLoading(true);
    try {
      setSettings(await api.getCheckinSettings());
    } catch (e: any) {
      pushToast("加载失败", e?.toString(), "error");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    load();
  }, []);

  const save = async () => {
    if (!settings) return;
    try {
      await api.setCheckinSettings(settings);
      pushToast("已保存", "定时任务设置已更新", "success");
    } catch (e: any) {
      pushToast("保存失败", e?.toString(), "error");
    }
  };

  if (loading || !settings) {
    return (
      <div className="flex items-center justify-center h-full">
        <RefreshCw className="h-8 w-8 animate-spin text-muted-foreground" />
      </div>
    );
  }

  return (
    <div className="p-6 space-y-6 max-w-2xl">
      <div>
        <h1 className="text-2xl font-bold">定时任务</h1>
        <p className="text-sm text-muted-foreground">
          配置定时签到，并查看补签与自动续期的执行规则
        </p>
      </div>

      {/* 定时签到 */}
      <Card>
        <CardHeader>
          <CardTitle className="text-base flex items-center gap-2">
            <Clock className="h-4 w-4" /> 定时签到
          </CardTitle>
          <CardDescription>设置每日自动签到时间与失败处理策略</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-center justify-between">
            <div>
              <p className="font-medium">启用定时签到</p>
              <p className="text-sm text-muted-foreground">到点自动批量签到</p>
            </div>
            <Switch
              checked={settings.enabled}
              onCheckedChange={() => setSettings({ ...settings, enabled: !settings.enabled })}
            />
          </div>
          <div className="grid grid-cols-2 gap-4">
            <div>
              <label className="text-sm font-medium">签到时间（时）</label>
              <Input
                type="number"
                min={0}
                max={23}
                value={settings.hour}
                onChange={(e) =>
                  setSettings({ ...settings, hour: Number(e.target.value) })
                }
              />
            </div>
            <div>
              <label className="text-sm font-medium">签到时间（分）</label>
              <Input
                type="number"
                min={0}
                max={59}
                value={settings.minute}
                onChange={(e) =>
                  setSettings({ ...settings, minute: Number(e.target.value) })
                }
              />
            </div>
          </div>
          <div className="grid grid-cols-3 gap-4">
            <div>
              <label className="text-sm font-medium">签到间隔（秒）</label>
              <Input
                type="number"
                min={1}
                value={settings.interval_seconds}
                onChange={(e) =>
                  setSettings({ ...settings, interval_seconds: Number(e.target.value) })
                }
              />
            </div>
            <div>
              <label className="text-sm font-medium">最大失败次数</label>
              <Input
                type="number"
                min={1}
                value={settings.max_fail_count}
                onChange={(e) =>
                  setSettings({ ...settings, max_fail_count: Number(e.target.value) })
                }
              />
            </div>
            <div>
              <label className="text-sm font-medium">冷却时长（分）</label>
              <Input
                type="number"
                min={1}
                value={settings.cooldown_minutes}
                onChange={(e) =>
                  setSettings({ ...settings, cooldown_minutes: Number(e.target.value) })
                }
              />
            </div>
          </div>
          <p className="text-xs text-muted-foreground">
            失败达到最大次数后，账户进入冷却，冷却期间跳过签到。定时签到对所有产品生效。
          </p>
          <Button onClick={save}>
            <Save className="h-4 w-4 mr-2" /> 保存定时任务设置
          </Button>
        </CardContent>
      </Card>

      {/* 其他定时任务 */}
      <Card>
        <CardHeader>
          <CardTitle className="text-base flex items-center gap-2">
            <Repeat className="h-4 w-4" /> 其他定时任务
          </CardTitle>
          <CardDescription>随应用运行自动执行，无需手动配置</CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-start gap-3">
            <RotateCcw className="h-4 w-4 mt-0.5 text-muted-foreground" />
            <div>
              <p className="text-sm font-medium">超时补签</p>
              <p className="text-sm text-muted-foreground">
                每 10 分钟检查一次，对当日未签到且不在冷却、未失效的账号自动补签。
              </p>
            </div>
          </div>
          <div className="flex items-start gap-3">
            <RefreshCw className="h-4 w-4 mt-0.5 text-muted-foreground" />
            <div>
              <p className="text-sm font-medium">自动续期巡检</p>
              <p className="text-sm text-muted-foreground">
                启动时立即执行一次，之后每 6 小时一轮；token 在 24 小时内到期（含已过期）的账号自动续期。
              </p>
            </div>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
