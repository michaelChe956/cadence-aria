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
            ever_confirmed: None,
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
            // F1.2：fixture 沿旧 record 形态（kind=None）。
            kind: None,
        });
    let journal = crate::product::work_item_plan_store::observe_compile_transaction_writes();
    assert!(initial_plan_compile_input_from_ir(&context, &ir(), &report).is_err());
    assert!(
        journal.snapshots().is_empty(),
        "stale/mechanical-error IR must fail before the first transaction put"
    );
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

// —— SG3b：Final Compile typed 失败与冻结结构复验纯判定 ——

fn sg3b_structure_context(
    digest: &str,
) -> crate::product::workspace_engine::draft_batch::compile_support::PlanningStructureContext {
    use crate::product::workspace_engine::draft_batch::compile_support::PlanningStructureContext;
    PlanningStructureContext {
        source_revision_ref: "project/p/issue/i/plan/pl/source_revision/s".to_string(),
        plan_candidate_ir_ref: "project/p/issue/i/plan/pl/plan_candidate_ir/ir".to_string(),
        source_hash: "source-hash".to_string(),
        compiler_version: WORK_ITEM_PLAN_COMPILER_VERSION.to_string(),
        rule_version:
            crate::product::workspace_engine::plan_projection::PLANNING_STRUCTURE_RULE_VERSION
                .to_string(),
        context_digest: digest.to_string(),
        confirmed_design_id: None,
        confirmed_design_digest: None,
        member_table_digest: None,
        selection_digest: None,
        physical_by_logical: Default::default(),
        change_order: Vec::new(),
        options_digest: "options-digest".to_string(),
        baseline_tree_digest: None,
        intent_digest: None,
    }
}

fn sg3b_frozen_payload(
    rule: &str,
    digest: &str,
) -> crate::product::planning_failure::PersistedPlanningStructureReport {
    crate::product::planning_failure::PersistedPlanningStructureReport {
        source_revision_id: "source-sg3b".to_string(),
        source_hash: "source-hash".to_string(),
        ir_id: "ir-sg3b".to_string(),
        compiler_version: WORK_ITEM_PLAN_COMPILER_VERSION.to_string(),
        rule_version: rule.to_string(),
        context_digest: digest.to_string(),
        effective_edges: Vec::new(),
        cycle_witness: Vec::new(),
        findings: Default::default(),
        structurally_valid: true,
        protected_predicates: Vec::new(),
        instruction_refs: Vec::new(),
        repair_audit_refs: Vec::new(),
        report_digest: "digest-sealed-by-store".to_string(),
    }
}

#[test]
fn sg_03b_frozen_structure_gate_rule_and_drift_pure_checks() {
    let current = sg3b_structure_context("ctx-live");
    // None（旧记录参考形态）不要求冻结 digest。
    assert!(super::single_candidate::verify_frozen_structure_context(None, &current).is_ok());
    // 同代（rule + context digest 一致）放行。
    assert!(
        super::single_candidate::verify_frozen_structure_context(
            Some(&sg3b_frozen_payload(
                crate::product::workspace_engine::plan_projection::PLANNING_STRUCTURE_RULE_VERSION,
                "ctx-live"
            )),
            &current
        )
        .is_ok()
    );
    // 规则版本过期：typed Operational 拒绝，stable code 单源。
    let stale = super::single_candidate::verify_frozen_structure_context(
        Some(&sg3b_frozen_payload(
            "rule/planning-structure@0",
            "ctx-live",
        )),
        &current,
    )
    .expect_err("stale rule must refuse");
    assert_eq!(stale.stable_code(), "single_candidate_structure_rule_stale");
    // 冻结 context 漂移：typed Operational 拒绝（拒不重 lower）。
    let drift = super::single_candidate::verify_frozen_structure_context(
        Some(&sg3b_frozen_payload(
            crate::product::workspace_engine::plan_projection::PLANNING_STRUCTURE_RULE_VERSION,
            "ctx-frozen",
        )),
        &current,
    )
    .expect_err("context drift must refuse");
    assert_eq!(
        drift.stable_code(),
        "single_candidate_structure_context_drift"
    );
    assert!(drift.render().contains("ctx-frozen"), "{}", drift.render());
    assert!(drift.render().contains("ctx-live"), "{}", drift.render());
}

