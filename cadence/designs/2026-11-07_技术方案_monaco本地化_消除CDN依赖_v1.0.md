# 技术方案 · monaco 本地化（消除 CDN 依赖）(2026-11-07)

> **一句话**：monaco 编辑器目前从 `cdn.jsdelivr.net` 动态加载 ⇒ **无外网/内网环境永远加载不出来** ⇒ Artifact 产物区白屏；本方案把 monaco 改为**本地打包**（`loader.config({ monaco })`），彻底去掉对外网的运行时依赖。
> **决策来源**：owner 2026-11-07 要求「这个问题要想办法解决一下」→ 选「先做可行性验证再定方案」→ 两轮 spike 实测后定 **方案 A（全量本地打包）**。

---

## 1. 意图与 why

- **问题**：`@monaco-editor/react` 不配置 `loader.config` 时，由 **`monaco-editor` 包自己的 loader**（`esm/vs/loader.js`）把 `paths.vs` 写成 `https://cdn.jsdelivr.net/npm/monaco-editor@0.55.1/min/vs` ⇒ 运行时**必须能出网**。
- **实测证据**（2026-11-07，浏览器网络面板，4317 实例）：产物审核页加载时发出 **12 个 `cdn.jsdelivr.net` 请求**（`loader.js` / `editor.main.js` / `nls.messages-loader.js` / 4 个 `monaco.contribution-*.js` / `basic-languages/monaco.contribution.js` / `editor.api-*.js` / `workers-*.js` / `editor.main.css` / `markdown-*.js`）。
- **后果**：无外网 ⇒ monaco 永不初始化 ⇒ **Artifact 产物区空白**（本次已在 `4a89c6ea` 用「正在加载内容…」可见提示兼顶，但**编辑器仍然不可用**）。
- **关键事实**：`monaco-editor@0.55.1` 与 `@monaco-editor/react@4.7.0` **早已作为本地依赖装在 `package.json` 里**，但**本地那份从未被用上** ⇒ 本方案不需要新增任何依赖。

## 2. 目标 / 非目标 / 反目标

| | 内容 |
|---|---|
| **目标** | ① 运行时**零**外部 CDN 依赖；② 与当前基线**功能完全等价**（编辑器 UI 能力、全部语言、全部语言服务、worker 全部保留）；③ 改动面最小；④ 有测试锁住「不再走 CDN」这条契约 |
| **非目标** | ① 不做 monaco 升级/换版本；② 不改 `MonacoViewer` / `MonacoDiffViewer` 的 props 与对外行为；③ 不动 Rust 侧任何代码；④ 不追求减小二进制体积（见 §5 代价） |
| **反目标（做成即失败）** | ① 任何**语言**在无外网时失去语法高亮；② 编辑器 UI（查找/右键菜单/剪贴板/括号匹配/格式化/折叠）能力退化；③ 引入「靠人工维护语言清单」的机制（见 §4 D 的实测教训） |

## 3. 约束

| 约束 | 事实 |
|---|---|
| **C1 前端 bundle 烤进二进制** | `Cargo.toml`：`rust-embed = { version = "8", features = ["debug-embed"] }` ⇒ **debug 也嵌入** ⇒ `web/dist` 涨多少，二进制就涨多少（release 另加 `strip`/`lto`/`opt-level=s`） |
| **C2 `?worker` import 需要类型** | `web/tsconfig.json` 未引 `vite/client` ⇒ `import ... from "monaco-editor/esm/vs/editor/editor.worker?worker"` 报 TS2307 ⇒ **必须**加 `"types": ["vite/client"]`（**任何方案都要**） |
| **C3 monaco 的 ESM 无聚合入口** | `esm/vs/basic-languages/` 只有 `_.contribution.js`（工具）+ 81 个语言目录，**没有** `monaco.contribution.js` 聚合入口 ⇒ 裁剪路线必须自己枚举 ⇒ 见 §4 |
| **C4 用户下载 ≠ 二进制体积** | worker 与语言 chunk **按需加载** ⇒ 二进制增量不等于用户下载增量（实测 gzip 差 **25.7 kB**，见 §5） |

