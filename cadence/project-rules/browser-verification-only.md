# 浏览器验证规则（禁用 Playwright，只用 harness 内置浏览器）

> 适用：所有会话（主会话与 subagent）。本项目**不使用 Playwright** 做任何浏览器验证。

## 规则

1. **🔴 禁止使用 Playwright**。包括但不限于：
   - `pnpm test:e2e` / `pnpm exec playwright test` / 任何 `playwright` CLI 调用
   - 代码或脚本里 `import { … } from "@playwright/test"` / `from "playwright"` / `require("playwright")`
   - `pnpm exec playwright install`（下载浏览器二进制）
   - 为跑通 Playwright 而临时改 `web/playwright.config.ts` 或新增临时配置

2. **需要浏览器能力时，只用 harness 内置的浏览器能力**：
   - 通过 Eval 的 `browser` 全局对象驱动真实 Chromium：`browser.open({ name, url })` → `tab.observe()` / `tab.click()` / `tab.fill()` / `tab.type()` / `tab.screenshot()` / `tab.text()` / `tab.run(fn)` / `tab.close()`
   - 用法文档：`read xd://eval/browser`（**动手前先读**）
   - 内置浏览器会自动选择并按需安装 managed Chromium，**不需要也不允许**自行下载浏览器

3. **仓库既有的 Playwright 资产保持原样，不删不改**：
   - `web/playwright.config.ts`、`web/e2e/**` 是 CI / 人工环境的既有资产
   - 本规则约束的是**本项目会话的行为**，不是仓库资产的存在与否
   - ⛔ 不因为「本项目禁用 Playwright」而删除或改写这些文件

4. **内置浏览器能力不可用时，如实报出缺口**，说明：缺什么、为什么不能用、需要谁补。
   - **禁止**以「Playwright 跑不通」为理由退回 Playwright
   - **禁止**把浏览器验证悄悄降级成「只读代码断言」后声称「已验证界面」

5. **真实界面验证的证据要求**：
   - 用 `tab.observe()` 取可交互元素与结构、`tab.screenshot()` 取视觉证据
   - 结论必须来自**真实渲染与真实点击**，不是从源码推断
   - 截图作为交付证据保留到 `cadence/`（例外见 `cadence/project-rules/test-content-tmp-only.md`：`cadence/reports/` 为正式验收证据目录）

6. **启动被测服务**：需要前后端联调时，用 `bash` 工具的具名长驻服务（`name` + `ready.port`）分别起 API 与前端 dev server，再用内置浏览器连上去。
   - 测试产物（临时工作区、构建目录、日志）仍只能落在 `/tmp`（见 `cadence/project-rules/test-content-tmp-only.md`）

## 背景记录

2026-11-04 会话实录：收口阶段要跑 `web/e2e/**` 的端到端用例，先撞 `playwright.config.ts` 的 `channel: "chrome"`（本机无 Google Chrome），去掉 channel 后撞缺 `chromium_headless_shell-1217`，再撞缓存只有 `chromium-1226`（版本不匹配），最后下载 170 MiB 浏览器仍因超时中断在解压阶段 —— **绕了四步都没跑起来**；而 harness 内置浏览器能力本可直接驱动真实 Chromium。

同场还发现两条相关教训，一并固化：
- **e2e 失败要先看真实报错**（`browserType.launch: Executable doesn't exist at …`），不要只看「全 F」的现象列表
- **测试的「环境不可完成」结论要逐条实证** —— 本机四家 provider（pi / codex / kimi / claude）与 codegraph **均已安装**，先前把 15 条真机验收项整体标成「本轮环境不可完成」是错的，正确表述是「尚未执行」
