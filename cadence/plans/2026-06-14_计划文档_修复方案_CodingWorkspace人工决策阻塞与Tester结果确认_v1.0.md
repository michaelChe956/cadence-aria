# Coding Workspace 人工决策阻塞与 Tester 结果确认修复方案 v1.0

## 文档信息

- 文档类型：计划文档 / 修复方案
- 创建日期：2026-06-14
- 适用范围：Coding Workspace 后端状态机、真实 Provider choice 桥接（Claude Code `AskUserQuestion` / Codex `requestUserInput`）、Tester 结果确认 gate、前端选择/确认交互
- 目标分支：`bugfix_branch`
- 关联现象：
  - Coder 中出现三选一问题后，用户未决策前流程继续执行。
  - Tester 完成后未停在人工确认状态，而是继续进入 Analyst。

## 目标

1. Coder、Tester、Analyst 等角色运行中，只要 Provider 发起需要用户决策的 Claude Code `AskUserQuestion` 或 Codex `requestUserInput`，并转成统一的 `ProviderEvent::ChoiceRequest`，流程必须硬阻塞，直到用户明确提交选择。
2. Tester 生成测试报告后，必须先展示 Tester 总结和人工处理入口，不得自动进入 Analyst。
3. 人工确认 Tester 结果可用后，才进入 Analyst；人工不满意时，可以回退本次 Tester 并重新测试。
4. 前端展示必须反映后端真实状态，不能乐观标记选择已完成，也不能展示已经失效但仍可提交的选择卡片。
5. 运行态必须可验证当前后端是否加载了包含修复的二进制，避免“代码已改但服务未重启/未重编译”的误判。

## 问题一：Coder Provider Choice 未硬阻塞

### 已观察现象

在 `coding_attempt_0001` 对应 Claude Code transcript 中，Coder 于 `2026-06-14T08:08:41Z` 发起一次 `AskUserQuestion`：

- 问题：这个 Work Item 包含 16 个 TASK，本次 attempt 希望执行到哪个边界？
- 选项：
  - `先做后端TASK-001~009(推荐)`
  - `后端全做+前端骨架`
  - `全部16个TASK一次完成`

同一秒 transcript 返回：

```text
Error: Answer questions?
```

之后 Coder 于 `2026-06-14T08:08:47Z` 自行解释为“用户取消了选择”，并继续创建 TASK-001 到 TASK-009，随后写文件和运行测试。也就是说，用户没有作出选择前，Coder 已经继续执行。

这次现场证据来自 Claude Code 的 `AskUserQuestion`，但修复范围不能写成 Claude Code 专有。当前程序里 Codex 已有 `requestUserInput` 工具调用桥接，Spec / Story / Design Workspace 也已经有用户选择卡片和 pending choice 恢复链路。因此本问题应按“Coding Workspace 对统一 choice 请求缺少硬阻塞与持久化状态”处理。

### 现有实现基线

当前代码中已经存在可复用的统一抽象：

- `src/cross_cutting/streaming_provider.rs`
  - `ChoiceRequestSource::AskUserQuestion`
  - `ChoiceRequestSource::RequestUserInput`
  - `ChoiceRequestData`
  - `ProviderEvent::ChoiceRequest`
  - `ProviderCommand::ChoiceResponse`
- `src/cross_cutting/claude_code_provider.rs`
  - Claude Code `AskUserQuestion` 会被解析为 `ChoiceRequestSource::AskUserQuestion`。
- `src/cross_cutting/codex_provider.rs`
  - Codex `item/tool/requestUserInput` 会被解析为 `ChoiceRequestSource::RequestUserInput`。
  - 已有 fixture：`tests/fixtures/provider/codex_app_server_user_input_fixture.sh`。
- `src/product/workspace_engine.rs`
  - Spec / Story / Design Workspace 已有 `pending_author_choice`、`recover_pending_author_choice`、`take_pending_author_choice_prompt` 等 pending choice 恢复能力。

