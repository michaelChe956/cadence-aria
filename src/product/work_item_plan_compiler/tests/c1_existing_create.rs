//! C1 Task 5（REQ-C1-PLAN-01）：existing/create 计划意图合同与 compile 校验。
//!
//! - 合法 `create`（provider Work Item、依赖、exclusive/forbidden scope、
//!   target 全部可验证）通过编译并随 IR 持久化保留意图；
//! - 写入面涉及基线外路径但未声明 create → `intent_undeclared`（停等修订，
//!   不产生 work item/compile child）；
//! - 错 provider Work Item、缺 dependency、exclusive/forbidden scope 冲突、
//!   target 不符 → `intent_unexecutable`（拒绝 compile，既有 finding 不清空）。

use super::*;
use crate::product::models::{RepositoryProfile, RepositoryProfileConfidence};
use crate::product::work_item_plan_compiler::{
    PlanCandidateValidationContext, validate_plan_candidate_ir,
};
use std::collections::BTreeSet;

const C1_TARGET_REPO: &str = "repo-c1";

fn c1_source(intent_section: &str, write_scopes: (&str, &str), depends_on: &str) -> String {
    format!(
        r#"# Work Item Plan

## Work Item WI-001: Levels API

### Identity
- schema_version: 1
- logical_work_item_id: WI-001
- title: Levels API
- kind: backend

### Goal
- summary: WHEN a levels request arrives THE SYSTEM SHALL return configured levels.

### Non Goals
- non_goals: Rendering is out of scope.

### Dependencies
- depends_on: {depends_on}
{intent_section}
### Inputs

### Outputs
- contract_id: contract.levels-api
- capabilities: api.levels.read

### Tasks
- task_id: TASK-001
- statement: WHEN the levels API starts THE SYSTEM SHALL serve configured levels.
- requirement_refs: REQ-C1-01
- done_when_refs: AC-001

### Write Policy
- exclusive_scopes: {exclusive}
- forbidden_scopes: {forbidden}

### Acceptance Criteria
- criterion_id: AC-001
- statement: WHEN GET /levels is requested THE SYSTEM SHALL return configured levels.
- required_evidence: source_diff

### Verification
- check_id: CHECK-001
- command: cargo test --locked --lib levels
- manual_instruction: Confirm the levels endpoint returns configured levels.
- required: true
- non_zero_test_execution_required: true

### Handoff Schema
- required_fields: commit_sha
- provided_contract_refs: contract.levels-api
- reviewer_check_refs: AC-001

### Blockers
- reason_code: levels_invalid
- route: plan_repair_current
- target_contract_refs: contract.levels-api

### Traceability
- source_type: design_spec
- source_id: design_c1_0001
- requirement_id: REQ-C1-01

## Work Item WI-002: Levels store

### Identity
- schema_version: 1
- logical_work_item_id: WI-002
- title: Levels store
- kind: backend

### Goal
- summary: WHEN levels are read THE SYSTEM SHALL return persisted rows.

### Non Goals
- non_goals: API surface is out of scope.

### Dependencies
- depends_on: []

### Plan Intent
- intent: create
- provider_work_item_id: WI-002
- intent_target_kind: single_repository
- intent_target_repo: {C1_TARGET_REPO}

### Inputs

### Outputs
- contract_id: contract.levels-store
- capabilities: store.levels.read

### Tasks
- task_id: TASK-002
- statement: WHEN the store loads THE SYSTEM SHALL expose persisted levels.
- requirement_refs: REQ-C1-02
- done_when_refs: AC-002

### Write Policy
- exclusive_scopes: src/levels_store/**
- forbidden_scopes: web/**

### Acceptance Criteria
- criterion_id: AC-002
- statement: WHEN the store is queried THE SYSTEM SHALL return persisted rows.
- required_evidence: source_diff

### Verification
- check_id: CHECK-002
- command: cargo test --locked --lib levels_store
- manual_instruction: Confirm the store returns persisted rows.
- required: true
- non_zero_test_execution_required: true

### Handoff Schema
- required_fields: commit_sha
- provided_contract_refs: contract.levels-store
- reviewer_check_refs: AC-002

### Blockers
- reason_code: levels_store_invalid
- route: plan_repair_current
- target_contract_refs: contract.levels-store

### Traceability
- source_type: design_spec
- source_id: design_c1_0001
- requirement_id: REQ-C1-02
"#,
        exclusive = write_scopes.0,
        forbidden = write_scopes.1,
    )
}

const CREATE_INTENT: &str = r#"
### Plan Intent
- intent: create
- provider_work_item_id: WI-002
- intent_target_kind: single_repository
- intent_target_repo: repo-c1
"#;

fn c1_profile() -> RepositoryProfile {
    RepositoryProfile {
        id: "repository_profile_c1_0001".to_string(),
        project_id: "project_c1_0001".to_string(),
        issue_id: "issue_c1_0001".to_string(),
        repository_id: C1_TARGET_REPO.to_string(),
        logical_repository_id: None,
        membership_revision: 0,
        provider_run_ref: None,
        languages: vec!["rust".to_string()],
        frameworks: vec!["axum".to_string()],
        package_managers: vec!["cargo".to_string()],
        test_frameworks: vec!["cargo-test".to_string()],
        build_systems: vec!["cargo".to_string()],
        verification_capabilities: vec!["unit".to_string()],
        detected_layers: vec!["backend".to_string()],
        split_recommendation: "single_layer".to_string(),
        confidence: RepositoryProfileConfidence::High,
        uncertainties: Vec::new(),
        created_at: "2026-09-29T00:00:00Z".to_string(),
        updated_at: "2026-09-29T00:00:00Z".to_string(),
    }
}

fn c1_options() -> &'static crate::product::models::IssueWorkItemPlanOptions {
    static C1_OPTIONS: std::sync::LazyLock<crate::product::models::IssueWorkItemPlanOptions> =
        std::sync::LazyLock::new(|| crate::product::models::IssueWorkItemPlanOptions {
            include_integration_tests: false,
            include_e2e_tests: false,
            force_frontend_backend_split: false,
            require_execution_plan_confirm: false,
        });
    &C1_OPTIONS
}

/// 基线树：只含既有写面（src/levels_store/** 有交集），src/levels_new/**
/// 完全在基线外＝新建写面，必须声明 create。
fn c1_baseline_tree() -> BTreeSet<String> {
    BTreeSet::from([
        "src/levels_store/mod.rs".to_string(),
        "web/src/app.ts".to_string(),
    ])
}

fn c1_spec_ids() -> (&'static [String], &'static [String]) {
    static STORY: std::sync::LazyLock<Vec<String>> =
        std::sync::LazyLock::new(|| vec!["story_spec_c1_0001".to_string()]);
    static DESIGN: std::sync::LazyLock<Vec<String>> =
        std::sync::LazyLock::new(|| vec!["design_spec_c1_0001".to_string()]);
    (&STORY, &DESIGN)
}

/// enrolled 缺省语境：binding target = C1_TARGET_REPO（与 compile 上下文一致）。
fn c1_context<'a>(
    existing_ids: &'a [String],
    baseline: Option<&'a BTreeSet<String>>,
) -> PlanCandidateValidationContext<'a> {
    static BOUND_TARGET: std::sync::LazyLock<
        crate::product::logical_codebase::EnrollmentTarget,
    > = std::sync::LazyLock::new(|| {
        crate::product::logical_codebase::EnrollmentTarget::SingleRepository {
            repository_id: C1_TARGET_REPO.to_string(),
        }
    });
    c1_context_with_target(existing_ids, baseline, Some(&BOUND_TARGET))
}

fn c1_context_with_target<'a>(
    existing_ids: &'a [String],
    baseline: Option<&'a BTreeSet<String>>,
    enrollment_target: Option<&'a crate::product::logical_codebase::EnrollmentTarget>,
) -> PlanCandidateValidationContext<'a> {
    PlanCandidateValidationContext {
        project_id: "project_c1_0001",
        issue_id: "issue_c1_0001",
        plan_id: "plan_c1_0001",
        source_story_spec_ids: c1_spec_ids().0,
        source_design_spec_ids: c1_spec_ids().1,
        repository_profile: Some(c1_profile_ref()),
        plan_options: &c1_options(),
        baseline_tree: baseline,
        existing_work_item_ids: existing_ids,
        enrollment_target,
        now: "2026-09-29T00:00:00Z",
    }
}

fn c1_profile_ref() -> &'static RepositoryProfile {
    static PROFILE: std::sync::LazyLock<RepositoryProfile> =
        std::sync::LazyLock::new(c1_profile);
    &PROFILE
}

/// 合法 create：provider 在 plan 内、依赖闭包可解析、scope 无冲突、
/// target 与 compile 上下文一致 → 通过校验，意图随 IR 保留。
#[test]
fn c1_existing_create_declared_create_compiles_and_persists_intent() {
    let source = c1_source(CREATE_INTENT, ("src/levels_new/**", "web/**"), "WI-002");
    let ir = compile_work_item_plan(
        &source,
        &WorkItemPlanSourceContext {
            target_repository_id: C1_TARGET_REPO.to_string(),
        },
    )
    .expect("declared create plan must lower");

    // lowering 后 canonical contract 携带完整意图合同（scope/依赖快照）。
    let item = &ir.items[0];
    let intent = item
        .contract
        .intent_contract
        .as_ref()
        .expect("intent contract lowered onto canonical contract");
    assert_eq!(intent.intent, crate::product::work_item_contract::WorkItemIntent::Create);
    assert_eq!(intent.provider_work_item_id, "WI-002");
    assert_eq!(intent.depends_on, vec!["WI-002".to_string()]);
    assert_eq!(intent.exclusive_scopes, vec!["src/levels_new/**".to_string()]);
    assert_eq!(intent.forbidden_scopes, vec!["web/**".to_string()]);
    assert_eq!(
        intent.target,
        crate::product::logical_codebase::EnrollmentTarget::SingleRepository {
            repository_id: C1_TARGET_REPO.to_string(),
        }
    );

    // 显式 intent 校验通过（基线外写面已由 create 声明）。
    crate::product::work_item_plan_compiler::validate_work_item_intent_contract(
        &ir,
        &c1_context(&[], Some(&c1_baseline_tree())),
    )
    .expect("declared create must pass intent validation");

    // 全链 validate：无 Error finding（preflight 三族 + intent 均满足）。
    let report = validate_plan_candidate_ir(
        &ir,
        &c1_context(&[], Some(&c1_baseline_tree())),
    )
    .expect("declared create must validate through mechanical report");
    assert!(
        !report.has_errors(),
        "declared create findings must be Error-free: {:#?}",
        report.findings
    );

    // IR 经真实 source store 落盘/回读后意图保留（binding 可追溯）。
    let root = tempfile::TempDir::new().expect("temp root");
    let app_paths = crate::product::app_paths::ProductAppPaths::new(root.path());
    let source_store = crate::product::work_item_plan_source_store::WorkItemPlanSourceStore::new(app_paths);
    let mut source_record = crate::product::work_item_plan_source_store::SourceRevisionRecord {
        id: "source-c1-create".to_string(),
        source: source.clone(),
        source_revision_hash: ir.source_revision_hash.clone(),
        content_hash: String::new(),
    };
    source_record.content_hash = source_record.content_hash().expect("source content hash");
    let source_ref = source_store
        .put_source_revision("project_c1_0001", "issue_c1_0001", "plan_c1_0001", &source_record)
        .expect("persist source");
    let mut ir_record = crate::product::work_item_plan_source_store::PlanCandidateIrRecord {
        id: "ir-c1-create".to_string(),
        source_revision_id: "source-c1-create".to_string(),
        ir: ir.clone(),
        content_hash: String::new(),
    };
    ir_record.content_hash = ir_record.content_hash().expect("ir content hash");
    let ir_ref = source_store
        .put_plan_candidate_ir("project_c1_0001", "issue_c1_0001", "plan_c1_0001", &ir_record)
        .expect("persist ir");
    let reloaded = source_store
        .get_plan_candidate_ir(
            &crate::product::work_item_plan_source_store::SourceStoreScope {
                project_id: "project_c1_0001".to_string(),
                issue_id: "issue_c1_0001".to_string(),
                plan_id: "plan_c1_0001".to_string(),
            },
            &ir_ref,
        )
        .expect("reload ir");
    assert_eq!(
        reloaded.ir.items[0].contract.intent_contract,
        ir.items[0].contract.intent_contract,
        "保存后的 revision 必须原样保留 create 意图"
    );
}

/// 未声明：基线外新建写面 + 无 Plan Intent → `intent_undeclared`
/// （区分“未声明”，进入修订等待；不产生 work item/compile child）。
#[test]
fn c1_existing_create_undeclared_baseline_writes_stop_for_revision() {
    let source = c1_source("", ("src/levels_new/**", "web/**"), "WI-002");
    let ir = compile_work_item_plan(
        &source,
        &WorkItemPlanSourceContext {
            target_repository_id: C1_TARGET_REPO.to_string(),
        },
    )
    .expect("plan without intent must still lower (intent is optional at grammar level)");
    assert!(
        ir.items[0].contract.intent_contract.is_none(),
        "未声明意图时 contract 不得伪造 intent"
    );

    // 显式校验：intent_undeclared。
    let diagnostics = crate::product::work_item_plan_compiler::validate_work_item_intent_contract(
        &ir,
        &c1_context(&[], Some(&c1_baseline_tree())),
    )
    .expect_err("undeclared baseline writes must fail intent validation");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "intent_undeclared"),
        "expected intent_undeclared, got {diagnostics:#?}"
    );

    // 全链 validate：intent_undeclared 作为 preflight 族 Error 保留在报告里
    // （author 修订轮回灌，不硬失败、不清既有 finding），候选不得发布。
    let report = validate_plan_candidate_ir(&ir, &c1_context(&[], Some(&c1_baseline_tree())))
        .expect("preflight-family findings stay in the mechanical report");
    assert!(report.has_errors());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "intent_undeclared"
                && finding.work_item_ids.contains(&"WI-001".to_string())),
        "report must carry intent_undeclared for WI-001: {:#?}",
        report.findings
    );

    // 基线不可用不触发（与 acceptance_path_not_in_baseline 同口径）。
    crate::product::work_item_plan_compiler::validate_work_item_intent_contract(
        &ir,
        &c1_context_with_target(&[], None, None),
    )
    .expect("baseline unavailable must not trigger the undeclared check");
    // 非 enrolled（enrollment target 缺席）不触发——Manual/旧路径零回归。
    crate::product::work_item_plan_compiler::validate_work_item_intent_contract(
        &ir,
        &c1_context_with_target(&[], Some(&c1_baseline_tree()), None),
    )
    .expect("non-enrolled sessions must not trigger the undeclared check");
}

/// 不能执行：错 provider、缺 dependency、scope 冲突、target 不符 →
/// `intent_unexecutable`；既有 finding 不清空（缺失诊断叠加呈现）。
#[test]
fn c1_existing_create_unexecutable_contract_is_rejected_with_facts() {
    let compile = |intent: &str, depends_on: &str| {
        compile_work_item_plan(
            &c1_source(intent, ("src/levels_new/**", "web/**"), depends_on),
            &WorkItemPlanSourceContext {
                target_repository_id: C1_TARGET_REPO.to_string(),
            },
        )
        .expect("lower")
    };

    // 1. 错 provider Work Item：create 的 provider 不在本 plan items。
    let ir = compile(
        r#"
### Plan Intent
- intent: create
- provider_work_item_id: WI-404
- intent_target_kind: single_repository
- intent_target_repo: repo-c1
"#,
        "WI-002",
    );
    let diagnostics = crate::product::work_item_plan_compiler::validate_work_item_intent_contract(
        &ir,
        &c1_context(&[], Some(&c1_baseline_tree())),
    )
    .expect_err("unknown provider must be unexecutable");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == "intent_unexecutable" && d.message.contains("WI-404")),
        "got {diagnostics:#?}"
    );

    // 2. 缺 dependency：compile 后 provider/依赖目标 item 被移出本 plan
    //（parse 期已保证文档内依赖闭合，悬空只可能来自 IR 后续裁剪）。
    let mut ir = compile(CREATE_INTENT, "WI-002");
    ir.items.truncate(1);
    let diagnostics = crate::product::work_item_plan_compiler::validate_work_item_intent_contract(
        &ir,
        &c1_context(&[], Some(&c1_baseline_tree())),
    )
    .expect_err("dangling dependency must be unexecutable");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == "intent_unexecutable" && d.message.contains("WI-002")),
        "got {diagnostics:#?}"
    );

    // 3. exclusive/forbidden scope 冲突：同一 scope 出现在两表。
    let ir = compile_work_item_plan(
        &c1_source(
            CREATE_INTENT,
            ("src/levels_new/**", "src/levels_new/**"),
            "WI-002",
        ),
        &WorkItemPlanSourceContext {
            target_repository_id: C1_TARGET_REPO.to_string(),
        },
    )
    .expect("lower");
    let diagnostics = crate::product::work_item_plan_compiler::validate_work_item_intent_contract(
        &ir,
        &c1_context(&[], Some(&c1_baseline_tree())),
    )
    .expect_err("scope conflict must be unexecutable");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == "intent_unexecutable" && d.message.contains("src/levels_new")),
        "got {diagnostics:#?}"
    );

    // 4. target 不符：enrollment 绑定 target 下 intent target repo ≠ 编译目标。
    let ir = compile(
        r#"
### Plan Intent
- intent: create
- provider_work_item_id: WI-002
- intent_target_kind: single_repository
- intent_target_repo: repo-other
"#,
        "WI-002",
    );
    let diagnostics = crate::product::work_item_plan_compiler::validate_work_item_intent_contract(
        &ir,
        &c1_context(&[], Some(&c1_baseline_tree())),
    )
    .expect_err("target mismatch must be unexecutable");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == "intent_unexecutable" && d.message.contains("repo-other")),
        "got {diagnostics:#?}"
    );

    // existing 只引用既有：provider 不在 durable 既有集 → unexecutable；
    // 命中既有集 → 通过。
    let ir = compile(
        r#"
### Plan Intent
- intent: existing
- provider_work_item_id: wi_existing_0001
- intent_target_kind: single_repository
- intent_target_repo: repo-c1
"#,
        "WI-002",
    );
    let diagnostics = crate::product::work_item_plan_compiler::validate_work_item_intent_contract(
        &ir,
        &c1_context(&[], Some(&c1_baseline_tree())),
    )
    .expect_err("existing must resolve an authorized existing work item");
    assert!(
        diagnostics.iter().any(|d| d.code == "intent_unexecutable"),
        "got {diagnostics:#?}"
    );
    let existing = vec!["wi_existing_0001".to_string()];
    crate::product::work_item_plan_compiler::validate_work_item_intent_contract(
        &ir,
        &c1_context(&existing, Some(&c1_baseline_tree())),
    )
    .expect("existing provider that resolves must pass");
}
