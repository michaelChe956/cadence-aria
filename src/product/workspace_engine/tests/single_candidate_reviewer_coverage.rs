use super::*;
use crate::product::json_store::write_json;
use crate::product::models::WorkItemSplitFinding;
use crate::product::work_item_contract::{
    ContractCompatibilityPolicy, PromisedOutputContract, RequiredInputContract,
};
use crate::product::work_item_plan_compiler::{
    PlanCandidateIr, PlanCandidateItemIr, PlanCandidateMechanicalReport,
    WORK_ITEM_PLAN_COMPILER_VERSION,
};
use crate::product::work_item_plan_policy::{ReviewInvocationScope, WorkItemPlanFlowKind};
use crate::product::work_item_plan_source_store::{
    PlanCandidateIrRecord, PlanCandidateMechanicalReportRecord, SourceRevisionRecord,
    WorkItemPlanSourceStore,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

fn source_hash(source: &str) -> String {
    hex::encode(Sha256::digest(source.as_bytes()))
}

#[test]
fn single_candidate_reviewer_prompt_budget_accepts_exact_limit_and_rejects_one_byte_over() {
    let exact = "x".repeat(SINGLE_CANDIDATE_REVIEW_PROMPT_MAX_BYTES);
    ensure_single_candidate_review_prompt_budget(&exact).expect("exact limit allowed");
    let over = format!("{exact}x");
    let error = ensure_single_candidate_review_prompt_budget(&over).expect_err("over limit fails");
    assert!(error.contains("65537"));
    assert!(error.contains("65536"));
}

#[test]
fn single_candidate_reviewer_coverage_prompt_contains_projection_and_gap_teaching() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        super::make_work_item_plan_engine_with_draft_candidate("reviewer_coverage_gap");
    let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
    let source = "# immutable coverage source\n";
    let mut source_revision = SourceRevisionRecord {
        id: "source-coverage".to_string(),
        source: source.to_string(),
        source_revision_hash: source_hash(source),
        content_hash: String::new(),
    };
    source_revision.content_hash = source_revision.content_hash().expect("source hash");
    source_store
        .put_source_revision("project_0001", "issue_0001", &plan_id, &source_revision)
        .expect("persist source revision");

    let mut provider = crate::product::work_item_contract::canonical_contract_fixture("WI-01");
    provider.input_contracts.clear();
    provider.output_contracts = vec![PromisedOutputContract {
        contract_id: "contract.workflow".to_string(),
        capabilities: vec!["capability.present".to_string()],
    }];
    provider.handoff_contract.provided_contract_refs = vec!["CT-005".to_string()];
    let mut consumer = crate::product::work_item_contract::canonical_contract_fixture("WI-02");
    consumer.input_contracts = vec![RequiredInputContract {
        contract_id: "contract.workflow".to_string(),
        provider_logical_work_item_id: "WI-01".to_string(),
        required_capabilities: vec!["capability.missing".to_string()],
        compatibility_policy: ContractCompatibilityPolicy::RequireAll,
    }];
    consumer.output_contracts.clear();
    let ir = PlanCandidateIr {
        source_revision_hash: source_revision.source_revision_hash.clone(),
        compiler_version: WORK_ITEM_PLAN_COMPILER_VERSION.to_string(),
        items: vec![
            PlanCandidateItemIr {
                target_repository_id: "repository_0001".to_string(),
                contract: provider,
                verification_plan: crate::product::models::WorkItemDraftVerificationPlan {
                    checks: Vec::new(),
                },
                trusted_commands: Vec::new(),
            },
            PlanCandidateItemIr {
                target_repository_id: "repository_0001".to_string(),
                contract: consumer,
                verification_plan: crate::product::models::WorkItemDraftVerificationPlan {
                    checks: Vec::new(),
                },
                trusted_commands: Vec::new(),
            },
        ],
    };
    let mut ir_record = PlanCandidateIrRecord {
        id: "ir-coverage".to_string(),
        source_revision_id: source_revision.id.clone(),
        ir,
        content_hash: String::new(),
    };
    ir_record.content_hash = ir_record.content_hash().expect("IR hash");
    let ir_ref = source_store
        .put_plan_candidate_ir("project_0001", "issue_0001", &plan_id, &ir_record)
        .expect("persist IR");
    let mut report_record = PlanCandidateMechanicalReportRecord {
        id: "report-coverage".to_string(),
        source_revision_id: source_revision.id,
        ir_id: ir_record.id,
        report: PlanCandidateMechanicalReport {
            source_revision_hash: ir_record.ir.source_revision_hash.clone(),
            compiler_version: ir_record.ir.compiler_version.clone(),
            findings: vec![WorkItemSplitFinding {
                severity: crate::product::models::WorkItemSplitFindingSeverity::Warning,
                code: "coverage_warning".to_string(),
                message: "coverage fixture mechanical evidence".to_string(),
                work_item_ids: vec!["WI-02".to_string()],
            }],
        },
        content_hash: String::new(),
    };
    report_record.content_hash = report_record.content_hash().expect("report hash");
    let report_ref = source_store
        .put_mechanical_report("project_0001", "issue_0001", &plan_id, &report_record)
        .expect("persist report");

    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("load session");
    record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    record.plan_candidate_ir_ref = Some(ir_ref);
    record.mechanical_report_ref = Some(report_ref);
    record.review_invocation_scope =
        Some(ReviewInvocationScope::initial("revision-reviewer-coverage"));
    write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist session refs");
    engine.session = WorkspaceSession::from_record(record);

    let input = engine
        .build_review_input()
        .expect("single-candidate reviewer input");
    assert!(input.prompt.len() <= SINGLE_CANDIDATE_REVIEW_PROMPT_MAX_BYTES);
    eprintln!(
        "single-candidate reviewer prompt bytes={} margin={}",
        input.prompt.len(),
        SINGLE_CANDIDATE_REVIEW_PROMPT_MAX_BYTES - input.prompt.len()
    );
    assert!(
        input
            .prompt
            .contains("服务端审核 invocation scope（Initial）")
    );
    assert!(
        input
            .prompt
            .contains("Reviewer Capability Coverage Projection")
    );
    let mut verification_record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("reload durable session for verification scope");
    verification_record.review_invocation_scope = Some(ReviewInvocationScope::verification(
        BTreeSet::new(),
        "ir-coverage",
        "report-coverage",
    ));
    write_json(
        &lifecycle
            .app_paths()
            .issue_root(
                &verification_record.project_id,
                &verification_record.issue_id,
            )
            .join("workspace-sessions")
            .join(format!("{}.json", verification_record.id)),
        &verification_record,
    )
    .expect("persist verification scope");
    engine.session = WorkspaceSession::from_record(verification_record);
    let verification_input = engine
        .build_review_input()
        .expect("single-candidate verification reviewer input");
    assert!(
        verification_input
            .prompt
            .contains("Reviewer Capability Coverage Projection")
    );
    assert!(
        verification_input
            .prompt
            .contains("服务端审核 invocation scope（Verification）")
    );
    assert!(
        verification_input.prompt.len() <= SINGLE_CANDIDATE_REVIEW_PROMPT_MAX_BYTES,
        "verification prompt exceeds byte budget: {} > {}",
        verification_input.prompt.len(),
        SINGLE_CANDIDATE_REVIEW_PROMPT_MAX_BYTES
    );
    for field in [
        "from",
        "to",
        "contract_id",
        "required_capabilities",
        "provided_capabilities",
        "missing_capabilities",
        "compatibility_policy",
        "dependency_graph",
        "depends_on",
        "declared_edges",
        "contract_edges",
        "cycles",
        "duplicate_edges",
        "unknown_providers",
        "handoff_consumption",
        "consumers",
        "consumed",
        "write_scope_conflicts",
    ] {
        assert!(
            input.prompt.contains(field),
            "missing projection field {field}"
        );
    }
    for teaching in [
        "must_fix",
        "contract_gap",
        "evidence",
        "WI-01 -> WI-02",
        "capability.missing",
        "CT-005",
        "consumers: []",
        "consumed: false",
        "severity=must_fix",
        "category=contract_gap",
        "class_hint=repairable",
        "evidence",
        "实际 nonce 开始标签",
        "必须是输出第一行",
        "禁止任何前言、声明、路由回执、说明、空白行或代码围栏",
    ] {
        assert!(
            input.prompt.contains(teaching),
            "missing teaching {teaching}"
        );
    }
}