因此不需要重新发明一套 provider 专有协议；需要把 Coding Workspace 接到这套统一 choice 语义，并补齐 Coding Attempt 级别的持久 gate、重连恢复和阶段阻塞。

### 根因判断

该问题不应只归因于前端展示。真实根因在运行时语义：

1. `ProviderEvent::ChoiceRequest` 没有被提升为 Coding Workspace 的持久化人工 gate。
2. `ChoiceRequest` 主要作为实时 WebSocket 事件发给前端，缺少后端持久化的 `open/resolved/stale` 状态。
3. 对 Claude Code `AskUserQuestion` 或 Codex `requestUserInput` 的桥接失败，没有在 Coding Workspace 层形成统一的 protocol error/block 语义，模型或 runner 仍可能继续消费后续输出。
4. 前端发送选择后会本地乐观 resolve，缺少后端 ack，因此无法可靠区分“已被 provider 接收”和“choice 已失效”。

### 修复原则

- 用户决策点必须由后端状态机兜底，而不是依赖模型自觉等待。
- `AskUserQuestion` / `requestUserInput` 的桥接失败不是普通文本错误，必须终止当前 role run 或进入人工阻塞态。
- 只要 pending choice 未解决，不允许推进到下一个 Coding Workspace 阶段。
- 前端只能展示和提交后端认为 open 的 choice。

### 后端方案

#### 1. 复用统一 Provider Choice 语义

Provider 层保持当前统一协议：

- Claude Code `AskUserQuestion` -> `ProviderEvent::ChoiceRequest { source: AskUserQuestion }`
- Codex `requestUserInput` -> `ProviderEvent::ChoiceRequest { source: RequestUserInput }`
- 前端或人工选择 -> `ProviderCommand::ChoiceResponse`

Coding Workspace 不应按 provider 名称写死分支，而应按 `ChoiceRequestData.source` 和 `choice_id` 做持久化、恢复、校验和响应转发。这样 Claude Code 与 Codex 共享同一套状态机，只在 provider adapter 内保留协议差异。

#### 2. 新增持久化 Choice Gate

在 Coding Attempt 目录下新增持久化目录，例如：

```text
.aria/projects/{project}/issues/{issue}/coding-attempts/{attempt}/choice-gates/
```

新增模型建议：

```rust
pub enum CodingChoiceGateStatus {
    Open,
    Resolved,
    Stale,
    Cancelled,
}

pub struct CodingChoiceGate {
    pub choice_id: String,
    pub attempt_id: String,
    pub node_id: Option<String>,
    pub role: CodingProviderRole,
    pub stage: CodingExecutionStage,
    pub provider: ProviderName,
    pub source: ChoiceRequestSource,
    pub prompt: String,
    pub options: Vec<ChoiceOption>,
    pub allow_multiple: bool,
    pub allow_free_text: bool,
    pub status: CodingChoiceGateStatus,
    pub response: Option<ChoiceResponseRecord>,
    pub created_at: String,
    pub updated_at: String,
}
```

Store 接口：

- `create_choice_gate(...)`
- `list_open_choice_gates(project_id, issue_id, attempt_id)`
- `resolve_choice_gate(choice_id, response)`
- `mark_choice_gate_stale(choice_id, reason)`
- `cancel_choice_gates_for_node(node_id)`

#### 3. ProviderEvent::ChoiceRequest 处理变成硬 gate

在 `CodingWorkspaceEngine` 处理 `ProviderEvent::ChoiceRequest` 时：

1. 持久化 `CodingChoiceGate { status: Open }`。
2. attempt 状态更新为 `WaitingForHuman`。
3. 发送：
   - `CodingChoiceRequest`
   - `CodingSessionState`，包含 open choice gates。
4. 继续保持 provider session 等待，不进入下一阶段。

如果 WebSocket 暂时断开，后端仍保存 open choice。重连后前端从 snapshot 恢复卡片。

