# P4 Plan Repair 与影响分析 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现 Plan Defect 领域模型、Contract Delta、两阶段影响分析、Work Item Repair Child Session 和可确认的 Plan Amendment 发布。

**Architecture:** `plan_repair` 模块负责纯领域分类、Delta 和 Impact；Work Item Workspace Engine 负责创建 Repair Session、生成 Draft Revision、Plan Review 和发布 Amendment。Coding Workspace 只提供 Request/Evidence，本阶段不应用 Manifest。

**Tech Stack:** Rust 2024、Serde、Workspace Engine、Revision Store、现有 Workspace Session/Timeline。

## Global Constraints

- Plan Defect 不能直接修改 Coding Attempt。
- Repair Session 必须绑定 `base_plan_revision_id`。
- 同一个 Plan 只能有一个 Active Amendment。
- 影响分析必须基于 Contract Edge，不得按 Work Item 序号全部失效。
- 用户可以扩大影响集；缩小必须有风险接受并重新 Plan Review。
- 所有测试名称使用 `plan_repair_`、`contract_delta_` 或 `contract_impact_` 前缀。

---

### Task 1: Plan Defect 与 Repair Request 领域模型

**Files:**
- Create: `src/product/plan_repair/mod.rs`
- Create: `src/product/plan_repair/model.rs`
- Create: `src/product/plan_repair/fingerprint.rs`
- Create: `src/product/plan_repair/tests.rs`
- Modify: `src/product/mod.rs`
- Modify: `src/product/models/work_item_revision.rs`

**Interfaces:**
- Consumes: P1 `PlanDefectClass`、`PlanDefectRoute`、`RepairTarget`、`PlanRepairRequest`，P2 `BlockerRoute`
- Produces: `PlanDefectFinding`、`default_route`、`normalize_blocker_route`、`plan_defect_fingerprint`
- Consumed by: Work Item Repair、P5 Coding Router

- [ ] **Step 1: 写分类和 Fingerprint 测试**

```rust
#[test]
fn plan_repair_fingerprint_is_stable_for_reordered_refs() {
    let left = finding_fixture(
        PlanDefectClass::UpstreamContractInvalid,
        vec!["contract_b", "contract_a"],
        vec!["cap_b", "cap_a"],
    );
    let right = finding_fixture(
        PlanDefectClass::UpstreamContractInvalid,
        vec!["contract_a", "contract_b"],
        vec!["cap_a", "cap_b"],
    );

    assert_eq!(
        plan_defect_fingerprint("plan_revision_0001", &left),
        plan_defect_fingerprint("plan_revision_0001", &right)
    );
}

#[test]
fn plan_repair_classification_marks_only_implementation_defect_as_coder_rework() {
    assert_eq!(
        default_route(&PlanDefectClass::ImplementationDefect),
        PlanDefectRoute::CoderRework
    );
    assert_eq!(
        default_route(&PlanDefectClass::UpstreamContractInvalid),
        PlanDefectRoute::PlanRepair
    );
}

#[test]
fn plan_repair_normalizes_projection_routes_without_losing_repair_target() {
    assert_eq!(
        normalize_blocker_route(BlockerRoute::PlanRepairCurrent),
        NormalizedPlanDefectRoute {
            route: PlanDefectRoute::PlanRepair,
            required_target_kind: Some(RepairTargetKind::CurrentWorkItem),
        }
    );
    assert_eq!(
        normalize_blocker_route(BlockerRoute::PlanRepairUpstream),
        NormalizedPlanDefectRoute {
            route: PlanDefectRoute::PlanRepair,
            required_target_kind: Some(RepairTargetKind::UpstreamWorkItem),
        }
    );
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib plan_repair_
```

Expected: FAIL，Plan Repair 模块不存在。

