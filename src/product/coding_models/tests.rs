use crate::product::coding_models::{
    CodeReviewReport, CodingAttemptStatus, CodingExecutionStage, CodingGateAction,
    CodingGateActionType, CodingGateKind, CodingGateRequired, CodingProviderRole, FindingSeverity,
    InternalPrReview, PushStatus, RemoteKind, ReviewFinding, ReviewRequest, ReviewRequestKind,
    ReviewRequestOwnerKind, ReviewVerdict,
};

#[test]
fn awaiting_manual_recovery_is_not_active() {
    assert!(!CodingAttemptStatus::AwaitingManualRecovery.is_active());
}

#[test]
fn retained_coding_stages_keep_their_relative_order_after_testing_removal() {
    let stages = [
        CodingExecutionStage::PrepareContext,
        CodingExecutionStage::WorktreePrepare,
        CodingExecutionStage::Coding,
        CodingExecutionStage::CodeReview,
        CodingExecutionStage::ReviewRequest,
        CodingExecutionStage::InternalPrReview,
        CodingExecutionStage::FinalConfirm,
    ];

    assert_eq!(stages.map(|stage| stage.order()), [0_u8, 1, 2, 3, 4, 5, 6]);
}

#[test]
fn review_artifacts_round_trip_preserves_evidence() {
    let review = CodeReviewReport {
        id: "code_review_0001".to_string(),
        attempt_id: "coding_attempt_0001".to_string(),
        round: 1,
        verdict: ReviewVerdict::RequestChanges,
        findings: vec![ReviewFinding {
            severity: FindingSeverity::Warning,
            file_path: Some("src/lib.rs".to_string()),
            line: Some(42),
            message: "missing validation".to_string(),
            required_action: Some("add validation".to_string()),
            source_stage: CodingExecutionStage::CodeReview,
            evidence: vec!["diff:src/lib.rs".to_string()],
            plan_defect_evidence: Vec::new(),
            related_requirements: vec!["REQ-1".to_string()],
            related_design_constraints: vec!["DES-1".to_string()],
            related_work_item_tasks: vec!["TASK-1".to_string()],
            defect_class: crate::product::models::PlanDefectClass::ImplementationDefect,
            reason_code: None,
            contract_refs: Vec::new(),
            capability_refs: Vec::new(),
            repair_target: None,
            recommended_route: crate::product::models::PlanDefectRoute::CoderRework,
            confidence: None,
        }],
        tested_evidence_refs: vec!["review-command.log".to_string()],
        diff_refs: vec!["attempt.diff".to_string()],
        summary: "needs validation".to_string(),
        created_at: "2026-06-10T00:00:02Z".to_string(),
        raw_provider_output_ref: Some("provider-raw/code_review/code_review_0001.txt".to_string()),
        role_run_id: None,
        run_no: None,
        unit_run_id: None,
    };
    let review_value = serde_json::to_value(&review).expect("serialize code review");
    assert_eq!(
        review_value["raw_provider_output_ref"],
        "provider-raw/code_review/code_review_0001.txt"
    );

    let gate = CodingGateRequired {
        gate_id: "coding_blocked_gate_0001".to_string(),
        kind: CodingGateKind::Blocked,
        title: "Review blocked".to_string(),
        description: "review payload could not be parsed".to_string(),
        stage: Some(CodingExecutionStage::CodeReview),
        role: Some(CodingProviderRole::CodeReviewer),
        expires_at: None,
        provider_snapshot: None,
        available_actions: vec![CodingGateAction {
            action_id: "retry_review".to_string(),
            label: "重试审查".to_string(),
            action_type: CodingGateActionType::RetryReview,
        }],
        reason_code: Some("review_payload_parse_error".to_string()),
        evidence_refs: vec!["code_review_0001.json".to_string()],
        raw_provider_output_ref: Some("provider-raw/code_review/code_review_0001.txt".to_string()),
        diagnostic: None,
    };
    let gate_value = serde_json::to_value(&gate).expect("serialize gate");
    assert_eq!(gate_value["reason_code"], "review_payload_parse_error");
    assert_eq!(gate_value["evidence_refs"][0], "code_review_0001.json");
    assert_eq!(
        gate_value["raw_provider_output_ref"],
        "provider-raw/code_review/code_review_0001.txt"
    );
}

