# Codex Resume 卡住治理技术方案

## 文档信息

- 文档类型：技术方案
- 日期：2026-06-07
- 版本：v1.0
- 关联现象：Coding Workspace 第二轮 `Coder · Codex` 返修节点长时间停留在 `running`
- 关联分支：`fix_author_confirm_followup`
- 关联既有方案：`2026-06-01_技术方案_Provider会话按角色续接_v1.0.md`

## 背景

Coding Workspace 在一轮代码编写、测试、审查后，如果 CodeReviewer 要求修改，会进入 Rework 分析，再启动下一轮 Coder 修复。当前真实 E2E 中，第二轮 `Coder · Codex` 节点 `coding_node_0007` 卡在 `running`：

- attempt 状态为 `running`，stage 为 `coding`。
- active node 为 `coding_node_0007`，无 pending gate。
- 本轮启动了新的 `codex app-server` 进程。
- Codex 日志显示 app-server 收到 `initialize`、`initialized`、`turn/start`，但没有进入后续 session loop，也没有 assistant 输出。
- `provider_conversations` 中已有第一轮 `coder + codex` 的 thread id，因此第二轮 Coder 会尝试 resume。

该问题暴露出两个独立但相互放大的缺陷：

1. Codex app-server resume 调用序列不符合当前官方协议和本机 schema。
2. Aria JSON-RPC request 缺少 request 级 timeout，导致 `turn/start` 不回包时节点永久运行。

## 已确认事实

### Codex app-server 正确生命周期

官方 Codex manual 对 app-server 生命周期的描述是：

- 新建会话：调用 `thread/start`。
- 继续已有会话：调用 `thread/resume`。
- 开始用户 turn：调用 `turn/start`，并传入目标 `threadId` 与输入。

本机 `codex-cli 0.136.0` 生成的 app-server JSON schema 也明确包含：

- `ClientRequest` 支持 `thread/resume`。
- `ThreadResumeParams` 必填 `threadId`，并可传 `cwd`、`approvalPolicy`、`sandbox` 等覆盖项。
- `TurnStartParams` 必填 `threadId` 和 `input`。

因此 Codex resume 的推荐序列应为：

````text
initialize
initialized
thread/resume { threadId, cwd, approvalPolicy, ... }
turn/start { threadId, input }
````

而不是直接：

````text
initialize
initialized
turn/start { threadId: old_thread_id, input }
````

### Aria 当前实现

当前 `CodexProvider` 行为：

- 无 `resume_provider_session_id`：发送 `thread/start`，再发送 `turn/start`。
- 有 `resume_provider_session_id`：跳过 `thread/start`，直接用旧 id 调 `turn/start`。
- `JsonRpcPeer::request()` 只等待 response，没有 timeout。
- CodexProvider 的 `timeout_secs` 在 `turn/start` 返回后才进入事件循环，因此不能覆盖 `turn/start` 本身卡住的情况。

当前 Coding Workspace 会话策略：

- `provider_conversations` 按 `ProviderConversationRole + ProviderName` 查找。
- Coder、Tester、Analyst、CodeReviewer、InternalReviewer 代码路径都会尝试读取同角色同 provider 的旧 session。
- 该策略不会跨角色复用 session，例如 tester 不会复用 coder session。

## 目标

1. 修复 Codex app-server resume 协议，避免第二轮 Coder 卡在 `turn/start`。
2. 为 provider JSON-RPC 请求增加超时，确保 provider 启动和 turn 启动失败能显式落到 failed/blocked。
3. 明确 Coding Workspace 五类节点的 resume 策略，避免 reviewer/tester 被旧上下文污染。
4. 规范各节点 prompt：每轮 prompt 必须自包含完成任务所需的关键上下文，不能依赖 provider 历史会话才能正确运行。
5. 保持 provider conversation 的角色隔离，不跨角色、不跨 provider、不跨 attempt 复用。
6. 保留可观测性：前端和持久化日志必须能看见 resume、turn start、timeout、fallback 或失败原因。

## 非目标

