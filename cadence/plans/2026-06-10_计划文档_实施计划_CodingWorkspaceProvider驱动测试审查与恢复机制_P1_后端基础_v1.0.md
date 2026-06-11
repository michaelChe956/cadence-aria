# CodingWorkspace Provider QA P1 后端基础实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建立 provider-driven testing/review 所需的后端基础模型、上下文包、持久化能力和 TestPlan 纯函数。

**Architecture:** P1 不改完整运行时流程，只新增可序列化模型、`EvaluationContextPack` 构建器、store API、TestPlan parser/report builder。后续 P2 在这些基础上接入工作流。

**Tech Stack:** Rust 1.95、serde/serde_json、chrono、LifecycleStore、CodingAttemptStore、Cargo。

---

## 依赖与边界

- 依赖设计文档：`cadence/designs/2026-06-10_技术方案_CodingWorkspaceProvider驱动测试审查与恢复机制_v1.0.md`
- 本阶段不修改前端。
- 本阶段不改变 Coding Workspace pipeline 行为。
- 新字段必须使用 `#[serde(default)]` 或 `Option<T>` 兼容历史 JSON。

## 文件结构

- Modify: `src/product/mod.rs`
- Create: `src/product/coding_evaluation_context.rs`
- Modify: `src/product/coding_models.rs`
- Modify: `src/product/coding_attempt_store.rs`
- Modify: `src/product/tester_agent_loop.rs`

## Task 1: 扩展 QA 模型

**Files:**
- Modify: `src/product/coding_models.rs`

- [x] **Step 1: 写失败测试**

在 `src/product/coding_models.rs` 新增测试 `test_plan_and_testing_report_round_trip_preserve_step_evidence`，断言：

- `TestPlan.steps[0].tool` 序列化为 `run_command`。
- `TestingReport.steps[0].step_id` 保留。
- `TestingOverallStatus::PassedWithWarnings` 序列化为 `passed_with_warnings`。
- `CodeReviewReport.raw_provider_output_ref` 可序列化。
- `CodingGateRequired.reason_code/evidence_refs/raw_provider_output_ref` 可序列化。

Run:

```bash
cargo test --locked --lib test_plan_and_testing_report_round_trip_preserve_step_evidence
```

Expected: FAIL，缺少新类型或新字段。

- [x] **Step 2: 实现模型**

在 `src/product/coding_models.rs` 新增：

- `TestPlanTool`
- `TestPlanRiskLevel`
- `TestPlanStep`
- `TestPlan`
- `TestingStepResult`

扩展：

- `TestingOverallStatus::PassedWithWarnings`
- `TestingReport.plan_id`
- `TestingReport.plan_summary`
- `TestingReport.steps`
- `TestingReport.unplanned_commands`
- `TestingReport.missing_required_steps`
- `TestingReport.skipped_required_steps`
- `TestingReport.context_warnings`
- `TestingReport.raw_provider_output_ref`
- `ReviewFinding.evidence`
- `ReviewFinding.related_requirements`
- `ReviewFinding.related_design_constraints`
- `ReviewFinding.related_work_item_tasks`
- `CodeReviewReport.raw_provider_output_ref`
- `InternalPrReview.raw_provider_output_ref`
- `CodingGateActionType::RetryTestPlan`
- `CodingGateActionType::RerunMissingSteps`
- `CodingGateActionType::ProvideContext`
- `CodingGateActionType::ManualContinue`
- `CodingGateActionType::RetryReview`
- `CodingGateActionType::SendRawOutputToAnalyst`
- `CodingGateRequired.reason_code`
- `CodingGateRequired.evidence_refs`
- `CodingGateRequired.raw_provider_output_ref`

- [x] **Step 3: 运行测试**

```bash
cargo test --locked --lib test_plan_and_testing_report_round_trip_preserve_step_evidence
```

Expected: PASS。

## Task 2: 新增 EvaluationContextPack

**Files:**
- Modify: `src/product/mod.rs`
- Create: `src/product/coding_evaluation_context.rs`

- [x] **Step 1: 写失败测试**

