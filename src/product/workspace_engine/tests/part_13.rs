#[test]
fn legacy_work_item_plan_candidate_revise_reopens_outline() {
    let (_tmp, store) = setup();
    let (tx, _rx) = mpsc::channel(16);
    let mut session = make_session("sess_review_legacy_candidate");
    session.workspace_type = WorkspaceType::WorkItemPlan;
    let engine = WorkspaceEngine::new(store, tx, session);
    let completion = crate::cross_cutting::streaming_provider::ProviderCompletion {
        full_output: "legacy candidate revise".to_string(),
        readable_output: "legacy candidate revise".to_string(),
        structured_output:
            crate::cross_cutting::structured_output::StructuredOutputState::Parsed(
                serde_json::json!({
                    "verdict": "revise",
                    "review_scope": "outline",
                    "generation_round_id": "legacy_work_item_plan_candidate",
                    "findings": [{
                        "severity": "must_fix",
                        "message": "拆分边界需要调整"
                    }]
                }),
            ),
        provider_session_id: None,
    };

    let verdict = engine
        .parse_review_completion_for_active_node(&completion)
        .expect("legacy candidate review should parse as outline");
    let review = verdict
        .work_item_plan_review
        .expect("work item plan review extension");
    assert_eq!(review.review_scope, WorkItemPlanReviewScope::Outline);
    assert_eq!(review.review_action, WorkItemPlanReviewAction::ReviseOutline);
    assert_eq!(
        review.gates,
        vec![WorkItemPlanReviewGate::RequiresPlanReopen]
    );
}

#[test]
fn persistent_work_item_plan_recovery_rejects_invalid_legacy_review_payloads() {
    let invalid_payloads = [
        serde_json::json!({
            "verdict": "pass",
            "generation_round_id": "",
            "findings": [{"severity": "suggestion", "message": "不可恢复"}]
        }),
        serde_json::json!({
            "verdict": "revise",
            "generation_round_id": "legacy_round",
            "target_outline_id": "outline_missing",
            "findings": [{"severity": "must_fix", "message": "非法引用"}]
        }),
    ];

    for (index, payload) in invalid_payloads.into_iter().enumerate() {
        let (tmp, checkpoint_store) = setup();
        let lifecycle_store =
            LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
        let (tx, _rx) = mpsc::channel(16);
        let mut session = make_session(&format!("sess_wip_recovery_{index}"));
        session.workspace_type = WorkspaceType::WorkItemPlan;
        session.messages.push(SessionMessage {
            id: "msg_reviewer".to_string(),
            role: "reviewer".to_string(),
            content: format!("legacy review\n```json\n{payload}\n```"),
            checkpoint_id: None,
            created_at: chrono::Utc::now().to_rfc3339(),
        });

        let engine = WorkspaceEngine::new_persistent(
            checkpoint_store,
            lifecycle_store,
            tx,
            session,
        );
        let verdict = engine
            .latest_review_verdict
            .as_ref()
            .expect("legacy reviewer message should recover safe fallback");

        assert_eq!(verdict.verdict, ReviewVerdictType::NeedsHuman);
        assert_eq!(verdict.review_gate, ReviewGate::UserTriageRequired);
        assert!(verdict.findings.is_empty());
        assert!(verdict.work_item_plan_review.is_none());
    }
}

#[test]
fn persistent_general_workspace_recovery_keeps_generic_review_contract() {
    for workspace_type in [
        WorkspaceType::Story,
        WorkspaceType::Design,
        WorkspaceType::WorkItem,
    ] {
        let (tmp, checkpoint_store) = setup();
        let lifecycle_store =
            LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
        let (tx, _rx) = mpsc::channel(16);
        let mut session = make_session(&format!("sess_recovery_{workspace_type:?}"));
        session.workspace_type = workspace_type;
        session.messages.push(SessionMessage {
            id: "msg_reviewer".to_string(),
            role: "reviewer".to_string(),
            content: "legacy review\n```json\n{\"verdict\":\"pass\",\"findings\":[]}\n```"
                .to_string(),
            checkpoint_id: None,
            created_at: chrono::Utc::now().to_rfc3339(),
        });

        let engine = WorkspaceEngine::new_persistent(
            checkpoint_store,
            lifecycle_store,
            tx,
            session,
        );
        let verdict = engine
            .latest_review_verdict
            .as_ref()
            .expect("generic reviewer message should recover");

        assert_eq!(verdict.verdict, ReviewVerdictType::Pass);
        assert_eq!(verdict.review_gate, ReviewGate::UserConfirmAllowed);
        assert!(verdict.work_item_plan_review.is_none());
    }
}
