# Code Review 分诊门禁 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Code Review 的三个人工路由决策（`StopForHumanTriage`、`RetryVerification`、`OpenOperationalGate`）落地可操作 blocked gate，并提供送回 Coder/重试审查/人工继续/终止四个动作，彻底消除 attempt 静默假死在 `running/code_review` 的缺陷。

**Architecture:** 在 `execute_code_review_with_commands` 内复用已计算的流程决策落地门禁（与 coding 侧 `open_coding_output_human_triage_gate` 位置一致）；复用 `create_review_blocked_gate` 并新增三个 reason code；放宽 `send_to_coder` 的 verdict 前置与 `SendToCoder` 分派，使分诊门禁能走代码审查反馈返修路径；在 reviewer projection 渲染契约补 implementation defect 字段边界。

**Tech Stack:** Rust（edition 2024，stable 工具链，`cargo test --locked`，🔴 禁止 `-j 1`）、OpenSpec。

**关联契约：** `openspec/changes/open-code-review-triage-gate/`（proposal/design/specs/coding-code-review-triage/spec.md/tasks.md）

## Global Constraints

- 所有仓库操作只在 worktree `/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0722-add-workspace` 中进行。
- 宿主机 Rust/Cargo；定向单测用 `cargo test --locked --lib <过滤名>` 或 `cargo test --locked --lib <模块路径>`；🔴 禁止 `-j 1`。
- 不修改 `validate_plan_defect_finding` 的校验判定，不放宽 plan defect 契约（方向 A）。
- 不改变 plan repair 唤起条件；门禁动作不触发 plan repair。
- 不为 `RetryVerification` 引入自动化验证补跑。
- 不引入、不恢复任何 Testing 或 tester 角色相关流程内容。
- 不改动 coding 阶段 `runner.rs` 中同类静默返回路径（另案）。
- 不自动迁移、重放或改写历史停滞 attempt。
- 不重启服务、不调用 Provider、不创建业务数据（除非用户在 Task 3.3 明确授权）。
- `src/product/coding_workspace_engine/tests.rs` 当前 798 行，新增引擎层测试优先放入拆分测试文件；契约文案测试放 `src/product/work_item_projection/tests/render_reviewer.rs`。
- `git commit` 消息使用中文项目惯用前缀（`test:` / `fix:` / `docs:`）。
- 实施采用 Subagent-Driven 模式：每个任务完成后执行规格+质量双阶段审查。

---

## File Structure

**修改文件：**

- `src/product/coding_workspace_engine/code_review.rs`：在 `execute_code_review_with_commands` 的 verdict 分支后新增分诊门禁落地逻辑，与既有 `code_review_blocked` 门禁互斥。
- `src/product/coding_workspace_engine/gates.rs`：扩展 `is_code_review_blocked_gate`（或新增判定函数），让三个分诊 reason code 的 `SendToCoder` 走 `send_code_review_feedback_to_coder`。
- `src/product/coding_workspace_engine/rework.rs`：放宽 `send_code_review_feedback_to_coder` 的 verdict 前置条件（`rework.rs:456`），支持 `request_changes`。
- `src/product/work_item_projection/render.rs`：在 `role_structured_output_contract(Reviewer)`（`render.rs:472`）增补 implementation defect 字段边界与证据出口文案。

**测试文件：**

- `src/product/coding_workspace_engine/tests/code_review_triage.rs`（新建）：三个决策的引擎层门禁落地测试、互斥回归、送回 Coder 路径测试、复现回归。
- `src/product/work_item_projection/tests/render_reviewer.rs`（已有文件追加）：契约文案断言。

**不修改文件：**

- `src/web/coding_ws_handler/runner.rs:609` 分支：保持 emit + return；门禁落地后 attempt 已是 `blocked`，runner 读取状态即可，不新增旁路。
- `src/product/coding_workspace_engine/tests/plan_defect_entrypoints.rs:136`：该断言针对 internal review 的 `internal_review_human_triage` 门禁，刻意不含 `send_to_coder`；与 code review 分诊门禁不冲突，禁止改动。

