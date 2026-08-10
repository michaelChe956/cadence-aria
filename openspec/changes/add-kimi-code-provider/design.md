# Design — Add Kimi Code Provider

> 本设计基于 Phase 0 Spike 实证（Kimi Code CLI 0.34.0，ACP over stdio）。Spike 抓取的真实 JSON-RPC 往返冻结于 `.pi-subagents/spike/acp/`，作为后续 fixture 来源。

## 1. 总体架构

**新增 provider**：`KimiCode`，与 Claude Code / Codex / Pi 并列的第四个真实流式 provider。

**传输与协议**：
- 子进程命令：`kimi acp`（无额外参数，纯 stdio JSON-RPC）。
- 传输层**复用现有 `JsonRpcPeer`**（Pi/Codex 都在用），不新造。
- 协议层按 **ACP（Agent Client Protocol）标准**新写解析。

**ACP 会话生命周期（spike 实证）**：
```
client → initialize (id:1)
server → result { protocolVersion:1, agentCapabilities, authMethods }
client → notifications/initialized
client → session/new (id:2, cwd, permissionMode)
server → result { sessionId, configOptions, modes }
client → session/prompt (id:N, prompt)
server → session/update *  (agent_thought_chunk / agent_message_chunk / tool_call / tool_call_update / session_info_update / usage_update)
server → session/request_permission (id:M)  [仅 Supervised 模式]
client → session/request_permission result (id:M, approve/reject)
server → session/prompt result (id:N) { stopReason: "end_turn" }  ← 终态
```

**模块骨架**（仿 `codex_provider` 的 RPC 长连接模型，协议解析按 ACP）：
```
src/cross_cutting/kimi_code_provider/
  mod.rs        # KimiCodeProvider + 命令构造 + 子进程 spawn + 生命周期 + 版本探测
  parse.rs      # ACP 消息解析（session/update 子类型、request_permission、stopReason）
  session.rs    # ACP 会话驱动（initialize→new→prompt 往返 + 事件分发 + 审批响应 + Abort）
  tests.rs      # 单测
  tests/fixtures/*.jsonl  # 冻结自 spike 0.34.0 的真实 ACP 往返
```

**对接 Aria 流式契约**：`impl StreamingProviderAdapter for KimiCodeProvider`，将 ACP 事件映射为 Aria 的 `ProviderEvent`，把 Aria 的 `ProviderCommand` 映射为 ACP 响应或进程终止。

**权限映射**：Kimi `default` ↔ Aria `Supervised`；Kimi `auto`/`yolo` ↔ Aria `Auto`；Kimi `plan` 暂不暴露。`session/new` 的 `permissionMode` 按 Aria 侧选择传入。

**会话与恢复**：每个 role 运行 = 一个 `kimi acp` 子进程，cwd = Aria workspace 代码库目录；Kimi 用默认 `~/.kimi-code/`（凭证复用用户全局登录）；Aria 持有 Kimi `sessionId`（上报为 `provider_session_id`）。**支持 resume**：复用 Aria 既有 `StreamingProviderInput.resume_provider_session_id` 机制，当传入历史 sessionId 时，适配器发 ACP `session/load`（spike 实证 initialize 声明 `loadSession:true` + `sessionCapabilities.resume`）续接同一会话，与 Claude/Codex/Pi 一致。第一阶段不实现同 provider **失败重试**（artifact retry / resume-stall fresh retry）。

**健康检查**：`kimi --version`，解析版本，`< 0.34.0` 报"版本过低"。

## 2. 枚举、契约与登记层

所有穷尽 `match` 显式补分支，禁止靠编译器报错局部补。

**核心枚举**：
- `src/product/models/provider.rs`：`ProviderName` 加 `KimiCode`（wire `"kimi_code"`）。
- `src/protocol/contracts.rs`：`ProviderType` 加 `KimiCode`（wire `"kimi_code"`）。

**`ProviderName ↔ ProviderType` 映射（4 处穷尽 match）**：
- `src/product/image_create/models.rs`
- `src/product/coding_workspace_engine/tool_format.rs`
- `src/product/workspace_engine/mappings.rs`
- `src/product/work_item_split_engine/types.rs`

每处加 `ProviderName::KimiCode => ProviderType::KimiCode`。

**模块导出**：`src/cross_cutting/mod.rs` 加 `pub mod kimi_code_provider;`。

**共享层登记**：
- `provider_registry.rs`：`available_names()` stable order 末尾加 `KimiCode`，更新顺序断言。
- `provider_health.rs`：新增 `kimi_version_command()`（`kimi --version`），加入 `tokio::join!` 并行 probe、providers vector、初始快照、"全部不可用"判定，更新并行数与排序测试。
- `provider_availability_gate.rs`：无 provider 专属实现；Kimi health entry 缺失自动阻断，补 gate 成功/失败测试。
- `provider_capabilities.rs`：Kimi 走 streaming 路径声明；task-run 相关 capability 不声明。

