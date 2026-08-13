# Kimi Code Provider 接入实施计划（总纲）

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在日常 Workspace（Story/Design/Work Item 的 Author/Reviewer）、Coding Workspace（Coder/Code Reviewer/Internal Reviewer）与 image-create 中把 Kimi Code 作为与 Claude Code、Codex 并列的第四个真实流式 Provider 接入（Auto 默认 + 可切 Supervised），通过 ACP（`kimi acp`）JSON-RPC over stdio 执行，支持工具审批、AskUserQuestion 结构化提问、会话恢复（resume）、取消与超时；task-run 与仓库初始化不接入。

**Architecture:** Kimi 通过 `kimi acp` 子进程（ACP JSON-RPC over stdio）执行；`session.rs` 复用 `JsonRpcPeer` 驱动 initialize → session/new|load → session/prompt 状态机；`parse.rs` 解析 ACP 事件（session/update 子类型、session/request_permission、session/prompt result）。每个角色运行 = 一个子进程，cwd = 项目代码库目录；Kimi 用默认 `~/.kimi-code/`（凭证复用用户全局登录）；Aria 持有 Kimi `sessionId`（上报为 `provider_session_id`）。终态由 `session/prompt` result `stopReason` 判定。

**Tech Stack:** Rust (tokio) 后端、TypeScript/React 前端、Kimi Code CLI 0.34.0（`kimi acp`，ACP protocolVersion 1）、serde、JsonRpcPeer、ApprovalBridge、ast-grep/CodeGraph（代码阅读）。

**Contract:** `openspec/changes/add-kimi-code-provider/`（最新提交 `c9587d4`）。

## 本文档结构

本计划按工作包拆分为一份总纲 + 每个工作包一份独立文件，全部自包含、可独立执行：

| 文件 | 对应 tasks.md | 内容 |
|---|---|---|
| `task-01-provider-catalog.md` | 1.1, 1.2, 1.3 | ProviderName/ProviderType 加 KimiCode + 穷尽 match + 健康检查（kimi --version, ≥0.34.0）+ capability 校验 + 状态 API + 前端 catalog + Task Runner 四入口稳定拒绝（不用 unreachable!）+ 仓库初始化过滤 |
| `task-02-acp-session-adapter.md` | 2.1 | ACP 协议 fixture（冻结 0.34.0）+ 流式适配器（mod/parse/session/tests）+ resume(session/load) + 终态(stopReason) + 工具事件(TextDelta/ToolCall/ToolResult is_error) + 取消(session/cancel+杀进程) + 超时(timeout_secs) + thought(stderr/tracing) + 协议降级 + 生产/测试 registry 注册 |
| `task-03-supervised-and-askuser.md` | 2.2, 2.3 | Supervised 工具审批（PermissionRequest→ApprovalBridge，收窄二元 approved→allow_once）+ AskUserQuestion 提问（request_permission→ChoiceRequest，逐题串行，allow_multiple=false）+ 自由文本（free_text 优先，Cancelled+下轮 prompt，对齐 Claude） |
| `task-04-workspace-coding-permission.md` | 3.1, 3.2, 4.1, 4.2 | 普通/Coding Workspace 接入 Kimi（默认 Auto，可切 Supervised，不强制 Auto）+ 前后端 interaction guidance 一致 + work_item_projection render/kimi_code.rs |
| `task-05-image-create-and-boundaries.md` | 5.1, 5.2 | image-create 支持 Kimi（From<ProviderName> + 前端 dropdown + 不可用禁用）+ 显式排除（artifact retry / review repair） |
| `task-06-frontend-ui.md` | 6.1, 6.2 | 前端 catalog/order/可用禁用 + ProviderConfigPanel（Auto+Supervised，非 Pi 的 Auto-only）+ WebSocket parser + CreateRepositoryDialog 过滤 + 错误文案（版本过低/未登录） |
| `task-07-regression-quality.md` | 7.1, 7.2, 7.3 | 回归测试（health/registry/gate/DTO/task-run 拒绝/凭证缺失/审批/提问/resume/取消/超时）+ 前端测试 + 质量门禁 |

**执行顺序：** Task 1 → 2 → 3 → 4 → 5 → 6 → 7（严格顺序，每个 Task 依赖前一个的接口）。

## Global Constraints