---

## Task 1: 失败测试 — 三个决策的门禁落地与互斥

**Files:**
- Create: `src/product/coding_workspace_engine/tests/code_review_triage.rs`
- Modify: `src/product/coding_workspace_engine/tests.rs`（注册新测试模块）

**Interfaces:**
- Consumes: `execute_code_review_with_commands(attempt, provider, command_rx)`（`code_review.rs`）；`CapturingProjectionProvider::new(output)`（`tests/provider_execution_context.rs`）；`review_plan_defect_output()`（`tests/provider_execution_context.rs:522`）；`running_attempt_with_worktree()`（参考 `plan_defect_entrypoints.rs`）。
- Produces: 测试覆盖 spec requirement 1（StopForHumanTriage）、2（RetryVerification）、3（OpenOperationalGate）、4（动作集合）、6（互斥）。

- [ ] **Step 1: 注册新测试模块**

Modify `src/product/coding_workspace_engine/tests.rs`，在现有 `mod` 声明区追加：

```rust
#[path = "tests/code_review_triage.rs"]
mod code_review_triage;
```

- [ ] **Step 2: 编写 StopForHumanTriage 门禁失败测试**

Create `src/product/coding_workspace_engine/tests/code_review_triage.rs`。先实现公共夹具，再写第一个测试。夹具复用 `running_attempt_with_worktree()` + `init_test_git_repo`，Provider 用一个返回「implementation defect 携带非空 plan_defect_evidence」JSON 的 `CapturingProjectionProvider`：

```rust
use super::*;
use crate::product::coding_attempt_store::CodingAttemptStatus;

fn implementation_finding_with_plan_defect_fields() -> serde_json::Value {
    serde_json::json!({
        "verdict": "request_changes",
        "summary": "需要返修",
        "findings": [{
            "severity": "error",
            "file_path": "src/lib.rs",
            "line": 1,
            "message": "缺少测试执行证据",
            "required_action": "补交红灯执行记录",
            "source_stage": "code_review",
            "defect_class": "implementation_defect",
            "recommended_route": "coder_rework",
            "plan_defect_evidence": [{
                "kind": "manual_check",
                "source_ref": "test_evidence_refs",
                "message": "证据为空"
            }]
        }]
    })
}

async fn run_code_review_with_provider_output(
    output: serde_json::Value,
) -> (
    crate::product::coding_attempt_store::CodingAttemptStore,
    crate::product::models::CodingExecutionAttempt,
) {
    let (root, store, attempt) = running_attempt_with_worktree();
    init_test_git_repo(attempt.worktree_path.as_ref().unwrap());
    let _ = root;
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(
        store.clone(),
        crate::product::git_workspace::GitWorkspaceService::new(),
        tx,
    );
    let provider = super::provider_execution_context::CapturingProjectionProvider::new(
        output.to_string(),
    );
    let (_cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel(1);
    engine
        .execute_code_review_with_commands(&attempt, &provider, &mut cmd_rx)
        .await
        .unwrap();
    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    (store, persisted)
}

#[tokio::test]
async fn stop_for_human_triage_lands_blocked_gate_with_review_actions() {
    let (store, attempt) =
        run_code_review_with_provider_output(implementation_finding_with_plan_defect_fields())
            .await;
    assert_eq!(attempt.status, CodingAttemptStatus::Blocked);
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    assert_eq!(gates.len(), 1, "only one gate must land");
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("code_review_output_human_triage")
    );
    let action_ids: Vec<&str> = gates[0]
        .available_actions
        .iter()
        .map(|a| a.action_id.as_str())
        .collect();
    assert_eq!(
        action_ids,
        vec!["retry_review", "send_to_coder", "manual_continue", "abort"]
    );
}
```

