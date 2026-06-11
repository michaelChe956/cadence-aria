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
- Modify: `src/web/test_controls.rs`
- Modify: `src/web/app.rs`
- Modify: `src/web/state.rs`

## Task 1: Tester prompt 与 step_id 契约

**Files:**
- Modify: `src/product/tester_agent_loop.rs`

- [x] **Step 1: 写失败测试**

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

- [x] **Step 2: 实现 prompt builder**

新增：

- `build_tester_plan_prompt(attempt, evaluation_context_json)`

要求：

- 明确 Provider 先生成 TestPlan。
- 明确 Aria 是通用项目，不限制 pnpm/cargo/pytest 等生态。
- 明确 provider 根据 Story/Design/WorkItem/diff/project rules 决策。
- 明确 tool call 必须带 `step_id`。
- 明确无 `step_id` 的命令只能进入 `unplanned_commands`。

- [x] **Step 3: 运行测试**

```bash
cargo test --locked --lib tester_plan_prompt_requires_openspec_superpowers_and_step_bound_tools
```

Expected: PASS。

## Task 2: Testing 运行时改为两段式

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `src/product/tester_agent_loop.rs`

- [x] **Step 1: 写 step 绑定回归测试**

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

- [x] **Step 2: 接入 plan_tests**

在 `execute_testing_with_provider_commands` 中：

- provider 不支持 tool calls 时，不得声明 plan-based passed；保持 legacy 路径或生成 blocked report。
- provider 支持 tool calls 时，先构建 `EvaluationContextPack`。
- 使用 `build_tester_plan_prompt` 发起 plan run。
- 保存 raw output：purpose=`plan_tests`。
- 调用 `parse_test_plan_payload`。
- 保存 `TestPlan`。

- [x] **Step 3: 接入 execute_test_plan**

在执行阶段：

- tool call input 有 `step_id` 时，绑定到对应 `TestPlanStep`。
- tool call input 无 `step_id` 时，结果进入 `unplanned_commands`。
- step result 写入 `TestingStepResult`。
- provider completed 后保存 execute raw output。
- 最终调用 `build_plan_based_testing_report`。
- 所有 Tester tool call input 都允许携带 `step_id`：
  - `run_command` 产生 `TestingStepResult`。
  - `read_file`、`list_files`、`search_code` 产生 step evidence，可记录到 `TestingStepResult.evidence_refs` 或 `provider_analysis`。
  - 没有 `step_id` 的任何 tool result 都只能进入 `unplanned_commands` 或 `unplanned_evidence`，不得满足 required step。
- `step_id` 不存在于 TestPlan 时，该 tool result 进入 `unplanned_evidence`，并在 report `context_warnings` 中记录 `unknown_step_id:<step_id>`。

- [x] **Step 4: Testing blocked gate**

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

`manual_continue` 必须额外写入质量门禁绕过审计记录，至少包含：

- `gate_id`
- `attempt_id`
- `stage`
- `reason_code`
- `skipped_required_steps`
- `operator_context`
- `created_at`

该审计记录必须注入后续 Code Reviewer 与 Internal Reviewer 的 `EvaluationContextPack`，让 reviewer 明确知道哪些验证被人工跳过。

## Task 3: Review raw output 与 parser 容错

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`

- [x] **Step 1: 写失败测试**

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

- [x] **Step 2: 实现 parser 容错**

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

- [x] **Step 3: 保存 raw output**

在 code review 与 internal review 构建报告时：

- 调用 `save_provider_raw_output`。
- 写入 `raw_provider_output_ref`。
- 完全无法解析时仍保存 raw output。

- [x] **Step 4: Review blocked gate**

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

- [x] **Step 5: 运行测试**

```bash
cargo test --locked --lib review_parser_preserves_findings_with_common_aliases
```

Expected: PASS。

## Task 4: Analyst、Code Reviewer、Internal Reviewer 注入契约

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`

