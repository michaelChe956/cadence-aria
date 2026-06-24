use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::protocol::contracts::AdapterInput;
use serde_json::json;

use super::{
    ChoiceRequestData, ChoiceRequestSource, FakeStreamingProvider, ProviderCommand, ProviderEvent,
    ProviderPermissionMode, ProviderSession, ProviderToolCall, ProviderToolResult,
    StreamingProviderAdapter, StreamingProviderInput,
};

const TEST_TIMEOUT: Duration = Duration::from_secs(1);

fn make_input(prompt: &str) -> AdapterInput {
    AdapterInput {
        prompt: prompt.to_string(),
        provider_type: crate::protocol::contracts::ProviderType::Fake,
        role: crate::protocol::contracts::AdapterRole::Orchestrator,
        worktree_path: None,
        context_files: Vec::new(),
        output_schema: String::new(),
        timeout: 60,
        max_retries: 0,
    }
}

fn make_provider_input(prompt: &str) -> StreamingProviderInput {
    StreamingProviderInput {
        provider_type: crate::protocol::contracts::ProviderType::Fake,
        role: crate::protocol::contracts::AdapterRole::Orchestrator,
        prompt: prompt.to_string(),
        working_dir: std::env::current_dir().unwrap(),
        workspace_session_id: None,
        resume_provider_session_id: None,
        permission_mode: ProviderPermissionMode::Auto,
        env_vars: std::collections::BTreeMap::new(),
        timeout_secs: 60,
    }
}

