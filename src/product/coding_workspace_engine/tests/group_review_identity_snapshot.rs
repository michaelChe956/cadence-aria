use super::*;
use crate::product::coding_models::{
    CompactFindingDigest, ReviewVerdict, UnitReviewConclusionSnapshot,
};
use crate::product::coding_workspace_engine::tests::provider_execution_context::CapturingProjectionProvider;

#[test]
fn unit_review_conclusion_snapshot_write_is_idempotent() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let snapshot = UnitReviewConclusionSnapshot {
        attempt_id: attempt.id.clone(),
        unit_id: "unit_0001".to_string(),
        unit_run_id: "unit_run_0001".to_string(),
        logical_work_item_id: "work_item_0001".to_string(),
        work_item_revision_id: "work_item_revision_0001".to_string(),
        code_review_report_id: "code_review_0001".to_string(),
        verdict: ReviewVerdict::Approve,
        finding_digest: vec![CompactFindingDigest {
            defect_class: None,
            reason_code: None,
            severity: "info".to_string(),
            message_digest: "message-hash".to_string(),
        }],
        evidence_refs: vec!["test-output/verification.txt".to_string()],
        diff_refs: vec!["HEAD..worktree".to_string()],
        raw_report_hash: "raw-report-hash".to_string(),
    };

    store
        .write_unit_review_conclusion_snapshot(&snapshot)
        .expect("first write");
    store
        .write_unit_review_conclusion_snapshot(&snapshot)
        .expect("idempotent write");

    assert_eq!(
        store
            .get_unit_review_conclusion_snapshot(&attempt.id, &snapshot.unit_run_id)
            .expect("get snapshot"),
        Some(snapshot)
    );
}

#[test]
fn rebuilding_legacy_report_without_unit_run_id_fails_closed() {
    let (_root, store, attempt, unit, unit_run) = group_attempt_with_completed_unit_run();
    let report = code_review_report(&attempt.id, "code_review_0001", None, None);
    store
        .save_code_review_report(&attempt, &report)
        .expect("persist legacy report");

    let error = store
        .rebuild_unit_review_conclusion_snapshot(&attempt.id, &unit_run.id)
        .expect_err("legacy report must fail closed");

    assert!(matches!(
        error,
        crate::product::coding_models::SnapshotRebuildError::MissingUnitRunId(id)
            if id == report.id
    ));
    assert!(
        store
            .get_unit_review_conclusion_snapshot(&attempt.id, &unit_run.id)
            .expect("get snapshot")
            .is_none()
    );
    assert_eq!(unit.id, unit_run.unit_id);
}

#[test]
fn rebuilding_snapshot_from_report_and_authoritative_binding_is_deterministic() {
    let (root, store, attempt, unit, unit_run) = group_attempt_with_completed_unit_run();
    let raw_output = "review raw output\n";
    let raw_path = root
        .path()
        .join(".aria")
        .join("projects")
        .join(&attempt.project_id)
        .join("issues")
        .join(&attempt.issue_id)
        .join("coding-attempts")
        .join(&attempt.id)
        .join("provider-raw/code-review/code_review_0001.txt");
    std::fs::create_dir_all(raw_path.parent().expect("raw parent")).expect("raw parent");
    std::fs::write(&raw_path, raw_output).expect("raw output");
    let raw_ref = "provider-raw/code-review/code_review_0001.txt".to_string();
    let report = code_review_report(
        &attempt.id,
        "code_review_0001",
        Some(unit_run.id.clone()),
        Some(raw_ref),
    );
    store
        .save_code_review_report(&attempt, &report)
        .expect("persist report");
    let direct = UnitReviewConclusionSnapshot {
        attempt_id: attempt.id.clone(),
        unit_id: unit.id.clone(),
        unit_run_id: unit_run.id.clone(),
        logical_work_item_id: unit.logical_work_item_id.clone(),
        work_item_revision_id: unit.work_item_revision_id.clone(),
        code_review_report_id: report.id.clone(),
        verdict: report.verdict.clone(),
        finding_digest: Vec::new(),
        evidence_refs: report.tested_evidence_refs.clone(),
        diff_refs: report.diff_refs.clone(),
        raw_report_hash: sha256_hex(raw_output),
    };

    let rebuilt = store
        .rebuild_unit_review_conclusion_snapshot(&attempt.id, &unit_run.id)
        .expect("rebuild snapshot");

    assert_eq!(rebuilt, direct);
    assert_eq!(
        store
            .get_unit_review_conclusion_snapshot(&attempt.id, &unit_run.id)
            .expect("get rebuilt snapshot"),
        Some(direct)
    );
}