**状态 API（`src/web/handlers/providers.rs`）**：DTO match 加 Kimi 分支：`provider="kimi_code"`、`display_name="Kimi Code"`、`install_hint="Install Kimi Code CLI and ensure \`kimi\` is available on PATH."`。

**task-run 边界（4 入口，稳定错误，不用 `unreachable!`）**：
- `task_run/provider_factory.rs`：`ProviderType::KimiCode => Err(incompatible_output(...))`。
- `task_run/step_runner.rs`：`ProviderType::KimiCode => "kimi_code"`（稳定文本）。
- `web/runtime/provider.rs`：`ProviderType::KimiCode => None`。
- `web/runtime/utils.rs`：返回稳定错误/空，不 `unreachable!`。

**adapter_compatibility.rs**：Kimi 不进 task-run 兼容矩阵（同 Pi），写明边界注释。

## 3. KimiCodeProvider 模块与 ACP 协议映射

### 3.1 mod.rs — provider 入口与生命周期

仿 `codex_provider/mod.rs`：
```
pub const KIMI_COMMAND: &str = "kimi";
const MIN_KIMI_VERSION: &str = "0.34.0";
pub struct KimiCodeProvider { command: PathBuf }
impl KimiCodeProvider { new / build_args(["acp"]) / probe_kimi_version / ensure_kimi_version_compatible }
impl StreamingProviderAdapter for KimiCodeProvider { start(input) -> ProviderSession }
```

`start()` 流程：probe 版本 → spawn(`kimi`, `["acp"]`, cwd) → `JsonRpcPeer::new` → 起 stderr 任务 → spawn `run_kimi_session` → 返回 `ProviderSession`（session_id 待 session/new 后上报）。

### 3.2 parse.rs — ACP 消息解析

强类型枚举：
```
enum KimiSessionUpdate {
    AgentThoughtChunk { content },   // 不计入 full_output
    AgentMessageChunk { content },   // 计入 full_output
    ToolCall { tool_call_id, title, kind, status: pending },
    ToolCallUpdate { tool_call_id, status: in_progress|completed|failed, content },
    SessionInfoUpdate { title },
    UsageUpdate { used, size },
    AvailableCommandsUpdate { .. },  // 第一阶段忽略
}
struct KimiPermissionRequest { request_id, tool_call_id, options: [AllowOnce|AllowAlways|RejectOnce] }
struct KimiPromptResult { stop_reason: StopReason }  // end_turn | ...
```
`function.arguments` 是 JSON 字符串，二次 `serde_json::from_str` 安全解析（失败不 panic）。未知 sessionUpdate 子类型向前兼容不 panic。

**协议向前兼容与降级**：ACP 协议随 Kimi 版本演进可能新增 method 或扩展 request_permission/session/update 的 schema。适配器 SHALL 对未知 method 静默忽略并记录日志；对 request_permission 中未知 option kind 按拒绝处理（安全侧）；对 session/update 中未知 sessionUpdate 子类型跳过不 panic。fixture 锁定 0.34.0 协议形态，健康检查门禁 (< 0.34.0) 保证运行期协议在 spike 实证范围内；超出范围的字段变化以 degrade 为主，不阻断已有能力。

### 3.3 session.rs — ACP 会话驱动（核心状态机）

仿 `codex_provider/session.rs`：
1. 发 initialize (id:1)，校验 protocolVersion==1。
2. 发 notifications/initialized。
3. 发 session/new (id:2, cwd, permissionMode)，拿 sessionId，上报 provider_session_id。
4. 发 session/prompt (id:N, input.prompt)，进入事件循环。
5. 事件循环（select peer/command/cancel）：
   - `session/update` → 按 KimiSessionUpdate 映射 ProviderEvent：
     - AgentMessageChunk → `ProviderEvent::TextChunk`，累计 full_output。
     - ToolCall/ToolCallUpdate → `ProviderEvent::ToolCall`/`ToolResult`。
     - AgentThoughtChunk → 写入既有 provider 流式日志（与 Claude Code 的 thinking 一致），不计入 full_output，不作为正文事件转发。
     - UsageUpdate → 日志。
   - `session/request_permission` (id:M) → `ProviderEvent::PermissionRequest`；等 `command_rx` 的 PermissionResponse → 回 `session/request_permission` result `{options:[{optionId, outcome:selected}]}`。用户 reject → 工具不执行，会话继续（仍可能再请求）。
   - `session/prompt` (id:N) result → 终态：`stopReason=end_turn` → `ProviderEvent::Completed(full_output)`（一次且仅一次）；其他/error → `Failed`。
   - `Abort`/cancel → terminate 进程 → `Failed`(取消)。
