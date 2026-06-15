# Coding Workspace Reviewer 重跑闭环实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 P1/P2 基础上，让 Code Reviewer 与 Internal Reviewer 都拥有可追踪的 role run，并且 reviewer 失败或 blocked 后可以回到执行 reviewer 前的状态重新执行。

**Architecture:** Code Reviewer 绑定 `CodingExecutionStage::CodeReview + CodingProviderRole::CodeReviewer`，Internal Reviewer 绑定 `CodingExecutionStage::InternalPrReview + CodingProviderRole::InternalReviewer`。两个 reviewer 复用 P2 的 role run refs helper；`retry_review` gate response 根据 gate stage/role 分流到 `RetryReview` 或 `RetryInternalReview`，避免 Internal Reviewer 重跑时误回 CodeReview。Reviewer chat 使用后端生成的可读 summary，不把 JSON contract 直接暴露给用户。

**Tech Stack:** Rust 1.95、serde JSON store、Tokio async、Axum WebSocket、React、Zustand、Vitest。

---

## 当前基线

本计划基于 `bugfix_branch` 当前提交 `1bad0be` 加 P2 计划完成后的预期代码。

当前已存在能力：

- `CodingRoleRunTrigger` 已包含 `RetryReview` 和 `RetryInternalReview`。
- Code Reviewer 与 Internal Reviewer 已有 provider prompt、raw output 保存、report 持久化、WebSocket complete event 和 chat entry。
- `retry_review` gate action 已存在。
- 当前 `handle_blocked_gate_response` 对 `RetryReview` 固定恢复到 `CodingExecutionStage::CodeReview`。

当前缺口：

- `CodeReviewReport` 与 `InternalPrReview` 没有 `role_run_id/run_no`。
- Code Reviewer/Internal Reviewer 执行入口不创建、不完成、不 blocked role run。
- Reviewer report raw refs 没有挂到 role run。
- Reviewer chat metadata 没有 `role_run_id/run_no`，前端无法把消息和 run 历史关联。
- `retry_review` 对 Internal Reviewer blocked gate 没有分流，理论上会把 Internal Reviewer 重跑错误地拉回 CodeReview。
- Reviewer chat 只显示 summary，缺少 verdict/findings/impact scope 的可读结构。

## Design Readiness Review

当前 design 符合实施落地条件。

- P2 完成后 store 已有 `update_role_run_refs`，Reviewer 只需复用，不需要再设计新 store 语义。
- Code Reviewer 与 Internal Reviewer 的执行入口相互独立，适合在一个 session 内做平行但受控的绑定。
- `retry_review` 仍保持同一个前端按钮，后端按 gate stage/role 做分流，不扩大 UI contract。
- 现有 `tests/it_web/web_coding_ws_handler.rs` 已有 CodeReview/InternalReview blocked 进入 Analyst 的集成上下文，可补 focused retry 分流测试。

不做范围：

- 不实现 Analyst evidence retry，P2 已覆盖。
- 不做历史 run UI，P4 处理。
- 不改变 reviewer provider JSON schema。
- 不改 Story/Design/Work Item workspace 链路；本计划仅影响 Coding Workspace。

## File Structure

- Modify: `src/product/coding_models.rs`
  - `CodeReviewReport` 增加可选 `role_run_id/run_no`。
  - `InternalPrReview` 增加可选 `role_run_id/run_no`。

- Modify: `src/product/coding_workspace_engine.rs`
  - Code Reviewer 执行入口创建/复用 role run。
  - Internal Reviewer 执行入口创建/复用 role run。
  - report、chat entry、role run refs/status 绑定。
  - `handle_blocked_gate_response` 的 `RetryReview` 按 gate stage/role 分流。
  - 新增 reviewer 可读 chat summary formatter。

- Modify: `src/web/coding_ws_handler.rs`
  - 如果 P2 已将 direct stage recovery 抽成 helper，本任务复用。
  - 确认 `retry_review` resume 后从当前 stage 继续执行对应 reviewer。

- Modify: `web/src/api/types.ts`
  - `CodeReviewReport` 与 `InternalPrReview` 增加 `role_run_id/run_no`。