#### 4. ChoiceResponse 必须校验 open gate

`CodingWsInMessage::ChoiceResponse` 的处理流程调整为：

1. 查找 matching open choice gate。
2. 若不存在，返回 `CodingProtocolError { code: "coding_choice_gate_not_found" }`，前端标记 stale。
3. 若存在，持久化 response，状态改为 `Resolved`。
4. 转发 `ProviderCommand::ChoiceResponse` 给当前 runner。
5. 发送 ack 或 session snapshot，前端据此标记 resolved。

#### 5. Provider choice 桥接失败必须阻断

在 provider adapter 层分别加兜底，但对 Coding Workspace 暴露统一结果。

Claude Code：

- 对 `AskUserQuestion` 的 assistant tool_use 或 control_request，必须通过 `ApprovalBridge::request_choice` 等待 `ChoiceDecision`。
- 如果出现以下任一情况：
  - `request_choice` 返回错误
  - event receiver closed
  - choice response channel closed
  - Claude transcript 出现 `AskUserQuestion` 对应 `is_error: true` 工具结果
- 则发送 `ProviderEvent::ProtocolError`，错误码建议：

```text
ask_user_question_unresolved
```

Codex：

- 对 `item/tool/requestUserInput`，必须通过 `ApprovalBridge::request_choice` 等待 `ChoiceDecision`。
- 如果出现以下任一情况：
  - `request_choice` 返回错误
  - event receiver closed
  - choice response channel closed
  - 写回 `requestUserInput` JSON-RPC response 失败
- 则发送 `ProviderEvent::ProtocolError`，错误码建议：

```text
request_user_input_unresolved
```

Coding Workspace 收到后：

- 当前 role run 标记 `Blocked` 或 `Failed`。
- attempt 进入 `WaitingForHuman` 或 `Blocked`。
- 不允许继续执行该 provider 后续输出。

#### 6. 运行循环不允许吞掉 choice 相关命令

确认 `execute_coding_with_commands` / `execute_testing_with_provider_commands` 运行中通过 `tokio::select!` 同时监听：

- provider events
- runner command_rx

这条链路当前已有基础，但需要新增测试覆盖：

- provider 发出 choice 后，未响应前不会 completed。
- 前端响应后，命令能转发到 provider session。
- stale choice response 不会被静默丢弃。
- Claude Code `AskUserQuestion` 和 Codex `requestUserInput` 都走同一 Coding Choice Gate 状态机。

### 前端方案

#### 1. Snapshot 恢复 open choice gates

`CodingSessionState` 增加 `pending_choice_gates`。前端 store 在 set snapshot 时：

- 渲染 open choice gate 为 `choice_request` entry。
- 已 resolved 的 choice 显示响应摘要。
- stale/cancelled choice 显示失效原因，无提交按钮。

#### 2. 移除乐观 resolve

当前前端发送 `choice_response` 后会本地 resolve。需改为：

1. 点击提交后进入 `submitting`。
2. 等后端返回 ack 或 snapshot 中该 choice 变为 resolved。
3. 若收到 `coding_choice_gate_not_found` / `CHOICE_ID_UNMATCHED`，标记 stale。

#### 3. 交互文案

- Open：显示“等待人工选择”。
- Submitting：显示“提交中”。
- Resolved：显示“已选择：xxx”。
- Stale：显示“该选择已失效，需要重新运行当前角色”。

### 测试计划

后端：

- `claude_provider_ask_user_question_waits_for_choice_response`
- `claude_provider_fails_on_unresolved_ask_user_question`
- `codex_provider_request_user_input_waits_for_choice_response`
- `codex_provider_fails_on_unresolved_request_user_input`
- `coding_ws_choice_request_persists_gate_and_blocks_progress`
- `coding_ws_choice_response_resolves_gate_and_resumes_provider`
- `coding_ws_stale_choice_response_returns_protocol_error`
- `coding_ws_snapshot_recovers_open_choice_gate`
- `coding_ws_choice_gate_supports_ask_user_question_and_request_user_input_sources`

