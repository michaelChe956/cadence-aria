# WorkItemPlan Review Findings 传递与 Outline 回退修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复 `Work Item Plan #workspace_session_0003` 中 reviewer 多轮指出同类问题但 author 无法准确返修的问题，确保 reviewer 的结构化 `findings` 完整进入下一轮 author prompt，并让 item 级 `plan_reopen_required` 能回退到 Outline revision。

**Architecture:** 在 Workspace Engine review 层新增统一的 feedback formatter，所有 WorkItemPlan review 返修、可选建议应用、Outline 重开路径都复用同一份格式化结果。WebSocket handler 保持 provider run 的 feedback 传递语义清晰，避免只依赖 `pending_revision_context` 的隐式状态。返修判定从“只认 Outline scope”扩展为识别 item/batch review 要求 `ReviseOutline` 或 `PlanReopenRequired` 的情况。

**Tech Stack:** Rust 2024、Workspace Engine、WorkItemPlan Draft/Outline 流程、Cargo 单元测试。验证命令必须遵守 `cadence/project-rules/build-test-commands.md`，禁止 `-j 1`。

---

## 结论

本案例不是 reviewer 入参缺少当前 draft。已排查到的 020/023/026 轮 reviewer prompt 都包含当前 draft。

主要问题是 reviewer 的结构化 `findings[].message/evidence/impact/required_action` 虽然已解析并落到 `ReviewVerdict.findings`，但进入下一轮 author 的 feedback 多数只用了 `verdict.comments` 或只保留了 severity/message，导致关键行动项、证据、影响范围丢失。第三轮 reviewer 指出的真实缺口是 `ProviderName` 扩展遗漏了写入范围外 match：`src/product/work_item_split_engine/types.rs:86` 与 `src/web/test_controls/fixtures.rs:239`。这说明 author 产物确实还有问题，但系统没有把 reviewer 的可执行 findings 稳定传回 author，是反复返修的重要原因。

另外，item review 里的 `plan_reopen_required` 当前无法自然进入 Outline revision。`review_decision_restarts_work_item_plan_outline()` 只认 `review_scope == Outline && review_action == ReviseOutline`，但 item reviewer 合法输出是 `review_scope == Item`、`review_action == ReviseOutline`、`verdict == PlanReopenRequired`。

## 文件结构

- Create: `src/product/workspace_engine/review/feedback.rs`
  - 统一格式化 `ReviewVerdict`，完整保留 `summary`、`comments`、`work_item_plan_review` 路由信息、`findings` 的 severity/message/evidence/impact/required_action。
- Modify: `src/product/workspace_engine/review.rs`
  - 增加 `mod feedback;`。
- Modify: `src/product/workspace_engine/review/routing.rs`
  - item draft required revise 使用完整 formatter，不再只写 `verdict.comments`。
- Modify: `src/product/workspace_engine/decisions.rs`
  - optional findings apply 使用完整 formatter。
  - `review_decision_restarts_work_item_plan_outline()` 支持 item/batch scope 的 `ReviseOutline` / `PlanReopenRequired`。
  - WorkItemPlan Outline revision feedback 复用完整 formatter 或至少保留 evidence/impact/required_action。
- Modify: `src/product/workspace_engine/plan_outline/revision.rs`
  - `work_item_plan_revision_feedback()`、`work_item_plan_outline_revision_feedback()` 复用统一 formatter，避免 outline 路径只保留 severity/message。
- Optional Modify: `src/product/workspace_engine/types.rs`
  - 如实现时选择让 handler 显式携带 feedback，则将 `WorkItemDraftDecisionOutcome::StartDraftRun` 改为 `StartDraftRun { feedback: Option<String> }`；否则保留现状，但测试必须覆盖 `pending_revision_context` fallback。
- Optional Modify: `src/web/workspace_ws_handler/decisions/inbound.rs`
  - 若上一项改 enum，则 `ProviderRunKind::WorkItemPlanDraft { feedback }` 透传 outcome feedback；否则只补测试/注释，说明 `feedback: None` 会触发 engine 读取 `pending_revision_context`。
- Test: `src/product/workspace_engine/review/feedback.rs`
- Test: `src/product/workspace_engine/tests/part_03/part_02.rs`

## Task 1: 增加统一 Review Feedback Formatter

