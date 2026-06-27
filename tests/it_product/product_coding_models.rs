use std::path::PathBuf;

use cadence_aria::product::coding_models::{
    AnalystDecisionNextStage, AnalystDecisionRecord, AnalystDecisionVerdict,
    AnalystHumanGateRecommendation, AnalystReworkInstructions, AnalystVerdict, CodeReviewReport,
    CodingAgentRole, CodingAttemptStatus, CodingChatEntry, CodingContextNote, CodingEntryType,
    CodingExecutionAttempt, CodingExecutionStage, CodingGateAction, CodingGateActionType,
    CodingGateKind, CodingGateRequired, CodingProviderRole, CodingRolePermissionModes,
    CodingRoleProviderConfigSnapshot, CodingStageGateState, CodingStageGateStatus,
    CodingTimelineNode, CodingTimelineNodeStatus, FindingSeverity, InternalPrReview, PushStatus,
    RemoteKind, ReviewFinding, ReviewRequest, ReviewRequestKind, ReviewVerdict, TestCommand,
    TestCommandStatus, TestingOverallStatus, TestingReport,
};
use cadence_aria::product::models::ProviderName;
use cadence_aria::web::workspace_ws_types::ProviderConfigSnapshot;
use serde_json::json;

#[test]
fn analyst_decision_record_uses_stable_wire_values() {
    let record = AnalystDecisionRecord {
        id: "analyst_decision_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        source_stage: CodingExecutionStage::Testing,
        rework_round: 1,
        verdict: AnalystDecisionVerdict::NeedsFix,
        next_stage: AnalystDecisionNextStage::Coding,
        reason: "required 测试步骤被跳过".to_string(),
        evidence_refs: vec!["testing_report_0001.json".to_string()],
        raw_provider_output_refs: vec![
            "provider-raw/testing/execute_test_plan_0001.txt".to_string(),
        ],
        rework_instructions: Some(AnalystReworkInstructions {
            summary: "补齐 required 测试覆盖".to_string(),
            required_changes: vec!["补充 B6 浏览器测试".to_string()],
            verification_expectations: vec!["B6 不再出现在 skipped_required_steps".to_string()],
        }),
        human_gate: Some(AnalystHumanGateRecommendation {
            reason_code: Some("external_browser_required".to_string()),
            available_actions: vec!["provide_context".to_string(), "manual_continue".to_string()],
        }),
        created_at: "2026-06-12T00:00:00Z".to_string(),
        parse_error: None,
        role_run_id: None,
        run_no: None,
    };

    let value = serde_json::to_value(&record).expect("serialize decision");
    assert_eq!(
        value,
        json!({
            "id": "analyst_decision_0001",
            "attempt_id": "coding_attempt_0001",
            "source_stage": "testing",
            "rework_round": 1,
            "verdict": "needs_fix",
            "next_stage": "coding",
            "reason": "required 测试步骤被跳过",
            "evidence_refs": ["testing_report_0001.json"],
            "raw_provider_output_refs": ["provider-raw/testing/execute_test_plan_0001.txt"],
            "rework_instructions": {
                "summary": "补齐 required 测试覆盖",
                "required_changes": ["补充 B6 浏览器测试"],
                "verification_expectations": ["B6 不再出现在 skipped_required_steps"]
            },
            "human_gate": {
                "reason_code": "external_browser_required",
                "available_actions": ["provide_context", "manual_continue"]
            },
            "created_at": "2026-06-12T00:00:00Z",
            "parse_error": null,
            "role_run_id": null,
            "run_no": null
        })
    );

    let parsed: AnalystDecisionRecord =
        serde_json::from_value(value).expect("deserialize decision");
    assert_eq!(parsed, record);
}

