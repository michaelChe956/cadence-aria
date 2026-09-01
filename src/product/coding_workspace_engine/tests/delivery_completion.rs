#[tokio::test]
async fn maybe_complete_issue_delivery_keeps_status_when_partial() {
    let (_root, store, engine) = delivery_fixture();
    let attempt1 = seed_delivery_attempt(&store, "work_item_0001", "repo_alpha", "sha111");
    seed_delivery_review_request(&store, &attempt1, PushStatus::Pushed, None);
    let attempt2 = seed_delivery_attempt(&store, "work_item_0002", "repo_beta", "sha222");
    seed_delivery_review_request(
        &store,
        &attempt2,
        PushStatus::Failed,
        Some("push rejected".to_string()),
    );

    engine
        .maybe_complete_issue_delivery(&attempt2)
        .expect("complete issue delivery");

    let issue = IssueStore::new(store.paths())
        .get(DELIVERY_PROJECT_ID, DELIVERY_ISSUE_ID)
        .expect("issue");
    assert_eq!(issue.status, IssueStatus::Draft);
}