**Files:**
- Create: `src/product/workspace_engine/review/feedback.rs`
- Modify: `src/product/workspace_engine/review.rs`

- [ ] **Step 1: 写失败测试**

在 `src/product/workspace_engine/review/feedback.rs` 中新增测试模块，先写 formatter 的期望输出：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::workspace_ws_types::review::{
        ReviewFinding, ReviewFindingSeverity, ReviewGate, ReviewVerdict, ReviewVerdictType,
        WorkItemPlanReviewAction, WorkItemPlanReviewComplete, WorkItemPlanReviewGate,
        WorkItemPlanReviewScope, WorkItemPlanReviewVerdict,
    };

    #[test]
    fn work_item_plan_review_feedback_includes_actionable_findings() {
        let verdict = ReviewVerdict {
            verdict: ReviewVerdictType::Revise,
            comments: "需要覆盖 provider metadata 的所有 match。".to_string(),
            summary: "遗漏 ProviderName 新枚举的边界写入范围".to_string(),
            findings: vec![ReviewFinding {
                severity: ReviewFindingSeverity::MustFix,
                message: "ProviderName 扩展遗漏 match 分支".to_string(),
                evidence: "src/product/work_item_split_engine/types.rs:86".to_string(),
                impact: "新增 provider 时 draft 会遗漏运行时映射。".to_string(),
                required_action: "把 provider_name_to_type 和测试 fixture provider_name 一并纳入本 work item 的写入范围。".to_string(),
            }],
            review_gate: ReviewGate::RequiresRevision,
            work_item_plan_review: Some(WorkItemPlanReviewComplete {
                verdict: WorkItemPlanReviewVerdict::Revise,
                review_scope: WorkItemPlanReviewScope::Item,
                target_outline_id: Some("outline_backend_metadata_state".to_string()),
                generation_round_id: "round_0001".to_string(),
                draft_id: Some("draft_001".to_string()),
                batch_id: None,
                review_action: WorkItemPlanReviewAction::ReviseCurrentItem,
                gates: vec![WorkItemPlanReviewGate::RequiresCurrentItemRevision],
                affects_items: vec![],
                warnings: vec![],
            }),
        };

        let feedback = format_review_feedback(&verdict);

        assert!(feedback.contains("[review_summary]"));
        assert!(feedback.contains("遗漏 ProviderName 新枚举的边界写入范围"));
        assert!(feedback.contains("[review_comments]"));
        assert!(feedback.contains("[review_findings]"));
        assert!(feedback.contains("severity: must_fix"));
        assert!(feedback.contains("message: ProviderName 扩展遗漏 match 分支"));
        assert!(feedback.contains("evidence: src/product/work_item_split_engine/types.rs:86"));
        assert!(feedback.contains("impact: 新增 provider 时 draft 会遗漏运行时映射。"));
        assert!(feedback.contains("required_action: 把 provider_name_to_type"));
        assert!(feedback.contains("[work_item_plan_review]"));
        assert!(feedback.contains("review_scope: item"));
        assert!(feedback.contains("review_action: revise_current_item"));
    }
}
```

- [ ] **Step 2: 运行定向测试确认失败**

```bash
cargo test --locked --lib work_item_plan_review_feedback_includes_actionable_findings
```

Expected: FAIL，原因是 `format_review_feedback` 尚未实现。

- [ ] **Step 3: 实现 formatter**

新增 `format_review_feedback(verdict: &ReviewVerdict) -> String`。格式稳定、可 grep、给 author 明确行动项：

```rust
use crate::product::workspace_engine::serialized_string;
use crate::web::workspace_ws_types::review::ReviewVerdict;

