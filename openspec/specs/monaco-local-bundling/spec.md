# monaco-local-bundling Specification

## Purpose
规范 web 前端 monaco 编辑器的本地化打包：monaco 资源与 worker 一律从同源本地产物加载，运行时不依赖外部 CDN（无外网环境下 Artifact 产物区仍可用）；`loader.config` 先于任何 Editor/DiffEditor 执行；编辑器 UI 能力与实际用到的语言高亮不相对 CDN 基线退化；以行为契约测试锁住「不走 CDN」；二进制体积增量如实记录。

## Requirements

### Requirement: monaco 资源同源本地加载且运行时不依赖外部 CDN（REQ-MONACO-01）
系统 SHALL 从同源本地产物加载 monaco 编辑器（`monaco-editor@0.55.1` 与 `@monaco-editor/react@4.7.0` 的本地依赖，经 `loader.config({ monaco })` 注入），运行时 SHALL NOT 依赖任何外部 CDN。系统 SHALL NOT 将 `paths.vs` 设为任何 `http(s)://` 外部地址，SHALL NOT 回落到 `@monaco-editor/react` / `monaco-editor` 默认的 `https://cdn.jsdelivr.net/npm/monaco-editor@0.55.1/min/vs`。本需求 SHALL NOT 新增任何依赖，SHALL NOT 改变 `MonacoViewer` / `MonacoDiffViewer` 的 props 与对外行为。

#### Scenario: 加载产物审核页零 CDN 请求

- **WHEN** 用户在 4317 发布态加载产物审核页（含 Artifact 产物区）
- **THEN** 浏览器网络面板 SHALL 出现 **0 个** `cdn.jsdelivr.net` 请求（基线为 12 个：`loader.js` / `editor.main.js` / `nls.messages-loader.js` / 4 个 `monaco.contribution-*.js` / `basic-languages/monaco.contribution.js` / `editor.api-*.js` / `workers-*.js` / `editor.main.css` / `markdown-*.js`）
- **AND** monaco 所需的全部脚本、样式与 worker SHALL 来自同源本地产物

#### Scenario: 无外网环境 Artifact 产物区可用

- **WHEN** 运行环境完全无外网（含内网环境）且用户打开 Artifact 产物区
- **THEN** monaco SHALL 正常初始化并渲染产物内容
- **AND** SHALL NOT 出现「monaco 永不初始化 ⇒ 产物区白屏」

#### Scenario: 不新增依赖且 props 不变

- **WHEN** 本需求落地
- **THEN** `package.json` 的依赖 SHALL 逐字不变（monaco 早已是本地依赖）
- **AND** `MonacoViewer` / `MonacoDiffViewer` 的 props、UI 与 `web/dist` 结构 SHALL NOT 因本需求改变

### Requirement: loader.config 必须先于任何 Editor/DiffEditor 组件执行（REQ-MONACO-02）
`loader.config({ monaco })` SHALL 在任何使用 `Editor` / `DiffEditor` 的组件执行**之前**生效（`@monaco-editor/react` 官方 `setupMonaco` 模式）。承载 `loader.config` 与 `MonacoEnvironment` 的 setup 模块（`web/src/components/shared/monaco-local-setup.ts`）SHALL 被 `MonacoViewer.tsx` 与 `MonacoDiffViewer.tsx` 以副作用 import（`import "./monaco-local-setup";`）引入，且该 import SHALL 位于各自 `lazy(() => import("@monaco-editor/react"))` **之前**。`web/tsconfig.json` SHALL 声明 `"types": ["vite/client"]`，使 `?worker` import 的类型声明可解析（缺失将报 TS2307）。

#### Scenario: 两个查看器均先于 Editor 加载本地配置

- **WHEN** `MonacoViewer` 或 `MonacoDiffViewer` 被首次渲染并触发 `@monaco-editor/react` 的懒加载
- **THEN** `loader.config({ monaco })` SHALL 已经执行完毕
- **AND** `@monaco-editor/react` SHALL NOT 在 `loader.config` 之前初始化 loader

#### Scenario: 类型检查可解析 worker import

