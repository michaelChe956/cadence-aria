# Pi Provider 接入实施计划（总纲）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在日常 Workspace（Story/Design/Work Item）与 Coding Workspace 中把 Pi 作为与 Claude Code、Codex 并列的真实流式 Provider 接入（**Auto-only**），支持选择、执行、取消、会话续接与失败直接报告；同时把普通/Coding Workspace 的权限模式统一为默认 `Auto`（Claude Code 与 Codex 保留既有 `Supervised`）。

**Architecture:** Pi 通过 `pi --mode rpc` 子进程（JSONL over stdin/stdout）执行；`session.rs` 驱动 RPC 往返并复用 `JsonRpcPeer`；**无授权扩展**——Pi 仅以 `Auto` 运行，工具调用直接执行，运行事件照常记录审计。每个角色运行 = 一个子进程 + 一对 stdio 管道，cwd 为项目代码库目录，会话标识交由 Pi 原生机制（`~/.pi`）管理，Aria 仅持有 `session-id` 用于续接。

**Tech Stack:** Rust (tokio) 后端、TypeScript/React 前端、Pi CLI 0.83.0 (`--mode rpc`)、serde、ast-grep/CodeGraph（代码阅读）。

**Contract:** `openspec/changes/add-pi-provider/`（最新提交 `52fb573`，Pi Auto-only）。

## 本文档结构

本计划按工作包拆分为一份总纲 + 每个工作包一份独立文件，全部自包含、可独立执行：

| 文件 | 对应 tasks.md | 内容 |
|---|---|---|
| `task-01-provider-catalog.md` | 1.1, 1.2, 1.3 | ProviderName/ProviderType 加 Pi + 穷尽 match + 健康检查 + 状态 API + 前端 catalog + Task Runner 拒绝 |
| `task-02-pi-rpc-adapter.md` | 2.1 | Pi RPC 协议冻结 fixture + 流式适配器（Auto-only）+ 生产/测试 registry 注册 |
| `task-03-workspace-permission.md` | 3.1, 3.2 | 普通 Workspace 权限持久化（真实体，默认 Auto）+ Pi 接入 + fail-fast |
| `task-04-coding-permission.md` | 4.1, 4.2 | Coding Workspace 默认 Auto + Pi 接入 + fail-fast |
| `task-05-frontend-ui.md` | 5.1, 5.2 | 前端 Provider 目录 + 权限控制（Pi 仅 Auto）+ 失败状态可见性 |
| `task-06-regression.md` | 6.1, 6.2, 6.3 | 回归测试 + 边界验证 + 前后端质量检查 |

**执行顺序：** Task 1 → 2 → 3 → 4 → 5 → 6（严格顺序，每个 Task 依赖前一个的接口）。

## Global Constraints

- 必须用中文回答；代码本身用英文。
- 遵循 TDD：每个任务先写失败测试，再实现，再验证通过。
- 🔴 Rust 构建/测试/检查命令**禁止 `-j 1`**；用 `cargo test -p cadence-aria <name>` 定向快反馈；标准命令见 `cadence/project-rules/build-test-commands.md`。
- 🔴 代码阅读大范围检索用 CodeGraph，精确结构阅读优先 `ast-grep outline`。
- **Decision 1（已获批）**：`ProviderName` 和 `ProviderType` 都加 `Pi`；`ProviderType::Pi` 只是共享类型变体，Task Runner 在 HTTP 调度入口、`RoutingProviderAdapter`、兼容性矩阵、节点契约四层显式拒绝 Pi，不调度、不路由、不执行。
- **Decision 2（已获批，Auto-only）**：Pi 仅以 `Auto` 模式运行，**不引入 Aria 授权扩展**。无 `aria-gate.ts`、无扩展资源交付、无授权 UI 往返、无机器可读授权 payload。
- **权限模式矩阵（已获批）**：

  | Provider | Auto | Supervised | 默认 |
  |---|---|---|---|
  | Claude Code | 支持 | 支持（既有，不改） | **Auto**（本变更改默认值） |
  | Codex | 支持 | 支持（既有，不改） | **Auto**（本变更改默认值） |
  | Pi | 支持 | **不支持** | Auto（唯一） |

- **fail-fast 边界（已澄清）**：禁「切换/重放/重试到**其他** Provider」，**不禁**同 Provider 内部重试（Claude/Codex 现有 artifact retry、resume-stall fresh retry 保留）。Pi 不实现同 Provider 内部重试：启动或运行失败即终态失败。
- **不扩大范围**：仓库初始化、Fake Provider Workspace Runner 不动；Task Runner 可调度范围和运行行为不变。
- **会话目录不干预**：不传 `--session-dir`，Pi 用默认 `~/.pi`。
- **工作目录 = 项目代码库目录**：`working_dir` 传 Aria Workspace 的代码库路径。
- **Pi 健康检查命令**：`pi --version`。

## Phase 0 Spike（✅ 已完成）

已在 Plan 编写前真实完成。本变更 Pi 为 Auto-only，监督相关 spike 不再需要；与本变更相关的会话能力验证结论：

| 能力 | 验证方式 | 结果 |
|---|---|---|
| 会话粒度 | `get_state` 返回 `sessionId` | ✅ |
| 流式事件 | `message_update`/`tool_execution_*`/`agent_settled` 均收到 | ✅ |
| 取消 | `abort` 命令可用 | ✅ |
| 恢复 | `--session-id` / `--resume` / `--fork` 支持 | ✅ |

**结论：** 契约 Decision 2（Auto-only）技术地基成立。Task 2 的协议 fixture 将参照此 spike 重新录制并提交到仓库（不依赖 `/tmp`）。

## 跨任务接口契约

后续任务依赖前序任务产出的精确符号，此处统一定义，各任务文件自包含时会重复引用：

- **Task 1 产出**：`ProviderName::Pi`、`ProviderType::Pi`（wire 均 `\"pi\"`）；`provider_type_for_name(&ProviderName::Pi) -> ProviderType::Pi`；健康检查 `pi_version_command()`（`pi --version`）；状态 API 返回 Pi 条目；`ProviderRegistry::available_names()` 含 `ProviderName::Pi`。
- **Task 2 产出**：`PiProvider::new(command: PathBuf)`；`impl StreamingProviderAdapter for PiProvider`；生产/测试 `default_provider_registry()` 含 `ProviderName::Pi` 注册；`run_pi_session`（Auto-only 会话驱动）。
- **Task 3 产出**：`WorkspaceRolePermissionModes { author: ProviderPermissionMode, reviewer: ProviderPermissionMode }`（`Default` 全 `Auto`）；`WorkspaceSessionRecord.permission_modes`（`#[serde(default)]`）；`WorkspaceSession.permission_modes`。
- **Task 4 产出**：`CodingRolePermissionModes::default()` 全 `Auto`（既有类型，只改默认值）。
- **Task 5 产出**：前端 `WsProviderConfig.permission_modes`；普通/Coding 面板权限控件（Pi 仅 Auto）。
