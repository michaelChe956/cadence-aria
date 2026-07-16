# P1 Schema v2 与 Revision Store Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建立不兼容历史数据的 Schema v2，以及 Plan、Work Item、Projection、Handoff、Repair 和 Amendment 的不可变持久化基础。

**Architecture:** 新增独立 `work_item_revision_store`，避免继续扩张 `work_item_plan_store.rs`。所有 Revision 使用稳定 Lineage ID 和不可变 JSON Artifact，指针/Journal 使用原子写；启动时只接受空数据或 Schema v2。

**Tech Stack:** Rust 2024、Serde、chrono、serde_json、现有 `ProductAppPaths` 与 `json_store::write_json`。

## Global Constraints

- 不读取、迁移或兼容旧 Work Item/Coding JSON。
- Store 方法必须要求 project/issue/plan 作用域，不提供只按局部 ID 的歧义查找。
- Revision Artifact 不允许覆盖；重复写相同内容幂等，内容不同返回 `IdentityMismatch`。
- 所有路径 ID 使用 `validate_relative_id`。
- 所有测试名称使用 `work_item_revision_` 或 `product_data_schema_` 前缀。
- 禁止 `-j 1`。

---

### Task 1: Product Data Schema v2 门禁

**Files:**
- Create: `src/product/product_data_schema.rs`
- Modify: `src/product/mod.rs`
- Modify: `src/product/app_paths.rs`
- Modify: `src/web/app.rs`
- Test: `src/product/product_data_schema.rs`

**Interfaces:**
- Consumes: `ProductAppPaths::root()`、`read_json`、`write_json`
- Produces: `ensure_product_data_schema(paths: &ProductAppPaths) -> Result<ProductDataSchema, ProductStoreError>`

- [ ] **Step 1: 写失败测试，覆盖空目录、v2 和旧数据拒绝**

```rust
#[test]
fn product_data_schema_creates_v2_for_empty_root() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));

    let schema = ensure_product_data_schema(&paths).unwrap();

    assert_eq!(schema.product_data_schema_version, 2);
    assert_eq!(
        read_json::<ProductDataSchema>(&paths.product_data_schema_path()).unwrap(),
        schema
    );
}

#[test]
fn product_data_schema_rejects_legacy_business_data() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    std::fs::create_dir_all(paths.projects_root()).unwrap();
    write_json(
        &paths.projects_root().join("legacy.json"),
        &serde_json::json!({"legacy": true}),
    )
    .unwrap();

    let error = ensure_product_data_schema(&paths).unwrap_err();
    assert!(error.to_string().contains("product_data_schema_unsupported"));
}

#[test]
fn product_data_schema_rejects_missing_schema_when_coding_data_exists() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let attempt_root = paths
        .issue_root("project_0001", "issue_0001")
        .join("coding-attempts");
    std::fs::create_dir_all(&attempt_root).unwrap();
    write_json(
        &attempt_root.join("coding_attempt_0001.json"),
        &serde_json::json!({"id": "coding_attempt_0001"}),
    )
    .unwrap();

    let error = ensure_product_data_schema(&paths).unwrap_err();
    assert!(error.to_string().contains("product_data_schema_unsupported"));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib product_data_schema_
```

Expected: FAIL，原因是 `product_data_schema` 模块和 `product_data_schema_path` 尚不存在。

- [ ] **Step 3: 实现 Schema 门禁**

