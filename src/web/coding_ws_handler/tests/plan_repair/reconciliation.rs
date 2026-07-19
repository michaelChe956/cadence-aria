use super::*;
use crate::product::coding_models::CodingTimelineNode;
use crate::product::json_store::write_json;

#[derive(Debug, Clone, Copy)]
enum PlanRepairPrefix {
    AnchorOnly,
    AttemptPaused,
    UnitRunBlocked,
    FullyReconciled,
}

#[tokio::test]
async fn coding_plan_repair_distinct_finding_does_not_reuse_active_request() {
    let fixture = plan_repair_fixture();
    let first = plan_defect_report(plan_defect_finding("evidence_a"));
    let after = fixture
        .engine
        .start_plan_repair_from_review(
            &fixture.attempt,
            &first.id,
            "code_review_report_0001_finding_0001",
            &first.findings[0],
            &fixture.projection,
        )
        .await
        .unwrap();
    let active_request = fixture
        .revision_store
        .list_open_repair_requests(&fixture.plan)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let run_before = fixture.store.get_active_unit_run(&after).unwrap();
    let lifecycle = LifecycleStore::new(fixture.store.paths());
    let links_before = lifecycle
        .list_session_links(&after.project_id, &after.issue_id)
        .unwrap();
    let mut distinct = plan_defect_report(plan_defect_finding("evidence_b"));
    distinct.id = "code_review_report_0002".to_string();
    let target = distinct.findings[0]
        .repair_target
        .as_mut()
        .expect("repair target");
    target.logical_work_item_ids.push("wi_current".to_string());
    target
        .work_item_revision_ids
        .push("work_item_revision_current".to_string());

    let error = fixture
        .engine
        .start_plan_repair_from_review(
            &after,
            &distinct.id,
            "code_review_report_0002_finding_0001",
            &distinct.findings[0],
            &fixture.projection,
        )
        .await
        .expect_err("a distinct finding must not reuse the active Plan Repair request");

    assert!(
        error
            .to_string()
            .contains("Plan Repair linked snapshot identity mismatch"),
        "unexpected error: {error}"
    );
    let requests = fixture
        .revision_store
        .list_open_repair_requests(&fixture.plan)
        .unwrap();
    assert_eq!(requests, vec![active_request]);
    assert_eq!(
        lifecycle
            .list_session_links(&after.project_id, &after.issue_id)
            .unwrap(),
        links_before
    );
    assert_eq!(
        fixture.store.get_active_unit_run(&after).unwrap(),
        run_before
    );
}

#[tokio::test]
async fn coding_plan_repair_reconnect_ignores_historical_completed_link() {
    let fixture = plan_repair_fixture();
    let report = plan_defect_report(plan_defect_finding("current_reconnect"));
    let started = fixture
        .engine
        .start_plan_repair_from_review(
            &fixture.attempt,
            &report.id,
            "code_review_report_0001_finding_0001",
            &report.findings[0],
            &fixture.projection,
        )
        .await
        .unwrap();
    let lifecycle = LifecycleStore::new(fixture.store.paths());
    let current_link = lifecycle
        .list_session_links(&started.project_id, &started.issue_id)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let mut historical_link = current_link.clone();
    historical_link.id = "plan_repair_link_history".to_string();
    historical_link.child_session_id = "plan_repair_session_history".to_string();
    historical_link.trigger.repair_request_id = "plan_repair_request_history".to_string();
    historical_link.trigger.amendment_id = "plan_amendment_history".to_string();
    historical_link.trigger.fingerprint = "historical_fingerprint".to_string();
    historical_link.created_at = "2026-07-18T00:00:00Z".to_string();
    lifecycle
        .put_session_link(&started.project_id, &started.issue_id, &historical_link)
        .unwrap();

    let state = build_coding_session_state(&fixture.store, started).unwrap();
    let encoded = serde_json::to_value(state).unwrap();

    assert_eq!(encoded["linked_plan_repair"]["link"]["id"], current_link.id);
    assert_eq!(
        encoded["linked_plan_repair"]["request"]["id"],
        current_link.trigger.repair_request_id
    );
}

#[tokio::test]
async fn coding_plan_repair_provider_entry_recovers_anchor_only_before_starting_provider() {
    let fixture = plan_repair_fixture();
    let report = plan_defect_report(plan_defect_finding("anchor_only"));
    let started = fixture
        .engine
        .start_plan_repair_from_review(
            &fixture.attempt,
            &report.id,
            "code_review_report_0001_finding_0001",
            &report.findings[0],
            &fixture.projection,
        )
        .await
        .unwrap();
    let request_id = reset_plan_repair_prefix(&fixture, &started, PlanRepairPrefix::AnchorOnly);
    let current = fixture
        .store
        .get_attempt(&started.project_id, &started.issue_id, &started.id)
        .unwrap();
    let provider = CountingProvider::default();

    let error = fixture
        .engine
        .execute_coding(
            &current,
            &provider,
            &crate::product::coding_workspace_engine::CodingExecutionContext::default(),
        )
        .await
        .expect_err("the durable repair anchor must pause Coding before provider start");

    assert!(
        error
            .to_string()
            .contains("plan_amendment_blocks_provider_run")
    );
    assert_eq!(provider.starts(), 0);
    assert_reconciled_plan_repair_prefix(&fixture, &started, &request_id);
}

