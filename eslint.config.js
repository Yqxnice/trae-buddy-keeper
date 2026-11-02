import js from "@eslint/js";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";

export default tseslint.config(
  { ignores: ["dist", "src-tauri", "node_modules"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  reactHooks.configs.flat["recommended-latest"],
  {
    rules: {
      // 项目约定：invoke 错误处理统一用 catch (e: any)，关闭显式 any 限制
      "@typescript-eslint/no-explicit-any": "off",
      // 各页面采用 keep-alive 常驻 + 挂载即拉取数据的模式（load 内同步 setLoading），
      // 该新规则要求全部改为异步订阅式，与现有架构冲突，按项目约定关闭
      "react-hooks/set-state-in-effect": "off",
      // 枚举值/类型导入混用场景下更省事，按需关闭
      "@typescript-eslint/consistent-type-imports": "off",
    },
  }
);
