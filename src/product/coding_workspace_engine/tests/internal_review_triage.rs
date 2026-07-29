use super::*;

#[tokio::test]
async fn group_final_review_blocked_actionable_finding_routes_to_coder_fix_gate() {
    let (_root, store, attempt, _engine, _event_rx, review) =
        execute_group_final_review_run_coder_fix().await;

    assert_eq!(review.verdict, ReviewVerdict::Blocked);
    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("persisted attempt");
    assert_eq!(persisted.status, CodingAttemptStatus::Blocked);
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("blocked gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("group_final_review_blocked")
    );
    assert_eq!(
        gates[0]
            .available_actions
            .iter()
            .map(|action| action.action_id.as_str())
            .collect::<Vec<_>>(),
        vec!["retry_internal_review", "manual_continue", "abort"]
    );
    let entry = store
        .list_chat_entries(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("chat entries")
        .into_iter()
        .find(|entry| {
            entry
                .metadata
                .as_ref()
                .is_some_and(|metadata| metadata["source"] == "internal_pr_review")
        })
        .expect("group final review chat entry");
    assert_eq!(
        entry.metadata.as_ref().expect("metadata")["plan_defect_route"],
        "run_coder_fix"
    );
}

#[tokio::test]
async fn group_final_review_retry_internal_review_does_not_start_plan_repair() {
    let (_root, store, attempt, engine, _event_rx, _review) =
        execute_group_final_review_run_coder_fix().await;
    let gate_id = single_open_gate_id(&store, &attempt);

    let updated = engine
        .handle_blocked_gate_response(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate_id,
            "retry_internal_review",
            None,
        )
        .await
        .expect("retry internal review");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::InternalPrReview);
    assert_no_plan_repair_or_lifecycle_child(&store, &attempt);
}

#[tokio::test]
async fn group_final_review_manual_continue_does_not_start_plan_repair() {
    let (_root, store, attempt, engine, _event_rx, _review) =
        execute_group_final_review_run_coder_fix().await;
    let gate_id = single_open_gate_id(&store, &attempt);

    let updated = engine
        .handle_blocked_gate_response(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate_id,
            "manual_continue",
            Some("operator accepts the review risk".to_string()),
        )
        .await
        .expect("manual continue");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_no_plan_repair_or_lifecycle_child(&store, &attempt);
}

#[tokio::test]
async fn group_final_review_abort_does_not_start_plan_repair() {
    let (_root, store, attempt, engine, _event_rx, _review) =
        execute_group_final_review_run_coder_fix().await;
    let gate_id = single_open_gate_id(&store, &attempt);

    let updated = engine
        .handle_blocked_gate_response(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate_id,
            "abort",
            None,
        )
        .await
        .expect("abort review attempt");

    assert_eq!(updated.status, CodingAttemptStatus::Aborted);
    assert_no_plan_repair_or_lifecycle_child(&store, &attempt);
}

async fn execute_group_final_review_run_coder_fix() -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingExecutionAttempt,
    CodingWorkspaceEngine,
    mpsc::Receiver<CodingWsOutMessage>,
    InternalPrReview,
) {
    let (root, store, attempt, engine, event_rx) =
        super::plan_defect_entrypoints::prepared_group_review_fixture();
    let provider = super::provider_execution_context::CapturingProjectionProvider::new(
        serde_json::json!({
            "verdict": "blocked",
            "summary": "implementation needs rework",
            "findings": [{
                "source_stage": "group_final_review",
                "severity": "error",
                "file_path": "src/lib.rs",
                "line": 1,
                "message": "required error handling is missing",
                "required_action": "add the missing error handling",
                "defect_class": "implementation_defect",
                "recommended_route": "coder_rework"
            }],
            "impact_scope": ["src/lib.rs"],
            "pr_description": "",
            "commit_message_suggestion": ""
        })
        .to_string(),
    );
    let review = engine
        .execute_internal_pr_review(&attempt, &provider)
        .await
        .expect("blocked group review with actionable implementation finding");
    (root, store, attempt, engine, event_rx, review)
}

fn single_open_gate_id(store: &CodingAttemptStore, attempt: &CodingExecutionAttempt) -> String {
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("blocked gates");
    assert_eq!(gates.len(), 1);
    gates[0].gate_id.clone()
}

fn assert_no_plan_repair_or_lifecycle_child(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) {
    let revision_store = WorkItemRevisionStore::new(store.paths());
    let lineage = revision_store
        .get_plan_lineage(
            &attempt.project_id,
            &attempt.issue_id,
            attempt
                .work_item_group_id
                .as_deref()
                .expect("group review fixture plan id"),
        )
        .expect("plan lineage");
    assert!(
        revision_store
            .list_repair_requests(&lineage)
            .expect("plan repair requests")
            .is_empty(),
        "triage action must not create a plan repair request"
    );
    let lifecycle = LifecycleStore::new(store.paths());
    assert!(
        lifecycle
            .list_session_links(&attempt.project_id, &attempt.issue_id)
            .expect("lifecycle session links")
            .is_empty(),
        "triage action must not create a lifecycle child session link"
    );
    assert!(
        lifecycle
            .list_workspace_sessions(&attempt.project_id, &attempt.issue_id)
            .expect("lifecycle workspace sessions")
            .is_empty(),
        "triage action must not create a lifecycle child session"
    );
}

#[tokio::test]
async fn group_final_review_operational_decision_lands_one_blocked_gate() {
    let (_root, store, attempt, engine, _event_rx) =
        super::plan_defect_entrypoints::prepared_group_review_fixture();
    let provider = super::provider_execution_context::CapturingProjectionProvider::new(
        serde_json::json!({
            "verdict": "request_changes",
            "summary": "provider environment is unavailable",
            "findings": [{
                "source_stage": "group_final_review",
                "severity": "error",
                "defect_class": "operational_blocker",
                "reason_code": "operational_blocker",
                "message": "required provider is unavailable",
                "contract_refs": [],
                "capability_refs": [],
                "repair_target": null,
                "recommended_route": "operational_gate",
                "confidence": "high",
                "evidence": []
            }],
            "impact_scope": [],
            "pr_description": "",
            "commit_message_suggestion": ""
        })
        .to_string(),
    );

    let review = engine
        .execute_internal_pr_review(&attempt, &provider)
        .await
        .expect("group review operational blocker");

    assert_eq!(review.verdict, ReviewVerdict::RequestChanges);
    let persisted = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("persisted attempt");
    assert_eq!(persisted.status, CodingAttemptStatus::Blocked);
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("blocked gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("internal_review_operational_blocker")
    );
    assert_eq!(gates[0].title, "GroupFinalReview operational blocker");
}