#[tokio::test]
async fn coding_plan_repair_session_state_recovers_every_pause_prefix_idempotently() {
    for prefix in [
        PlanRepairPrefix::AttemptPaused,
        PlanRepairPrefix::UnitRunBlocked,
        PlanRepairPrefix::FullyReconciled,
    ] {
        let fixture = plan_repair_fixture();
        let report = plan_defect_report(plan_defect_finding("session_reconcile"));
        let started = fixture
            .engine
            .start_plan_repair_from_review(
                &fixture.attempt,
                &report.id,
                "code_review_report_0001_finding_0001",
                &report.findings[0],
                &fixture.projection,
            )
            .await
            .unwrap();
        let request_id = reset_plan_repair_prefix(&fixture, &started, prefix);
        let current = fixture
            .store
            .get_attempt(&started.project_id, &started.issue_id, &started.id)
            .unwrap();

        for _ in 0..2 {
            let state = build_coding_session_state(&fixture.store, current.clone()).unwrap();
            let encoded = serde_json::to_value(state).unwrap();
            assert_eq!(encoded["status"], "awaiting_plan_amendment", "{prefix:?}");
            assert_eq!(
                encoded["linked_plan_repair"]["request"]["id"], request_id,
                "{prefix:?}"
            );
        }
        assert_reconciled_plan_repair_prefix(&fixture, &started, &request_id);
    }
}

fn reset_plan_repair_prefix(
    fixture: &PlanRepairFixture,
    attempt: &CodingExecutionAttempt,
    prefix: PlanRepairPrefix,
) -> String {
    let request_id = fixture
        .revision_store
        .list_open_repair_requests(&fixture.plan)
        .unwrap()
        .into_iter()
        .next()
        .unwrap()
        .id;
    let mut current = fixture
        .store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    let mut run = fixture.store.get_active_unit_run(&current).unwrap();
    match prefix {
        PlanRepairPrefix::AnchorOnly => {
            current.status = CodingAttemptStatus::Running;
            run.status = CodingUnitRunStatus::Running;
            run.plan_repair_count = 0;
        }
        PlanRepairPrefix::AttemptPaused => {
            run.status = CodingUnitRunStatus::Running;
            run.plan_repair_count = 0;
        }
        PlanRepairPrefix::UnitRunBlocked | PlanRepairPrefix::FullyReconciled => {}
    }
    fixture.store.save_coding_attempt(&current).unwrap();
    write_json(
        &fixture.store.coding_unit_run_path(
            &current.project_id,
            &current.issue_id,
            &current.id,
            &run.unit_id,
            &run.id,
        ),
        &run,
    )
    .unwrap();
    if !matches!(prefix, PlanRepairPrefix::FullyReconciled) {
        write_json::<Vec<CodingTimelineNode>>(
            &fixture
                .store
                .attempt_dir(&current.project_id, &current.issue_id, &current.id)
                .join("timeline-nodes.json"),
            &Vec::new(),
        )
        .unwrap();
    }
    request_id
}

fn assert_reconciled_plan_repair_prefix(
    fixture: &PlanRepairFixture,
    attempt: &CodingExecutionAttempt,
    request_id: &str,
) {
    let current = fixture
        .store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    assert_eq!(current.status, CodingAttemptStatus::AwaitingPlanAmendment);
    let run = fixture.store.get_active_unit_run(&current).unwrap();
    assert_eq!(run.status, CodingUnitRunStatus::BlockedByPlanDefect);
    assert_eq!(run.plan_repair_count, 1);
    let matching_timeline = fixture
        .store
        .get_timeline_nodes(&current.project_id, &current.issue_id, &current.id)
        .unwrap()
        .into_iter()
        .filter(|node| {
            node.title == "Plan Repair"
                && node
                    .artifact_refs
                    .iter()
                    .any(|artifact| artifact == request_id)
        })
        .count();
    assert_eq!(matching_timeline, 1);
    assert_eq!(
        fixture
            .revision_store
            .list_open_repair_requests(&fixture.plan)
            .unwrap()
            .len(),
        1
    );
    let lifecycle = LifecycleStore::new(fixture.store.paths());
    assert_eq!(
        lifecycle
            .list_session_links(&current.project_id, &current.issue_id)
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        lifecycle
            .list_workspace_sessions(&current.project_id, &current.issue_id)
            .unwrap()
            .len(),
        2
    );
}