创建 `src/product/coding_evaluation_context.rs`，新增测试 `evaluation_context_pack_includes_story_design_work_item_and_contracts`。

测试数据：

- 使用 `LifecycleStore` 创建 Story Spec、Design Spec、Work Item。
- 给 Story/Design/WorkItem 各追加一个 artifact version。
- 给 Story/Design/WorkItem 各创建 workspace session，`openspec_enabled=true`、`superpowers_enabled=true`。
- 构造 `CodingExecutionAttempt` 指向 Work Item。

断言：

- `pack.story_spec.raw_markdown_or_sections` 包含 `Acceptance Criteria`。
- `pack.design_spec.raw_markdown_or_sections` 包含 `Security`。
- `pack.work_item.raw_markdown_or_sections` 包含 `验证命令`。
- `pack.openspec_context.enabled == true`。
- `pack.superpowers_context.enabled == true`。
- `required_methods_by_role` 包含 `tester`、`analyst`、`code_reviewer`、`internal_reviewer`。

Run:

```bash
cargo test --locked --lib evaluation_context_pack_includes_story_design_work_item_and_contracts
```

Expected: FAIL，缺少模块或构建函数。

- [x] **Step 2: 实现模块**

在 `src/product/mod.rs` 加入：

```rust
pub mod coding_evaluation_context;
```

在新模块中实现：

- `EvaluationContextRole`
- `EvaluationContextPack`
- `EvaluationSpecContext`
- `EvaluationRepoContext`
- `OpenSpecContext`
- `SuperpowersContext`
- `build_evaluation_context_pack(paths, attempt, provider_role)`

实现要求：

- 从 `LifecycleStore::list_work_items` 找到 attempt 的 Work Item。
- 通过 Work Item 的 `story_spec_ids`、`design_spec_ids` 找到对应 Story/Design。
- 从对应 workspace session 的最新 artifact version 读取 markdown；如果同一个 entity 有多个 workspace session，按 `updated_at` / `created_at` 选择最新 session，并在 `context_warnings` 中记录 `multiple_sessions_for_entity`。
- 如果 Work Item 关联多个 Story/Design，按 `story_spec_ids`、`design_spec_ids` 原顺序全部纳入 context，并为每个 spec 记录 `artifact_id`、`version_id`、`workspace_session_id`。
- 缺 Story/Design 时写入 `context_warnings`，不 panic。
- 缺 Work Item 时写入 `missing_work_item`。
- `required_methods_by_role` 固定包含四个角色。
- markdown、diff、project rules 等长文本必须通过模块内常量裁剪，例如 `MAX_CONTEXT_SECTION_CHARS = 30_000`。
- 敏感行必须脱敏：包含 `api_key`、`token`、`secret`、`password`、`authorization`、`private key` 的行输出为 `[REDACTED]`。

- [x] **Step 3: 运行测试**

```bash
cargo test --locked --lib evaluation_context_pack_includes_story_design_work_item_and_contracts
```

Expected: PASS。

## Task 3: 新增 TestPlan、raw output、blocked gate 持久化

**Files:**
- Modify: `src/product/coding_attempt_store.rs`

- [x] **Step 1: 写失败测试**

在 `src/product/coding_attempt_store.rs` 新增测试 `persists_test_plan_raw_output_and_blocked_gate`。

断言：

- `save_provider_raw_output(attempt_id, Testing, "plan_tests", "...")` 返回 `provider-raw/testing/plan_tests_0001.txt`。
- `save_test_plan` 后 `list_test_plans` 可读到 raw ref。
- `create_blocked_gate` 后 `list_open_blocked_gates` 可读到 `reason_code`、`evidence_refs` 和 actions。
- `resolve_blocked_gate` 后 open blocked gates 为空。

Run:

```bash
cargo test --locked --lib persists_test_plan_raw_output_and_blocked_gate
```

Expected: FAIL，store API 不存在。

- [x] **Step 2: 实现 store API**

新增结构：

- `CreateBlockedGateInput`

新增方法：