- 不重构完整 provider run 审计系统。
- 不改变 Claude Code 的 `--resume <session_id>` 基本调用方式。
- 不跨 attempt 复用 Coding Workspace provider session。
- 不在本方案中实现自动修复 CodeReviewer 指出的业务代码问题。
- 不用静默新建 thread 掩盖 resume 协议错误。

## 总体方案

采用“三层治理”：

1. **协议层修正**：Codex 有 resume id 时先调用 `thread/resume`，成功后再 `turn/start`。
2. **超时层兜底**：JSON-RPC request 增加有限超时，覆盖 `initialize`、`thread/start`、`thread/resume`、`turn/start`。
3. **产品层策略**：按角色明确哪些节点适合 resume，哪些节点应默认 fresh thread，并保证 prompt 自包含。

## Codex Provider 协议改造

### 新建 thread

无 `resume_provider_session_id` 时保持当前主流程：

````json
{ "method": "thread/start", "params": { "cwd": "...", "approvalPolicy": "never", "ephemeral": true } }
{ "method": "turn/start", "params": { "threadId": "<new_thread_id>", "input": [...] } }
````

### Resume thread

有 `resume_provider_session_id` 时改为：

````json
{
  "method": "thread/resume",
  "params": {
    "threadId": "<old_thread_id>",
    "cwd": "<attempt_worktree>",
    "approvalPolicy": "never"
  }
}
````

然后从 `thread/resume` response 中确认 thread id：

1. 优先读取 `/thread/id`。
2. 如果 response 没有返回 thread id，则使用原 `resume_provider_session_id`。
3. 使用确认后的 thread id 调 `turn/start`。

### 错误处理

Codex resume 失败时：

- 将 provider event 标记为 failed/protocol error。
- timeline node 标记 failed。
- attempt 状态进入 blocked 或 failed，具体沿用现有 `fail_provider_stream` 语义。
- 前端展示明确错误，例如：

````text
Codex thread/resume failed for thread 019e...: <provider error>
````

默认不静默退化为新 thread。原因是 Coder 返修可能依赖上一轮修改语境，静默新 thread 会让用户误以为 resume 生效，也会掩盖 provider 协议问题。

可选增强：后续可以增加人工确认式 fallback gate：

````text
Codex resume 失败。是否改用新 thread 继续？新 thread 将只依赖当前 prompt、worktree 和 diff。
````

但该 fallback 不作为本轮首选。

## JSON-RPC Timeout 改造

### 问题

当前 request 等待 response 的逻辑没有 timeout：

````text
send request
await response_rx
````

如果 app-server 对 `turn/start` 不回包，provider.start 不返回，CodingWorkspaceEngine 也无法进入 provider event loop，最终 UI 永久显示 running。

### 方案

增加 request 级 timeout：

````rust
pub async fn request_with_timeout(
    &self,
    payload: Value,
    timeout: Duration,
) -> Result<Value, ProviderAdapterError>
````

CodexProvider 对关键请求使用该方法：

- `initialize`：建议 30 秒。
- `thread/start`：建议 60 秒。
- `thread/resume`：建议 60 秒。
- `turn/start`：建议 60 秒。

timeout 后必须：

- 从 pending map 移除该 request id。
- 返回 `ProviderAdapterError::timeout` 或明确的 execution failed。
- 关闭或 cancel 当前 provider process，避免 app-server 孤儿进程继续占用资源。

### UI 事件

建议在关键边界发送 provider execution event：

- `Initialize started/completed`
- `Thread resume started/completed`
- `Turn start started/completed`
- `Request timeout`

这样用户能区分“模型正在输出”和“provider 协议启动卡住”。

## Coding Workspace Resume 策略

当前实现是所有角色都有历史 session 就 resume。建议改为显式策略表。

