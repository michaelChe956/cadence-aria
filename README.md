# Cadence Aria

AI 辅助的软件开发工作平台。Cadence Aria 把需求（Issue）系统地拆分为 Story Spec、Design Spec 与 Work Item，并通过统一的 Provider Workspace 与 Coding Workspace 完成生成、交叉审核、编码实现与验证回滚，让 AI 成为可追踪、可审计、可回退的研发协作者。

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

---

## 目录

- [核心能力](#核心能力)
- [工作台概览](#工作台概览)
- [技术栈](#技术栈)
- [快速开始](#快速开始)
- [开发指南](#开发指南)
- [项目结构](#项目结构)
- [许可证](#许可证)

---

## 核心能力

### 1. Issue 生命周期工作台

以 **Project → Repository → Issue** 为产品主线，首页提供四列看板：

- **Issue**：需求源头，绑定代码库上下文。
- **Story Spec**：由 Issue 派生的用户故事与拆分建议。
- **Design Spec**：由已确认 Story 派生的前端 / 后端设计文档。
- **Work Item**：由 Story 与 Design 联合派生的可执行任务。

点击 Issue 进入聚焦态，右侧三列自动过滤出该 Issue 的完整派生链路，方便追踪从需求到代码的完整路径。

![Issue 生命周期工作台](readme-pic/5341782975854_.pic.jpg)

### 2. Project / Repository / Issue 管理

左侧边栏统一管理 Project 与代码库（Repository）：

- 创建 Project 作为需求集合。
- 为 Project 添加本地代码库路径，Issue 自动生成即绑定代码上下文。
- Issue 支持 Markdown 描述、阶段（clarification / development）与状态追踪。

![Project 与代码库管理](readme-pic/5351782975870_.pic.jpg)

### 3. AI Provider 驱动与依赖自检

Aria 默认依赖 **Claude Code**（必装）与 **Codex**（可选）作为底层执行器。首次启动时会自动检测环境：

- 检测 `node` / `npm` 是否可用。
- 检测 `claude` / `codex` 命令是否在 `PATH`。
- 缺失时通过 `npm install -g` 引导自动安装（不使用官方一键安装链接，便于内网与包管理器同步）。

Provider 架构可扩展，后续可低成本接入 OpenCode、Kimi Code 等新的 AI 执行器。

![Issue 描述：Provider 依赖自检](readme-pic/5361782975881_.pic.jpg)

### 4. Story / Design / Work Item 生成与版本历史

每个产物卡片都支持：

- **AI 生成**：从 Issue 或上游产物触发 Provider 生成建议。
- **交叉 Review**：作者 provider 生成 → reviewer provider 审查 → 作者 provider 修订，达到配置轮次后进入人工确认。
- **版本历史**：每次生成与修订都保留版本（v1、v2 …），可查看 Markdown 全文与审核记录。
- **人工确认**：确认后的产物才能进入下游派生。

![Story Spec 版本历史](readme-pic/5371782975894_.pic.jpg)

![Story Spec 全文预览](readme-pic/5381782975905_.pic.jpg)

![Design Spec 版本历史](readme-pic/5391782975921_.pic.jpg)

![Design Spec 全文预览](readme-pic/5401782975932_.pic.jpg)

### 5. Work Item Group 与执行计划

Work Item 以 Group 形式组织，包含：

- 依赖图（Dependency Graph）
- 拆分选项（是否拆分前后端、是否包含集成/E2E 测试、是否需要执行计划确认）
- 子 Work Item 明细
- 关联的 Coding Attempt 状态

生成 Plan 并经人工确认后，方可进入 Coding Workspace 执行开发、测试、Review、Rework 与最终验收。

![Work Item Group 明细](readme-pic/5411782975950_.pic.jpg)

### 6. Provider Workspace 弹窗

统一的对话式工作区承载 Story、Design、Work Item 的全流程：

- 左侧流程轨道显示当前节点与状态。
- 中间对话区展示 provider 输入、输出、澄清问答与用户补充指令。
- 右侧产物区展示 Markdown/JSON 产物、版本历史、Review 意见与确认记录。

![Story Spec 审核流程](readme-pic/5421782975994_.pic.jpg)

![Design Spec 审核流程](readme-pic/5441782976041_.pic.jpg)

### 7. Coding Workspace

Work Item 确认后进入 Coding Workspace：

- 多角色（Analyst、Planner、Coder、Tester、Reviewer）顺序/并行执行。
- 实时流式事件、权限请求、Stage Gate 与人工确认。
- 支持执行计划变更、Abort、Diff 查看与回滚（Rollback）。

---

## 技术栈

| 层级 | 技术 |
|------|------|
| 后端 | Rust（Edition 2024）、Axum、Tokio、rust-embed |
| 前端 | TypeScript、React 19、Vite、TanStack Router、Zustand、Tailwind CSS、Monaco Editor |
| 测试 | Rust 集成测试 / 单元测试、Vitest、Playwright E2E |
| 分发 | npm / npx（`@cadence-aria/cli`） |

---

## 快速开始

### 通过 npx 使用（推荐终端用户）

```bash
npx @cadence-aria/cli
```

启动后将自动打开浏览器并进入 `http://127.0.0.1:<port>/workbench`。

### 本地源码启动

```bash
# 1. 构建前端产物（后端编译时通过 rust-embed 嵌入 web/dist，必须先构建）
pnpm -C web install   # 首次
pnpm -C web build

# 2. 启动后端 + 前端开发服务
cargo run --locked -- web --workspace . --host 127.0.0.1 --port 4317
# 在另一个终端启动前端 dev server
cd web && pnpm dev --port 5173
```

开发模式下前端代理 `/api` 到 `http://127.0.0.1:4317`，访问 `http://127.0.0.1:5173` 即可。

### 环境要求

- macOS 或 Linux（Windows 不在当前支持范围）
- Rust（版本以 `rust-toolchain.toml` 为准）
- Node.js >= 18 与 pnpm
- Claude Code（首次启动时如不存在的会引导安装）

---

## 开发指南

### 构建与检查

```bash
# 任何 cargo 构建/测试前必须先构建前端产物
pnpm -C web build

# 格式化
cargo fmt --check

# Clippy
cargo clippy --all-targets --all-features --locked -- -D warnings

# 检查
cargo check --locked

# 测试
cargo test --locked

# 前端类型检查
cd web && pnpm tsc -b

# 前端单元测试
cd web && pnpm test
```

> 注意：禁止给 `cargo test` 添加 `-j 1`；并行度由 `.cargo/config.toml` 统一托管。

### npx 本地冒烟

```bash
pnpm -C web build
cargo build --release
node scripts/smoke-npx.mjs
```

详见：

- `cadence/readmes/2026-05-31_README_npx分发开发与发布指南_v1.0.md`
- `cadence/readmes/2026-05-30_README_sccache可选缓存_v1.0.md`
- `cadence/project-rules/build-test-commands.md`

---

## 项目结构

```text
.
├── src/                    # Rust 后端
│   ├── cli.rs              # CLI 入口
│   ├── cross_cutting/      # Provider 适配、进程管理、artifact 校验等
│   ├── daemon/             # 守护进程与状态机
│   ├── interactive/        # REPL / 交互投影
│   ├── product/            # Project / Issue / Repository / WorkItem 等产品索引
│   ├── protocol/           # 协议定义
│   ├── runtime_units/      # 运行时执行单元
│   ├── task_run/           # 任务运行编排
│   └── web/                # Web API、WebSocket、静态资源
├── web/                    # React + TypeScript 前端
│   ├── src/
│   │   ├── components/     # 组件（lifecycle、chat-workspace、coding-workspace）
│   │   ├── pages/          # 页面（Workbench、ChatWorkspace、CodingWorkspace）
│   │   ├── state/          # Zustand 状态
│   │   └── api/            # API 客户端与类型
│   └── e2e/                # Playwright E2E
├── npm/                    # npx 分发包
│   ├── cli/                # JS launcher
│   ├── cli-darwin-arm64/
│   ├── cli-darwin-x64/
│   └── cli-linux-x64/
├── cadence/                # 项目产物文档、设计、计划、规则
│   ├── designs/            # 技术方案
│   ├── plans/              # 计划文档
│   ├── prds/               # 需求文档
│   └── project-rules/      # 项目自定义规则
├── tests/                  # Rust 集成测试
├── scripts/                # 构建与发布脚本
└── README.en.md            # English README
```

---

## 许可证

[MIT](LICENSE)
