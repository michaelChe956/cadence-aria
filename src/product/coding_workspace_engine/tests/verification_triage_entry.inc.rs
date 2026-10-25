/// REQ-CVT-03：转入「验证处理」前后门动作集合逐项相同（含是否含
/// `send_to_coder`）、门仍 open、finding 不变——本用例用 `approve` 派生的三项门，
/// 转入不得把被派生掉的 `send_to_coder` 加回来。
#[tokio::test]
async fn verification_triage_entry_keeps_verdict_derived_actions_unchanged() {
    let (_root, store, engine, attempt) = verification_triage_group_fixture();
    store
        .save_code_review_report(
            &attempt,
            &CodeReviewReport {
                id: "code_review_report_0001".to_string(),
                attempt_id: attempt.id.clone(),
                round: 1,
                verdict: ReviewVerdict::Approve,
                findings: vec![ReviewFinding {
                    severity: FindingSeverity::Error,
                    file_path: Some("src/lib.rs".to_string()),
                    line: Some(42),
                    message: "missing validation".to_string(),
                    required_action: Some("add validation".to_string()),
                    source_stage: CodingExecutionStage::CodeReview,
                    evidence: Vec::new(),
                    plan_defect_evidence: Vec::new(),
                    related_requirements: Vec::new(),
                    related_design_constraints: Vec::new(),
                    related_work_item_tasks: Vec::new(),
                    defect_class: crate::product::models::PlanDefectClass::ImplementationDefect,
                    reason_code: None,
                    contract_refs: Vec::new(),
                    capability_refs: Vec::new(),
                    repair_target: None,
                    recommended_route: crate::product::models::PlanDefectRoute::CoderRework,
                    confidence: None,
                }],
                tested_evidence_refs: Vec::new(),
                diff_refs: Vec::new(),
                summary: "reviewer approved with advisory finding".to_string(),
                created_at: "2026-07-01T00:00:00Z".to_string(),
                raw_provider_output_ref: None,
                role_run_id: None,
                run_no: None,
                unit_run_id: None,
                acceptance: None,
                review_window_id: None,
            },
        )
        .expect("code review report");
    let gated = engine
        .create_review_blocked_gate(ReviewBlockedGateInput {
            attempt: &attempt,
            node_id: "coding_node_0001",
            stage: CodingExecutionStage::CodeReview,
            role: CodingProviderRole::CodeReviewer,
            title: "Code Review 验证证据不完整".to_string(),
            description: "approve 派生的三项门".to_string(),
            reason_code: "code_review_verification_incomplete",
            evidence_refs: vec!["code_review_report_0001".to_string()],
            raw_provider_output_ref: None,
        })
        .await
        .expect("triage gate");
    let before = open_triage_gate(&store, &gated);
    assert_eq!(
        gate_action_ids(&before),
        vec!["retry_review", "manual_continue", "abort"],
    );

    engine
        .enter_verification_triage(
            &gated.project_id,
            &gated.issue_id,
            &gated.id,
            equivalent_evidence_entry(TRIAGE_CHECK_PLAIN, Some(3)),
        )
        .await
        .expect("enter verification triage");

    let after = open_triage_gate(&store, &gated);
    assert_eq!(after.gate_id, before.gate_id, "转入不得换门");
    assert_eq!(
        gate_action_ids(&after),
        gate_action_ids(&before),
        "转入前后动作集合必须逐项相同（含是否含 send_to_coder）",
    );
    assert_eq!(after.latest_review_verdict, before.latest_review_verdict);
    let reports = store
        .list_code_review_reports(&gated.project_id, &gated.issue_id, &gated.id)
        .expect("reports");
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].findings.len(), 1);
    assert_eq!(reports[0].findings[0].message, "missing validation");
}