| 角色 | 推荐默认策略 | 原因 |
|---|---|---|
| `Coder` | resume | Coder 返修最可能受益于上一轮实现思路、已读文件和修改上下文。 |
| `Tester` | 不 resume | Tester 应基于当前 worktree、命令和测试输出独立验证，旧对话价值低，可能带来偏见。 |
| `Analyst` | 不 resume | Analyst prompt 已包含上一阶段 evidence 和 ContextNote，应做独立路由判断。 |
| `CodeReviewer` | 不 resume | Review 应基于当前 diff 独立审查，resume 可能继承上一轮结论或遗漏新变化。 |
| `InternalReviewer` | 不 resume | Internal PR review 应基于最终 diff、ReviewRequest 和 commit 状态独立判断。 |

### 实现方式

新增策略函数：

````rust
fn should_resume_provider_conversation(role: &CodingProviderRole, provider: &ProviderName) -> bool
````

初始规则：

````text
Coder -> true
Tester -> false
Analyst -> false
CodeReviewer -> false
InternalReviewer -> false
````

后续如果用户或配置需要，可以扩展为 role provider config：

````json
{
  "resume_policy": {
    "coder": "resume",
    "tester": "fresh",
    "analyst": "fresh",
    "code_reviewer": "fresh",
    "internal_reviewer": "fresh"
  }
}
````

但 v1.0 建议先用固定策略，减少配置面。

## Prompt 策略

核心原则：**resume 是上下文增强，不是正确性的必要条件。**

每轮 prompt 必须包含该角色完成任务所需的最小完整上下文。

### Coder Prompt

Coder 初次实现需要包含：

- project、issue、work item、attempt、branch、worktree path。
- 已确认 Work Item markdown。
- 验证命令。
- 执行要求：真实修改代码、遵循仓库规则、完成后报告测试。

Coder 返修需要额外包含：

- 上一轮 reviewer/analyst 的 summary。
- `fix_hints`。
- 待澄清问题。
- 必须检查 `git diff/status`。
- 明确优先修复 review findings，而不是重新规划。

建议补强：

- 如果 CodeReview findings 有结构化 file_path/line/required_action，应在 Coder prompt 中直接列出完整 findings，而不是只放 analyst summary。
- 如果已有测试报告，应附上失败/通过命令摘要。

### Tester Prompt

Tester prompt 应包含：

- worktree path。
- 允许工具和禁止工具。
- 可用测试命令。
- 当前变更文件。
- Work Item 验证命令和 markdown。
- 输出 JSON schema。

Tester 不依赖历史会话，必须每轮 fresh。

### Analyst Prompt

Analyst prompt 应包含：

- previous stage。
- rework round。
- 上一阶段完整 evidence。
- 本轮 ContextNote。
- JSON 输出 schema。

Analyst 不修改代码、不执行命令，不需要 resume。

### CodeReviewer Prompt

CodeReviewer prompt 应包含：

- Work Item markdown。
- base branch。
- 当前 git diff。
- findings 输出结构要求。
- 只读审查约束。

CodeReviewer 不需要 resume，避免历史结论影响当前 diff 审查。

### InternalReviewer Prompt

InternalReviewer prompt 应包含：

- ReviewRequest。
- review remote。
- commit。
- Work Item markdown。
- 完整 git diff。
- PR description、impact scope、commit message suggestion 输出要求。

InternalReviewer 不需要 resume。

## Provider Conversation 持久化策略

即使某角色默认不 resume，也可以继续记录其 `provider_session_id`，用于审计和调试。

读取 resume id 时增加策略过滤：

````text
if should_resume_provider_conversation(role, provider) {
    provider_resume_session_id_for_attempt(...)
} else {
    None
}
````

完成时仍可记录：

````text
record_attempt_provider_session(...)
````

这样保留调试能力，同时避免默认续接。

## 验收标准

### Codex 协议

1. 有 `resume_provider_session_id` 时，CodexProvider 先发送 `thread/resume`，再发送 `turn/start`。
2. 无 `resume_provider_session_id` 时，仍发送 `thread/start`，再发送 `turn/start`。
3. `thread/resume` 失败时节点显式 failed/blocked，不永久 running。
4. `turn/start` 不回包时，在 request timeout 后失败退出。

### Coding Workspace 策略

