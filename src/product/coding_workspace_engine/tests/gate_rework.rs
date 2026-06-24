use super::*;

#[test]
fn tester_tool_results_without_step_id_remain_unplanned_evidence() {
    let plan = TestPlan {
        id: "test_plan_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        role_run_id: None,
        run_no: None,
        summary: "unit checks".to_string(),
        context_warnings: Vec::new(),
        assumptions: Vec::new(),
        steps: vec![crate::product::coding_models::TestPlanStep {
            id: "unit".to_string(),
            title: "Unit tests".to_string(),
            intent: "verify unit behavior".to_string(),
            required: true,
            tool: crate::product::coding_models::TestPlanTool::RunCommand,
            risk_level: crate::product::coding_models::TestPlanRiskLevel::Low,
            command_or_tool_input: serde_json::json!({"command": ["true"]}),
            evidence_expectation: "exit 0".to_string(),
            related_requirements: Vec::new(),
            related_design_constraints: Vec::new(),
            related_work_item_tasks: Vec::new(),
        }],
        created_at: "2026-06-10T00:00:00Z".to_string(),
        raw_provider_output_ref: None,
    };
    let calls = [
        ProviderToolCall {
            id: "read_file_0001".to_string(),
            tool_name: "read_file".to_string(),
            input: serde_json::json!({"path": "src/lib.rs"}),
        },
        ProviderToolCall {
            id: "search_code_0001".to_string(),
            tool_name: "search_code".to_string(),
            input: serde_json::json!({"query": "unsafe"}),
        },
        ProviderToolCall {
            id: "run_command_0001".to_string(),
            tool_name: "run_command".to_string(),
            input: serde_json::json!({"command": ["true"]}),
        },
    ];
    let mut step_results = Vec::new();
    let mut unplanned_commands = Vec::new();
    let mut unplanned_evidence = Vec::new();
    let mut context_warnings = Vec::new();
    for call in &calls {
        let command = (call.tool_name == "run_command").then(|| TestCommand {
            command: vec!["true".to_string()],
            cwd: PathBuf::from("/tmp/worktree"),
            exit_code: Some(0),
            duration_ms: 1,
            stdout_ref: "stdout.log".to_string(),
            stderr_ref: "stderr.log".to_string(),
            status: TestCommandStatus::Passed,
        });
        let result = ProviderToolResult {
            tool_use_id: call.id.clone(),
            output: format!("{} ok", call.tool_name),
            is_error: false,
        };
        record_tester_step_result(
            &plan,
            call,
            command,
            &result,
            TesterStepResultOutputs {
                step_results: &mut step_results,
                unplanned_commands: &mut unplanned_commands,
                unplanned_evidence: &mut unplanned_evidence,
                context_warnings: &mut context_warnings,
            },
        );
    }

    let mut report = build_plan_based_testing_report(
        "testing_report_0001",
        "coding_attempt_0001",
        &plan,
        step_results,
        unplanned_commands,
        None,
        None,
    );
    report.unplanned_evidence = unplanned_evidence;

    assert_eq!(report.overall_status, TestingOverallStatus::Blocked);
    assert_eq!(report.missing_required_steps, vec!["unit"]);
    assert!(report.steps.is_empty());
    assert_eq!(report.unplanned_commands.len(), 1);
    assert_eq!(report.unplanned_evidence.len(), 3);
}

#[tokio::test]
async fn blocked_gate_response_is_idempotent_across_reconnects() {
    let store = CodingAttemptStore::new(ProductAppPaths::new(
        tempdir().expect("tempdir").path().join(".aria"),
    ));
    let attempt = store
        .create_attempt(
            crate::product::coding_attempt_store::CreateCodingAttemptInput {
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
                },
                max_auto_rework: 2,
            },
        )
        .expect("create attempt");
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing");
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("blocked");
    let gate = store
        .create_blocked_gate(CreateBlockedGateInput {
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::Testing,
            node_id: Some("coding_node_0001".to_string()),
            role: Some(CodingProviderRole::Tester),
            title: "Testing blocked".to_string(),
            description: "missing required step".to_string(),
            reason_code: Some("missing_required_steps".to_string()),
            evidence_refs: vec!["testing_report_0001.json".to_string()],
            raw_provider_output_ref: None,
            available_actions: testing_blocked_gate_actions(),
        })
        .expect("blocked gate");
    assert_eq!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates")
            .len(),
        1
    );
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let _updated = engine
        .handle_blocked_gate_response(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate.gate_id,
            "retry_test_plan",
            None,
        )
        .await
        .expect("first response");
    assert!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates after resolve")
            .is_empty()
    );

    let second = engine
        .handle_blocked_gate_response(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate.gate_id,
            "retry_test_plan",
            None,
        )
        .await
        .expect("second response is idempotent");
    assert_eq!(second.status, CodingAttemptStatus::Running);
    assert_eq!(second.stage, CodingExecutionStage::Testing);
}

