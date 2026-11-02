import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles/index.css";

// 屏蔽浏览器行为快捷键：刷新（F5/Ctrl+R）、开发者工具（F12/Ctrl+Shift+I/J/C）、
// 查找（Ctrl+F）、打印（Ctrl+P）、Ctrl+滚轮缩放。编辑类快捷键（复制/粘贴/撤销）不受影响。
window.addEventListener("keydown", (e) => {
  const key = e.key.toLowerCase();
  if (
    e.key === "F5" ||
    e.key === "F12" ||
    ((e.ctrlKey || e.metaKey) && (key === "r" || key === "f" || key === "p")) ||
    (e.ctrlKey && e.shiftKey && (key === "i" || key === "j" || key === "c"))
  ) {
    e.preventDefault();
  }
});
window.addEventListener(
  "wheel",
  (e) => {
    if (e.ctrlKey) e.preventDefault();
  },
  { passive: false }
);

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>
);
