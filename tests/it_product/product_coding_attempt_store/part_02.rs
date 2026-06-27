#[test]
fn store_persists_and_resolves_choice_gates_in_attempt_scope() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");

    let gate = store
        .create_choice_gate(CreateChoiceGateInput {
            attempt_id: attempt.id.clone(),
            choice_id: "choice_0001".to_string(),
            stage: CodingExecutionStage::Coding,
            node_id: Some("coding_node_0001".to_string()),
            role: CodingProviderRole::Coder,
            provider: ProviderName::Codex,
            source: "request_user_input".to_string(),
            prompt: "请选择实现范围".to_string(),
            options: vec![CodingChoiceOption {
                id: "backend_first".to_string(),
                label: "先做后端".to_string(),
                description: Some("TASK-001 到 TASK-009".to_string()),
            }],
            allow_multiple: false,
            allow_free_text: true,
        })
        .expect("create choice gate");

    assert_eq!(gate.gate_id, "coding_choice_gate_0001");
    assert_eq!(gate.choice_id, "choice_0001");
    assert_eq!(gate.attempt_id, attempt.id);
    assert_eq!(gate.status, CodingChoiceGateStatus::Open);
    assert_eq!(gate.provider, ProviderName::Codex);
    assert_eq!(gate.source, "request_user_input");
    assert_eq!(
        store
            .list_open_choice_gates("project_0001", "issue_0001", &attempt.id)
            .expect("open choice gates")
            .len(),
        1
    );

    let resolved = store
        .resolve_choice_gate(
            "project_0001",
            "issue_0001",
            &attempt.id,
            "choice_0001",
            vec!["backend_first".to_string()],
            Some("先控制范围".to_string()),
        )
        .expect("resolve choice gate");

    assert_eq!(resolved.status, CodingChoiceGateStatus::Resolved);
    assert_eq!(
        resolved
            .response
            .as_ref()
            .expect("response")
            .selected_option_ids,
        vec!["backend_first"]
    );
    assert!(
        store
            .list_open_choice_gates("project_0001", "issue_0001", &attempt.id)
            .expect("open choice gates")
            .is_empty()
    );
}

#[test]
fn status_and_stage_transitions_reject_invalid_backwards_moves() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");

    assert!(
        store
            .update_attempt_status(
                "project_0001",
                "issue_0001",
                &attempt.id,
                CodingAttemptStatus::Completed,
            )
            .is_err(),
        "created cannot jump directly to completed"
    );

    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("forward stage");
    assert!(
        store
            .update_attempt_stage(
                "project_0001",
                "issue_0001",
                &attempt.id,
                CodingExecutionStage::Coding,
            )
            .is_err(),
        "stage cannot move backwards outside rework"
    );
    store
        .update_attempt_stage(
            "project_0001",
            "issue_0001",
            &attempt.id,
            CodingExecutionStage::Rework,
        )
        .expect("enter rework");
}

fn create_input(work_item_id: &str) -> CreateCodingAttemptInput {
    CreateCodingAttemptInput {
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: work_item_id.to_string(),
        base_branch: "main".to_string(),
        branch_name: format!("aria/work-items/{work_item_id}/attempt-1"),
        worktree_path: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Fake,
            reviewer: Some(ProviderName::Fake),
            review_rounds: 1,
        },
        max_auto_rework: 2,
    }
}

fn sample_testing_report(attempt_id: &str) -> TestingReport {
    TestingReport {
        id: "testing_report_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        role_run_id: None,
        run_no: None,
        commands: vec![TestCommand {
            command: vec!["cargo".to_string(), "test".to_string()],
            cwd: PathBuf::from("/tmp/worktree"),
            exit_code: Some(0),
            duration_ms: 100,
            stdout_ref: "artifacts/stdout.txt".to_string(),
            stderr_ref: "artifacts/stderr.txt".to_string(),
            status: TestCommandStatus::Passed,
        }],
        overall_status: TestingOverallStatus::Passed,
        provider_claim: None,
        backend_verified: true,
        started_at: "2026-05-23T00:00:00Z".to_string(),
        completed_at: Some("2026-05-23T00:01:00Z".to_string()),
        plan_id: None,
        plan_summary: None,
        steps: Vec::new(),
        unplanned_commands: Vec::new(),
        unplanned_evidence: Vec::new(),
        missing_required_steps: Vec::new(),
        skipped_required_steps: Vec::new(),
        context_warnings: Vec::new(),
        raw_provider_output_ref: None,
    }
}