- `save_test_plan(&self, plan: &TestPlan)`
- `list_test_plans(project_id, issue_id, attempt_id)`
- `save_provider_raw_output(attempt_id, stage, purpose, output)`
- `create_blocked_gate(input)`
- `list_open_blocked_gates(project_id, issue_id, attempt_id)`
- `resolve_blocked_gate(project_id, issue_id, attempt_id, gate_id)`

落盘目录：

- TestPlan: `<attempt-dir>/test-plans/<plan-id>.json`
- Raw output: `<attempt-dir>/provider-raw/<stage>/<purpose>_NNNN.txt`
- Blocked gate: `<attempt-dir>/blocked-gates/<gate-id>.json`
- Resolved blocked gate: `<attempt-dir>/blocked-gates/resolved/<gate-id>.json`

- [x] **Step 3: 运行测试**

```bash
cargo test --locked --lib persists_test_plan_raw_output_and_blocked_gate
```

Expected: PASS。

## Task 4: TestPlan parser 与 plan-based report builder

**Files:**
- Modify: `src/product/tester_agent_loop.rs`

- [x] **Step 1: 写失败测试**

新增测试 `parses_test_plan_from_provider_json_and_blocks_missing_required_step`。

测试输入：provider 输出一个 fenced JSON TestPlan，包含：

- required step `unit`，tool=`run_command`
- required step `security`，tool=`provider_managed`

只提供 `unit` 的 `TestingStepResult`。

断言：

- parser 能从 markdown fence 中提取 JSON。
- plan 有两个 step。
- report `overall_status == TestingOverallStatus::Blocked`。
- `missing_required_steps == ["security"]`。

Run:

```bash
cargo test --locked --lib parses_test_plan_from_provider_json_and_blocks_missing_required_step
```

Expected: FAIL，parser/report builder 不存在。

- [x] **Step 2: 实现 parser 与 builder**

在 `tester_agent_loop.rs` 实现：

- `parse_test_plan_payload(attempt_id, plan_id, raw_output, raw_provider_output_ref)`
- `build_plan_based_testing_report(report_id, attempt_id, plan, steps, unplanned_commands, provider_claim, raw_provider_output_ref)`

校验规则：

- JSON 必须存在。
- `steps` 非空。
- step id 非空且唯一。
- `title`、`intent`、`evidence_expectation` 非空。
- 所有 required steps 未执行完时 report 为 `blocked`。
- required step `failed/timed_out` 时 report 为 `failed`。
- required 全过但有 context warning 或 optional 失败时 report 为 `passed_with_warnings`。
- required 全过且无 warning 时 report 为 `passed`。

同时更新旧 `build_testing_report`，为新增字段填默认值，保证历史路径编译。

- [x] **Step 3: 运行测试**

```bash
cargo test --locked --lib parses_test_plan_from_provider_json_and_blocks_missing_required_step
```

Expected: PASS。

## Task 5: 历史兼容、上下文安全与 gate 幂等

**Files:**
- Modify: `src/product/coding_models.rs`
- Modify: `src/product/coding_evaluation_context.rs`
- Modify: `src/product/coding_attempt_store.rs`

- [x] **Step 1: 写历史 JSON 兼容失败测试**

在 `src/product/coding_models.rs` 新增测试 `legacy_coding_qa_records_deserialize_with_defaults`，使用旧格式 JSON：

```rust
let legacy_testing_report = r#"{
  "id": "testing_report_0001",
  "attempt_id": "coding_attempt_0001",
  "commands": [],
  "overall_status": "passed",
  "provider_claim": null,
  "backend_verified": true,
  "started_at": "2026-06-10T00:00:00Z",
  "completed_at": "2026-06-10T00:00:01Z"
}"#;

let report: TestingReport = serde_json::from_str(legacy_testing_report).unwrap();
assert_eq!(report.plan_id, None);
assert!(report.steps.is_empty());
assert!(report.missing_required_steps.is_empty());
assert_eq!(report.raw_provider_output_ref, None);
```

同时覆盖旧 `CodeReviewReport`、旧 `InternalPrReview`、旧 `CodingGateRequired`：

- 缺 `raw_provider_output_ref` 时反序列化为 `None`。
- 缺 `reason_code` 时反序列化为 `None`。
- 缺 `evidence_refs` 时反序列化为空数组。
- 缺新增 finding 追踪字段时反序列化为空数组或 `None`。

