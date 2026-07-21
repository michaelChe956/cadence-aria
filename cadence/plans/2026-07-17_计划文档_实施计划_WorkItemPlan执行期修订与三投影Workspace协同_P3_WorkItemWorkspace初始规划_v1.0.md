# P3 Work Item Workspace 初始规划 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Work Item Workspace 的 Initial Planning 直接生成 Canonical Contract Candidate、Plan/WorkItem Revision 和三 Projection，并提供人类可理解的 Group Overview、Contract Flow 和 Projection Tabs。

**Architecture:** 保留 Outline → Draft → Review → Compile 的交互骨架，但 Draft 权威内容改为 Canonical Contract Candidate；Final Compile 写入 P1 Revision Store，不再把旧 `LifecycleWorkItemRecord` 单文件作为执行事实源。前端默认展示 Human Projection，原始 JSON 仅作为诊断页签。

**Tech Stack:** Rust Workspace Engine、Work Item Split Engine、WebSocket DTO、React/TypeScript/Vitest。

## Global Constraints

- 不保留旧 `implementation_context` 作为权威字段。
- Group 和 Work Item Projection 必须在 Final Compile 前通过 Validation。
- Plan Reviewer 使用 Plan Review Context，不使用 Code Reviewer Prompt。
- Informative UI 不能写回 Canonical Contract。
- Story/Design 共享 Workspace 协议变更必须有三类型回归。
- 前端使用 pnpm。

---

### Task 1: Work Item Author 输出 Canonical Contract Candidate

**Files:**
- Modify: `src/product/models/outline.rs`
- Modify: `src/product/work_item_split_engine/types.rs`
- Modify: `src/product/work_item_split_engine/schema.rs`
- Modify: `src/product/work_item_split_engine/prompts.rs`
- Modify: `src/product/work_item_split_engine/parse.rs`
- Modify: `src/product/work_item_split_engine/tests.rs`
- Modify: `src/product/work_item_split_engine/tests/part_01.rs`
- Modify: `src/product/work_item_split_validator/draft.rs`
- Modify: `src/product/work_item_split_validator/tests.rs`

**Interfaces:**
- Consumes: P2 `CanonicalWorkItemContract`
- Produces: `WorkItemDraftCandidate.canonical_contract_candidate`

- [ ] **Step 1: 写 Author Parse 和 Validator 失败测试**

```rust
#[test]
fn work_item_plan_author_parses_canonical_contract_candidate() {
    let output = serde_json::json!({
        "outline_id": "outline_core",
        "canonical_contract": canonical_contract_json_fixture("wi_core"),
        "verification_plan": verification_plan_fixture()
    });

    let candidate = parse_work_item_draft_candidate(&output.to_string()).unwrap();

    assert_eq!(
        candidate.canonical_contract_candidate.identity.logical_work_item_id,
        "wi_core"
    );
}

#[test]
fn work_item_plan_draft_validator_rejects_missing_output_provider() {
    let mut candidate = draft_candidate_fixture();
    candidate
        .canonical_contract_candidate
        .input_contracts[0]
        .provider_logical_work_item_id = "wi_missing".to_string();

    let findings = validate_work_item_draft_candidate(&candidate, &outline_fixture());

    assert!(findings.iter().any(|finding| {
        finding.code == "unknown_provider_logical_work_item"
    }));
}

#[test]
fn work_item_plan_draft_validator_rejects_verification_plan_drift() {
    let mut candidate = draft_candidate_fixture();
    candidate.verification_plan.checks[0].check_id = "check_not_in_contract".to_string();

    let findings = validate_work_item_draft_candidate(&candidate, &outline_fixture());

    assert!(findings.iter().any(|finding| {
        finding.code == "verification_plan_not_derived_from_contract"
    }));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib work_item_plan_author_canonical_
cargo test --locked --lib work_item_plan_draft_validator_
```

Expected: FAIL，Draft Candidate 仍使用 `implementation_context`。

- [ ] **Step 3: 修改 Draft Candidate 和 Provider Schema**