#[test]
fn sg_03b_planning_compile_failure_identity_seal_is_deterministic() {
    use crate::product::workspace_engine::types::PlanningCompileFailure;
    let mut failure = PlanningCompileFailure::operational(
        "SOURCE_STORE_MALFORMED_REF",
        "single candidate source revision ref is missing".to_string(),
    );
    assert_eq!(failure.stable_code(), "SOURCE_STORE_MALFORMED_REF");
    assert!(
        failure.render().contains("[SOURCE_STORE_MALFORMED_REF]"),
        "{}",
        failure.render()
    );

    failure.seal_with_identity(
        "workspace_session_sg3b",
        Some("approval-1".to_string()),
        Some("source-ref".to_string()),
        Some("ir-ref".to_string()),
        Some("report-ref".to_string()),
    );
    let first_id = failure.failure_id.clone();
    assert!(
        first_id.starts_with("planning_compile_failure_"),
        "{first_id}"
    );

    // 同输入幂等同 id；身份字段变化 → 新 id。
    let mut replay = PlanningCompileFailure::operational(
        "SOURCE_STORE_MALFORMED_REF",
        "single candidate source revision ref is missing".to_string(),
    );
    replay.seal_with_identity(
        "workspace_session_sg3b",
        Some("approval-1".to_string()),
        Some("source-ref".to_string()),
        Some("ir-ref".to_string()),
        Some("report-ref".to_string()),
    );
    assert_eq!(replay.failure_id, first_id);
    replay.seal_with_identity(
        "workspace_session_sg3b",
        Some("approval-2".to_string()),
        Some("source-ref".to_string()),
        Some("ir-ref".to_string()),
        Some("report-ref".to_string()),
    );
    assert_ne!(replay.failure_id, first_id);

    // Structure kind：稳定 code 单源。
    let structure = PlanningCompileFailure::structure(Default::default());
    assert_eq!(structure.stable_code(), "final_compile_structure_rejected");
}

// ─── fix-cascade-deletion Task D2.1b：compile及恢复lease完整覆盖 ───
//
// REQ-DEL-06（plan D2.1b Step 1）：Final Compile（legacy/SC）与恢复入口持
// owning Plan run lease——preparing→commit 各 checkpoint 删除独占立即 Busy、
// 零墓碑（无 attempt 也不能清运行中 compile）；resume 还在跑同样 Busy；
// 删除赢（purging）后恢复动作先于任何 durable 读/写稳定拒绝（不写
// transaction/WorkItems/source/provenance）；正常链路 published refs/
// hash/CAS 仍同代。

use std::sync::{Arc, Mutex};

use super::{WorkItemPlanCompileAdmission, WorkItemPlanCompileFinalizerCheckpoint};
use crate::product::app_paths::ProductAppPaths;
use crate::product::entity_deletion_store::{
    DeletionEntityKind, DeletionPhase, EntityDeletionStore, EntityScope,
};
use crate::product::entity_operation_store::{EntityOperationError, EntityOperationStore};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    SingleCandidatePhase, WorkItemPlanCommitState, WorkItemPlanCompileStatus,
};
use crate::product::work_item_plan_policy::RunPolicy;
use crate::product::work_item_plan_source_store::{SourceStoreScope, WorkItemPlanSourceStore};
use crate::product::workspace_engine::WorkspaceEngine;
use crate::product::workspace_engine::tests::make_work_item_plan_engine_with_accepted_contract_drafts;
use crate::product::workspace_engine::tests::single_candidate_recovery::single_candidate_recovery_record;
use crate::web::workspace_ws_types::WorkItemPlanCompileRecoveryActionDto;

type D21bProbeViolations = Arc<Mutex<Vec<String>>>;
type D21bProbeFired = Arc<Mutex<std::collections::BTreeSet<String>>>;

