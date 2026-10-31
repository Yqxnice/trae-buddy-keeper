/**
 * 外观主题应用：light | dark | system
 * 通过在 <html> 上切换 .dark 类驱动 index.css 中的暗色变量；
 * system 模式监听系统配色变化实时同步。
 */

let mediaList: MediaQueryList | null = null;
let mediaHandler: ((e: MediaQueryListEvent) => void) | null = null;

export function applyTheme(mode: string) {
  const root = document.documentElement;

  // 清理旧的系统配色监听
  if (mediaList && mediaHandler) {
    mediaList.removeEventListener("change", mediaHandler);
    mediaList = null;
    mediaHandler = null;
  }

  if (mode === "system") {
    mediaList = window.matchMedia("(prefers-color-scheme: dark)");
    const sync = () => root.classList.toggle("dark", mediaList!.matches);
    sync();
    mediaHandler = sync;
    mediaList.addEventListener("change", mediaHandler);
  } else {
    root.classList.toggle("dark", mode === "dark");
  }
}

/** 读取当前已保存的主题（设置页下拉框用），缺省浅色 */
export function normalizeTheme(mode: string | undefined | null): string {
  return mode === "dark" || mode === "system" ? mode : "light";
}