前端：

- `useCodingWorkspaceWs` 收到 snapshot 后恢复 open choice。
- `ChoiceRequestEntry` 提交后等待后端确认。
- stale choice 显示失效状态，不再展示提交按钮。

验证命令：

```bash
cargo test --locked --lib claude_code_provider
cargo test --locked --lib codex_provider
cargo test --locked --lib coding_workspace_engine
cargo test --locked --test it_web web_coding_ws_handler
pnpm -C web test -- useCodingWorkspaceWs
pnpm -C web test -- ChoiceRequestEntry
```

## 问题二：Tester 完成后未停在人工确认状态

### 预期行为

Tester 完成后，系统必须：

1. 持久化 TestingReport。
2. 展示 Tester 总结。
3. 创建“确认 Tester 测试结果”的人工 gate。
4. 停止自动推进。
5. 用户选择：
   - `结果可用，进入 Analyst`：把 TestingReport 作为 evidence 交给 Analyst。
   - `不满意，重新测试`：回退本次 Tester，重新执行测试。
   - `终止`：中止 attempt。

### 已观察现象

`coding_attempt_0001` 中：

- `testing_report_0001` 已生成，整体状态为 `failed`。
- `coding_node_0003` 于 `2026-06-14T08:50:49Z` 完成。
- 随后创建了普通 `rework` stage gate：`coding_stage_gate_0003`。
- `coding_stage_gate_0003` 于 5 秒后过期。
- 之后进入 Analyst，生成 `analyst_decision_0001`。

现场没有出现 reason_code 为 `testing_result_review_required` 的 blocked gate，也没有出现标题为“确认 Tester 测试结果”的 gate。

### 当前代码状态

`bugfix_branch` 当前 HEAD 为：

```text
77875d2 feat: gate tester results before analyst
```

该提交已经包含：

- `TESTING_RESULT_REVIEW_REASON_CODE = "testing_result_review_required"`
- `create_testing_result_review_gate(...)`
- Tester 完成后创建人工确认 gate 并 `return emit_current_session_state(...)`
- `accept_testing_result` / `rerun_testing` / `abort` 三类动作
- Web 集成测试覆盖：
  - 未人工确认前 Analyst 不应启动
  - 接受 Tester 结果后进入 Analyst
  - 不满意后重新测试

因此这次观察到的现象更像运行态未加载新逻辑，或测试发生在后端旧二进制上，而不是当前源码逻辑本身仍会自动进入 Analyst。

### 运行态风险

本次现场证据显示：

- attempt 中没有 `testing_result_review_required` gate。
- 出现的是旧路径的 `rework` stage gate。
- 当前源码中已包含新 gate 逻辑。

这说明需要补一层“运行产物可验证”能力，否则后续仍会出现“代码已改，但服务实际跑的是旧逻辑”的判断困难。

### 后端方案

#### 1. Tester 完成后强制创建 Testing Result Review Gate

保留并强化当前逻辑：

```rust
let testing_report = engine.execute_testing_with_provider_commands(...).await?;

if engine
    .create_testing_result_review_gate(&current, &testing_report)
    .await?
    .is_some()
{
    return emit_current_session_state(event_tx, coding_store, &current).await;
}
```

要求：

- 不根据测试是否 passed/failed 决定是否创建确认 gate。
- 不根据 `testing_report_should_enter_analyst` 直接进入 Analyst。
- 创建 gate 后必须 `return`，不能继续走后续 rework/analyst 分支。

#### 2. Gate 类型与行为

Blocked gate：

```text
reason_code = testing_result_review_required
stage = testing
role = tester
title = 确认 Tester 测试结果
```

Actions：

- `accept_testing_result`
- `rerun_testing`
- `abort`

