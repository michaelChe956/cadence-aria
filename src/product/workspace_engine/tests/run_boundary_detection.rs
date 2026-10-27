//! REQ-PRB-10/11：告警事件形状、去重、上界、范围，以及「只告警不阻断」。

use super::*;

const CODE_TMP: &str = "run_boundary_tmp_draft_touch";

fn command_event(event_id: &str, command: &str) -> ProviderExecutionEvent {
    ProviderExecutionEvent {
        event_id: event_id.to_string(),
        kind: ProviderExecutionEventKind::Command,
        status: ProviderExecutionEventStatus::Started,
        title: "bash".to_string(),
        detail: Some(command.to_string()),
        command: Some(command.to_string()),
        cwd: None,
        output: None,
        exit_code: None,
    }
}

fn alert_events(store: &LifecycleStore, session_id: &str, node_id: &str) -> Vec<serde_json::Value> {
    store
        .load_node_detail(session_id, node_id)
        .expect("node detail")
        .execution_events
        .into_iter()
        .filter(|event| {
            event
                .get("event_id")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|id| id.starts_with("run_boundary_"))
        })
        .collect()
}

#[tokio::test]
async fn command_event_keeps_original_and_appends_alert() {
    let (_tmp, store, mut engine) = persistent_test_engine();
    let session_id = engine.session().session_id.clone();
    let node_id = create_author_run_node(&mut engine).await;

    engine
        .emit_execution_event(
            command_event("call_1", "find /tmp -iname '*work*item*plan*'"),
            Some(node_id.clone()),
            Some(ProviderName::ClaudeCode),
        )
        .await;

    let detail = store
        .load_node_detail(&session_id, &node_id)
        .expect("detail");
    let original = detail
        .execution_events
        .iter()
        .find(|event| event.get("event_id").and_then(|value| value.as_str()) == Some("call_1"))
        .expect("原事件仍在");
    assert_eq!(
        original.get("command").and_then(|value| value.as_str()),
        Some("find /tmp -iname '*work*item*plan*'"),
        "原事件不得被修改"
    );

    let alerts = alert_events(&store, &session_id, &node_id);
    assert_eq!(alerts.len(), 1, "{alerts:?}");
    assert_eq!(alerts[0]["kind"], "provider");
    assert_eq!(alerts[0]["status"], "completed");
    assert_eq!(alerts[0]["title"], format!("运行边界告警: {CODE_TMP}"));
    let alert_id = alerts[0]["event_id"].as_str().unwrap();
    assert!(alert_id.starts_with("run_boundary_tmp_draft_touch_"));
    assert_eq!(alert_id.len(), "run_boundary_tmp_draft_touch_".len() + 8);
    let alert_detail = alerts[0]["detail"].as_str().unwrap();
    assert!(alert_detail.contains("path="), "{alert_detail}");
    assert!(alert_detail.contains("access=read"), "{alert_detail}");
    assert!(
        alert_detail.contains("source_event=call_1"),
        "{alert_detail}"
    );
    assert!(
        alerts[0]["command"].is_null()
            && alerts[0]["cwd"].is_null()
            && alerts[0]["output"].is_null()
    );
}

#[tokio::test]
async fn alerts_are_deduped_and_capped_at_eight() {
    let (_tmp, store, mut engine) = persistent_test_engine();
    let session_id = engine.session().session_id.clone();
    let node_id = create_author_run_node(&mut engine).await;

    for _ in 0..3 {
        engine
            .emit_execution_event(
                command_event("call_same", "cat /tmp/work-item-plan.md"),
                Some(node_id.clone()),
                Some(ProviderName::ClaudeCode),
            )
            .await;
    }
    assert_eq!(
        alert_events(&store, &session_id, &node_id).len(),
        1,
        "同路径同规则按 id 去重"
    );

    for index in 0..10 {
        engine
            .emit_execution_event(
                command_event(
                    &format!("call_{index}"),
                    &format!("cat /tmp/work-item-plan-{index}.md"),
                ),
                Some(node_id.clone()),
                Some(ProviderName::ClaudeCode),
            )
            .await;
    }
    assert!(
        alert_events(&store, &session_id, &node_id).len() <= 8,
        "每 node 至多 8 条"
    );
}

#[tokio::test]
async fn reviewer_node_is_not_scanned() {
    let (_tmp, store, mut engine) = persistent_test_engine();
    let session_id = engine.session().session_id.clone();
    let node_id = create_reviewer_run_node(&mut engine).await;

    engine
        .emit_execution_event(
            command_event("call_review", "cat /tmp/work-item-plan.md"),
            Some(node_id.clone()),
            Some(ProviderName::Codex),
        )
        .await;

    assert!(
        alert_events(&store, &session_id, &node_id).is_empty(),
        "reviewer 节点不扫"
    );
}

#[tokio::test]
async fn alert_event_does_not_rescan() {
    let (_tmp, store, mut engine) = persistent_test_engine();
    let session_id = engine.session().session_id.clone();
    let node_id = create_author_run_node(&mut engine).await;

    engine
        .emit_execution_event(
            ProviderExecutionEvent {
                event_id: format!("run_boundary_tmp_draft_touch_{}", "0".repeat(8)),
                kind: ProviderExecutionEventKind::Provider,
                status: ProviderExecutionEventStatus::Completed,
                title: "运行边界告警: run_boundary_tmp_draft_touch".to_string(),
                detail: Some(
                    "path=/tmp/work-item-plan.md access=read source_event=call_x".to_string(),
                ),
                command: None,
                cwd: None,
                output: None,
                exit_code: None,
            },
            Some(node_id.clone()),
            Some(ProviderName::ClaudeCode),
        )
        .await;

    assert_eq!(
        alert_events(&store, &session_id, &node_id).len(),
        1,
        "告警事件不再触发扫描"
    );
}

#[tokio::test]
async fn detection_failure_does_not_affect_run() {
    // REQ-PRB-11：无 store 的内存态 engine，检测路径整段跳过，事件照常广播。
    let (tmp, checkpoint_store) = setup();
    let (event_tx, mut event_rx) = mpsc::channel(64);
    let mut engine = WorkspaceEngine::new(
        checkpoint_store,
        event_tx,
        make_session("run-boundary-headless"),
    );
    let node_id = create_author_run_node(&mut engine).await;

    engine
        .emit_execution_event(
            command_event("call_headless", "cat /tmp/work-item-plan.md"),
            Some(node_id),
            Some(ProviderName::ClaudeCode),
        )
        .await;

    let events = drain_engine_events(&mut event_rx);
    assert!(
        events.iter().any(|event| matches!(
            event,
            EngineEvent::ExecutionEvent { event, .. } if event.event_id == "call_headless"
        )),
        "原事件必须照常广播"
    );
    drop(tmp);
}
