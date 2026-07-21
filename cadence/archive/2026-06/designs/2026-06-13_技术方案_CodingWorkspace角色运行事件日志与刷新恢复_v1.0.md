# CodingWorkspace 角色运行事件日志与刷新恢复技术方案

## 背景

Coding Workspace 已有角色 run 模型，能够把 Tester、Analyst、Code Reviewer、Internal Reviewer 的最终产物、raw provider output、chat entry、role run status 和 retry 关系串起来。但真实调试仍暴露出一个缺口：运行中的 provider 事件流主要通过 WebSocket 实时发给前端，没有完整落盘到 `.aria`。

这会带来三个问题：

- 刷新页面后，只能看到最终产物和少量可读 chat，看不到运行中曾经发生过什么。
- Provider 卡在 task/progress/tool update 时，历史 role run 有状态提示，但缺少可排查内容。
- 重试只能依赖 raw output、report 或少量 reason code，无法稳定引用上一轮“卡在哪个事件”。

本方案补齐 Coding Workspace 的角色运行事件日志，让实时输出、刷新恢复、故障排查和刷新后重试使用同一套可追溯证据链。

## 目标

1. 为 Tester、Analyst、Code Reviewer、Internal Reviewer 持久化 provider 运行过程事件。
2. 保持现有实时 WebSocket 输出，不因新增持久化而削弱实时体验。
3. 刷新页面后能看到每个 role run 的最近进度、终止原因和诊断线索。
4. retry 时保留旧 run 完整事件链，新 run 使用上一轮诊断摘要和 refs，不注入完整日志全文。
5. 保持 JSON-only 最终产物契约，同时允许过程事件承载用户可读进度。

## 非目标

- 不覆盖 Coder 角色。本轮只覆盖 Tester、Analyst、Code Reviewer、Internal Reviewer。
- 不改变最终 Provider 输出契约。TestPlan、TestingReport、AnalystDecision、CodeReviewReport、InternalPrReview 仍由结构化 JSON 解析。
- 不把完整事件日志塞进主 chat 对话流。
- 不在第一版实现完整日志 artifact 预览弹窗。第一版可展示 ref，后续再做按需查看。
- 不自动修复历史遗留的 running run。第一版先让日志暴露真实状态。

## 方案选择

### 方案 A：Role Run Event Log

为每个 `CodingRoleRun` 单独保存事件日志，例如 `role-run-events/<role_run_id>.jsonl`。role run 主 JSON 继续保存状态、trigger、refs 和关系；事件日志保存 provider 过程。

优点：

- 与现有 role run 模型贴合。
- 高频事件不会反复重写 role run 主 JSON。
- 支持刷新恢复、按需加载和后续日志裁剪。
- 能把诊断日志与主 chat 分离。

缺点：

- 需要新增 store 方法、snapshot DTO 和 UI 展示逻辑。

### 方案 B：events 嵌入 CodingRoleRun

把事件数组直接放进 `CodingRoleRun`。

优点是实现简单。缺点是大 run 会膨胀主 JSON，高频写入会反复重写同一个文件，后续按需加载困难。

### 方案 C：复用 chat entry 保存事件

把 provider event 转成 chat entry。

优点是前端改动少。缺点是会污染主对话流，难以区分可读结论和诊断事件，也不利于 retry 诊断摘要。

推荐采用方案 A。

## 数据模型

新增 `CodingRoleRunEvent`：

```text
CodingRoleRunEvent
- attempt_id
- role_run_id
- node_id
- stage
- role
- sequence
- event_type
- created_at
- payload
- truncated
- artifact_ref
```

`event_type` 覆盖：

- `provider_prompt`
- `provider_start`
- `text_delta`
- `execution_event`
- `tool_call`
- `tool_result`
- `status_changed`
- `permission_request`
- `choice_request`
- `message_complete`
- `provider_failed`
- `timeout`
- `aborted`
- `persistence_warning`

推荐落盘路径：

```text
coding-attempts/<attempt_id>/role-run-events/<role_run_id>.jsonl
```

事件采用 JSON Lines 追加写入。`sequence` 在单个 role run 内递增，读取时按 sequence 排序。

### 大字段策略

事件 payload 中可能出现大字段：

- prompt
- text delta content
- execution output
- tool input/output
- error stderr

策略：

- 小字段直接写入 payload。
- 单字段超过阈值时截断 payload，完整内容写到 artifact。
- event 中设置 `truncated = true`，并写入 `artifact_ref`。

推荐 artifact 路径：

```text
artifacts/role-run-events/<role_run_id>/<sequence>_<field>.txt
```

现有 `provider-raw/...` 仍保存最终 raw provider output。事件 artifact 不替代 raw output。

## 写入流程

写入点放在 `CodingWorkspaceEngine::run_provider_stream_to_completion` 及其调用上下文。

流程：

1. Tester、Analyst、Code Reviewer、Internal Reviewer 创建或 attach role run。
2. 调用 provider 前记录 `provider_prompt`，大 prompt 写 artifact。
3. provider start 成功记录 `provider_start`。
4. provider start 失败或超时记录 `provider_failed` 或 `timeout`。
5. 每个 provider event 到达后规范化为 `CodingRoleRunEvent` 并 append `.jsonl`。
6. 同一个 event 继续按现有逻辑发送 WebSocket。
7. provider completed 时记录 `message_complete`，再保存现有 raw provider output。
8. timeout、abort、protocol error、permission timeout 都写 terminal event。
9. role run status 仍由现有 role run 主 JSON 更新，不重复写到事件日志作为唯一状态源。

### 实时输出原则

事件处理采用双写：

```text
ProviderEvent
  -> normalize to CodingRoleRunEvent
  -> append .aria jsonl
  -> send WebSocket event
  -> update existing report/chat/run status when applicable
```