`accept_testing_result`：

1. 关闭当前 review gate。
2. 把 `testing_report_0001` 转换为 analyst evidence。
3. 创建/进入 `rework` 阶段。
4. 调用 Analyst。

`rerun_testing`：

1. 关闭当前 review gate。
2. 将 testing role run supersede 或创建新 run。
3. 回到 Testing 阶段重新执行。
4. 新 Tester 结果再次进入人工确认 gate。

`abort`：

1. 关闭 gate。
2. attempt 标记 `Aborted`。
3. active runner 退出。

#### 3. 避免 stage gate 自动过期绕过人工确认

普通 `await_stage_gate` 有 5 秒倒计时，过期后会自动推进。这适合阶段开始前的 provider 选择确认，但不适合 Tester 结果确认。

要求：

- `testing_result_review_required` 必须使用 blocked gate，不使用可自动过期的 stage gate。
- blocked gate 不允许自动确认。
- blocked gate 只通过明确 action 变更状态。

#### 4. 防止旧 open gate 干扰

`create_testing_result_review_gate` 前后应明确处理已有 gate：

- 如果已有同 reason_code 的 open gate，复用并返回它。
- 如果已有 Testing 阶段其他 blocked gate，保留，不重复创建 review gate。
- 如果已有过期 stage gate，不应影响 review gate 创建。

#### 5. 运行产物版本校验

新增调试能力，解决“是不是没重启”的判断问题：

1. 后端启动日志打印：

```text
Aria build fingerprint: git_sha=..., built_at=..., binary_mtime=...
```

2. 新增或扩展健康接口，例如：

```text
GET /api/runtime-info
```

返回：

```json
{
  "git_sha": "77875d2...",
  "branch": "bugfix_branch",
  "built_at": "...",
  "workspace_root": "...",
  "features": {
    "testing_result_review_gate": true,
    "coding_choice_gate": true
  }
}
```

3. 前端开发环境可在控制台或隐藏诊断面板显示 runtime info。

这样下次看到现象时，可以直接判断：

- 代码没有部署：runtime info 不含 feature。
- 代码已部署但状态仍错：继续排查状态机。

### 前端方案

#### 1. Tester Report 总结展示

在 Coding Workspace 页面测试报告区域展示：

- overall status
- plan summary
- failed steps summary
- evidence refs
- context warnings
- raw provider output link

不需要等 Analyst 才展示测试结论。

#### 2. Gate 操作区域

当 `pending_gates` 包含：

```text
reason_code = testing_result_review_required
```

前端展示独立操作条：

- 标题：`确认 Tester 测试结果`
- 主按钮：`结果可用，进入 Analyst`
- 次按钮：`不满意，重新测试`
- 危险按钮：`终止`

按钮点击后发送 `gate_response` 或现有 blocked gate action 协议。

#### 3. 禁止误展示 Analyst 进行中

在该 gate open 时：

- 不显示 Analyst running。
- 不创建 Analyst timeline 节点。
- 如果 snapshot 中同时存在 open tester review gate 和 analyst node，前端应提示数据不一致，便于定位后端状态机错误。

### 测试计划

后端 WebSocket 集成：

- `coding_ws_testing_result_waits_for_human_before_analyst`
  - Tester 完成后收到 `testing_result_review_required` gate。
  - 在未响应 gate 前，不出现 analyst role run。

- `coding_ws_accept_testing_result_enters_analyst_with_testing_report_evidence`
  - 点击 `accept_testing_result` 后进入 Analyst。
  - Analyst prompt 中包含 testing report evidence。

- `coding_ws_rerun_testing_result_review_reexecutes_tester`
  - 点击 `rerun_testing` 后重新执行 Tester。
  - 新的 testing report 再次创建 review gate。

- `coding_ws_testing_result_review_gate_survives_reconnect`
  - 断开重连后 snapshot 仍包含 open gate。

前端：