- [x] **Step 1: 写失败测试**

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

- [x] **Step 2: 实现统一 contract helper**

新增：

- `provider_runtime_contract(role: &str) -> String`

内容必须包含：

- OpenSpec: 使用 Story/Design/WorkItem 追踪关系。
- OpenSpec: 发现冲突必须 blocked 或请求人工澄清。
- Superpowers: 先证据后结论。
- Superpowers: 验证前置。
- Superpowers: 不用未执行推断替代证据。

- [x] **Step 3: 注入 EvaluationContextPack**

在以下 prompt 中注入 context JSON：

- Analyst/Rework: `EvaluationContextRole::Analyst`
- Code Reviewer: `EvaluationContextRole::CodeReviewer`
- Internal Reviewer: `EvaluationContextRole::InternalReviewer`

如果 `build_rework_prompt` 是自由函数，则把签名改为接收 `evaluation_context_json: &str`，在调用点构建 context 后传入。

- [x] **Step 4: 运行测试**

```bash
cargo test --locked --lib rework_and_internal_review_prompts_require_openspec_and_superpowers
```

Expected: PASS。

## Task 5: WebSocket blocked gate 恢复

**Files:**
- Modify: `src/web/coding_ws_handler.rs`
- Modify: `src/product/coding_workspace_engine.rs`

- [x] **Step 1: 写协议测试**

新增测试 `blocked_attempt_allows_gate_response_messages`。

断言 blocked attempt 允许：

- `GateResponse`
- `AbortAttempt`

Run:

```bash
cargo test --locked --lib blocked_attempt_allows_gate_response_messages
```

Expected: PASS，锁定协议规则。

- [x] **Step 2: session state 合并 blocked gate**

在 `build_coding_session_state` 中：

- 先读取 open stage gates。
- 再读取 open blocked gates。
- 合并到 `pending_gates`。

- [x] **Step 3: 处理 `GateResponse`**

在 inbound handler 增加 `CodingWsInMessage::GateResponse` 分支：

- 调用 `engine.handle_blocked_gate_response`。
- 成功后发送最新 session state。
- 失败时发送 `coding_protocol_error`，code=`coding_gate_response_failed`。

- [x] **Step 4: engine 恢复动作**

新增 `handle_blocked_gate_response`。

动作语义：

- `abort` -> `handle_abort`
- `retry_test_plan` / `rerun_missing_steps` -> status running，stage testing
- `retry_review` -> status running，stage code_review
- `send_raw_output_to_analyst` -> status running，stage rework
- `provide_context` -> 保存 context note，status waiting_for_human
- `manual_continue` / `accept_risk` -> 保存质量门禁绕过审计记录后 status running

处理完成后调用 `resolve_blocked_gate`。

## Task 6: 并发、审计与安全边界回归

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `src/product/tester_agent_loop.rs`
- Modify: `src/web/coding_ws_handler.rs`

- [x] **Step 1: 写 step evidence 归属测试**

新增测试 `tester_tool_results_without_step_id_remain_unplanned_evidence`。

测试数据：

- TestPlan 有 required step `unit`。
- Provider 调用 `read_file`、`search_code`、`run_command`，但都没有 `step_id`。
- 三个 tool result 都成功。

断言：

- `report.overall_status == TestingOverallStatus::Blocked`。
- `missing_required_steps == ["unit"]`。
- `unplanned_commands` 或 `unplanned_evidence` 包含三个工具结果。
- required step `unit` 没有被标记为 passed。

Run:

```bash
cargo test --locked --lib tester_tool_results_without_step_id_remain_unplanned_evidence
```

Expected: FAIL，当前实现还没有 unplanned evidence 归属。

- [x] **Step 2: 写 blocked gate 重连幂等测试**

新增测试 `blocked_gate_response_is_idempotent_across_reconnects`。

断言：