#[test]
fn coding_provider_roles_use_stable_wire_values_and_display_names() {
    assert_eq!(
        serde_json::to_value(CodingProviderRole::Coder).expect("serialize coder"),
        json!("coder")
    );
    assert_eq!(
        serde_json::to_value(CodingProviderRole::Tester).expect("serialize tester"),
        json!("tester")
    );
    assert_eq!(
        serde_json::to_value(CodingProviderRole::Analyst).expect("serialize analyst"),
        json!("analyst")
    );
    assert_eq!(
        serde_json::to_value(CodingProviderRole::CodeReviewer).expect("serialize code reviewer"),
        json!("code_reviewer")
    );
    assert_eq!(
        serde_json::to_value(CodingProviderRole::InternalReviewer)
            .expect("serialize internal reviewer"),
        json!("internal_reviewer")
    );

    assert_eq!(CodingProviderRole::Coder.to_string(), "Coder");
    assert_eq!(CodingProviderRole::Tester.to_string(), "Tester");
    assert_eq!(CodingProviderRole::Analyst.to_string(), "Analyst");
    assert_eq!(
        CodingProviderRole::CodeReviewer.to_string(),
        "Code Reviewer"
    );
    assert_eq!(
        CodingProviderRole::InternalReviewer.to_string(),
        "Internal Reviewer"
    );
}

#[test]
fn coding_role_provider_config_snapshot_derives_from_legacy_provider_snapshot() {
    let snapshot = CodingRoleProviderConfigSnapshot::from(ProviderConfigSnapshot {
        author: ProviderName::Codex,
        reviewer: Some(ProviderName::Fake),
        review_rounds: 2,
    });

    assert_eq!(snapshot.coder, ProviderName::Codex);
    assert_eq!(snapshot.tester, ProviderName::Codex);
    assert_eq!(snapshot.analyst, ProviderName::Codex);
    assert_eq!(snapshot.code_reviewer, ProviderName::Fake);
    assert_eq!(snapshot.internal_reviewer, ProviderName::Fake);
    assert_eq!(snapshot.review_rounds, 2);

    let value = serde_json::to_value(snapshot).expect("serialize role provider snapshot");
    assert_eq!(
        value,
        json!({
            "coder": "codex",
            "tester": "codex",
            "analyst": "codex",
            "code_reviewer": "fake",
            "internal_reviewer": "fake",
            "review_rounds": 2,
            "permission_modes": {
                "coder": "supervised",
                "tester": "auto",
                "analyst": "auto",
                "code_reviewer": "supervised",
                "internal_reviewer": "supervised"
            }
        })
    );
}

#[test]
fn coding_role_provider_config_snapshot_falls_back_to_author_when_reviewer_is_missing() {
    let snapshot = CodingRoleProviderConfigSnapshot::from(ProviderConfigSnapshot {
        author: ProviderName::ClaudeCode,
        reviewer: None,
        review_rounds: 1,
    });

    assert_eq!(snapshot.coder, ProviderName::ClaudeCode);
    assert_eq!(snapshot.tester, ProviderName::ClaudeCode);
    assert_eq!(snapshot.analyst, ProviderName::ClaudeCode);
    assert_eq!(snapshot.code_reviewer, ProviderName::ClaudeCode);
    assert_eq!(snapshot.internal_reviewer, ProviderName::ClaudeCode);
}

#[test]
fn coding_chat_entries_context_notes_and_analyst_verdicts_have_stable_json_shape() {
    let entry = CodingChatEntry {
        id: "coding_chat_entry_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        node_id: Some("coding_node_0001".to_string()),
        role: CodingAgentRole::System,
        entry_type: CodingEntryType::AnalystVerdict {
            verdict: AnalystVerdict::NeedsHumanInput,
        },
        content: Some("需要用户补充仓库路径".to_string()),
        metadata: Some(json!({"source": "analyst"})),
        created_at: "2026-05-28T00:00:00Z".to_string(),
    };
    let note = CodingContextNote {
        id: "coding_context_note_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        content: "请优先使用 unittest".to_string(),
        created_at: "2026-05-28T00:01:00Z".to_string(),
        consumed_by_rework_round: None,
    };

    let entry_value = serde_json::to_value(&entry).expect("serialize chat entry");
    assert_eq!(entry_value["entry_type"]["type"], "analyst_verdict");
    assert_eq!(entry_value["entry_type"]["verdict"], "needs_human_input");
    assert_eq!(entry_value["node_id"], "coding_node_0001");
    assert_eq!(entry_value["role"], "system");

    let decoded_entry: CodingChatEntry =
        serde_json::from_value(entry_value).expect("deserialize chat entry");
    assert_eq!(decoded_entry, entry);

    let note_value = serde_json::to_value(&note).expect("serialize context note");
    assert_eq!(
        note_value["consumed_by_rework_round"],
        serde_json::Value::Null
    );
    assert_eq!(
        serde_json::from_value::<CodingContextNote>(note_value).unwrap(),
        note
    );
}