- Tests:
  - `tests/it_product/product_coding_models.rs`
  - `tests/it_product/product_coding_workspace_engine.rs`
  - `tests/it_web/web_coding_ws_handler.rs`
  - `web/src/api/types.test.ts`
  - `web/src/state/coding-workspace-store.test.ts`
  - `web/src/components/chat-workspace/MessageGroupView.test.tsx`
  - `web/src/pages/CodingWorkspacePage.test.tsx`

## Task 1: Extend Review Report Contracts

**Files:**
- Modify: `src/product/coding_models.rs`
- Modify: `web/src/api/types.ts`
- Test: `tests/it_product/product_coding_models.rs`
- Test: `web/src/api/types.test.ts`

- [ ] **Step 1: RED - backend report round trip**

Add to `tests/it_product/product_coding_models.rs`:

```rust
#[test]
fn review_reports_round_trip_role_run_metadata() {
    let code_review = CodeReviewReport {
        id: "code_review_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        round: 1,
        verdict: ReviewVerdict::Approve,
        findings: Vec::new(),
        tested_evidence_refs: Vec::new(),
        diff_refs: Vec::new(),
        summary: "review ok".to_string(),
        created_at: "2026-06-13T00:00:00Z".to_string(),
        raw_provider_output_ref: Some("provider-raw/code_review/code_review_0001.txt".to_string()),
        role_run_id: Some("coding_role_run_0001".to_string()),
        run_no: Some(1),
    };
    let value = serde_json::to_value(&code_review).expect("serialize code review");
    assert_eq!(value["role_run_id"], "coding_role_run_0001");
    let decoded: CodeReviewReport = serde_json::from_value(value).expect("decode code review");
    assert_eq!(decoded.role_run_id.as_deref(), Some("coding_role_run_0001"));
    assert_eq!(decoded.run_no, Some(1));

    let internal_review = InternalPrReview {
        id: "internal_review_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        review_request_id: "review_request_0001".to_string(),
        verdict: ReviewVerdict::Approve,
        findings: Vec::new(),
        impact_scope: vec!["src/lib.rs".to_string()],
        pr_description: "PR".to_string(),
        commit_message_suggestion: "feat: work".to_string(),
        tested_evidence_refs: Vec::new(),
        diff_refs: Vec::new(),
        summary: "internal ok".to_string(),
        created_at: "2026-06-13T00:00:01Z".to_string(),
        raw_provider_output_ref: Some("provider-raw/internal_pr_review/internal_pr_review_0001.txt".to_string()),
        role_run_id: Some("coding_role_run_0002".to_string()),
        run_no: Some(1),
    };
    let value = serde_json::to_value(&internal_review).expect("serialize internal review");
    assert_eq!(value["role_run_id"], "coding_role_run_0002");
    let decoded: InternalPrReview = serde_json::from_value(value).expect("decode internal review");
    assert_eq!(decoded.role_run_id.as_deref(), Some("coding_role_run_0002"));
    assert_eq!(decoded.run_no, Some(1));
}
```

Run:

```bash
cargo test --locked --test it_product review_reports_round_trip_role_run_metadata
```

Expected: compile failure because report fields are missing.

- [ ] **Step 2: GREEN - add optional fields**

In `src/product/coding_models.rs`, add to both `CodeReviewReport` and `InternalPrReview`:

```rust
#[serde(default)]
pub role_run_id: Option<String>,
#[serde(default)]
pub run_no: Option<u32>,
```

Update all Rust fixtures constructing these structs with:

```rust
role_run_id: None,
run_no: None,
```

In `web/src/api/types.ts`, add to both frontend report types:

```ts
role_run_id?: string | null;
run_no?: number | null;
```

- [ ] **Step 3: frontend type test**

Add to `web/src/api/types.test.ts`:

