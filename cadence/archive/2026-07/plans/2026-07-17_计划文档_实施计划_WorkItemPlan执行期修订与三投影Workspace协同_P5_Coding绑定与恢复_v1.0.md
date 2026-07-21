# P5 Coding Plan Binding 与恢复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Coding Workspace 绑定具体 Plan/WorkItem/Handoff Revision，识别 Plan Defect，暂停当前 Unit，幂等应用 Plan Amendment，并从正确上游 UnitRun 自动恢复。

**Architecture:** CodingAttempt 保存 Plan Binding，CodingUnit 与 CodingUnitRun 分离。Code Review Structured Output 进入 PlanDefectRouter；PlanRepairRequest 创建后 Attempt 等待 Amendment；发布后的 Manifest 通过 Journal 原子更新绑定、UnitRun 和 Resume Target。

**Tech Stack:** Rust Coding Workspace Engine、Coding Attempt Store、Axum WebSocket、Tokio、Serde。

## Global Constraints

- Plan Defect 不调用现有 `execute_coder_fix_from_review`。
- `unit_rework_count`、`verification_retry_count`、`operational_retry_count`、`plan_repair_count` 分离。
- 旧 completed UnitRun 不改回 running。
- Manifest 应用失败时不得继续 Coding。
- 当前工作树有未提交 Diff 时必须阻止自动回退并创建 Gate。
- 所有测试名称使用 `coding_plan_repair_`、`coding_unit_run_`、`coding_amendment_` 或 `coding_runtime_handoff_` 前缀。

---

### Task 1: Coding Plan Binding 与 UnitRun 模型/Store

**Files:**
- Create: `src/product/coding_models/plan_repair.rs`
- Modify: `src/product/coding_models/mod.rs`
- Modify: `src/product/coding_models/execution.rs`
- Modify: `src/product/coding_models/group.rs`
- Create: `src/product/coding_attempt_store/plan_binding.rs`
- Create: `src/product/coding_attempt_store/unit_run.rs`
- Modify: `src/product/coding_attempt_store/mod.rs`
- Modify: `src/product/coding_attempt_store/paths.rs`
- Modify: `src/product/coding_attempt_store/tests.rs`
- Modify: `src/web/types.rs`
- Modify: `web/src/api/types/coding.ts`

**Interfaces:**
- Produces: `CodingAttemptPlanBinding`、`CodingUnitRun`、`CodingAmendmentApplicationJournal`、新 Attempt/Unit 状态
- Consumed by: Tasks 2-4

- [ ] **Step 1: 写 Binding 和 UnitRun Store 测试**

```rust
#[test]
fn coding_unit_run_records_exact_revision_and_handoff_bindings() {
    let store = coding_attempt_store_fixture();
    let attempt = group_attempt_fixture();
    let run = CodingUnitRun {
        id: "coding_unit_run_0001".to_string(),
        unit_id: "coding_unit_0001".to_string(),
        execution_no: 1,
        work_item_revision_id: "work_item_revision_0001".to_string(),
        resolved_handoff_revision_ids: vec!["handoff_revision_0001".to_string()],
        canonical_contract_hash: "contract_hash".to_string(),
        projection_bundle_id: "work_item_projection_bundle_0001".to_string(),
        projection_compiler_version: "projection-v1".to_string(),
        coder_provider_renderer_version: "codex-v1".to_string(),
        reviewer_provider_renderer_version: "codex-v1".to_string(),
        coder_projection_hash: "coder_hash".to_string(),
        reviewer_projection_hash: "reviewer_hash".to_string(),
        coder_execution_context_hash: None,
        reviewer_execution_context_hash: None,
        status: CodingUnitRunStatus::Pending,
        unit_rework_count: 0,
        verification_retry_count: 0,
        operational_retry_count: 0,
        plan_repair_count: 0,
        start_commit: None,
        completion_commit: None,
        created_at: now(),
        updated_at: now(),
    };

    store.create_coding_unit_run(&attempt, &run).unwrap();

    assert_eq!(
        store.list_coding_unit_runs(&attempt, "coding_unit_0001").unwrap(),
        vec![run]
    );
}

#[test]
fn coding_attempt_plan_binding_is_scoped_and_versioned() {
    let store = coding_attempt_store_fixture();
    let attempt = group_attempt_fixture();
    let binding = CodingAttemptPlanBinding {
        attempt_id: attempt.id.clone(),
        plan_id: "issue_plan_0001".to_string(),
        bound_plan_revision_id: "plan_revision_0001".to_string(),
        applied_amendment_ids: Vec::new(),
        updated_at: now(),
    };

    store.save_plan_binding(&attempt, &binding).unwrap();
    assert_eq!(store.get_plan_binding(&attempt).unwrap(), binding);
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib coding_unit_run_
cargo test --locked --lib coding_attempt_plan_binding_
```