- [ ] **Step 3: 运行测试，确认 RED**

Run: `cargo test --locked --lib code_review_triage::stop_for_human_triage_lands_blocked_gate_with_review_actions -- --nocapture`
Expected: FAIL — attempt 仍为 `Running`，`gates` 为空（当前静默退出不落门禁）。

- [ ] **Step 4: 编写 RetryVerification 与 OpenOperationalGate 失败测试**

追加两个测试，Provider 输出分别为通过契约校验的 `verification_incomplete` 与 `operational_blocker` finding。这两个 finding 需满足 `validate_plan_defect_finding` 的完整校验（reason_code + confidence=high + 证据 + 与 reviewer_projection blocker_routing 对齐）。**注意：** 这两个 finding 要落在真实的 reviewer_projection 上下文中。如果夹具构造 reviewer_projection blocker_rule 成本过高，改为通过 `execute_internal_pr_review` 无法触达、直接调用引擎门禁函数的更低层测试；若仍不可行，在测试中 `todo!("reason_code/route projection 对齐夹具待补")` 并在 Task 2 实现后回填，但必须明确记录为 RED 而非 PASS。

预期 RED 点：reason code 分别为 `code_review_verification_incomplete` / `code_review_operational_blocker`，动作集合同为四项。

- [ ] **Step 5: 编写互斥回归测试**

```rust
#[tokio::test]
async fn blocked_verdict_without_actionable_findings_lands_only_code_review_blocked_gate() {
    // Provider 返回 verdict=blocked 且无可执行 finding
    let (store, attempt) = run_code_review_with_provider_output(serde_json::json!({
        "verdict": "blocked",
        "summary": "被阻塞",
        "findings": []
    })).await;
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0].reason_code.as_deref(), Some("code_review_blocked"));
    assert!(
        gates.iter().all(|g| g.reason_code.as_deref()
            != Some("code_review_output_human_triage")),
        "must not double-gate"
    );
}
```

- [ ] **Step 6: 运行全部新测试，确认 RED 状态符合预期**

Run: `cargo test --locked --lib code_review_triage -- --nocapture`
Expected: `stop_for_human_triage_lands_blocked_gate_with_review_actions` RED（Running/无 gate）；互斥测试当前应 PASS（既有 `code_review_blocked` 行为）；`RetryVerification`/`OpenOperationalGate` 按 Step 4 记录的状态。

- [ ] **Step 7: `cargo check --locked` + `cargo fmt --check`**

Run: `cargo check --locked && cargo fmt --check`
Expected: 通过。

- [ ] **Step 8: Commit**

```bash
git add src/product/coding_workspace_engine/tests/code_review_triage.rs \
        src/product/coding_workspace_engine/tests.rs
git commit -m "test: cover code review triage gate landing"
```

---

## Task 2: 生产实现 — 分诊门禁落地与动作集合

**Files:**
- Modify: `src/product/coding_workspace_engine/code_review.rs:147-215`
- Modify: `src/product/coding_workspace_engine/provider_failure.rs:42-60`（动作集合分派）

**Interfaces:**
- Consumes: `code_review_flow_decision(&report, &reviewer_projection)`（`plan_defect.rs:107`，已在 `code_review.rs:148` 计算）；`create_review_blocked_gate(ReviewBlockedGateInput)`（`provider_failure.rs:16`）；`code_review_report_has_actionable_findings(&report)`（`code_review.rs:201` 已用）。
- Produces: 三个新 reason code 的 blocked gate；attempt 置 `blocked`；动作集合含四项。

- [ ] **Step 1: 在 provider_failure.rs 扩展动作集合分派**

`create_review_blocked_gate`（`provider_failure.rs:42`）当前对三个新 reason code 会落到 else 分支，动作是 `retry_review` + `send_to_coder` + `abort`（三项，缺 `manual_continue`）。把三个 code review 分诊 reason code 归入一个显式分支，动作集合为四项：

