#[tokio::test]
async fn plan_repair_review_revise_routes_to_existing_review_decision_without_attestation() {
    let (tmp, lifecycle, revision_store, mut parent) = plan_repair_parent_engine();
    let child = parent
        .start_plan_repair(plan_repair_fixture(
            "plan_repair_request_review_revise_0001",
            "fingerprint_review_revise_0001",
        ))
        .await
        .unwrap();
    let mut engine = plan_repair_restarted_child_engine(&tmp, &lifecycle, child);
    engine.plan_repair_snapshot.as_mut().unwrap().stage =
        crate::product::models::PlanRepairSessionStage::PlanReview;
    engine.begin_work_item_plan_outline_review_run().await;

    engine
        .route_plan_repair_candidate_review(ReviewVerdict {
            verdict: ReviewVerdictType::Revise,
            comments: "candidate requires revision".to_string(),
            summary: "revise candidate".to_string(),
            findings: vec![],
            review_gate: ReviewGate::RequiresRevision,
            work_item_plan_review: Some(WorkItemPlanReviewComplete {
                verdict: WorkItemPlanReviewVerdict::Revise,
                review_scope: WorkItemPlanReviewScope::Outline,
                target_outline_id: None,
                generation_round_id: "round_0001".to_string(),
                draft_id: None,
                batch_id: None,
                review_action: WorkItemPlanReviewAction::ReviseOutline,
                gates: vec![WorkItemPlanReviewGate::RequiresPlanReopen],
                affects_items: vec![],
                warnings: vec![],
            }),
            structured_output_diagnostic: None,
        })
        .await;

    assert_eq!(engine.current_stage(), WorkspaceStage::ReviewDecision);
    assert!(
        engine
            .plan_repair_session_state()
            .unwrap()
            .package_identity
            .is_none()
    );
    let plan = revision_store
        .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
        .unwrap();
    assert!(matches!(
        revision_store.get_plan_repair_review_attestation(
            &plan,
            "plan_repair_review_attestation_plan_amendment_missing_round_0001"
        ),
        Err(crate::product::json_store::ProductStoreError::NotFound { .. })
    ));
}
