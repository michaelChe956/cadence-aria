use super::*;

#[tokio::test]
async fn code_review_blocked_gate_accepts_manual_feedback_without_findings() {
    let paths = ProductAppPaths::new(tempdir().expect("tempdir").path().join(".aria"));
    let store = CodingAttemptStore::new(paths);
    let mut attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("create attempt");
    attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("blocked");
    store
        .save_code_review_report(
            &attempt,
            &CodeReviewReport {
                id: "code_review_report_0001".to_string(),
                attempt_id: attempt.id.clone(),
                round: 1,
                verdict: ReviewVerdict::Blocked,
                findings: Vec::new(),
                tested_evidence_refs: Vec::new(),
                diff_refs: Vec::new(),
                summary: "review 输出不是有效 JSON，已阻塞并等待人工确认".to_string(),
                created_at: "2026-07-07T00:00:00Z".to_string(),
                raw_provider_output_ref: Some(
                    "provider-raw/code_review/code_review_0001.txt".to_string(),
                ),
                role_run_id: None,
                run_no: Some(1),
                unit_run_id: None,
            },
        )
        .expect("code review report");
    let gate = store
        .create_blocked_gate(
            &attempt,
            CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::CodeReview,
                node_id: Some("coding_node_0001".to_string()),
                role: Some(CodingProviderRole::CodeReviewer),
                title: "Code review blocked".to_string(),
                description: "review 输出不是有效 JSON".to_string(),
                reason_code: Some("code_review_blocked".to_string()),
                evidence_refs: vec!["code_review_report_0001".to_string()],
                raw_provider_output_ref: Some(
                    "provider-raw/code_review/code_review_0001.txt".to_string(),
                ),
                available_actions: vec![
                    coding_gate_action_for_id("retry_review").expect("retry review action"),
                    coding_gate_action_for_id("abort").expect("abort action"),
                ],
            },
        )
        .expect("blocked gate");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let restored_gate = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("open gates")
        .into_iter()
        .next()
        .expect("restored gate");
    assert!(
        restored_gate
            .available_actions
            .iter()
            .any(|action| action.action_id == "send_to_coder")
    );

    let updated = engine
        .handle_blocked_gate_response(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate.gate_id,
            "send_to_coder",
            Some("人工意见：按截图里的问题直接修复".to_string()),
        )
        .await
        .expect("send to coder from code review gate");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::Coding);
    assert_eq!(updated.rework_count, 1);
    let notes = store
        .list_context_notes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("context notes");
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].content, "人工意见：按截图里的问题直接修复");
    let instructions = store
        .list_rework_instructions(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("rework instructions");
    assert_eq!(instructions.len(), 1);
    assert_eq!(
        instructions[0].summary,
        "review 输出不是有效 JSON，已阻塞并等待人工确认"
    );
}
