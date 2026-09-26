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

async fn receive_json(ws: &mut TestWsStream, phase: &str) -> Value {
    loop {
        match timeout(Duration::from_secs(3), ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => return serde_json::from_str(&text).unwrap(),
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(error))) => panic!("{phase}: websocket receive failed: {error}"),
            Ok(None) => panic!("{phase}: websocket closed before expected message"),
            Err(_) => panic!("{phase}: websocket timed out before expected message"),
        }
    }
}

async fn receive_type(ws: &mut TestWsStream, expected_type: &str) -> Value {
    loop {
        let message = receive_json(ws, expected_type).await;
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
        .get_attempt_for_work_item_group(
            "project_0001",
            "issue_plan_0001",
            "work_item_plan_0001",
            None,
        )
        .unwrap()
        .unwrap();
    crate::web::coding_ws_handler::build_coding_session_state(&store, attempt.clone())
        .expect("plan repair fixture must build the initial coding websocket state");
    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    let failpoint =
        register_plan_amendment_delivery_mark_failpoint(&store, &attempt, &identity.amendment_id);
    let coding_url = format!(
        "ws://{addr}/ws/projects/{}/issues/{}/coding-attempts/{}",
        attempt.project_id, attempt.issue_id, attempt.id
    );
    let (mut coding_ws, _) = connect_async(coding_url).await.unwrap();
    assert_eq!(
        receive_json(&mut coding_ws, "initial coding state").await["type"],
        "coding_session_state"
    );
    let child_url = format!("ws://{addr}/api/ws/workspace/{}", identity.child_session_id);
    let (mut child_ws, _) = connect_async(child_url).await.unwrap();
    let initial_child = receive_json(&mut child_ws, "initial plan repair child state").await;
    assert_eq!(initial_child["type"], "session_state");
    assert_eq!(
        initial_child["stage"], "human_confirm",
        "plan amendment child must accept confirmation: {initial_child}"
    );
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
    let _first_event = receive_type(&mut coding_ws, "plan_amendment_updated").await;
    // REQ-WIGA-06：mark failpoint 不再令业务失败——attempt 直达 Running，
    // marker 停 Pending（Delivered 落盘被 failpoint 拦截），runner 唯一。
    timeout(Duration::from_secs(3), async {
        loop {
            let current = store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .unwrap();
            if current.status == CodingAttemptStatus::Running
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
            .get_plan_amendment_delivery(&attempt, &identity.amendment_id)
            .unwrap()
            .status,
        CodingPlanAmendmentDeliveryStatus::Pending
    );
    drop(failpoint);

    // 重复确认：runner 已在推进（early-return 触发补投递）；首轮被拦的
    // mark 与补投递竞争（注册互斥 + mark 幂等），最终收敛真实 Delivered。
    child_ws.send(confirmation()).await.unwrap();
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

#[tokio::test]
async fn zero_socket_plan_amendment_activation_resumes_attempt_with_unsent_delivery() {
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
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .get_attempt_for_work_item_group(
            "project_0001",
            "issue_plan_0001",
            "work_item_plan_0001",
            None,
        )
        .unwrap()
        .unwrap();
    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);

    // 全程不连接 coding WS：只从 workspace 子会话确认。
    let child_url = format!("ws://{addr}/api/ws/workspace/{}", identity.child_session_id);
    let (mut child_ws, _) = connect_async(child_url).await.unwrap();
    let initial_child = receive_json(&mut child_ws, "initial plan repair child state").await;
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
        .unwrap();

    let resumed = timeout(Duration::from_secs(3), async {
        loop {
            let current = store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
                .unwrap();
            if current.status == CodingAttemptStatus::Running
                && state.coding_runs.runner_count(&attempt_key) == 1
            {
                return current;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("zero-socket activation must resume the attempt (REQ-WIGA-06)");

    let delivery = timeout(Duration::from_secs(3), async {
        loop {
            let delivery = store
                .get_plan_amendment_delivery(&resumed, &identity.amendment_id)
                .unwrap();
            if delivery.status == CodingPlanAmendmentDeliveryStatus::Unsent {
                return delivery;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("no live socket must leave a durable unsent fact, never Delivered");
    assert_eq!(delivery.delivered_at, None);

    child_ws.close(None).await.ok();
    server.abort();
}