#[tokio::test]
async fn manual_continue_persists_quality_bypass_audit_and_injects_reviewer_context() {
    let paths = ProductAppPaths::new(tempdir().expect("tempdir").path().join(".aria"));
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = store
        .create_attempt(
            crate::product::coding_attempt_store::CreateCodingAttemptInput {
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
                },
                max_auto_rework: 2,
            },
        )
        .expect("create attempt");
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Testing,
        )
        .expect("testing");
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("blocked");
    store
        .save_testing_report(&TestingReport {
            id: "testing_report_0001".to_string(),
            attempt_id: attempt.id.clone(),
            role_run_id: None,
            run_no: None,
            commands: Vec::new(),
            overall_status: TestingOverallStatus::Blocked,
            provider_claim: None,
            backend_verified: true,
            started_at: "2026-06-10T00:00:00Z".to_string(),
            completed_at: Some("2026-06-10T00:00:01Z".to_string()),
            plan_id: Some("test_plan_0001".to_string()),
            plan_summary: Some("unit checks".to_string()),
            steps: Vec::new(),
            unplanned_commands: Vec::new(),
            unplanned_evidence: Vec::new(),
            missing_required_steps: vec!["unit".to_string()],
            skipped_required_steps: Vec::new(),
            context_warnings: Vec::new(),
            raw_provider_output_ref: None,
        })
        .expect("testing report");
    let gate = store
        .create_blocked_gate(CreateBlockedGateInput {
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::Testing,
            node_id: Some("coding_node_0001".to_string()),
            role: Some(CodingProviderRole::Tester),
            title: "Testing blocked".to_string(),
            description: "missing required step".to_string(),
            reason_code: Some("missing_required_steps".to_string()),
            evidence_refs: vec!["testing_report_0001.json".to_string()],
            raw_provider_output_ref: None,
            available_actions: testing_blocked_gate_actions(),
        })
        .expect("blocked gate");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    assert!(
        engine
            .handle_blocked_gate_response(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &gate.gate_id,
                "manual_continue",
                None,
            )
            .await
            .is_err()
    );

    let updated = engine
        .handle_blocked_gate_response(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate.gate_id,
            "manual_continue",
            Some("operator accepts residual risk".to_string()),
        )
        .await
        .expect("manual continue");
    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::Testing);

    let audits = store
        .list_quality_bypass_audits(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("audits");
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].gate_id, gate.gate_id);
    assert_eq!(audits[0].skipped_required_steps, vec!["unit"]);
    assert_eq!(audits[0].operator_context, "operator accepts residual risk");

    let updated = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt");
    let pack = build_evaluation_context_pack(paths, &updated, EvaluationContextRole::CodeReviewer)
        .expect("evaluation context");
    assert_eq!(pack.quality_bypass_audits.len(), 1);
    assert_eq!(
        pack.quality_bypass_audits[0].skipped_required_steps,
        vec!["unit"]
    );
}

#[tokio::test]
async fn continue_rework_after_limit_persists_instruction_without_quality_bypass() {
    let paths = ProductAppPaths::new(tempdir().expect("tempdir").path().join(".aria"));
    let store = CodingAttemptStore::new(paths);
    let attempt = store
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
            },
            max_auto_rework: 2,
        })
        .expect("create attempt");
    let mut attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("running");
    attempt = store
        .increment_attempt_rework_count(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("first rework");
    attempt = store
        .increment_attempt_rework_count(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("second rework");
    attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Rework,
        )
        .expect("rework stage");
    attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("blocked");
    store
        .save_analyst_decision(&AnalystDecisionRecord {
            id: "analyst_decision_0001".to_string(),
            attempt_id: attempt.id.clone(),
            source_stage: CodingExecutionStage::CodeReview,
            rework_round: 3,
            verdict: AnalystDecisionVerdict::NeedsFix,
            next_stage: AnalystDecisionNextStage::Coding,
            reason: "CodeReview 仍有阻塞问题".to_string(),
            evidence_refs: vec!["code_review_0001/findings[0]".to_string()],
            raw_provider_output_refs: vec![
                "provider-raw/code_review/code_review_0001.txt".to_string(),
            ],
            rework_instructions: Some(AnalystReworkInstructions {
                summary: "修复 provider install 契约".to_string(),
                required_changes: vec!["改为 202 installing".to_string()],
                verification_expectations: vec!["补并发安装测试".to_string()],
            }),
            human_gate: None,
            created_at: "2026-06-14T00:00:00Z".to_string(),
            parse_error: None,
            role_run_id: None,
            run_no: Some(1),
        })
        .expect("analyst decision");
    let gate = store
        .create_blocked_gate(CreateBlockedGateInput {
            attempt_id: attempt.id.clone(),
            stage: CodingExecutionStage::Rework,
            node_id: Some("coding_node_0001".to_string()),
            role: Some(CodingProviderRole::Analyst),
            title: "Rework limit reached".to_string(),
            description: "已达到自动重写上限".to_string(),
            reason_code: Some("max_auto_rework_exceeded".to_string()),
            evidence_refs: vec!["code_review_0001/findings[0]".to_string()],
            raw_provider_output_ref: Some(
                "provider-raw/code_review/code_review_0001.txt".to_string(),
            ),
            available_actions: vec![
                coding_gate_action_for_id("continue_rework").expect("continue rework action"),
                coding_gate_action_for_id("abort").expect("abort action"),
            ],
        })
        .expect("blocked gate");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let updated = engine
        .handle_blocked_gate_response(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate.gate_id,
            "continue_rework",
            Some("继续修 CodeReview findings".to_string()),
        )
        .await
        .expect("continue rework");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::Coding);
    assert_eq!(updated.rework_count, 3);
    assert!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates")
            .is_empty()
    );
    let instructions = store
        .list_rework_instructions(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("rework instructions");
    assert_eq!(instructions.len(), 1);
    assert_eq!(instructions[0].summary, "修复 provider install 契约");
    assert_eq!(
        instructions[0].fix_hints,
        vec!["改为 202 installing", "补并发安装测试"]
    );
    let notes = store
        .list_context_notes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("context notes");
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].content, "继续修 CodeReview findings");
    assert!(
        store
            .list_quality_bypass_audits(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("quality bypass audits")
            .is_empty()
    );
}
