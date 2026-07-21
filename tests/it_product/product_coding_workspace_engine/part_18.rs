#[tokio::test]
async fn handle_abort_marks_attempt_aborted_and_closes_active_timeline_node() {
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input())
        .expect("create attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing stage");
    store
        .save_timeline_node(&attempt, CodingTimelineNode {
            id: "coding_node_0001".to_string(),
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::Testing,
            title: "执行测试".to_string(),
            status: CodingTimelineNodeStatus::Running,
            agent_role: Some(CodingAgentRole::Tester),
            summary: None,
            started_at: "2026-05-23T00:00:00Z".to_string(),
            completed_at: None,
            artifact_refs: Vec::new(),
        })
        .expect("save testing node");
    let (tx, mut rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let updated = engine
        .handle_abort("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("handle abort");

    assert_eq!(updated.status, CodingAttemptStatus::Aborted);
    assert_eq!(updated.stage, CodingExecutionStage::Testing);
    assert!(updated.completed_at.is_some());
    let nodes = store
        .get_timeline_nodes("project_0001", "issue_0001", &attempt.id)
        .expect("timeline nodes");
    assert_eq!(nodes[0].status, CodingTimelineNodeStatus::Failed);
    assert_eq!(nodes[0].summary.as_deref(), Some("用户已中止"));
    assert!(nodes[0].completed_at.is_some());

    match rx.recv().await.expect("abort timeline update") {
        CodingWsOutMessage::CodingTimelineNodeUpdated {
            node_id,
            status,
            summary,
            completed_at,
        } => {
            assert_eq!(node_id, "coding_node_0001");
            assert_eq!(status, CodingTimelineNodeStatus::Failed);
            assert_eq!(summary.as_deref(), Some("用户已中止"));
            assert!(completed_at.is_some());
        }
        other => panic!("expected abort timeline update, got {other:?}"),
    }

    let attempt_before_retry = store
        .get_attempt("project_0001", "issue_0001", &attempt.id)
        .expect("attempt before idempotent abort");
    let nodes_before_retry = store
        .get_timeline_nodes("project_0001", "issue_0001", &attempt.id)
        .expect("timeline before idempotent abort");
    let retried = engine
        .handle_abort("project_0001", "issue_0001", &attempt.id)
        .await
        .expect("idempotent abort");

    assert_eq!(retried, attempt_before_retry);
    assert_eq!(
        store
            .get_attempt("project_0001", "issue_0001", &attempt.id)
            .expect("attempt after idempotent abort"),
        attempt_before_retry
    );
    assert_eq!(
        store
            .get_timeline_nodes("project_0001", "issue_0001", &attempt.id)
            .expect("timeline after idempotent abort"),
        nodes_before_retry
    );
    assert!(rx.try_recv().is_err(), "idempotent abort emitted an event");
}