## 4. Approaches（三条路，实测后选 A）

| | 做法 | 实测 dist 合计 | 二进制增量 | 功能等价 | 结论 |
|---|---|---|---|---|---|
| **A 全量本地打包** ✅ 选中 | `loader.config({ monaco })` + `import * as monaco from "monaco-editor"` + `MonacoEnvironment.getWorker`（本地 worker） | **15.04 MB** | **+13.6 MB** | **完全等价** | **改动最小、零漏项**；代价只在二进制（见 §5） |
| **B 自托管 `min/vs`** | `loader.config({ paths: { vs: "/monaco/vs" } })` + 构建期把 `node_modules/monaco-editor/min/vs`（16 MB / 121 文件）拷进 `web/public` | 未实测 | **+16 MB** | 等价 | 需给 `build` 脚本加拷贝步骤（产物不入 git）；二进制比 A 还大 ⇒ **无收益** |
| **D 裁剪语言** | `editor.api` + `editor.all` + 手工/glob 挑语言 | D-2 5.67 MB / D-3 6.10 MB / D-6 6.10 MB | +4.2 ~ +4.7 MB | **有漏项** | ❌ **两次实测都漏**（见下）⇒ 否决 |

### 4.1 D 路线的两次实测漏项（否决理由）

| 尝试 | 漏项 | 性质 |
|---|---|---|
| D-2 / D-3（手工挑语言） | `toml` —— monaco 里**根本不存在**该语言（`basic-languages` 与 `language/` 都没有）⇒ 基线（CDN）下也**无高亮** ⇒ 不是退化，但证明**语言清单极易出错** | 手工清单风险 |
| D-6（`import.meta.glob(".../*/*.contribution.js", { eager: true })`） | **只匹配到一部分**：`rust`/`shell`/`yaml`/`python` 进了主 chunk，但 **`abap` 没有** ⇒ 各语言目录内文件名不一致 ⇒ **固定 glob 模式必然漏** | 模式匹配必然漏 |

⇒ ⇒ **结论**：语言丢失是**用户可见的功能退化**，而裁剪路线**两次都漏** ⇒ 选 A。

### 4.2 A 的具体做法（4 处改动）

1. **新增** `web/src/components/shared/monaco-local-setup.ts`：
   ```ts
   import * as monaco from "monaco-editor";
   import { loader } from "@monaco-editor/react";
   import EditorWorker from "monaco-editor/esm/vs/editor/editor.worker?worker";
   import JsonWorker from "monaco-editor/esm/vs/language/json/json.worker?worker";
   import CssWorker from "monaco-editor/esm/vs/language/css/css.worker?worker";
   import HtmlWorker from "monaco-editor/esm/vs/language/html/html.worker?worker";
   import TsWorker from "monaco-editor/esm/vs/language/typescript/ts.worker?worker";

   window.MonacoEnvironment = {
     getWorker: (_: unknown, label: string) => {
       if (label === "json") return new JsonWorker();
       if (label === "css" || label === "scss" || label === "less") return new CssWorker();
       if (label === "html" || label === "handlebars" || label === "razor") return new HtmlWorker();
       if (label === "typescript" || label === "javascript") return new TsWorker();
       return new EditorWorker();
     },
   };

   loader.config({ monaco });
   ```
   ★**必须**在任何使用 `Editor` 的组件**之前**被 import（`@monaco-editor/react` 官方 `setupMonaco` 模式）。
2. **改** `web/src/components/shared/MonacoViewer.tsx`：加 `import "./monaco-local-setup";`（放在最前，**先于** `lazy(() => import("@monaco-editor/react"))`）。
3. **改** `web/src/components/shared/MonacoDiffViewer.tsx`：同上。
4. **改** `web/tsconfig.json`：加 `"types": ["vite/client"]`（C2）。

⚠️ **不做**的事：不改任何 props / 不改 UI / 不改 `dist` 结构 / 不改 Rust / 不改 `package.json` 依赖（monaco 已在依赖里）。

## 5. 代价（诚实记录）