- **WHEN** 执行 `pnpm tsc -b`
- **THEN** `monaco-local-setup.ts` 中的 `?worker` import SHALL 无 TS2307 错误
- **AND** 通过结果 SHALL 由 `web/tsconfig.json` 的 `"types": ["vite/client"]` 提供

### Requirement: worker 按 label 从本地 worker 分发（REQ-MONACO-03）
`window.MonacoEnvironment.getWorker` SHALL 按 label 从**本地**打包的 worker 分发：`json` ⇒ `JsonWorker`；`css` / `scss` / `less` ⇒ `CssWorker`；`html` / `handlebars` / `razor` ⇒ `HtmlWorker`；`typescript` / `javascript` ⇒ `TsWorker`；其余默认 ⇒ `EditorWorker`。五类 worker SHALL 通过 `monaco-editor/esm/vs/**` 下对应 `*.worker` 的 `?worker` 导入本地构建，SHALL NOT 从外部 URL 取得。worker 与语言 chunk SHALL 按需加载：只读展示场景 SHALL NOT 加载 ts / css / html / json 语言服务 worker。

#### Scenario: 各 label 命中对应 worker

- **WHEN** monaco 以 label `json` / `css` / `scss` / `less` / `html` / `handlebars` / `razor` / `typescript` / `javascript` 或其它任意 label 请求 worker
- **THEN** `getWorker` SHALL 分别返回 `JsonWorker` / `CssWorker`（css、scss、less）/ `HtmlWorker`（html、handlebars、razor）/ `TsWorker`（typescript、javascript）/ `EditorWorker`（默认）的实例

#### Scenario: worker 在发布态真起得来

- **WHEN** 在 4317 发布态加载 markdown 与 json 的产物内容
- **THEN** 对应 worker SHALL 真实启动并为同源本地资源，浏览器控制台 SHALL NOT 出现 worker 创建失败
- **AND** 该条 SHALL 在实施时于 4317 发布态实测，SHALL NOT 以 vite dev server 上的 spike 结果替代

#### Scenario: 只读展示场景不加载语言服务 worker

- **WHEN** 用户仅查看只读产物内容
- **THEN** ts / css / html / json 语言服务 worker SHALL NOT 被加载（按需 chunk 机制，二进制增量 ≠ 用户下载增量）

### Requirement: 编辑器 UI 能力与实际用到的语言高亮不得退化（REQ-MONACO-04）
本地打包后，编辑器 UI 能力与项目实际用到的语言高亮 SHALL NOT 相对基线（CDN 加载）退化；产品 diff 功能 SHALL 保持不变；系统 SHALL NOT 裁剪语言，SHALL NOT 引入「靠人工维护语言清单」的机制。`toml` / `plaintext` / `diff` 在 monaco 中不存在，基线下亦无高亮，其无高亮 SHALL NOT 视为退化。逐条核对项见下列场景。

#### Scenario: markdown 与 json 有语法高亮

- **WHEN** 在 4317 发布态打开 Artifact 面板的 markdown 与 json 产物
- **THEN** 两者 SHALL 均有语法高亮（token class 断言）
- **AND** json 高亮的运行时效果 SHALL 实测确认，SHALL NOT 仅凭产物含 `json.worker` + `jsonMode` 推断

#### Scenario: 实际用到的语言逐条保留

- **WHEN** 检查 `CodingWorkspaceArtifacts.tsx:309` 的 `languageForPath` 所用语言 `python` / `typescript` / `javascript` / `json` / `markdown` / `rust` / `toml` / `yaml` / `shell` / `css` / `html` / `plaintext`，以及 `WorkItemPlanArtifactContent.tsx:36` / `:500` 的 `language="json"`、`ArtifactPane.tsx:280` / `LifecycleCardDrawer.tsx:352,358,401` 的 markdown、`CodingWorkspaceArtifacts.tsx:176` 的 `language="diff"`
- **THEN** 其中 monaco 实际存在的语言（`python` / `typescript` / `javascript` / `json` / `markdown` / `rust` / `yaml` / `shell` / `css` / `html`）的高亮 SHALL NOT 少于基线（CDN 加载）

#### Scenario: toml plaintext diff 无高亮不算退化

- **WHEN** 内容语言为 `toml` / `plaintext` / `diff`
- **THEN** 其无高亮 SHALL 与基线一致（monaco 中不存在这三种语言，基线亦无高亮）
- **AND** SHALL NOT 将其记为本 change 引入的退化