6. 进程退出：exit 0 且已发 Completed 则静默；未发终态则发 Failed。

**不变量**：Completed/Failed 一次且仅一次；provider_session_id 仅 session/new 确认后上报；Abort 清理子孙进程；full_output 只累计 AgentMessageChunk。

### 3.4 权限模式映射
```
ProviderPermissionMode::Auto       → session/new permissionMode: "auto"
ProviderPermissionMode::Supervised → session/new permissionMode: "default"
```
Auto 模式不发 request_permission；Supervised 模式正常走 ApprovalBridge。

### 3.4a 提问能力（spike 实证，与 Claude Code 对齐）

Phase 0 Spike 强触发实测：Kimi **有 `AskUserQuestion` 工具**（与 Claude Code 同名同 schema：参数为 `{questions:[{question,header,options:[{label,description}]}]}`）。Kimi 把提问统一收敛到 ACP `session/request_permission` 机制：

- 提问正文 = `session/request_permission` 的 `toolCall.content` text。
- 选项 = `options` 数组，每项 `{optionId, name(显示文本), kind}`；Kimi 会自动补一个 `*_skip`（kind=reject_once）作为跳过。
- 用户选某选项 → 客户端回 `session/request_permission` result `{options:[{optionId, outcome:"selected"}]}`（标准 ACP `Selected`，可靠）。
- **自由文本补充**：ACP `session/request_permission` 的 result 只支持 `Cancelled`/`Selected`/`Other`，其中 `Other` 协议明确「非为用户输入文本设计」。故自由文本不走 request_permission，而走**下一轮 `session/prompt`**（同 sessionId 续接）。

**方案 D 的映射（选项 + 始终可编辑文本框的融合卡片）**：
- 适配器识别 `session/request_permission` 中 `toolCall.title == "AskUserQuestion"` → 映射为既有 `ProviderEvent::ChoiceRequest`（source=`AskUserQuestion`，options 来自 request_permission.options，`allow_free_text=true`，UI 同时展示选项按钮 + 自由文本框）。
- 其他 title 的 request_permission → 映射为既有 `ProviderEvent::PermissionRequest`（工具审批）。
- **用户选选项** → `ChoiceResponse(selected_option_ids=[对应 optionId])` → 适配器回 ACP `Selected(optionId)`，同轮继续。
- **用户填自由文本（选项都不合适）** → 不回 request_permission，适配器以“用户自主输入”结束当前轮（Failed/Cancelled 语义或正常收尾），用户文本作为**下一轮 `session/prompt`** 注入同 sessionId，上下文完整。

因此：
- Kimi **不复用** Pi 的 `aria-ask.ts` 扩展（那是 Pi 私有 RPC 的产物）。
- Kimi 复用 Claude Code 的 `AskUserQuestion → ChoiceRequest` 映射思路（同一工具名/schema）。
- 两路均走标准 ACP 机制（Selected / session-prompt），零非标准解析、不坏菜。

### 3.5 终态判定（spike 实证）
`session/prompt` 的 id 响应即终态：result `{stopReason:"end_turn"}` → Completed；result error / 非 end_turn → Failed；进程异常退出（未收到 prompt 响应）→ Failed。退出码：0 成功 / 1 不可重试 / 75 可重试（映射到 Failed 文案区分）。

## 4. 产品层映射、前端与 image-create

### 4.1 权限矩阵
| Provider | Auto | Supervised | 默认 |
|---|---|---|---|
| Claude Code | ✅ | ✅ | Auto |
| Codex | ✅ | ✅ | Auto |
| Pi | ✅ | ❌ | Auto |
| **Kimi Code** | ✅ | ✅ | **Auto** |

- `coding_models/provider_config.rs`：Kimi 默认 Auto，UI 可切 Supervised（非 Pi 的 Auto-only）。
- `workspace_engine/mappings.rs`：Kimi 保留用户选择的 Supervised（不强制 Auto）。
- `workspace_context/prompts.rs` + 前端 `workspace-ws-store-guidance.ts`：两端同时加 Kimi interaction guidance，能力声明一致（支持 structured permission request）。

### 4.2 Work Item Projection renderer
新增 `src/product/work_item_projection/render/kimi_code.rs`（仿 `render/pi.rs`），含专属 label、renderer version、Supervised tool hint、structured-output wrapper；接入 `render/mod.rs` import + `renderer_for()`。

### 4.3 artifact retry / review repair（排除）
同 Pi 先例，第一阶段排除 Kimi 于 `workspace_engine/provider_drive.rs`（artifact retry）与 `workspace_engine/review/drive.rs`（review repair），写明注释。

