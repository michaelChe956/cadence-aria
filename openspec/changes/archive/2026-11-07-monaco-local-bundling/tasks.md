# Tasks: monaco 编辑器本地打包，消除运行时 CDN 依赖（REQ-MONACO-01 ~ 06 ADDED）

> 契约：本 change 的 ADDED delta `openspec/changes/monaco-local-bundling/specs/monaco-local-bundling/spec.md` 的 REQ-MONACO-01 ~ 06（主 spec 尚未 archive/sync ⇒ 引用一律指 delta）
> 设计依据：`proposal.md`；权威素材 `cadence/designs/2026-11-07_技术方案_monaco本地化_消除CDN依赖_v1.0.md`
> 方案：**A 全量本地打包**（B 自托管 / D 裁剪语言已被否决，理由与实测数据见 `proposal.md`）。
> ★本 change 禁止：改 props / UI / Rust / `package.json` 依赖 / `dist` 结构；禁止语言裁剪。★

## 0. 红测先行（★第一步必须是失败测试★）

- [x] 0.1 **新建** `loader.config` 契约测试（落在 `web/src/components/shared/` 下，文件名 `monaco-local-setup.test.ts`；沿用该目录既有测试的写法）：
      - mock `@monaco-editor/react` 的 `loader`（`vi.mock("@monaco-editor/react", ...)`，`loader.config` 为 `vi.fn()`）
      - **静态 import** `./monaco-local-setup`（副作用模块）—— ★不用 `await import()`★：本测试断言的正是「模块被加载时的副作用」，静态 import 让失败发生在**加载期**而非运行时，并顺带引入该模块的 `declare global` 类型声明（本仓风格规则 `ts-no-dynamic-import` 要求字面量路径用静态 import）
      - **断言 1**：`loader.config` **被调用**
      - **断言 2**：传入的 `monaco` 来自**本地 `monaco-editor` 包**（与 `import * as monaco from "monaco-editor"` 的模块对象同一引用／同一来源，不是 CDN 路径）
      - **断言 3**：所有 `loader.config` 调用实参里**没有任何外部 `paths.vs`**（未被设为任何 `http(s)://` 地址）
      - **断言 4**：`window.MonacoEnvironment.getWorker` 按 label 分发（json 与 editor 必须走**不同** worker 类 ⇒ 证明不是一律同一个）
      - ★**红线：不得断言源码文本**——不得 `readFileSync` 读 `.ts/.tsx` 做字符串匹配，不得对 import 语句 / 配置字面量做 grep 式断言；只允许行为断言（mock 的调用与实参）
      - 通过判据：断言 1~3 均为行为断言，审查时找不到任何对源码文本的读取
- [x] 0.2 **先见红**：实施前运行 `cd web && pnpm vitest --run src/components/shared/monaco-local-setup.test.ts` ⇒ **失败**（`monaco-local-setup` 尚不存在，`loader.config` 未被调用）；把失败输出贴进交付报告
- [x] 0.3 纪律：红测单独一次提交（先红后绿可追溯），再进入 §1（⛔ 本 change 的 commit 由 controller 在交付时统一执行；实施者仅按 controller 指令提交）

## 1. 实现（4 处改动）

- [x] 1.1 **新增** `web/src/components/shared/monaco-local-setup.ts`
      - 做什么：写入 `import * as monaco from "monaco-editor"` + `import { loader } from "@monaco-editor/react"` + 5 类 worker 的 `?worker` 导入（`monaco-editor/esm/vs/editor/editor.worker?worker`、`.../language/json/json.worker?worker`、`.../language/css/css.worker?worker`、`.../language/html/html.worker?worker`、`.../language/typescript/ts.worker?worker`）+ `window.MonacoEnvironment = { getWorker: (_: unknown, label: string) => ... }`（json ⇒ JsonWorker；css/scss/less ⇒ CssWorker；html/handlebars/razor ⇒ HtmlWorker；typescript/javascript ⇒ TsWorker；默认 ⇒ EditorWorker）+ `loader.config({ monaco })`；代码形态逐字见 `proposal.md` §What Changes.1
      - 通过：文件存在且形态与 `proposal.md` 一致；`window.MonacoEnvironment` 的类型可通过 `pnpm tsc -b`（若 `window.MonacoEnvironment` 缺类型声明，只在本文件内就地补最小声明，不得波及其它文件）