```ts
it("accepts role run metadata on review reports", () => {
  const report: CodeReviewReport = {
    id: "code_review_0001",
    attempt_id: "coding_attempt_0001",
    round: 1,
    verdict: "approve",
    findings: [],
    tested_evidence_refs: [],
    diff_refs: [],
    summary: "review ok",
    created_at: "2026-06-13T00:00:00Z",
    raw_provider_output_ref: "provider-raw/code_review/code_review_0001.txt",
    role_run_id: "coding_role_run_0001",
    run_no: 1,
  };

  const internal: InternalPrReview = {
    id: "internal_review_0001",
    attempt_id: "coding_attempt_0001",
    review_request_id: "review_request_0001",
    verdict: "approve",
    findings: [],
    impact_scope: ["src/lib.rs"],
    pr_description: "PR",
    commit_message_suggestion: "feat: work",
    tested_evidence_refs: [],
    diff_refs: [],
    summary: "internal ok",
    created_at: "2026-06-13T00:00:01Z",
    raw_provider_output_ref: "provider-raw/internal_pr_review/internal_pr_review_0001.txt",
    role_run_id: "coding_role_run_0002",
    run_no: 1,
  };

  expect(report.run_no).toBe(1);
  expect(internal.role_run_id).toBe("coding_role_run_0002");
});
```

- [ ] **Step 4: Run focused tests**

```bash
cargo test --locked --test it_product review_reports_round_trip_role_run_metadata
pnpm -C web exec vitest --run src/api/types.test.ts
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src/product/coding_models.rs web/src/api/types.ts tests/it_product/product_coding_models.rs web/src/api/types.test.ts
git commit -m "feat: add reviewer role run metadata"
```

## Task 2: Bind Code Reviewer RoleRun

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`
- Test: `tests/it_product/product_coding_workspace_engine.rs`

- [ ] **Step 1: RED - Code Reviewer binding test**

Add `execute_code_review_binds_report_chat_and_status_to_role_run` to `tests/it_product/product_coding_workspace_engine.rs`. The test must:

- create a worktree-backed attempt;
- call `execute_code_review` with provider output:

```json
{"verdict":"approve","summary":"review ok","findings":[]}
```

- assert one role run exists with `stage == CodeReview`, `role == CodeReviewer`, `status == Completed`, `run_no == 1`;
- assert `report.role_run_id == Some(run.id)` and `report.run_no == Some(1)`;
- assert `run.raw_provider_output_refs` contains `code_review`;
- assert saved chat entry metadata contains `source = "code_review"`, `role_run_id`, and `run_no`.

Run:

```bash
cargo test --locked --test it_product execute_code_review_binds_report_chat_and_status_to_role_run
```

Expected: FAIL because Code Reviewer does not create role run.

- [ ] **Step 2: GREEN - implement Code Reviewer binding**

In `execute_code_review_with_commands`, immediately after timeline node creation:

```rust
let role_run = match self.store.latest_role_run(
    &attempt.project_id,
    &attempt.issue_id,
    &attempt.id,
    CodingExecutionStage::CodeReview,
    CodingProviderRole::CodeReviewer,
)? {
    Some(run) if run.status == CodingRoleRunStatus::Running && run.node_id.is_none() => {
        self.store.attach_role_run_node(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &run.id,
            node.id.clone(),
        )?
    }
    _ => self.store.create_role_run(
        &attempt,
        CodingExecutionStage::CodeReview,
        CodingProviderRole::CodeReviewer,
        CodingRoleRunTrigger::Initial,
        Some(node.id.clone()),
    )?,
};
```

After raw output save:

```rust
self.store.update_role_run_refs(
    &attempt.project_id,
    &attempt.issue_id,
    &attempt.id,
    &role_run.id,
    vec![raw_provider_output_ref.clone()],
    Vec::new(),
)?;
```

Set report fields after `build_code_review_report` returns and before saving:

```rust
let mut report = self.build_code_review_report(
    &attempt,
    &full_output,
    Some(raw_provider_output_ref.clone()),
)?;
report.role_run_id = Some(role_run.id.clone());
report.run_no = Some(role_run.run_no);
self.store.save_code_review_report(&report)?;
```

After node status is derived, call:

```rust
self.store.update_role_run_status(
    &attempt.project_id,
    &attempt.issue_id,
    &attempt.id,
    &role_run.id,
    review_role_run_status(&report.verdict),
    review_role_run_reason(&report.verdict),
)?;
```

Add helpers:

```rust
fn review_role_run_status(verdict: &ReviewVerdict) -> CodingRoleRunStatus {
    match verdict {
        ReviewVerdict::Approve => CodingRoleRunStatus::Completed,
        ReviewVerdict::RequestChanges => CodingRoleRunStatus::Completed,
        ReviewVerdict::Blocked => CodingRoleRunStatus::Blocked,
    }
}