/// 注册「运行中 compile 该 checkpoint 删除必须 Busy 且零墓碑」探针。
fn d21b_register_delete_busy_probe(
    app_paths: &ProductAppPaths,
    plan_id: &str,
    checkpoint: &str,
    violations: &D21bProbeViolations,
    fired: &D21bProbeFired,
) -> super::WorkItemPlanCompileAdmissionProbeGuard {
    let app_paths_capture = app_paths.clone();
    let plan_capture = plan_id.to_string();
    let checkpoint_capture = checkpoint.to_string();
    let violations = violations.clone();
    let fired = fired.clone();
    super::register_work_item_plan_compile_admission_probe(
        "project_0001",
        "issue_0001",
        plan_id,
        checkpoint,
        move || {
            let scope = EntityScope::new(
                "project_0001",
                "issue_0001",
                DeletionEntityKind::Plan,
                &plan_capture,
            );
            let operations = EntityOperationStore::new(app_paths_capture.clone());
            match operations.try_acquire_delete(&scope) {
                Err(EntityOperationError::Busy) => {}
                Err(other) => violations.lock().unwrap().push(format!(
                    "{checkpoint_capture}: 运行中 compile 期间删除必须 Busy，got Err({other:?})"
                )),
                Ok(_) => violations.lock().unwrap().push(format!(
                    "{checkpoint_capture}: 运行中 compile 期间删除不得成功（必须 Busy）"
                )),
            }
            if EntityDeletionStore::new(app_paths_capture.clone())
                .get(&scope)
                .unwrap()
                .is_some()
            {
                violations
                    .lock()
                    .unwrap()
                    .push(format!("{checkpoint_capture}: busy 拒绝必须零墓碑"));
            }
            fired.lock().unwrap().insert(checkpoint_capture.clone());
        },
    )
}

fn d21b_snapshot_tree(root: &std::path::Path) -> Vec<(String, u64)> {
    fn walk(dir: &std::path::Path, out: &mut Vec<(String, u64)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else {
                let len = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                out.push((
                    path.file_name().unwrap().to_string_lossy().into_owned(),
                    len,
                ));
            }
        }
    }
    let mut out = Vec::new();
    walk(root, &mut out);
    out.sort();
    out
}

#[tokio::test]
async fn cascade_d2_1b_compile_and_recovery_lease() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    let app_paths = lifecycle.app_paths();
    let operations = EntityOperationStore::new(app_paths.clone());
    let deletions = EntityDeletionStore::new(app_paths.clone());
    let scope = EntityScope::new(
        "project_0001",
        "issue_0001",
        DeletionEntityKind::Plan,
        &plan_id,
    );

    // (a) 准入契约面：compile admission 在持 → 删除独占立即 Busy、零墓碑
    //     （删除拒绝是零破坏拒绝）。
    let admission =
        WorkItemPlanCompileAdmission::acquire(&app_paths, "project_0001", "issue_0001", &plan_id)
            .expect("live plan compile admission");
    match operations.try_acquire_delete(&scope) {
        Err(EntityOperationError::Busy) => {}
        Err(other) => panic!("expected Busy, got: {other:?}"),
        Ok(_) => panic!("delete must be busy while the compile admission is held"),
    }
    assert!(
        deletions.get(&scope).unwrap().is_none(),
        "busy 拒绝必须零墓碑"
    );
    drop(admission);

    // (b) 无 attempt 前提 + 全 checkpoint 探针：legacy preparing→commit 各
    //     checkpoint 删除 Busy 且零墓碑。
    assert!(
        crate::product::coding_attempt_store::CodingAttemptStore::new(app_paths.clone())
            .list_attempts_for_issue("project_0001", "issue_0001")
            .unwrap()
            .is_empty(),
        "夹具必须零 coding attempts（无 attempt 也不能清运行中 compile）"
    );
    let violations: D21bProbeViolations = Arc::new(Mutex::new(Vec::new()));
    let fired: D21bProbeFired = Arc::new(Mutex::new(Default::default()));
    let checkpoints = [
        "preparing",
        "validating",
        "committing",
        "PlanSummaryPrepared",
        "WorkItemRecordsPersisted",
        "FirstChildSessionEnsured",
        "FirstChildBindingEnsured",
        "FirstChildContextPrepared",
        "CompileReportPersisted",
        "committed",
    ];
    let guards: Vec<_> = checkpoints
        .iter()
        .map(|checkpoint| {
            d21b_register_delete_busy_probe(&app_paths, &plan_id, checkpoint, &violations, &fired)
        })
        .collect();
    let outcome = engine
        .run_work_item_plan_compile()
        .await
        .expect("live plan compile must succeed");
    drop(guards);
    assert!(
        violations.lock().unwrap().is_empty(),
        "{:?}",
        violations.lock().unwrap()
    );
    let fired_set = fired.lock().unwrap().clone();
    let expected: std::collections::BTreeSet<String> =
        checkpoints.iter().map(|cp| cp.to_string()).collect();
    assert_eq!(
        fired_set, expected,
        "每个 checkpoint 都必须在 run lease 在持时被真实编译触发"
    );

    // (c) 正常 published refs/hash/CAS 仍同代：committed tx + active lineage
    //     + plan_revision refs 一致；final 提交后 lease 随帧释放（删除不再
    //     Busy，不泄漏）。
    let plan_store = engine.work_item_plan_store().unwrap();
    let transactions = plan_store
        .list_compile_transactions("project_0001", "issue_0001", &plan_id)
        .unwrap();
    assert_eq!(transactions.len(), 1);
    let tx = &transactions[0];
    assert_eq!(tx.status, WorkItemPlanCompileStatus::Committed);
    assert_eq!(tx.plan_commit_state, WorkItemPlanCommitState::Committed);
    let lineage = engine
        .revision_store()
        .get_plan_lineage("project_0001", "issue_0001", &plan_id)
        .unwrap();
    assert_eq!(
        lineage.active_revision_id.as_deref(),
        Some(outcome.plan_revision.id.as_str())
    );
    let delete_probe = operations
        .try_acquire_delete(&scope)
        .expect("compile 完成后删除独占可行（lease 已随帧释放）");
    drop(delete_probe);
}

