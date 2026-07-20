use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::coding_attempt_store::CodingAttemptStore;
use cadence_aria::product::coding_models::CodingPlanAmendmentDeliveryStatus;
use cadence_aria::product::lifecycle_store::LifecycleStore;
use cadence_aria::product::models::PlanRepairSessionStage;
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::{CodingAttemptRunKey, WebAppState};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

type PlanRepairWsStream = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

async fn receive_plan_repair_ws_json(ws: &mut PlanRepairWsStream) -> Value {
    loop {
        match timeout(Duration::from_secs(3), ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                return serde_json::from_str(&text).expect("websocket json message");
            }
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(error))) => panic!("websocket receive failed: {error}"),
            Ok(None) => panic!("websocket closed before expected message"),
            Err(_) => panic!("websocket timed out before expected message"),
        }
    }
}

async fn receive_plan_repair_ws_type(
    ws: &mut PlanRepairWsStream,
    expected_type: &str,
) -> Value {
    loop {
        let message = receive_plan_repair_ws_json(ws).await;
        if message["type"] == expected_type {
            return message;
        }
    }
}

#[tokio::test]
async fn child_confirmation_publishes_applies_and_restarts_through_real_websockets() {
    let root = tempdir().expect("fixture root");
    let runtime = PlanRepairFixtureRuntime::seed(root.path(), PlanRepairFixtureControl::default())
        .await
        .expect("seed plan repair fixture");
    let identity = runtime
        .drive_until_awaiting_confirmation()
        .await
        .expect("prepare awaiting confirmation fixture");

    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local address");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve web app");
    });

    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let coding_store = CodingAttemptStore::new(paths.clone());
    let attempt = coding_store
        .get_attempt_for_work_item_group(
            "project_0001",
            "issue_plan_0001",
            "work_item_plan_0001",
        )
        .expect("load group attempt")
        .expect("group attempt");
    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);

    let coding_url = format!(
        "ws://{addr}/ws/projects/{}/issues/{}/coding-attempts/{}",
        attempt.project_id, attempt.issue_id, attempt.id
    );
    let (mut coding_ws, _) = connect_async(coding_url).await.expect("connect coding websocket");
    let initial_coding = receive_plan_repair_ws_json(&mut coding_ws).await;
    assert_eq!(initial_coding["type"], "coding_session_state");

    let child_url = format!(
        "ws://{addr}/api/ws/workspace/{}",
        identity.child_session_id
    );
    let (mut child_ws, _) = connect_async(child_url).await.expect("connect child websocket");
    let initial_child = receive_plan_repair_ws_json(&mut child_ws).await;
    assert_eq!(
        initial_child["type"],
        "session_state",
        "unexpected child websocket message: {initial_child}"
    );

    child_ws
        .send(Message::Text(
            json!({
                "type": "confirm_plan_amendment",
                "amendment_id": identity.amendment_id,
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send amendment confirmation");

    let amendment_event =
        receive_plan_repair_ws_type(&mut coding_ws, "plan_amendment_updated").await;
    assert_eq!(
        amendment_event["amendment"]["id"],
        identity.amendment_id
    );

    let resumed = timeout(Duration::from_secs(3), async {
        loop {
            let current = coding_store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .expect("reload attempt");
            let binding = coding_store
                .get_plan_binding(&current)
                .expect("reload plan binding");
            if binding.bound_plan_revision_id == "plan_revision_0002"
                && binding
                    .applied_amendment_ids
                    .contains(&identity.amendment_id)
                && state.coding_runs.runner_count(&attempt_key) == 1
            {
                break current;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("amendment application and runner restart timed out");

    let delivery = coding_store
        .get_plan_amendment_delivery(&resumed, &identity.amendment_id)
        .expect("load amendment delivery");
    assert_eq!(delivery.status, CodingPlanAmendmentDeliveryStatus::Delivered);
    let child_snapshot = LifecycleStore::new(paths)
        .load_plan_repair_session_state(
            &attempt.project_id,
            &attempt.issue_id,
            &identity.child_session_id,
        )
        .expect("load child snapshot")
        .expect("child snapshot");
    assert_eq!(child_snapshot.stage, PlanRepairSessionStage::Completed);

    child_ws
        .send(Message::Text(
            json!({
                "type": "confirm_plan_amendment",
                "amendment_id": identity.amendment_id,
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("replay amendment confirmation");
    let _ = receive_plan_repair_ws_json(&mut child_ws).await;
    assert_eq!(state.coding_runs.runner_count(&attempt_key), 1);

    coding_ws.close(None).await.ok();
    child_ws.close(None).await.ok();
    server.abort();
}

#[tokio::test]
async fn child_confirmation_retries_activation_after_coding_socket_connects() {
    let root = tempdir().expect("fixture root");
    let runtime = PlanRepairFixtureRuntime::seed(root.path(), PlanRepairFixtureControl::default())
        .await
        .expect("seed plan repair fixture");
    let identity = runtime
        .drive_until_awaiting_confirmation()
        .await
        .expect("prepare awaiting confirmation fixture");

    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local address");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve web app");
    });

    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let coding_store = CodingAttemptStore::new(paths);
    let attempt = coding_store
        .get_attempt_for_work_item_group(
            "project_0001",
            "issue_plan_0001",
            "work_item_plan_0001",
        )
        .expect("load group attempt")
        .expect("group attempt");
    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    let child_url = format!(
        "ws://{addr}/api/ws/workspace/{}",
        identity.child_session_id
    );
    let (mut child_ws, _) = connect_async(child_url).await.expect("connect child websocket");
    let initial_child = receive_plan_repair_ws_json(&mut child_ws).await;
    assert_eq!(initial_child["type"], "session_state");

    child_ws
        .send(Message::Text(
            json!({
                "type": "confirm_plan_amendment",
                "amendment_id": identity.amendment_id,
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("send confirmation without coding websocket");
    let activation_error = receive_plan_repair_ws_type(&mut child_ws, "protocol_error").await;
    assert_eq!(
        activation_error["code"],
        "PLAN_AMENDMENT_ACTIVATION_FAILED"
    );
    assert_eq!(state.coding_runs.runner_count(&attempt_key), 0);

    let coding_url = format!(
        "ws://{addr}/ws/projects/{}/issues/{}/coding-attempts/{}",
        attempt.project_id, attempt.issue_id, attempt.id
    );
    let (mut coding_ws, _) = connect_async(coding_url).await.expect("connect coding websocket");
    let initial_coding = receive_plan_repair_ws_json(&mut coding_ws).await;
    assert_eq!(initial_coding["type"], "coding_session_state");
    child_ws
        .send(Message::Text(
            json!({
                "type": "confirm_plan_amendment",
                "amendment_id": identity.amendment_id,
            })
            .to_string()
            .into(),
        ))
        .await
        .expect("retry amendment confirmation");

    let amendment_event =
        receive_plan_repair_ws_type(&mut coding_ws, "plan_amendment_updated").await;
    assert_eq!(
        amendment_event["amendment"]["id"],
        identity.amendment_id
    );
    timeout(Duration::from_secs(3), async {
        loop {
            let current = coding_store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .expect("reload attempt");
            if coding_store
                .get_plan_binding(&current)
                .expect("reload plan binding")
                .applied_amendment_ids
                == vec![identity.amendment_id.clone()]
                && state.coding_runs.runner_count(&attempt_key) == 1
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("activation retry timed out");
    assert_eq!(
        coding_store
            .list_amendment_application_journals(&attempt)
            .expect("list amendment applications")
            .len(),
        1
    );

    coding_ws.close(None).await.ok();
    child_ws.close(None).await.ok();
    server.abort();
}