fn prompt_with_word_count(word_count: usize) -> String {
    (0..word_count)
        .map(|index| format!("word{index}"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn streaming_provider_input_distinguishes_workspace_and_resume_sessions() {
    let input = StreamingProviderInput {
        provider_type: crate::protocol::contracts::ProviderType::Fake,
        role: crate::protocol::contracts::AdapterRole::Orchestrator,
        prompt: "prompt".to_string(),
        working_dir: std::env::current_dir().unwrap(),
        workspace_session_id: Some("workspace_session_0001".to_string()),
        resume_provider_session_id: Some("provider_session_0001".to_string()),
        permission_mode: ProviderPermissionMode::Auto,
        env_vars: std::collections::BTreeMap::new(),
        timeout_secs: 60,
    };

    assert_eq!(
        input.workspace_session_id.as_deref(),
        Some("workspace_session_0001")
    );
    assert_eq!(
        input.resume_provider_session_id.as_deref(),
        Some("provider_session_0001")
    );
}

#[test]
fn provider_tool_call_and_result_have_stable_json_shape() {
    let call = ProviderToolCall {
        id: "tool_call_0001".to_string(),
        tool_name: "run_command".to_string(),
        input: json!({"command": ["cargo", "test"]}),
    };
    let result = ProviderToolResult {
        tool_use_id: "tool_call_0001".to_string(),
        output: "{\"status\":\"passed\"}".to_string(),
        is_error: false,
    };

    assert_eq!(
        serde_json::to_value(&call).expect("serialize tool call"),
        json!({
            "id": "tool_call_0001",
            "tool_name": "run_command",
            "input": {"command": ["cargo", "test"]}
        })
    );
    assert_eq!(
        serde_json::from_value::<ProviderToolCall>(
            serde_json::to_value(&call).expect("serialize tool call")
        )
        .expect("deserialize tool call"),
        call
    );
    assert_eq!(
        serde_json::to_value(&result).expect("serialize tool result"),
        json!({
            "tool_use_id": "tool_call_0001",
            "output": "{\"status\":\"passed\"}",
            "is_error": false
        })
    );
    assert_eq!(
        serde_json::from_value::<ProviderToolResult>(
            serde_json::to_value(&result).expect("serialize tool result")
        )
        .expect("deserialize tool result"),
        result
    );
}

async fn wait_for_buffer_len<T>(rx: &mpsc::Receiver<T>, expected_len: usize) {
    for _ in 0..200 {
        if rx.len() >= expected_len {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!(
        "receiver buffer did not reach {expected_len} items; actual len is {}",
        rx.len()
    );
}

async fn wait_for_receiver_closed<T>(rx: &mpsc::Receiver<T>) {
    for _ in 0..200 {
        if rx.is_closed() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("receiver was not closed after cancellation");
}

#[tokio::test]
async fn fake_streaming_provider_emits_chunks_then_done() {
    let provider = FakeStreamingProvider;
    let cancel = CancellationToken::new();
    let input = make_input("Workspace 类型: Story Spec\nIssue: 爬楼梯问题\n[user]: 开始生成");

    let mut rx = provider.run_streaming(&input, cancel).await.unwrap();

    let mut output = String::new();
    let mut done_output = None;

    while let Some(chunk) = rx.recv().await {
        match chunk {
            super::StreamChunk::Text(t) => output.push_str(&t),
            super::StreamChunk::Done { full_output } => {
                done_output = Some(full_output);
                break;
            }
            super::StreamChunk::Error(_) => panic!("unexpected error"),
        }
    }

    let done_output = done_output.unwrap();
    assert_eq!(output, done_output);
    assert!(done_output.contains("## 范围"));
    assert!(done_output.contains("## 用户故事"));
    assert!(done_output.contains("## 功能需求"));
    assert!(done_output.contains("[REQ-001]"));
    assert!(done_output.contains("## 成功标准"));
    assert!(done_output.contains("[AC-001]"));
    assert!(done_output.contains("## 待确认项"));
    assert!(done_output.contains("## 非功能需求"));
    assert!(
        !done_output.contains("[system]"),
        "fake provider should generate a candidate artifact instead of echoing full prompt"
    );
}

#[tokio::test]
async fn fake_streaming_provider_session_emits_text_and_completed() {
    let provider = FakeStreamingProvider;
    let cancel = CancellationToken::new();
    let input = make_provider_input(
        "[system]\nWorkspace 类型: Story Spec\nIssue: 爬楼梯问题\n[user]: 开始生成",
    );

    let mut session = provider.start(input, cancel).await.unwrap();
    let mut output = String::new();
    while let Some(event) = session.events.recv().await {
        match event {
            ProviderEvent::TextDelta { content } => output.push_str(&content),
            ProviderEvent::Completed { full_output, .. } => {
                assert_eq!(full_output, output);
                break;
            }
            other => panic!("unexpected provider event: {other:?}"),
        }
    }
    assert!(output.contains("## 范围"));
    assert!(output.contains("[REQ-001]"));
    assert!(output.contains("[AC-001]"));
    assert!(!output.contains("[system]"));
}

#[tokio::test]
async fn fake_streaming_provider_outputs_work_item_split_sentinel() {
    let provider = FakeStreamingProvider;
    let input = StreamingProviderInput {
        provider_type: crate::protocol::contracts::ProviderType::Fake,
        role: crate::protocol::contracts::AdapterRole::WorkItemSplitter,
        prompt: "你是 Aria 的 Work Item Splitter".to_string(),
        working_dir: std::env::current_dir().unwrap(),
        workspace_session_id: Some("workspace_session_0001".to_string()),
        resume_provider_session_id: None,
        permission_mode: ProviderPermissionMode::Supervised,
        env_vars: std::collections::BTreeMap::new(),
        timeout_secs: 60,
    };

    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();
    let mut streamed = String::new();
    let mut completed = None;
    while let Some(event) = session.events.recv().await {
        match event {
            ProviderEvent::TextDelta { content } => streamed.push_str(&content),
            ProviderEvent::Completed { full_output, .. } => {
                completed = Some(full_output);
                break;
            }
            other => panic!("unexpected provider event: {other:?}"),
        }
    }

    let full_output = completed.expect("completed output");
    assert!(streamed.contains("Fake Work Item Plan streaming draft"));
    assert!(full_output.contains("<ARIA_STRUCTURED_OUTPUT>"));
    assert!(full_output.contains("\"work_items\""));
    assert!(full_output.contains("\"target_context_k\""));
}

#[tokio::test]
async fn fake_streaming_provider_abort_after_final_text_suppresses_completed() {
    let provider = FakeStreamingProvider;
    let cancel = CancellationToken::new();
    let input = make_provider_input("Issue: final");

    let mut session = provider.start(input, cancel).await.unwrap();
    let first = session.events.recv().await.unwrap();
    assert!(matches!(first, ProviderEvent::TextDelta { .. }));

    let _ = session.commands.send(ProviderCommand::Abort).await;

    while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
        .await
        .expect("provider should close after abort")
    {
        assert!(
            !matches!(event, ProviderEvent::Completed { .. }),
            "abort after the final text delta should suppress completion"
        );
    }
}

#[tokio::test]
async fn fake_streaming_provider_cancel_closes_commands_when_completed_is_backpressured() {
    let provider = FakeStreamingProvider;
    let cancel = CancellationToken::new();
    let prompt = prompt_with_word_count(32);
    let session = provider
        .start(make_provider_input(&prompt), cancel.clone())
        .await
        .unwrap();

    wait_for_buffer_len(&session.events, 6).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    cancel.cancel();

    tokio::time::timeout(TEST_TIMEOUT, session.commands.closed())
        .await
        .expect(
            "cancel should close the provider command receiver under completed backpressure",
        );
}

#[tokio::test]
async fn fake_streaming_provider_run_streaming_cancel_closes_bridge_when_output_is_backpressured()
{
    let provider = FakeStreamingProvider;
    let cancel = CancellationToken::new();
    let prompt = prompt_with_word_count(80);
    let input = make_input(&prompt);
    let rx = provider
        .run_streaming(&input, cancel.clone())
        .await
        .unwrap();

    wait_for_buffer_len(&rx, 6).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    cancel.cancel();

    wait_for_receiver_closed(&rx).await;
}

#[tokio::test]
async fn fake_streaming_provider_cancel_stops_output() {
    let provider = FakeStreamingProvider;
    let cancel = CancellationToken::new();
    let input = make_input("a b c d e f g h i j");

    let mut rx = provider
        .run_streaming(&input, cancel.clone())
        .await
        .unwrap();

    let first = rx.recv().await.unwrap();
    assert!(matches!(first, super::StreamChunk::Text(_)));
    cancel.cancel();

    for _ in 0..9 {
        let Some(chunk) = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
            .await
            .expect("provider should close after cancel")
        else {
            return;
        };
        assert!(
            !matches!(chunk, super::StreamChunk::Done { .. }),
            "cancelled provider should not emit a completion marker"
        );
    }

    panic!("cancelled provider should close before emitting the full stream");
}

use async_trait::async_trait;

struct ChoiceEmittingProvider;

#[async_trait]
impl StreamingProviderAdapter for ChoiceEmittingProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, crate::cross_cutting::provider_adapter::ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::ChoiceRequest(ChoiceRequestData {
                    id: "choice_001".to_string(),
                    prompt: "Continue?".to_string(),
                    options: vec![],
                    allow_multiple: false,
                    allow_free_text: true,
                    source: ChoiceRequestSource::AskUserQuestion,
                }))
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[tokio::test]
async fn run_streaming_declines_choice_request_instead_of_hanging() {
    let provider = ChoiceEmittingProvider;
    let mut rx = provider
        .run_streaming(&make_input("test"), CancellationToken::new())
        .await
        .unwrap();

    let chunk = tokio::time::timeout(Duration::from_secs(1), rx.recv())
        .await
        .expect("run_streaming 不应在 ChoiceRequest 上挂起")
        .expect("stream 应该发出错误块");

    assert!(
        matches!(chunk, super::StreamChunk::Error(ref msg) if msg.contains("choice")),
        "expected error chunk, got {chunk:?}"
    );
}