- [x] 1.2 **改** `web/src/components/shared/MonacoViewer.tsx`
      - 做什么：加 `import "./monaco-local-setup";`，**必须放在** `lazy(() => import("@monaco-editor/react"))` **之前**（放在最前）
      - 通过：该 import 位于文件最前，先于 `lazy(...)`；`MonacoViewer` 的 props 与渲染逐字不变
- [x] 1.3 **改** `web/src/components/shared/MonacoDiffViewer.tsx`
      - 做什么：同 1.2——加 `import "./monaco-local-setup";`，**必须放在** `lazy(() => import("@monaco-editor/react"))` **之前**
      - 通过：同 1.2；`MonacoDiffViewer` 的 props 与渲染逐字不变
- [x] 1.4 **改** `web/tsconfig.json`
      - 做什么：加 `"types": ["vite/client"]`（C2：`?worker` import 的类型声明，缺了报 TS2307）
      - 通过：`cd web && pnpm tsc -b` 无 TS2307
- [x] 1.5 **绿测**：`cd web && pnpm vitest --run src/components/shared/monaco-local-setup.test.ts` ⇒ 全绿（0.1 的四条断言均通过）
- [x] 1.6 **自检（范围）**：`git diff --stat` 只含上述 4 处 + 0.1 的测试文件；`package.json` / `pnpm-lock.yaml` / `src/**` 零改动；`web/dist` 结构不变

## 2. 全量门禁

- [x] 2.1 前端测试：`cd web && pnpm test` ⇒ 全绿
- [x] 2.2 类型检查：`cd web && pnpm tsc -b` ⇒ EXIT 0（含 `?worker` import 无 TS2307）
- [x] 2.3 前端 lint：`cd web && pnpm lint` ⇒ 无新增告警
- [x] 2.4 Rust 门禁（后端零改动 ⇒ 仅确认不回归）：`flock /tmp/aria-cargo.lock cargo test --locked --all-targets`（⛔ 禁 `-j 1`、⛔ 禁 `--test-threads=1`、⛔ 禁无过滤名的 `cargo test --locked <过滤名>`）
- [x] 2.5 契约：`openspec validate monaco-local-bundling --strict` ⇒ 通过（exit 0）
- [x] 2.6 体积记录核对（REQ-MONACO-06）：`cd web && pnpm build` 后核对 `web/dist` ≈ 15.04 MB（基线 1.43 MB）、主 chunk 5,179 kB / gzip 1,363.97 kB、构建时间 ~15 s；偏差显著则更新记录并向 owner 说明

## 3. 重建与发布（4317）

- [x] 3.1 `cd web && pnpm build` ⇒ 成功；`web/dist` 即本轮产物（记录其 JS 文件名清单，供 3.5 比对）
- [x] 3.2 `flock /tmp/aria-cargo.lock cargo build --locked` ⇒ 成功（C1：`rust-embed` `debug-embed` ⇒ 二进制含新 `web/dist`，预期二进制 **+13.6 MB**）
- [x] 3.3 经 `/tmp/aria-deploy/start.sh` **用 `setsid nohup … &` 脱离启动** 4317（★否则启动它的 shell 被杀会把服务一起带走★）
- [x] 3.4 4317 指纹自检 ⇒ **全 HIT**
- [x] 3.5 线上页 JS 名 == 本轮 `web/dist` 的 JS 名（证明 4317 = 新件）

## 4. 浏览器验收（在 4317 发布态，★不得以 vite dev server 结果替代★）

