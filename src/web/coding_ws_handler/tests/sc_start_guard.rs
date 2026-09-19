//! REQ-ADV-05（work-item-plan-advance delta，change retire-legacy-workitem-protocol）
//! 句③ socket 层锚：SC admission attempt 未经 advance 置 `Ready` 时，`StartCoding`
//! 经真实 ws 入口 fail-closed 拒绝（错误码 `SC_CODING_REQUIRES_ADVANCE`），
//! attempt 状态不变、无 runner 启动、无后续事件。

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::{CodingAttemptStore, CreateGroupCodingAttemptInput};
use crate::product::coding_models::{CodingAdmissionKind, CodingAttemptStatus};
use crate::product::models::ProviderName;
use crate::web::app::build_web_router;
use crate::web::runtime::WebRuntime;
use crate::web::state::WebAppState;
use crate::web::workspace_ws_types::ProviderConfigSnapshot;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use tokio::net::TcpListener;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

type TestWsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn receive_type(ws: &mut TestWsStream, expected_type: &str, phase: &str) -> Value {
    loop {
        match timeout(Duration::from_secs(5), ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                let value: Value = serde_json::from_str(&text).unwrap();
                if value["type"] == expected_type {
                    return value;
                }
            }
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(error))) => panic!("{phase}: websocket receive failed: {error}"),
            Ok(None) => panic!("{phase}: websocket closed before expected message"),
            Err(_) => panic!("{phase}: websocket timed out before expected message"),
        }
    }
}

#[tokio::test]
async fn start_coding_before_advance_ready_is_rejected_with_sc_coding_requires_advance() {
    let root = tempfile::tempdir().unwrap();
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let mut attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(root.path().join("unmaterialized-worktree")),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes: Default::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("group attempt");
    // SC admission：绑定 group，但 durable advance 记录缺失（缺失即 fail-closed 维度）。
    attempt.admission_kind = CodingAdmissionKind::ScAdvance;
    store
        .write_coding_attempt_for_test(&attempt)
        .expect("seed sc admission attempt");
    let seeded = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("seeded attempt");
    assert_eq!(seeded.status, CodingAttemptStatus::Created);

    let app = build_web_router(state.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let url = format!(
        "ws://{addr}/ws/projects/{}/issues/{}/coding-attempts/{}",
        seeded.project_id, seeded.issue_id, seeded.id
    );
    let (mut ws, _) = connect_async(url).await.expect("coding ws connect");

    ws.send(Message::Text(r#"{"type":"start_coding"}"#.into()))
        .await
        .expect("send StartCoding");
    let error = receive_type(&mut ws, "coding_protocol_error", "guard rejection").await;
    assert_eq!(error["code"], "SC_CODING_REQUIRES_ADVANCE");
    assert_eq!(
        error["message"],
        "SC coding attempts must be made ready through advance before StartCoding"
    );

    // fail-closed：attempt 状态不变，无 runner 启动痕迹（无后续 ws 事件）。
    let unchanged = store
        .get_attempt(&seeded.project_id, &seeded.issue_id, &seeded.id)
        .expect("unchanged attempt");
    assert_eq!(unchanged.status, seeded.status);
    assert_eq!(unchanged.stage, seeded.stage);
    assert!(
        timeout(Duration::from_millis(300), ws.next())
            .await
            .is_err(),
        "no further ws traffic after guard rejection"
    );

    ws.close(None).await.ok();
    server.abort();
}
