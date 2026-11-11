//! REQ-CRP-02/03：Plan Intent 分支等价解析 + lowering typed fault 定向测试
//!（工作包 1.1/1.2/1.3/2.2 编译器侧）。
//!
//! - `single_repository` 值填在 `intent_target_logical_repo` 时唯一命中成员表
//!   才采用，audit 恰 1 条（`original_key`/`value`/`resolution_basis` 钉死）；
//! - 值不在成员表 / 双命中（成员逻辑 key ∩ 另一成员物理仓值）一律
//!   fail-closed 拒绝，诊断携带 typed `BranchKeyMismatch`（行号取实际行，
//!   禁止 0 伪行），绝不取第一个命中；
//! - legacy（`member_targets=None`）非聚合视图会话 target 才回落（basis
//!   「session target」）；聚合根视图 ID typed 拒绝（不猜）；
//! - `logical_codebase` 缺 `intent_target_codebase` 仍 typed 报错
//!   （`RequiredFieldAbsent`，不从会话事实臆造）。

use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::work_item_plan_compiler::types::{CompilerDiagnostic, LoweringFault};
use crate::product::work_item_plan_compiler::{
    PlanMemberTargets, WorkItemPlanSourceContext, compile_work_item_plan_audited,
};

/// 成员 A 的逻辑仓 UUID（两成员表的逻辑 key 之一）。
const LOGICAL_MEMBER_A: &str = "00000000-0000-4000-8000-000000000001";
/// 成员 B 的逻辑仓 UUID。
const LOGICAL_MEMBER_B: &str = "00000000-0000-4000-8000-000000000002";
/// 聚合根视图 ID（B1 铁律：不得流入 IR target）。
const AGGREGATE_VIEW_ID: &str = "lc_aggregate_root_view_00000000-0000-4000-8000-0000000000aa";

fn logical(id: &str) -> LogicalRepositoryId {
    LogicalRepositoryId(id.parse().expect("logical member uuid"))
}

/// 两成员表（1.1 口径）：A→phys-busi、B→phys-web。
fn two_member_targets() -> PlanMemberTargets {
    let mut physical_by_logical = std::collections::BTreeMap::new();
    physical_by_logical.insert(logical(LOGICAL_MEMBER_A), "phys-busi".to_string());
    physical_by_logical.insert(logical(LOGICAL_MEMBER_B), "phys-web".to_string());
    PlanMemberTargets {
        physical_by_logical,
        fallback_physical_id: None,
    }
}

fn member_context(
    member_targets: Option<PlanMemberTargets>,
    target_repository_id: &str,
) -> WorkItemPlanSourceContext {
    WorkItemPlanSourceContext {
        target_repository_id: target_repository_id.to_string(),
        member_targets,
    }
}

