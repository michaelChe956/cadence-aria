use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::protocol::contracts::AdapterInput;
use serde_json::json;

use crate::cross_cutting::structured_output::{StructuredOutputContract, StructuredOutputState};

use super::{
    ChoiceRequestData, ChoiceRequestSource, FakeStreamingProvider, ProviderCommand,
    ProviderCompletion, ProviderEvent, ProviderPermissionMode, ProviderSession, ProviderToolCall,
    ProviderToolResult, StreamingProviderAdapter, StreamingProviderInput,
};

const TEST_TIMEOUT: Duration = Duration::from_secs(1);

fn make_input(prompt: &str) -> AdapterInput {
    AdapterInput {
        prompt: prompt.to_string(),
        provider_type: crate::protocol::contracts::ProviderType::Fake,
        role: crate::protocol::contracts::AdapterRole::Orchestrator,
        worktree_path: None,
        provider_stream_log_dir: None,
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
        structured_output_contract: None,
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
fn provider_completion_parses_requested_structured_output() {
    let contract = StructuredOutputContract {
        nonce: "96aca42f".to_string(),
        schema_name: "workspace_review".to_string(),
    };
    let completion = ProviderCompletion::from_output(
        "可读说明\n<ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">{\"verdict\":\"pass\"}</ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">".to_string(),
        Some(&contract),
        Some("provider-session-1".to_string()),
    );

    assert_eq!(completion.readable_output, "可读说明");
    assert!(matches!(
        completion.structured_output,
        StructuredOutputState::Parsed(_)
    ));
    assert_eq!(
        completion.provider_session_id.as_deref(),
        Some("provider-session-1")
    );
}

#[test]
fn provider_completion_plain_marks_structured_output_not_requested() {
    let completion = ProviderCompletion::plain("plain output", None);

    assert_eq!(completion.full_output, "plain output");
    assert_eq!(completion.readable_output, "plain output");
    assert_eq!(
        completion.structured_output,
        StructuredOutputState::NotRequested
    );
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
        structured_output_contract: None,
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
            ProviderEvent::Completed(completion) => {
                let full_output = completion.full_output;
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
async fn fake_streaming_provider_parses_requested_structured_output() {
    let provider = FakeStreamingProvider;
    let mut input = make_provider_input("请作为 reviewer 审核当前 Workspace 产物。");
    input.role = crate::protocol::contracts::AdapterRole::Reviewer;
    input.structured_output_contract = Some(StructuredOutputContract {
        nonce: "96aca42f".to_string(),
        schema_name: "workspace_review".to_string(),
    });
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("start provider");

    let completion = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit completion")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::Completed(completion) => break completion,
            ProviderEvent::TextDelta { .. } => {}
            other => panic!("unexpected provider event: {other:?}"),
        }
    };

    assert!(matches!(
        completion.structured_output,
        StructuredOutputState::Parsed(ref value) if value["verdict"] == "pass"
    ));
    assert_eq!(completion.readable_output, "审核说明");
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
        structured_output_contract: None,
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
            ProviderEvent::Completed(completion) => {
                let full_output = completion.full_output;
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
            !matches!(event, ProviderEvent::Completed(_)),
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
        .expect("cancel should close the provider command receiver under completed backpressure");
}

#[tokio::test]
async fn fake_streaming_provider_run_streaming_cancel_closes_bridge_when_output_is_backpressured() {
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
                    questions: vec![],
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

async fn collect_scripted_provider_events(mut session: ProviderSession) -> Vec<ProviderEvent> {
    let mut events = Vec::new();
    while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
        .await
        .expect("脚本化 provider 应在超时前结束")
    {
        events.push(event);
    }
    events
}

#[tokio::test]
async fn scripted_fake_provider_matches_prompts_and_emits_role_events_in_order() {
    let author_events = vec![
        ProviderEvent::TextDelta {
            content: "作者草稿".to_string(),
        },
        ProviderEvent::TextDelta {
            content: "作者补充".to_string(),
        },
        ProviderEvent::Completed(ProviderCompletion::plain("作者草稿作者补充", None)),
    ];
    let reviewer_events = vec![
        ProviderEvent::TextDelta {
            content: "审核意见".to_string(),
        },
        ProviderEvent::Failed {
            message: "需要修订".to_string(),
        },
    ];
    let provider = super::ScriptedFakeProvider::new(vec![
        super::ScriptedReply {
            match_prompt_contains: "author".to_string(),
            events: author_events.clone(),
        },
        super::ScriptedReply {
            match_prompt_contains: "reviewer".to_string(),
            events: reviewer_events.clone(),
        },
    ]);

    let author_session = provider
        .start(
            make_provider_input("群聊 author 生成候选稿"),
            CancellationToken::new(),
        )
        .await
        .expect("启动作者脚本");
    let mut reviewer_input = make_provider_input("群聊 reviewer 审核候选稿");
    reviewer_input.role = crate::protocol::contracts::AdapterRole::Reviewer;
    let reviewer_session = provider
        .start(reviewer_input, CancellationToken::new())
        .await
        .expect("启动审核者脚本");

    assert_eq!(
        collect_scripted_provider_events(author_session).await,
        author_events
    );
    assert_eq!(
        collect_scripted_provider_events(reviewer_session).await,
        reviewer_events
    );
}

#[tokio::test]
async fn scripted_fake_provider_uses_default_output_when_prompt_does_not_match() {
    let provider = super::ScriptedFakeProvider::new(vec![super::ScriptedReply {
        match_prompt_contains: "author".to_string(),
        events: vec![ProviderEvent::TextDelta {
            content: "不应匹配".to_string(),
        }],
    }]);
    let session = provider
        .start(
            make_provider_input("Workspace 类型: Story Spec\nIssue: 默认输出\n[user]: 开始生成"),
            CancellationToken::new(),
        )
        .await
        .expect("启动未匹配脚本");

    let events = collect_scripted_provider_events(session).await;
    let output = events
        .iter()
        .filter_map(|event| match event {
            ProviderEvent::TextDelta { content } => Some(content.as_str()),
            _ => None,
        })
        .collect::<String>();

    assert!(output.contains("# Story Spec"));
    assert!(matches!(events.last(), Some(ProviderEvent::Completed(_))));
}

#[tokio::test]
async fn scripted_fake_provider_stops_scripted_events_after_cancel() {
    let provider = super::ScriptedFakeProvider::new(vec![super::ScriptedReply {
        match_prompt_contains: "author".to_string(),
        events: vec![
            ProviderEvent::TextDelta {
                content: "第一段".to_string(),
            },
            ProviderEvent::TextDelta {
                content: "第二段".to_string(),
            },
            ProviderEvent::Completed(ProviderCompletion::plain("第一段第二段", None)),
        ],
    }]);
    let cancel = CancellationToken::new();
    let mut session = provider
        .start(make_provider_input("author 生成候选稿"), cancel.clone())
        .await
        .expect("启动可取消脚本");

    assert_eq!(
        session.events.recv().await,
        Some(ProviderEvent::TextDelta {
            content: "第一段".to_string(),
        })
    );
    cancel.cancel();

    while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
        .await
        .expect("取消后脚本化 provider 应关闭")
    {
        assert!(
            !matches!(event, ProviderEvent::Completed(_)),
            "取消后的脚本不应发送完成事件"
        );
    }
}
