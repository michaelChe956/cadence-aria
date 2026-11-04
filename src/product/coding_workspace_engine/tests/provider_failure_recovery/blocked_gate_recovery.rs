#[tokio::test]
async fn internal_review_blocked_gates_use_triage_actions_without_coder_rework() {
    for reason_code in [
        "group_final_review_blocked",
        "internal_review_blocked",
        "internal_review_change_requested",
        "internal_review_verification_incomplete",
        "internal_review_human_triage",
        "internal_review_operational_blocker",
    ] {
        let root = tempdir().expect("tempdir");
        let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let attempt = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: WORK_ITEM_ID.to_string(),
                base_branch: "HEAD".to_string(),
                branch_name: "aria/issues/issue_0001".to_string(),
                worktree_path: None,
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::Codex,
                    reviewer: Some(ProviderName::ClaudeCode),
                    review_rounds: 1,
                    permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(
                    ),
                },
                target_snapshot: None,
                max_auto_rework: 2,
            })
            .expect("attempt");
        let attempt = store
            .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &attempt.id)
            .expect("running attempt");
        let attempt = store
            .update_attempt_stage(
                PROJECT_ID,
                ISSUE_ID,
                &attempt.id,
                CodingExecutionStage::InternalPrReview,
            )
            .expect("internal review stage");
        let (tx, _rx) = mpsc::channel(8);
        let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

        engine
            .create_review_blocked_gate(ReviewBlockedGateInput {
                attempt: &attempt,
                node_id: "coding_node_0010",
                stage: CodingExecutionStage::InternalPrReview,
                role: CodingProviderRole::InternalReviewer,
                title: "Internal PR review blocked".to_string(),
                description: "review interrupted".to_string(),
                reason_code,
                evidence_refs: Vec::new(),
                raw_provider_output_ref: None,
            })
            .await
            .expect("internal review blocked gate");

        let persisted = store
            .get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id)
            .expect("blocked attempt");
        assert_eq!(persisted.status, CodingAttemptStatus::Blocked);
        let gate = store
            .list_open_blocked_gates(PROJECT_ID, ISSUE_ID, &attempt.id)
            .expect("open gates")
            .into_iter()
            .next()
            .expect("internal review gate");
        assert_eq!(gate.reason_code.as_deref(), Some(reason_code));
        assert_eq!(
            gate.available_actions
                .iter()
                .map(|action| action.action_id.as_str())
                .collect::<Vec<_>>(),
            vec!["retry_internal_review", "manual_continue", "abort"]
        );
        assert_eq!(gate.available_actions[0].label, "重试 Internal Review");
    }
}

#[tokio::test]
async fn retry_coding_gate_clears_latest_coder_provider_conversation() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let mut role_provider_config = store
        .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role provider config");
    role_provider_config.coder = ProviderName::ClaudeCode;
    store
        .update_role_provider_config_snapshot(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            role_provider_config,
        )
        .expect("update role provider config");
    let attempt = store
        .replace_attempt_provider_conversations(
            &attempt,
            vec![
                ProviderConversationRef {
                    role: ProviderConversationRole::Coder,
                    provider: ProviderName::Codex,
                    provider_session_id: "old-codex-coder-thread".to_string(),
                    updated_at: "2026-07-13T00:00:00Z".to_string(),
                    last_node_id: Some("coding_node_0001".to_string()),
                },
                ProviderConversationRef {
                    role: ProviderConversationRole::Coder,
                    provider: ProviderName::ClaudeCode,
                    provider_session_id: "latest-claude-coder-thread".to_string(),
                    updated_at: "2026-07-13T00:00:01Z".to_string(),
                    last_node_id: Some("coding_node_0002".to_string()),
                },
                ProviderConversationRef {
                    role: ProviderConversationRole::CodeReviewer,
                    provider: ProviderName::ClaudeCode,
                    provider_session_id: "review-thread".to_string(),
                    updated_at: "2026-07-13T00:00:02Z".to_string(),
                    last_node_id: Some("coding_node_0003".to_string()),
                },
            ],
        )
        .expect("seed provider conversations");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Coding,
        )
        .expect("coding stage");
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("blocked attempt");
    let gate = store
        .create_blocked_gate(
            &attempt,
            CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::Coding,
                node_id: Some("coding_node_0002".to_string()),
                role: Some(CodingProviderRole::Coder),
                title: "Coder 执行中断".to_string(),
                description: "provider interrupted".to_string(),
                reason_code: Some("coder_provider_interrupted".to_string()),
                evidence_refs: Vec::new(),
                raw_provider_output_ref: None,
                available_actions: vec![
                    coding_gate_action_for_id("retry_coding").expect("retry coding action"),
                    coding_gate_action_for_id("abort").expect("abort action"),
                ],
            },
        )
        .expect("coder recovery gate");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let resumed = engine
        .handle_blocked_gate_response(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate.gate_id,
            "retry_coding",
            None,
        )
        .await
        .expect("retry coding gate response");

    assert_eq!(resumed.status, CodingAttemptStatus::Running);
    assert_eq!(resumed.stage, CodingExecutionStage::Coding);
    assert!(resumed.provider_conversations.iter().any(|conversation| {
        conversation.role == ProviderConversationRole::Coder
            && conversation.provider == ProviderName::Codex
            && conversation.provider_session_id == "old-codex-coder-thread"
    }));
    assert!(resumed.provider_conversations.iter().all(|conversation| {
        !(conversation.role == ProviderConversationRole::Coder
            && conversation.provider == ProviderName::ClaudeCode)
    }));
    assert!(resumed.provider_conversations.iter().any(|conversation| {
        conversation.role == ProviderConversationRole::CodeReviewer
            && conversation.provider_session_id == "review-thread"
    }));
    assert!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("resolved gate")
            .is_empty()
    );
}