```rust
let available_actions = if reason_code == "code_review_provider_interrupted" {
    vec![retry_action]
} else if matches!(
    reason_code,
    "internal_review_operational_blocker" | "internal_review_human_triage"
) {
    vec![retry_action, coding_gate_action_for_id("abort").expect("abort action")]
} else if matches!(
    reason_code,
    "code_review_output_human_triage"
        | "code_review_verification_incomplete"
        | "code_review_operational_blocker"
) {
    vec![
        retry_action,
        coding_gate_action_for_id("send_to_coder").expect("send to coder action"),
        coding_gate_action_for_id("manual_continue").expect("manual continue action"),
        coding_gate_action_for_id("abort").expect("abort action"),
    ]
} else {
    vec![
        retry_action,
        coding_gate_action_for_id("send_to_coder").expect("send to coder action"),
        coding_gate_action_for_id("abort").expect("abort action"),
    ]
};
```

顺序：`retry_review`、`send_to_coder`、`manual_continue`、`abort`（与 spec scenario 一致）。

- [ ] **Step 2: 在 code_review.rs 落地分诊门禁，与 code_review_blocked 互斥**

`code_review.rs:201` 既有逻辑：`verdict == Blocked && !has_actionable_findings` → `code_review_blocked` gate。这是 `StopForHumanTriage` 决策的子集（verdict=blocked 无 actionable finding 时 `code_review_flow_decision` 返回 `StopForHumanTriage`）。

互斥策略：**让既有 `code_review_blocked` 分支继续处理它覆盖的情形，新逻辑只在「该分支未触发」时按决策落分诊门禁。** 在 `code_review.rs:201` 的 `if` 块之后追加：

```rust
// 既有的 code_review_blocked 门禁已落地时，不再重复落地分诊门禁。
let code_review_blocked_landed = report.verdict == ReviewVerdict::Blocked
    && !code_review_report_has_actionable_findings(&report);
if !code_review_blocked_landed {
    let (triage_reason_code, triage_title) = match plan_defect_route {
        CodeReviewFlowDecision::StopForHumanTriage => (
            "code_review_output_human_triage",
            "Code Review 结论需人工分诊",
        ),
        CodeReviewFlowDecision::RetryVerification => (
            "code_review_verification_incomplete",
            "Code Review 验证证据不完整",
        ),
        CodeReviewFlowDecision::OpenOperationalGate => (
            "code_review_operational_blocker",
            "Code Review 命中运维阻塞",
        ),
        // RunCoderFix / StartPlanRepair / ContinueAfterApprove 不在此处理
        _ => (core::option::Option::<&str>::None, ""),
    };
    if let (Some(reason_code), title) = (triage_reason_code, triage_title) {
        self.create_review_blocked_gate(ReviewBlockedGateInput {
            attempt: &attempt,
            node_id: &node.id,
            stage: CodingExecutionStage::CodeReview,
            role: CodingProviderRole::CodeReviewer,
            title: title.to_string(),
            description: report.summary.clone(),
            reason_code,
            evidence_refs: vec![report.id.clone()],
            raw_provider_output_ref: Some(raw_provider_output_ref.clone()),
        })
        .await?;
    }
}
```

**关键约束：**
- 必须在既有 `code_review_blocked` 的 `if` 块**之后**追加，用 `code_review_blocked_landed` 守卫互斥。
- 不得覆盖 `RunCoderFix`、`StartPlanRepair`、`ContinueAfterApprove`（这些由 runner 后续处理，不落门禁）。
- `description` 用 `report.summary`，与 coding 侧 triage gate 的描述风格一致。

- [ ] **Step 3: 确认 plan_defect_route 已在作用域内**

`code_review.rs:148` 已计算 `plan_defect_route`，变量在该函数体内可见。`raw_provider_output_ref` 在 `code_review.rs:135` 已构造。`node` 在 `code_review.rs:38`。确认无未定义符号。