- `CodingWorkspacePage` 渲染 Tester 结果确认 gate。
- 点击接受后调用正确 action。
- 点击重新测试后调用 rerun action。
- open tester review gate 时不渲染 Analyst running 状态。

验证命令：

```bash
cargo test --locked --test it_web web_coding_ws_handler
cargo test --locked --lib coding_workspace_engine
pnpm -C web test -- CodingWorkspacePage
```

## 分阶段实施计划

### P0：确认运行态

1. 重启后端，确保 `cargo run --locked -- web --workspace . --host 127.0.0.1 --port 4317` 来自 `.worktrees/bugfix_branch`。
2. 增加 runtime info 或至少启动日志，确认运行二进制包含 `testing_result_review_required`。
3. 用新 attempt 验证 Tester 后是否出现 `确认 Tester 测试结果` gate。

验收：

- `GET /api/runtime-info` 或日志能看到当前 git sha。
- 新 attempt 的 Tester 结束后，`pending_gates` 中出现 `testing_result_review_required`。

### P1：Tester 结果确认闭环加固

1. 确认 `create_testing_result_review_gate` 在所有 Tester 完成路径都会执行。
2. 对 `execute_testing` 非 provider 路径和 provider 路径都补测试。
3. 修复任何仍会自动进入 Analyst 的路径。
4. 前端补 Tester summary 和 gate action 展示。

验收：

- 未人工确认前，Analyst 不启动。
- 接受后，Analyst 启动并收到 testing report evidence。
- 重新测试后，生成新的 Tester run。

### P2：Choice Gate 持久化与硬阻塞

1. 增加 `CodingChoiceGate` 数据模型和 store。
2. `ProviderEvent::ChoiceRequest` 创建 open choice gate。
3. `ChoiceResponse` 必须 resolve 后才转发。
4. Claude Code `AskUserQuestion` unresolved 与 Codex `requestUserInput` unresolved 都进入 protocol error/block，不允许模型继续。
5. 前端移除 optimistic resolve，改为 ack/snapshot 驱动。

验收：

- Coder 三选一问题未回答前，不创建后续文件修改，不进入后续阶段。
- 用户提交选择后，同一 provider session 继续。
- stale choice 不可提交，前端明确提示。

### P3：全量回归

运行：

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked
pnpm -C web test
pnpm -C web build
```

真实 E2E：

1. 新建 coding attempt。
2. 分别触发 Coder 的 Claude Code `AskUserQuestion` 与 Codex `requestUserInput`。
3. 不选择，观察流程停住。
4. 选择后继续。
5. Tester 完成后，观察 Tester summary 和人工确认 gate。
6. 点击重新测试，确认重新执行 Tester。
7. 再次完成后点击进入 Analyst，确认 Analyst 收到 testing evidence。

## 风险与注意事项

1. Provider session 长时间等待 choice 时，需要确认取消/中止能正确发送 `ProviderCommand::Abort`。
2. WebSocket 断开时，provider 仍可能等待选择；必须依赖持久化 choice gate 恢复 UI。
3. 若 provider 在 `AskUserQuestion` / `requestUserInput` 桥接失败后仍输出内容，Aria 必须忽略后续输出并终止当前 role run。
4. Tester result review gate 与已有 stage gate 是不同类型，不应共用 5 秒自动过期逻辑。
5. 旧 attempt 已经进入 Analyst 的历史状态不能作为新逻辑验证依据，需要新 attempt 验证。

## 完成标准

- Coder 的 Claude Code `AskUserQuestion` 或 Codex `requestUserInput` 未回答前不会继续执行。
- Tester 完成后一定停在人工确认 gate。
- 前端刷新或重连后，open choice 和 tester review gate 都能恢复。
- 人工接受 Tester 结果后才进入 Analyst。
- 人工选择重新测试后，能生成新的 Tester run 和新的测试报告。
- Runtime info 能证明当前运行服务加载了包含修复的构建。