fn review_role_run_reason(verdict: &ReviewVerdict) -> Option<String> {
    match verdict {
        ReviewVerdict::Blocked => Some("review_blocked".to_string()),
        _ => None,
    }
}
```

- [ ] **Step 3: Run focused test**

```bash
cargo test --locked --test it_product execute_code_review_binds_report_chat_and_status_to_role_run
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/product/coding_workspace_engine.rs tests/it_product/product_coding_workspace_engine.rs
git commit -m "feat: bind code reviewer role runs"
```

## Task 3: Bind Internal Reviewer RoleRun

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`
- Test: `tests/it_product/product_coding_workspace_engine.rs`

- [ ] **Step 1: RED - Internal Reviewer binding test**

Add `execute_internal_pr_review_binds_review_chat_and_status_to_role_run` to `tests/it_product/product_coding_workspace_engine.rs`. The test must:

- create worktree-backed attempt and a saved `ReviewRequest`;
- call `execute_internal_pr_review` with provider output:

```json
{"verdict":"approve","summary":"internal ok","findings":[],"impact_scope":["src/lib.rs"],"pr_description":"PR body","commit_message_suggestion":"feat: work"}
```

- assert one role run exists with `stage == InternalPrReview`, `role == InternalReviewer`, `status == Completed`, `run_no == 1`;
- assert review `role_run_id/run_no`;
- assert raw refs contain `internal_pr_review`;
- assert chat metadata contains `source = "internal_pr_review"`, `role_run_id`, `run_no`, and `impact_scope`.

Run:

```bash
cargo test --locked --test it_product execute_internal_pr_review_binds_review_chat_and_status_to_role_run
```

Expected: FAIL because Internal Reviewer does not create role run.

- [ ] **Step 2: GREEN - implement Internal Reviewer binding**

Mirror Task 2 in `execute_internal_pr_review_with_commands`, using:

```rust
CodingExecutionStage::InternalPrReview
CodingProviderRole::InternalReviewer
CodingRoleRunTrigger::Initial
```

Set `review.role_run_id/run_no`, append `raw_provider_output_ref` to role run refs, update role run status with the same `review_role_run_status` helper, and add `role_run_id/run_no` to `emit_internal_pr_review_chat_entry`.

- [ ] **Step 3: Run focused test**