Run:

```bash
cargo test --locked --lib legacy_coding_qa_records_deserialize_with_defaults
```

Expected: FAIL，新增字段还没有默认值。

- [x] **Step 2: 写上下文裁剪与脱敏失败测试**

在 `src/product/coding_evaluation_context.rs` 新增测试 `evaluation_context_pack_truncates_and_redacts_sensitive_lines`。

测试构造 markdown：

```text
## Acceptance Criteria
normal requirement
api_key = "should-not-leak"
Authorization: Bearer should-not-leak
-----BEGIN PRIVATE KEY-----
should-not-leak
-----END PRIVATE KEY-----
```

断言：

- `raw_markdown_or_sections` 包含 `normal requirement`。
- 不包含 `should-not-leak`。
- 至少包含一次 `[REDACTED]`。
- 超过 `MAX_CONTEXT_SECTION_CHARS` 的内容被裁剪，并在 `context_warnings` 中包含 `context_truncated`。

Run:

```bash
cargo test --locked --lib evaluation_context_pack_truncates_and_redacts_sensitive_lines
```

Expected: FAIL，裁剪和脱敏尚未实现。

- [x] **Step 3: 写 blocked gate 幂等失败测试**

在 `src/product/coding_attempt_store.rs` 新增测试 `blocked_gate_creation_is_idempotent_for_same_node_and_reason`。

断言：

- 第一次 `create_blocked_gate` 创建 gate。
- 第二次用相同 `attempt_id`、`stage`、`node_id`、`reason_code` 创建时返回同一个 `gate_id`。
- `list_open_blocked_gates` 只返回一个 gate。
- 第二次传入的新 `evidence_refs` 被合并去重。

Run:

```bash
cargo test --locked --lib blocked_gate_creation_is_idempotent_for_same_node_and_reason
```

Expected: FAIL，blocked gate 目前还没有幂等语义。

- [x] **Step 4: 实现兼容、安全和幂等约束**

实现要求：

- 所有新增字段使用 `#[serde(default)]` 或 `Option<T>`。
- `EvaluationContextPack` 输出前统一调用 `sanitize_context_text`。
- `sanitize_context_text` 按行脱敏，命中敏感关键字的整行替换为 `[REDACTED]`。
- 私钥块从 `-----BEGIN` 到 `-----END` 整段替换为单行 `[REDACTED_PRIVATE_KEY]`。
- `create_blocked_gate` 先扫描 open gate；同一 `attempt_id`、`stage`、`node_id`、`reason_code` 命中时刷新 metadata 并返回已有 gate。

- [x] **Step 5: 运行补充测试**

```bash
cargo test --locked --lib legacy_coding_qa_records_deserialize_with_defaults
cargo test --locked --lib evaluation_context_pack_truncates_and_redacts_sensitive_lines
cargo test --locked --lib blocked_gate_creation_is_idempotent_for_same_node_and_reason
```

Expected: 全部 PASS。

## 阶段验证

Run:

```bash
cargo fmt --check
cargo test --locked --lib test_plan_and_testing_report_round_trip_preserve_step_evidence
cargo test --locked --lib evaluation_context_pack_includes_story_design_work_item_and_contracts
cargo test --locked --lib persists_test_plan_raw_output_and_blocked_gate
cargo test --locked --lib parses_test_plan_from_provider_json_and_blocks_missing_required_step
cargo test --locked --lib legacy_coding_qa_records_deserialize_with_defaults
cargo test --locked --lib evaluation_context_pack_truncates_and_redacts_sensitive_lines
cargo test --locked --lib blocked_gate_creation_is_idempotent_for_same_node_and_reason
cargo clippy --all-targets --all-features --locked -- -D warnings
```

Expected: 全部 PASS。

## 提交

```bash
git add src/product/mod.rs src/product/coding_evaluation_context.rs src/product/coding_models.rs src/product/coding_attempt_store.rs src/product/tester_agent_loop.rs
git commit -m "feat: add coding QA backend foundation"
```