### 4.4 image-create（支持）
image-create 复用 streaming provider 会话，无需独立图像 API：
- `image_create/models.rs` `From<ProviderName>` 加 Kimi 分支。
- 前端 `web/src/api/types/image-create.ts` dropdown 加 Kimi。
- 回归测试覆盖 Kimi 在 image-create 跑通（脚本化 provider）。

### 4.5 前端 catalog + 可用性
- `state/provider-options.ts`：`REAL_PROVIDER_CATALOG` 加 `{ value:"kimi_code", fallbackLabel:"Kimi Code" }`；`PROVIDER_ORDER` 加 `"kimi_code"`（pi 之后、fake 之前）。
- `api/types/provider.ts`：`RealProviderName` union 加 `"kimi_code"`。
- 穷尽 match/union 补 Kimi：`ProviderConfigPanel.tsx`（Auto+Supervised）、`CodingProviderConfigPanel.tsx`、`ChatWorkspacePageParts.tsx`、`workspace-ws-message-handler.ts`、`CreateRepositoryDialog.tsx`（仓库初始化过滤 Kimi，加 policy 注释）、`workspace-ws-store-guidance.ts`，及相关 `.test.tsx` fixture union。

### 4.6 错误文案
- **凭证前置陷阱**：Kimi 不从环境变量（如 `KIMI_API_KEY`）自动读取凭证，必须写入 `~/.kimi-code/config.toml` 或完成 `kimi login`。Aria 的 `StreamingProviderInput.env_vars` 对 Kimi 认证无效。运行期若凭证缺失，从 ACP 错误/stderr 捕获并映射为清晰运行错误（提示用户运行 `kimi login`）。install_hint 不提登录，保持模板一致。
- 版本过低：health snapshot reason 提示。
- task-run 误调度：稳定错误文案。

## 5. 测试与验收

TDD（每 task 先失败测试再实现）。命令规范：🔴 禁止 `-j 1`；定向用 `cargo test --locked --lib <name>`。

### 5.1 provider 模块单测（`kimi_code_provider/tests.rs`）
fixture 来自 `.pi-subagents/spike/acp/` 冻结的 0.34.0 真实往返。

**协议解析**：initialize 握手 / session/new（sessionId+modes）/ session/update 各子类型 / request_permission（options）/ prompt result（stopReason）/ arguments 二次解析（损坏 JSON 不 panic）/ 未知子类型向前兼容。

**会话驱动**（脚本化子进程 fixture，仿 `pi_provider/tests.rs`）：正常文本流→Completed（full_output 累计正确，thought 不计入）/ 工具调用完整往返（approve）/ 审批 reject（工具不执行，会话继续）/ Auto 不发 request_permission / 退出码 0/1/75 / Abort（进程终止，不双发终态）/ 异常退出→Failed / 版本 < 0.34.0 启动门禁报错。

**不变量**：Completed/Failed 一次且仅一次；provider_session_id 仅 session/new 后上报。

### 5.2 共享层与登记
provider_health（Kimi 可用/缺失/超时/版本过低，snapshot 持久化，并行 probe）/ provider_registry（stable order，available/executable 断言）/ provider_availability_gate（entry 存在/缺失）/ handlers/providers（DTO 正确）。

### 5.3 产品层
work_item_projection render（Kimi golden）/ 权限配置（默认 Auto 可切 Supervised）/ artifact retry·review repair（Kimi 被排除断言）/ image-create（From<ProviderName> 含 Kimi，脚本化跑通）。

### 5.4 task-run 边界
provider_factory（incompatible_output，仿 Pi expect_err）/ step_runner·utils（Kimi 分支不 panic）。

### 5.5 前端
provider-options.test（catalog/order/可用禁用）/ ProviderConfigPanel.test（Auto+Supervised）/ CreateRepositoryDialog.test（Kimi 被过滤）/ WebSocket parser、Chat fallback、fixture union 含 kimi_code。

### 5.6 质量门禁
`cargo fmt --check` / `cargo clippy --all-targets --all-features --locked -- -D warnings` / `cargo test --locked` / `cd web && pnpm tsc -b && pnpm test`。

## 6. 不在本次范围（显式排除）

- task-run 调度 Kimi。
- 同 provider 内部重试（artifact retry / resume retry）。
- Kimi `plan` 模式暴露。
- thought chunk 展示给用户（仅写入 provider 流式日志，与 Claude Code thinking 一致）。
- **自由文本走 ACP `Other` 扩展字段**：协议明确非为用户输入设计，不采用；自由文本统一走下一轮 `session/prompt`。
- Pi 现有 `unreachable!` 的清理（不动 Pi）。

## 7. 兜底策略

设计核心假设（Supervised 可实现）已由 spike 实证成立。若未来 Kimi 版本移除 `session/request_permission`，则 Kimi 退化为 Auto-only（UI 隐藏 Supervised 选项），ACP 的会话强控、终态判定等其他优势仍成立，change 不阻塞。
