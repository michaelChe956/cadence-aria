# CodingWorkspace Provider QA P2 Provider 运行时与恢复闭环实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 P1 的基础模型接入 Coding Workspace 后端运行时，让 Tester、Analyst、Code Reviewer、Internal Reviewer 都使用 OpenSpec/Superpowers，并在 blocked 时生成可恢复 gate。

**Architecture:** Testing 改为 provider 两段式 `plan_tests` -> `execute_test_plan`；Review 保存 raw output 并容错解析；WebSocket session state 合并 blocked gate，`gate_response` 触发恢复动作。

**Tech Stack:** Rust 1.95、tokio、StreamingProviderAdapter、Axum WebSocket、CodingAttemptStore、Cargo。

---

## 依赖与边界

- 必须先完成 P1。
- 本阶段不改前端 UI；只保证 WebSocket 输出和协议可用。
- 不在 Aria 中硬编码测试命令、语言生态或安全工具。

## 文件结构

- Modify: `src/product/tester_agent_loop.rs`
- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `src/web/coding_ws_handler.rs`

## Task 1: Tester prompt 与 step_id 契约

**Files:**
- Modify: `src/product/tester_agent_loop.rs`

- [ ] **Step 1: 写失败测试**

新增测试 `tester_plan_prompt_requires_openspec_superpowers_and_step_bound_tools`。

断言 prompt 包含：

- `plan_tests`
- `execute_test_plan`
- `[openspec_contract]`
- `[superpowers_contract]`
- `Story Spec`
- `Design Spec`
- `Work Item`
- `step_id`
- `不要硬编码某种语言或包管理器`

Run:

```bash
cargo test --locked --lib tester_plan_prompt_requires_openspec_superpowers_and_step_bound_tools
```

Expected: FAIL，缺少 prompt builder。

- [ ] **Step 2: 实现 prompt builder**

新增：

- `build_tester_plan_prompt(attempt, evaluation_context_json)`

要求：

- 明确 Provider 先生成 TestPlan。
- 明确 Aria 是通用项目，不限制 pnpm/cargo/pytest 等生态。
- 明确 provider 根据 Story/Design/WorkItem/diff/project rules 决策。
- 明确 tool call 必须带 `step_id`。
- 明确无 `step_id` 的命令只能进入 `unplanned_commands`。

- [ ] **Step 3: 运行测试**

```bash
cargo test --locked --lib tester_plan_prompt_requires_openspec_superpowers_and_step_bound_tools
```

Expected: PASS。

## Task 2: Testing 运行时改为两段式

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `src/product/tester_agent_loop.rs`

- [ ] **Step 1: 写 step 绑定回归测试**

新增测试 `test_tool_call_without_step_id_is_unplanned_and_does_not_pass_required_step`。

断言：

- TestPlan 有 required step `unit`。
- 执行结果只进入 `unplanned_commands`。
- report 为 `blocked`。
- `missing_required_steps == ["unit"]`。

Run:

```bash
cargo test --locked --lib test_tool_call_without_step_id_is_unplanned_and_does_not_pass_required_step
```

Expected: PASS 或在实现 builder 后 PASS；该测试用于锁定 P1 builder 语义。

- [ ] **Step 2: 接入 plan_tests**

在 `execute_testing_with_provider_commands` 中：

- provider 不支持 tool calls 时，不得声明 plan-based passed；保持 legacy 路径或生成 blocked report。
- provider 支持 tool calls 时，先构建 `EvaluationContextPack`。
- 使用 `build_tester_plan_prompt` 发起 plan run。
- 保存 raw output：purpose=`plan_tests`。
- 调用 `parse_test_plan_payload`。
- 保存 `TestPlan`。

- [ ] **Step 3: 接入 execute_test_plan**

在执行阶段：

- tool call input 有 `step_id` 时，绑定到对应 `TestPlanStep`。
- tool call input 无 `step_id` 时，结果进入 `unplanned_commands`。
- step result 写入 `TestingStepResult`。
- provider completed 后保存 execute raw output。
- 最终调用 `build_plan_based_testing_report`。

- [ ] **Step 4: Testing blocked gate**

当 plan parse 失败、provider 中断、required step 缺失、工具权限不足或 context 冲突时：

- 保存 `TestingReport`。
- `attempt.status = blocked`。
- timeline node status = blocked。
- 创建 blocked gate。

Testing blocked gate actions：

- `retry_test_plan`
- `rerun_missing_steps`
- `provide_context`
- `manual_continue`
- `abort`

