import { fileURLToPath } from "node:url";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vitest/config";
import { devServerProxy } from "./dev-server-proxy";

export default defineConfig({
  plugins: [react()],
  server: {
    proxy: devServerProxy,
  },
  test: {
    environment: "jsdom",
    exclude: ["e2e/**", "node_modules/**", "dist/**"],
    setupFiles: ["./src/test/setup.ts"],
    // ★测试环境把 monaco-editor 指向本地 stub。
    //   jsdom 未实现 `document.queryCommandSupported`，而 monaco 主包在模块求值期就会调用它
    //   ⇒ 只要测试间接 import `monaco-local-setup.ts`，整个文件会在收集阶段抛错。
    //   ★生产构建不受影响（`test.alias` 仅作用于 vitest）。
    //   ★`find` 用精确匹配：worker 子路径（`monaco-editor/esm/**?worker`）必须保持各自独立，
    //     否则 `monaco-local-setup.test.ts` 里 5 个 `vi.mock` 会解析到同一文件而互相覆盖。
    alias: [
      {
        find: /^monaco-editor$/,
        replacement: fileURLToPath(
          new URL("./src/test/monaco-stub.ts", import.meta.url),
        ),
      },
    ],
  },
});