将 `WorkItemDraftCandidate` 改为：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemDraftVerificationPlan {
    pub checks: Vec<VerificationCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemDraftCandidate {
    pub outline_id: String,
    pub logical_work_item_id: String,
    pub canonical_contract_candidate: CanonicalWorkItemContract,
    pub verification_plan: WorkItemDraftVerificationPlan,
}
```

Author Prompt 必须要求：

```text
只输出 Canonical Contract Candidate。
所有 input/output contract、task、acceptance、verification、handoff 和 blocker rule 必须有稳定 ID。
不得输出面向 Coder 的长篇 implementation_context。
```

Parser 直接反序列化结构化结果，不从 Markdown 猜测 Contract。
`WorkItemDraftVerificationPlan` 只能是 `canonical_contract_candidate.verification_checks` 的执行视图；Check ID、Command、Required 和 non-zero 要求不一致时返回 `verification_plan_not_derived_from_contract`。

- [ ] **Step 4: 运行 Author/Validator 测试**

Run:

```bash
cargo test --locked --lib work_item_plan_author_canonical_
cargo test --locked --lib work_item_plan_draft_validator_
```

Expected: 新 Draft Schema 和 Contract Validator 测试通过。

- [ ] **Step 5: 提交**

```bash
git add src/product/models/outline.rs src/product/work_item_split_engine src/product/work_item_split_validator
git commit -m "feat(work-item-workspace): author canonical work item drafts"
```

### Task 2: Initial Final Compile 发布 PlanRevision

**Files:**
- Modify: `src/product/workspace_engine/compile.rs`
- Modify: `src/product/workspace_engine/compile_parse.rs`
- Create: `src/product/workspace_engine/plan_projection.rs`
- Modify: `src/product/workspace_engine/mod.rs`
- Modify: `src/product/lifecycle_store/plan.rs`
- Modify: `src/product/lifecycle_store/work_item.rs`
- Modify: `src/product/workspace_engine/tests/part_03/part_08.rs`
- Modify: `src/product/workspace_engine/tests/part_08.rs`

**Interfaces:**
- Consumes: P1 `WorkItemRevisionStore`、P2 Compiler/Validator
- Produces: Initial `WorkItemPlanRevision`、`WorkItemRevision`、Plan/WorkItem ProjectionBundle

- [ ] **Step 1: 写 Initial Compile 测试**

```rust
#[tokio::test]
async fn work_item_plan_initial_compile_publishes_revision_and_projection_bundles() {
    let mut engine = make_work_item_plan_engine_with_accepted_contract_drafts();

    let outcome = engine.run_work_item_plan_compile().await.unwrap();

    let plan = engine
        .revision_store()
        .get_plan_lineage("project_0001", "issue_0001", "issue_plan_0001")
        .unwrap();
    let revision_id = plan.active_revision_id.as_deref().unwrap();
    let revision = engine
        .revision_store()
        .get_plan_revision("project_0001", "issue_0001", &plan.id, revision_id)
        .unwrap();

    assert_eq!(revision.revision_no, 1);
    assert_eq!(revision.work_item_bindings.len(), 2);
    assert!(outcome.projection_validation.is_valid());
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib work_item_plan_initial_compile_
```

Expected: FAIL，Compile 仍创建旧 LifecycleWorkItemRecord。

- [ ] **Step 3: 实现 Revision Compile Transaction**

在现有 compile transaction 的 `committing` 阶段：

```rust
let compiled = accepted_drafts
    .iter()
    .map(|draft| compile_work_item_revision(draft, &projection_compiler))
    .collect::<Result<Vec<_>, _>>()?;

let contracts = compiled
    .iter()
    .map(|item| item.work_item_revision.canonical_contract.clone())
    .collect::<Vec<_>>();
let dependency_graph = build_dependency_contract_graph(&contracts)
    .map_err(WorkspaceEngineError::WorkItemPlanValidation)?;
let dependency_report = validate_dependency_contract_graph(&dependency_graph);
if !dependency_report.is_valid() {
    return Err(WorkspaceEngineError::WorkItemPlanValidation(
        dependency_report.into_findings(),
    ));
}

let projection_map = compiled
    .iter()
    .map(|item| {
        (
            item.work_item_revision.logical_work_item_id.clone(),
            CompiledWorkItemProjections {
                human: item.projection_bundle.human_projection.clone(),
                coder: item.projection_bundle.coder_projection.clone(),
                reviewer: item.projection_bundle.reviewer_projection.clone(),
            },
        )
    })
    .collect::<BTreeMap<_, _>>();
let plan_projection = compile_plan_projection_bundle(
    &plan_revision_id,
    &dependency_graph_revision_id,
    plan_projection_input(&outline, &dependency_graph, &projection_map),
    &plan_projection_compiler,
    &compiled,
)?;
let plan_revision = publish_initial_plan_revision(
    &revision_store,
    &session,
    compiled,
    plan_projection,
)?;
```

Compile helper 必须使用以下签名，避免执行者在 Workspace Engine、Projection Compiler 和 Store 之间临时发明返回结构：

```rust
pub struct CompiledWorkItemRevision {
    pub draft_revision: WorkItemDraftRevision,
    pub work_item_revision: WorkItemRevision,
    pub verification_plan_revision: VerificationPlanRevision,
    pub projection_bundle: WorkItemProjectionBundle,
}

pub struct InitialPlanCompileOutcome {
    pub plan_revision: WorkItemPlanRevision,
    pub dependency_graph_revision: DependencyGraphRevision,
    pub validation_report: PlanValidationReportArtifact,
    pub plan_projection_bundle: PlanProjectionBundle,
    pub work_items: Vec<CompiledWorkItemRevision>,
    pub contract_validation: ContractValidationReport,
    pub projection_validation: ProjectionValidationReport,
}

pub fn compile_work_item_revision(
    draft: &WorkItemDraftRevision,
    projection_compiler: &WorkItemProjectionCompiler,
) -> Result<CompiledWorkItemRevision, WorkspaceEngineError>;

pub fn compile_plan_projection_bundle(
    plan_revision_id: &str,
    dependency_graph_revision_id: &str,
    input: PlanProjectionCompileInput<'_>,
    projection_compiler: &PlanProjectionCompiler,
    work_items: &[CompiledWorkItemRevision],
) -> Result<PlanProjectionBundle, WorkspaceEngineError>;

pub fn plan_projection_input<'a>(
    outline: &'a WorkItemPlanOutline,
    dependency_graph: &'a DependencyContractGraph,
    work_item_projections: &'a BTreeMap<String, CompiledWorkItemProjections>,
) -> PlanProjectionCompileInput<'a>;

pub fn publish_initial_plan_revision(
    store: &WorkItemRevisionStore,
    lineage: &WorkItemPlanLineage,
    plan_revision: WorkItemPlanRevision,
    dependency_graph_revision: DependencyGraphRevision,
    plan_projection_bundle: PlanProjectionBundle,
    work_items: Vec<CompiledWorkItemRevision>,
    contract_validation: ContractValidationReport,
    projection_validation: ProjectionValidationReport,
) -> Result<InitialPlanCompileOutcome, WorkspaceEngineError>;

impl WorkspaceEngine {
    pub fn compile_initial_plan_revision(
        &mut self,
        accepted_drafts: &[WorkItemDraftRevision],
    ) -> Result<InitialPlanCompileOutcome, WorkspaceEngineError>;
}
```

`compile_initial_plan_revision` 先调用 `build_dependency_contract_graph` 和 `validate_dependency_contract_graph(&graph)`，再汇总每个 Work Item 的 `validate_projection_coverage` 与 Plan 级 `validate_plan_projection_coverage`；只有 Contract 与 Projection 两份 Report 都有效才创建 `PlanValidationReportArtifact` 并调用 `publish_initial_plan_revision`。`LifecycleStore::commit_issue_work_item_plan` 改为只更新 Lifecycle Summary 指针，不再创建旧 Work Item JSON 作为执行事实源。

`compile_work_item_revision` 必须先由 Store 分配 `work_item_revision_id`，再调用 `WorkItemProjectionCompiler::compile(&draft.canonical_contract_candidate, &work_item_revision_id)`；Physical Revision ID 不允许由 Author Provider 写入 Canonical Contract Candidate。

- [ ] **Step 4: 运行 Compile 测试**

Run:

```bash
cargo test --locked --lib work_item_plan_initial_compile_
cargo test --locked --lib work_item_plan_compile_
```

Expected: Initial Revision、Projection、事务恢复测试通过。

- [ ] **Step 5: 提交**

```bash
git add src/product/workspace_engine src/product/lifecycle_store
git commit -m "feat(work-item-workspace): publish initial plan revision"
```

### Task 3: WebSocket Artifact 与 Plan Review Context

**Files:**
- Modify: `src/web/workspace_ws_types/artifact.rs`
- Modify: `src/web/workspace_ws_types/artifact_version.rs`
- Modify: `src/web/workspace_ws_types/out.rs`
- Modify: `src/web/workspace_ws_types/tests.rs`
- Modify: `src/product/workspace_engine/session_state.rs`
- Modify: `src/product/workspace_engine/prompts/review.rs`
- Modify: `src/product/workspace_engine/review/structured_output.rs`
- Modify: `src/web/workspace_ws_handler/run.rs`
- Modify: `src/web/workspace_ws_handler/mapping.rs`
- Modify: `src/web/workspace_ws_handler/tests.rs`

**Interfaces:**
- Produces: Plan/WorkItem Projection Artifact DTO、Plan Review Context
- Consumed by: frontend Workspace store

- [ ] **Step 1: 写 DTO Roundtrip 和 Review Prompt 测试**

```rust
#[test]
fn work_item_plan_projection_artifact_roundtrips() {
    let payload = ArtifactPayload::WorkItemPlanProjection {
        projection: Box::new(plan_projection_bundle_dto_fixture()),
    };

    let value = serde_json::to_value(&payload).unwrap();
    assert_eq!(value["type"], "work_item_plan_projection");
    assert_eq!(serde_json::from_value::<ArtifactPayload>(value).unwrap(), payload);
}

#[test]
fn work_item_plan_reviewer_prompt_contains_projection_validation_and_contract_flow() {
    let engine = make_work_item_plan_engine_with_projection_candidate();
    let input = engine.build_work_item_plan_review_input().unwrap();

    assert!(input.prompt.contains("Projection Validation Report"));
    assert!(input.prompt.contains("Dependency Contract Graph"));
    assert!(input.prompt.contains("PlanProjectionBundle Candidate"));
}

#[test]
fn workspace_artifact_version_binding_remains_stable_for_story_design_and_work_item() {
    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
    ] {
        let snapshot = workspace_snapshot_with_bound_artifact(workspace_type.clone());
        let restored = roundtrip_workspace_snapshot(snapshot.clone());

        assert_eq!(restored.workspace_type, workspace_type);
        assert_eq!(restored.artifact_version_id, snapshot.artifact_version_id);
        assert_eq!(restored.selected_timeline_node_id, snapshot.selected_timeline_node_id);
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib work_item_plan_projection_artifact_
cargo test --locked --lib work_item_plan_reviewer_prompt_
```

Expected: FAIL，Artifact 和 Review Context 尚未接入。

- [ ] **Step 3: 扩展 ArtifactPayload**

```rust
#[serde(untagged)]
pub enum ArtifactPayload {
    Markdown {
        markdown: String,
        diff: Option<String>,
    },
    WorkItemPlanCandidate {
        candidate: Box<WorkItemPlanCandidateDto>,
    },
    WorkItemPlanOutlineCandidate {
        outline_candidate: Box<WorkItemPlanOutlineCandidateDto>,
    },
    WorkItemPlanContextBlocker {
        context_blocker: Box<WorkItemPlanContextBlockerPayload>,
    },
    WorkItemDraftCandidate {
        draft_candidate: Box<WorkItemDraftCandidatePayload>,
    },
    WorkItemBatchState {
        batch_state: Box<WorkItemBatchStatePayload>,
    },
    WorkItemPlanCompileReport {
        compile_report: Box<WorkItemPlanCompileReportPayload>,
    },
    WorkItemPlanProjection {
        projection: Box<PlanProjectionBundleDto>,
    },
    WorkItemProjection {
        projection: Box<WorkItemProjectionBundleDto>,
    },
    WorkItemRevisionHistory {
        history: Box<WorkItemRevisionHistoryDto>,
    },
    ProjectionValidation {
        report: Box<ProjectionValidationReportDto>,
    },
}
```

Session Snapshot 必须恢复当前 Projection Artifact、Contract Hash 和 Compiler Version。Plan Reviewer Prompt 使用 Plan Review Context，不复用 Coding Reviewer Projection。
`ArtifactPayload::markdown`、`into_markdown` 和所有 exhaustive match 必须为四个新结构化 Variant 返回 `None`，避免新增 Variant 后遗漏编译分支。

DTO 不复制领域字段，直接使用可序列化类型别名，避免后端 Domain/WS 两套字段漂移：

```rust
pub type PlanProjectionBundleDto = PlanProjectionBundle;
pub type WorkItemProjectionBundleDto = WorkItemProjectionBundle;
pub type ProjectionValidationReportDto = ProjectionValidationReport;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemHistoryEntryKind {
    DraftRevision,
    WorkItemRevision,
    PlanReview,
    ContractDelta,
    UnitRun,
    HandoffRevision,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemHistoryEntryDto {
    pub kind: WorkItemHistoryEntryKind,
    pub id: String,
    pub logical_work_item_id: String,
    pub related_revision_id: Option<String>,
    pub summary: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemRevisionHistoryDto {
    pub entries: Vec<WorkItemHistoryEntryDto>,
}
```

- [ ] **Step 4: 运行共享 Workspace 协议测试**

Run:

```bash
cargo test --locked --lib work_item_plan_projection_artifact_
cargo test --locked --lib work_item_plan_reviewer_prompt_
cargo test --locked --lib workspace_artifact_version_
```

Expected: Work Item Plan 新 Artifact 通过；Story、Design、Work Item 的 Artifact Version Binding、Timeline 定位和 Snapshot 恢复表驱动测试全部通过。

- [ ] **Step 5: 提交**

```bash
git add src/web/workspace_ws_types src/product/workspace_engine src/web/workspace_ws_handler
git commit -m "feat(work-item-workspace): expose projection artifacts"
```

### Task 4: Human Group UI 与三 Projection Tabs

**Files:**
- Modify: `web/src/api/types/work-item-plan.ts`
- Create: `web/src/components/workspace/WorkItemPlanOverview.tsx`
- Create: `web/src/components/workspace/WorkItemContractFlow.tsx`
- Create: `web/src/components/workspace/WorkItemProjectionTabs.tsx`
- Create: `web/src/components/workspace/WorkItemPlanOverview.test.tsx`
- Create: `web/src/components/workspace/WorkItemContractFlow.test.tsx`
- Create: `web/src/components/workspace/WorkItemProjectionTabs.test.tsx`
- Modify: `web/src/components/workspace/WorkItemPlanArtifactContent.tsx`
- Modify: `web/src/components/workspace/WorkItemPlanArtifactPanel.tsx`
- Modify: `web/src/pages/ChatWorkspacePage.work-item-plan.test.tsx`
- Modify: `web/src/state/workspace-ws-store.artifacts.test.ts`

**Interfaces:**
- Consumes: Task 3 DTO
- Produces: Human Overview、Contract Flow、Human/Coder/Reviewer Tabs

- [ ] **Step 1: 写人类默认视图测试**

```tsx
it("shows human projection without exposing coder prompt by default", () => {
  render(
    <WorkItemPlanOverview
      projection={planProjectionFixture()}
      presentation={null}
    />,
  );

  expect(screen.getByText("仓库初始化实时进度")).toBeInTheDocument();
  expect(screen.getByText("WI-01 初始化领域模型")).toBeInTheDocument();
  expect(screen.getByText("提供 finalization contract")).toBeInTheDocument();
  expect(screen.queryByText("Coder 执行协议")).not.toBeInTheDocument();
});

it("shows a missing capability on the contract edge", () => {
  render(<WorkItemContractFlow projection={contractMismatchFixture()} />);
  expect(screen.getByText("缺少 failure_message")).toBeInTheDocument();
});
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cd web
pnpm test -- WorkItemPlanOverview WorkItemContractFlow WorkItemProjectionTabs
```

Expected: FAIL，新组件不存在。

- [ ] **Step 3: 实现组件和类型**

```ts
export type WorkItemProjectionTab = "overview" | "contract" | "coder" | "reviewer" | "history";

export type ContractCompatibilityPolicy = "require_all" | "require_any";

export type RequiredDependencyContract = {
  contract_id: string;
  required_capabilities: string[];
  compatibility_policy: ContractCompatibilityPolicy;
};

export type DependencyContractEdge = {
  from: string;
  to: string;
  required_contracts: RequiredDependencyContract[];
};

export type WorkItemWritePolicy = {
  exclusive_scopes: string[];
  forbidden_scopes: string[];
};

export type HumanScopeSummary = {
  owned_scopes: string[];
  forbidden_scopes: string[];
};

export type HumanGroupProjection = {
  plan_id: string;
  goal: string;
  split_reason: string;
  work_items: Array<{
    logical_work_item_id: string;
    title: string;
    goal: string;
    depends_on: string[];
    provides: string[];
    scope_summary: HumanScopeSummary;
  }>;
  contract_flow: Array<{
    from: string;
    to: string;
    contract_id: string;
    required_capabilities: string[];
    provided_capabilities: string[];
    missing_capabilities: string[];
  }>;
  risks: string[];
  source_refs: string[];
  normative: false;
  used_by_provider: false;
};

export type CoderGroupContext = {
  plan_id: string;
  ordered_logical_work_item_ids: string[];
  dependency_edges: DependencyContractEdge[];
  group_write_scopes: Record<string, WorkItemWritePolicy>;
};

export type ReviewerGroupMatrix = {
  plan_id: string;
  work_items: Array<{
    logical_work_item_id: string;
    criterion_refs: string[];
    input_contract_refs: string[];
    output_contract_refs: string[];
  }>;
  dependency_edges: DependencyContractEdge[];
  design_traceability_refs: Array<{
    source_type: string;
    source_id: string;
    requirement_id: string;
  }>;
};

export type RequiredInputContract = {
  contract_id: string;
  provider_logical_work_item_id: string;
  required_capabilities: string[];
  compatibility_policy: ContractCompatibilityPolicy;
};

export type PromisedOutputContract = {
  contract_id: string;
  capabilities: string[];
};

export type WorkItemTask = {
  task_id: string;
  statement: string;
  requirement_refs: string[];
  done_when_refs: string[];
};

export type EvidenceKind =
  | "source_diff"
  | "non_zero_test_execution"
  | "manual_check"
  | "handoff_field";

export type AcceptanceCriterion = {
  criterion_id: string;
  statement: string;
  required_evidence: EvidenceKind[];
};

export type VerificationCheck = {
  check_id: string;
  command: string | null;
  manual_instruction: string | null;
  required: boolean;
  non_zero_test_execution_required: boolean;
};

export type BlockerRoute =
  | "coder_rework"
  | "verification_retry"
  | "plan_repair_current"
  | "plan_repair_upstream"
  | "subgraph_replan"
  | "story_amendment"
  | "design_amendment"
  | "operational_gate";

export type BlockerRule = {
  reason_code: string;
  route: BlockerRoute;
  target_contract_refs: string[];
};

export type HandoffContract = {
  required_fields: string[];
  provided_contract_refs: string[];
  reviewer_check_refs: string[];
};

export type HumanWorkItemProjection = {
  logical_work_item_id: string;
  title: string;
  goal: string;
  non_goals: string[];
  inputs: Array<{ contract_id: string; capabilities: string[]; source_refs: string[] }>;
  outputs: Array<{ contract_id: string; capabilities: string[]; source_refs: string[] }>;
  dependencies: string[];
  scope_summary: HumanScopeSummary;
  completion_summary: string[];
  source_refs: string[];
  normative: false;
  used_by_provider: false;
};

export type CoderWorkItemProjection = {
  work_item_revision_id: string;
  objective: string;
  required_input_contracts: RequiredInputContract[];
  task_refs: string[];
  tasks: WorkItemTask[];
  write_policy: WorkItemWritePolicy;
  acceptance_criteria: AcceptanceCriterion[];
  verification_checks: VerificationCheck[];
  blocker_rules: BlockerRule[];
  handoff_contract: HandoffContract;
};

export type ReviewerWorkItemProjection = {
  work_item_revision_id: string;
  criterion_refs: string[];
  requirement_matrix: Array<{
    criterion_id: string;
    requirement_refs: string[];
    required_evidence: EvidenceKind[];
    failure_route: BlockerRoute;
  }>;
  scope_policy: WorkItemWritePolicy;
  input_contract_checks: RequiredInputContract[];
  output_contract_checks: PromisedOutputContract[];
  verification_evidence_rules: VerificationCheck[];
  blocker_routing: BlockerRule[];
};

export type WorkItemProjectionBundle = {
  id: string;
  work_item_revision_id: string;
  canonical_contract_hash: string;
  projection_schema_version: number;
  compiler_version: string;
  human_projection: HumanWorkItemProjection;
  coder_projection: CoderWorkItemProjection;
  reviewer_projection: ReviewerWorkItemProjection;
  human_projection_hash: string;
  coder_projection_hash: string;
  reviewer_projection_hash: string;
  created_at: string;
};

export type WorkItemHistoryEntry = {
  kind:
    | "draft_revision"
    | "work_item_revision"
    | "plan_review"
    | "contract_delta"
    | "unit_run"
    | "handoff_revision";
  id: string;
  logical_work_item_id: string;
  related_revision_id: string | null;
  summary: string;
  created_at: string;
};

export type PlanProjectionBundle = {
  id: string;
  plan_revision_id: string;
  dependency_graph_revision_id: string;
  work_item_projection_bundle_refs: string[];
  human_group_projection: HumanGroupProjection;
  coder_group_context: CoderGroupContext;
  reviewer_group_matrix: ReviewerGroupMatrix;
  human_group_projection_hash: string;
  coder_group_context_hash: string;
  reviewer_group_matrix_hash: string;
  compiler_version: string;
  created_at: string;
};
```

`WorkItemProjectionTabs` 默认 `overview`，Coder/Reviewer 内容只有用户显式切换后渲染。Coder View 同时提供 Provider-neutral 内容和调用 P2 Renderer 生成的 Codex/Claude Code/Fake 只读预览；Reviewer View 展示验证矩阵与失败路由。Contract Flow 显示 required/provided capability 的交集和缺口。History 使用 `WorkItemRevisionHistoryDto`，按时间展示 Draft/WorkItem Revision、Plan Review 和 Delta；P5 接入后追加 UnitRun/Handoff，不允许用模拟文本代替真实 Artifact 引用。

- [ ] **Step 4: 运行 Human UI 定向验证**

Run:

```bash
cargo fmt --check
cargo check --locked
cargo test --locked --lib work_item_plan_
cd web
pnpm tsc -b
pnpm test -- WorkItemPlanOverview WorkItemContractFlow WorkItemProjectionTabs ChatWorkspacePage.work-item-plan
```

Expected: Rust 和前端测试通过，测试运行数大于 0。

- [ ] **Step 5: 提交**

```bash
git add web/src/api/types/work-item-plan.ts web/src/components/workspace web/src/pages/ChatWorkspacePage.work-item-plan.test.tsx web/src/state/workspace-ws-store.artifacts.test.ts
git commit -m "feat(work-item-workspace): add human plan projections"
```

### Task 5: Informative Human Presentation Revision

**Files:**
- Create: `src/product/workspace_engine/human_presentation.rs`
- Modify: `src/product/workspace_engine/mod.rs`
- Modify: `src/product/work_item_revision_store/presentation.rs`
- Modify: `src/web/workspace_ws_types/in_.rs`
- Modify: `src/web/workspace_ws_types/out.rs`
- Modify: `src/web/workspace_ws_handler/decisions/inbound.rs`
- Modify: `src/product/workspace_engine/tests/part_20.rs`
- Modify: `src/product/workspace_engine/tests.rs`
- Create: `web/src/components/workspace/HumanPresentationEditor.tsx`
- Create: `web/src/components/workspace/HumanPresentationEditor.test.tsx`
- Modify: `web/src/components/workspace/WorkItemPlanOverview.tsx`
- Modify: `web/src/components/workspace/WorkItemProjectionTabs.tsx`
- Modify: `web/src/state/workspace-ws-store.ts`
- Modify: `web/src/api/types/work-item-plan.ts`

**Interfaces:**
- Consumes: P1 `HumanPresentationRevision`、P2 `HumanPresentationBase` Validator
- Produces: `SaveHumanPresentationRevision`、`save_human_presentation_revision`

- [ ] **Step 1: 写 Informative Edit 不改变 Provider 输入的失败测试**

```rust
#[test]
fn work_item_human_presentation_revision_changes_only_human_rendering() {
    let fixture = published_plan_projection_fixture();
    let before = fixture.provider_hashes();

    let saved = save_human_presentation_revision(
        &fixture.store,
        &fixture.plan,
        fixture.presentation_revision("更容易理解的拆分说明"),
    )
    .unwrap();

    assert_eq!(saved.supersedes, None);
    assert_eq!(fixture.provider_hashes(), before);
    assert_eq!(fixture.active_plan_revision_id(), "plan_revision_0001");
    assert_eq!(fixture.active_work_item_revision_id("wi_core"), "work_item_revision_0001");
}
```

```tsx
it("saves an informative explanation without exposing normative controls", async () => {
  const user = userEvent.setup();
  render(<HumanPresentationEditor base={humanProjectionFixture()} />);

  await user.clear(screen.getByLabelText("拆分说明"));
  await user.type(screen.getByLabelText("拆分说明"), "先稳定核心状态机，再接 API");
  await user.click(screen.getByRole("button", { name: "保存说明" }));

  expect(mockSend).toHaveBeenCalledWith(expect.objectContaining({
    type: "save_human_presentation_revision",
  }));
  expect(screen.queryByLabelText("修改 Coder Contract")).not.toBeInTheDocument();
});
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib work_item_human_presentation_
cd web
pnpm test -- HumanPresentationEditor
```

Expected: FAIL，Workspace 还没有 Presentation Revision 命令和编辑组件。

- [ ] **Step 3: 实现保存命令、Validator 和叠加渲染**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HumanPresentationScopeDto {
    Plan,
    WorkItem,
}

pub struct SaveHumanPresentationRevision {
    pub source_projection_bundle_id: String,
    pub scope: HumanPresentationScopeDto,
    pub supersedes: Option<String>,
    pub human_summary: String,
    pub why_split: Option<String>,
    pub dependency_explanation: Vec<String>,
    pub risk_explanation: Vec<String>,
    pub source_refs: Vec<String>,
}

pub fn save_human_presentation_revision(
    store: &WorkItemRevisionStore,
    plan: &WorkItemPlanLineage,
    mut revision: HumanPresentationRevision,
) -> Result<HumanPresentationRevision, WorkspaceEngineError> {
    revision.normative = false;
    revision.used_by_provider = false;
    match (
        revision.source_plan_projection_bundle_id.as_deref(),
        revision.source_work_item_projection_bundle_id.as_deref(),
    ) {
        (Some(bundle_id), None) => {
            let bundle = store.get_plan_projection_bundle(plan, bundle_id)?;
            validate_human_presentation_revision(
                HumanPresentationBase::Plan(&bundle.human_group_projection),
                &revision,
            )
            .map_err(WorkspaceEngineError::ProjectionCompile)?;
        }
        (None, Some(bundle_id)) => {
            let bundle = store.get_work_item_projection_bundle(plan, bundle_id)?;
            validate_human_presentation_revision(
                HumanPresentationBase::WorkItem(&bundle.human_projection),
                &revision,
            )
            .map_err(WorkspaceEngineError::ProjectionCompile)?;
        }
        _ => {
            return Err(WorkspaceEngineError::InvalidHumanPresentationTarget);
        }
    }
    store.put_human_presentation_revision(plan, &revision)?;
    Ok(revision)
}
```

创建 Revision 时后端强制写入 `normative = false`、`used_by_provider = false`，并校验 `supersedes` 等于当前最新 Presentation Revision。前端读取时按“不可变 Base Human Projection + 最新 Presentation Revision”叠加；不得更新 PlanRevision、WorkItemRevision、Coder/Reviewer Hash，也不得把 Presentation 传入 Provider Renderer。

前端类型与 WS 命令固定为：

```ts
export type HumanPresentationRevision = {
  id: string;
  source_plan_projection_bundle_id: string | null;
  source_work_item_projection_bundle_id: string | null;
  supersedes: string | null;
  human_summary: string;
  why_split: string | null;
  dependency_explanation: string[];
  risk_explanation: string[];
  source_refs: string[];
  normative: false;
  used_by_provider: false;
  created_at: string;
};

export type SaveHumanPresentationRevisionMessage = {
  type: "save_human_presentation_revision";
  source_projection_bundle_id: string;
  scope: "plan" | "work_item";
  supersedes: string | null;
  human_summary: string;
  why_split: string | null;
  dependency_explanation: string[];
  risk_explanation: string[];
  source_refs: string[];
};
```

- [ ] **Step 4: 运行 P3 全部验证**

Run:

```bash
cargo fmt --check
cargo check --locked
cargo test --locked --lib work_item_plan_
cargo test --locked --lib work_item_human_presentation_
cd web
pnpm tsc -b
pnpm test -- WorkItemPlanOverview WorkItemContractFlow WorkItemProjectionTabs HumanPresentationEditor ChatWorkspacePage.work-item-plan
```

Expected: Initial Planning、三 Projection UI、Presentation Revision 和共享 Workspace 回归全部通过，测试运行数大于 0。

- [ ] **Step 5: 提交**

```bash
git add src/product/workspace_engine/human_presentation.rs src/product/workspace_engine/mod.rs src/product/work_item_revision_store/presentation.rs src/product/workspace_engine/tests/part_20.rs src/product/workspace_engine/tests.rs src/web/workspace_ws_types src/web/workspace_ws_handler/decisions/inbound.rs web/src/components/workspace web/src/state/workspace-ws-store.ts web/src/api/types/work-item-plan.ts
git commit -m "feat(work-item-workspace): version human presentation edits"
```