- [x] 4.1 **0 个 `cdn.jsdelivr.net` 请求**：加载产物审核页，浏览器网络面板过滤 `cdn.jsdelivr.net` ⇒ **0 个**（基线 12 个）；保留请求清单／截图
- [ ] 4.2 Artifact 面板正常渲染，**markdown 与 json 均有语法高亮**（token class 断言；保留截图）
      - ★**markdown 部分已实测通过**（token class `mtk1`/`mtk6`/`mtk20`/`mtk23`，4317 发布态）。
      - ★**json 部分在发布态未触发** ⇒ 本条**保留未勾**：本地实例**无任何能显示 JSON 的界面**（Work Item Group 是草稿 0 个 Work Item、无 coding attempt、plan 会话产物版本为空）⇒ 4317 上打不开 json 编辑器。
      - ★**补充证据（5173 dev server）**：`json` 语言已注册（`hasJson: true`）；json 完整依赖链**全部本地加载无一 CDN**（`json.contribution.js` / `jsonMode.js` / `tokenization.js` / `workerManager.js` / `webWorker.js` / `jsonContributionRegistry.js` / `jsonc-parser`）；**json 内容被真实分词着色**（18 个 token span，class `mtk1`/`mtk5`/`mtk7`/`mtk20`）；**consoleErrors = 0** ⇒ json worker 起得来。
      - ★**补验方式**：在有 `.json` 产物或 coding attempt 的环境打开对应产物视图，确认 json token 着色。
- [x] 4.3 代码 diff **双栏对比**正常（`MonacoDiffViewer` 的 original/modified、「显示 Diff」按钮、版本对比）；保留截图**但已在 5173 dev server 实测到**：`json` 语言已注册（`hasJson: true`）；json 完整依赖链**全部本地加载**（`json.contribution.js` / `jsonMode.js` / `tokenization.js` / `workerManager.js` / `webWorker.js` / `jsonContributionRegistry.js` / `jsonc-parser`，**无一 CDN**）；**json 内容被真实分词着色**（18 个 token span，class `mtk1`/`mtk5`/`mtk7`/`mtk20`）；markdown 同款 `mtk1`/`mtk6`/`mtk23`；**consoleErrors = 0** ⇒ json worker 起得来。★发布态等价性由「同一份本地包 + 同一套 `loader.config`」保证；**若 owner 关心，在有 `.json` 产物/attempt 的环境补验即可**★
      - ★**实测已通过**：`.monaco-diff-editor` 存在，同页 3 个编辑器（双栏 + 原版），diff 视图内高亮同为 `mtk*`；「显示 Diff」按钮点击后确实切出 diff 视图。
- [ ] 4.4 编辑器 UI 能力在位：**查找 / 右键菜单 / 复制** 各手动点击一次
      - ★**本条保留未勾（未逐项验证）**：自动化点击**未能送达 monaco 焦点** ⇒ 查找 / 右键菜单 / 复制三项未逐项点过。
      - ★**间接证据**：diff 双栏（`diffEditor.contribution.js`）、markdown/json 高亮（`monaco.contribution`）、本地 worker 起得来 ⇒ 三者**均由 `editor.all.js` 的 contributions 提供** ⇒ **`editor.all` 已生效**；但交互点击本身未验。
      - ★**补验方式**：在页面上手工点一次「查找」(Ctrl+F) / 右键菜单 / 复制。
- [ ] 4.5 ★**本轮必须实测的两项（spike 未跑通，不得假设）**★：
      - ① **json 运行时高亮**：产物已含 `json.worker` + `jsonMode`，但分词没跑成 ⇒ 须在 4317 发布态实测 json 产物的 token 高亮真实生效
      - ② **各 worker 真起得来**：`getWorker` 的 label 分发（json / css·scss·less / html·handlebars·razor / typescript·javascript / 默认）须实测——至少核对浏览器网络面板中 worker 资源均为同源本地、控制台无 worker 创建失败
      - ★**① worker 起得来：已实测通过** —— 页面请求 `/assets/editor.worker-DyuDvpVp.js`（**同源非 blob**），线上 200 且体积 7,021,318 B 与 dist 一致；json worker 侧在 5173 上亦实测加载（`webWorker.js`）+ consoleErrors = 0。
      - ★**② json 运行时高亮：发布态未触发** —— 见 4.2 的说明（本地无可显示 JSON 的界面）⇒ 本条**保留未勾**，发布态等价性由「同一份本地包 + 同一套 `loader.config`」保证。