实时输出不能因为持久化失败而中断。若 append event 失败：

- 继续发送 WebSocket。
- 尽量追加一条 `persistence_warning`。
- 如果连 warning 也失败，只记录后端日志，不阻断 provider 流。

## JSON 契约与过程输出

JSON-only 只约束最终产物，不约束过程事件。

最终产物仍是机器可解析 JSON：

- Tester 最终输出 TestPlan / TestingReport JSON。
- Analyst 最终输出 AnalystDecision JSON。
- Code Reviewer 最终输出 CodeReviewReport JSON。
- Internal Reviewer 最终输出 InternalPrReview JSON。

过程事件不参与最终 JSON 解析，可以承载实时可读进度：

- text delta
- task/progress update
- execution event
- tool call/result
- status changed
- permission/choice request
- stderr/error/timeout 诊断

前端显示这些过程事件的规范化版本，而不是要求 provider 在最终 JSON 中额外输出 markdown。这样既保留 JSON 契约，也解决 JSON-only prompt 导致用户看不到实时过程的问题。

## 刷新恢复

`CodingSessionState` 增加 role run 事件摘要。

每个 role run 可返回：

```text
event_summary
- event_count
- last_event_at
- last_event_type
- last_event_title
- last_event_status
- terminal_event_type
- terminal_reason

recent_events
- 最近 5 到 10 条规范化事件
```

第一版只需要在 WebSocket snapshot 中返回摘要和 recent events。完整日志可以后续增加按需 API。

刷新页面后：

- running run 显示最近事件和当前卡点。
- blocked run 显示 terminal event、reason code 和 retry actions。
- completed run 显示 event count 和最后完成事件。
- superseded run 保留完整事件链，但默认折叠。

## UI 展示

主 chat 区保持可读结果为主，不直接恢复全量日志流。

`RoleRunHistoryPanel` 扩展展示：

- role / run no / status / trigger / reason code
- raw refs / artifact refs
- event count
- last event title / status
- recent events 列表

默认行为：

- 当前 running run 默认展开 recent events。
- blocked run 默认显示 terminal event 和最近事件。
- completed / superseded run 默认折叠。
- 点击 run 可以定位对应 timeline node。
- 有 `artifact_ref` 的事件先显示 ref 文本，后续再支持查看内容。

## 重试语义

重试不销毁旧日志。

当用户触发：

- `retry_test_plan`
- `rerun_missing_steps`
- `retry_analyst`
- `retry_review`
- `retry_internal_review`

系统行为：

1. 旧 role run 标记为 superseded。
2. 旧 role run events 保留。
3. 新建 role run。
4. 新 run 写入自己的 events。
5. retry prompt 只引用旧 run 的诊断摘要和 refs。

retry diagnostic summary 来源：

- 旧 run reason code。
- terminal event。
- 最近关键事件的 title/status。
- raw provider output refs。
- event artifact refs。

retry prompt 不注入完整日志全文，避免 prompt 过长或污染 provider 判断。

## 错误处理

### Provider start 超时

记录：

- `timeout`
- reason code，如 `plan_tests_timeout`
- role run id
- node id

UI 刷新后显示超时前已收到的事件。

### Provider event stream 中断

记录 `provider_failed`，payload 包含错误摘要。若已有 partial output，保留在 events 或 artifact 中。

### 日志写入失败

不阻断实时输出。尽量记录 `persistence_warning`。role run status 不因日志写入失败自动 failed。

### 大事件截断

payload 保留摘要和 artifact ref。UI 显示“已截断”提示和 ref。

## 测试策略

### 后端 Store

- append/list role run events。
- sequence 稳定递增。
- JSONL 追加后可恢复排序。
- 大字段截断并生成 artifact ref。
- artifact ref 通过路径校验，不能逃逸 attempt 目录。

### Engine

- `TextDelta` 写 event 且继续发 WebSocket。
- `ExecutionEvent` 写 event 且继续发 WebSocket。
- tool call/result 写 event。
- permission/choice request 写 event。
- provider completed 写 `message_complete`。
- timeout 写 terminal event。
- 持久化失败不阻断 WebSocket event。

### Snapshot / API

- `CodingSessionState` 包含 role run event summary。
- running run 刷新后能看到 recent events。
- blocked run 刷新后保留 terminal event。

### 前端

- `RoleRunHistoryPanel` 显示 event count、last event、recent events。
- running run 默认展开 recent events。
- blocked/superseded run 可展开查看。
- 主 chat 区不被 event log 淹没。

### E2E

- Tester `plan_tests` 期间只发过程事件、不发最终 JSON 时，页面实时显示进度。
- 刷新后仍能看到 Tester 最近事件。
- Tester 超时后 blocked gate 保留最近事件和 retry action。
- Analyst/Reviewer/InternalReviewer retry 后，新 run 继续实时输出，旧 run 日志可追溯。
- 刷新后点击 retry，prompt 使用上一轮 diagnostic summary + refs。

## 实施边界

建议拆成三个实施阶段：

1. 后端 event log store 与 engine 双写。
2. WebSocket snapshot 与前端 RoleRunHistoryPanel 展示。
3. retry diagnostic summary 接入 prompt，并补真实 E2E。

每阶段都按 TDD 实施，先补失败测试，再改实现。

## 验收标准

- Tester、Analyst、Code Reviewer、Internal Reviewer 的 role run 都能产生日志。
- 实时 WebSocket 输出仍正常。
- 刷新页面后可看到 running/blocked run 的最近事件。
- JSON-only 最终产物契约不变。
- retry 不覆盖旧日志，新 run 有独立日志。
- `No tasks found` 等 provider 过程输出如果出现，会在 recent events 或 event artifact 中可追溯。