- [ ] **Step 4: 运行 Task 1 测试，确认 RED 转 GREEN**

Run: `cargo test --locked --lib code_review_triage -- --nocapture`
Expected: `stop_for_human_triage_lands_blocked_gate_with_review_actions` PASS（reason code + 四项动作 + Blocked）；互斥测试仍 PASS；`RetryVerification`/`OpenOperationalGate` 若 Step 4 夹具已补齐则 PASS，否则保持记录状态。

- [ ] **Step 5: 运行既有 code review 门禁回归，确认无破坏**

Run: `cargo test --locked --lib code_review -- --nocapture`
Expected: 全部 PASS。重点关注 `code_review_provider_failure_blocks_attempt_without_cleaning_shared_worktree`（`provider_failure_recovery.rs:10`）与 `blocked_code_review_without_structured_findings_accepts_manual_feedback_for_coder`（`gate_coder_feedback.rs:4`）—— 既有 `code_review_blocked` 门禁行为不得改变。

- [ ] **Step 6: `cargo check --locked` + `cargo clippy` + `cargo fmt --check`**

Run: `cargo clippy --all-targets --all-features --locked -- -D warnings && cargo check --locked && cargo fmt --check`
Expected: 全部通过。

- [ ] **Step 7: Commit**

```bash
git add src/product/coding_workspace_engine/code_review.rs \
        src/product/coding_workspace_engine/provider_failure.rs
git commit -m "fix: land blocked gate for code review human triage decisions"
```

---

## Task 3: 生产实现 — 放宽送回 Coder 通道

**Files:**
- Modify: `src/product/coding_workspace_engine/gates.rs:633-641`（`SendToCoder` 分派）+ `gates.rs:741`（`is_code_review_blocked_gate`）
- Modify: `src/product/coding_workspace_engine/rework.rs:456`（verdict 前置）

**Interfaces:**
- Consumes: `send_code_review_feedback_to_coder(&current, extra_context)`（`rework.rs:420`）；`send_review_limit_feedback_to_coder(&current, extra_context)`（`rework.rs:330`）。
- Produces: 分诊门禁的 `send_to_coder` 走代码审查反馈返修路径；`request_changes` verdict 可送回 Coder。

- [ ] **Step 1: 失败测试 — 分诊门禁送回 Coder 路径**

在 `code_review_triage.rs` 追加两个测试。先测成功路径：先调用 `execute_code_review_with_commands` 落地 `code_review_output_human_triage` 门禁，再通过引擎 gate 响应入口执行 `send_to_coder`（带 operator_context），断言：

```rust
#[tokio::test]
async fn triage_gate_send_to_coder_routes_request_changes_to_coder_rework() {
    let (store, attempt) =
        run_code_review_with_provider_output(implementation_finding_with_plan_defect_fields())
            .await;
    let gate_id = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
        .gate_id;
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(
        store.clone(),
        crate::product::git_workspace::GitWorkspaceService::new(),
        tx,
    );
    let updated = engine
        .respond_gate_action(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate_id,
            "send_to_coder",
            Some("请按审查结论返修".to_string()),
        )
        .unwrap();
    assert_eq!(updated.stage, CodingExecutionStage::Coding);
    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.rework_count, attempt.rework_count + 1);
}
```

确认 `respond_gate_action` 的确切签名（在 `gates.rs` 内查找 `pub.*fn respond_gate_action` 或同等公开入口），按实际签名调整调用。

再测拒绝路径（spec requirement 5 第二条 scenario）：