- blocked attempt reconnect 后 `build_coding_session_state` 返回同一个 open blocked gate。
- 连续两次发送同一 `gate_response`，第一次 resolve gate，第二次返回可解释的 `coding_gate_already_resolved` 或 no-op success。
- 不会重复启动两个 runner，不会创建重复 timeline node。

Run:

```bash
cargo test --locked --lib blocked_gate_response_is_idempotent_across_reconnects
```

Expected: FAIL，当前恢复动作还没有幂等保护。

- [x] **Step 3: 写 manual_continue 审计测试**

新增测试 `manual_continue_persists_quality_bypass_audit_and_injects_reviewer_context`。

断言：

- 对 Testing blocked gate 执行 `manual_continue` 时必须要求 `extra_context` 非空。
- store 落盘一条 bypass audit。
- 后续 `build_code_review_prompt` 或 `EvaluationContextPack` 包含 skipped required step 和人工原因。

Run:

```bash
cargo test --locked --lib manual_continue_persists_quality_bypass_audit_and_injects_reviewer_context
```

Expected: FAIL，当前尚未记录质量门禁绕过审计。

- [x] **Step 4: 写高风险 TestPlan step 安全测试**

新增测试 `dangerous_test_plan_step_requires_permission_or_blocks`。

测试 TestPlan step：

```json
{
  "id": "destructive",
  "title": "destructive command",
  "intent": "should require approval",
  "required": true,
  "tool": "run_command",
  "risk_level": "high",
  "command_or_tool_input": {"command": ["rm", "-rf", "/tmp/some-target"]},
  "evidence_expectation": "must not run without approval"
}
```

断言：

- 在没有 permission approval 的情况下不执行命令。
- report 为 `blocked`。
- blocked gate `reason_code == "high_risk_test_step_requires_permission"`。

Run:

```bash
cargo test --locked --lib dangerous_test_plan_step_requires_permission_or_blocks
```

Expected: FAIL，当前没有 TestPlan risk gate。

- [x] **Step 5: 实现并发、审计和安全边界**

实现要求：

- `handle_blocked_gate_response` 对已 resolved gate 做幂等处理。
- runner 启动前检查当前 attempt 是否已经因同一 gate action 进入 running，避免重复 runner。
- manual continue / accept risk 必须保存 audit，并要求 `extra_context` 非空。
- `risk_level=high` 的 required step 在没有 permission approval 时创建 blocked gate，而不是直接执行。
- 所有 gate response 失败必须返回稳定错误码：`coding_gate_not_found`、`coding_gate_already_resolved`、`coding_gate_action_not_allowed`、`coding_gate_response_failed`。

- [x] **Step 6: 运行补充测试**

```bash
cargo test --locked --lib tester_tool_results_without_step_id_remain_unplanned_evidence
cargo test --locked --lib blocked_gate_response_is_idempotent_across_reconnects
cargo test --locked --lib manual_continue_persists_quality_bypass_audit_and_injects_reviewer_context
cargo test --locked --lib dangerous_test_plan_step_requires_permission_or_blocks
```

Expected: 全部 PASS。

## Task 7: Deterministic QA controlled fixtures

**Files:**
- Modify: `src/web/test_controls.rs`
- Modify: `src/web/app.rs`
- Modify: `src/web/state.rs`
- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `src/product/tester_agent_loop.rs`

- [x] **Step 1: 写 Testing fixture provider 测试**

新增测试 `testing_fixture_fake_provider_emits_plan_and_step_results`。

测试 fixture：

```json
{
  "plan_output": {
    "summary": "controlled QA plan",
    "steps": [
      {
        "id": "unit",
        "title": "Unit tests",
        "intent": "prove unit behavior",
        "required": true,
        "tool": "run_command",
        "risk_level": "low",
        "command_or_tool_input": {"command": ["true"]},
        "evidence_expectation": "exit 0"
      },
      {
        "id": "security",
        "title": "Security check",
        "intent": "prove security checklist",
        "required": true,
        "tool": "provider_managed",
        "risk_level": "medium",
        "command_or_tool_input": {"note": "controlled missing step"},
        "evidence_expectation": "provider evidence"
      }
    ]
  },
  "step_results": [
    {"step_id": "unit", "status": "passed"}
  ]
}
```

