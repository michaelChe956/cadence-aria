# P2 Canonical Contract 与三 Projection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建立结构化 Canonical Work Item Contract、Dependency Contract、Human/Coder/Reviewer Projection、Provider Renderer 和投影一致性验证。

**Architecture:** Canonical Contract 使用稳定 ID 和显式引用表达规范语义；Projection Compiler 生成 Provider-neutral Artifact；Provider Renderer 只改变表达格式。Contract Hash 只覆盖规范字段，HumanPresentationRevision 独立于 Contract。

**Tech Stack:** Rust 2024、Serde、serde_json、sha2、hex、现有 `ProviderName`。

## Global Constraints

- Canonical Contract 不包含纯人类展示字段。
- 所有 Task/AC/Verification/Blocker 引用必须可解析。
- Mandatory Projection Sections 不允许截断。
- Renderer 不得增加或删除规范项。
- 不引入新依赖；复用已有 `sha2` 和 `hex`。
- 所有测试名称使用 `canonical_work_item_`、`work_item_projection_` 或 `provider_projection_renderer_` 前缀。

---

### Task 1: Canonical Contract 模型与稳定 Hash

**Files:**
- Create: `src/product/work_item_contract/mod.rs`
- Create: `src/product/work_item_contract/model.rs`
- Create: `src/product/work_item_contract/hash.rs`
- Create: `src/product/work_item_contract/tests.rs`
- Modify: `src/product/models/work_item_revision.rs`
- Modify: `src/product/mod.rs`

**Interfaces:**
- Produces: `CanonicalWorkItemContract`、`canonical_contract_hash`
- Consumed by: Projection Compiler、Draft Compiler、Impact Analyzer

- [ ] **Step 1: 写稳定 Hash 和 Serde 测试**

```rust
#[test]
fn canonical_work_item_hash_is_stable_for_identical_contracts() {
    let left = canonical_contract_fixture();
    let right = canonical_contract_fixture();

    assert_eq!(
        canonical_contract_hash(&left).unwrap(),
        canonical_contract_hash(&right).unwrap()
    );
}

#[test]
fn canonical_work_item_contract_roundtrips_without_human_presentation_fields() {
    let contract = canonical_contract_fixture();
    let value = serde_json::to_value(&contract).unwrap();

    assert!(value.get("human_summary").is_none());
    assert!(value.get("why_split").is_none());
    assert_eq!(
        serde_json::from_value::<CanonicalWorkItemContract>(value).unwrap(),
        contract
    );
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib canonical_work_item_
```

Expected: FAIL，模块和类型不存在。