```rust
#[tokio::test]
async fn triage_gate_send_to_coder_without_operator_context_is_rejected() {
    let (store, attempt) =
        run_code_review_with_provider_output(implementation_finding_with_plan_defect_fields())
            .await;
    let gate_id = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
        .gate_id;
    let (tx, _rx) = tokio::sync::mpsc::channel(64);
    let engine = CodingWorkspaceEngine::new(
        store.clone(),
        crate::product::git_workspace::GitWorkspaceService::new(),
        tx,
    );
    let result = engine
        .respond_gate_action(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate_id,
            "send_to_coder",
            None,
        );
    assert!(result.is_err(), "must reject send_to_coder without operator context");
    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    assert_eq!(persisted.status, CodingAttemptStatus::Blocked, "must stay blocked");
}
```

该拒绝语义来自 `send_code_review_feedback_to_coder`（`rework.rs:436`）对 `operator_context` 的强制要求；本 change 复用该不变量，不改它。

- [ ] **Step 2: 运行测试，确认 RED**

Run: `cargo test --locked --lib code_review_triage::triage_gate_send_to_coder -- --nocapture`
Expected: 成功路径 FAIL（当前 `send_to_coder` 在分诊 reason code 上走 `send_review_limit_feedback_to_coder`，因 `rework_count(0) < max_auto_rework(2)` 返回 `send_to_coder_not_available`；或即便走到 `send_code_review_feedback_to_coder`，`verdict=request_changes` 在 `rework.rs:456` 被拒）；拒绝路径当前可能 PASS（既有 operator_context 强制），记录其状态。

- [ ] **Step 3: 扩展 SendToCoder 分派条件**

`gates.rs:635` 的 `is_code_review_blocked_gate(&gate)` 只认 `code_review_blocked`。新增判定函数或扩展该函数，使三个分诊 reason code 也走 `send_code_review_feedback_to_coder`：

```rust
fn is_code_review_feedback_gate(gate: &CodingGateRequired) -> bool {
    gate.stage == Some(CodingExecutionStage::CodeReview)
        && gate.role == Some(CodingProviderRole::CodeReviewer)
        && matches!(
            gate.reason_code.as_deref(),
            Some("code_review_blocked")
                | Some("code_review_output_human_triage")
                | Some("code_review_verification_incomplete")
                | Some("code_review_operational_blocker")
        )
}
```

`gates.rs:635` 改用 `is_code_review_feedback_gate`：

```rust
CodingGateActionType::SendToCoder => {
    if is_code_review_feedback_gate(&gate) {
        self.send_code_review_feedback_to_coder(&current, extra_context)?
    } else {
        self.send_review_limit_feedback_to_coder(&current, extra_context)?
    }
}
```

**保留** `is_code_review_blocked_gate`（`gates.rs:741`）原样，它可能在别处使用；先 grep 确认所有调用点，若仅此处使用可删除，否则保留。

- [ ] **Step 4: 放宽 rework.rs verdict 前置条件**

`rework.rs:456`：

```rust
if review_report.verdict != ReviewVerdict::Blocked {
    return Err(... "send_to_coder_latest_review_not_actionable" ...);
}
```

改为支持 `RequestChanges`：

```rust
if !matches!(
    review_report.verdict,
    ReviewVerdict::Blocked | ReviewVerdict::RequestChanges
) {
    return Err(CodingWorkspaceEngineError::ProviderStream(
        "send_to_coder_latest_review_not_actionable".to_string(),
    ));
}
```

**不得** 放宽 `send_review_limit_feedback_to_coder`（`rework.rs:340`）的 `rework_count >= max_auto_rework` 前置——那是审查轮次超限路径的固有语义，分诊门禁不走它。

- [ ] **Step 5: 运行新测试，确认 GREEN**

Run: `cargo test --locked --lib code_review_triage::triage_gate_send_to_coder -- --nocapture`
Expected: 成功路径 PASS（stage=`Coding`、status=`Running`、rework_count 递增）；拒绝路径 PASS（status 保持 `Blocked`）。

- [ ] **Step 6: 运行既有 send_to_coder 回归**