#[test]
fn legacy_coding_qa_records_deserialize_with_defaults() {
    let legacy_code_review = r#"{
      "id": "code_review_0001",
      "attempt_id": "coding_attempt_0001",
      "round": 1,
      "verdict": "request_changes",
      "findings": [
        {
          "severity": "warning",
          "file_path": "src/lib.rs",
          "line": 42,
          "message": "missing validation",
          "required_action": "add validation"
        }
      ],
      "tested_evidence_refs": [],
      "diff_refs": [],
      "summary": "needs validation",
      "created_at": "2026-06-10T00:00:02Z"
    }"#;
    let review: CodeReviewReport = serde_json::from_str(legacy_code_review).unwrap();
    assert_eq!(review.raw_provider_output_ref, None);
    assert_eq!(
        review.findings[0].source_stage,
        CodingExecutionStage::CodeReview
    );
    assert!(review.findings[0].evidence.is_empty());
    assert!(review.findings[0].related_requirements.is_empty());
    assert!(review.findings[0].related_design_constraints.is_empty());
    assert!(review.findings[0].related_work_item_tasks.is_empty());

    let legacy_internal_review = r#"{
      "id": "internal_pr_review_0001",
      "attempt_id": "coding_attempt_0001",
      "review_request_id": "review_request_0001",
      "verdict": "approve",
      "findings": [],
      "impact_scope": [],
      "pr_description": "ready",
      "commit_message_suggestion": "feat: ready",
      "tested_evidence_refs": [],
      "diff_refs": [],
      "summary": "ready",
      "created_at": "2026-06-10T00:00:03Z"
    }"#;
    let internal_review: InternalPrReview = serde_json::from_str(legacy_internal_review).unwrap();
    assert_eq!(internal_review.raw_provider_output_ref, None);

    let legacy_gate = r#"{
      "gate_id": "coding_gate_0001",
      "kind": "stage_gate",
      "title": "Confirm",
      "description": "confirm stage",
      "stage": "code_review",
      "role": "code_reviewer",
      "expires_at": null,
      "provider_snapshot": null,
      "available_actions": []
    }"#;
    let gate: CodingGateRequired = serde_json::from_str(legacy_gate).unwrap();
    assert_eq!(gate.reason_code, None);
    assert!(gate.evidence_refs.is_empty());
    assert_eq!(gate.raw_provider_output_ref, None);
}

#[test]
fn review_request_deserializes_legacy_json_as_attempt_owner() {
    let legacy = serde_json::json!({
        "id": "rr_1", "attempt_id": "attempt_1", "kind": "git_branch_only",
        "remote_kind": "generic_git", "remote": "origin",
        "base_branch": "main", "branch_name": "aria/attempt_1",
        "commit_sha": "abc", "push_status": "pushed",
        "external_url": null, "manual_instructions": [],
        "created_at": "2026-08-14T00:00:00Z", "updated_at": "2026-08-14T00:00:00Z"
    });
    let req: ReviewRequest = serde_json::from_value(legacy).unwrap();
    assert_eq!(req.owner_kind, ReviewRequestOwnerKind::Attempt);
    assert_eq!(req.pointer_publication_id, None);
}

#[test]
fn review_request_roundtrips_pointer_publication_owner() {
    let req = ReviewRequest {
        id: "rr_2".to_string(),
        attempt_id: "pointer-pub-pub_1".to_string(),
        kind: ReviewRequestKind::GitBranchOnly,
        remote_kind: RemoteKind::GenericGit,
        remote: "origin".to_string(),
        base_branch: "main".to_string(),
        branch_name: "aria/pointer_pub_1".to_string(),
        commit_sha: "abc".to_string(),
        push_status: PushStatus::Pushed,
        external_url: None,
        manual_instructions: Vec::new(),
        push_error: None,
        owner_kind: ReviewRequestOwnerKind::PointerPublication,
        pointer_publication_id: Some("pub_1".to_string()),
        created_at: "2026-08-14T00:00:00Z".to_string(),
        updated_at: "2026-08-14T00:00:00Z".to_string(),
    };
    let value = serde_json::to_value(&req).unwrap();
    assert_eq!(value["owner_kind"], "pointer_publication");
    assert_eq!(value["pointer_publication_id"], "pub_1");

    let decoded: ReviewRequest = serde_json::from_value(value).unwrap();
    assert_eq!(
        decoded.owner_kind,
        ReviewRequestOwnerKind::PointerPublication
    );
    assert_eq!(decoded.pointer_publication_id, Some("pub_1".to_string()));
    assert_eq!(decoded.attempt_id, "pointer-pub-pub_1");
}