#[tokio::test]
async fn cascade_d2_1b_single_candidate_compile_lease_and_same_generation_publication() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    single_candidate_recovery_record(
        &lifecycle,
        &mut engine,
        SingleCandidatePhase::Approval,
        RunPolicy::Interactive,
    );
    let app_paths = lifecycle.app_paths();
    assert!(
        crate::product::coding_attempt_store::CodingAttemptStore::new(app_paths.clone())
            .list_attempts_for_issue("project_0001", "issue_0001")
            .unwrap()
            .is_empty(),
        "无 attempt 前提：运行中 SC compile 同样不可清"
    );

    let violations: D21bProbeViolations = Arc::new(Mutex::new(Vec::new()));
    let fired: D21bProbeFired = Arc::new(Mutex::new(Default::default()));
    let checkpoints = [
        "ApprovalPersisted",
        "ReservationPersisted",
        "ProvenancePersisted",
        "preparing",
        "validating",
        "committing",
        "PlanSummaryPrepared",
        "WorkItemRecordsPersisted",
        "FirstChildSessionEnsured",
        "FirstChildBindingEnsured",
        "FirstChildContextPrepared",
        "CompileReportPersisted",
        "committed",
    ];
    let guards: Vec<_> = checkpoints
        .iter()
        .map(|checkpoint| {
            d21b_register_delete_busy_probe(&app_paths, &plan_id, checkpoint, &violations, &fired)
        })
        .collect();
    let outcome = engine
        .run_work_item_plan_compile()
        .await
        .expect("SC compile must succeed");
    drop(guards);
    assert!(
        violations.lock().unwrap().is_empty(),
        "{:?}",
        violations.lock().unwrap()
    );
    let fired_set = fired.lock().unwrap().clone();
    let expected: std::collections::BTreeSet<String> =
        checkpoints.iter().map(|cp| cp.to_string()).collect();
    assert_eq!(fired_set, expected);

    // 正常 SC 发布同代：tx committed、provenance 内容 hash 与 CAS 身份一致、
    // lineage active revision 与 plan_revision 同 refs。
    let plan_store = engine.work_item_plan_store().unwrap();
    let transactions = plan_store
        .list_compile_transactions("project_0001", "issue_0001", &plan_id)
        .unwrap();
    assert_eq!(transactions.len(), 1);
    let tx = &transactions[0];
    assert_eq!(tx.status, WorkItemPlanCompileStatus::Committed);
    assert_eq!(tx.plan_commit_state, WorkItemPlanCommitState::Committed);
    let provenance_ref = tx
        .publication_provenance_ref
        .as_deref()
        .expect("SC tx durable provenance ref");
    let source_scope = SourceStoreScope {
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        plan_id: plan_id.clone(),
    };
    let provenance = WorkItemPlanSourceStore::new(app_paths.clone())
        .get_publication_provenance(&source_scope, provenance_ref)
        .unwrap();
    assert_eq!(provenance.plan_revision_id, outcome.plan_revision.id);
    assert_eq!(
        provenance.content_hash,
        provenance.content_hash().expect("provenance content hash")
    );
    let lineage = engine
        .revision_store()
        .get_plan_lineage("project_0001", "issue_0001", &plan_id)
        .unwrap();
    assert_eq!(
        lineage.active_revision_id.as_deref(),
        Some(outcome.plan_revision.id.as_str())
    );
}