Run: `cargo test --locked --lib gate_coder_feedback gate_rework -- --nocapture`
Expected: 全部 PASS。重点关注 `send_to_coder_after_review_limit_accepts_actionable_blocked_code_review`（`gate_rework.rs:581`）—— 既有 `code_review_blocked` 门禁的 `send_to_coder` 行为不得改变（它现在也走 `is_code_review_feedback_gate`，verdict=Blocked 仍被接受）。

- [ ] **Step 7: 确认未误改 internal review 测试**

Run: `cargo test --locked --lib plan_defect_entrypoints -- --nocapture`
Expected: 全部 PASS。`plan_defect_entrypoints.rs:136` 的断言针对 internal review，不受影响。

- [ ] **Step 8: `cargo check --locked` + clippy + fmt**

Run: `cargo clippy --all-targets --all-features --locked -- -D warnings && cargo check --locked && cargo fmt --check`
Expected: 通过。

- [ ] **Step 9: Commit**

```bash
git add src/product/coding_workspace_engine/gates.rs \
        src/product/coding_workspace_engine/rework.rs \
        src/product/coding_workspace_engine/tests/code_review_triage.rs
git commit -m "fix: route code review triage send-to-coder to coder rework"
```

---

## Task 4: 生产实现 — Reviewer 契约强化

**Files:**
- Modify: `src/product/work_item_projection/render.rs:472-479`
- Test: `src/product/work_item_projection/tests/render_reviewer.rs`

**Interfaces:**
- Consumes: `role_structured_output_contract(ProjectionRenderRole::Reviewer)`（`render.rs:469`）。
- Produces: Reviewer projection 渲染文本包含 implementation defect 字段边界与证据出口文案。

- [ ] **Step 1: 失败测试 — 契约文案**

在 `render_reviewer.rs` 追加测试，调用既有 reviewer projection 渲染入口（参考该文件已有测试的渲染调用方式），断言渲染文本包含两条文案：

```rust
#[test]
fn reviewer_contract_bounds_implementation_defect_fields() {
    let rendered = /* 复用本文件已有的 reviewer projection 渲染夹具 */;
    let text = &rendered.text;
    assert!(
        text.contains("implementation_defect"),
        "contract must mention implementation_defect class"
    );
    assert!(
        text.contains("plan_defect_evidence")
            && text.contains("禁止"),
        "contract must forbid plan defect fields on implementation defect"
    );
    assert!(
        text.contains("message") && text.contains("required_action"),
        "contract must name message/required_action as evidence outlet"
    );
}
```

- [ ] **Step 2: 运行测试，确认 RED**

Run: `cargo test --locked --lib render_reviewer::reviewer_contract_bounds_implementation_defect_fields -- --nocapture`
Expected: FAIL — 当前契约文案不含这些字段。

- [ ] **Step 3: 在 render.rs 增补契约文案**

`render.rs:474` 的 `ProjectionRenderRole::Reviewer` 分支，在现有 `findings` 字段说明后追加两条：

```rust
ProjectionRenderRole::Reviewer => concat!(
    "\nCode Review 结构化输出契约:\n",
    "- 最终审查结论必须只输出一个 JSON 对象：{\"verdict\":\"approve|request_changes|blocked\",\"summary\":\"...\",\"findings\":[...]}\n",
    "- verdict 只能使用 approve、request_changes、blocked；如果没有阻塞问题，verdict 使用 approve。\n",
    "- findings 必须包含 severity、file_path、line、message、required_action、source_stage=code_review。\n",
    "- defect_class=implementation_defect 的 finding 禁止填写 reason_code、contract_refs、capability_refs、repair_target、confidence、plan_defect_evidence；这些字段必须省略或留空。\n",
    "- implementation_defect 的证据写入 message 与 required_action 的自然语言描述；只有计划类缺陷（current_work_item_invalid、upstream_contract_invalid、dependency_graph_invalid 等）才允许携带 plan_defect_evidence 与路由字段。\n",
    "- 除最终结论 JSON 外，其余任何内容（包括路由回执、验证证据、示例和表格）不得出现 { 或 }；证据中的 JSON 片段必须改写为自然语言描述。\n",
    "- JSON 必须以 { 开头，以 } 结尾；不要输出 Markdown 代码块或自然语言总结。\n"
),
```