fn sample_code_review_report(attempt_id: &str) -> CodeReviewReport {
    CodeReviewReport {
        id: "code_review_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        round: 1,
        verdict: ReviewVerdict::Approve,
        findings: vec![sample_finding()],
        tested_evidence_refs: vec!["testing_report_0001".to_string()],
        diff_refs: vec!["diff_0001".to_string()],
        summary: "通过".to_string(),
        created_at: "2026-05-23T00:01:00Z".to_string(),
        raw_provider_output_ref: None,
        role_run_id: None,
        run_no: None,
    }
}

fn sample_review_request(attempt_id: &str) -> ReviewRequest {
    ReviewRequest {
        id: "review_request_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        kind: ReviewRequestKind::GitBranchOnly,
        remote_kind: RemoteKind::GenericGit,
        remote: "origin".to_string(),
        base_branch: "main".to_string(),
        branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
        commit_sha: "abc123".to_string(),
        push_status: PushStatus::Pushed,
        external_url: None,
        manual_instructions: vec![],
        created_at: "2026-05-23T00:02:00Z".to_string(),
        updated_at: "2026-05-23T00:02:00Z".to_string(),
    }
}

fn sample_internal_review(attempt_id: &str, review_request_id: &str) -> InternalPrReview {
    InternalPrReview {
        id: "internal_review_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        review_request_id: review_request_id.to_string(),
        verdict: ReviewVerdict::Approve,
        findings: vec![sample_finding()],
        impact_scope: vec!["src/lib.rs".to_string()],
        pr_description: "实现 work item".to_string(),
        commit_message_suggestion: "feat: implement work item".to_string(),
        tested_evidence_refs: vec!["testing_report_0001".to_string()],
        diff_refs: vec!["diff_0001".to_string()],
        summary: "最终审查通过".to_string(),
        created_at: "2026-05-23T00:03:00Z".to_string(),
        raw_provider_output_ref: None,
        role_run_id: None,
        run_no: None,
    }
}

fn sample_finding() -> ReviewFinding {
    ReviewFinding {
        severity: FindingSeverity::Info,
        file_path: Some("src/lib.rs".to_string()),
        line: Some(1),
        message: "ok".to_string(),
        required_action: None,
        source_stage: CodingExecutionStage::CodeReview,
        evidence: Vec::new(),
        related_requirements: Vec::new(),
        related_design_constraints: Vec::new(),
        related_work_item_tasks: Vec::new(),
    }
}

fn sample_node(attempt_id: &str) -> CodingTimelineNode {
    CodingTimelineNode {
        id: "coding_node_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        stage: CodingExecutionStage::Testing,
        title: "测试".to_string(),
        status: CodingTimelineNodeStatus::Running,
        agent_role: Some(CodingAgentRole::Tester),
        summary: None,
        started_at: "2026-05-23T00:01:00Z".to_string(),
        completed_at: None,
        artifact_refs: vec![],
    }
}

#[test]
fn updates_coding_role_run_refs_without_duplicates() {
    let root = tempfile::tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            worktree_path: None,
            ..create_input("work_item_0001")
        })
        .expect("create attempt");
    let run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Rework,
            CodingProviderRole::Analyst,
            CodingRoleRunTrigger::Initial,
            None,
        )
        .expect("role run");

    let updated = store
        .update_role_run_refs(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &run.id,
            vec!["provider-raw/rework/analyst_decision_0001.txt".to_string()],
            vec!["provider-raw/rework/analyst_evidence_0001.txt".to_string()],
        )
        .expect("update refs");
    assert_eq!(updated.raw_provider_output_refs.len(), 1);
    assert_eq!(updated.artifact_refs.len(), 1);

    let updated = store
        .update_role_run_refs(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &run.id,
            vec!["provider-raw/rework/analyst_decision_0001.txt".to_string()],
            vec!["provider-raw/rework/analyst_evidence_0001.txt".to_string()],
        )
        .expect("update refs again");
    assert_eq!(updated.raw_provider_output_refs.len(), 1);
    assert_eq!(updated.artifact_refs.len(), 1);
}

