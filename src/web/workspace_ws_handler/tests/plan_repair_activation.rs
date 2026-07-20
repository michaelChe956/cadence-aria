use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::{
    CodingAttemptStore, register_plan_amendment_delivery_mark_failpoint,
};
use crate::product::coding_models::{CodingAttemptStatus, CodingPlanAmendmentDeliveryStatus};
use crate::web::app::build_web_router;
use crate::web::runtime::WebRuntime;
use crate::web::state::{CodingAttemptRunKey, WebAppState};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

type TestWsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn receive_json(ws: &mut TestWsStream) -> Value {
    loop {
        match timeout(Duration::from_secs(3), ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => return serde_json::from_str(&text).unwrap(),
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(error))) => panic!("websocket receive failed: {error}"),
            Ok(None) => panic!("websocket closed before expected message"),
            Err(_) => panic!("websocket timed out before expected message"),
        }
    }
}

async fn receive_type(ws: &mut TestWsStream, expected_type: &str) -> Value {
    loop {
        let message = receive_json(ws).await;
        if message["type"] == expected_type {
            return message;
        }
    }
}

#[tokio::test]
async fn repeated_confirmation_recovers_delivery_mark_failure_without_duplicate_application() {
    let root = tempfile::tempdir().unwrap();
    let runtime = crate::web::test_controls::PlanRepairFixtureRuntime::seed(
        root.path(),
        crate::web::test_controls::PlanRepairFixtureControl::default(),
    )
    .await
    .unwrap();
    let identity = runtime.drive_until_awaiting_confirmation().await.unwrap();
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .get_attempt_for_work_item_group("project_0001", "issue_plan_0001", "work_item_plan_0001")
        .unwrap()
        .unwrap();
    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    let failpoint =
        register_plan_amendment_delivery_mark_failpoint(&store, &attempt, &identity.amendment_id);
    let coding_url = format!(
        "ws://{addr}/ws/projects/{}/issues/{}/coding-attempts/{}",
        attempt.project_id, attempt.issue_id, attempt.id
    );
    let (mut coding_ws, _) = connect_async(coding_url).await.unwrap();
    assert_eq!(
        receive_json(&mut coding_ws).await["type"],
        "coding_session_state"
    );
    let child_url = format!("ws://{addr}/api/ws/workspace/{}", identity.child_session_id);
    let (mut child_ws, _) = connect_async(child_url).await.unwrap();
    assert_eq!(receive_json(&mut child_ws).await["type"], "session_state");
    let confirmation = || {
        Message::Text(
            json!({
                "type": "confirm_plan_amendment",
                "amendment_id": identity.amendment_id,
            })
            .to_string()
            .into(),
        )
    };

    child_ws.send(confirmation()).await.unwrap();
    let first_event = receive_type(&mut coding_ws, "plan_amendment_updated").await;
    timeout(Duration::from_secs(3), async {
        loop {
            let current = store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .unwrap();
            if current.status == CodingAttemptStatus::AmendmentApplyFailed
                && state.coding_runs.runner_count(&attempt_key) == 0
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        store
            .get_plan_amendment_delivery(&attempt, &identity.amendment_id)
            .unwrap()
            .status,
        CodingPlanAmendmentDeliveryStatus::Pending
    );
    drop(failpoint);

    child_ws.send(confirmation()).await.unwrap();
    let second_event = receive_type(&mut coding_ws, "plan_amendment_updated").await;
    assert_eq!(second_event["event_id"], first_event["event_id"]);
    timeout(Duration::from_secs(3), async {
        loop {
            let current = store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .unwrap();
            let delivery = store
                .get_plan_amendment_delivery(&current, &identity.amendment_id)
                .unwrap();
            if current.status == CodingAttemptStatus::Running
                && delivery.status == CodingPlanAmendmentDeliveryStatus::Delivered
                && state.coding_runs.runner_count(&attempt_key) == 1
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        store
            .list_amendment_application_journals(&attempt)
            .unwrap()
            .len(),
        1
    );

    coding_ws.close(None).await.ok();
    child_ws.close(None).await.ok();
    server.abort();
}
