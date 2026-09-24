//! REQ-NDR-05 用例：usage 事件 durable 落盘失败必须留可检索痕迹，且不阻断 emit。
//!
//! 降级链（都不改 gate / provider 行为）：节点 detail 诊断事件（首选，durable + 可见）
//! → 会话级 append-only `usage-diagnostics.jsonl`（节点 detail 通道本身不可写时）
//! → 结构化 warn（最后手段）。

use std::path::PathBuf;

use tempfile::TempDir;

use super::*;
use crate::cross_cutting::streaming_provider::UsageReportData;
use crate::cross_cutting::tracing_capture::SharedTracingCapture;
use crate::product::app_paths::ProductAppPaths;
use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, LifecycleStore};
use crate::product::workspace_engine::provider_drive::usage_diagnostic::UsagePersistDiagnostic;

/// 与 `persistent_test_engine` 同构，但保留 engine 事件通道以断言「emit 未被阻断」。
fn usage_probe_engine() -> (
    TempDir,
    LifecycleStore,
    WorkspaceEngine,
    mpsc::Receiver<EngineEvent>,
) {
    let (tmp, checkpoint_store) = setup();
    let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let (event_tx, event_rx) = mpsc::channel(64);
    let session_record = lifecycle_store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_spec_0001".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: ProviderName::Codex,
            review_rounds: 2,
            superpowers_enabled: true,
            openspec_enabled: true,
            work_item_plan_options: None,
        })
        .expect("create session");
    let session = WorkspaceSession::from_record(session_record);
    let engine = WorkspaceEngine::new_persistent(
        checkpoint_store,
        lifecycle_store.clone(),
        event_tx,
        session,
    );
    (tmp, lifecycle_store, engine, event_rx)
}

fn timeline_root(store: &LifecycleStore, session_id: &str) -> PathBuf {
    store
        .workspace_timeline_root_for_session(session_id)
        .expect("timeline root")
}

/// 诊断 jsonl 路径：文件名是事后检索脚本的契约，用例按字面量拼。
fn usage_diagnostics_path(store: &LifecycleStore, session_id: &str) -> PathBuf {
    timeline_root(store, session_id).join("usage-diagnostics.jsonl")
}

/// 让 node detail 通道读写失败：把 `timeline_node_details` 目录换成同名文件；
/// 会话 timeline 根仍可写，因此 journal 降级通道可用。
fn block_node_detail_channel(store: &LifecycleStore, session_id: &str) {
    let root = timeline_root(store, session_id);
    std::fs::create_dir_all(&root).expect("timeline root");
    let details = root.join("timeline_node_details");
    let _ = std::fs::remove_dir_all(&details);
    std::fs::write(&details, "blocked").expect("block node detail dir");
}

fn author_usage_report() -> UsageReportData {
    UsageReportData {
        role: "author".to_string(),
        input_tokens: Some(1200),
        output_tokens: Some(80),
        cache_read_tokens: Some(600),
        cache_creation_tokens: None,
    }
}

/// 用例①（红→绿）：usage 事件节点 detail 落盘失败 → durable 诊断 journal 留痕，
/// 且 usage 事件照旧广播（落盘失败不得阻断 emit 或改变会话行为）。
#[tokio::test]
async fn usage_persist_failure_leaves_durable_diagnostic_and_keeps_emitting() {
    let (_tmp, store, mut engine, mut event_rx) = usage_probe_engine();
    let session_id = engine.session().session_id.clone();
    let node_id = create_author_run_node(&mut engine).await;
    block_node_detail_channel(&store, &session_id);

    engine
        .emit_execution_event(
            execution_event_from_usage_report(author_usage_report()),
            Some(node_id.clone()),
            Some(ProviderName::ClaudeCode),
        )
        .await;

    let events = drain_engine_events(&mut event_rx);
    assert!(
        events.iter().any(|event| matches!(
            event,
            EngineEvent::ExecutionEvent { event, .. }
                if event.kind == ProviderExecutionEventKind::Usage
                    && event.event_id == "usage_author"
        )),
        "usage 事件必须照旧广播（落盘失败不得阻断 emit），已收 {} 个事件",
        events.len()
    );

    let journal = usage_diagnostics_path(&store, &session_id);
    let content = std::fs::read_to_string(&journal).expect("durable usage diagnostics journal");
    let record: serde_json::Value =
        serde_json::from_str(content.lines().last().expect("journal record"))
            .expect("journal json");
    assert_eq!(record["schema_version"], 1);
    assert_eq!(record["stage"], "persist");
    assert_eq!(record["workspace_session_id"], session_id);
    assert_eq!(record["node_id"], node_id);
    assert_eq!(record["event_id"], "usage_author");
    let error = record["error"].as_str().expect("journal error");
    assert!(
        error.contains("node detail failed"),
        "journal 必须记录 node detail 通道的失败原因：{record:?}"
    );
    assert!(
        error.contains("timeline_node_details"),
        "journal 必须能定责到落盘点（node detail 路径）：{record:?}"
    );
    assert!(
        error.chars().count() <= 200,
        "journal 错误文本必须有界：{record:?}"
    );
    assert!(
        !record["recorded_at"]
            .as_str()
            .expect("recorded_at")
            .is_empty(),
        "{record:?}"
    );
}