断言：

- TestControlledFakeStreamingProvider 在 tester role 下先输出 TestPlan。
- execute 阶段只返回 `unit` 的 step result。
- 最终 report 为 blocked，`missing_required_steps == ["security"]`。

Run:

```bash
cargo test --locked --lib testing_fixture_fake_provider_emits_plan_and_step_results
```

Expected: FAIL，当前 test controls 没有 Testing/TestPlan fixture。

- [x] **Step 2: 写 Review fixture alias/malformed 测试**

新增测试 `review_fixture_can_emit_alias_findings_and_malformed_json`。

断言：

- review fixture 支持直接配置 raw JSON。
- raw JSON 可以包含 `file`、`description`、`recommendation` 等 alias 字段。
- malformed 模式能输出非 JSON 文本，用于触发 review blocked gate。

Run:

```bash
cargo test --locked --lib review_fixture_can_emit_alias_findings_and_malformed_json
```

Expected: FAIL，当前 review fixture 只输出 `verdict` 和 `summary`。

- [x] **Step 3: 实现 test controls API**

在 `ARIA_E2E_TEST_CONTROLS=1` 时新增或扩展路由：

```text
POST /api/test/coding-attempts/{attempt_id}/testing-fixture
POST /api/test/coding-attempts/{attempt_id}/review-fixture
```

实现要求：

- fixture 以 `attempt_id` 作为 `workspace_session_id` key，因为 Coding Workspace provider input 使用 attempt id 作为 session id。
- Testing fixture 支持 `plan_output`、`step_results`、`malformed_plan_output`、`provider_failure`。
- Review fixture 支持 `raw_json`、`raw_text`、`verdict`、`summary`、`findings`。
- fixture 被 consume 一次后自动删除，避免污染后续真实 E2E。
- 未设置 fixture 时保持现有 fake provider fallback 行为。

- [x] **Step 4: 运行 fixture 测试**

```bash
cargo test --locked --lib testing_fixture_fake_provider_emits_plan_and_step_results
cargo test --locked --lib review_fixture_can_emit_alias_findings_and_malformed_json
```

Expected: 全部 PASS。

## 阶段验证

Run:

```bash
cargo fmt --check
cargo test --locked --lib tester_plan_prompt_requires_openspec_superpowers_and_step_bound_tools
cargo test --locked --lib test_tool_call_without_step_id_is_unplanned_and_does_not_pass_required_step
cargo test --locked --lib review_parser_preserves_findings_with_common_aliases
cargo test --locked --lib rework_and_internal_review_prompts_require_openspec_and_superpowers
cargo test --locked --lib blocked_attempt_allows_gate_response_messages
cargo test --locked --lib tester_tool_results_without_step_id_remain_unplanned_evidence
cargo test --locked --lib blocked_gate_response_is_idempotent_across_reconnects
cargo test --locked --lib manual_continue_persists_quality_bypass_audit_and_injects_reviewer_context
cargo test --locked --lib dangerous_test_plan_step_requires_permission_or_blocks
cargo test --locked --lib testing_fixture_fake_provider_emits_plan_and_step_results
cargo test --locked --lib review_fixture_can_emit_alias_findings_and_malformed_json
cargo check --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
```

Expected: 全部 PASS。

## 提交

```bash
git add src/product/tester_agent_loop.rs src/product/coding_workspace_engine.rs src/web/coding_ws_handler.rs src/web/test_controls.rs src/web/app.rs src/web/state.rs
git commit -m "feat: run provider QA workflow with recovery gates"
```
