#[tokio::test]
async fn coding_plan_repair_terminal_group_ws_rejects_start_without_invoking_provider() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let app = app_with_group_full_chain_attempt(root.path());
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            CodingAttemptStatus::Running,
        )
        .expect("run group attempt");
    store
        .update_attempt_status(
            "project_0001",
            "issue_0001",
            "coding_attempt_0001",
            CodingAttemptStatus::Aborted,
        )
        .expect("abort group attempt");
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/ws/coding-attempts/coding_attempt_0001");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;
    send_json(&mut ws, &CodingWsInMessage::StartCoding).await;

    match recv_json(&mut ws).await {
        CodingWsOutMessage::CodingProtocolError { code, .. } => {
            assert_eq!(code, "coding_message_not_allowed");
        }
        other => panic!("expected terminal start rejection, got {other:?}"),
    }
    assert!(
        store
            .list_role_runs("project_0001", "issue_0001", "coding_attempt_0001")
            .expect("role runs")
            .is_empty(),
        "terminal group invoked a provider role run"
    );

    ws.close(None).await.expect("close ws");
    server.abort();
}