```rust
pub const PRODUCT_DATA_SCHEMA_VERSION: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProductDataSchema {
    pub product_data_schema_version: u32,
}

pub fn ensure_product_data_schema(
    paths: &ProductAppPaths,
) -> Result<ProductDataSchema, ProductStoreError> {
    let schema_path = paths.product_data_schema_path();
    if schema_path.is_file() {
        let schema: ProductDataSchema = read_json(&schema_path)?;
        if schema.product_data_schema_version == PRODUCT_DATA_SCHEMA_VERSION {
            return Ok(schema);
        }
        return Err(ProductStoreError::Io(format!(
            "product_data_schema_unsupported: expected v{PRODUCT_DATA_SCHEMA_VERSION}, found v{}",
            schema.product_data_schema_version
        )));
    }

    let legacy_business_data_exists = [paths.projects_root(), paths.state_root()]
        .into_iter()
        .filter(|path| path.exists())
        .map(std::fs::read_dir)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| ProductStoreError::Io(error.to_string()))?
        .into_iter()
        .any(|mut entries| entries.next().is_some());

    if legacy_business_data_exists {
        return Err(ProductStoreError::Io(
            "product_data_schema_unsupported: legacy business data exists".to_string(),
        ));
    }

    let schema = ProductDataSchema {
        product_data_schema_version: PRODUCT_DATA_SCHEMA_VERSION,
    };
    write_json(&schema_path, &schema)?;
    Ok(schema)
}
```

在 `ProductAppPaths` 增加：

```rust
pub fn product_data_schema_path(&self) -> PathBuf {
    self.root.join("schema.json")
}
```

`src/web/app.rs::serve_web` 在构造 `WebAppState` 和任何 Product Store 之前执行：

```rust
let product_paths = ProductAppPaths::new(workspace_root.join(".aria"));
ensure_product_data_schema(&product_paths)
    .map_err(|error| anyhow::anyhow!(error.to_string()))?;
```

该调用是 Schema v2 的唯一启动门禁；不得只在 Revision Store 首次访问时延迟校验。

- [ ] **Step 4: 运行测试确认通过**

Run:

```bash
cargo test --locked --lib product_data_schema_
```

Expected: 4 个 Schema 测试通过，0 失败，并且 Web 启动路径调用 Schema 门禁。

- [ ] **Step 5: 提交**

```bash
git add src/product/product_data_schema.rs src/product/app_paths.rs src/product/mod.rs src/web/app.rs
git commit -m "feat(work-item-revision): enforce product data schema v2"
```

### Task 2: 定义 Revision 领域模型

**Files:**
- Create: `src/product/models/work_item_revision.rs`
- Create: `src/product/models/plan_repair.rs`
- Modify: `src/product/models/mod.rs`
- Test: `src/product/models/tests.rs`

**Interfaces:**
- Produces: Plan/Work Item/Projection/Handoff/Repair/Amendment 的 Serde 模型
- Consumed by: Task 3 Store、P2 Contract、P4 Repair、P5 Coding Binding

- [ ] **Step 1: 写 Serde Roundtrip 和状态不变量测试**

```rust
#[test]
fn work_item_revision_models_roundtrip_without_legacy_fields() {
    let revision = WorkItemPlanRevision {
        id: "plan_revision_0001".to_string(),
        plan_id: "issue_work_item_plan_0001".to_string(),
        revision_no: 1,
        supersedes: None,
        reason: PlanRevisionReason::InitialCompile,
        work_item_bindings: BTreeMap::from([(
            "wi_core".to_string(),
            "work_item_revision_0001".to_string(),
        )]),
        dependency_graph_revision_id: "dependency_graph_revision_0001".to_string(),
        validation_report_ref: "validation_report_0001".to_string(),
        plan_projection_bundle_id: "plan_projection_bundle_0001".to_string(),
        created_at: "2026-07-17T00:00:00Z".to_string(),
    };

    let value = serde_json::to_value(&revision).unwrap();
    assert_eq!(value["revision_no"], 1);
    assert!(value.get("work_item_ids").is_none());
    assert_eq!(
        serde_json::from_value::<WorkItemPlanRevision>(value).unwrap(),
        revision
    );
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib work_item_revision_models_
```

Expected: FAIL，模型尚不存在。

- [ ] **Step 3: 实现领域模型**

