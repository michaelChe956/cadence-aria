// coding provider stream 的 token 用量接线测试（usage 接通/Rust 面）。
//
// 行为契约与 workspace_engine 的 provider_session_maps_usage_report_to_usage_execution_event
// 对齐：ProviderEvent::UsageReport 必须映射为 kind=usage 的 CodingExecutionEvent
// 送达 WS（event_id 固定 `usage_{role}`，消费侧按 event_id upsert 覆盖最新快照），
// 同时落 role-run 审计；usage 采集为 best-effort，不得影响完成语义。
use super::*;
use crate::cross_cutting::streaming_provider::{ProviderCompletion, UsageReportData};

struct UsageReportingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for UsageReportingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::UsageReport(UsageReportData {
                    role: "author".to_string(),
                    input_tokens: Some(1200),
                    output_tokens: Some(80),
                    cache_read_tokens: Some(600),
                    cache_creation_tokens: None,
                }))
                .await;
            // 同 role 第二次上报：event_id 稳定为 usage_{role}，消费侧按
            // event_id upsert 覆盖为最新快照。
            let _ = event_tx
                .send(ProviderEvent::UsageReport(UsageReportData {
                    role: "author".to_string(),
                    input_tokens: Some(1500),
                    output_tokens: Some(120),
                    cache_read_tokens: Some(0),
                    cache_creation_tokens: Some(30),
                }))
                .await;
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::plain(
                    "# coding output".to_string(),
                    Some("usage-session".to_string()),
                )))
                .await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }
}

fn usage_test_invocation_inputs(
    attempt: &CodingExecutionAttempt,
) -> (AdapterInput, StreamingProviderInput) {
    let worktree = attempt.worktree_path.clone().expect("worktree path");
    let legacy_input = AdapterInput {
        provider_type: ProviderType::Codex,
        role: AdapterRole::Executor,
        worktree_path: Some(worktree.to_string_lossy().to_string()),
        provider_stream_log_dir: None,
        prompt: "usage event wiring evidence".to_string(),
        context_files: Vec::new(),
        output_schema: "coding_workspace_markdown".to_string(),
        timeout: 30,
        max_retries: 0,
    };
    let input = streaming_input_from_adapter(
        &legacy_input,
        worktree,
        crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
    );
    (legacy_input, input)
}

#[tokio::test]
async fn usage_report_maps_to_coding_usage_execution_events() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let role_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::Initial,
            Some("coding_usage_event".to_string()),
        )
        .expect("role run");
    let (event_tx, mut event_rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);
    let provider = UsageReportingProvider;
    let (legacy_input, input) = usage_test_invocation_inputs(&attempt);
    let provider_name = ProviderName::Codex;
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let outcome = engine
        .run_provider_stream_invocation(CodingProviderStreamRun {
            attempt: &attempt,
            node_id: "coding_usage_event",
            role_run: Some(&role_run),
            provider: &provider,
            legacy_input: &legacy_input,
            input,
            provider_name: &provider_name,
            provider_role: CodingProviderRole::Coder,
            command_rx: &mut command_rx,
            allow_legacy_stream_fallback: false,
            timeout: None,
            timeout_reason_code: None,
            suppress_failure_side_effects: false,
            validated_input: None,
        })
        .await;

    assert!(
        matches!(
            &outcome,
            ProviderInvocationOutcome::Completed(ProviderStreamOutcome { full_output, .. })
                if full_output == "# coding output"
        ),
        "usage 采集为 best-effort，不得改变完成语义: {outcome:?}"
    );

    // WS 出站：两次 UsageReport 应各产生一条 kind=usage 的 CodingExecutionEvent。
    let mut usage_events = Vec::new();
    while let Ok(message) = event_rx.try_recv() {
        if let CodingWsOutMessage::CodingExecutionEvent { event } = message
            && event.kind == WsExecutionEventKind::Usage
        {
            usage_events.push(event);
        }
    }
    assert_eq!(
        usage_events.len(),
        2,
        "两次 UsageReport 应各发一条 usage 事件，实际: {usage_events:?}"
    );
    assert!(
        usage_events
            .iter()
            .all(|event| event.event_id == "usage_author"),
        "event_id 必须稳定为 usage_{{role}} 以承载 upsert 覆盖语义"
    );

    let parse_output = |event: &WsExecutionEvent| {
        serde_json::from_str::<serde_json::Value>(
            event.output.as_deref().expect("usage output json"),
        )
        .expect("usage output parses as UsageReportData json")
    };
    let first_snapshot = parse_output(&usage_events[0]);
    assert_eq!(first_snapshot["role"], "author");
    assert_eq!(first_snapshot["input_tokens"], 1200);
    assert_eq!(first_snapshot["output_tokens"], 80);
    assert_eq!(first_snapshot["cache_read_tokens"], 600);
    assert_eq!(
        first_snapshot["cache_creation_tokens"],
        serde_json::Value::Null
    );

    let latest_snapshot = parse_output(&usage_events[1]);
    assert_eq!(usage_events[1].title, "author token usage");
    assert_eq!(latest_snapshot["input_tokens"], 1500);
    assert_eq!(latest_snapshot["output_tokens"], 120);
    assert_eq!(latest_snapshot["cache_creation_tokens"], 30);
    assert_ne!(
        first_snapshot["input_tokens"], latest_snapshot["input_tokens"],
        "最新快照必须区别于旧快照，消费侧按 event_id 覆盖"
    );

    // role-run 审计落盘：usage 事件以 ExecutionEvent 审计记录持久化。
    let persisted = store
        .list_role_run_events(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
        )
        .expect("role run events");
    let usage_audit = persisted
        .iter()
        .filter(|event| {
            event.event_type == CodingRoleRunEventType::ExecutionEvent
                && event.payload["kind"] == "Usage"
                && event.payload["event_id"] == "usage_author"
        })
        .count();
    assert_eq!(
        usage_audit, 2,
        "两次 usage 上报应各落一条 ExecutionEvent 审计记录"
    );
}