1. Coder 第二轮会尝试 resume 同角色同 provider session。
2. Tester 不 resume tester 历史 session。
3. Analyst 不 resume analyst 历史 session。
4. CodeReviewer 不 resume code reviewer 历史 session。
5. InternalReviewer 不 resume internal reviewer 历史 session。
6. 所有角色仍可记录完成后的 provider session id。

### Prompt

1. Coder 返修 prompt 包含 review/analyst 结构化修复信息。
2. Reviewer/Tester/Analyst prompt 均可在 fresh thread 中独立完成任务。
3. 前端 Provider Prompt 事件能看到完整 prompt，便于 E2E 排查。

### 可观测性

1. UI 能显示 provider 正在 initialize、resume thread、start turn 或 timeout。
2. 失败 summary 包含方法名和 provider 错误摘要。
3. 卡住的 app-server 进程会被 cancel/回收。

## 测试计划

### 单元测试

- `codex_resume_calls_thread_resume_before_turn_start`
- `codex_new_session_still_calls_thread_start`
- `codex_thread_resume_timeout_returns_provider_error`
- `json_rpc_request_with_timeout_removes_pending_request`
- `coding_resume_policy_only_resumes_coder`

### 集成测试

- CodingWorkspaceEngine：构造已有五类 provider conversations，验证只有 Coder input 带 resume id。
- Codex fixture：模拟 app-server 收到 `thread/resume` 后返回 thread，再收到 `turn/start` 后 completed。
- Codex fixture：模拟 `thread/resume` 不回包，验证 request timeout 后节点失败。

### 真实 E2E

复用 2026-06-07 真实流程：

1. 使用真实项目、真实 issue、真实 target repo。
2. Coder 选择 Codex，Reviewer 选择 Claude Code。
3. 第一轮 Coder 完成。
4. CodeReviewer 返回 request_changes。
5. Analyst 生成 rework instruction。
6. 第二轮 Coder 启动。
7. 验证第二轮 Codex 不再卡在 `turn/start`，且 UI 显示 `thread/resume`/`turn/start` 事件。

## 风险与取舍

### 风险 1：Codex `thread/resume` 对 ephemeral thread 的兼容性

当前第一轮 `thread/start` 使用 `ephemeral: true`。如果 ephemeral thread 不保证可跨 app-server 进程恢复，那么即使用 `thread/resume` 也可能失败。

缓解：

- 用真实 app-server fixture 或本机 schema 行为验证。
- 如确认 ephemeral 不可恢复，Coder 需要改为非 ephemeral thread，或记录可恢复 rollout path。
- 在失败时显式报错，不永久 running。

### 风险 2：Coder fresh fallback 可能丢上下文

如果 resume 失败后自动 fresh thread，Coder 仍可依赖自包含 prompt 和当前 worktree 修复，但会丢失上一轮 provider 对话上下文。

缓解：

- v1.0 不自动 fallback。
- 后续通过人工 gate 确认是否 fresh retry。

### 风险 3：Reviewer 不 resume 可能增加 token

Reviewer 每轮 fresh 需要重新读取 diff 和上下文。

取舍：

- Review 正确性优先于 token 节省。
- diff prompt 已经是当前审查的权威输入，fresh 更可控。

## 实施顺序

1. 增加 Codex app-server resume fixture：期望 `thread/resume -> turn/start`。
2. 修改 CodexProvider resume 分支，调用 `thread/resume`。
3. 增加 JSON-RPC request timeout API，并迁移 CodexProvider 关键 request。
4. 增加 Coding Workspace resume policy，只允许 Coder 默认 resume。
5. 补强 Coder 返修 prompt，直接包含 CodeReview findings。
6. 增加 provider execution event，暴露 initialize/resume/start/timeout 状态。
7. 跑单元测试、集成测试、真实 E2E。

## 待确认问题

1. Codex app-server `ephemeral: true` thread 是否可在新 app-server 进程中 `thread/resume`。
2. Codex thread resume 失败后，产品是否允许用户手动选择 fresh retry。
3. Tester/Analyst/Reviewer 默认 fresh 是否需要做成可配置项，还是先固定策略。
4. request timeout 的默认时长是否统一为 60 秒，还是区分 initialize/thread/turn。

