use super::*;
use crate::product::models::PlanRepairRequest;
use crate::product::plan_repair::{PlanDefectFinding, PlanDefectSeverity, plan_defect_fingerprint};
use std::fs;

#[tokio::test]
async fn coding_plan_repair_orphan_child_without_link_reuses_canonical_parent() {
    let fixture = plan_repair_fixture();
    let finding = plan_defect_finding("orphan_evidence");
    let request = seed_open_request(&fixture, &finding, None, None, true);
    let amendment_id = request.amendment_id.clone().unwrap();
    fixture
        .revision_store
        .acquire_active_amendment(&fixture.plan, &amendment_id)
        .unwrap();
    let lifecycle = LifecycleStore::new(fixture.store.paths());
    let orphan = lifecycle
        .create_workspace_session_with_id(
            CreateWorkspaceSessionInput {
                project_id: fixture.attempt.project_id.clone(),
                issue_id: fixture.attempt.issue_id.clone(),
                entity_id: fixture.plan.id.clone(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Codex,
                reviewer_provider: ProviderName::ClaudeCode,
                review_rounds: 1,
                superpowers_enabled: true,
                openspec_enabled: true,
                work_item_plan_options: None,
            },
            format!("workspace_session_{amendment_id}"),
        )
        .unwrap();

    let updated = fixture
        .engine
        .start_plan_repair_from_review(
            &fixture.attempt,
            "code_review_report_retry",
            "code_review_report_retry_finding_0001",
            &finding,
            &fixture.projection,
        )
        .await
        .expect("orphan child must not be mistaken for a second parent");

    assert_eq!(updated.status, CodingAttemptStatus::AwaitingPlanAmendment);
    assert_eq!(
        lifecycle
            .list_workspace_sessions(&updated.project_id, &updated.issue_id)
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        lifecycle
            .get_session_link(&orphan.id)
            .unwrap()
            .child_session_id,
        orphan.id
    );
}

#[tokio::test]
async fn coding_plan_repair_existing_request_attempt_mismatch_is_zero_write() {
    assert_existing_request_identity_mismatch_is_zero_write(Some("coding_attempt_foreign"), None)
        .await;
}

#[tokio::test]
async fn coding_plan_repair_existing_request_unit_run_mismatch_is_zero_write() {
    assert_existing_request_identity_mismatch_is_zero_write(None, Some("coding_unit_run_foreign"))
        .await;
}

#[tokio::test]
async fn coding_plan_repair_reconnect_rejects_noncanonical_link_and_child_identity() {
    for corruption in [
        ReconnectCorruption::LinkId,
        ReconnectCorruption::ChildId,
        ReconnectCorruption::ParentId,
        ReconnectCorruption::ReturnRoute,
        ReconnectCorruption::ChildProject,
        ReconnectCorruption::ChildIssue,
        ReconnectCorruption::ChildEntity,
        ReconnectCorruption::ChildType,
        ReconnectCorruption::ChildStatus,
    ] {
        let fixture = plan_repair_fixture();
        let finding = plan_defect_finding("reconnect_identity");
        let started = fixture
            .engine
            .start_plan_repair_from_review(
                &fixture.attempt,
                "code_review_report_0001",
                "code_review_report_0001_finding_0001",
                &finding,
                &fixture.projection,
            )
            .await
            .unwrap();
        corrupt_reconnect_identity(&fixture, corruption);

        let error = build_coding_session_state(&fixture.store, started)
            .expect_err("reconnect must reuse the P4 canonical link and child validator");

        assert!(
            error.to_string().contains("identity_mismatch"),
            "unexpected reconnect error for {corruption:?}: {error}"
        );
    }
}

#[test]
fn coding_plan_repair_active_amendment_anchor_blocks_recovery_before_attempt_pause() {
    let fixture = plan_repair_fixture();
    fixture
        .revision_store
        .acquire_active_amendment(&fixture.plan, "plan_amendment_race_anchor")
        .unwrap();

    let error = fixture
        .store
        .prepare_failed_code_review_recovery_journal(
            &fixture.attempt,
            "coding_blocked_gate_0001",
            "coding_node_0009",
            "coding_role_run_0008",
        )
        .expect_err("durable P4 anchor must block recovery before Attempt pause");

    assert!(
        error
            .to_string()
            .contains("plan_amendment_blocks_provider_run")
    );
    assert!(
        fixture
            .store
            .get_failed_code_review_recovery_journal(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .unwrap()
            .is_none()
    );
}

#[derive(Debug, Clone, Copy)]
enum ReconnectCorruption {
    LinkId,
    ChildId,
    ParentId,
    ReturnRoute,
    ChildProject,
    ChildIssue,
    ChildEntity,
    ChildType,
    ChildStatus,
}

fn corrupt_reconnect_identity(fixture: &PlanRepairFixture, corruption: ReconnectCorruption) {
    let lifecycle = LifecycleStore::new(fixture.store.paths());
    let mut link = lifecycle
        .list_session_links(&fixture.attempt.project_id, &fixture.attempt.issue_id)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let original_link_id = link.id.clone();
    let original_child_id = link.child_session_id.clone();
    let mut snapshot = lifecycle
        .load_plan_repair_session_state(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &original_child_id,
        )
        .unwrap()
        .unwrap();
    let mut child = lifecycle.get_workspace_session(&original_child_id).unwrap();

    match corruption {
        ReconnectCorruption::LinkId => link.id = "workspace_session_link_forged".to_string(),
        ReconnectCorruption::ChildId => {
            let forged = lifecycle
                .create_workspace_session_with_id(
                    CreateWorkspaceSessionInput {
                        project_id: fixture.attempt.project_id.clone(),
                        issue_id: fixture.attempt.issue_id.clone(),
                        entity_id: fixture.plan.id.clone(),
                        workspace_type: WorkspaceType::WorkItemPlan,
                        author_provider: ProviderName::Codex,
                        reviewer_provider: ProviderName::ClaudeCode,
                        review_rounds: 1,
                        superpowers_enabled: true,
                        openspec_enabled: true,
                        work_item_plan_options: None,
                    },
                    "workspace_session_forged".to_string(),
                )
                .unwrap();
            link.child_session_id = forged.id;
        }
        ReconnectCorruption::ParentId => {
            link.parent_session_id = "workspace_session_parent_forged".to_string();
        }
        ReconnectCorruption::ReturnRoute => {
            link.return_context.original_route = "/forged/return/route".to_string();
        }
        ReconnectCorruption::ChildProject => child.project_id = "project_forged".to_string(),
        ReconnectCorruption::ChildIssue => child.issue_id = "issue_forged".to_string(),
        ReconnectCorruption::ChildEntity => child.entity_id = "work_item_plan_forged".to_string(),
        ReconnectCorruption::ChildType => child.workspace_type = WorkspaceType::WorkItem,
        ReconnectCorruption::ChildStatus => child.status = WorkspaceSessionStatus::Confirmed,
    }

    let issue_root = fixture
        .store
        .paths()
        .issue_lifecycle_root(&fixture.attempt.project_id, &fixture.attempt.issue_id);
    if matches!(
        corruption,
        ReconnectCorruption::ChildProject
            | ReconnectCorruption::ChildIssue
            | ReconnectCorruption::ChildEntity
            | ReconnectCorruption::ChildType
            | ReconnectCorruption::ChildStatus
    ) {
        fs::write(
            issue_root
                .join("workspace-sessions")
                .join(format!("{original_child_id}.json")),
            serde_json::to_vec_pretty(&child).unwrap(),
        )
        .unwrap();
        return;
    }

    snapshot.link = link.clone();
    lifecycle
        .save_plan_repair_session_state(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &link.child_session_id,
            &snapshot,
        )
        .unwrap();
    let original_link_path = issue_root
        .join("workspace-session-links")
        .join(format!("{original_link_id}.json"));
    if link.id != original_link_id {
        fs::remove_file(&original_link_path).unwrap();
    }
    fs::write(
        issue_root
            .join("workspace-session-links")
            .join(format!("{}.json", link.id)),
        serde_json::to_vec_pretty(&link).unwrap(),
    )
    .unwrap();
}

async fn assert_existing_request_identity_mismatch_is_zero_write(
    trigger_attempt_id: Option<&str>,
    trigger_unit_run_id: Option<&str>,
) {
    let fixture = plan_repair_fixture();
    let finding = plan_defect_finding("existing_evidence");
    let request = seed_open_request(
        &fixture,
        &finding,
        trigger_attempt_id,
        trigger_unit_run_id,
        false,
    );
    let lifecycle = LifecycleStore::new(fixture.store.paths());
    let attempt_before = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    let unit_run_before = fixture.store.get_active_unit_run(&fixture.attempt).unwrap();
    let timeline_before = fixture
        .store
        .get_timeline_nodes(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    let sessions_before = lifecycle
        .list_workspace_sessions(&fixture.attempt.project_id, &fixture.attempt.issue_id)
        .unwrap();
    let links_before = lifecycle
        .list_session_links(&fixture.attempt.project_id, &fixture.attempt.issue_id)
        .unwrap();
    let plan_before = fixture
        .revision_store
        .get_plan_lineage(
            &fixture.plan.project_id,
            &fixture.plan.issue_id,
            &fixture.plan.id,
        )
        .unwrap();
    let mut retry_finding = finding.clone();
    retry_finding.plan_defect_evidence.push(PlanDefectEvidence {
        kind: "review".to_string(),
        source_ref: "retry_evidence".to_string(),
        message: "must not be merged".to_string(),
    });

    let error = fixture
        .engine
        .start_plan_repair_from_review(
            &fixture.attempt,
            "code_review_report_retry",
            "code_review_report_retry_finding_0001",
            &retry_finding,
            &fixture.projection,
        )
        .await
        .expect_err("persisted trigger identity mismatch must fail before P4 writes");

    assert!(
        error
            .to_string()
            .contains("linked snapshot identity mismatch")
    );
    assert_eq!(
        fixture
            .revision_store
            .get_repair_request(&fixture.plan, &request.id)
            .unwrap(),
        request
    );
    assert_eq!(
        fixture
            .revision_store
            .get_plan_lineage(
                &fixture.plan.project_id,
                &fixture.plan.issue_id,
                &fixture.plan.id,
            )
            .unwrap(),
        plan_before
    );
    assert_eq!(
        lifecycle
            .list_workspace_sessions(&fixture.attempt.project_id, &fixture.attempt.issue_id)
            .unwrap(),
        sessions_before
    );
    assert_eq!(
        lifecycle
            .list_session_links(&fixture.attempt.project_id, &fixture.attempt.issue_id)
            .unwrap(),
        links_before
    );
    assert_eq!(
        fixture
            .store
            .get_attempt(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .unwrap(),
        attempt_before
    );
    assert_eq!(
        fixture.store.get_active_unit_run(&fixture.attempt).unwrap(),
        unit_run_before
    );
    assert_eq!(
        fixture
            .store
            .get_timeline_nodes(
                &fixture.attempt.project_id,
                &fixture.attempt.issue_id,
                &fixture.attempt.id,
            )
            .unwrap(),
        timeline_before
    );
}

fn seed_open_request(
    fixture: &PlanRepairFixture,
    finding: &ReviewFinding,
    trigger_attempt_id: Option<&str>,
    trigger_unit_run_id: Option<&str>,
    with_amendment: bool,
) -> PlanRepairRequest {
    let canonical = PlanDefectFinding {
        finding_id: "persisted_finding".to_string(),
        severity: PlanDefectSeverity::Error,
        defect_class: finding.defect_class.clone(),
        reason_code: finding.reason_code.clone().unwrap(),
        message: finding.message.clone(),
        evidence: finding.plan_defect_evidence.clone(),
        contract_refs: finding.contract_refs.clone(),
        capability_refs: finding.capability_refs.clone(),
        repair_target: finding.repair_target.clone(),
        recommended_route: finding.recommended_route.clone(),
        confidence: finding.confidence.clone().unwrap(),
    };
    let base_plan_revision_id = fixture.plan.active_revision_id.clone().unwrap();
    let fingerprint = plan_defect_fingerprint(&base_plan_revision_id, &canonical);
    let request = PlanRepairRequest {
        id: "plan_repair_request_existing".to_string(),
        plan_id: fixture.plan.id.clone(),
        base_plan_revision_id,
        trigger_attempt_id: trigger_attempt_id
            .unwrap_or(&fixture.attempt.id)
            .to_string(),
        trigger_unit_run_id: trigger_unit_run_id
            .unwrap_or("coding_unit_run_0001")
            .to_string(),
        trigger_review_id: Some("code_review_report_existing".to_string()),
        trigger_finding_id: "persisted_finding".to_string(),
        amendment_id: with_amendment.then(|| format!("plan_amendment_{fingerprint}")),
        defect_class: canonical.defect_class,
        reason_code: canonical.reason_code,
        repair_target: canonical.repair_target.unwrap(),
        contract_refs: canonical.contract_refs,
        capability_refs: canonical.capability_refs,
        evidence: canonical.evidence,
        fingerprint,
        status: if with_amendment {
            PlanRepairRequestStatus::InProgress
        } else {
            PlanRepairRequestStatus::Open
        },
        created_at: "2026-07-19T00:00:00Z".to_string(),
        updated_at: "2026-07-19T00:00:00Z".to_string(),
    };
    fixture
        .revision_store
        .put_repair_request(&fixture.plan, &request)
        .unwrap();
    request
}
