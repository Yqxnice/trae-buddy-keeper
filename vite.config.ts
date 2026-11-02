import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import path from "path";

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  clearScreen: false,
  build: {
    // 桌面端资源从本地磁盘加载，大 chunk 不影响启动速度，放宽警告阈值
    chunkSizeWarningLimit: 1500,
  },
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      ignored: ["**/src-tauri/target/**", "**/node_modules/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_"],
});