#[test]
fn appends_and_lists_coding_role_run_events_in_sequence() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");
    let run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0003".to_string()),
        )
        .expect("role run");

    let first = store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ProviderPrompt,
            serde_json::json!({
                "mode": "plan_tests",
                "prompt": "plan tests as JSON"
            }),
        )
        .expect("append first event");
    let second = store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::TextDelta,
            serde_json::json!({
                "content": "No tasks found"
            }),
        )
        .expect("append second event");

    assert_eq!(first.sequence, 1);
    assert_eq!(second.sequence, 2);
    assert_eq!(first.attempt_id, attempt.id);
    assert_eq!(first.role_run_id, run.id);
    assert_eq!(first.node_id.as_deref(), Some("coding_node_0003"));
    assert_eq!(first.stage, CodingExecutionStage::Testing);
    assert_eq!(first.role, CodingProviderRole::Tester);
    assert_eq!(second.node_id.as_deref(), Some("coding_node_0003"));

    let events = store
        .list_role_run_events("project_0001", "issue_0001", &attempt.id, &run.id)
        .expect("events");
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].attempt_id, attempt.id);
    assert_eq!(events[0].role_run_id, run.id);
    assert_eq!(events[0].node_id.as_deref(), Some("coding_node_0003"));
    assert_eq!(events[0].stage, CodingExecutionStage::Testing);
    assert_eq!(events[0].role, CodingProviderRole::Tester);
    assert_eq!(events[0].event_type, CodingRoleRunEventType::ProviderPrompt);
    assert_eq!(events[1].event_type, CodingRoleRunEventType::TextDelta);
    assert_eq!(events[1].payload["content"], "No tasks found");
}

#[test]
fn role_run_event_large_string_payload_is_moved_to_artifact() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");
    let run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::CodeReview,
            CodingProviderRole::CodeReviewer,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0007".to_string()),
        )
        .expect("role run");
    let long_prompt = "review this diff\n".repeat(2_000);

    let event = store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ProviderPrompt,
            serde_json::json!({
                "mode": "full_conversation",
                "prompt": long_prompt
            }),
        )
        .expect("append event");

    assert!(event.truncated);
    assert_eq!(
        event.artifact_ref.as_deref(),
        Some("artifacts/role-run-events/coding_role_run_0001/0001_prompt.txt")
    );
    assert_eq!(
        event.payload["prompt"]["artifact_ref"],
        "artifacts/role-run-events/coding_role_run_0001/0001_prompt.txt"
    );
    assert_eq!(event.payload["prompt"]["truncated"], true);
    let preview = event.payload["prompt"]["preview"]
        .as_str()
        .expect("preview string");
    assert!(preview.starts_with("review this diff"));
    assert!(preview.len() <= 16_384);

    let artifact = store
        .read_attempt_artifact_text(&attempt.id, event.artifact_ref.as_deref().expect("ref"))
        .expect("artifact text");
    assert_eq!(artifact, long_prompt);
}

#[test]
fn role_run_event_truncates_each_large_payload_field() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");
    let run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0011".to_string()),
        )
        .expect("role run");
    let long_stdout = "stdout line\n".repeat(2_000);
    let long_stderr = "stderr line\n".repeat(2_000);

    let event = store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "command": "cargo test --locked",
                "stdout": long_stdout,
                "stderr": long_stderr
            }),
        )
        .expect("append event");

    assert!(event.truncated);
    assert_eq!(
        event.artifact_ref.as_deref(),
        Some("artifacts/role-run-events/coding_role_run_0001/0001_stdout.txt")
    );

    let stdout_payload = event.payload["stdout"].as_object().expect("stdout object");
    let stderr_payload = event.payload["stderr"].as_object().expect("stderr object");
    assert!(
        stdout_payload["preview"]
            .as_str()
            .expect("stdout preview")
            .len()
            <= 16_384
    );
    assert!(
        stderr_payload["preview"]
            .as_str()
            .expect("stderr preview")
            .len()
            <= 16_384
    );
    assert_eq!(stdout_payload["truncated"], true);
    assert_eq!(stderr_payload["truncated"], true);
    let stdout_ref = stdout_payload["artifact_ref"]
        .as_str()
        .expect("stdout artifact ref");
    let stderr_ref = stderr_payload["artifact_ref"]
        .as_str()
        .expect("stderr artifact ref");
    assert_ne!(stdout_ref, stderr_ref);

    let stdout_artifact = store
        .read_attempt_artifact_text(&attempt.id, stdout_ref)
        .expect("stdout artifact text");
    let stderr_artifact = store
        .read_attempt_artifact_text(&attempt.id, stderr_ref)
        .expect("stderr artifact text");
    assert_eq!(stdout_artifact, long_stdout);
    assert_eq!(stderr_artifact, long_stderr);
}