```bash
cargo test --locked --test it_product execute_internal_pr_review_binds_review_chat_and_status_to_role_run
```

Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add src/product/coding_workspace_engine.rs tests/it_product/product_coding_workspace_engine.rs
git commit -m "feat: bind internal reviewer role runs"
```

## Task 4: Split RetryReview By Gate Stage

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `src/web/coding_ws_handler.rs`
- Test: `tests/it_product/product_coding_workspace_engine.rs`
- Test: `tests/it_web/web_coding_ws_handler.rs`

- [ ] **Step 1: RED - product retry branch test**

Add `retry_review_gate_response_uses_gate_stage_for_role_run_trigger` to `tests/it_product/product_coding_workspace_engine.rs`. The test should create two cases:

1. Gate stage `CodeReview`, role `CodeReviewer`, action `retry_review`.
   - Assert updated attempt stage is `CodeReview`.
   - Assert new role run trigger is `RetryReview`.
   - Assert previous CodeReviewer run is `Superseded`.

2. Gate stage `InternalPrReview`, role `InternalReviewer`, action `retry_review`.
   - Assert updated attempt stage is `InternalPrReview`.
   - Assert new role run trigger is `RetryInternalReview`.
   - Assert previous InternalReviewer run is `Superseded`.

Run:

```bash
cargo test --locked --test it_product retry_review_gate_response_uses_gate_stage_for_role_run_trigger
```

Expected: FAIL because current code always resumes `CodeReview`.

- [ ] **Step 2: GREEN - implement stage/role split**

Replace the current `CodingGateActionType::RetryReview` branch with:

```rust
CodingGateActionType::RetryReview => {
    let (stage, role, trigger) = match (gate.stage.clone(), gate.role.clone()) {
        (Some(CodingExecutionStage::InternalPrReview), Some(CodingProviderRole::InternalReviewer)) => (
            CodingExecutionStage::InternalPrReview,
            CodingProviderRole::InternalReviewer,
            CodingRoleRunTrigger::RetryInternalReview,
        ),
        _ => (
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::RetryReview,
        ),
    };
    let resumed = self.resume_blocked_attempt_at_stage(&current, stage.clone())?;
    self.store.supersede_latest_role_run_and_create(
        &resumed,
        stage,
        role,
        trigger,
        None,
        gate.reason_code.clone(),
    )?;
    resumed
}
```

- [ ] **Step 3: RED - WS internal reviewer retry does not enter CodeReview**

Add `coding_ws_retry_review_from_internal_pr_review_resumes_internal_reviewer` to `tests/it_web/web_coding_ws_handler.rs`. The fixture must:

- create blocked attempt at `InternalPrReview`;
- create one blocked InternalReviewer role run;
- create blocked gate with `stage = InternalPrReview`, `role = InternalReviewer`, action `retry_review`;
- send gate response over `/ws/coding-attempts/coding_attempt_0001`;
- assert the next timeline node stage is `InternalPrReview`;
- assert no new `CodeReview` timeline node appears before the InternalReviewer provider runs;
- assert role runs include run #2 with `RetryInternalReview`.

Run:

```bash
cargo test --locked --test it_web coding_ws_retry_review_from_internal_pr_review_resumes_internal_reviewer
```

Expected: FAIL before the runner resume path is aligned.

- [ ] **Step 4: GREEN - direct InternalPrReview recovery**

`execute_start_coding_flow` 当前从 `Coding` 开始顺序执行，不会在 `InternalPrReview` 阶段直接恢复。需要在 P2 引入的 stage-based 入口之后、 `CodeReview` 分支之前增加 `InternalPrReview` 直接恢复分支：

1. 如果 `current.stage == CodingExecutionStage::InternalPrReview`：
   - 调用 `await_stage_gate(..., CodingExecutionStage::InternalPrReview)`；
   - 获取 Internal Reviewer provider；
   - 调用 `execute_internal_pr_review_with_commands`；
   - 之后进入 `await_stage_gate(..., Rework)` 并执行 Analyst；
   - 根据 Analyst 结果继续 `Coding`、`Testing`、`CodeReview`、`ReviewRequest`、`InternalPrReview` 或结束。
2. 如果 `current.stage` 不是 `InternalPrReview`，保持现有 pipeline 不变。

该分支与 CodeReview 分支类似，只是从 `InternalPrReview` 阶段开始，避免 `retry_review` 在 InternalPrReview gate 下被错误地拉回 CodeReview。

- [ ] **Step 5: Run focused tests**

```bash
cargo test --locked --test it_product retry_review_gate_response_uses_gate_stage_for_role_run_trigger
cargo test --locked --test it_web coding_ws_retry_review_from_internal_pr_review_resumes_internal_reviewer
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/product/coding_workspace_engine.rs src/web/coding_ws_handler.rs tests/it_product/product_coding_workspace_engine.rs tests/it_web/web_coding_ws_handler.rs
git commit -m "feat: retry reviewer by gate stage"
```

## Task 5: Reviewer Readable Chat Summary

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `web/src/components/chat-workspace/MessageGroupView.tsx`
- Test: `tests/it_product/product_coding_workspace_engine.rs`
- Test: `web/src/components/chat-workspace/MessageGroupView.test.tsx`
- Test: `web/src/pages/CodingWorkspacePage.test.tsx`

- [ ] **Step 1: RED - backend readable summary test**

Add a product test that runs Code Reviewer with request-changes output and asserts saved chat content contains:

```text
Verdict: request_changes
Summary: needs validation
Findings:
- warning src/lib.rs:42 missing validation
```

Run:

```bash
cargo test --locked --test it_product code_reviewer_chat_entry_contains_readable_summary
```

Expected: FAIL because current content is summary only.

- [ ] **Step 2: GREEN - implement formatter**

In `src/product/coding_workspace_engine.rs`, add:

```rust
fn review_verdict_label(verdict: &ReviewVerdict) -> &'static str {
    match verdict {
        ReviewVerdict::Approve => "approve",
        ReviewVerdict::RequestChanges => "request_changes",
        ReviewVerdict::Blocked => "blocked",
    }
}