必须定义以下核心类型：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemPlanLineage {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub story_spec_refs: Vec<String>,
    pub design_spec_refs: Vec<String>,
    pub active_revision_id: Option<String>,
    pub active_amendment_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanRevisionReason {
    InitialCompile,
    RepairCurrentWorkItem,
    RepairUpstreamContract,
    SubgraphReplan,
    StoryAmendment,
    DesignAmendment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemPlanRevision {
    pub id: String,
    pub plan_id: String,
    pub revision_no: u32,
    pub supersedes: Option<String>,
    pub reason: PlanRevisionReason,
    pub work_item_bindings: BTreeMap<String, String>,
    pub dependency_graph_revision_id: String,
    pub validation_report_ref: String,
    pub plan_projection_bundle_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogicalWorkItem {
    pub id: String,
    pub plan_id: String,
    pub title: String,
    pub active_revision_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemDraftRevision {
    pub id: String,
    pub logical_work_item_id: String,
    pub revision_no: u32,
    pub supersedes: Option<String>,
    pub revision_reason: PlanRevisionReason,
    pub canonical_contract_candidate: serde_json::Value,
    pub trigger_repair_request_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemDraftRevisionState {
    pub draft_revision_id: String,
    pub status: WorkItemDraftRevisionStatus,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemDraftRevisionStatus {
    Drafting,
    Reviewing,
    ChangesRequested,
    Approved,
    Rejected,
    Compiled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemRevision {
    pub id: String,
    pub logical_work_item_id: String,
    pub source_draft_revision_id: String,
    pub canonical_contract: serde_json::Value,
    pub canonical_contract_hash: String,
    pub work_item_projection_bundle_id: String,
    pub verification_plan_revision_id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationPlanRevision {
    pub id: String,
    pub logical_work_item_id: String,
    pub source_draft_revision_id: String,
    pub verification_checks: serde_json::Value,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanValidationReportArtifact {
    pub id: String,
    pub plan_id: String,
    pub contract_validation: serde_json::Value,
    pub projection_validation: serde_json::Value,
    pub created_at: String,
}
```

为保持 Artifact 真正不可变：

- Draft 内容保存在不可变 `WorkItemDraftRevision`，流程状态单独写入 `WorkItemDraftRevisionState`；Changes Requested 后修改 Contract 必须创建下一 Draft Revision。
- WorkItemRevision 的 `active/superseded` 是查询时派生状态：Revision ID 等于 `LogicalWorkItem.active_revision_id` 时为 Active，否则在后续 PlanRevision/Manifest 引用中显示为 Superseded；禁止为了更新状态重写旧 WorkItemRevision JSON。

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemProjectionBundle {
    pub id: String,
    pub work_item_revision_id: String,
    pub canonical_contract_hash: String,
    pub projection_schema_version: u32,
    pub compiler_version: String,
    pub human_projection: serde_json::Value,
    pub coder_projection: serde_json::Value,
    pub reviewer_projection: serde_json::Value,
    pub human_projection_hash: String,
    pub coder_projection_hash: String,
    pub reviewer_projection_hash: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanProjectionBundle {
    pub id: String,
    pub plan_revision_id: String,
    pub dependency_graph_revision_id: String,
    pub work_item_projection_bundle_refs: Vec<String>,
    pub human_group_projection: serde_json::Value,
    pub coder_group_context: serde_json::Value,
    pub reviewer_group_matrix: serde_json::Value,
    pub human_group_projection_hash: String,
    pub coder_group_context_hash: String,
    pub reviewer_group_matrix_hash: String,
    pub compiler_version: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HumanPresentationRevision {
    pub id: String,
    pub source_plan_projection_bundle_id: Option<String>,
    pub source_work_item_projection_bundle_id: Option<String>,
    pub supersedes: Option<String>,
    pub human_summary: String,
    pub why_split: Option<String>,
    pub dependency_explanation: Vec<String>,
    pub risk_explanation: Vec<String>,
    pub source_refs: Vec<String>,
    pub normative: bool,
    pub used_by_provider: bool,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandoffRevision {
    pub id: String,
    pub logical_work_item_id: String,
    pub work_item_revision_id: String,
    pub coding_unit_run_id: String,
    pub provided_contracts: Vec<String>,
    pub provided_capabilities: BTreeMap<String, Vec<String>>,
    pub contract_hash: String,
    pub commit_sha: String,
    pub tests: Vec<String>,
    pub artifacts: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyGraphRevision {
    pub id: String,
    pub plan_id: String,
    pub edges: Vec<serde_json::Value>,
    pub created_at: String,
}
```

在 `models/plan_repair.rs` 定义跨阶段共享记录：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanDefectClass {
    ImplementationDefect,
    VerificationIncomplete,
    CurrentWorkItemInvalid,
    UpstreamContractInvalid,
    DependencyGraphInvalid,
    DesignAmendmentRequired,
    StoryAmendmentRequired,
    OperationalBlocker,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanDefectRoute {
    CoderRework,
    VerificationRetry,
    PlanRepair,
    StoryAmendment,
    DesignAmendment,
    OperationalGate,
    HumanTriage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepairTargetKind {
    CurrentWorkItem,
    UpstreamWorkItem,
    Subgraph,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepairTarget {
    pub kind: RepairTargetKind,
    pub logical_work_item_ids: Vec<String>,
    pub work_item_revision_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanRepairRequestStatus {
    Open,
    InProgress,
    AwaitingConfirmation,
    Published,
    Applied,
    Cancelled,
    Failed,
}

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
    pub evidence: Vec<serde_json::Value>,
    pub fingerprint: String,
    pub status: PlanRepairRequestStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemRevisionReplacement {
    pub previous_revision_id: String,
    pub next_revision_id: String,
    pub delta_kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AmendmentResumeMode {
    Reexecute,
    Revalidate,
    AwaitHandoff,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmendmentResumeTarget {
    pub logical_work_item_id: String,
    pub mode: AmendmentResumeMode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanAmendmentManifest {
    pub id: String,
    pub repair_request_id: String,
    pub previous_plan_revision_id: String,
    pub new_plan_revision_id: String,
    pub revised_work_items: BTreeMap<String, WorkItemRevisionReplacement>,
    pub superseded_revisions: Vec<String>,
    pub dependency_graph_changes: Vec<serde_json::Value>,
    pub contract_deltas: Vec<serde_json::Value>,
    pub unaffected_units: Vec<String>,
    pub revalidation_required_units: Vec<String>,
    pub stale_units: Vec<String>,
    pub replacement_units: BTreeMap<String, Vec<String>>,
    pub resume_target: AmendmentResumeTarget,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanAmendmentPublicationPhase {
    Prepared,
    PlanPublished,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanAmendmentPublicationJournal {
    pub id: String,
    pub plan_id: String,
    pub amendment_id: String,
    pub phase: PlanAmendmentPublicationPhase,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
```

- [ ] **Step 4: 运行模型测试**

Run:

```bash
cargo test --locked --lib work_item_revision_models_
```

Expected: Roundtrip、enum serialization 和缺失必填字段测试全部通过。

- [ ] **Step 5: 提交**

```bash
git add src/product/models/work_item_revision.rs src/product/models/plan_repair.rs src/product/models/mod.rs src/product/models/tests.rs
git commit -m "feat(work-item-revision): add immutable revision models"
```

### Task 3: 新增 Revision Store 与不可变写入

**Files:**
- Create: `src/product/work_item_revision_store/mod.rs`
- Create: `src/product/work_item_revision_store/paths.rs`
- Create: `src/product/work_item_revision_store/plan.rs`
- Create: `src/product/work_item_revision_store/work_item.rs`
- Create: `src/product/work_item_revision_store/projection.rs`
- Create: `src/product/work_item_revision_store/presentation.rs`
- Create: `src/product/work_item_revision_store/dependency.rs`
- Create: `src/product/work_item_revision_store/handoff.rs`
- Create: `src/product/work_item_revision_store/repair.rs`
- Create: `src/product/work_item_revision_store/tests.rs`
- Modify: `src/product/mod.rs`

**Interfaces:**
- Consumes: P1 Task 2 models
- Produces: scoped CRUD、不可变 Artifact 写入、Active Revision 指针更新

- [ ] **Step 1: 写不可变写入和作用域测试**

```rust
#[test]
fn work_item_revision_store_rejects_overwriting_revision_with_different_content() {
    let (store, plan) = test_store_and_plan();
    let revision = plan_revision(&plan, "plan_revision_0001", 1);

    store.put_plan_revision(&plan, &revision).unwrap();
    store.put_plan_revision(&plan, &revision).unwrap();

    let mut changed = revision.clone();
    changed.reason = PlanRevisionReason::SubgraphReplan;
    let error = store.put_plan_revision(&plan, &changed).unwrap_err();

    assert!(matches!(error, ProductStoreError::IdentityMismatch { .. }));
}

#[test]
fn work_item_revision_store_never_resolves_revision_outside_issue_scope() {
    let (store, plan) = test_store_and_plan();
    store
        .put_plan_revision(&plan, &plan_revision(&plan, "plan_revision_0001", 1))
        .unwrap();

    let error = store
        .get_plan_revision(
            "project_0001",
            "issue_other",
            &plan.id,
            "plan_revision_0001",
        )
        .unwrap_err();
    assert!(matches!(error, ProductStoreError::NotFound { .. }));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib work_item_revision_store_
```

Expected: FAIL，Store 尚不存在。

- [ ] **Step 3: 实现路径和不可变写入助手**

```rust
fn write_immutable<T>(
    path: &Path,
    kind: &'static str,
    id: &str,
    value: &T,
) -> Result<(), ProductStoreError>
where
    T: Serialize + DeserializeOwned + PartialEq,
{
    if path.is_file() {
        let existing: T = read_json(path)?;
        if existing == *value {
            return Ok(());
        }
        return Err(ProductStoreError::IdentityMismatch {
            kind,
            id: id.to_string(),
        });
    }
    write_json(path, value)
}
```

`WorkItemRevisionStore` 必须提供：

```rust
pub fn put_plan_lineage(&self, value: &WorkItemPlanLineage) -> Result<(), ProductStoreError>;
pub fn get_plan_lineage(
    &self,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
) -> Result<WorkItemPlanLineage, ProductStoreError>;
pub fn put_plan_revision(
    &self,
    lineage: &WorkItemPlanLineage,
    value: &WorkItemPlanRevision,
) -> Result<(), ProductStoreError>;
pub fn get_plan_revision(
    &self,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    revision_id: &str,
) -> Result<WorkItemPlanRevision, ProductStoreError>;
pub fn set_active_plan_revision(
    &self,
    lineage: &WorkItemPlanLineage,
    revision_id: &str,
) -> Result<WorkItemPlanLineage, ProductStoreError>;
pub fn compare_and_set_active_plan_revision(
    &self,
    lineage: &WorkItemPlanLineage,
    expected_revision_id: &str,
    next_revision_id: &str,
) -> Result<WorkItemPlanLineage, ProductStoreError>;
pub fn acquire_active_amendment(
    &self,
    lineage: &WorkItemPlanLineage,
    amendment_id: &str,
) -> Result<WorkItemPlanLineage, ProductStoreError>;
pub fn release_active_amendment(
    &self,
    lineage: &WorkItemPlanLineage,
    amendment_id: &str,
) -> Result<WorkItemPlanLineage, ProductStoreError>;
pub fn put_logical_work_item(&self, value: &LogicalWorkItem) -> Result<(), ProductStoreError>;
pub fn set_active_work_item_revision(
    &self,
    logical_work_item: &LogicalWorkItem,
    expected_revision_id: Option<&str>,
    next_revision_id: &str,
) -> Result<LogicalWorkItem, ProductStoreError>;
pub fn put_draft_revision(&self, plan: &WorkItemPlanLineage, value: &WorkItemDraftRevision)
    -> Result<(), ProductStoreError>;
pub fn update_draft_revision_state(
    &self,
    plan: &WorkItemPlanLineage,
    draft_revision_id: &str,
    status: WorkItemDraftRevisionStatus,
) -> Result<WorkItemDraftRevisionState, ProductStoreError>;
pub fn put_work_item_revision(&self, plan: &WorkItemPlanLineage, value: &WorkItemRevision)
    -> Result<(), ProductStoreError>;
pub fn get_work_item_revision(
    &self,
    plan: &WorkItemPlanLineage,
    logical_work_item_id: &str,
    revision_id: &str,
) -> Result<WorkItemRevision, ProductStoreError>;
pub fn put_verification_plan_revision(
    &self,
    plan: &WorkItemPlanLineage,
    value: &VerificationPlanRevision,
) -> Result<(), ProductStoreError>;
pub fn get_verification_plan_revision(
    &self,
    plan: &WorkItemPlanLineage,
    revision_id: &str,
) -> Result<VerificationPlanRevision, ProductStoreError>;
pub fn put_plan_validation_report(
    &self,
    plan: &WorkItemPlanLineage,
    value: &PlanValidationReportArtifact,
) -> Result<(), ProductStoreError>;
pub fn get_plan_validation_report(
    &self,
    plan: &WorkItemPlanLineage,
    report_id: &str,
) -> Result<PlanValidationReportArtifact, ProductStoreError>;
pub fn put_work_item_projection_bundle(
    &self,
    plan: &WorkItemPlanLineage,
    value: &WorkItemProjectionBundle,
) -> Result<(), ProductStoreError>;
pub fn get_work_item_projection_bundle(
    &self,
    plan: &WorkItemPlanLineage,
    bundle_id: &str,
) -> Result<WorkItemProjectionBundle, ProductStoreError>;
pub fn put_plan_projection_bundle(
    &self,
    plan: &WorkItemPlanLineage,
    value: &PlanProjectionBundle,
) -> Result<(), ProductStoreError>;
pub fn get_plan_projection_bundle(
    &self,
    plan: &WorkItemPlanLineage,
    bundle_id: &str,
) -> Result<PlanProjectionBundle, ProductStoreError>;
pub fn put_human_presentation_revision(
    &self,
    plan: &WorkItemPlanLineage,
    value: &HumanPresentationRevision,
) -> Result<(), ProductStoreError>;
pub fn get_latest_human_presentation_revision(
    &self,
    plan: &WorkItemPlanLineage,
    source_projection_bundle_id: &str,
) -> Result<Option<HumanPresentationRevision>, ProductStoreError>;
pub fn put_dependency_graph_revision(
    &self,
    plan: &WorkItemPlanLineage,
    value: &DependencyGraphRevision,
) -> Result<(), ProductStoreError>;
pub fn get_dependency_graph_revision(
    &self,
    plan: &WorkItemPlanLineage,
    revision_id: &str,
) -> Result<DependencyGraphRevision, ProductStoreError>;
pub fn put_handoff_revision(&self, plan: &WorkItemPlanLineage, value: &HandoffRevision)
    -> Result<(), ProductStoreError>;
pub fn get_handoff_revision(
    &self,
    plan: &WorkItemPlanLineage,
    logical_work_item_id: &str,
    handoff_revision_id: &str,
) -> Result<HandoffRevision, ProductStoreError>;
pub fn put_repair_request(&self, plan: &WorkItemPlanLineage, value: &PlanRepairRequest)
    -> Result<(), ProductStoreError>;
pub fn update_repair_request_status(
    &self,
    plan: &WorkItemPlanLineage,
    request_id: &str,
    status: PlanRepairRequestStatus,
) -> Result<PlanRepairRequest, ProductStoreError>;
pub fn merge_repair_request_evidence(
    &self,
    plan: &WorkItemPlanLineage,
    request_id: &str,
    evidence: Vec<serde_json::Value>,
) -> Result<PlanRepairRequest, ProductStoreError>;
pub fn list_open_repair_requests(
    &self,
    plan: &WorkItemPlanLineage,
) -> Result<Vec<PlanRepairRequest>, ProductStoreError>;
pub fn put_amendment_manifest(
    &self,
    plan: &WorkItemPlanLineage,
    value: &PlanAmendmentManifest,
) -> Result<(), ProductStoreError>;
pub fn get_amendment_manifest(
    &self,
    plan: &WorkItemPlanLineage,
    amendment_id: &str,
) -> Result<PlanAmendmentManifest, ProductStoreError>;
pub fn put_plan_amendment_publication_journal(
    &self,
    plan: &WorkItemPlanLineage,
    value: &PlanAmendmentPublicationJournal,
) -> Result<(), ProductStoreError>;
```

- [ ] **Step 4: 运行 Store 测试**

Run:

```bash
cargo test --locked --lib work_item_revision_store_
```

Expected: 不可变写入、作用域隔离、幂等、Active Revision 指针测试全部通过。

- [ ] **Step 5: 提交**

```bash
git add src/product/work_item_revision_store src/product/mod.rs
git commit -m "feat(work-item-revision): persist scoped immutable revisions"
```

### Task 4: Plan Amendment Publication Journal 与发布顺序

**Files:**
- Modify: `src/product/work_item_revision_store/repair.rs`
- Modify: `src/product/work_item_revision_store/tests.rs`

**Interfaces:**
- Produces: `put_plan_amendment_publication_journal`、`advance_plan_amendment_publication`、`mark_plan_amendment_publication_failed`
- Consumed by: P4 Amendment Publisher；P5 使用独立的 Coding Application Journal

- [ ] **Step 1: 写状态转换和恢复测试**

```rust
#[test]
fn work_item_revision_publication_journal_allows_only_forward_idempotent_transitions() {
    let (store, plan) = test_store_and_plan();
    let journal = publication_journal(
        "amendment_0001",
        PlanAmendmentPublicationPhase::Prepared,
    );
    store
        .put_plan_amendment_publication_journal(&plan, &journal)
        .unwrap();

    let published = store
        .advance_plan_amendment_publication(
            &plan,
            &journal.id,
            PlanAmendmentPublicationPhase::PlanPublished,
        )
        .unwrap();
    assert_eq!(published.phase, PlanAmendmentPublicationPhase::PlanPublished);

    let error = store
        .advance_plan_amendment_publication(
            &plan,
            &journal.id,
            PlanAmendmentPublicationPhase::Prepared,
        )
        .unwrap_err();
    assert!(error.to_string().contains("amendment_phase_regression"));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib work_item_revision_publication_journal_
```

Expected: FAIL，Journal API 尚不存在。

- [ ] **Step 3: 实现单调状态转换**

```rust
impl PlanAmendmentPublicationPhase {
    fn order(&self) -> u8 {
        match self {
            Self::Prepared => 0,
            Self::PlanPublished => 1,
        }
    }
}

pub fn advance_plan_amendment_publication(
    &self,
    plan: &WorkItemPlanLineage,
    amendment_id: &str,
    next: PlanAmendmentPublicationPhase,
) -> Result<PlanAmendmentPublicationJournal, ProductStoreError> {
    let mut journal = self.get_plan_amendment_publication_journal(plan, amendment_id)?;
    if journal.phase == next {
        return Ok(journal);
    }
    if journal.phase == PlanAmendmentPublicationPhase::PlanPublished
        || next.order() <= journal.phase.order()
    {
        return Err(ProductStoreError::Io(format!(
            "amendment_phase_regression: {:?} -> {:?}",
            journal.phase, next
        )));
    }
    journal.phase = next;
    journal.updated_at = Utc::now().to_rfc3339();
    write_json(
        &self.plan_amendment_publication_journal_path(plan, amendment_id),
        &journal,
    )?;
    Ok(journal)
}
```

发布失败不改变最后成功 Phase；Store 另提供：

```rust
pub fn mark_plan_amendment_publication_failed(
    &self,
    plan: &WorkItemPlanLineage,
    amendment_id: &str,
    error: String,
) -> Result<PlanAmendmentPublicationJournal, ProductStoreError>;
```

恢复时读取 `phase` 决定从 Prepared 后的 CAS/Plan Published 步骤继续，成功推进 Phase 时清空 `error`。

- [ ] **Step 4: 运行 P1 全部定向测试与检查**

Run:

```bash
cargo fmt --check
cargo check --locked
cargo test --locked --lib product_data_schema_
cargo test --locked --lib work_item_revision_
```

Expected: 所有命令 exit 0，测试实际运行数大于 0。

- [ ] **Step 5: 提交**

```bash
git add src/product/work_item_revision_store
git commit -m "feat(work-item-revision): journal plan amendment publication"
```