/// 构造 SC RecoveryRequired 现场：阶段一 provenance 边界固定确定性
/// compile_id（tx 产生前中断），阶段二 finalizer Commit 段中断 →
/// mark_recovery + 恢复节点 Active。
async fn d21b_single_candidate_recovery_required_fixture() -> (
    tempfile::TempDir,
    LifecycleStore,
    String,
    WorkspaceEngine,
    String,
) {
    let (tmp, lifecycle, plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    single_candidate_recovery_record(
        &lifecycle,
        &mut engine,
        SingleCandidatePhase::Approval,
        RunPolicy::Interactive,
    );
    let failpoint = engine.register_single_candidate_compile_failpoint(
        super::SingleCandidateCompileCheckpoint::ProvenancePersisted,
    );
    let error = engine
        .run_work_item_plan_compile()
        .await
        .expect_err("provenance 边界必须先中断");
    drop(failpoint);
    assert!(error.contains("ProvenancePersisted"), "{error}");
    let record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .unwrap();
    let compile_id = record
        .compile_reservation
        .as_ref()
        .expect("provenance 后 durable reservation")
        .compile_id
        .clone();

    let failpoint = engine.register_work_item_plan_compile_finalizer_failpoint(
        &compile_id,
        WorkItemPlanCompileFinalizerCheckpoint::CompileReportPersisted,
    );
    let error = engine
        .run_work_item_plan_compile()
        .await
        .expect_err("finalizer checkpoint 必须中断");
    drop(failpoint);
    assert!(error.contains("CompileReportPersisted"), "{error}");
    assert!(engine.mark_latest_compile_transaction_recovery_required(&error));
    engine
        .enter_work_item_plan_compile_recovery(Some(error))
        .await;
    (tmp, lifecycle, plan_id, engine, compile_id)
}

#[tokio::test]
async fn cascade_d2_1b_recovery_resume_busy_while_running() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_tmp, lifecycle, plan_id, mut engine, _compile_id) =
        d21b_single_candidate_recovery_required_fixture().await;
    let app_paths = lifecycle.app_paths();

    // resume 还在跑：恢复 Continue 在真实 resume 重放位点删除必须 Busy。
    let violations: D21bProbeViolations = Arc::new(Mutex::new(Vec::new()));
    let fired: D21bProbeFired = Arc::new(Mutex::new(Default::default()));
    let _probe = d21b_register_delete_busy_probe(
        &app_paths,
        &plan_id,
        "single_candidate_publication_resumed",
        &violations,
        &fired,
    );
    let outcome = engine
        .handle_work_item_plan_compile_recovery_action(
            WorkItemPlanCompileRecoveryActionDto::Continue,
            None,
        )
        .await
        .expect("live plan recovery continue");
    assert_eq!(
        outcome,
        crate::product::workspace_engine::WorkItemPlanCompileRecoveryOutcome::HumanConfirm
    );
    assert!(
        violations.lock().unwrap().is_empty(),
        "{:?}",
        violations.lock().unwrap()
    );
    assert!(
        fired
            .lock()
            .unwrap()
            .contains("single_candidate_publication_resumed")
    );

    // resume 完成后 committed（正常恢复语义不回退）。
    let transactions = engine
        .work_item_plan_store()
        .unwrap()
        .list_compile_transactions("project_0001", "issue_0001", &plan_id)
        .unwrap();
    let tx = transactions
        .iter()
        .find(|tx| tx.status == WorkItemPlanCompileStatus::Committed)
        .expect("resumed transaction committed");
    assert_eq!(tx.plan_commit_state, WorkItemPlanCommitState::Committed);
}