pub(crate) fn format_review_feedback(verdict: &ReviewVerdict) -> String {
    let mut parts = Vec::new();

    if !verdict.summary.trim().is_empty() {
        parts.push(format!("[review_summary]\n{}", verdict.summary.trim()));
    }
    if !verdict.comments.trim().is_empty() {
        parts.push(format!("[review_comments]\n{}", verdict.comments.trim()));
    }
    if let Some(review) = &verdict.work_item_plan_review {
        parts.push(format!(
            "[work_item_plan_review]\nverdict: {}\nreview_scope: {}\nreview_action: {}\ntarget_outline_id: {}\ndraft_id: {}\nbatch_id: {}",
            serialized_string(&review.verdict),
            serialized_string(&review.review_scope),
            serialized_string(&review.review_action),
            review.target_outline_id.as_deref().unwrap_or(""),
            review.draft_id.as_deref().unwrap_or(""),
            review.batch_id.as_deref().unwrap_or("")
        ));
    }
    if !verdict.findings.is_empty() {
        let findings = verdict
            .findings
            .iter()
            .enumerate()
            .map(|(index, finding)| {
                format!(
                    "{}. severity: {}\n   message: {}\n   evidence: {}\n   impact: {}\n   required_action: {}",
                    index + 1,
                    serialized_string(&finding.severity),
                    finding.message.trim(),
                    finding.evidence.trim(),
                    finding.impact.trim(),
                    finding.required_action.trim()
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        parts.push(format!("[review_findings]\n{findings}"));
    }

    parts.join("\n\n")
}
```

- [ ] **Step 4: 挂载模块并跑测试**

在 `src/product/workspace_engine/review.rs` 添加：

```rust
mod feedback;
```

运行：

```bash
cargo test --locked --lib work_item_plan_review_feedback_includes_actionable_findings
```

Expected: PASS。

## Task 2: item/batch 返修路径使用完整 findings

**Files:**
- Modify: `src/product/workspace_engine/review/routing.rs`
- Modify: `src/product/workspace_engine/decisions.rs`
- Modify: `src/product/workspace_engine/plan_outline/revision.rs`
- Test: `src/product/workspace_engine/tests/part_03/part_02.rs`

- [ ] **Step 1: 写失败测试，锁住 optional item apply 的 prompt 内容**

在 `work_item_plan_item_optional_choice_can_apply_findings` 的 outcome 后追加 prompt 断言：

```rust
let input = engine
    .build_current_work_item_draft_streaming_input(None)
    .expect("draft streaming input");
assert!(input.prompt.contains("[review_findings]"));
assert!(input.prompt.contains("evidence: 主路径完整"));
assert!(input.prompt.contains("impact: 不影响继续"));
assert!(input.prompt.contains("required_action: 可补充说明"));
```

当前实现只依赖 `verdict.comments`，该测试应 FAIL。

- [ ] **Step 2: 写失败测试，锁住 batch optional apply 的完整 findings**

在 `work_item_plan_batch_optional_choice_can_apply_findings` 的现有 prompt 断言后追加：

```rust
assert!(input.prompt.contains("[review_findings]"));
assert!(input.prompt.contains("evidence: 主路径完整"));
assert!(input.prompt.contains("impact: 不影响继续"));
assert!(input.prompt.contains("required_action: 可补充说明"));
```

当前实现只断言 comments，该新增断言应 FAIL。

- [ ] **Step 3: 写失败测试，锁住 required item revise 的完整 findings**

新增测试 `work_item_plan_item_required_revision_feedback_includes_findings`：准备 `WorkItemDraftReview` active node，驱动 reviewer 输出 `strong_recommend_fix`，然后在自动重写的 draft run 上构建 prompt，断言包含 `[review_findings]`、`evidence`、`impact`、`required_action`。

核心断言：

```rust
assert_eq!(engine.session().stage, WorkspaceStage::Running);
let active_node = engine
    .timeline_nodes
    .iter()
    .find(|node| Some(&node.node_id) == engine.active_node_id.as_ref())
    .expect("active draft run node");
assert_eq!(active_node.node_type, TimelineNodeType::WorkItemDraftRun);

let input = engine
    .build_current_work_item_draft_streaming_input(None)
    .expect("draft streaming input");
assert!(input.prompt.contains("[review_findings]"));
assert!(input.prompt.contains("required_action: 当前 draft 需明确 spawn_blocking 或改为 async 方案"));
```

- [ ] **Step 4: 运行失败测试**

```bash
cargo test --locked --lib work_item_plan_item_optional_choice_can_apply_findings
cargo test --locked --lib work_item_plan_batch_optional_choice_can_apply_findings
cargo test --locked --lib work_item_plan_item_required_revision_feedback_includes_findings
```

Expected: FAIL，失败点是 prompt 缺少完整 findings 字段。

- [ ] **Step 5: 替换 routing/decisions 的 feedback 来源**

在 `src/product/workspace_engine/review/routing.rs` 引入 formatter，并替换：

```rust
self.pending_revision_context = Some(format_review_feedback(&verdict));
```

在 `src/product/workspace_engine/decisions.rs` 的 `apply_optional_findings` item/batch 分支中，把：

```rust
.map(|verdict| verdict.comments.clone())
```

替换为：

```rust
.map(format_review_feedback)
```

在 `src/product/workspace_engine/plan_outline/revision.rs` 中让 `work_item_plan_revision_feedback()` 与 `work_item_plan_outline_revision_feedback()` 复用 `format_review_feedback(verdict)`，再拼接用户补充信息。

- [ ] **Step 6: 运行定向测试确认通过**

```bash
cargo test --locked --lib work_item_plan_item_optional_choice_can_apply_findings
cargo test --locked --lib work_item_plan_batch_optional_choice_can_apply_findings
cargo test --locked --lib work_item_plan_item_required_revision_feedback_includes_findings
```

Expected: PASS。

## Task 3: item 级 plan_reopen_required 回退到 Outline revision

**Files:**
- Modify: `src/product/workspace_engine/decisions.rs`
- Test: `src/product/workspace_engine/tests/part_03/part_02.rs`

- [ ] **Step 1: 写失败测试**

新增测试 `work_item_plan_item_plan_reopen_review_decision_restarts_outline_revision`。构造 `ReviewVerdict`：

```rust
ReviewVerdict {
    verdict: ReviewVerdictType::NeedsHuman,
    comments: "当前 item 暴露出 Outline 边界错误".to_string(),
    summary: "需要重开 Outline".to_string(),
    findings: vec![ReviewFinding {
        severity: ReviewFindingSeverity::MustFix,
        message: "item 写入范围漏掉共享 provider match".to_string(),
        evidence: "src/product/work_item_split_engine/types.rs:86".to_string(),
        impact: "只修当前 draft 会继续遗漏边界。".to_string(),
        required_action: "回到 Outline，把 provider metadata 状态边界扩到所有 ProviderName match。".to_string(),
    }],
    review_gate: ReviewGate::UserTriageRequired,
    work_item_plan_review: Some(WorkItemPlanReviewComplete {
        verdict: WorkItemPlanReviewVerdict::PlanReopenRequired,
        review_scope: WorkItemPlanReviewScope::Item,
        target_outline_id: Some("outline_backend_metadata_state".to_string()),
        generation_round_id: "round_0001".to_string(),
        draft_id: Some("draft_001".to_string()),
        batch_id: None,
        review_action: WorkItemPlanReviewAction::ReviseOutline,
        gates: vec![WorkItemPlanReviewGate::RequiresPlanReopen],
        affects_items: vec![],
        warnings: vec![],
    }),
}
```

执行：

```rust
let outcome = engine
    .handle_review_decision("revise".to_string(), None)
    .await
    .expect("item plan reopen should restart outline revision");

let ReviewDecisionOutcome::StartWorkItemPlanOutlineRevision { feedback } = outcome else {
    panic!("expected outline revision outcome");
};
let feedback = feedback.expect("outline feedback");
assert!(feedback.contains("[review_findings]"));
assert!(feedback.contains("required_action: 回到 Outline"));
assert_eq!(engine.session().stage, WorkspaceStage::Running);
```

当前实现应 FAIL，因为 `review_decision_restarts_work_item_plan_outline()` 只认 Outline scope。

- [ ] **Step 2: 运行失败测试**

```bash
cargo test --locked --lib work_item_plan_item_plan_reopen_review_decision_restarts_outline_revision
```

Expected: FAIL，实际走 generic `Revision` 或人工分支。

- [ ] **Step 3: 修改判定函数**

把 `review_decision_restarts_work_item_plan_outline()` 改为识别以下任一条件：

```rust
review.review_action == WorkItemPlanReviewAction::ReviseOutline
    || review.verdict == WorkItemPlanReviewVerdict::PlanReopenRequired
    || review.gates.contains(&WorkItemPlanReviewGate::RequiresPlanReopen)
```

保留 `self.session.workspace_type == WorkspaceType::WorkItemPlan` 前置条件。

- [ ] **Step 4: 运行定向测试确认通过**

```bash
cargo test --locked --lib work_item_plan_item_plan_reopen_review_decision_restarts_outline_revision
```

Expected: PASS。

## Task 4: 明确 WebSocket draft run feedback 语义

**Files:**
- Optional Modify: `src/product/workspace_engine/types.rs`
- Optional Modify: `src/product/workspace_engine/draft_batch/decisions.rs`
- Optional Modify: `src/web/workspace_ws_handler/decisions/inbound.rs`
- Test: 选择现有 Workspace Engine 单测覆盖 prompt 内容；如已有 Task 2 覆盖 `pending_revision_context` fallback，则本 task 可不改 enum。

- [ ] **Step 1: 判断是否需要改 enum**

当前 `ProviderRunKind::WorkItemPlanDraft { feedback: None }` 并不必然丢反馈，因为 `build_current_work_item_draft_streaming_input(None)` 会读取并消费 `engine.pending_revision_context`。但这个语义隐式，容易误读，也容易在后续异步流程中被改坏。

推荐两种实现选一：

1. 低风险方案：不改 enum，只靠 Task 2 的 prompt 单测固定 `pending_revision_context` fallback 行为。
2. 显式方案：把 `WorkItemDraftDecisionOutcome::StartDraftRun` 改成 `StartDraftRun { feedback: Option<String> }`，handler 透传到 `ProviderRunKind::WorkItemPlanDraft { feedback }`。

- [ ] **Step 2: 如选显式方案，先改测试期望**

把现有：

```rust
ReviewDecisionOutcome::StartWorkItemDraft { feedback: None }
```

仅在真实应该无 feedback 的 skip/continue 路径保留。rewrite/apply findings 路径期望 `Some(...)`，并断言包含 `[review_findings]`。

- [ ] **Step 3: 如选显式方案，改 handler**

把 `src/web/workspace_ws_handler/decisions/inbound.rs` 的：

```rust
ProviderRunKind::WorkItemPlanDraft { feedback: None }
```

改为透传 outcome 中携带的 `feedback`。

- [ ] **Step 4: 运行相关定向测试**

```bash
cargo test --locked --lib work_item_plan_item_optional_choice_can_apply_findings
cargo test --locked --lib accepting_work_item_draft_updates_current_artifact_without_new_version
```

Expected: PASS。

## Task 5: 回归验证与三模块影响说明

**Files:**
- No code files unless前面测试暴露共享链路影响。

- [ ] **Step 1: 定向回归**

```bash
cargo test --locked --lib work_item_plan_review_feedback_includes_actionable_findings
cargo test --locked --lib work_item_plan_item_optional_choice_can_apply_findings
cargo test --locked --lib work_item_plan_batch_optional_choice_can_apply_findings
cargo test --locked --lib work_item_plan_item_required_revision_feedback_includes_findings
cargo test --locked --lib work_item_plan_item_plan_reopen_review_decision_restarts_outline_revision
```

- [ ] **Step 2: Workspace Engine 相关回归**

```bash
cargo test --locked --lib part_03
cargo test --locked --lib part_10
```

- [ ] **Step 3: 标准验证**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
```

- [ ] **Step 4: 三模块联动说明**

本修复触及 Workspace review/revision 共享链路，但修改点以 WorkItemPlan 专属 `work_item_plan_review` 扩展和 WorkItemPlan Draft/Outline 路由为主。Story Spec / Design Spec 仍走通用 `ReviewVerdict` 与 generic `Revision`，不使用 `WorkItemPlanReviewComplete` 的 item/batch/outline 路由字段。执行完成汇报时必须说明：

- Work Item：已通过新增/修改单测覆盖。
- Story Spec：不受 WorkItemPlan 专属路由影响；若 formatter 被复用于 generic revision，需要补现有 generic review 单测。
- Design Spec：同 Story Spec。

## 验收标准

- reviewer 的 `findings[].evidence/impact/required_action` 能出现在下一轮 WorkItem Draft author prompt 的 `[user_or_reviewer_feedback]` 中。
- optional findings 的“应用建议”路径与 required revise 路径行为一致，均传完整 actionable feedback。
- item 级 `plan_reopen_required` 或 `review_action == revise_outline` 能进入 WorkItemPlan Outline revision，而不是 generic revision 或只停人工。
- 不改变 reviewer prompt 是否包含当前 draft 的既有行为；该部分当前不是根因。
- 定向测试与标准 Rust 验证通过，命令不使用 `-j 1`。