#[test]
fn role_run_retry_diagnostic_summary_compacts_events_and_refs() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");
    let run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0003".to_string()),
        )
        .expect("role run");
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Task update",
                "status": "running",
                "detail": "No tasks found"
            }),
        )
        .expect("event");
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::Timeout,
            serde_json::json!({
                "reason_code": "plan_tests_timeout",
                "message": "Tester provider timed out"
            }),
        )
        .expect("timeout");
    store
        .update_role_run_refs(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &run.id,
            vec!["provider-raw/testing/plan_tests_0001.txt".to_string()],
            vec!["artifacts/role-run-events/coding_role_run_0001/0001_output.txt".to_string()],
        )
        .expect("refs");
    store
        .update_role_run_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &run.id,
            CodingRoleRunStatus::Blocked,
            Some("plan_tests_timeout".to_string()),
        )
        .expect("blocked");

    let summary = store
        .role_run_retry_diagnostic_summary("project_0001", "issue_0001", &attempt.id, &run.id)
        .expect("summary")
        .expect("summary text");

    assert!(summary.contains("role_run_id: coding_role_run_0001"));
    assert!(summary.contains("reason_code: plan_tests_timeout"));
    assert!(summary.contains("terminal_event: timeout"));
    assert!(summary.contains("Task update"));
    assert!(summary.contains("No tasks found"));
    assert!(summary.contains("provider-raw/testing/plan_tests_0001.txt"));
    assert!(summary.contains("artifacts/role-run-events/coding_role_run_0001/0001_output.txt"));
    assert!(
        summary.len() < 8_000,
        "retry diagnostic summary must stay prompt-safe"
    );
}

#[test]
fn role_run_retry_diagnostic_summary_keeps_recent_metadata_and_payload_refs() {
    let root = tempdir().expect("tempdir");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_attempt(create_input("work_item_0001"))
        .expect("create attempt");
    let run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0004".to_string()),
        )
        .expect("role run");

    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Old event",
                "status": "running",
                "detail": "Dropped old event"
            }),
        )
        .expect("old event");
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::TextDelta,
            serde_json::json!({
                "content": "DROPPED_TEXT_DELTA_BODY"
            }),
        )
        .expect("old text delta");
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Recent setup",
                "status": "running",
                "detail": "Preparing test run"
            }),
        )
        .expect("recent setup");
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::TextDelta,
            serde_json::json!({
                "content": "DO_NOT_INJECT_TEXT_DELTA_BODY"
            }),
        )
        .expect("recent text delta");
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Recent event 5",
                "status": "running",
                "detail": "Still running"
            }),
        )
        .expect("recent event 5");
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Recent event 6",
                "status": "running",
                "detail": "Almost done"
            }),
        )
        .expect("recent event 6");
    let long_stdout = "stdout line\n".repeat(2_000);
    let long_stderr = "stderr line\n".repeat(2_000);
    store
        .append_role_run_event(
            &attempt,
            &run,
            CodingRoleRunEventType::ExecutionEvent,
            serde_json::json!({
                "title": "Cargo test",
                "status": "failed",
                "detail": "Captured command output",
                "stdout": long_stdout,
                "stderr": long_stderr
            }),
        )
        .expect("captured output");
    store
        .update_role_run_status(
            "project_0001",
            "issue_0001",
            &attempt.id,
            &run.id,
            CodingRoleRunStatus::Blocked,
            Some("tests_failed".to_string()),
        )
        .expect("blocked");

    let summary = store
        .role_run_retry_diagnostic_summary("project_0001", "issue_0001", &attempt.id, &run.id)
        .expect("summary")
        .expect("summary text");

    assert!(!summary.contains("DO_NOT_INJECT_TEXT_DELTA_BODY"));
    assert!(!summary.contains("Dropped old event"));
    assert!(summary.contains("artifacts/role-run-events/coding_role_run_0001/0007_stdout.txt"));
    assert!(summary.contains("artifacts/role-run-events/coding_role_run_0001/0007_stderr.txt"));
    assert!(
        summary.len() <= 8_000,
        "retry diagnostic summary must stay prompt-safe"
    );
}