/// 单 WI source：除 `<<INTENT_TARGET>>` 占位的 Plan Intent target 字段外
/// 骨架恒定（Identity/Goal 必填齐全，可完整 lower）。
fn plan_source(intent_target_fields: &str) -> String {
    const TEMPLATE: &str = r#"# Work Item Plan

## Work Item WI-001: Backend levels API

### Identity
- schema_version: 1
- logical_work_item_id: WI-001
- title: Backend levels API
- kind: backend

### Goal
- summary: WHEN a level list request arrives THE SYSTEM SHALL return the configured levels JSON.

### Non Goals
- non_goals: Rendering HTML and changing browser assets are out of scope.

### Dependencies
- depends_on: []

### Plan Intent
- intent: create
- provider_work_item_id: WI-001
<<INTENT_TARGET>>

### Inputs

### Outputs
- contract_id: contract.levels-api
- capabilities: api.levels.read

### Tasks
- task_id: TASK-001
- statement: WHEN the levels API receives GET /api/levels THE SYSTEM SHALL return the configured levels JSON.
- requirement_refs: REQ-WSC-02
- done_when_refs: AC-001

### Write Policy
- exclusive_scopes: src/product/levels/**
- forbidden_scopes: web/**

### Acceptance Criteria
- criterion_id: AC-001
- statement: WHEN GET /api/levels is requested THE SYSTEM SHALL return the configured levels JSON.
- required_evidence: source_diff

### Verification
- check_id: CHECK-001
- command: cargo test --locked --lib levels_api
- required: true
- non_zero_test_execution_required: false

### Handoff Schema
- required_fields: commit_sha
- provided_contract_refs: contract.levels-api
- reviewer_check_refs: AC-001

### Blockers
- reason_code: levels_api_contract_invalid
- route: plan_repair_current
- target_contract_refs: contract.levels-api

### Traceability
- source_type: design_spec
- source_id: design_spec_levels_0001
- requirement_id: REQ-WSC-02
"#;
    TEMPLATE.replacen("<<INTENT_TARGET>>", intent_target_fields, 1)
}

/// 断言诊断集携带 BranchKeyMismatch（等价解析 fail-closed 的 typed 形态），
/// 核对 kind/expected/actual/value 四元组，返回 fault 行号（调用方核非 0）。
fn expect_branch_key_mismatch(diagnostics: &[CompilerDiagnostic], expected_value: &str) -> usize {
    let fault = diagnostics
        .iter()
        .find_map(|diagnostic| diagnostic.lowering_fault.clone())
        .expect("fail-closed 诊断必须携带 typed lowering_fault");
    match fault {
        LoweringFault::BranchKeyMismatch {
            kind,
            expected_field,
            actual_field,
            value,
            line,
            ..
        } => {
            assert_eq!(kind, "single_repository");
            assert_eq!(expected_field, "intent_target_repo");
            assert_eq!(actual_field, "intent_target_logical_repo");
            assert_eq!(value, expected_value, "fault 必须携带模型原值");
            line
        }
        other => panic!("必须是 BranchKeyMismatch，实际 {other:?}"),
    }
}

fn logical_repo_line(source: &str) -> usize {
    source
        .lines()
        .position(|line| line.contains("intent_target_logical_repo"))
        .expect("fixture 必须含 intent_target_logical_repo 行")
        + 1
}

#[test]
fn equivalent_target_adopted_with_audit_when_unique_mapping() {
    let source = plan_source(&format!(
        "- intent_target_kind: single_repository\n- intent_target_logical_repo: {LOGICAL_MEMBER_A}"
    ));
    let (ir, audit) = compile_work_item_plan_audited(
        &source,
        &member_context(Some(two_member_targets()), AGGREGATE_VIEW_ID),
    )
    .expect("唯一命中成员表时等价解析必须采用并编译通过");
    assert_eq!(
        ir.items[0].target_repository_id, "phys-busi",
        "IR 的 WI target 必须是等价解析映射到的成员物理仓"
    );
    let canonicalizations = audit.equivalence_canonicalizations;
    assert_eq!(canonicalizations.len(), 1, "审计必须恰 1 条");
    assert_eq!(canonicalizations[0].kind, "intent_target_repository");
    assert_eq!(
        canonicalizations[0].original_key, "intent_target_logical_repo",
        "original_key 必须是模型实际写的 key"
    );
    assert_eq!(canonicalizations[0].value, LOGICAL_MEMBER_A);
    assert!(
        canonicalizations[0]
            .resolution_basis
            .contains("physical_by_logical"),
        "resolution_basis 必须记录成员表映射依据：{}",
        canonicalizations[0].resolution_basis
    );
}

#[test]
fn equivalent_target_value_not_in_member_table_rejected() {
    let unknown = "00000000-0000-4000-8000-000000000099";
    let source = plan_source(&format!(
        "- intent_target_kind: single_repository\n- intent_target_logical_repo: {unknown}"
    ));
    let diagnostics = compile_work_item_plan_audited(
        &source,
        &member_context(Some(two_member_targets()), AGGREGATE_VIEW_ID),
    )
    .expect_err("值不在成员表（且不等于任何物理仓值）必须 fail-closed 拒绝");
    let fault_line = expect_branch_key_mismatch(&diagnostics, unknown);
    assert_eq!(
        fault_line,
        logical_repo_line(&source),
        "fault 行号必须取 intent_target_logical_repo 实际行，禁止 0"
    );
}

#[test]
fn equivalent_target_double_hit_fails_closed() {
    // 成员 B 的物理仓值 == 成员 A 的逻辑 key：值双命中，绝不选其一。
    let mut targets = two_member_targets();
    targets
        .physical_by_logical
        .insert(logical(LOGICAL_MEMBER_B), LOGICAL_MEMBER_A.to_string());
    let source = plan_source(&format!(
        "- intent_target_kind: single_repository\n- intent_target_logical_repo: {LOGICAL_MEMBER_A}"
    ));
    let diagnostics =
        compile_work_item_plan_audited(&source, &member_context(Some(targets), AGGREGATE_VIEW_ID))
            .expect_err("双命中必须 fail-closed 拒绝，不选其一");
    expect_branch_key_mismatch(&diagnostics, LOGICAL_MEMBER_A);
}

#[test]
fn equivalent_target_legacy_without_members_uses_session_target() {
    let source = plan_source(&format!(
        "- intent_target_kind: single_repository\n- intent_target_logical_repo: {LOGICAL_MEMBER_A}"
    ));
    let (ir, audit) =
        compile_work_item_plan_audited(&source, &member_context(None, "repo-session-001"))
            .expect("legacy 无成员表且会话 target 非聚合视图时必须回落会话 target");
    assert_eq!(
        ir.items[0].target_repository_id, "repo-session-001",
        "legacy 等价解析必须采用会话 target"
    );
    assert_eq!(audit.equivalence_canonicalizations.len(), 1);
    assert!(
        audit.equivalence_canonicalizations[0]
            .resolution_basis
            .contains("session target"),
        "basis 必须记 session target：{}",
        audit.equivalence_canonicalizations[0].resolution_basis
    );

    let diagnostics =
        compile_work_item_plan_audited(&source, &member_context(None, AGGREGATE_VIEW_ID))
            .expect_err("会话 target 为聚合根视图时不得猜，必须 typed 拒绝");
    expect_branch_key_mismatch(&diagnostics, LOGICAL_MEMBER_A);
}

#[test]
fn logical_codebase_missing_codebase_still_typed_error() {
    let source = plan_source(&format!(
        "- intent_target_kind: logical_codebase\n- intent_target_logical_repo: {LOGICAL_MEMBER_A}"
    ));
    let diagnostics = compile_work_item_plan_audited(
        &source,
        &member_context(Some(two_member_targets()), AGGREGATE_VIEW_ID),
    )
    .expect_err("logical_codebase 缺 intent_target_codebase 必须 fail-closed");
    let fault = diagnostics
        .iter()
        .find_map(|diagnostic| diagnostic.lowering_fault.clone())
        .expect("缺 codebase 必须携带 typed RequiredFieldAbsent，不从会话事实臆造");
    match fault {
        LoweringFault::RequiredFieldAbsent { field, .. } => {
            assert_eq!(field, "intent_target_codebase");
        }
        other => panic!("必须是 RequiredFieldAbsent，实际 {other:?}"),
    }
}