#[test]
fn coding_stage_gate_state_serializes_open_gate_contract() {
    let gate = CodingStageGateState {
        gate_id: "coding_stage_gate_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        stage: CodingExecutionStage::Testing,
        role: CodingProviderRole::Tester,
        expires_at: "2026-05-28T00:00:05Z".to_string(),
        provider_snapshot: CodingRoleProviderConfigSnapshot {
            coder: ProviderName::Codex,
            tester: ProviderName::Fake,
            analyst: ProviderName::Codex,
            code_reviewer: ProviderName::Fake,
            internal_reviewer: ProviderName::Fake,
            review_rounds: 1,
            permission_modes: CodingRolePermissionModes::default(),
        },
        status: CodingStageGateStatus::Open,
        created_at: "2026-05-28T00:00:00Z".to_string(),
        updated_at: "2026-05-28T00:00:00Z".to_string(),
    };

    let value = serde_json::to_value(&gate).expect("serialize stage gate");

    assert_eq!(value["status"], "open");
    assert_eq!(value["stage"], "testing");
    assert_eq!(value["role"], "tester");
    assert_eq!(value["provider_snapshot"]["tester"], "fake");
    assert_eq!(
        serde_json::from_value::<CodingStageGateState>(value).expect("deserialize stage gate"),
        gate
    );
}

#[test]
fn coding_attempt_serializes_stage_status_and_provider_snapshot() {
    let attempt = CodingExecutionAttempt {
        id: "coding_attempt_0001".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        work_item_id: "work_item_0001".to_string(),
        attempt_no: 1,
        scope: cadence_aria::product::coding_models::CodingAttemptScope::WorkItem,
        status: CodingAttemptStatus::Created,
        stage: CodingExecutionStage::PrepareContext,
        base_branch: "main".to_string(),
        branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
        worktree_path: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::Fake,
            reviewer: Some(ProviderName::Codex),
            review_rounds: 1,
        },
        rework_count: 0,
        max_auto_rework: 2,
        work_item_group_id: None,
        current_work_item_id: Some("work_item_0001".to_string()),
        active_unit_id: None,
        head_commit: None,
        pushed_remote: None,
        review_request_id: None,
        provider_conversations: Vec::new(),
        created_at: "2026-05-23T00:00:00Z".to_string(),
        updated_at: "2026-05-23T00:00:00Z".to_string(),
        completed_at: None,
    };

    let value = serde_json::to_value(&attempt).expect("serialize attempt");

    assert_eq!(value["status"], "created");
    assert_eq!(value["stage"], "prepare_context");
    assert_eq!(value["provider_config_snapshot"]["author"], "fake");

    let decoded: CodingExecutionAttempt =
        serde_json::from_value(value).expect("deserialize attempt");
    assert_eq!(decoded.status, CodingAttemptStatus::Created);
    assert_eq!(decoded.stage, CodingExecutionStage::PrepareContext);
}