fn finding_severity_label(severity: &FindingSeverity) -> &'static str {
    match severity {
        FindingSeverity::Error => "error",
        FindingSeverity::Warning => "warning",
        FindingSeverity::Info => "info",
    }
}

fn format_code_review_chat_summary(report: &CodeReviewReport) -> String {
    let mut lines = vec![
        format!("Verdict: {}", review_verdict_label(&report.verdict)),
        format!("Summary: {}", report.summary),
    ];
    if !report.findings.is_empty() {
        lines.push("Findings:".to_string());
        for finding in &report.findings {
            let location = match (&finding.file_path, finding.line) {
                (Some(path), Some(line)) => format!("{path}:{line}"),
                (Some(path), None) => path.clone(),
                _ => "-".to_string(),
            };
            lines.push(format!(
                "- {} {} {}",
                finding_severity_label(&finding.severity),
                location,
                finding.message
            ));
        }
    }
    lines.join("\n")
}
```

Add an equivalent `format_internal_pr_review_chat_summary` that reuses `review_verdict_label` and `finding_severity_label`, and includes `Impact Scope`, `PR Description`, and `Commit Message`.

Use these formatters in `emit_code_review_chat_entry` and `emit_internal_pr_review_chat_entry`.

- [ ] **Step 3: frontend group title shows run number when metadata exists**

Add to `web/src/components/chat-workspace/MessageGroupView.test.tsx`:

```tsx
it("shows run number in reviewer group titles when role run metadata exists", () => {
  render(
    <MessageGroupView
      group={{
        id: "group-code-reviewer",
        nodeId: "coding_node_0005",
        role: "code_reviewer",
        primaryEntry: makeEntry(
          "entry-review",
          "provider_stream",
          "code_reviewer",
          "Verdict: approve",
          { provider: "fake", role_run_id: "coding_role_run_0004", run_no: 2 },
        ),
        inlineEvents: [],
        interruptEntries: [],
      }}
    />,
  );

  expect(screen.getByText("Code Reviewer · Fake · Run #2")).toBeInTheDocument();
});
```

Update `groupTitle` to append `Run #n` when any grouped entry metadata has numeric `run_no`.

- [ ] **Step 4: page smoke test for reviewer readable bubble**

Add to `web/src/pages/CodingWorkspacePage.test.tsx` a chat entry with role `code_reviewer`, content `Verdict: request_changes\nSummary: needs validation`, metadata `run_no: 1`, and assert the chat list contains `Code Reviewer` and `Verdict: request_changes`.

- [ ] **Step 5: Run tests**

```bash
cargo test --locked --test it_product code_reviewer_chat_entry_contains_readable_summary
pnpm -C web exec vitest --run src/components/chat-workspace/MessageGroupView.test.tsx src/pages/CodingWorkspacePage.test.tsx
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src/product/coding_workspace_engine.rs tests/it_product/product_coding_workspace_engine.rs web/src/components/chat-workspace/MessageGroupView.tsx web/src/components/chat-workspace/MessageGroupView.test.tsx web/src/pages/CodingWorkspacePage.test.tsx
git commit -m "feat: render reviewer run summaries"
```

## Verification

Run focused checks:

```bash
cargo test --locked --test it_product review_reports_round_trip_role_run_metadata
cargo test --locked --test it_product execute_code_review_binds_report_chat_and_status_to_role_run
cargo test --locked --test it_product execute_internal_pr_review_binds_review_chat_and_status_to_role_run
cargo test --locked --test it_product retry_review_gate_response_uses_gate_stage_for_role_run_trigger
cargo test --locked --test it_product code_reviewer_chat_entry_contains_readable_summary
cargo test --locked --test it_web coding_ws_retry_review_from_internal_pr_review_resumes_internal_reviewer
pnpm -C web exec vitest --run src/api/types.test.ts src/state/coding-workspace-store.test.ts src/components/chat-workspace/MessageGroupView.test.tsx src/pages/CodingWorkspacePage.test.tsx
```

Run full local gate:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
pnpm -C web exec vitest --run
```

Do not use Docker. Do not add `-j 1` to any Cargo command.

## Implementation Handoff

This plan should be executed after P2 and before P4.

Recommended execution mode:

1. Use `superpowers:subagent-driven-development` for Task 1-5.
2. Commit after each task.
3. Stop after verification and report exact commands/results before starting P4.