- [ ] **Step 4: 运行测试，确认 GREEN**

Run: `cargo test --locked --lib render_reviewer::reviewer_contract_bounds_implementation_defect_fields -- --nocapture`
Expected: PASS。

- [ ] **Step 5: 运行 reviewer 渲染全量回归**

Run: `cargo test --locked --lib render_reviewer -- --nocapture`
Expected: 全部 PASS。注意：若既有测试断言完整渲染文本（如 hash 断言），新增文案会改变 content_hash —— 更新这些断言为新的期望值（不是放宽，是同步文案变更）。

- [ ] **Step 6: 确认校验判定未放宽**

Run: `cargo test --locked --lib plan_defect -- --nocapture`
Expected: 全部 PASS。`validate_plan_defect_finding` 行为不变。

- [ ] **Step 7: `cargo check --locked` + clippy + fmt**

Run: `cargo clippy --all-targets --all-features --locked -- -D warnings && cargo check --locked && cargo fmt --check`
Expected: 通过。

- [ ] **Step 8: Commit**

```bash
git add src/product/work_item_projection/render.rs \
        src/product/work_item_projection/tests/render_reviewer.rs
git commit -m "fix: bound implementation defect fields in reviewer contract"
```

---

## Task 5: 全量验证与交付

**Files:**
- 无（仅验证与 OpenSpec 勾选）

- [ ] **Step 1: lib 全量测试**

Run: `cargo test --locked --lib`
Expected: 全部 PASS（区分既有失败基线：`it_product` 3 项、`it_core` large_file_guard 1 项属已知基线，不在 lib 范围）。

- [ ] **Step 2: it_product 定向回归**

Run: `cargo test --locked --test it_product product_coding_workspace_engine -- --nocapture`
Expected: 仅有已知 3 项基线失败，无新增失败。

- [ ] **Step 3: 既有失败基线复核**

确认 `it_product` 仍只有这 3 项失败：
- `group_final_review_prompt_includes_all_unit_handoffs`
- `execute_group_final_review_prompt_includes_request_commit_diff_and_function_context`
- `group_final_confirm_completes_attempt_after_all_units_completed`

若出现新的 code review / triage / send_to_coder 相关失败，立即停止并排查。

- [ ] **Step 4: OpenSpec strict 校验**

Run: `openspec validate open-code-review-triage-gate --strict`
Expected: Change valid.

- [ ] **Step 5: 勾选 OpenSpec tasks 1.1-1.6、2.1-2.5、3.1、3.2**

修改 `openspec/changes/open-code-review-triage-gate/tasks.md`，勾选完成项（3.3 运行时验收保持未勾）。

- [ ] **Step 6: `git diff --check` + 状态提交**

```bash
git diff --check
git add openspec/changes/open-code-review-triage-gate/tasks.md
git commit -m "docs: record code review triage gate status"
```

- [ ] **Step 7: 请求用户授权重启后端（对应 OpenSpec 3.3）**

向用户报告：实现与验证完成，请求重启后端以进行人工业务验收。用户确认前不重启、不调用 Provider、不创建业务数据。

---

## Verification Anchors

- **核心回归（必须 GREEN）：** `code_review_triage` 全部、`code_review` 全部、`gate_coder_feedback`、`gate_rework`、`render_reviewer`、`plan_defect`、`plan_defect_entrypoints`。
- **既有基线失败（不影响本 change）：** `it_product` 3 项、`it_core` `large_file_guard`。
- **安全断言：** `validate_plan_defect_finding` 判定不变；plan repair 唤起条件不变；internal review triage 门禁不含 `send_to_coder` 的断言不变；既有 `code_review_blocked` 门禁行为不变。
