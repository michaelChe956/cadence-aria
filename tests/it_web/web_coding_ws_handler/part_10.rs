#[tokio::test]
async fn coding_ws_hello_and_ping_do_not_wait_for_attempt_recovery_lock() {
    let _guard = WS_TEST_LOCK.lock().await;
    let root = tempdir().expect("root");
    let _setup = app_with_attempt(root.path());
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let attempt_id = "coding_attempt_0001";
    let url = format!("ws://{addr}/ws/coding-attempts/{attempt_id}");
    let (mut ws, _) = connect_async(url).await.expect("connect ws");
    let _initial = recv_json(&mut ws).await;
    let attempt_guard = state.coding_runs.lock_attempt(attempt_id).await;

    send_json(
        &mut ws,
        &CodingWsInMessage::CodingHello {
            attempt_id: attempt_id.to_string(),
            last_seen_node_id: None,
        },
    )
    .await;
    send_json(&mut ws, &CodingWsInMessage::CodingPing).await;

    let pong = timeout(Duration::from_millis(100), recv_json(&mut ws))
        .await
        .expect("hello/ping must bypass the attempt recovery lock");
    assert_eq!(pong, CodingWsOutMessage::CodingPong);

    drop(attempt_guard);
    ws.close(None).await.expect("close ws");
    server.abort();
}