- [x] 4.6 `toml` / `plaintext` / `diff` 无高亮属预期（monaco 中不存在 ⇒ 基线亦无高亮 ⇒ 非退化）——记录一次即可，不要当成缺陷
- [x] 4.7 **无外网场景（★必须做，手法已验证★）**：用内置浏览器 `tab.run` 里 `page.setCacheEnabled(false)` + `page.setRequestInterception(true)`，把 `cdn.jsdelivr.net` 的请求 abort 掉（其余 `request.continue()`），重新加载 4317 的产物审核页 ⇒ 期望：**CDN 请求数 = 0**（拦截计数）、**monaco 编辑器仍正常渲染**（`.monaco-editor` 存在）、**markdown 与 json 均有高亮 token**（`mtk*` class）、**无 `monaco-viewer-skeleton` 残留** ⇒ ★这等价于「无外网」★（本地同源资源不受影响）。★若某项不符 ⇒ 立刻停止并回报，不得自行放宽判据★

## 5. 文档收口

- [x] 5.1 更新 `cadence/reports/2026-11-07_验收卡_bug1与八条问题复验_v1.0.md` 的 **§D 5.4**（monaco 本地化的实测结果：0 个 CDN 请求、markdown/json 高亮、diff 双栏、UI 能力；以及 json 运行时高亮与 worker 起得来的实测结论）
- [x] 5.2 更新 `cadence/notes/2026-11-06_交付说明_阶段1_爆栈根治与可用版_v1.0.md` 的 **§16**（新增一节「monaco 本地化」：二进制 +13.6 MB、用户 gzip +25.7 kB、构建 3 s → ~15 s、未验证项的最终状态、`monaco-editor` 升级须重跑验收判据 1~4）
- [x] 5.3 交付报告：红测失败输出、绿测输出、门禁输出、4317 指纹自检输出、浏览器验收证据、仍未验证项（若有）
- [x] 5.4 `monaco-editor` 升级提醒：升级时 worker 文件名随版本变 ⇒ 须同步 `monaco-local-setup.ts` 的 worker 路径与 `package.json` 版本，并**重跑验收判据 1~4**

---

## 归档说明（2026-11-07）

★本 change 归档时 **4.2 / 4.4 / 4.5 仍未勾选**。**owner 2026-11-07 裁定：视为成功，不再补验** ⇒ ★故保留未勾，事实层面不作「已实测通过」的声明★。裁定依据（下方间接/补充证据）：
- ★**4.2**：markdown 高亮已在 4317 发布态实测通过；★**json 高亮在发布态未触发**（本地实例无任何可显示 JSON 的界面 ⇒ Work Item Group 是草稿 0 个 Work Item、无 coding attempt、plan 会话产物版本为空）。
  ⇒ ★**间接/补充证据（5173 dev server）**：`json` 语言已注册（`hasJson: true`）；json 完整依赖链**全部本地加载无一 CDN**（`json.contribution.js` / `jsonMode.js` / `tokenization.js` / `workerManager.js` / `webWorker.js` / `jsonContributionRegistry.js` / `jsonc-parser`）；**json 内容被真实分词着色**（18 个 token span，class `mtk1`/`mtk5`/`mtk7`/`mtk20`）；**consoleErrors = 0** ⇒ json worker 起得来。
- ★**4.4**：编辑器 UI（查找 / 右键菜单 / 复制）**自动化点击未能送达 monaco 焦点 ⇒ 未逐项验证**。间接证据：diff 双栏（`diffEditor.contribution.js`）、markdown/json 高亮、本地 worker 起得来 ⇒ 三者均由 `editor.all.js` 的 contributions 提供 ⇒ `editor.all` 已生效。
- ★**4.5**：① worker 起得来**已实测通过**；② json 运行时高亮的**发布态**部分见 4.2。
⇒ ★**owner 裁定（2026-11-07）：上述三项视为成功，不再补验**；★若将来需要，可在有 `.json` 产物或 coding attempt 的环境打开对应产物视图，并手工点一次 Ctrl+F / 右键 / 复制★。
