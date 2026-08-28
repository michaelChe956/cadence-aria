use super::{InitialPlanCompileDurableContext, prepare_initial_plan_compile};
use crate::product::models::IssueWorkItemPlan;
use crate::product::work_item_contract::canonical_contract_fixture;
use crate::product::work_item_plan_compiler::{
    PlanCandidateIr, PlanCandidateItemIr, PlanCandidateMechanicalReport,
    PlanCandidatePublicationProvenance, WORK_ITEM_PLAN_COMPILER_VERSION,
};
use crate::product::workspace_engine::compile::ir_adapter::{
    IrCompileAdapterContext, durable_compile_context_from_ir, initial_plan_compile_input_from_ir,
};

fn context() -> IrCompileAdapterContext {
    IrCompileAdapterContext {
        project_id: "project_001".to_string(),
        issue_id: "issue_001".to_string(),
        plan_id: "plan_001".to_string(),
        previous_plan: IssueWorkItemPlan {
            id: "plan_001".to_string(),
            project_id: "project_001".to_string(),
            issue_id: "issue_001".to_string(),
            source_story_spec_ids: Vec::new(),
            source_design_spec_ids: Vec::new(),
            options: crate::product::models::IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            status: crate::product::models::IssueWorkItemPlanStatus::Draft,
            work_item_ids: Vec::new(),
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
            review_summary: None,
            created_at: "2026-08-27T00:00:00Z".to_string(),
            updated_at: "2026-08-27T00:00:00Z".to_string(),
        },
        source_revision_id: "source_001".to_string(),
        source_revision_ref:
            "project/project_001/issue/issue_001/plan/plan_001/source_revision/source_001"
                .to_string(),
        plan_candidate_ir_ref:
            "project/project_001/issue/issue_001/plan/plan_001/plan_candidate_ir/ir_001".to_string(),
        mechanical_report_ref:
            "project/project_001/issue/issue_001/plan/plan_001/mechanical_report/report_001"
                .to_string(),
        publication_provenance_ref:
            "project/project_001/issue/issue_001/plan/plan_001/publication_provenance/compile_001"
                .to_string(),
        logical_targets: None,
        repository_id: "repository_001".to_string(),
        change_order: Vec::new(),
        compile_id: "compile_001".to_string(),
        now: "2026-08-27T00:00:00Z".to_string(),
    }
}

fn ir() -> PlanCandidateIr {
    let mut contract = canonical_contract_fixture("logical_001");
    contract.input_contracts.clear();
    PlanCandidateIr {
        source_revision_hash: "source-hash".to_string(),
        compiler_version: WORK_ITEM_PLAN_COMPILER_VERSION.to_string(),
        items: vec![PlanCandidateItemIr {
            target_repository_id: "repository_001".to_string(),
            contract,
            verification_plan: crate::product::models::WorkItemDraftVerificationPlan {
                checks: Vec::new(),
            },
            trusted_commands: Vec::new(),
        }],
    }
}

fn report() -> PlanCandidateMechanicalReport {
    PlanCandidateMechanicalReport {
        source_revision_hash: "source-hash".to_string(),
        compiler_version: WORK_ITEM_PLAN_COMPILER_VERSION.to_string(),
        findings: Vec::new(),
    }
}

#[test]
fn ir_adapter_constructs_deterministic_typed_initial_compile_input() {
    let context = context();
    let input = initial_plan_compile_input_from_ir(&context, &ir(), &report()).unwrap();
    assert_eq!(input.compile_id, "compile_001");
    assert_eq!(input.now, "2026-08-27T00:00:00Z");
    assert_eq!(input.outline_order, vec!["logical_001"]);
    assert_eq!(
        input.draft_records[0].status,
        crate::product::models::WorkItemDraftStatus::Accepted
    );
    assert_eq!(input.repository_id, "repository_001");
    assert_eq!(input.logical_targets, None);
    let prepared = prepare_initial_plan_compile(input, InitialPlanCompileDurableContext::legacy());
    assert!(prepared.is_ok(), "prepare failed: {:?}", prepared.err());
}

#[test]
fn ir_adapter_rejects_mechanical_error_before_pure_prepare() {
    let context = context();
    let mut report = report();
    report
        .findings
        .push(crate::product::models::WorkItemSplitFinding {
            severity: crate::product::models::WorkItemSplitFindingSeverity::Error,
            code: "mechanical_error".to_string(),
            message: "invalid candidate".to_string(),
            work_item_ids: Vec::new(),
        });
    assert!(initial_plan_compile_input_from_ir(&context, &ir(), &report).is_err());
}

#[test]
fn work_item_plan_initial_compile_ir_adapter_rejects_context_identity_drift() {
    let mut context = context();
    context.plan_id = "other_plan".to_string();
    assert!(initial_plan_compile_input_from_ir(&context, &ir(), &report()).is_err());
}

#[test]
fn work_item_plan_initial_compile_ir_adapter_rejects_empty_ir() {
    let context = context();
    let mut empty = ir();
    empty.items.clear();
    assert!(initial_plan_compile_input_from_ir(&context, &empty, &report()).is_err());
}

#[test]
fn work_item_plan_initial_compile_ir_adapter_rejects_missing_logical_target() {
    let mut context = context();
    context.logical_targets = Some(std::collections::BTreeMap::new());
    assert!(initial_plan_compile_input_from_ir(&context, &ir(), &report()).is_err());
}

#[test]
fn work_item_plan_initial_compile_ir_adapter_rejects_duplicate_logical_identity() {
    let context = context();
    let mut duplicated = ir();
    duplicated.items.push(duplicated.items[0].clone());
    assert!(initial_plan_compile_input_from_ir(&context, &duplicated, &report()).is_err());
}

#[test]
fn work_item_plan_initial_compile_ir_adapter_preserves_change_order_in_input() {
    let mut context = context();
    let first = crate::product::logical_codebase::LogicalRepositoryId(uuid::Uuid::from_u128(1));
    context.change_order = vec![first];
    let input = initial_plan_compile_input_from_ir(&context, &ir(), &report()).unwrap();
    assert_eq!(input.change_order, vec![first]);
}

#[test]
fn ir_adapter_provenance_context_requires_exact_reservation_values() {
    let context = context();
    let provenance = PlanCandidatePublicationProvenance {
        id: "compile_001".to_string(),
        plan_id: "plan_001".to_string(),
        plan_revision_id: "plan_revision_001".to_string(),
        source_revision_ref: context.source_revision_ref.clone(),
        plan_candidate_ir_ref: context.plan_candidate_ir_ref.clone(),
        mechanical_report_ref: context.mechanical_report_ref.clone(),
        source_revision_hash: "source-hash".to_string(),
        compiler_version: WORK_ITEM_PLAN_COMPILER_VERSION.to_string(),
        published_at: context.now.clone(),
        content_hash: "hash".to_string(),
    };
    assert!(durable_compile_context_from_ir(&context, &provenance).is_ok());
    let mut wrong = provenance;
    wrong.published_at = "2026-08-28T00:00:00Z".to_string();
    assert!(durable_compile_context_from_ir(&context, &wrong).is_err());
}