- 必须用中文回答；代码本身用英文。
- 遵循 TDD：每个任务先写失败测试，再实现，再验证通过。
- 🔴 Rust 构建/测试/检查命令**禁止 `-j 1`**；定向单测用 `cargo test --locked --lib <过滤名>`（禁 `cargo test --locked <名>`，会遍历 integration test 二进制）；标准命令见 `cadence/project-rules/build-test-commands.md`。
- 🔴 代码阅读大范围检索用 CodeGraph，精确结构阅读优先 `ast-grep outline`。
- 前端用 `pnpm`（禁 npm/yarn）；测试 `cd web && pnpm test`，类型 `pnpm tsc -b`。
- **ProviderName::KimiCode / ProviderType::KimiCode**：wire 均为 `"kimi_code"`。
- **健康检查命令**：`kimi --version`，最低版本 `0.34.0`；低于则报告不可用。
- **initialize 后 capability 校验**（不只靠版本）：`protocolVersion==1`；resume 需 `loadSession==true` + `sessionCapabilities.resume`；Supervised 需 request_permission 生效（fixture 保证）。
- **权限矩阵**：Kimi 默认 Auto（ACP `permissionMode="auto"`），可切 Supervised（`default`）；与 Claude Code/Codex 一致，区别于 Pi 的 Auto-only。
- **Auto 不藏提问**（B1 spike 实证）：Auto 模式不发**普通工具**的 PermissionRequest，但 AskUserQuestion 的 request_permission **仍发**并映射 ChoiceRequest。
- **AskUserQuestion 提问**（B3 spike 实证）：Kimi 逐题串行发 request_permission，每次单题单选（含 `*_skip`）；映射单个 ChoiceRequest，`allow_multiple=false`，`allow_free_text=true`，source=`AskUserQuestion`。
- **自由文本**（B2 对齐 Claude）：free_text 非空 → free_text 优先，忽略 selected_option_ids（不拼接），回 ACP `Cancelled` + 内部发第二个 session/prompt 注入文本，第二轮 result 为唯一终态，不发中间 Failed/Completed。
- **Supervised 审批**（B6 收窄二元）：`approved=true → allow_once`，`approved=false → reject_once`；不用 AllowAlways，不扩展共享 PermissionResponse:bool 契约。
- **事件映射**：AgentMessageChunk → `ProviderEvent::TextDelta{content}`（计入 full_output）；ToolCall(pending) → `ProviderToolCall`；ToolCallUpdate 仅 completed/failed 发一次 `ProviderToolResult{tool_use_id,output,is_error}`；AgentThoughtChunk 不计 full_output、不落盘（stderr/tracing）。
- **resume**：`input.resume_provider_session_id` 存在 → 发 `session/load`（互斥于 new）；load 失败→Failed，不脚到 new。resume ≠ 失败重试（第一阶段不做同 provider 重试）。
- **取消**（B7）：优先 ACP `session/cancel`，短超时 drain，未果再 ProcessManager 进程组 terminate；发 `ProviderStatus::Aborted`，不发 Failed。
- **超时**（B8）：消费 `input.timeout_secs` 全 session 总超时 + initialize/new|load/prompt 各独立 request timeout + resume stall timeout；有效 session/update 重置空闲。
- **协议降级**（M3）：未知 notification 忽略+日志；未知 request（有 id）回 `-32601 Method not found`；request_permission 未知 option kind 按拒绝 + 保留原 RPC id。
- **凭证**：不从 `KIMI_API_KEY` env 自动读取；需 `~/.kimi-code/config.toml` 或 `kimi login`；`env_vars` 对 Kimi 认证无效；health 通过但运行时未登录 → 映射清晰运行错误（提示 `kimi login`）。
- **task-run 边界**：四入口（provider_factory/step_runner/web runtime provider.rs/utils.rs）稳定拒绝 Kimi，返回 incompatible 错误或稳定文本，**禁止 `unreachable!`**；不进 adapter_compatibility 矩阵。
- **不动 Pi**：Pi 现有 `unreachable!`、Pi 的 selected 优先 ChoiceResponse 逻辑均不改（历史差异，后续单独任务统一）。
- **不扩大范围**：仓库初始化仅 Claude Code（过滤 Kimi，同 Pi）；task-run 不调度 Kimi；同 provider 重试不做；Kimi `plan`/`yolo` mode 不暴露（仅 auto/default）；structured-output repair / review repair 排除 Kimi。
- **install_hint**：`Install Kimi Code CLI and ensure \`kimi\` is available on PATH.`（不提登录，保持模板一致）。
- **fixture 来源**：`.pi-subagents/spike/acp/` 已冻结的 0.34.0 真实 ACP 往返，转为 `src/cross_cutting/kimi_code_provider/tests/fixtures/*.jsonl` 与 `tests/fixtures/provider/kimi_acp_*_fixture.sh`。

## 跨任务接口契约

后续任务依赖前序任务产出的精确符号，此处统一定义：

- **Task 1 产出**：`ProviderName::KimiCode`、`ProviderType::KimiCode`（wire 均 `"kimi_code"`）；所有 `provider_type_for_name`/`From<ProviderName>` 加 `KimiCode => ProviderType::KimiCode`；健康检查 `kimi_version_command()`（`kimi --version`）；状态 API 返回 Kimi 条目；`ProviderRegistry::available_names()` 含 `ProviderName::KimiCode`；Task Runner 四入口稳定拒绝 Kimi。
- **Task 2 产出**：`KimiCodeProvider::new(command: PathBuf)`；`impl StreamingProviderAdapter for KimiCodeProvider`；`run_kimi_session(peer, command_rx, event_tx, input, cancel)`；生产/测试 `default_provider_registry()` 含 `ProviderName::KimiCode` 注册；ACP fixture 文件。
- **Task 3 产出**：Supervised 审批往返（PermissionRequest→ApprovalBridge→allow_once/reject_once）；AskUserQuestion→ChoiceRequest 映射；自由文本 free_text 优先路径。
- **Task 4 产出**：普通/Coding Workspace 接入 Kimi（默认 Auto、可切 Supervised）；`render/kimi_code.rs` renderer；前后端 guidance。
- **Task 5 产出**：image-create `From<ProviderName>` 含 Kimi；前端 dropdown 含 kimi_code；artifact retry/review repair 排除 Kimi。
- **Task 6 产出**：前端 catalog/order/权限控件（Auto+Supervised）/WebSocket parser/仓库初始化过滤/错误文案。