## Task 3: Review raw output 与 parser 容错

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`

- [ ] **Step 1: 写失败测试**

新增测试 `review_parser_preserves_findings_with_common_aliases`。

Provider JSON 使用：

- `file` -> `file_path`
- `description` -> `message`
- `recommendation` -> `required_action`
- 缺 `severity`
- 缺 `source_stage`

断言：

- verdict 保留 `request_changes`。
- finding 不丢失。
- severity 默认 `FindingSeverity::Warning`。
- source_stage 默认 `CodingExecutionStage::CodeReview`。

Run:

```bash
cargo test --locked --lib review_parser_preserves_findings_with_common_aliases
```

Expected: FAIL，当前 parser 会因缺 severity 丢失 findings。

- [ ] **Step 2: 实现 parser 容错**

更新 `RawReviewFinding`：

- `severity: Option<FindingSeverity>`
- alias: `file`
- alias: `description`
- alias: `failure_scenario`
- alias: `recommendation`
- alias: `fix`
- 默认 source stage。

扩展 finding：

- `evidence`
- `related_requirements`
- `related_design_constraints`
- `related_work_item_tasks`

- [ ] **Step 3: 保存 raw output**

在 code review 与 internal review 构建报告时：

- 调用 `save_provider_raw_output`。
- 写入 `raw_provider_output_ref`。
- 完全无法解析时仍保存 raw output。

- [ ] **Step 4: Review blocked gate**

当 review verdict 为 `blocked`：

- 保存 report。
- 创建 blocked gate。
- 发送 `CodingGateRequired` WebSocket 消息。
- attempt status 设为 blocked。

Review blocked gate actions：

- `retry_review`
- `send_raw_output_to_analyst`
- `provide_context`
- `manual_continue`
- `abort`

- [ ] **Step 5: 运行测试**

```bash
cargo test --locked --lib review_parser_preserves_findings_with_common_aliases
```

Expected: PASS。

## Task 4: Analyst、Code Reviewer、Internal Reviewer 注入契约

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`

- [ ] **Step 1: 写失败测试**

新增测试 `rework_and_internal_review_prompts_require_openspec_and_superpowers`。

断言：

- Rework prompt 包含 `[openspec_contract]`。
- Rework prompt 包含 `[superpowers_contract]`。
- Rework prompt 包含 `Story Spec`、`Design Spec`、`Work Item`。
- `provider_runtime_contract("InternalReviewer")` 包含 role 和两个 contract。

Run:

```bash
cargo test --locked --lib rework_and_internal_review_prompts_require_openspec_and_superpowers
```

Expected: FAIL，当前 prompt 没有统一契约。

- [ ] **Step 2: 实现统一 contract helper**

新增：

- `provider_runtime_contract(role: &str) -> String`

内容必须包含：

- OpenSpec: 使用 Story/Design/WorkItem 追踪关系。
- OpenSpec: 发现冲突必须 blocked 或请求人工澄清。
- Superpowers: 先证据后结论。
- Superpowers: 验证前置。
- Superpowers: 不用未执行推断替代证据。

- [ ] **Step 3: 注入 EvaluationContextPack**

在以下 prompt 中注入 context JSON：

- Analyst/Rework: `EvaluationContextRole::Analyst`
- Code Reviewer: `EvaluationContextRole::CodeReviewer`
- Internal Reviewer: `EvaluationContextRole::InternalReviewer`

如果 `build_rework_prompt` 是自由函数，则把签名改为接收 `evaluation_context_json: &str`，在调用点构建 context 后传入。

- [ ] **Step 4: 运行测试**

```bash
cargo test --locked --lib rework_and_internal_review_prompts_require_openspec_and_superpowers
```

Expected: PASS。

## Task 5: WebSocket blocked gate 恢复

**Files:**
- Modify: `src/web/coding_ws_handler.rs`
- Modify: `src/product/coding_workspace_engine.rs`

- [ ] **Step 1: 写协议测试**

新增测试 `blocked_attempt_allows_gate_response_messages`。

断言 blocked attempt 允许：

- `GateResponse`
- `AbortAttempt`

Run:

```bash
cargo test --locked --lib blocked_attempt_allows_gate_response_messages
```

Expected: PASS，锁定协议规则。

- [ ] **Step 2: session state 合并 blocked gate**

在 `build_coding_session_state` 中：

- 先读取 open stage gates。
- 再读取 open blocked gates。
- 合并到 `pending_gates`。

- [ ] **Step 3: 处理 `GateResponse`**

在 inbound handler 增加 `CodingWsInMessage::GateResponse` 分支：

- 调用 `engine.handle_blocked_gate_response`。
- 成功后发送最新 session state。
- 失败时发送 `coding_protocol_error`，code=`coding_gate_response_failed`。

- [ ] **Step 4: engine 恢复动作**

新增 `handle_blocked_gate_response`。

动作语义：

- `abort` -> `handle_abort`
- `retry_test_plan` / `rerun_missing_steps` -> status running，stage testing
- `retry_review` -> status running，stage code_review
- `send_raw_output_to_analyst` -> status running，stage rework
- `provide_context` -> 保存 context note，status waiting_for_human
- `manual_continue` / `accept_risk` -> status running

处理完成后调用 `resolve_blocked_gate`。

## 阶段验证

Run:

```bash
cargo fmt --check
cargo test --locked --lib tester_plan_prompt_requires_openspec_superpowers_and_step_bound_tools
cargo test --locked --lib test_tool_call_without_step_id_is_unplanned_and_does_not_pass_required_step
cargo test --locked --lib review_parser_preserves_findings_with_common_aliases
cargo test --locked --lib rework_and_internal_review_prompts_require_openspec_and_superpowers
cargo test --locked --lib blocked_attempt_allows_gate_response_messages
cargo check --locked
```

Expected: 全部 PASS。

## 提交

```bash
git add src/product/tester_agent_loop.rs src/product/coding_workspace_engine.rs src/web/coding_ws_handler.rs
git commit -m "feat: run provider QA workflow with recovery gates"
```
