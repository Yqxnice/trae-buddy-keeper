import { check, type Update, type DownloadEvent } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

export type UpdatePhase =
  | "idle" // 未检查
  | "checking" // 检查中
  | "available" // 有新版本，待下载
  | "downloading" // 下载安装中
  | "ready" // 下载完成，待重启
  | "latest" // 已是最新
  | "error"; // 检查/下载失败

/**
 * 应用自动更新：封装 tauri-plugin-updater 的检查 / 下载 / 安装 / 重启流程。
 * 更新清单来自 GitHub Releases（tauri.conf.json 的 plugins.updater.endpoints）。
 */
export class UpdaterService {
  private pending: Update | null = null;

  /** 检查更新；有新版本时返回其版本号，无更新返回 null */
  async check(): Promise<string | null> {
    this.pending = null;
    const update = await check();
    if (!update) return null;
    this.pending = update;
    return update.version;
  }

  /** 当前安装的版本号（tauri.conf.json 的 version） */
  async currentVersion(): Promise<string> {
    const { getVersion } = await import("@tauri-apps/api/app");
    return getVersion();
  }

  /**
   * 下载并安装更新包，onProgress 回调汇报 0~100 的百分比。
   * 完成后应用仍运行旧版本，需调用 relaunch() 重启生效。
   */
  async downloadAndInstall(onProgress: (percent: number) => void): Promise<void> {
    if (!this.pending) throw new Error("没有待安装的更新");
    let total = 0;
    let received = 0;
    const onEvent = (event: DownloadEvent) => {
      if (event.event === "Started") {
        total = event.data.contentLength ?? 0;
        received = 0;
      } else if (event.event === "Progress") {
        received += event.data.chunkLength;
        if (total > 0) onProgress(Math.min(99, Math.round((received / total) * 100)));
      } else if (event.event === "Finished") {
        onProgress(100);
      }
    };
    await this.pending.downloadAndInstall(onEvent);
  }

  /** 重启应用以应用已安装的更新 */
  async relaunch(): Promise<void> {
    await relaunch();
  }
}

export const updater = new UpdaterService();
