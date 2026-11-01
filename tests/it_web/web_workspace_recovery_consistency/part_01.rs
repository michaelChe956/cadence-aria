use axum::http::Method;
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::checkpoint_store::CheckpointStore;
use cadence_aria::product::lifecycle_store::LifecycleStore;
use cadence_aria::product::models::WorkspaceType;
use cadence_aria::product::workspace_engine::{EngineEvent, WorkspaceEngine, WorkspaceSession};
use cadence_aria::web::workspace_ws_types::{
    ArtifactPayload, ProviderConfigSnapshot, TimelineNode, TimelineNodeStatus, TimelineNodeType,
    ValidatorFindingDto, VerificationCommandDto, VerificationManualCheckDto, VerificationPlanDto,
    WorkItemCandidateDto, WorkItemCandidateMetaDto, WorkItemDependencyEdgeDto,
    WorkItemPlanCandidateDto, WorkItemPlanDto, WorkItemSplitOptionsDto, WorkspaceStage,
    WsCheckpointDto, WsMessageDto, WsOutMessage, WsProviderConfig,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::time::{Duration, timeout};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::web_work_item_generation::{
    app_with_confirmed_story_and_design_and_streaming_outputs, request_json, valid_outline_output,
};

static WS_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;



async fn connect_ws(app: axum::Router, session_id: &str) -> WsStream {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });

    let url = format!("ws://{addr}/api/ws/workspace/{session_id}");
    let (ws, _) = connect_async(url).await.expect("connect ws");

    tokio::spawn(async move {
        server.await.ok();
    });

    ws
}

async fn recv_ws_until<F>(ws: &mut WsStream, timeout_after: Duration, predicate: F) -> Vec<Value>
where
    F: Fn(&[Value]) -> bool,
{
    let mut messages = Vec::new();
    let deadline = tokio::time::Instant::now() + timeout_after;
    while tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        match timeout(remaining, ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str(&text) {
                Ok(value) => {
                    messages.push(value);
                    if predicate(&messages) {
                        break;
                    }
                }
                Err(error) => panic!("ws json parse error: {error}\n{text}"),
            },
            Ok(Some(Ok(Message::Close(_)))) => break,
            Ok(Some(Ok(other))) => {
                eprintln!("ignoring non-text ws message: {other:?}");
                continue;
            }
            Ok(Some(Err(error))) => panic!("ws error: {error}"),
            Ok(None) => break,
            Err(_) => break,
        }
    }
    messages
}

async fn generate_session_to_author_confirm(
    app: &axum::Router,
    endpoint: &str,
    body: Value,
) -> String {
    let (_status, resp) = request_json(app.clone(), Method::POST, endpoint, body).await;
    let session_id = resp["workspace_session"]["workspace_session_id"]
        .as_str()
        .unwrap()
        .to_string();

    let mut ws = connect_ws(app.clone(), &session_id).await;
    ws.send(Message::Text(
        json!({
            "type": "start_generation",
            "provider_config": { "author": "fake", "reviewer": null, "review_rounds": 0 },
            "reviewer_enabled": false
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send start_generation");

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(15), |msgs| {
        msgs.iter().any(|m| m["type"] == "artifact_update")
            && msgs
                .iter()
                .any(|m| m["type"] == "stage_change" && m["stage"] == "author_confirm")
    })
    .await;

    ws.close(None).await.ok();
    session_id
}


fn recover_engine(repo: &tempfile::TempDir, session_id: &str) -> WorkspaceEngine {
    let app_paths = ProductAppPaths::new(repo.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let checkpoint_store = Arc::new(CheckpointStore::new(
        app_paths.issue_lifecycle_root("project_0001", "issue_0001"),
    ));
    let (event_tx, _event_rx) = mpsc::channel::<EngineEvent>(1);
    let session_record = lifecycle
        .get_workspace_session(session_id)
        .expect("session record exists");
    let mut session = WorkspaceSession::from_record(session_record);
    session.repository_path = Some(repo.path().join("repo"));
    WorkspaceEngine::new_persistent(checkpoint_store, lifecycle, event_tx, session)
}

// 退役留档（T5/REQ-RET-02）：`story_design_work_item_plan_recovery_consistency` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。


// 退役留档（T5/REQ-RET-02）：`story_workspace_review_sentinel_fallback_still_passes` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

#[tokio::test]
async fn design_workspace_artifact_history_still_loads_markdown() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, repo, _prompts) =
        app_with_confirmed_story_and_design_and_streaming_outputs(vec![valid_outline_output()])
            .await;

    let design_session_id = generate_session_to_author_confirm(
        &app,
        "/api/projects/project_0001/issues/issue_0001/design-specs:generate",
        json!({
            "title": "Design artifact history",
            "story_spec_ids": ["story_spec_0001"],
            "author_provider": "fake",
            "reviewer_provider": null,
            "review_rounds": 1,
            "superpowers_enabled": false,
            "openspec_enabled": true
        }),
    )
    .await;

    let app_paths = ProductAppPaths::new(repo.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths);
    let versions = lifecycle
        .list_artifact_versions(&design_session_id)
        .expect("list design artifact versions");
    assert!(
        versions
            .iter()
            .any(|version| version.is_current && version.markdown().contains("# Design Spec")),
        "Design artifact history should keep readable markdown versions: {versions:?}"
    );

    let design_engine = recover_engine(&repo, &design_session_id);
    match design_engine.build_session_state() {
        WsOutMessage::SessionState {
            workspace_type,
            artifact,
            artifact_versions,
            artifact_version_summaries,
            ..
        } => {
            assert_eq!(workspace_type, WorkspaceType::Design);
            assert!(artifact_versions.is_empty());
            assert!(
                artifact_version_summaries
                    .iter()
                    .any(|summary| summary.is_current && summary.markdown_size > 0),
                "SessionState should expose Design artifact history summaries"
            );
            let markdown = artifact
                .as_ref()
                .and_then(|payload| payload.markdown())
                .expect("recovered design artifact markdown");
            assert!(markdown.contains("# Design Spec"));
        }
        other => panic!("expected Design SessionState, got {other:?}"),
    }
}

// 退役留档（T5/REQ-RET-02）：`ordinary_work_item_workspace_review_unaffected` 直接驱动已删除的 legacy 决策面，
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。