#[test]
fn testing_and_review_reports_preserve_backend_evidence() {
    let command = TestCommand {
        command: vec!["cargo".to_string(), "test".to_string()],
        cwd: PathBuf::from("/tmp/worktree"),
        exit_code: Some(0),
        duration_ms: 1234,
        stdout_ref: "artifacts/stdout.txt".to_string(),
        stderr_ref: "artifacts/stderr.txt".to_string(),
        status: TestCommandStatus::Passed,
    };
    let testing = TestingReport {
        id: "testing_report_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        role_run_id: None,
        run_no: None,
        commands: vec![command],
        overall_status: TestingOverallStatus::Passed,
        provider_claim: Some(json!({"claimed": true})),
        backend_verified: true,
        started_at: "2026-05-23T00:01:00Z".to_string(),
        completed_at: Some("2026-05-23T00:02:00Z".to_string()),
        plan_id: None,
        plan_summary: None,
        steps: Vec::new(),
        unplanned_commands: Vec::new(),
        unplanned_evidence: Vec::new(),
        missing_required_steps: Vec::new(),
        skipped_required_steps: Vec::new(),
        context_warnings: Vec::new(),
        raw_provider_output_ref: None,
    };
    let finding = ReviewFinding {
        severity: FindingSeverity::Warning,
        file_path: Some("src/lib.rs".to_string()),
        line: Some(42),
        message: "需要补充边界测试".to_string(),
        required_action: Some("添加 n=0 用例".to_string()),
        source_stage: CodingExecutionStage::CodeReview,
        evidence: Vec::new(),
        related_requirements: Vec::new(),
        related_design_constraints: Vec::new(),
        related_work_item_tasks: Vec::new(),
    };
    let code_review = CodeReviewReport {
        id: "code_review_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        round: 1,
        verdict: ReviewVerdict::RequestChanges,
        findings: vec![finding.clone()],
        tested_evidence_refs: vec!["testing_report_0001".to_string()],
        diff_refs: vec!["diff_0001".to_string()],
        summary: "需要返工".to_string(),
        created_at: "2026-05-23T00:03:00Z".to_string(),
        raw_provider_output_ref: None,
        role_run_id: None,
        run_no: None,
    };
    let internal = InternalPrReview {
        id: "internal_review_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        review_request_id: "review_request_0001".to_string(),
        verdict: ReviewVerdict::Approve,
        findings: vec![finding],
        impact_scope: vec!["src/lib.rs".to_string()],
        pr_description: "实现 work item".to_string(),
        commit_message_suggestion: "feat: implement work item".to_string(),
        tested_evidence_refs: vec!["testing_report_0001".to_string()],
        diff_refs: vec!["diff_0001".to_string()],
        summary: "可以合入".to_string(),
        created_at: "2026-05-23T00:04:00Z".to_string(),
        raw_provider_output_ref: None,
        role_run_id: None,
        run_no: None,
    };

    assert_eq!(
        serde_json::to_value(&testing).unwrap()["backend_verified"],
        true
    );
    assert_eq!(
        serde_json::to_value(&code_review).unwrap()["verdict"],
        "request_changes"
    );
    assert_eq!(
        serde_json::to_value(&internal).unwrap()["verdict"],
        "approve"
    );
}

#[test]
fn review_finding_deserializes_provider_severity_aliases() {
    let json = r#"{"severity":"medium","file_path":"src/lib.rs","line":1,"message":"fix","required_action":"change","source_stage":"code_review"}"#;

    let finding: ReviewFinding = serde_json::from_str(json).expect("finding should parse");

    assert_eq!(finding.severity, FindingSeverity::Warning);
}

#[test]
fn review_request_timeline_and_gate_actions_use_stable_wire_values() {
    let review_request = ReviewRequest {
        id: "review_request_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        kind: ReviewRequestKind::GitBranchOnly,
        remote_kind: RemoteKind::GenericGit,
        remote: "origin".to_string(),
        base_branch: "main".to_string(),
        branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
        commit_sha: "abc123".to_string(),
        push_status: PushStatus::Pushed,
        external_url: None,
        manual_instructions: vec!["手动打开 review branch".to_string()],
        created_at: "2026-05-23T00:05:00Z".to_string(),
        updated_at: "2026-05-23T00:05:00Z".to_string(),
    };
    let node = CodingTimelineNode {
        id: "coding_node_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        stage: CodingExecutionStage::ReviewRequest,
        title: "创建 Review Request".to_string(),
        status: CodingTimelineNodeStatus::Running,
        agent_role: Some(CodingAgentRole::Git),
        summary: None,
        started_at: "2026-05-23T00:05:00Z".to_string(),
        completed_at: None,
        artifact_refs: vec!["review_request_0001".to_string()],
    };
    let gate = CodingGateRequired {
        gate_id: "gate_0001".to_string(),
        kind: CodingGateKind::Blocked,
        title: "Push 失败".to_string(),
        description: "需要用户选择下一步".to_string(),
        stage: None,
        role: None,
        expires_at: None,
        provider_snapshot: None,
        available_actions: vec![CodingGateAction {
            action_id: "retry".to_string(),
            label: "重试 Push".to_string(),
            action_type: CodingGateActionType::RetryPush,
        }],
        reason_code: None,
        evidence_refs: Vec::new(),
        raw_provider_output_ref: None,
    };

    assert_eq!(
        serde_json::to_value(&review_request).unwrap()["kind"],
        "git_branch_only"
    );
    assert_eq!(serde_json::to_value(&node).unwrap()["agent_role"], "git");
    assert_eq!(
        serde_json::to_value(&gate).unwrap()["available_actions"][0]["action_type"],
        "retry_push"
    );
}