- [ ] **Step 3: 实现模型和稳定 Fingerprint**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanDefectSeverity {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanDefectConfidence {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanDefectEvidence {
    pub kind: String,
    pub source_ref: String,
    pub message: String,
}

// 在本阶段把 P1 的弱类型 evidence 字段收紧。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRepairRequest {
    pub id: String,
    pub plan_id: String,
    pub base_plan_revision_id: String,
    pub trigger_attempt_id: String,
    pub trigger_unit_run_id: String,
    pub trigger_review_id: Option<String>,
    pub trigger_finding_id: String,
    pub amendment_id: Option<String>,
    pub defect_class: PlanDefectClass,
    pub reason_code: String,
    pub repair_target: RepairTarget,
    pub contract_refs: Vec<String>,
    pub capability_refs: Vec<String>,
    pub evidence: Vec<PlanDefectEvidence>,
    pub fingerprint: String,
    pub status: PlanRepairRequestStatus,
    pub created_at: String,
    pub updated_at: String,
}

impl WorkItemRevisionStore {
    pub fn merge_repair_request_evidence(
        &self,
        plan: &WorkItemPlanLineage,
        request_id: &str,
        evidence: Vec<PlanDefectEvidence>,
    ) -> Result<PlanRepairRequest, ProductStoreError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanDefectFinding {
    pub finding_id: String,
    pub severity: PlanDefectSeverity,
    pub defect_class: PlanDefectClass,
    pub reason_code: String,
    pub message: String,
    pub evidence: Vec<PlanDefectEvidence>,
    pub contract_refs: Vec<String>,
    pub capability_refs: Vec<String>,
    pub repair_target: Option<RepairTarget>,
    pub recommended_route: PlanDefectRoute,
    pub confidence: PlanDefectConfidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedPlanDefectRoute {
    pub route: PlanDefectRoute,
    pub required_target_kind: Option<RepairTargetKind>,
}

#[derive(Debug)]
pub enum PlanRepairError {
    InvalidFinding(String),
    InvalidRepairTarget(String),
    ContractValidation(ContractValidationReport),
    ProjectionValidation(ProjectionValidationReport),
    ActiveAmendmentExists { amendment_id: String },
    AmendmentConflict { expected: String, actual: String },
    ConfirmationRequired,
    RiskAcceptanceRequired,
    Store(ProductStoreError),
}

pub fn default_route(class: &PlanDefectClass) -> PlanDefectRoute {
    match class {
        PlanDefectClass::ImplementationDefect => PlanDefectRoute::CoderRework,
        PlanDefectClass::VerificationIncomplete => PlanDefectRoute::VerificationRetry,
        PlanDefectClass::CurrentWorkItemInvalid
        | PlanDefectClass::UpstreamContractInvalid
        | PlanDefectClass::DependencyGraphInvalid => PlanDefectRoute::PlanRepair,
        PlanDefectClass::DesignAmendmentRequired => PlanDefectRoute::DesignAmendment,
        PlanDefectClass::StoryAmendmentRequired => PlanDefectRoute::StoryAmendment,
        PlanDefectClass::OperationalBlocker => PlanDefectRoute::OperationalGate,
    }
}

pub fn normalize_blocker_route(route: BlockerRoute) -> NormalizedPlanDefectRoute {
    match route {
        BlockerRoute::CoderRework => NormalizedPlanDefectRoute {
            route: PlanDefectRoute::CoderRework,
            required_target_kind: None,
        },
        BlockerRoute::VerificationRetry => NormalizedPlanDefectRoute {
            route: PlanDefectRoute::VerificationRetry,
            required_target_kind: None,
        },
        BlockerRoute::PlanRepairCurrent => NormalizedPlanDefectRoute {
            route: PlanDefectRoute::PlanRepair,
            required_target_kind: Some(RepairTargetKind::CurrentWorkItem),
        },
        BlockerRoute::PlanRepairUpstream => NormalizedPlanDefectRoute {
            route: PlanDefectRoute::PlanRepair,
            required_target_kind: Some(RepairTargetKind::UpstreamWorkItem),
        },
        BlockerRoute::SubgraphReplan => NormalizedPlanDefectRoute {
            route: PlanDefectRoute::PlanRepair,
            required_target_kind: Some(RepairTargetKind::Subgraph),
        },
        BlockerRoute::StoryAmendment => NormalizedPlanDefectRoute {
            route: PlanDefectRoute::StoryAmendment,
            required_target_kind: None,
        },
        BlockerRoute::DesignAmendment => NormalizedPlanDefectRoute {
            route: PlanDefectRoute::DesignAmendment,
            required_target_kind: None,
        },
        BlockerRoute::OperationalGate => NormalizedPlanDefectRoute {
            route: PlanDefectRoute::OperationalGate,
            required_target_kind: None,
        },
    }
}
```

P1 的 `PlanDefectRoute::PlanRepair` 是持久化层的通用路由；P2 的 Current/Upstream/Subgraph 粒度必须通过 `RepairTargetKind` 保留。Router 在接受 Finding 前校验 `recommended_route`、`defect_class` 与 `repair_target.kind` 三者一致。Fingerprint 实现必须排序 `contract_refs`、`capability_refs` 和 Target IDs 后再做 SHA-256。

- [ ] **Step 4: 运行模型测试**

Run:

```bash
cargo test --locked --lib plan_repair_
```

Expected: 分类、默认路由、Fingerprint 和 Serde 测试通过。

- [ ] **Step 5: 提交**

```bash
git add src/product/plan_repair src/product/models/work_item_revision.rs src/product/mod.rs
git commit -m "feat(plan-repair): model plan defects and repair targets"
```

### Task 2: Contract Delta 与静态影响分析

**Files:**
- Create: `src/product/plan_repair/delta.rs`
- Create: `src/product/plan_repair/impact.rs`
- Modify: `src/product/plan_repair/mod.rs`
- Modify: `src/product/plan_repair/tests.rs`

**Interfaces:**
- Consumes: P2 `CanonicalWorkItemContract`、`DependencyContractEdge`
- Produces: `compute_contract_delta`、`ContractImpactAnalyzer::analyze_static`

- [ ] **Step 1: 写当前案例和 Breaking Delta 测试**

```rust
#[test]
fn contract_delta_classifies_added_finalization_capabilities_as_compatible_extension() {
    let previous = provider_contract_fixture(&["workflow_explicit_completion"]);
    let next = provider_contract_fixture(&[
        "workflow_explicit_completion",
        "finalization_failure",
        "failure_message",
    ]);

    let delta = compute_contract_delta(
        "work_item_revision_0001",
        &previous,
        "work_item_revision_0002",
        &next,
    );

    assert_eq!(delta.kind, ContractDeltaKind::CompatibleContractExtension);
    assert_eq!(
        delta.added_capabilities,
        vec!["failure_message", "finalization_failure"]
    );
}

#[test]
fn contract_impact_marks_only_consumers_of_removed_capability_stale() {
    let graph = dependency_graph_fixture();
    let delta = breaking_delta_fixture("contract_x", "capability_removed");

    let report = ContractImpactAnalyzer::default()
        .analyze_static(&graph, &delta, &execution_state_fixture())
        .unwrap();

    assert_eq!(report.direct_stale, vec!["wi_consumer_x"]);
    assert_eq!(report.unaffected, vec!["wi_unrelated"]);
    assert_eq!(report.conditional_downstream, vec!["wi_after_consumer_x"]);
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib contract_delta_
cargo test --locked --lib contract_impact_
```

Expected: FAIL，Delta/Analyzer 尚不存在。

- [ ] **Step 3: 实现 Delta 和 Analyzer**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractDeltaKind {
    InformativeOnly,
    ImplementationGuidance,
    CompatibleContractExtension,
    BreakingContractChange,
    TopologyChange,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractDelta {
    pub logical_work_item_id: String,
    pub previous_revision_id: String,
    pub next_revision_id: String,
    pub kind: ContractDeltaKind,
    pub added_contracts: Vec<String>,
    pub removed_contracts: Vec<String>,
    pub added_capabilities: Vec<String>,
    pub removed_capabilities: Vec<String>,
    pub changed_capabilities: Vec<String>,
    pub acceptance_changed: bool,
    pub verification_changed: bool,
    pub write_policy_changed: bool,
}

pub fn compute_contract_delta(
    previous_revision_id: &str,
    previous: &CanonicalWorkItemContract,
    next_revision_id: &str,
    next: &CanonicalWorkItemContract,
) -> ContractDelta;
```

静态分析状态和结果类型必须完整定义为：

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitExecutionSnapshot {
    pub logical_work_item_id: String,
    pub work_item_revision_id: String,
    pub completed_handoff_revision_id: Option<String>,
    pub has_started: bool,
    pub has_completed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanExecutionState {
    pub units: BTreeMap<String, UnitExecutionSnapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImpactExplanationPath {
    pub from: String,
    pub to: String,
    pub contract_id: String,
    pub capability_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractImpactReport {
    pub unaffected: Vec<String>,
    pub direct_revalidation: Vec<String>,
    pub direct_stale: Vec<String>,
    pub conditional_downstream: Vec<String>,
    pub explanation_paths: Vec<ImpactExplanationPath>,
}

pub fn analyze_static(
    &self,
    graph: &DependencyContractGraph,
    delta: &ContractDelta,
    execution: &PlanExecutionState,
) -> Result<ContractImpactReport, PlanRepairError>;
```

实现只沿消费变化 Contract/Capability 的边传播；`InformativeOnly` 不创建执行影响，`CompatibleContractExtension` 只重验证显式需要新增 Capability 的消费者，`BreakingContractChange` 将已执行直接消费者标记 stale，`TopologyChange` 交给 P6 Subgraph Replanner。

- [ ] **Step 4: 运行 Delta/Impact 测试**

Run:

```bash
cargo test --locked --lib contract_delta_
cargo test --locked --lib contract_impact_
```

Expected: Compatible、Breaking、Informative、Topology 和条件传播测试通过。

- [ ] **Step 5: 提交**

```bash
git add src/product/plan_repair
git commit -m "feat(plan-repair): analyze contract deltas and impact"
```

### Task 3: Work Item Repair Child Session 与一次确认

**Files:**
- Create: `src/product/models/workspace_link.rs`
- Modify: `src/product/models/mod.rs`
- Create: `src/product/workspace_engine/plan_repair.rs`
- Modify: `src/product/workspace_engine/mod.rs`
- Modify: `src/product/workspace_engine/types.rs`
- Modify: `src/product/workspace_engine/lifecycle.rs`
- Modify: `src/product/workspace_engine/session_state.rs`
- Modify: `src/product/workspace_engine/tests.rs`
- Create: `src/product/workspace_engine/tests/part_21.rs`
- Modify: `src/web/workspace_ws_types/in_.rs`
- Modify: `src/web/workspace_ws_types/out.rs`
- Modify: `src/web/workspace_ws_types/timeline.rs`
- Modify: `src/web/workspace_ws_handler/decisions/inbound.rs`
- Modify: `src/web/workspace_ws_handler/tests.rs`

**Interfaces:**
- Produces: `WorkspaceSessionLink`、Repair Session State、`ConfirmPlanAmendment`、`CancelPlanAmendment`
- Consumed by: P5/P6

- [ ] **Step 1: 写 Child Session 和恢复测试**

```rust
#[tokio::test]
async fn plan_repair_creates_linked_work_item_child_session_without_changing_parent_route() {
    let mut engine = make_coding_triggered_plan_repair_engine();

    let session = engine
        .start_plan_repair(repair_request_fixture())
        .await
        .unwrap();

    assert_eq!(session.workspace_type, WorkspaceType::WorkItemPlan);
    let link = engine.workspace_repository().get_session_link(&session.id).unwrap();
    assert_eq!(link.parent_session_id, "coding_attempt_0001");
    assert_eq!(link.relation, WorkspaceSessionRelation::PlanRepair);
    assert_eq!(
        link.return_context.original_unit_run_id,
        "coding_unit_run_0002"
    );
}

#[tokio::test]
async fn plan_repair_refresh_restores_awaiting_confirmation_state() {
    let engine = make_plan_repair_engine_at_confirmation();
    let snapshot = engine.session_state().unwrap();

    assert_eq!(snapshot.stage, WorkspaceStage::HumanConfirm);
    assert!(snapshot.pending_plan_amendment.is_some());
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib plan_repair_child_session_
cargo test --locked --lib plan_repair_refresh_
```

Expected: FAIL，Session Link 和 Repair State 尚不存在。

- [ ] **Step 3: 实现 Session Link 和 Repair Stage**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSessionLink {
    pub id: String,
    pub relation: WorkspaceSessionRelation,
    pub parent_session_id: String,
    pub child_session_id: String,
    pub trigger: WorkspaceSessionLinkTrigger,
    pub return_context: WorkspaceReturnContext,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSessionLinkTrigger {
    pub attempt_id: String,
    pub unit_run_id: String,
    pub review_id: Option<String>,
    pub finding_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceReturnContext {
    pub original_attempt_id: String,
    pub original_unit_run_id: String,
    pub timeline_anchor_id: String,
    pub original_route: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceSessionRelation {
    PlanRepair,
    StoryAmendment,
    DesignAmendment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanRepairSessionStage {
    Triaging,
    AuthoringRevision,
    ValidatingContract,
    GeneratingProjections,
    PlanReview,
    AwaitingConfirmation,
    Published,
    AmendmentConflict,
    ApplyingAmendment,
    AmendmentApplyFailed,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRepairSessionSnapshotDto {
    pub request: PlanRepairRequest,
    pub link: WorkspaceSessionLink,
    pub stage: PlanRepairSessionStage,
    pub projection: Option<PlanProjectionBundle>,
    pub amendment: Option<PlanAmendmentManifest>,
    pub timeline_nodes: Vec<TimelineNode>,
    pub error: Option<String>,
}
```

`WorkspaceEngine::start_plan_repair` 必须：

1. 按 Fingerprint 复用现有 Open Request。
2. 分配 Amendment ID，调用 `acquire_active_amendment`；已有其他 Active Amendment 时返回该 Session，不创建第二个。
3. 把 Amendment ID 回写到 PlanRepairRequest，并创建 Work Item Plan Child Session。
4. 持久化 Session Link 和 Return Context。
5. 进入 `authoring_revision`。
6. 生成 Contract/Projection/Impact。
7. Plan Review 通过后只进入一次 `awaiting_confirmation`。

`CancelPlanAmendment` 只允许在 `PlanPublished` 之前执行：把 Request 标记 `Cancelled`、持久化取消 Timeline 节点并调用 `release_active_amendment`；已发布 Amendment 只能继续幂等应用或进入人工恢复，不能“取消”成旧 Plan Binding。

- [ ] **Step 4: 运行共享 Workspace 测试**

Run:

```bash
cargo test --locked --lib plan_repair_
cargo test --locked --lib workspace_session_link_
cargo test --locked --lib workspace_artifact_
```

Expected: Repair Session 恢复通过；Story/Design 普通 Session 不产生 Plan Repair Link。

- [ ] **Step 5: 提交**

```bash
git add src/product/models/workspace_link.rs src/product/models/mod.rs src/product/workspace_engine src/web/workspace_ws_types src/web/workspace_ws_handler
git commit -m "feat(plan-repair): run linked work item repair sessions"
```

### Task 4: Amendment Prepare、Plan Review 与发布

**Files:**
- Create: `src/product/plan_repair/engine.rs`
- Modify: `src/product/plan_repair/mod.rs`
- Modify: `src/product/workspace_engine/plan_repair.rs`
- Modify: `src/product/work_item_revision_store/repair.rs`
- Modify: `src/product/workspace_engine/tests/part_21.rs`
- Modify: `src/web/workspace_ws_types/artifact.rs`

**Interfaces:**
- Produces: `PreparedPlanAmendment`、`PlanAmendmentManifest`
- Consumed by: P5 `apply_plan_amendment`

- [ ] **Step 1: 写单节点 Amendment 和并发冲突测试**

```rust
#[test]
fn plan_repair_prepares_upstream_only_amendment_for_compatible_extension() {
    let engine = plan_repair_engine_fixture();
    let prepared = engine
        .prepare_amendment(&upstream_contract_request_fixture())
        .unwrap();

    assert_eq!(
        prepared.manifest.revised_work_items.keys().collect::<Vec<_>>(),
        vec![&"wi_core".to_string()]
    );
    assert_eq!(
        prepared.manifest.revalidation_required_units,
        vec!["wi_registration"]
    );
    assert!(prepared.manifest.stale_units.is_empty());
}

#[test]
fn plan_repair_publish_rejects_changed_base_revision() {
    let engine = plan_repair_engine_fixture();
    let prepared = engine
        .prepare_amendment(&upstream_contract_request_fixture())
        .unwrap();
    engine.force_active_plan_revision("plan_revision_0002");

    let error = engine
        .publish_amendment(prepared, confirmation_fixture())
        .unwrap_err();
    assert!(matches!(error, PlanRepairError::AmendmentConflict { .. }));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib plan_repair_prepares_
cargo test --locked --lib plan_repair_publish_
```

Expected: FAIL，Amendment Engine 尚不存在。

- [ ] **Step 3: 实现 Prepare 和 Publish**

```rust
pub struct PreparedPlanAmendment {
    pub base_plan_revision_id: String,
    pub next_plan_revision: WorkItemPlanRevision,
    pub draft_revisions: Vec<WorkItemDraftRevision>,
    pub revised_work_items: Vec<WorkItemRevision>,
    pub work_item_projection_bundles: Vec<WorkItemProjectionBundle>,
    pub plan_projection_bundle: PlanProjectionBundle,
    pub validation_report: PlanValidationReportArtifact,
    pub contract_deltas: Vec<ContractDelta>,
    pub impact_report: ContractImpactReport,
    pub manifest: PlanAmendmentManifest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyGraphChangeKind {
    EdgeAdded,
    EdgeRemoved,
    EdgeReplaced,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyGraphChange {
    pub kind: DependencyGraphChangeKind,
    pub previous: Option<DependencyContractEdge>,
    pub next: Option<DependencyContractEdge>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanAmendmentConfirmation {
    pub amendment_id: String,
    pub base_plan_revision_id: String,
    pub accepted_impact_scope: Vec<String>,
    pub risk_acceptance_reason: Option<String>,
    pub confirmed_by: String,
    pub confirmed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemRevisionReplacement {
    pub previous_revision_id: String,
    pub next_revision_id: String,
    pub delta_kind: ContractDeltaKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanAmendmentManifest {
    pub id: String,
    pub repair_request_id: String,
    pub previous_plan_revision_id: String,
    pub new_plan_revision_id: String,
    pub revised_work_items: BTreeMap<String, WorkItemRevisionReplacement>,
    pub superseded_revisions: Vec<String>,
    pub dependency_graph_changes: Vec<DependencyGraphChange>,
    pub contract_deltas: Vec<ContractDelta>,
    pub unaffected_units: Vec<String>,
    pub revalidation_required_units: Vec<String>,
    pub stale_units: Vec<String>,
    pub replacement_units: BTreeMap<String, Vec<String>>,
    pub resume_target: AmendmentResumeTarget,
    pub created_at: String,
}

impl PlanRepairEngine {
    pub fn prepare_amendment(
        &self,
        request: &PlanRepairRequest,
    ) -> Result<PreparedPlanAmendment, PlanRepairError>;

    pub fn publish_amendment(
        &self,
        prepared: PreparedPlanAmendment,
        confirmation: PlanAmendmentConfirmation,
    ) -> Result<PlanAmendmentManifest, PlanRepairError>;
}
```

P1 中 `WorkItemRevisionReplacement.delta_kind`、`PlanAmendmentManifest.dependency_graph_changes` 和 `contract_deltas` 在本 Task 替换为上述强类型，之后不得再序列化成任意 JSON。`publish_amendment` 必须校验 Confirmation 的 Amendment/Base ID；Breaking/Topology 变化缺少 Confirmation 时拒绝发布，用户缩小静态最小影响集时必须提供非空 `risk_acceptance_reason` 并重新通过 Plan Review。

发布顺序必须是：验证当前 Session 仍持有 Active Amendment Lock → 写全部不可变 Artifact → 写 `PlanAmendmentPublicationPhase::Prepared` → CAS 校验 Base → 更新 Active Plan Revision → 标记 `PlanPublished`。Lock 保持到 P5 Coding Binding 和 Resume Target 全部应用完成后再释放；失败时调用 `mark_plan_amendment_publication_failed`，保留最后成功 Phase、Journal Error 和 Lock 恢复信息，不得更新 Coding Binding。

- [ ] **Step 4: 运行 P4 验证**

Run:

```bash
cargo fmt --check
cargo check --locked
cargo test --locked --lib plan_repair_
cargo test --locked --lib contract_delta_
cargo test --locked --lib contract_impact_
cargo test --locked --lib workspace_session_link_
```

Expected: 所有命令 exit 0，测试运行数大于 0。

- [ ] **Step 5: 提交**

```bash
git add src/product/plan_repair src/product/workspace_engine/plan_repair.rs src/product/work_item_revision_store/repair.rs src/web/workspace_ws_types/artifact.rs
git commit -m "feat(plan-repair): prepare and publish plan amendments"
```