#[tokio::test]
async fn cascade_d2_1b_delete_winner_recovery_writes_nothing() {
    let _serial = crate::product::workspace_engine::single_candidate_compile_test_lock().await;
    let (_tmp, lifecycle, plan_id, mut engine, _compile_id) =
        d21b_single_candidate_recovery_required_fixture().await;
    let app_paths = lifecycle.app_paths();
    let operations = EntityOperationStore::new(app_paths.clone());
    let deletions = EntityDeletionStore::new(app_paths.clone());
    let scope = EntityScope::new(
        "project_0001",
        "issue_0001",
        DeletionEntityKind::Plan,
        &plan_id,
    );

    // 存面快照（删除赢之前）。
    let plan_store = engine.work_item_plan_store().unwrap();
    let tx_before: Vec<(String, WorkItemPlanCompileStatus, String, Option<String>)> = plan_store
        .list_compile_transactions("project_0001", "issue_0001", &plan_id)
        .unwrap()
        .into_iter()
        .map(|tx| (tx.compile_id, tx.status, tx.step_cursor, tx.failure_reason))
        .collect();
    let work_items_before = lifecycle
        .count_work_items("project_0001", "issue_0001")
        .unwrap();
    let source_root = app_paths
        .issue_root("project_0001", "issue_0001")
        .join("work-item-plan-sources")
        .join(&plan_id);
    let source_before = d21b_snapshot_tree(&source_root);

    // 删除赢：purging 墓碑在位（删除独占 lease 已释放，仅墓碑事实）。
    let delete_lease = operations
        .try_acquire_delete(&scope)
        .expect("恢复待决的 live plan 删除独占可行");
    deletions.begin(&delete_lease).expect("purging tombstone");
    drop(delete_lease);
    assert_eq!(
        deletions.get(&scope).unwrap().map(|record| record.phase),
        Some(DeletionPhase::Purging)
    );

    // 恢复动作三类全部先于 durable 写稳定拒绝，零 transaction 新写。
    let journal = crate::product::work_item_plan_store::observe_compile_transaction_writes();
    for action in [
        WorkItemPlanCompileRecoveryActionDto::Continue,
        WorkItemPlanCompileRecoveryActionDto::HumanTriage,
        WorkItemPlanCompileRecoveryActionDto::AbortAndRollback,
    ] {
        let rejected = engine
            .handle_work_item_plan_compile_recovery_action(action, None)
            .await
            .expect_err("删除赢后恢复动作必须稳定拒绝");
        assert!(rejected.contains("entity_deleting"), "{rejected}");
    }
    assert!(
        journal.snapshots().is_empty(),
        "拒绝路径不得写 compile transaction"
    );
    drop(journal);

    // 存面零变化：transaction/WorkItems/source/provenance 均未被触碰。
    let tx_after: Vec<(String, WorkItemPlanCompileStatus, String, Option<String>)> = plan_store
        .list_compile_transactions("project_0001", "issue_0001", &plan_id)
        .unwrap()
        .into_iter()
        .map(|tx| (tx.compile_id, tx.status, tx.step_cursor, tx.failure_reason))
        .collect();
    assert_eq!(tx_after, tx_before, "拒绝路径不得改写 compile transaction");
    assert_eq!(
        lifecycle
            .count_work_items("project_0001", "issue_0001")
            .unwrap(),
        work_items_before,
        "拒绝路径不得增删 WorkItems"
    );
    assert_eq!(
        d21b_snapshot_tree(&source_root),
        source_before,
        "拒绝路径不得写 source/provenance"
    );
    assert!(
        crate::product::coding_attempt_store::CodingAttemptStore::new(app_paths.clone())
            .list_attempts_for_issue("project_0001", "issue_0001")
            .unwrap()
            .is_empty()
    );
}