#[tokio::test]
async fn code_review_snapshot_write_failure_rolls_back_report() {
    let (root, store, attempt, _unit, unit_run) = group_attempt_with_completed_unit_run();
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running attempt");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    let snapshot_root = root
        .path()
        .join(".aria")
        .join("projects")
        .join(&attempt.project_id)
        .join("issues")
        .join(&attempt.issue_id)
        .join("coding-attempts")
        .join(&attempt.id)
        .join("unit-review-conclusion-snapshots");
    std::fs::create_dir_all(&snapshot_root).expect("snapshot root");
    std::fs::write(
        snapshot_root.join(&unit_run.id).with_extension("json"),
        "not json",
    )
    .expect("poison snapshot path");
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = CapturingProjectionProvider::new(
        serde_json::json!({
            "verdict": "approve",
            "summary": "review approved",
            "findings": []
        })
        .to_string(),
    );
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = engine
        .execute_code_review_with_commands(&attempt, &provider, &mut command_rx)
        .await
        .expect_err("snapshot failure must fail review persistence");

    assert!(error.to_string().contains("product_store_json"));
    assert!(
        store
            .list_code_review_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("list reports")
            .is_empty()
    );
}

#[tokio::test]
async fn code_review_persists_report_and_snapshot_on_normal_path() {
    let (_root, store, attempt, _unit, unit_run) = group_attempt_with_completed_unit_run();
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running attempt");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = CapturingProjectionProvider::new(
        serde_json::json!({
            "verdict": "approve",
            "summary": "review approved",
            "findings": []
        })
        .to_string(),
    );
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let report = engine
        .execute_code_review_with_commands(&attempt, &provider, &mut command_rx)
        .await
        .expect("review succeeds");

    assert_eq!(report.unit_run_id.as_deref(), Some(unit_run.id.as_str()));
    assert_eq!(
        store
            .list_code_review_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("list reports"),
        vec![report.clone()]
    );
    let snapshot = store
        .get_unit_review_conclusion_snapshot(&attempt.id, &unit_run.id)
        .expect("get snapshot")
        .expect("snapshot persisted");
    assert_eq!(snapshot.code_review_report_id, report.id);
}

fn sha256_hex(value: &str) -> String {
    use sha2::{Digest, Sha256};

    hex::encode(Sha256::digest(value.as_bytes()))
}

fn group_attempt_with_completed_unit_run() -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingExecutionAttempt,
    crate::product::coding_models::CodingExecutionUnit,
    crate::product::coding_models::CodingUnitRun,
) {
    let root = tempdir().expect("tempdir");
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("worktree");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("group attempt");
    seed_group_attempt_fixture(&store, &attempt, true, false);
    let unit = store
        .get_active_coding_unit(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("active unit")
        .expect("unit");
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &attempt.project_id,
            &attempt.issue_id,
            "work_item_plan_0001",
        )
        .expect("plan lineage");
    let revision = revision_store
        .get_work_item_revision(
            &lineage,
            &unit.logical_work_item_id,
            &unit.work_item_revision_id,
        )
        .expect("work item revision");
    let bundle = revision_store
        .get_work_item_projection_bundle(&lineage, &revision.work_item_projection_bundle_id)
        .expect("projection bundle");
    let unit_run = crate::product::coding_models::CodingUnitRun {
        id: "coding_unit_run_0001".to_string(),
        unit_id: unit.id.clone(),
        execution_no: 1,
        work_item_revision_id: unit.work_item_revision_id.clone(),
        resolved_handoff_revision_ids: Vec::new(),
        canonical_contract_hash: bundle.canonical_contract_hash,
        projection_bundle_id: bundle.id,
        projection_compiler_version: bundle.compiler_version,
        coder_provider_renderer_version: renderer_for(&ProviderName::Fake)
            .renderer_version()
            .to_string(),
        reviewer_provider_renderer_version: renderer_for(&ProviderName::Fake)
            .renderer_version()
            .to_string(),
        internal_reviewer_provider_renderer_version: None,
        coder_projection_hash: bundle.coder_projection_hash,
        reviewer_projection_hash: bundle.reviewer_projection_hash,
        coder_execution_context_hash: None,
        reviewer_execution_context_hash: None,
        internal_reviewer_execution_context_hash: None,
        status: crate::product::coding_models::CodingUnitRunStatus::Running,
        unit_rework_count: 0,
        verification_retry_count: 0,
        operational_retry_count: 0,
        plan_repair_count: 0,
        start_commit: Some("start-commit".to_string()),
        completion_commit: Some("completion-commit".to_string()),
        created_at: "2026-08-04T00:00:00Z".to_string(),
        updated_at: "2026-08-04T00:00:00Z".to_string(),
    };
    store
        .create_coding_unit_run(&attempt, &unit_run)
        .expect("completed unit run");
    (root, store, attempt, unit, unit_run)
}

fn code_review_report(
    attempt_id: &str,
    id: &str,
    unit_run_id: Option<String>,
    raw_provider_output_ref: Option<String>,
) -> crate::product::coding_models::CodeReviewReport {
    crate::product::coding_models::CodeReviewReport {
        id: id.to_string(),
        attempt_id: attempt_id.to_string(),
        round: 1,
        verdict: ReviewVerdict::Approve,
        findings: Vec::new(),
        tested_evidence_refs: vec!["test-output/verification.txt".to_string()],
        diff_refs: vec!["HEAD..worktree".to_string()],
        summary: "review approved".to_string(),
        created_at: "2026-08-04T00:00:00Z".to_string(),
        raw_provider_output_ref,
        role_run_id: None,
        run_no: None,
        unit_run_id,
    }
}