- [ ] **Step 3: 实现 Canonical Contract 类型**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CanonicalWorkItemContract {
    pub schema_version: u32,
    pub identity: WorkItemContractIdentity,
    pub goal: WorkItemGoal,
    pub non_goals: Vec<String>,
    pub input_contracts: Vec<RequiredInputContract>,
    pub output_contracts: Vec<PromisedOutputContract>,
    pub tasks: Vec<WorkItemTask>,
    pub write_policy: WorkItemWritePolicy,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    pub verification_checks: Vec<VerificationCheck>,
    pub handoff_contract: HandoffContract,
    pub blocker_rules: Vec<BlockerRule>,
    pub design_traceability: Vec<DesignTraceabilityRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemContractIdentity {
    pub logical_work_item_id: String,
    pub title: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredInputContract {
    pub contract_id: String,
    pub provider_logical_work_item_id: String,
    pub required_capabilities: Vec<String>,
    pub compatibility_policy: ContractCompatibilityPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromisedOutputContract {
    pub contract_id: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemTask {
    pub task_id: String,
    pub statement: String,
    pub requirement_refs: Vec<String>,
    pub done_when_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptanceCriterion {
    pub criterion_id: String,
    pub statement: String,
    pub required_evidence: Vec<EvidenceKind>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationCheck {
    pub check_id: String,
    pub command: Option<String>,
    pub manual_instruction: Option<String>,
    pub required: bool,
    pub non_zero_test_execution_required: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockerRule {
    pub reason_code: String,
    pub route: BlockerRoute,
    pub target_contract_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemGoal {
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemWritePolicy {
    pub exclusive_scopes: Vec<String>,
    pub forbidden_scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffContract {
    pub required_fields: Vec<String>,
    pub provided_contract_refs: Vec<String>,
    pub reviewer_check_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesignTraceabilityRef {
    pub source_type: String,
    pub source_id: String,
    pub requirement_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    SourceDiff,
    NonZeroTestExecution,
    ManualCheck,
    HandoffField,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContractCompatibilityPolicy {
    RequireAll,
    RequireAny,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockerRoute {
    CoderRework,
    VerificationRetry,
    PlanRepairCurrent,
    PlanRepairUpstream,
    SubgraphReplan,
    StoryAmendment,
    DesignAmendment,
    OperationalGate,
}
```

把 P1 的通用 JSON 字段改为以下完整强类型结构：

```rust
pub struct WorkItemDraftRevision {
    pub id: String,
    pub logical_work_item_id: String,
    pub revision_no: u32,
    pub supersedes: Option<String>,
    pub revision_reason: PlanRevisionReason,
    pub canonical_contract_candidate: CanonicalWorkItemContract,
    pub trigger_repair_request_id: Option<String>,
    pub created_at: String,
}

pub struct WorkItemRevision {
    pub id: String,
    pub logical_work_item_id: String,
    pub source_draft_revision_id: String,
    pub canonical_contract: CanonicalWorkItemContract,
    pub canonical_contract_hash: String,
    pub work_item_projection_bundle_id: String,
    pub verification_plan_revision_id: String,
    pub created_at: String,
}

pub struct VerificationPlanRevision {
    pub id: String,
    pub logical_work_item_id: String,
    pub source_draft_revision_id: String,
    pub verification_checks: Vec<VerificationCheck>,
    pub created_at: String,
}

```

稳定 Hash：

```rust
pub fn canonical_contract_hash(
    contract: &CanonicalWorkItemContract,
) -> Result<String, ProductStoreError> {
    let bytes = serde_json::to_vec(contract)
        .map_err(|error| ProductStoreError::Json(error.to_string()))?;
    let digest = Sha256::digest(bytes);
    Ok(hex::encode(digest))
}
```

- [ ] **Step 4: 运行测试确认通过**

Run:

```bash
cargo test --locked --lib canonical_work_item_
```

Expected: Contract Roundtrip、Hash 和规范字段边界测试通过。

- [ ] **Step 5: 提交**

```bash
git add src/product/work_item_contract src/product/models/work_item_revision.rs src/product/mod.rs
git commit -m "feat(work-item-contract): add canonical contract model"
```

### Task 2: Dependency Contract 与严格 Validator

**Files:**
- Create: `src/product/work_item_contract/validation.rs`
- Create: `src/product/work_item_contract/dependency.rs`
- Modify: `src/product/work_item_contract/mod.rs`
- Modify: `src/product/work_item_contract/tests.rs`

**Interfaces:**
- Produces: `DependencyContractEdge`、`DependencyContractGraph`、`build_dependency_contract_graph`、`validate_canonical_contract`、`validate_dependency_contract_graph`
- Consumed by: P3 Final Compile、P4 Impact Analysis

- [ ] **Step 1: 写引用和上下游能力测试**

```rust
#[test]
fn canonical_work_item_dependency_validation_reports_missing_capability() {
    let provider = provider_contract_fixture(&["workflow_explicit_completion"]);
    let consumer = consumer_contract_fixture(&[
        "workflow_explicit_completion",
        "finalization_failure",
    ]);

    let graph = build_dependency_contract_graph(&[provider, consumer]).unwrap();
    let report = validate_dependency_contract_graph(&graph);

    assert!(report.findings.iter().any(|finding| {
        finding.code == "required_capability_missing"
            && finding.capability_ref.as_deref() == Some("finalization_failure")
    }));
}

#[test]
fn canonical_work_item_validation_requires_reviewer_check_for_every_acceptance_criterion() {
    let mut contract = canonical_contract_fixture();
    contract.handoff_contract.reviewer_check_refs.clear();

    let report = validate_canonical_contract(&contract);

    assert!(report.findings.iter().any(|finding| {
        finding.code == "acceptance_criterion_without_reviewer_check"
    }));
}

#[test]
fn canonical_work_item_validation_rejects_duplicate_acceptance_criterion_ids() {
    let mut contract = canonical_contract_fixture();
    contract.acceptance_criteria.push(contract.acceptance_criteria[0].clone());

    let report = validate_canonical_contract(&contract);
    assert!(report.findings.iter().any(|finding| {
        finding.code == "duplicate_acceptance_criterion_id"
    }));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib canonical_work_item_dependency_
cargo test --locked --lib canonical_work_item_validation_
```

Expected: FAIL，Validator 尚不存在。

- [ ] **Step 3: 实现 Validator**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractValidationReport {
    pub findings: Vec<ContractValidationFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContractValidationFinding {
    pub code: String,
    pub severity: ContractFindingSeverity,
    pub logical_work_item_id: Option<String>,
    pub contract_ref: Option<String>,
    pub capability_ref: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContractFindingSeverity {
    Warning,
    Error,
}

impl ContractValidationReport {
    pub fn is_valid(&self) -> bool {
        self.findings
            .iter()
            .all(|finding| finding.severity != ContractFindingSeverity::Error)
    }
}
```

依赖图类型和构建函数必须完整定义为：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequiredDependencyContract {
    pub contract_id: String,
    pub required_capabilities: Vec<String>,
    pub compatibility_policy: ContractCompatibilityPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyContractEdge {
    pub from: String,
    pub to: String,
    pub required_contracts: Vec<RequiredDependencyContract>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyContractGraph {
    pub contracts: BTreeMap<String, CanonicalWorkItemContract>,
    pub edges: Vec<DependencyContractEdge>,
}

pub fn build_dependency_contract_graph(
    contracts: &[CanonicalWorkItemContract],
) -> Result<DependencyContractGraph, ContractValidationReport>;

pub fn validate_dependency_contract_graph(
    graph: &DependencyContractGraph,
) -> ContractValidationReport;
```

P1 的 `DependencyGraphRevision` 同步改成强类型，禁止 P4 再解析任意 JSON：

```rust
pub struct DependencyGraphRevision {
    pub id: String,
    pub plan_id: String,
    pub edges: Vec<DependencyContractEdge>,
    pub created_at: String,
}
```

`validate_canonical_contract` 必须检查：

```text
duplicate_task_id
duplicate_acceptance_criterion_id
duplicate_verification_check_id
unknown_done_when_ref
unknown_requirement_ref
unknown_reviewer_check_ref
empty_required_write_scope
overlapping_exclusive_and_forbidden_scope
missing_required_verification_command
stage_blocker_without_target_contract
```

`validate_dependency_contract_graph` 必须检查：

```text
unknown_provider_logical_work_item
dependency_cycle
required_contract_missing
required_capability_missing
unconsumed_required_handoff
duplicate_dependency_contract_edge
```

- [ ] **Step 4: 运行 Validator 测试**

Run:

```bash
cargo test --locked --lib canonical_work_item_
```

Expected: 所有 Contract/Dependency Validator 测试通过。

- [ ] **Step 5: 提交**

```bash
git add src/product/work_item_contract
git commit -m "feat(work-item-contract): validate contract dependency graph"
```

### Task 3: 三 Projection Compiler 与 Coverage Validation

**Files:**
- Create: `src/product/work_item_projection/mod.rs`
- Create: `src/product/work_item_projection/model.rs`
- Create: `src/product/work_item_projection/human.rs`
- Create: `src/product/work_item_projection/coder.rs`
- Create: `src/product/work_item_projection/reviewer.rs`
- Create: `src/product/work_item_projection/plan.rs`
- Create: `src/product/work_item_projection/validation.rs`
- Create: `src/product/work_item_projection/tests.rs`
- Modify: `src/product/models/work_item_revision.rs`
- Modify: `src/product/mod.rs`

**Interfaces:**
- Consumes: `CanonicalWorkItemContract`、`DependencyContractGraph`
- Produces: `CompiledWorkItemProjections`、`CompiledPlanProjections`、`ProjectionValidationReport`

- [ ] **Step 1: 写 Coverage 和 No-Invention 测试**

```rust
#[test]
fn work_item_projection_compiler_covers_every_task_and_acceptance_criterion() {
    let contract = canonical_contract_fixture();
    let compiled = WorkItemProjectionCompiler::default()
        .compile(&contract, "work_item_revision_0001")
        .unwrap();

    assert_eq!(
        compiled.coder.task_refs,
        contract
            .tasks
            .iter()
            .map(|task| task.task_id.clone())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        compiled.reviewer.criterion_refs,
        contract
            .acceptance_criteria
            .iter()
            .map(|criterion| criterion.criterion_id.clone())
            .collect::<Vec<_>>()
    );
    assert!(validate_projection_coverage(&contract, &compiled).is_valid());
}

#[test]
fn work_item_projection_validation_rejects_invented_reviewer_check() {
    let contract = canonical_contract_fixture();
    let mut compiled = WorkItemProjectionCompiler::default()
        .compile(&contract, "work_item_revision_0001")
        .unwrap();
    compiled
        .reviewer
        .criterion_refs
        .push("ac_not_in_contract".to_string());

    let report = validate_projection_coverage(&contract, &compiled);
    assert!(report.findings.iter().any(|finding| {
        finding.code == "projection_invented_contract_ref"
    }));
}

#[test]
fn work_item_projection_human_presentation_does_not_change_provider_hashes() {
    let contract = canonical_contract_fixture();
    let compiled = WorkItemProjectionCompiler::default()
        .compile(&contract, "work_item_revision_0001")
        .unwrap();
    let before = projection_hashes(&compiled).unwrap();
    let presentation = human_presentation_fixture(&compiled.human);

    validate_human_presentation_revision(
        HumanPresentationBase::WorkItem(&compiled.human),
        &presentation,
    )
    .unwrap();

    assert_eq!(projection_hashes(&compiled).unwrap(), before);
    assert!(!presentation.normative);
    assert!(!presentation.used_by_provider);
}

#[test]
fn work_item_projection_plan_bundle_uses_the_same_contract_edges_and_refs() {
    let fixture = compiled_plan_projection_fixture();

    let report = validate_plan_projection_coverage(
        &fixture.graph,
        &fixture.plan,
        &fixture.work_items,
    );

    assert!(report.is_valid());
    assert_eq!(
        fixture.plan.coder.dependency_edges,
        fixture.plan.reviewer.dependency_edges
    );
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib work_item_projection_
```

Expected: FAIL，Projection 模块不存在。

- [ ] **Step 3: 实现 Projection 模型和 Compiler**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanContractSummary {
    pub contract_id: String,
    pub capabilities: Vec<String>,
    pub source_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanScopeSummary {
    pub owned_scopes: Vec<String>,
    pub forbidden_scopes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanWorkItemProjection {
    pub logical_work_item_id: String,
    pub title: String,
    pub goal: String,
    pub non_goals: Vec<String>,
    pub inputs: Vec<HumanContractSummary>,
    pub outputs: Vec<HumanContractSummary>,
    pub dependencies: Vec<String>,
    pub scope_summary: HumanScopeSummary,
    pub completion_summary: Vec<String>,
    pub source_refs: Vec<String>,
    pub normative: bool,
    pub used_by_provider: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoderWorkItemProjection {
    pub work_item_revision_id: String,
    pub objective: String,
    pub required_input_contracts: Vec<RequiredInputContract>,
    pub task_refs: Vec<String>,
    pub tasks: Vec<WorkItemTask>,
    pub write_policy: WorkItemWritePolicy,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    pub verification_checks: Vec<VerificationCheck>,
    pub blocker_rules: Vec<BlockerRule>,
    pub handoff_contract: HandoffContract,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewerWorkItemProjection {
    pub work_item_revision_id: String,
    pub criterion_refs: Vec<String>,
    pub requirement_matrix: Vec<ReviewerRequirementCheck>,
    pub scope_policy: WorkItemWritePolicy,
    pub input_contract_checks: Vec<RequiredInputContract>,
    pub output_contract_checks: Vec<PromisedOutputContract>,
    pub verification_evidence_rules: Vec<VerificationCheck>,
    pub blocker_routing: Vec<BlockerRule>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewerRequirementCheck {
    pub criterion_id: String,
    pub requirement_refs: Vec<String>,
    pub required_evidence: Vec<EvidenceKind>,
    pub failure_route: BlockerRoute,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanGroupWorkItemSummary {
    pub logical_work_item_id: String,
    pub title: String,
    pub goal: String,
    pub depends_on: Vec<String>,
    pub provides: Vec<String>,
    pub scope_summary: HumanScopeSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanContractFlowEdge {
    pub from: String,
    pub to: String,
    pub contract_id: String,
    pub required_capabilities: Vec<String>,
    pub provided_capabilities: Vec<String>,
    pub missing_capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanGroupProjection {
    pub plan_id: String,
    pub goal: String,
    pub split_reason: String,
    pub work_items: Vec<HumanGroupWorkItemSummary>,
    pub contract_flow: Vec<HumanContractFlowEdge>,
    pub risks: Vec<String>,
    pub source_refs: Vec<String>,
    pub normative: bool,
    pub used_by_provider: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoderGroupContext {
    pub plan_id: String,
    pub ordered_logical_work_item_ids: Vec<String>,
    pub dependency_edges: Vec<DependencyContractEdge>,
    pub group_write_scopes: BTreeMap<String, WorkItemWritePolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewerGroupMatrixEntry {
    pub logical_work_item_id: String,
    pub criterion_refs: Vec<String>,
    pub input_contract_refs: Vec<String>,
    pub output_contract_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewerGroupMatrix {
    pub plan_id: String,
    pub work_items: Vec<ReviewerGroupMatrixEntry>,
    pub dependency_edges: Vec<DependencyContractEdge>,
    pub design_traceability_refs: Vec<DesignTraceabilityRef>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledWorkItemProjections {
    pub human: HumanWorkItemProjection,
    pub coder: CoderWorkItemProjection,
    pub reviewer: ReviewerWorkItemProjection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledPlanProjections {
    pub human: HumanGroupProjection,
    pub coder: CoderGroupContext,
    pub reviewer: ReviewerGroupMatrix,
}
```

Compiler：

```rust
#[derive(Debug, Default)]
pub struct WorkItemProjectionCompiler;

impl WorkItemProjectionCompiler {
    pub fn compile(
        &self,
        contract: &CanonicalWorkItemContract,
        work_item_revision_id: &str,
    ) -> Result<CompiledWorkItemProjections, ProjectionCompileError> {
        let compiled = CompiledWorkItemProjections {
            human: compile_human_projection(contract),
            coder: compile_coder_projection(contract, work_item_revision_id),
            reviewer: compile_reviewer_projection(contract, work_item_revision_id),
        };
        let validation = validate_projection_coverage(contract, &compiled);
        if !validation.is_valid() {
            return Err(ProjectionCompileError::Validation(validation));
        }
        Ok(compiled)
    }
}
```

Plan 级编译器使用完全显式的输入，P3 不再自己拼装三份 JSON：

```rust
pub struct PlanProjectionCompileInput<'a> {
    pub plan_id: &'a str,
    pub goal: &'a str,
    pub split_reason: &'a str,
    pub source_refs: &'a [String],
    pub dependency_graph: &'a DependencyContractGraph,
    pub work_item_projections: &'a BTreeMap<String, CompiledWorkItemProjections>,
}

#[derive(Debug, Default)]
pub struct PlanProjectionCompiler;

impl PlanProjectionCompiler {
    pub fn compile(
        &self,
        input: PlanProjectionCompileInput<'_>,
    ) -> Result<CompiledPlanProjections, ProjectionCompileError>;
}
```

Validation 类型和函数必须在本 Task 定义，供 P3/P4 直接消费：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionValidationFinding {
    pub code: String,
    pub projection: String,
    pub contract_ref: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionValidationReport {
    pub findings: Vec<ProjectionValidationFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanValidationReportArtifact {
    pub id: String,
    pub plan_id: String,
    pub contract_validation: ContractValidationReport,
    pub projection_validation: ProjectionValidationReport,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionHashes {
    pub human: String,
    pub coder: String,
    pub reviewer: String,
}

#[derive(Debug)]
pub enum ProjectionCompileError {
    Validation(ProjectionValidationReport),
    InvalidHumanPresentation(String),
    Serialization(String),
}

impl ProjectionValidationReport {
    pub fn is_valid(&self) -> bool {
        self.findings.is_empty()
    }
}

pub fn validate_projection_coverage(
    contract: &CanonicalWorkItemContract,
    compiled: &CompiledWorkItemProjections,
) -> ProjectionValidationReport;

pub fn validate_plan_projection_coverage(
    graph: &DependencyContractGraph,
    compiled: &CompiledPlanProjections,
    work_items: &BTreeMap<String, CompiledWorkItemProjections>,
) -> ProjectionValidationReport;

pub enum HumanPresentationBase<'a> {
    Plan(&'a HumanGroupProjection),
    WorkItem(&'a HumanWorkItemProjection),
}

pub fn validate_human_presentation_revision(
    base: HumanPresentationBase<'_>,
    revision: &HumanPresentationRevision,
) -> Result<(), ProjectionCompileError>;

pub fn projection_hashes(
    compiled: &CompiledWorkItemProjections,
) -> Result<ProjectionHashes, ProjectionCompileError>;
```

`validate_human_presentation_revision` 必须拒绝 `normative = true`、`used_by_provider = true`、同时绑定 Plan/Work Item 或两者都未绑定，以及不属于 Base Projection 的 `source_refs`。

把 P1 的 Work Item 和 Plan ProjectionBundle 全部替换为强类型：

```rust
pub struct WorkItemProjectionBundle {
    pub id: String,
    pub work_item_revision_id: String,
    pub canonical_contract_hash: String,
    pub projection_schema_version: u32,
    pub compiler_version: String,
    pub human_projection: HumanWorkItemProjection,
    pub coder_projection: CoderWorkItemProjection,
    pub reviewer_projection: ReviewerWorkItemProjection,
    pub human_projection_hash: String,
    pub coder_projection_hash: String,
    pub reviewer_projection_hash: String,
    pub created_at: String,
}

pub struct PlanProjectionBundle {
    pub id: String,
    pub plan_revision_id: String,
    pub dependency_graph_revision_id: String,
    pub work_item_projection_bundle_refs: Vec<String>,
    pub human_group_projection: HumanGroupProjection,
    pub coder_group_context: CoderGroupContext,
    pub reviewer_group_matrix: ReviewerGroupMatrix,
    pub human_group_projection_hash: String,
    pub coder_group_context_hash: String,
    pub reviewer_group_matrix_hash: String,
    pub compiler_version: String,
    pub created_at: String,
}
```

- [ ] **Step 4: 运行 Projection 测试**

Run:

```bash
cargo test --locked --lib work_item_projection_
```

Expected: 三 Projection、Coverage、No-Invention 和 Hash 测试通过。

- [ ] **Step 5: 提交**

```bash
git add src/product/work_item_projection src/product/models/work_item_revision.rs src/product/mod.rs
git commit -m "feat(work-item-contract): compile three role projections"
```

### Task 4: Provider Renderer 与 Golden Tests

**Files:**
- Create: `src/product/work_item_projection/render.rs`
- Create: `src/product/work_item_projection/render/codex.rs`
- Create: `src/product/work_item_projection/render/claude_code.rs`
- Create: `src/product/work_item_projection/render/fake.rs`
- Create: `src/product/work_item_projection/execution_context.rs`
- Modify: `src/product/work_item_projection/mod.rs`
- Modify: `src/product/work_item_projection/tests.rs`
- Test: `src/product/coding_workspace_engine/tests/parser_prompt.rs`
- Test: `src/product/workspace_engine/tests/part_08.rs`

**Interfaces:**
- Produces: `CoderExecutionEnvelope`、`ReviewerExecutionEnvelope`、`RenderedExecutionContext`、`ProviderProjectionRenderer`
- Consumed by: P3 Work Item Author/Plan Reviewer、P5 Coder/Code Reviewer

- [ ] **Step 1: 写跨 Provider 语义一致性测试**

```rust
#[test]
fn provider_projection_renderers_preserve_all_normative_refs() {
    let contract = canonical_contract_fixture();
    let projections = WorkItemProjectionCompiler::default()
        .compile(&contract, "work_item_revision_0001")
        .unwrap();

    for provider in [
        ProviderName::Codex,
        ProviderName::ClaudeCode,
        ProviderName::Fake,
    ] {
        let renderer = renderer_for(&provider);
        let coder = renderer
            .render_coder(&projections.coder, &coder_execution_envelope_fixture())
            .unwrap();
        let reviewer = renderer
            .render_reviewer(
                &projections.reviewer,
                &reviewer_execution_envelope_fixture(),
            )
            .unwrap();

        for task in &contract.tasks {
            assert!(
                coder.text.contains(&task.task_id),
                "{provider:?} lost {}",
                task.task_id
            );
        }
        for criterion in &contract.acceptance_criteria {
            assert!(
                reviewer.text.contains(&criterion.criterion_id),
                "{provider:?} lost {}",
                criterion.criterion_id
            );
        }
        assert!(!coder.renderer_version.is_empty());
        assert!(!reviewer.content_hash.is_empty());
    }
}

#[test]
fn provider_projection_renderer_never_truncates_mandatory_sections() {
    let contract = large_canonical_contract_fixture();
    let projections = WorkItemProjectionCompiler::default()
        .compile(&contract, "work_item_revision_0001")
        .unwrap();
    let rendered = renderer_for(&ProviderName::Codex)
        .render_coder(&projections.coder, &coder_execution_envelope_fixture())
        .unwrap();

    for section in [
        "Objective",
        "Resolved Inputs",
        "Implementation Tasks",
        "Write Policy",
        "Acceptance Criteria",
        "Verification Checks",
        "Blocker Routing",
        "Handoff Requirements",
    ] {
        assert!(rendered.text.contains(section), "missing {section}");
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib provider_projection_renderer_
```

Expected: FAIL，Renderer 不存在。

- [ ] **Step 3: 实现 Renderer Trait**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoderExecutionEnvelope {
    pub repository_state_ref: String,
    pub resolved_handoff_revision_ids: Vec<String>,
    pub unit_run_id: String,
    pub previous_actionable_review: Option<String>,
    pub start_commit: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewerExecutionEnvelope {
    pub unit_run_id: String,
    pub diff_ref: String,
    pub test_evidence_refs: Vec<String>,
    pub handoff_revision_ids: Vec<String>,
    pub contract_delta_refs: Vec<String>,
    pub completion_commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedExecutionContext {
    pub text: String,
    pub renderer_version: String,
    pub content_hash: String,
}

pub trait ProviderProjectionRenderer: Send + Sync {
    fn render_coder(
        &self,
        projection: &CoderWorkItemProjection,
        envelope: &CoderExecutionEnvelope,
    ) -> Result<RenderedExecutionContext, ProjectionRenderError>;

    fn render_reviewer(
        &self,
        projection: &ReviewerWorkItemProjection,
        envelope: &ReviewerExecutionEnvelope,
    ) -> Result<RenderedExecutionContext, ProjectionRenderError>;
}

#[derive(Debug)]
pub enum ProjectionRenderError {
    MandatorySectionMissing(String),
    Serialization(String),
}

pub fn renderer_for(provider: &ProviderName) -> Box<dyn ProviderProjectionRenderer> {
    match provider {
        ProviderName::Codex => Box::new(CodexProjectionRenderer),
        ProviderName::ClaudeCode => Box::new(ClaudeCodeProjectionRenderer),
        ProviderName::Fake => Box::new(FakeProjectionRenderer),
    }
}
```

三个 Renderer 必须输出相同规范章节和 ID，仅允许标题、Provider 权限说明和 Structured Output 包装不同。每次渲染都必须在返回前检查固定章节完整性并计算 `content_hash`；Token Budget 不得删除 Objective、Input Contract、Task、Write Policy、Acceptance、Verification、Blocker 和 Handoff 任一章节。

- [ ] **Step 4: 运行 P2 验证**

Run:

```bash
cargo fmt --check
cargo check --locked
cargo test --locked --lib canonical_work_item_
cargo test --locked --lib work_item_projection_
cargo test --locked --lib provider_projection_renderer_
```

Expected: 所有命令 exit 0，测试运行数均大于 0。

- [ ] **Step 5: 提交**

```bash
git add src/product/work_item_projection src/product/coding_workspace_engine/tests/parser_prompt.rs src/product/workspace_engine/tests/part_08.rs
git commit -m "feat(work-item-contract): render provider-specific projections"
```
