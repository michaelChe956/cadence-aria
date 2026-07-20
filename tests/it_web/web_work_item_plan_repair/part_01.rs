#[tokio::test]
async fn web_work_item_plan_repair_rewrites_only_upstream_and_resumes_consumer() {
    let root = tempdir().expect("fixture root");
    let runtime = PlanRepairFixtureRuntime::seed(root.path(), PlanRepairFixtureControl::default())
        .await
        .expect("seed plan repair fixture");

    let waiting = runtime
        .drive_until_review_finds_upstream_contract_invalid()
        .await
        .expect("route upstream contract defect to plan repair");
    assert_eq!(waiting.attempt_status, "awaiting_plan_amendment");
    assert_eq!(waiting.active_logical_work_item_id, "wi_registration");
    assert_eq!(waiting.active_unit_rework_count, 0);

    let recovered = runtime
        .confirm_publish_apply_and_resume()
        .await
        .expect("confirm amendment and resume the consumer");
    assert_eq!(recovered.bound_plan_revision_id, "plan_revision_0002");
    assert_eq!(recovered.active_plan_revision_id, "plan_revision_0002");
    assert_eq!(recovered.active_amendment_id, None);
    assert_eq!(
        recovered.logical_active_revision_ids["wi_core"],
        "work_item_revision_wi_core_0002"
    );
    assert_eq!(
        recovered.logical_active_revision_ids["wi_registration"],
        "work_item_revision_wi_registration_0001"
    );
    assert_eq!(
        recovered.current_work_item_revision_id,
        "work_item_revision_wi_registration_0001"
    );
    assert_eq!(
        recovered.current_resolved_handoff_revision_ids,
        vec!["handoff_revision_0002"]
    );
    assert_eq!(recovered.rewritten_logical_work_item_ids, vec!["wi_core"]);
    assert_eq!(
        recovered.revalidated_logical_work_item_ids,
        vec!["wi_registration"]
    );
    assert!(
        !recovered
            .rewritten_logical_work_item_ids
            .contains(&"wi_unrelated".to_string())
    );
    assert_eq!(recovered.amendment_ids.len(), 1);
    assert!(recovered.amendment_ids[0].starts_with("plan_amendment_"));
    assert_eq!(
        recovered.handoff_revision_ids,
        vec!["handoff_revision_0001", "handoff_revision_0002"]
    );
}