| 项 | 变化 |
|---|---|
| **二进制**（`web/dist` 被 `rust-embed` 烤进件里，C1） | **+13.6 MB**（227.8 MB → ~241 MB debug；release 同比例） |
| **用户首屏下载**（gzip 传输） | **+25.7 kB**（主 chunk gzip 1,338 → 1,364 kB）⇒ ★**几乎不变**★ |
| **按需 chunk** | worker/语言共 9.4 MB 进 `dist`（⇒ 进二进制）⇒ **只在实际用到该语言/服务时才下载** ⇒ 只读展示场景**永不加载** |
| **加载体验** | 变**快**（本地同源，无外网往返）⇒ ③ 的「正在加载内容…」提示几乎瞬间消失（正常；`4a89c6ea` 的修复在慢网/无外网场景仍有效） |
| **构建时间** | `pnpm build` 3 s → ~15 s |

⚠️ **若 owner 认为二进制 +13.6 MB 不可接受** ⇒ 退路是 §4 的 D-7（脚本生成完整 basic-languages 清单，需实测体积 + monaco 升级要重跑脚本）⇒ **本方案不推荐**，理由见 §4.1。

## 6. 留给实施者

- **TDD 先行**：先写**失败测试**锁住契约 —— mock `@monaco-editor/react` 的 `loader`，断言 `loader.config` 被调用且**传入的 `monaco` 来自本地 `monaco-editor` 包**（不是 CDN 路径），且 `paths.vs` **未被**设为任何 `http(s)://` 外部地址。★红线：不得断言源码文本★。
- **实施顺序**：红测 → 加 setup 模块 + 2 处 import + tsconfig → 绿测 → `pnpm build` → `pnpm test` / `pnpm tsc -b` / `pnpm lint` → `cargo build --locked` → 经 `/tmp/aria-deploy/start.sh`（**`setsid nohup` 脱离**）重启 4317 → 指纹自检 → 浏览器验收。
- **必须实测的两件事**（spike 未跑通，不得假设）：① **json 高亮运行时生效**（产物已含 `json.worker` + `jsonMode`，但分词未实测）；② **各 worker 真的起得来**（`getWorker` 的 label 分发）。
- **必须保留的证据**：浏览器网络面板 **0 个 `cdn.jsdelivr.net` 请求**；Artifact 面板 markdown/json 高亮正常；代码 diff 双栏正常。

## 7. 验收判据

| # | 判据 | 证据 |
|---|---|---|
| 1 | 加载产物审核页时 **0 个 `cdn.jsdelivr.net` 请求** | 浏览器网络面板（截图/请求清单） |
| 2 | Artifact 面板正常渲染，**markdown 与 json 均有语法高亮** | 截图（token class 断言） |
| 3 | 代码 diff **双栏对比**正常（`MonacoDiffViewer` 的 original/modified） | 截图 |
| 4 | 编辑器 UI 能力在位（查找/右键/复制） | 手动点击一次 |
| 5 | `loader.config` 契约测试绿（本地 monaco、无外部 `paths.vs`） | 测试输出 |
| 6 | 门禁全绿：`pnpm test` / `pnpm tsc -b` / `pnpm lint` / `cargo test --locked --all-targets` / `openspec validate --strict` | 命令输出 |
| 7 | 4317 = 新件（指纹自检全 HIT；线上页 JS 名 == 本轮 dist） | 自检输出 |

## 8. 诚实边界

- 本方案**不减小**二进制体积，反而**增大 13.6 MB**；换来的是「零外部依赖 + 零功能退化 + 零维护清单」。
- ★**离线/内网可用性是本方案的主要价值**：在完全无外网的环境里，产物审核从「白屏 + 编辑器不可用」变成「正常可用」★。
- `monaco-editor` 升级时，`monaco-local-setup.ts` 的 worker 路径与 `package.json` 版本需同步（worker 文件名随版本变）⇒ 升级时**必须**重跑验收判据 1~4。
- 本设计**未**验证：json/各语言的高亮在**生产构建**（非 dev）下的表现 —— spike 是在 vite dev server 上验的 ⇒ 实施时须在 **4317 发布态**复验。