#### Scenario: 代码 diff 双栏对比正常

- **WHEN** 用户在代码 diff 场景下使用 `MonacoDiffViewer`
- **THEN** original / modified 双栏对比 SHALL 正常渲染
- **AND** 「显示 Diff」按钮与版本对比 SHALL 保持既有行为
- **AND** 该功能来自 `editor.all.js` 的 `diffEditor.contribution.js`，任何方案都 SHALL NOT 改动它

#### Scenario: 编辑器 UI 能力在位

- **WHEN** 用户在编辑器内执行查找、打开右键菜单、复制
- **THEN** 三项能力 SHALL 均可用，括号匹配、格式化、折叠 SHALL NOT 丢失
- **AND** 这些能力 SHALL 来自 `editor.all.js`（65 import，含 find / contextmenu / clipboard / bracketMatching / format / folding 等）

### Requirement: 契约测试必须以行为方式锁住「不走 CDN」（REQ-MONACO-05）
系统 SHALL 提供 `loader.config` 契约测试：mock `@monaco-editor/react` 的 `loader`，断言 `loader.config` 被调用，且传入的 `monaco` 来自**本地 `monaco-editor` 包**，且**没有任何外部 `paths.vs`**（未被设为任何 `http(s)://` 外部地址）。该测试 SHALL 为**行为断言**，SHALL NOT 断言源码文本（不得读取 `.ts` / `.tsx` 源文件内容做字符串匹配、不得对 import 语句或配置字面量做 grep 式断言）。该测试 SHALL 先于实现编写并先见红（TDD）。

#### Scenario: loader.config 被调用且使用本地 monaco

- **WHEN** 加载 setup 模块（`monaco-local-setup`）
- **THEN** 被 mock 的 `loader.config` SHALL 被调用
- **AND** 传入参数中的 `monaco` SHALL 来自本地 `monaco-editor` 包

#### Scenario: 无外部 paths.vs

- **WHEN** 检查 `loader.config` 的全部调用实参
- **THEN** 任一调用实参 SHALL NOT 含 `paths.vs` 或任何 `http(s)://` 外部地址

#### Scenario: 测试不得断言源码文本

- **WHEN** 审查该契约测试
- **THEN** 其断言 SHALL 仅针对运行时行为（mock 的调用与实参）
- **AND** SHALL NOT 出现对源码文本的读取与匹配

#### Scenario: 先红后绿

- **WHEN** 实施前运行该契约测试
- **THEN** 它 SHALL 失败（`loader.config` 未被调用，`monaco-local-setup` 尚不存在）
- **AND** 实施 4 处改动后 SHALL 转绿

### Requirement: 二进制体积增量必须被记录（REQ-MONACO-06）
本 change 带来的二进制体积增量 SHALL 被如实记录并随交付说明披露：`Cargo.toml` 为 `rust-embed = { version = "8", features = ["debug-embed"] }`，debug 也嵌入 ⇒ `web/dist` 涨多少、二进制就涨多少。记录 SHALL 至少包含：二进制 **+13.6 MB**（`web/dist` 1.43 MB → 15.04 MB；227.8 MB → ~241 MB debug）；主 chunk 5,179 kB / gzip 1,363.97 kB；worker + 语言 chunk 合计 9.4 MB（其中 `ts.worker` 单独 7.02 MB）；用户首屏下载（gzip）仅 **+25.7 kB**（1,338 → 1,364 kB）；`pnpm build` 3 s → ~15 s。系统 SHALL NOT 声称本 change 减小体积。

#### Scenario: 体积增量写入交付说明

- **WHEN** 本 change 交付
- **THEN** 交付说明 SHALL 如实记录二进制 +13.6 MB、用户首屏 gzip +25.7 kB、构建时间 3 s → ~15 s
- **AND** SHALL NOT 将体积变化表述为减小或持平

#### Scenario: 实施后体积复核

- **WHEN** 实施完成并执行 `pnpm build`
- **THEN** `web/dist` 体积 SHALL 与记录值同量级（1.43 MB → 15.04 MB）
- **AND** 偏差显著时 SHALL 更新记录并向 owner 说明