/// 用例②：节点 detail 通道可用时诊断落到节点 detail 本身（durable + 可见），
/// 且 error 有界、`output` 不携带 usage 负载、不重复落 journal。
#[tokio::test]
async fn usage_persist_diagnostic_prefers_node_detail_and_stays_bounded() {
    let (_tmp, store, mut engine, _event_rx) = usage_probe_engine();
    let session_id = engine.session().session_id.clone();
    let node_id = create_author_run_node(&mut engine).await;
    let long_error = "persist-failure;".repeat(40);

    let tier = engine
        .record_usage_persist_diagnostic(&node_id, "usage_author", &long_error)
        .await;
    assert_eq!(tier, UsagePersistDiagnostic::NodeDetail);

    let detail = store
        .load_node_detail(&session_id, &node_id)
        .expect("node detail");
    let event = detail
        .execution_events
        .iter()
        .find(|event| event["event_id"] == "usage_author_persist_diag")
        .expect("usage persist diagnostic event");
    assert_eq!(event["kind"], "usage");
    assert_eq!(event["status"], "failed");
    assert_eq!(event["title"], "Usage persistence failed");
    assert!(
        event["output"].is_null(),
        "诊断事件不得携带 usage 负载：{event:?}"
    );
    let diagnostic: serde_json::Value =
        serde_json::from_str(event["detail"].as_str().expect("diagnostic detail json"))
            .expect("diagnostic detail parses");
    assert_eq!(diagnostic["diagnostic_version"], 1);
    assert_eq!(diagnostic["persist_stage"], "persist");
    assert_eq!(diagnostic["event_id"], "usage_author");
    assert_eq!(
        diagnostic["error"]
            .as_str()
            .expect("diagnostic error")
            .chars()
            .count(),
        200,
        "错误文本必须有界：{diagnostic:?}"
    );
    assert!(
        !usage_diagnostics_path(&store, &session_id).exists(),
        "节点 detail 可用时不得重复落 journal"
    );
}

/// 用例③：连降级分区都不可写（timeline 根被文件占用）→ 只剩结构化 warn，
/// 且 emit 仍广播、调用正常返回（诊断写失败不影响 provider/gate 行为）。
#[tokio::test]
async fn usage_persist_diagnostic_degrading_to_warn_does_not_block_emit() {
    let capture = SharedTracingCapture::global();
    let (_tmp, store, mut engine, mut event_rx) = usage_probe_engine();
    let session_id = engine.session().session_id.clone();
    let node_id = create_author_run_node(&mut engine).await;
    let root = timeline_root(&store, &session_id);
    let _ = std::fs::remove_dir_all(&root);
    std::fs::write(&root, "blocked").expect("block timeline root");

    let tier = engine
        .record_usage_persist_diagnostic(&node_id, "usage_author", "boom")
        .await;
    assert_eq!(tier, UsagePersistDiagnostic::LogOnly);
    let logs = capture.captured();
    assert!(logs.contains("stage=\"persist\""), "logs: {logs}");
    assert!(logs.contains("usage_author"), "logs: {logs}");
    assert!(
        logs.contains("usage persistence diagnostic unavailable"),
        "logs: {logs}"
    );

    engine
        .emit_execution_event(
            execution_event_from_usage_report(author_usage_report()),
            Some(node_id),
            Some(ProviderName::ClaudeCode),
        )
        .await;
    let events = drain_engine_events(&mut event_rx);
    assert!(
        events.iter().any(|event| matches!(
            event,
            EngineEvent::ExecutionEvent { event, .. }
                if event.event_id == "usage_author"
        )),
        "诊断写失败不得阻断 usage 事件广播，已收 {} 个事件",
        events.len()
    );
}
