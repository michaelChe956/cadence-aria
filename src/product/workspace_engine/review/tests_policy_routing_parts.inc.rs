// B3 · SC 路由收口的测试辅助（从 tests_policy_routing.rs 整块搬移，逐字未改；
// 主模块已近行数上限，按职责搬移不压行）。
fn sc_routing_engine() -> (TempDir, LifecycleStore, WorkspaceEngine) {
    let (tmp, lifecycle, _plan_id, mut engine) =
        make_work_item_plan_engine_with_accepted_contract_drafts();
    single_candidate::single_candidate_record(
        &lifecycle,
        &mut engine,
        crate::product::models::SingleCandidatePhase::Evaluate,
        RunPolicy::Interactive,
    );
    (tmp, lifecycle, engine)
}

fn sc_routing_finding(
    severity: ReviewFindingSeverity,
    message: &str,
    category: Option<ReviewFindingCategory>,
    class_hint: Option<crate::product::work_item_plan_policy::FindingClassHint>,
) -> ReviewFinding {
    ReviewFinding {
        severity,
        message: message.to_string(),
        evidence: "SC review evidence".to_string(),
        required_action: "SC review action".to_string(),
        category,
        class_hint,
        contract_field: None,
        machine_repair: None,
        owner_question: None,
        owner_options: None,
        disposition: None,
        target_work_item_id: None,
        finding_key: None,
    }
}

/// 命中窄例外全部条件（Suggestion + 显式非契约 category + Repairable hint）；
/// contract_field 带稳定 ID ⇒ 指纹落 F-52 稳定域，才会真实进入重复闸门的
/// 非 advisory 集合（纯措辞域 finding 本就被 unstable 过滤，构不成钉子）。
fn sc_demotable_suggestion(message: &str) -> ReviewFinding {
    let mut finding = sc_routing_finding(
        ReviewFindingSeverity::Suggestion,
        message,
        Some(ReviewFindingCategory::Completeness),
        Some(crate::product::work_item_plan_policy::FindingClassHint::Repairable),
    );
    finding.contract_field = Some("WI-001.acceptance_criteria[AC-002].rows".to_string());
    finding
}

fn sc_out_of_scope_finding() -> ReviewFinding {
    sc_routing_finding(
        ReviewFindingSeverity::MustFix,
        "范围外事实：文档缺登记行",
        None,
        None,
    )
}

fn sc_revise_verdict(findings: Vec<ReviewFinding>) -> ReviewVerdict {
    ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "SC review comments".to_string(),
        summary: "SC review summary".to_string(),
        findings,
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    }
}

/// 真实 SC 会话 verdict 恒带 work_item_plan_review extension（Item scope）。
fn sc_extension_revise_verdict(findings: Vec<ReviewFinding>) -> ReviewVerdict {
    let mut verdict = sc_revise_verdict(findings);
    verdict.work_item_plan_review = Some(WorkItemPlanReviewComplete {
        verdict: WorkItemPlanReviewVerdict::Revise,
        review_scope: WorkItemPlanReviewScope::Item,
        target_outline_id: None,
        generation_round_id: "round_0001".to_string(),
        draft_id: None,
        batch_id: None,
        review_action: WorkItemPlanReviewAction::ReviseCurrentItem,
        gates: vec![WorkItemPlanReviewGate::RequiresCurrentItemRevision],
        affects_items: Vec::new(),
        warnings: Vec::new(),
    });
    verdict
}

async fn complete_sc_routing_review(engine: &mut WorkspaceEngine, verdict: ReviewVerdict) {
    engine
        .complete_review(
            crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                "single candidate review".to_string(),
                None,
            ),
            verdict,
        )
        .await;
}

/// B3收口：在场门必须是审批门（零评审义务、相位 Approval），非升级门（升级面=EnterHumanGate）。
fn sc_assert_pure_approval_gate(engine: &WorkspaceEngine) {
    assert_eq!(engine.session().stage, WorkspaceStage::HumanConfirm);
    let expected_phase = Some(crate::product::models::SingleCandidatePhase::Approval);
    assert_eq!(engine.session.single_candidate_phase, expected_phase);
    let snapshot = engine.session.human_gate_snapshot.as_ref();
    let gate = snapshot.expect("审批门快照在场");
    assert_eq!(gate.trigger, HumanReason::NativeHumanRequired);
    assert!(gate.findings.is_empty() && gate.repeated_fingerprints.is_empty());
}