Expected: FAIL，模型和 Store 不存在。

- [ ] **Step 3: 实现模型和 Store**

`CodingAttemptStatus` 完整枚举改为：

```rust
Created,
Running,
WaitingForHuman,
Blocked,
AwaitingPlanAmendment,
ApplyingPlanAmendment,
AmendmentApplyFailed,
Completed,
Failed,
Aborted,
```

`CodingExecutionUnitStatus` 完整枚举改为：

```rust
Pending,
Running,
WaitingForHuman,
Completed,
Failed,
Blocked,
BlockedByPlanDefect,
AwaitingAmendment,
NeedsRevalidation,
Stale,
Superseded,
Skipped,
```

`CodingUnitRunStatus`：

```rust
Pending,
Running,
Completed,
Failed,
Blocked,
BlockedByPlanDefect,
AwaitingAmendment,
NeedsRevalidation,
Stale,
Superseded,
```

`CodingUnitRun` 必须保存可复现运行时 Envelope 的全部绑定：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingAttemptPlanBinding {
    pub attempt_id: String,
    pub plan_id: String,
    pub bound_plan_revision_id: String,
    pub applied_amendment_ids: Vec<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingExecutionUnit {
    pub id: String,
    pub attempt_id: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub logical_work_item_id: String,
    pub work_item_revision_id: String,
    pub dependency_logical_work_item_ids: Vec<String>,
    pub order_index: u32,
    pub status: CodingExecutionUnitStatus,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub latest_handoff_revision_id: Option<String>,
    pub completion_commit: Option<String>,
    pub summary: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub struct CodingUnitRun {
    pub id: String,
    pub unit_id: String,
    pub execution_no: u32,
    pub work_item_revision_id: String,
    pub resolved_handoff_revision_ids: Vec<String>,
    pub canonical_contract_hash: String,
    pub projection_bundle_id: String,
    pub projection_compiler_version: String,
    pub coder_provider_renderer_version: String,
    pub reviewer_provider_renderer_version: String,
    pub coder_projection_hash: String,
    pub reviewer_projection_hash: String,
    pub coder_execution_context_hash: Option<String>,
    pub reviewer_execution_context_hash: Option<String>,
    pub status: CodingUnitRunStatus,
    pub unit_rework_count: u32,
    pub verification_retry_count: u32,
    pub operational_retry_count: u32,
    pub plan_repair_count: u32,
    pub start_commit: Option<String>,
    pub completion_commit: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingAmendmentApplicationPhase {
    Started,
    PlanBindingWritten,
    UnitRunsWritten,
    ResumeTargetWritten,
    Completed,
}

impl CodingAmendmentApplicationPhase {
    pub fn order(&self) -> u8 {
        match self {
            Self::Started => 0,
            Self::PlanBindingWritten => 1,
            Self::UnitRunsWritten => 2,
            Self::ResumeTargetWritten => 3,
            Self::Completed => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingAmendmentApplicationJournal {
    pub id: String,
    pub attempt_id: String,
    pub amendment_id: String,
    pub phase: CodingAmendmentApplicationPhase,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
```

前端状态联合类型必须与后端同步：

```ts
export type CodingAttemptStatus =
  | "created"
  | "running"
  | "waiting_for_human"
  | "blocked"
  | "awaiting_plan_amendment"
  | "applying_plan_amendment"
  | "amendment_apply_failed"
  | "completed"
  | "failed"
  | "aborted";

export type CodingExecutionUnitStatus =
  | "pending"
  | "running"
  | "waiting_for_human"
  | "completed"
  | "failed"
  | "blocked"
  | "blocked_by_plan_defect"
  | "awaiting_amendment"
  | "needs_revalidation"
  | "stale"
  | "superseded"
  | "skipped";

export type CodingExecutionUnit = {
  unit_id: string;
  logical_work_item_id: string;
  work_item_revision_id: string;
  dependency_logical_work_item_ids: string[];
  order_index: number;
  status: CodingExecutionUnitStatus;
  summary: string | null;
  latest_handoff_revision_id: string | null;
  completion_commit: string | null;
};
```

同步更新 `is_active()`：Attempt 的 `Created/Running/WaitingForHuman/Blocked/AwaitingPlanAmendment/ApplyingPlanAmendment/AmendmentApplyFailed` 均为 Active；Unit 的 `Running/WaitingForHuman/Blocked/BlockedByPlanDefect/AwaitingAmendment/NeedsRevalidation/Stale` 均为 Active。终态不得被恢复扫描误判为可运行状态。

Store 路径：

```text
coding-attempts/{attempt_id}/plan-binding.json
coding-attempts/{attempt_id}/amendment-applications/{amendment_id}.json
coding-attempts/{attempt_id}/units/{unit_id}/runs/{unit_run_id}.json
```

Store 接口固定为：

```rust
impl CodingAttemptStore {
    pub fn save_plan_binding(
        &self,
        attempt: &CodingExecutionAttempt,
        binding: &CodingAttemptPlanBinding,
    ) -> Result<(), ProductStoreError>;

    pub fn get_plan_binding(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingAttemptPlanBinding, ProductStoreError>;

    pub fn create_coding_unit_run(
        &self,
        attempt: &CodingExecutionAttempt,
        run: &CodingUnitRun,
    ) -> Result<(), ProductStoreError>;

    pub fn list_coding_unit_runs(
        &self,
        attempt: &CodingExecutionAttempt,
        unit_id: &str,
    ) -> Result<Vec<CodingUnitRun>, ProductStoreError>;

    pub fn list_unit_runs_by_logical_id(
        &self,
        attempt: &CodingExecutionAttempt,
        logical_work_item_id: &str,
    ) -> Result<Vec<CodingUnitRun>, ProductStoreError>;

    pub fn get_active_unit_run(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingUnitRun, ProductStoreError>;

    pub fn bind_unit_run_execution_context(
        &self,
        attempt: &CodingExecutionAttempt,
        unit_run_id: &str,
        role: CodingProviderRole,
        rendered: &RenderedExecutionContext,
    ) -> Result<CodingUnitRun, ProductStoreError>;
}
```

- [ ] **Step 4: 运行模型/Store 测试**

Run:

```bash
cargo test --locked --lib coding_unit_run_
cargo test --locked --lib coding_attempt_plan_binding_
```

Expected: Store 作用域、排序、不可变执行号和状态测试通过。

- [ ] **Step 5: 提交**

```bash
git add src/product/coding_models src/product/coding_attempt_store src/web/types.rs web/src/api/types/coding.ts
git commit -m "feat(coding-repair): bind attempts to plan revisions and unit runs"
```

### Task 2: Code Review Defect Class 与 Router

**Files:**
- Modify: `src/product/coding_models/review.rs`
- Modify: `src/product/coding_workspace_engine/review_parser.rs`
- Create: `src/product/coding_workspace_engine/plan_defect.rs`
- Modify: `src/product/coding_workspace_engine/mod.rs`
- Modify: `src/product/coding_workspace_engine/code_review.rs`
- Modify: `src/product/coding_workspace_engine/internal_pr_review.rs`
- Modify: `src/product/coding_workspace_engine/coding.rs`
- Modify: `src/product/coding_workspace_engine/handoffs.rs`
- Modify: `src/product/coding_workspace_engine/testing_parser.rs`
- Modify: `src/product/coding_workspace_engine/rework.rs`
- Modify: `src/product/tester_agent_loop/prompts.rs`
- Modify: `src/product/tester_agent_loop/report.rs`
- Modify: `src/product/tester_agent_loop/types.rs`
- Modify: `src/product/coding_workspace_engine/tests/parser_prompt/review_parser.rs`
- Modify: `src/product/coding_workspace_engine/tests/provider_driven.rs`
- Modify: `src/web/coding_ws_handler/runner.rs`

**Interfaces:**
- Consumes: P4 `PlanDefectFinding`
- Produces: `CodeReviewFlowDecision::StartPlanRepair`

- [ ] **Step 1: 写 Reviewer Parse 和路由测试**

```rust
#[test]
fn coding_plan_repair_parser_preserves_upstream_contract_invalid() {
    let payload = serde_json::json!({
        "verdict": "blocked",
        "summary": "upstream contract invalid",
        "findings": [{
            "severity": "error",
            "defect_class": "upstream_contract_invalid",
            "reason_code": "upstream_contract_capability_missing",
            "message": "missing finalization_failure",
            "contract_refs": ["repository_initialization_finalization"],
            "capability_refs": ["finalization_failure"],
            "repair_target": {
                "kind": "upstream_work_item",
                "logical_work_item_ids": ["wi_core"],
                "work_item_revision_ids": ["work_item_revision_0001"]
            },
            "recommended_route": "plan_repair",
            "confidence": "high",
            "evidence": []
        }]
    });

    let parsed = parse_review_payload(&payload.to_string()).unwrap();
    assert_eq!(
        parsed.findings[0].defect_class,
        PlanDefectClass::UpstreamContractInvalid
    );
}

#[test]
fn coding_plan_repair_router_never_sends_plan_defect_to_coder() {
    let report = blocked_review_with_plan_defect();

    assert_eq!(
        code_review_flow_decision(&report, &reviewer_projection_fixture()),
        CodeReviewFlowDecision::StartPlanRepair
    );
}

#[test]
fn coding_and_tester_plan_defects_use_the_same_finding_schema() {
    for source in [PlanDefectSource::Coder, PlanDefectSource::Tester] {
        let parsed = parse_execution_plan_defects(
            source.clone(),
            &blocked_provider_output_with_upstream_contract_invalid(),
        )
        .unwrap();

        assert_eq!(parsed.source, source);
        assert_eq!(
            parsed.findings[0].defect_class,
            PlanDefectClass::UpstreamContractInvalid
        );
        assert_eq!(
            parsed.findings[0].repair_target.as_ref().unwrap().kind,
            RepairTargetKind::UpstreamWorkItem
        );
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib coding_plan_repair_parser_
cargo test --locked --lib coding_plan_repair_router_
```

Expected: FAIL，Review Finding 没有 Defect Class。

- [ ] **Step 3: 扩展 Structured Output 与 Router**

`ReviewFinding` 增加：

```rust
pub defect_class: PlanDefectClass,
pub reason_code: Option<String>,
pub contract_refs: Vec<String>,
pub capability_refs: Vec<String>,
pub repair_target: Option<RepairTarget>,
pub recommended_route: PlanDefectRoute,
pub confidence: PlanDefectConfidence,
```

Coder 和 Tester 复用同一个 Finding Schema：

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanDefectSource {
    Coder,
    Tester,
    CodeReviewer,
    GroupReviewer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionPlanDefectReport {
    pub source: PlanDefectSource,
    pub findings: Vec<PlanDefectFinding>,
}

pub fn parse_execution_plan_defects(
    source: PlanDefectSource,
    provider_output: &str,
) -> Result<ExecutionPlanDefectReport, CodingWorkspaceEngineError>;
```

Coder Structured Output 和 Tester blocked result 均新增 `plan_defect_findings` 数组；数组非空时先经过 `validate_plan_defect_finding`，再走与 Code Reviewer 相同的路由。Internal/Group Final Reviewer 复用 `parse_review_payload` 并以 `PlanDefectSource::GroupReviewer` 进入相同优先级。普通代码失败继续走 Coder Rework，普通测试证据不足继续走 Verification Retry，Provider/权限/环境失败继续走 Operational Gate。

启动 Coder 或 Reviewer Provider 前，Engine 必须按 UnitRun 绑定加载静态 ProjectionBundle，构建 P2 `CoderExecutionEnvelope` / `ReviewerExecutionEnvelope`，调用所选 Provider Renderer，并通过 `bind_unit_run_execution_context` 保存 Renderer Version 与 Content Hash。不得从当前“最新 Plan”重新取 Projection，也不得只保存最终 Prompt 文本而丢失 Hash/Version。

路由：

```rust
pub(crate) enum CodeReviewFlowDecision {
    RunCoderFix,
    RetryVerification,
    StartPlanRepair,
    StartStoryAmendment,
    StartDesignAmendment,
    OpenOperationalGate,
    StopForHumanTriage,
    ContinueAfterApprove,
}

pub(crate) fn code_review_flow_decision(
    report: &CodeReviewReport,
    reviewer_projection: &ReviewerWorkItemProjection,
) -> CodeReviewFlowDecision {
    let valid = report
        .findings
        .iter()
        .filter(|finding| validate_plan_defect_finding(finding, reviewer_projection).is_ok())
        .collect::<Vec<_>>();

    if valid.iter().any(|finding| {
        finding.defect_class == PlanDefectClass::StoryAmendmentRequired
    }) {
        return CodeReviewFlowDecision::StartStoryAmendment;
    }
    if valid.iter().any(|finding| {
        finding.defect_class == PlanDefectClass::DesignAmendmentRequired
    }) {
        return CodeReviewFlowDecision::StartDesignAmendment;
    }
    if valid.iter().any(|finding| matches!(
        finding.defect_class,
        PlanDefectClass::CurrentWorkItemInvalid
            | PlanDefectClass::UpstreamContractInvalid
            | PlanDefectClass::DependencyGraphInvalid
    )) {
        return CodeReviewFlowDecision::StartPlanRepair;
    }
    if valid.iter().any(|finding| {
        finding.defect_class == PlanDefectClass::OperationalBlocker
    }) {
        return CodeReviewFlowDecision::OpenOperationalGate;
    }
    if valid.iter().any(|finding| {
        finding.defect_class == PlanDefectClass::VerificationIncomplete
    }) {
        return CodeReviewFlowDecision::RetryVerification;
    }
    match report.verdict {
        ReviewVerdict::RequestChanges => CodeReviewFlowDecision::RunCoderFix,
        ReviewVerdict::Blocked if code_review_report_has_actionable_findings(report) => {
            CodeReviewFlowDecision::RunCoderFix
        }
        ReviewVerdict::Blocked => CodeReviewFlowDecision::StopForHumanTriage,
        ReviewVerdict::Approve => CodeReviewFlowDecision::ContinueAfterApprove,
    }
}

pub(crate) fn validate_plan_defect_finding(
    finding: &ReviewFinding,
    reviewer_projection: &ReviewerWorkItemProjection,
) -> Result<(), PlanRepairError>;
```

`validate_plan_defect_finding` 必须调用 P4 `normalize_blocker_route`，检查 Target Kind、Contract/Capability Ref、Evidence、Confidence 和 Reviewer Projection 中的 Blocker Rule。混合 Findings 的优先级固定为 Story → Design → Plan Repair → Operational → Verification → Implementation，Plan 修订后必须使用新 Reviewer Projection 重新判断旧 Finding。

- [ ] **Step 4: 运行 Parser/Router 测试**

Run:

```bash
cargo test --locked --lib coding_plan_repair_parser_
cargo test --locked --lib coding_plan_repair_router_
cargo test --locked --lib coding_review_
```

Expected: Coder、Tester、Code Reviewer 的 Plan Defect 都进入统一 Repair；普通 implementation_defect 保持 Coder Rework，普通测试不足保持 Verification Retry。

- [ ] **Step 5: 提交**

```bash
git add src/product/coding_models/review.rs src/product/coding_workspace_engine src/web/coding_ws_handler/runner.rs
git commit -m "feat(coding-repair): route plan defects out of coder rework"
```

### Task 3: 创建 PlanRepairRequest 并暂停 Coding

**Files:**
- Modify: `src/product/coding_workspace_engine/plan_defect.rs`
- Modify: `src/product/coding_workspace_engine/timeline.rs`
- Modify: `src/product/coding_attempt_store/gate.rs`
- Modify: `src/web/coding_ws_handler/runner.rs`
- Modify: `src/web/coding_ws_handler/protocol.rs`
- Modify: `src/web/coding_ws_handler/state.rs`
- Modify: `src/web/coding_ws_handler/tests.rs`
- Create: `src/web/coding_ws_handler/tests/plan_repair.rs`
- Modify: `web/src/api/types/coding.ts`

**Interfaces:**
- Produces: Open PlanRepairRequest、`CodingWsOutMessage::PlanRepairRequired`
- Consumed by: P6 frontend、Task 4 apply

- [ ] **Step 1: 写暂停和去重测试**

```rust
#[tokio::test]
async fn coding_plan_repair_pauses_unit_without_incrementing_rework_count() {
    let fixture = coding_runner_with_upstream_contract_defect();
    let before = fixture.attempt_store.get_attempt_scoped().unwrap();

    fixture.run_code_review().await.unwrap();

    let after = fixture.attempt_store.get_attempt_scoped().unwrap();
    let run = fixture.attempt_store.get_active_unit_run(&after).unwrap();

    assert_eq!(after.status, CodingAttemptStatus::AwaitingPlanAmendment);
    assert_eq!(run.status, CodingUnitRunStatus::BlockedByPlanDefect);
    assert_eq!(run.unit_rework_count, 0);
    assert_eq!(
        fixture
            .revision_store
            .list_open_repair_requests(&fixture.plan)
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn coding_plan_repair_duplicate_finding_reuses_open_request() {
    let fixture = coding_runner_with_upstream_contract_defect();
    fixture.run_code_review().await.unwrap();
    fixture.run_duplicate_review_recovery().await.unwrap();

    assert_eq!(
        fixture
            .revision_store
            .list_open_repair_requests(&fixture.plan)
            .unwrap()
            .len(),
        1
    );
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib coding_plan_repair_pauses_
cargo test --locked --lib coding_plan_repair_duplicate_
```

Expected: FAIL，Runner 仍进入 Coder Rework。

- [ ] **Step 3: 实现暂停和 WS 事件**

```rust
// 在现有 `CodingWsOutMessage` 末尾新增以下两个具名变体。
pub enum CodingWsOutMessage {
    PlanRepairRequired {
        request: Box<PlanRepairRequestDto>,
        session_link: Option<WorkspaceSessionLinkDto>,
    },
    PlanAmendmentUpdated {
        amendment: Box<PlanAmendmentManifestDto>,
    },
}

pub type PlanRepairRequestDto = PlanRepairRequest;
pub type WorkspaceSessionLinkDto = WorkspaceSessionLink;
pub type PlanAmendmentManifestDto = PlanAmendmentManifest;
```

`start_plan_repair_from_review` 必须：

1. 验证 Finding 与 Reviewer Projection。
2. 生成 Fingerprint；命中 Open Request 时调用 `merge_repair_request_evidence`，不创建新 Request/Session。
3. 未命中时保存新的 PlanRepairRequest。
4. 当前 UnitRun → `BlockedByPlanDefect`。
5. Attempt → `AwaitingPlanAmendment`。
6. 创建 Plan Repair Timeline 节点。
7. 启动或关联 Work Item Repair Session。
8. 推送 `PlanRepairRequired`。

`CodingSessionState` 同时增加 `linked_plan_repair: Option<PlanRepairSessionSnapshotDto>`，用于断线重连；当 Attempt 为 `AwaitingPlanAmendment`、`ApplyingPlanAmendment` 或 `AmendmentApplyFailed` 时，`StartCoding`、Tester、Code Reviewer 和 Internal Reviewer 入口都必须返回 `plan_amendment_blocks_provider_run`。

- [ ] **Step 4: 运行暂停/WS 测试**

Run:

```bash
cargo test --locked --lib coding_plan_repair_
cargo test --locked --lib coding_ws_plan_repair_
```

Expected: Coding 正确暂停，重复 Finding 去重，WS DTO Roundtrip 通过。

- [ ] **Step 5: 提交**

```bash
git add src/product/coding_workspace_engine src/product/coding_attempt_store src/web/coding_ws_handler web/src/api/types/coding.ts
git commit -m "feat(coding-repair): pause coding for plan amendments"
```

### Task 4: 幂等应用 Amendment 与崩溃恢复

**Files:**
- Create: `src/product/coding_workspace_engine/amendment.rs`
- Modify: `src/product/coding_workspace_engine/mod.rs`
- Modify: `src/product/coding_workspace_engine/group.rs`
- Modify: `src/product/coding_attempt_store/plan_binding.rs`
- Modify: `src/product/coding_attempt_store/unit_run.rs`
- Modify: `src/product/coding_attempt_store/recovery.rs`
- Create: `src/product/coding_workspace_engine/tests/plan_amendment.rs`
- Modify: `src/web/coding_ws_handler/runner.rs`
- Modify: `src/web/coding_ws_handler/tests/plan_repair.rs`

**Interfaces:**
- Consumes: P4 `PlanAmendmentManifest`
- Produces: updated Plan Binding、new UnitRun、Resume Target

- [ ] **Step 1: 写当前案例和 Journal 恢复测试**

```rust
#[tokio::test]
async fn coding_amendment_reexecutes_only_upstream_and_rebinds_blocked_consumer() {
    let fixture = coding_attempt_awaiting_finalization_amendment();
    let manifest = finalization_amendment_manifest_fixture();

    let updated = fixture
        .engine
        .apply_plan_amendment(&fixture.attempt, &manifest)
        .await
        .unwrap();

    let binding = fixture.store.get_plan_binding(&updated).unwrap();
    assert_eq!(binding.bound_plan_revision_id, "plan_revision_0002");

    let core_runs = fixture.store.list_unit_runs_by_logical_id(&updated, "wi_core").unwrap();
    assert_eq!(core_runs.last().unwrap().work_item_revision_id, "work_item_revision_0002");

    let registration_runs = fixture
        .store
        .list_unit_runs_by_logical_id(&updated, "wi_registration")
        .unwrap();
    assert_eq!(
        registration_runs.last().unwrap().status,
        CodingUnitRunStatus::AwaitingAmendment
    );
    assert!(fixture.store.list_unit_runs_by_logical_id(&updated, "wi_unrelated").unwrap().is_empty());
}

#[tokio::test]
async fn coding_amendment_recovers_after_plan_binding_write() {
    let fixture = coding_attempt_with_interrupted_amendment(
        CodingAmendmentApplicationPhase::PlanBindingWritten,
    );

    let recovered = fixture.engine.recover_plan_amendment(&fixture.attempt).await.unwrap();

    assert_eq!(recovered.status, CodingAttemptStatus::Running);
    assert_eq!(
        fixture.store.list_unit_runs_by_logical_id(&recovered, "wi_core").unwrap().len(),
        1
    );
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib coding_amendment_
```

Expected: FAIL，Amendment Apply 尚不存在。

- [ ] **Step 3: 实现事务应用**

```rust
impl CodingWorkspaceEngine {
    pub async fn apply_plan_amendment(
        &self,
        attempt: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        self.ensure_clean_or_checkpointed_worktree(attempt)?;
        let mut journal = self
            .store
            .load_or_prepare_amendment_application(attempt, manifest)?;
        if journal.phase == CodingAmendmentApplicationPhase::Completed {
            return self.store.get_attempt_scoped(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
            );
        }
        self.store.set_attempt_status(attempt, CodingAttemptStatus::ApplyingPlanAmendment)?;

        if journal.phase.order()
            < CodingAmendmentApplicationPhase::PlanBindingWritten.order()
        {
            self.store.update_plan_binding_from_manifest(attempt, manifest)?;
            journal = self.store.advance_application_phase(
                attempt,
                &journal.id,
                CodingAmendmentApplicationPhase::PlanBindingWritten,
            )?;
        }

        if journal.phase.order()
            < CodingAmendmentApplicationPhase::UnitRunsWritten.order()
        {
            self.store.materialize_unit_runs_from_manifest(attempt, manifest)?;
            journal = self.store.advance_application_phase(
                attempt,
                &journal.id,
                CodingAmendmentApplicationPhase::UnitRunsWritten,
            )?;
        }

        if journal.phase.order()
            < CodingAmendmentApplicationPhase::ResumeTargetWritten.order()
        {
            self.store.set_resume_target(attempt, &manifest.resume_target)?;
            journal = self.store.advance_application_phase(
                attempt,
                &journal.id,
                CodingAmendmentApplicationPhase::ResumeTargetWritten,
            )?;
        }

        let binding = self.store.get_plan_binding(attempt)?;
        self.revision_store.release_active_amendment(
            &self.revision_store.get_plan_lineage(
                &attempt.project_id,
                &attempt.issue_id,
                &binding.plan_id,
            )?,
            &manifest.id,
        )?;
        self.revision_store.update_repair_request_status(
            &self.revision_store.get_plan_lineage(
                &attempt.project_id,
                &attempt.issue_id,
                &binding.plan_id,
            )?,
            &manifest.repair_request_id,
            PlanRepairRequestStatus::Applied,
        )?;
        self.store.advance_application_phase(
            attempt,
            &journal.id,
            CodingAmendmentApplicationPhase::Completed,
        )?;
        let updated = self.store.get_attempt_scoped(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        self.resume_attempt_after_amendment(&updated)
    }
}
```

`CodingAttemptStore` 必须提供 `load_or_prepare_amendment_application`、`advance_application_phase`、`mark_application_failed`、`materialize_unit_runs_from_manifest` 和 `set_resume_target`，每个步骤以 Amendment ID 幂等。任一步失败时保留最后成功 Phase、Journal 写入 `error`、Attempt → `AmendmentApplyFailed`，禁止启动 Provider Run；恢复入口清除错误并从最后成功 Phase 继续。重复应用相同 `amendment_id` 必须返回已完成结果，不创建重复 UnitRun。未提交 Diff 时创建 `worktree_dirty_before_plan_amendment` Gate。

- [ ] **Step 4: 运行 Amendment 应用定向验证**

Run:

```bash
cargo fmt --check
cargo check --locked
cargo test --locked --lib coding_unit_run_
cargo test --locked --lib coding_plan_repair_
cargo test --locked --lib coding_amendment_
cargo test --locked --lib coding_ws_plan_repair_
```

Expected: 所有命令 exit 0，当前案例、重复应用和每个 Journal 边界恢复测试通过。

- [ ] **Step 5: 提交**

```bash
git add src/product/coding_workspace_engine src/product/coding_attempt_store src/web/coding_ws_handler
git commit -m "feat(coding-repair): apply and recover plan amendments"
```

### Task 5: Handoff Revision 与运行时影响传播

**Files:**
- Modify: `src/product/coding_workspace_engine/handoffs.rs`
- Modify: `src/product/coding_workspace_engine/group.rs`
- Create: `src/product/coding_workspace_engine/runtime_impact.rs`
- Modify: `src/product/coding_workspace_engine/mod.rs`
- Modify: `src/product/coding_attempt_store/unit_run.rs`
- Modify: `src/product/work_item_revision_store/handoff.rs`
- Create: `src/product/coding_workspace_engine/tests/runtime_handoff_impact.rs`
- Modify: `src/product/coding_workspace_engine/tests.rs`
- Modify: `src/web/coding_ws_handler/runner.rs`
- Modify: `src/web/workspace_ws_handler/mapping.rs`
- Modify: `src/web/workspace_ws_types/artifact.rs`

**Interfaces:**
- Consumes: P4 `ContractImpactReport`、P5 Task 4 新 UnitRun、P1 `HandoffRevision`
- Produces: `compare_handoff_revisions`、`RuntimeHandoffImpactPropagator::apply_completed_handoff`

- [ ] **Step 1: 写固定案例恢复和停止传播失败测试**

```rust
#[tokio::test]
async fn coding_runtime_handoff_resumes_original_consumer_revision_with_new_handoff() {
    let fixture = coding_attempt_after_upstream_revision_two_completed();
    let result = fixture
        .engine
        .apply_completed_handoff(
            &fixture.attempt,
            &handoff_revision_two_with_finalization_capabilities(),
        )
        .await
        .unwrap();

    assert_eq!(result.resumed_units, vec!["wi_registration"]);
    let runs = fixture
        .store
        .list_unit_runs_by_logical_id(&fixture.attempt, "wi_registration")
        .unwrap();
    let resumed = runs.last().unwrap();
    assert_eq!(resumed.work_item_revision_id, "work_item_revision_wi02_v1");
    assert_eq!(
        resumed.resolved_handoff_revision_ids,
        vec!["handoff_revision_0002"]
    );
    assert_eq!(resumed.status, CodingUnitRunStatus::Pending);
    assert!(fixture
        .store
        .list_unit_runs_by_logical_id(&fixture.attempt, "wi_unrelated")
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn coding_runtime_handoff_stops_conditional_propagation_when_contract_hash_is_unchanged() {
    let fixture = coding_attempt_with_conditional_downstream();
    let result = fixture
        .engine
        .apply_completed_handoff(&fixture.attempt, &same_contract_new_commit_handoff())
        .await
        .unwrap();

    assert!(result.newly_stale_units.is_empty());
    assert!(result.conditional_units_released.is_empty());
    assert_eq!(result.propagation_stopped_at, Some("wi_core".to_string()));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib coding_runtime_handoff_
```

Expected: FAIL，完成 UnitRun 仍只写旧 Handoff，没有运行时影响传播。

- [ ] **Step 3: 实现 Handoff Delta 和幂等恢复**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandoffDeltaKind {
    Unchanged,
    CompatibleExtension,
    BreakingChange,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeHandoffImpactResult {
    pub resumed_units: Vec<String>,
    pub revalidation_units: Vec<String>,
    pub newly_stale_units: Vec<String>,
    pub conditional_units_released: Vec<String>,
    pub propagation_stopped_at: Option<String>,
}

pub fn compare_handoff_revisions(
    previous: Option<&HandoffRevision>,
    next: &HandoffRevision,
) -> HandoffDeltaKind;

pub struct RuntimeHandoffImpactPropagator;

impl RuntimeHandoffImpactPropagator {
    pub fn apply_completed_handoff(
        &self,
        attempt: &CodingExecutionAttempt,
        next_handoff: &HandoffRevision,
        manifest: &PlanAmendmentManifest,
        graph: &DependencyContractGraph,
    ) -> Result<RuntimeHandoffImpactResult, CodingWorkspaceEngineError>;
}

impl CodingWorkspaceEngine {
    pub async fn apply_completed_handoff(
        &self,
        attempt: &CodingExecutionAttempt,
        next_handoff: &HandoffRevision,
    ) -> Result<RuntimeHandoffImpactResult, CodingWorkspaceEngineError>;
}
```

`CodingWorkspaceEngine::apply_completed_handoff` 按 Attempt Plan Binding 加载最近已应用 Manifest 和其 Dependency Graph，再调用 `RuntimeHandoffImpactPropagator`；没有 Amendment 时沿现有正常 Group Handoff 路径继续。

UnitRun 完成时先不可变写入新 `HandoffRevision`，再比较 old/new `contract_hash` 与 Capability：

`contract_hash` 只对排序后的 `provided_contracts` 和 `provided_capabilities` 做稳定 SHA-256；`commit_sha`、测试文本和普通说明不得造成 Contract Hash 变化。

1. `Unchanged`：停止 conditional downstream 传播，但恢复仅等待该 Handoff 且输入仍满足的直接消费者。
2. `CompatibleExtension`：为 Manifest 中显式 `revalidation_required_units` 创建新 UnitRun，保留其原 `work_item_revision_id`，绑定新 Handoff。
3. `BreakingChange`：直接消费者新 UnitRun 标记 `Stale`，沿 P4 `conditional_downstream` 等待其后续 Handoff。
4. 每个新 UnitRun 的幂等键固定为 `(attempt_id, amendment_id, logical_work_item_id, work_item_revision_id, resolved_handoff_revision_ids)`。
5. WI-01 → WI-02 固定案例必须创建 WI-02 UnitRun 2；WI-03/WI-04 只有在实际消费变化 Contract 或后续 Handoff 变化时才受影响。

每次创建 UnitRun 或 HandoffRevision 后同步刷新 P3 `WorkItemRevisionHistoryDto` 的真实 Artifact 引用，使完整 Work Item Workspace 与内嵌 Repair Center 使用同一历史数据。

- [ ] **Step 4: 运行 P5 全部验证**

Run:

```bash
cargo fmt --check
cargo check --locked
cargo test --locked --lib coding_unit_run_
cargo test --locked --lib coding_plan_repair_
cargo test --locked --lib coding_amendment_
cargo test --locked --lib coding_runtime_handoff_
cargo test --locked --lib coding_ws_plan_repair_
```

Expected: Plan Defect 暂停、Manifest 恢复、运行时 Handoff 传播和固定案例全部通过，测试运行数大于 0。

- [ ] **Step 5: 提交**

```bash
git add src/product/coding_workspace_engine src/product/coding_attempt_store/unit_run.rs src/product/work_item_revision_store/handoff.rs src/web/coding_ws_handler/runner.rs src/web/workspace_ws_handler/mapping.rs src/web/workspace_ws_types/artifact.rs
git commit -m "feat(coding-repair): propagate runtime handoff impact"
```
