use super::*;
use crate::product::coding_models::{
    CodingAgentRole, CodingTimelineNode, CodingTimelineNodeStatus,
};

#[tokio::test]
async fn global_failed_review_recovery_ignores_unrelated_amendment_gate_id_collision() {
    let fixture = provider_interrupted_review_fixture(CodingAttemptScope::WorkItem).await;
    let gate_id = fixture
        .dirty_gate
        .as_ref()
        .expect("recoverable gate")
        .gate_id
        .clone();
    let mut unrelated = fixture.attempt.clone();
    unrelated.project_id = "project_0002".to_string();
    unrelated.issue_id = "issue_0002".to_string();
    unrelated.work_item_id = "work_item_0002".to_string();
    unrelated.status = CodingAttemptStatus::Running;
    unrelated.active_unit_id = None;
    unrelated.work_item_group_id = None;
    unrelated.completed_at = None;
    unrelated.worktree_path = Some(fixture._tmp.path().join("unrelated-worktree"));
    std::fs::create_dir_all(unrelated.worktree_path.as_ref().unwrap()).unwrap();
    fixture.store.save_coding_attempt(&unrelated).unwrap();
    fixture
        .store
        .save_timeline_node(
            &unrelated,
            CodingTimelineNode {
                id: support::FAILED_NODE_ID.to_string(),
                attempt_id: unrelated.id.clone(),
                stage: CodingExecutionStage::CodeReview,
                title: "代码审查".to_string(),
                status: CodingTimelineNodeStatus::Running,
                agent_role: Some(CodingAgentRole::Reviewer),
                summary: None,
                started_at: "2026-07-19T00:00:00Z".to_string(),
                completed_at: None,
                artifact_refs: Vec::new(),
            },
        )
        .unwrap();
    fixture
        .store
        .create_role_run(
            &unrelated,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::Initial,
            Some(support::FAILED_NODE_ID.to_string()),
        )
        .unwrap();
    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine =
        CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);
    let _: Result<(), _> = engine
        .fail_provider_stream_ended(&unrelated, support::FAILED_NODE_ID)
        .await;
    let mut unrelated = fixture
        .store
        .get_attempt(&unrelated.project_id, &unrelated.issue_id, &unrelated.id)
        .unwrap();
    assert_eq!(
        fixture
            .store
            .list_open_blocked_gates(&unrelated.project_id, &unrelated.issue_id, &unrelated.id,)
            .unwrap()[0]
            .gate_id,
        gate_id,
        "gate IDs must collide across attempts for this regression"
    );
    unrelated.status = CodingAttemptStatus::AwaitingPlanAmendment;
    fixture.store.save_coding_attempt(&unrelated).unwrap();

    let recovered = engine
        .recover_failed_code_review(&gate_id)
        .await
        .expect("the unique recoverable attempt must win over an unrelated blocked candidate");

    assert_eq!(recovered.project_id, fixture.attempt.project_id);
    assert_eq!(recovered.issue_id, fixture.attempt.issue_id);
    assert_eq!(recovered.id, fixture.attempt.id);
    assert_eq!(
        fixture
            .store
            .get_attempt(&unrelated.project_id, &unrelated.issue_id, &unrelated.id)
            .unwrap()
            .status,
        CodingAttemptStatus::AwaitingPlanAmendment
    );
    assert!(
        fixture
            .store
            .get_failed_code_review_recovery_journal(
                &unrelated.project_id,
                &unrelated.issue_id,
                &unrelated.id,
            )
            .unwrap()
            .is_none()
    );
    assert_eq!(
        fixture
            .store
            .list_open_blocked_gates(&unrelated.project_id, &unrelated.issue_id, &unrelated.id,)
            .unwrap()
            .len(),
        1,
        "recovery must not create a second gate for the blocked candidate"
    );
}
