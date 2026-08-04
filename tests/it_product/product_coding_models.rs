use cadence_aria::product::coding_models::{
    CodeReviewReport, CodingAgentRole, CodingAttemptStatus, CodingChatEntry, CodingContextNote,
    CodingEntryType, CodingExecutionAttempt, CodingExecutionStage, CodingGateAction,
    CodingGateActionType, CodingGateKind, CodingGateRequired, CodingProviderRole,
    CodingRolePermissionModes, CodingRoleProviderConfigSnapshot, CodingStageGateState,
    CodingStageGateStatus, CodingTimelineNode, CodingTimelineNodeStatus, FindingSeverity,
    InternalPrReview, PushStatus, RemoteKind, ReviewFinding, ReviewRequest, ReviewRequestKind,
    ReviewVerdict,
};
use cadence_aria::product::models::ProviderName;
use cadence_aria::web::workspace_ws_types::ProviderConfigSnapshot;
use serde_json::json;

#[test]
fn coding_provider_roles_use_stable_wire_values_and_display_names() {
    assert_eq!(
        serde_json::to_value(CodingProviderRole::Coder).expect("serialize coder"),
        json!("coder")
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
        permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
    });

    assert_eq!(snapshot.coder, ProviderName::Codex);
    assert_eq!(snapshot.code_reviewer, ProviderName::Fake);
    assert_eq!(snapshot.internal_reviewer, ProviderName::Fake);
    assert_eq!(snapshot.review_rounds, 2);

    let value = serde_json::to_value(snapshot).expect("serialize role provider snapshot");
    assert_eq!(
        value,
        json!({
            "coder": "codex",
            "code_reviewer": "fake",
            "internal_reviewer": "fake",
            "review_rounds": 2,
            "permission_modes": {
                "coder": "auto",
                "code_reviewer": "auto",
                "internal_reviewer": "auto"
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
        permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
    });

    assert_eq!(snapshot.coder, ProviderName::ClaudeCode);
    assert_eq!(snapshot.code_reviewer, ProviderName::ClaudeCode);
    assert_eq!(snapshot.internal_reviewer, ProviderName::ClaudeCode);
}

#[test]
fn coding_chat_entries_context_notes_and_stage_summaries_have_stable_json_shape() {
    let entry = CodingChatEntry {
        id: "coding_chat_entry_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        node_id: Some("coding_node_0001".to_string()),
        role: CodingAgentRole::System,
        entry_type: CodingEntryType::StageSummary {
            stage: CodingExecutionStage::CodeReview,
            summary: "Code Reviewer 等待人工处理".to_string(),
        },
        content: Some("Code Reviewer 等待人工处理".to_string()),
        metadata: Some(json!({"source": "code_review"})),
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
    assert_eq!(entry_value["entry_type"]["type"], "stage_summary");
    assert_eq!(entry_value["entry_type"]["stage"], "code_review");
    assert_eq!(
        entry_value["entry_type"]["summary"],
        "Code Reviewer 等待人工处理"
    );
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
        stage: CodingExecutionStage::CodeReview,
        role: CodingProviderRole::CodeReviewer,
        expires_at: "2026-05-28T00:00:05Z".to_string(),
        provider_snapshot: CodingRoleProviderConfigSnapshot {
            coder: ProviderName::Codex,
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
    assert_eq!(value["stage"], "code_review");
    assert_eq!(value["role"], "code_reviewer");
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
            permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(
            ),
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
fn review_reports_preserve_backend_evidence() {
    let finding = ReviewFinding {
        severity: FindingSeverity::Warning,
        file_path: Some("src/lib.rs".to_string()),
        line: Some(42),
        message: "需要补充边界测试".to_string(),
        required_action: Some("添加 n=0 用例".to_string()),
        source_stage: CodingExecutionStage::CodeReview,
        evidence: Vec::new(),
        plan_defect_evidence: Vec::new(),
        related_requirements: Vec::new(),
        related_design_constraints: Vec::new(),
        related_work_item_tasks: Vec::new(),
        defect_class: cadence_aria::product::models::PlanDefectClass::ImplementationDefect,
        reason_code: None,
        contract_refs: Vec::new(),
        capability_refs: Vec::new(),
        repair_target: None,
        recommended_route: cadence_aria::product::models::PlanDefectRoute::CoderRework,
        confidence: None,
    };
    let code_review = CodeReviewReport {
        id: "code_review_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        round: 1,
        verdict: ReviewVerdict::RequestChanges,
        findings: vec![finding.clone()],
        tested_evidence_refs: vec!["verification_log_0001".to_string()],
        diff_refs: vec!["diff_0001".to_string()],
        summary: "需要返工".to_string(),
        created_at: "2026-05-23T00:03:00Z".to_string(),
        raw_provider_output_ref: None,
        role_run_id: None,
        run_no: None,
        unit_run_id: None,
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
        tested_evidence_refs: vec!["verification_log_0001".to_string()],
        diff_refs: vec!["diff_0001".to_string()],
        summary: "可以合入".to_string(),
        created_at: "2026-05-23T00:04:00Z".to_string(),
        raw_provider_output_ref: None,
        role_run_id: None,
        run_no: None,
    };

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
        push_error: None,
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
fn coding_gate_action_type_round_trips_send_to_coder() {
    let action = CodingGateAction {
        action_id: "send_to_coder".to_string(),
        label: "提交给 Coder 修复".to_string(),
        action_type: CodingGateActionType::SendToCoder,
    };

    let value = serde_json::to_value(&action).expect("serialize action");
    assert_eq!(value["action_type"], "send_to_coder");
    let decoded: CodingGateAction = serde_json::from_value(value).expect("decode action");
    assert_eq!(decoded.action_type, CodingGateActionType::SendToCoder);
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
        unit_run_id: None,
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