#[test]
fn coding_gate_action_type_round_trips_retry_analyst() {
    let action = CodingGateAction {
        action_id: "retry_analyst".to_string(),
        label: "重试 Analyst".to_string(),
        action_type: CodingGateActionType::RetryAnalyst,
    };

    let value = serde_json::to_value(&action).expect("serialize action");
    assert_eq!(value["action_type"], "retry_analyst");
    let decoded: CodingGateAction = serde_json::from_value(value).expect("decode action");
    assert_eq!(decoded.action_type, CodingGateActionType::RetryAnalyst);
}

#[test]
fn analyst_decision_round_trips_role_run_metadata() {
    let decision = AnalystDecisionRecord {
        id: "analyst_decision_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        source_stage: CodingExecutionStage::Testing,
        rework_round: 1,
        verdict: AnalystDecisionVerdict::HumanRequired,
        next_stage: AnalystDecisionNextStage::HumanGate,
        reason: "Analyst 输出不是有效 JSON".to_string(),
        evidence_refs: vec!["testing_report_0001.json".to_string()],
        raw_provider_output_refs: vec!["provider-raw/rework/analyst_decision_0001.txt".to_string()],
        rework_instructions: None,
        human_gate: None,
        created_at: "2026-06-13T00:00:00Z".to_string(),
        parse_error: Some("expected JSON".to_string()),
        role_run_id: Some("coding_role_run_0001".to_string()),
        run_no: Some(1),
    };

    let value = serde_json::to_value(&decision).expect("serialize decision");
    assert_eq!(value["role_run_id"], "coding_role_run_0001");
    assert_eq!(value["run_no"], 1);
    let decoded: AnalystDecisionRecord = serde_json::from_value(value).expect("decode decision");
    assert_eq!(decoded.role_run_id.as_deref(), Some("coding_role_run_0001"));
    assert_eq!(decoded.run_no, Some(1));
}

#[test]
fn review_reports_round_trip_role_run_metadata() {
    let code_review = CodeReviewReport {
        id: "code_review_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        round: 1,
        verdict: ReviewVerdict::Approve,
        findings: Vec::new(),
        tested_evidence_refs: Vec::new(),
        diff_refs: Vec::new(),
        summary: "review ok".to_string(),
        created_at: "2026-06-13T00:00:00Z".to_string(),
        raw_provider_output_ref: Some("provider-raw/code_review/code_review_0001.txt".to_string()),
        role_run_id: Some("coding_role_run_0001".to_string()),
        run_no: Some(1),
    };
    let value = serde_json::to_value(&code_review).expect("serialize code review");
    assert_eq!(value["role_run_id"], "coding_role_run_0001");
    let decoded: CodeReviewReport = serde_json::from_value(value).expect("decode code review");
    assert_eq!(decoded.role_run_id.as_deref(), Some("coding_role_run_0001"));
    assert_eq!(decoded.run_no, Some(1));

    let internal_review = InternalPrReview {
        id: "internal_review_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        review_request_id: "review_request_0001".to_string(),
        verdict: ReviewVerdict::Approve,
        findings: Vec::new(),
        impact_scope: vec!["src/lib.rs".to_string()],
        pr_description: "PR".to_string(),
        commit_message_suggestion: "feat: work".to_string(),
        tested_evidence_refs: Vec::new(),
        diff_refs: Vec::new(),
        summary: "internal ok".to_string(),
        created_at: "2026-06-13T00:00:01Z".to_string(),
        raw_provider_output_ref: Some(
            "provider-raw/internal_pr_review/internal_pr_review_0001.txt".to_string(),
        ),
        role_run_id: Some("coding_role_run_0002".to_string()),
        run_no: Some(1),
    };
    let value = serde_json::to_value(&internal_review).expect("serialize internal review");
    assert_eq!(value["role_run_id"], "coding_role_run_0002");
    let decoded: InternalPrReview = serde_json::from_value(value).expect("decode internal review");
    assert_eq!(decoded.role_run_id.as_deref(), Some("coding_role_run_0002"));
    assert_eq!(decoded.run_no, Some(1));
}
