use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::protocol::contracts::AdapterInput;
use serde_json::json;

use crate::cross_cutting::structured_output::{StructuredOutputContract, StructuredOutputState};

use super::{
    ChoiceRequestData, ChoiceRequestSource, FakeStreamingProvider, FakeStreamingProviderInput,
    ProviderCommand, ProviderCompletion, ProviderEvent, ProviderPermissionMode, ProviderSession,
    ProviderToolCall, ProviderToolPolicy, ProviderToolResult, StreamingProviderAdapter,
    StreamingProviderInput,
};

const TEST_TIMEOUT: Duration = Duration::from_secs(1);

fn make_input(prompt: &str) -> AdapterInput {
    AdapterInput {
        working_directory: None,
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
        working_directory: None,
        baseline_tree: None,
        tool_policy: None,
        audit_sink: None,
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
        "可读说明\n<ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">{\"nonce\":\"96aca42f\",\"verdict\":\"pass\"}</ARIA_STRUCTURED_OUTPUT>".to_string(),
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
fn single_candidate_reviewer_completion_discards_nonce_preamble() {
    let contract = StructuredOutputContract {
        nonce: "96aca42f".to_string(),
        schema_name: "single_candidate_work_item_plan_review".to_string(),
    };
    let completion = ProviderCompletion::from_output(
        "路由回执\n<ARIA_STRUCTURED_OUTPUT nonce=\"96aca42f\">{\"nonce\":\"96aca42f\",\"verdict\":\"pass\"}</ARIA_STRUCTURED_OUTPUT>".to_string(),
        Some(&contract),
        Some("provider-session-1".to_string()),
    );

    assert_eq!(completion.readable_output, "");
    assert_eq!(
        completion.structured_output,
        StructuredOutputState::Parsed(json!({"verdict": "pass"}))
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
        working_directory: None,
        baseline_tree: None,
        tool_policy: None,
        audit_sink: None,
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
        working_directory: None,
        baseline_tree: None,
        tool_policy: None,
        audit_sink: None,
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
    assert!(full_output.contains("<ARIA_STRUCTURED_OUTPUT nonce=\"FAKE0001\">"));
    assert!(full_output.contains("\"nonce\":\"FAKE0001\""));
    assert!(full_output.contains("</ARIA_STRUCTURED_OUTPUT>"));
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
            native_session_id: None,
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

// ---- F3 修复轮 P2-1：legacy bridge 按角色派生策略时必须同时注入 sink ----

struct LegacySinkHookProvider {
    sink: std::sync::Arc<dyn crate::cross_cutting::tool_policy_audit::ToolPolicyAuditSink>,
    saw_policy: std::sync::atomic::AtomicBool,
    saw_sink: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for LegacySinkHookProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, crate::cross_cutting::provider_adapter::ProviderAdapterError> {
        use std::sync::atomic::Ordering;
        self.saw_policy
            .store(input.tool_policy.is_some(), Ordering::SeqCst);
        self.saw_sink
            .store(input.audit_sink.is_some(), Ordering::SeqCst);
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(4);
        tokio::spawn(async move {
            let _ = event_tx
                .send(
                    crate::cross_cutting::streaming_provider::ProviderEvent::Completed(
                        crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                            "legacy sink probe done",
                            None,
                        ),
                    ),
                )
                .await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }

    fn legacy_tool_policy_audit_sink(
        &self,
    ) -> Option<std::sync::Arc<dyn crate::cross_cutting::tool_policy_audit::ToolPolicyAuditSink>>
    {
        Some(self.sink.clone())
    }
}

#[tokio::test]
async fn run_streaming_bridge_keeps_sink_and_derives_no_deny() {
    use crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink;
    use std::sync::atomic::Ordering;

    let provider = LegacySinkHookProvider {
        sink: RecordingToolPolicyAuditSink::new(),
        saw_policy: std::sync::atomic::AtomicBool::new(false),
        saw_sink: std::sync::atomic::AtomicBool::new(false),
    };
    // 批2 Task 11：legacy bridge 不再按 AdapterRole 一概派生 deny（Author
    // 岗位投影的唯一铸造点=受信装配；Coder/Reviewer/规划族 wire 面全开），
    // 但 engine hook 提供的 run-bound sink 仍必须注入——Reviewer 执行审计
    // 不随去 deny 丢失。
    let mut rx = provider
        .run_streaming(&make_input("legacy sink"), CancellationToken::new())
        .await
        .expect("legacy bridge run");
    while let Some(chunk) = rx.recv().await {
        if matches!(chunk, super::StreamChunk::Done { .. }) {
            break;
        }
    }
    assert!(
        !provider.saw_policy.load(Ordering::SeqCst),
        "legacy bridge must not derive the deny policy from AdapterRole (T11 matrix)"
    );
    assert!(
        provider.saw_sink.load(Ordering::SeqCst),
        "legacy bridge must still inject the engine-provided audit sink"
    );
}

/// 批2 Task 11：legacy bridge 对全部 AdapterRole 一律不派生 deny——
/// 表驱动覆盖五角色（Orchestrator/WorkItemSplitter/Reviewer/Executor/
/// Handoff），deny 只能由受信装配（Author 岗位投影）铸造。
#[tokio::test]
async fn run_streaming_bridge_derives_no_policy_for_any_role() {
    use crate::protocol::contracts::AdapterRole;

    for role in [
        AdapterRole::Orchestrator,
        AdapterRole::WorkItemSplitter,
        AdapterRole::Reviewer,
        AdapterRole::Executor,
        AdapterRole::Handoff,
    ] {
        let provider = LegacySinkHookProvider {
            sink: crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink::new(),
            saw_policy: std::sync::atomic::AtomicBool::new(false),
            saw_sink: std::sync::atomic::AtomicBool::new(false),
        };
        let mut input = make_input("legacy role matrix");
        input.role = role.clone();
        let mut rx = provider
            .run_streaming(&input, CancellationToken::new())
            .await
            .expect("legacy bridge run");
        while let Some(chunk) = rx.recv().await {
            if matches!(chunk, super::StreamChunk::Done { .. }) {
                break;
            }
        }
        assert!(
            !provider.saw_policy.load(std::sync::atomic::Ordering::SeqCst),
            "legacy bridge must not derive deny for {role:?} (T11 matrix)"
        );
    }
}

#[test]
fn fake_launch_is_constructible_only_through_fake_registry_path() {
    // Task 12:Fake launch input 必须经 `FakeStreamingProviderInput::for_test` 构造,
    // 且强制 `ProviderType::Fake`。这是「Fake 经过 registry/编译期隔离」的 type-level
    // 断言:逻辑代码库真实 provider 启动路径不接受裸 StreamingProviderInput,而
    // Fake 用此 wrapper 明确标记为测试隔离,而非通过运行时 `if provider != Fake`
    // 例外放行真实 provider。
    let launch = FakeStreamingProviderInput::for_test(
        crate::protocol::contracts::ProviderType::Fake,
        "fixture",
    );
    assert_eq!(
        launch.provider_type(),
        &crate::protocol::contracts::ProviderType::Fake
    );
}

#[test]
#[should_panic(expected = "仅接受 ProviderType::Fake")]
fn fake_launch_for_test_panics_on_non_fake_provider_type() {
    // 非真实 provider 类型不得伪装成 Fake launch input。
    FakeStreamingProviderInput::for_test(
        crate::protocol::contracts::ProviderType::Codex,
        "disguised",
    );
}

#[test]
fn usage_report_serde_roundtrip_snake_case() {
    use super::UsageReportData;

    let report = UsageReportData {
        role: "author".to_string(),
        input_tokens: Some(120),
        output_tokens: Some(34),
        cache_read_tokens: Some(0),
        cache_creation_tokens: None,
    };
    let json = serde_json::to_value(&report).expect("serialize usage report");
    assert_eq!(
        json,
        serde_json::json!({
            "role": "author",
            "input_tokens": 120,
            "output_tokens": 34,
            "cache_read_tokens": 0,
            "cache_creation_tokens": null,
        })
    );
    let parsed: UsageReportData = serde_json::from_value(json).expect("deserialize usage report");
    assert_eq!(parsed, report);
    assert!(parsed.has_any_tokens());
}

#[test]
fn usage_report_role_text_maps_reviewer_and_author() {
    use super::UsageReportData;
    use crate::protocol::contracts::AdapterRole;

    assert_eq!(
        UsageReportData::role_text(&AdapterRole::Reviewer),
        "reviewer"
    );
    assert_eq!(UsageReportData::role_text(&AdapterRole::Executor), "author");
    assert_eq!(
        UsageReportData::role_text(&AdapterRole::Orchestrator),
        "author"
    );
}

#[test]
fn canonical_tool_policy_uses_tp_v1_provider_tokens_and_ap_v1() {
    use super::{ProviderToolPolicy, ToolPolicyIntent, canonical_tool_policy};

    let policy = ProviderToolPolicy {
        intent: ToolPolicyIntent::DenyFileWriteBuiltins,
    };
    let actual = canonical_tool_policy("pi", &policy).unwrap();
    assert_eq!(actual.provider, "pi");
    assert_eq!(actual.tokens, vec!["--exclude-tools", "edit,write"]);
    assert_eq!(actual.approval_policy_version, "ap-v1");
    // 实算命令（禁手写假 digest，物理片段/审批规则变化必须改变 digest）：
    // printf 'tp-v1\x1fpi\x1f--exclude-tools\x1fedit,write\x1fap-v1' | sha256sum
    assert_eq!(
        actual.digest,
        "06ad73691367e2029b7c6c172532ff724eb4604040165ef244c17eae5c66ef91"
    );
    // 同一输入 digest 穷定：重复计算必须逐字节一致。
    let again = canonical_tool_policy("pi", &policy).unwrap();
    assert_eq!(actual.digest, again.digest);
}

#[test]
fn canonical_tool_policy_freezes_provider_physical_fragments() {
    use super::{ProviderToolPolicy, canonical_tool_policy, translate_tool_policy};

    let policy = ProviderToolPolicy::deny_file_write_builtins();
    // claude 实算命令：
    // printf 'tp-v1\x1fclaude-code\x1f--disallowedTools\x1fEdit,Write,NotebookEdit\x1fap-v1' | sha256sum
    let claude = canonical_tool_policy("claude-code", &policy).unwrap();
    assert_eq!(
        claude.tokens,
        vec!["--disallowedTools", "Edit,Write,NotebookEdit"]
    );
    assert_eq!(
        claude.digest,
        "4d6a4ddf803656ce6e298888beec1980671fc40fdb5bd92aaabbf0dba4bff9bf"
    );
    // codex 实算命令：
    // printf 'tp-v1\x1fcodex\x1fsandbox=read-only\x1fapprovalPolicy=on-request\x1fap-v1' | sha256sum
    let codex = canonical_tool_policy("codex", &policy).unwrap();
    assert!(codex.tokens.contains(&"sandbox=read-only".to_string()));
    assert!(
        codex
            .tokens
            .contains(&"approvalPolicy=on-request".to_string())
    );
    assert_eq!(
        codex.digest,
        "55589d94224ef120ba52681e12ac9a9f0a940c600f094a6a49d9fcd8eac48315"
    );

    // translator 与 canonical tokens 同源：argv flag/value 按出现顺序原样、大小写保留。
    assert_eq!(
        translate_tool_policy("pi", &policy).unwrap(),
        vec!["--exclude-tools", "edit,write"]
    );
    assert_eq!(
        translate_tool_policy("claude-code", &policy).unwrap(),
        vec!["--disallowedTools", "Edit,Write,NotebookEdit"]
    );
    // 未知 provider fail-closed（kimi 不接策略，不得静默翻译成空片段）。
    assert!(canonical_tool_policy("kimi-code", &policy).is_err());
    assert!(translate_tool_policy("kimi-code", &policy).is_err());
}

#[test]
fn canonical_tool_policy_digest_drifts_on_fragment_case_order_and_approval_version() {
    use super::tool_policy_digest;

    // 基线 = pi 冻结片段 + ap-v1（与实算向量测试同源）。
    let base = tool_policy_digest(
        "pi",
        &["--exclude-tools".to_string(), "edit,write".to_string()],
        "ap-v1",
    );
    // 物理片段漂移：换成 claude denylist 必须改变 digest。
    assert_ne!(
        base,
        tool_policy_digest(
            "pi",
            &["--disallowedTools".to_string(), "Edit,Write".to_string(),],
            "ap-v1",
        )
    );
    // 大小写漂移：名单大小写是冻结语义，变化必须改变 digest。
    assert_ne!(
        base,
        tool_policy_digest(
            "pi",
            &["--exclude-tools".to_string(), "Edit,Write".to_string(),],
            "ap-v1",
        )
    );
    // token 顺序漂移：flag/value 顺序是规范输入一部分。
    assert_ne!(
        base,
        tool_policy_digest(
            "pi",
            &["edit,write".to_string(), "--exclude-tools".to_string(),],
            "ap-v1",
        )
    );
    // 审批规则版本漂移：ap-v1 → ap-v2 必须改变 digest（Codex 审批规则升级即升版本）。
    assert_ne!(
        base,
        tool_policy_digest(
            "pi",
            &["--exclude-tools".to_string(), "edit,write".to_string(),],
            "ap-v2",
        )
    );
    // provider 名漂移：同片段不同 provider 不得碰撞。
    assert_ne!(
        base,
        tool_policy_digest(
            "codex",
            &["--exclude-tools".to_string(), "edit,write".to_string(),],
            "ap-v1",
        )
    );
}

/// F3 Task 4.1 deferred minor②（前轮 Task 1 Minor）：codex translator 补 canonical
/// token 全序列一致性断言（完整向量比对，顺序即契约；canonical 与 translator 同源）。
#[test]
fn translate_tool_policy_freezes_full_canonical_token_sequence() {
    use super::{ProviderToolPolicy, canonical_tool_policy, translate_tool_policy};

    let policy = ProviderToolPolicy::deny_file_write_builtins();
    let frozen = vec![
        "sandbox=read-only".to_string(),
        "approvalPolicy=on-request".to_string(),
    ];

    // canonical：全序列等值（不再 contains 式部分匹配，多/少/乱序 token 均红）。
    let codex = canonical_tool_policy("codex", &policy).unwrap();
    assert_eq!(
        codex.tokens, frozen,
        "codex canonical tokens must be the exact frozen sequence"
    );

    // translator 与 canonical tokens 同源：argv/参数原文按出现顺序、大小写保留。
    assert_eq!(translate_tool_policy("codex", &policy).unwrap(), frozen);
    assert_eq!(
        translate_tool_policy("pi", &policy).unwrap(),
        vec!["--exclude-tools".to_string(), "edit,write".to_string()]
    );
    assert_eq!(
        translate_tool_policy("claude-code", &policy).unwrap(),
        vec![
            "--disallowedTools".to_string(),
            "Edit,Write,NotebookEdit".to_string()
        ]
    );

    // 三 provider 的 canonical 与 translator 全序列一一互等（同源锁定）。
    for provider in ["pi", "claude-code", "codex"] {
        assert_eq!(
            canonical_tool_policy(provider, &policy).unwrap().tokens,
            translate_tool_policy(provider, &policy).unwrap(),
            "canonical tokens and translator fragments must stay identical for {provider}"
        );
    }
}

#[test]
fn usage_report_without_any_tokens_is_not_reportable() {
    use super::UsageReportData;

    let empty = UsageReportData {
        role: "reviewer".to_string(),
        input_tokens: None,
        output_tokens: None,
        cache_read_tokens: None,
        cache_creation_tokens: None,
    };
    assert!(!empty.has_any_tokens());
}

// ---- 批2 Task 11（REQ-ENV-09/B6 公共角色矩阵）：deny 退役为 Author 岗位投影 ----

#[test]
fn adapter_tool_policy_guard_is_bidirectional_for_every_role() {
    use super::validate_tool_policy_for_role;
    use crate::protocol::contracts::AdapterRole;

    let deny = ProviderToolPolicy::deny_file_write_builtins();
    // 全角色 wire 面默认全开（None 合法）：Coder/Reviewer/规划族不再按
    // AdapterRole 一概套 deny——纪律移入受信冻结合同（Author 岗位策略/
    // Reviewer 窗口合同）。
    for role in [
        AdapterRole::Orchestrator,
        AdapterRole::WorkItemSplitter,
        AdapterRole::Reviewer,
        AdapterRole::Executor,
        AdapterRole::Handoff,
    ] {
        assert!(
            validate_tool_policy_for_role(&role, None).is_ok(),
            "role {role:?} without policy must be accepted (deny matrix retired by T11)"
        );
    }
    // DenyFileWriteBuiltins = Author 岗位纪律的唯一 wire 物理投影，只在
    // Author 可承载的角色（Orchestrator=story/design/revision、
    // WorkItemSplitter=plan authoring）合法；Coder（Executor）、Reviewer、
    // Handoff 携带 deny 一律拒绝（Coder 不带 deny；Reviewer 只携
    // ReviewerVerify+窗口合同，不回退旧 deny fallback）。
    for role in [AdapterRole::Orchestrator, AdapterRole::WorkItemSplitter] {
        assert!(
            validate_tool_policy_for_role(&role, Some(&deny)).is_ok(),
            "author carrier {role:?} with deny projection must be accepted"
        );
    }
    for role in [
        AdapterRole::Reviewer,
        AdapterRole::Executor,
        AdapterRole::Handoff,
    ] {
        assert!(
            validate_tool_policy_for_role(&role, Some(&deny)).is_err(),
            "non-author role {role:?} carrying deny must be rejected (T11 matrix)"
        );
    }
}

/// 批2 Task 11 表驱动：六职责→wire 投影权威矩阵（`tool_policy_for_duty`）
/// ——Author 岗位纪律唯一带 deny；Coder/Orchestrator/Splitter/Handoff/
/// Reviewer 全开（Reviewer 只携 ReviewerVerify+窗口合同）。
#[test]
fn duty_tool_policy_matrix_is_author_only() {
    use crate::cross_cutting::provider_execution::TrustedProviderRole;
    use crate::cross_cutting::streaming_provider::tool_policy_for_duty;

    let deny = ProviderToolPolicy::deny_file_write_builtins();
    assert_eq!(
        tool_policy_for_duty(&TrustedProviderRole::Author),
        Some(deny),
        "Author duty carries the deny projection (Claude Edit/Write/NotebookEdit, Pi edit/write)"
    );
    for duty in [
        TrustedProviderRole::Orchestrator,
        TrustedProviderRole::Splitter,
        TrustedProviderRole::Handoff,
        TrustedProviderRole::Coder,
        TrustedProviderRole::Reviewer,
    ] {
        assert_eq!(
            tool_policy_for_duty(&duty),
            None,
            "duty {duty:?} must keep the wire fully open (T11 matrix)"
        );
    }
}

/// 守卫必须先于子进程创建：用不存在的 CLI 路径区分「守卫拒绝」与「spawn 失败」。
/// 若守卫位于 spawn 之前，返回错误是 tool-policy guard 文案；否则是进程启动错误。
#[tokio::test]
async fn tool_policy_guard_rejects_invalid_role_policy_before_spawn() {
    use crate::cross_cutting::claude_code_provider::ClaudeCodeProvider;
    use crate::cross_cutting::codex_provider::CodexProvider;
    use crate::cross_cutting::pi_provider::PiProvider;
    use crate::protocol::contracts::{AdapterRole, ProviderType};

    async fn expect_guard_rejection(
        provider: &str,
        adapter: &dyn StreamingProviderAdapter,
        input: StreamingProviderInput,
    ) {
        match adapter.start(input, CancellationToken::new()).await {
            Ok(_) => panic!("{provider}: expected launch guard rejection before spawn"),
            Err(error) => {
                assert!(
                    error.details.contains("tool policy guard"),
                    "{provider}: rejection must be the launch guard, got: {}",
                    error.details
                );
            }
        }
    }

    let missing_cli = std::path::PathBuf::from("/nonexistent/aria-tool-policy-guard-probe-cli");

    let base_input = |provider_type: ProviderType,
                      role: AdapterRole,
                      tool_policy: Option<ProviderToolPolicy>| {
        StreamingProviderInput {
            working_directory: None,
            baseline_tree: None,
            tool_policy,
            audit_sink: None,
            provider_type,
            role,
            prompt: "guard probe".to_string(),
            working_dir: std::env::temp_dir(),
            workspace_session_id: None,
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Auto,
            structured_output_contract: None,
            env_vars: std::collections::BTreeMap::new(),
            timeout_secs: 5,
        }
    };

    // 批2 Task 11：非 Author 承载角色误带 deny 投影——spawn 前拒绝
    // （Coder 不带 deny；Reviewer 只携窗口合同，不回退旧 deny fallback）。
    expect_guard_rejection(
        "pi",
        &PiProvider::new(missing_cli.clone()),
        base_input(
            ProviderType::Pi,
            AdapterRole::Reviewer,
            Some(ProviderToolPolicy::deny_file_write_builtins()),
        ),
    )
    .await;
    expect_guard_rejection(
        "claude",
        &ClaudeCodeProvider::new(missing_cli.clone()),
        base_input(
            ProviderType::ClaudeCode,
            AdapterRole::Handoff,
            Some(ProviderToolPolicy::deny_file_write_builtins()),
        ),
    )
    .await;
    expect_guard_rejection(
        "codex",
        &CodexProvider::new(missing_cli.clone()),
        base_input(
            ProviderType::Codex,
            AdapterRole::Reviewer,
            Some(ProviderToolPolicy::deny_file_write_builtins()),
        ),
    )
    .await;

    // 非策略角色误带策略（Executor+deny）：spawn 前拒绝。
    expect_guard_rejection(
        "pi",
        &PiProvider::new(missing_cli.clone()),
        base_input(
            ProviderType::Pi,
            AdapterRole::Executor,
            Some(ProviderToolPolicy::deny_file_write_builtins()),
        ),
    )
    .await;
    expect_guard_rejection(
        "claude",
        &ClaudeCodeProvider::new(missing_cli.clone()),
        base_input(
            ProviderType::ClaudeCode,
            AdapterRole::Executor,
            Some(ProviderToolPolicy::deny_file_write_builtins()),
        ),
    )
    .await;
    expect_guard_rejection(
        "codex",
        &CodexProvider::new(missing_cli),
        base_input(
            ProviderType::Codex,
            AdapterRole::Executor,
            Some(ProviderToolPolicy::deny_file_write_builtins()),
        ),
    )
    .await;
}

/// Task 3.2（D1/REQ-BOOT-04）：三 adapter spawn 边界的 BootstrapExecutor
/// marker 消费锁。marker 四要素（credential、bootstrap action、canonical
/// root、receipt context）联合证明的完整自举通道在真实 adapter `start`
/// 首步被消费：
/// - Executor + 完整 marker：唯一放行面——守卫通过，失败转移到子进程
///   启动（错误不再是 tool-policy guard 文案）；
/// - Reviewer + marker：marker 不是策略替代品，spawn 前 PolicyRequired；
/// - Handoff + marker：非策略角色不得使用自举通道，spawn 前
///   PolicyForbidden。
/// 构造面（空 receipt context/空 root/非写 action）与退化 marker 的
/// guard 函数级拒绝已由
/// `bootstrap_executor_without_credential_or_receipt_context_is_rejected`
/// 锁定，此处只锁三 adapter 的真实 spawn 边界面。
#[tokio::test]
async fn bootstrap_executor_requires_credential_and_receipt_context() {
    use crate::cross_cutting::claude_code_provider::ClaudeCodeProvider;
    use crate::cross_cutting::codex_provider::CodexProvider;
    use crate::cross_cutting::pi_provider::PiProvider;
    use crate::product::logical_codebase::aggregate_initialization::AggregateInitializationStepKind;
    use crate::product::logical_codebase::policy::SessionPolicyAction;
    use crate::product::logical_codebase::provider_admission_preflight::{
        BootstrapExecutorMarker, BootstrapPhaseCredential,
    };
    use crate::protocol::contracts::{AdapterRole, ProviderType};
    use std::sync::Arc;

    use super::ToolPolicyIntent;

    let canonical_root = std::path::PathBuf::from("/tmp/aria-bootstrap-marker-root");
    let credential = BootstrapPhaseCredential::for_test(
        "project_0001",
        "logical_codebase_0001",
        "aggregate_initialization_0001",
        AggregateInitializationStepKind::PreCheck,
        "sha256:test-input-digest",
        canonical_root.clone(),
    );
    let marker = BootstrapExecutorMarker::new(
        credential,
        SessionPolicyAction::CodingTargetWrite,
        canonical_root,
        "root-recipe:pre_check:command-1",
    )
    .expect("complete bootstrap executor marker");
    let marker_policy = ProviderToolPolicy {
        intent: ToolPolicyIntent::BootstrapExecutorMarker(marker),
    };

    let missing_cli = std::path::PathBuf::from("/nonexistent/aria-bootstrap-marker-probe-cli");
    let adapters: [(&str, Arc<dyn StreamingProviderAdapter>, ProviderType); 3] = [
        (
            "claude",
            Arc::new(ClaudeCodeProvider::new(missing_cli.clone())),
            ProviderType::ClaudeCode,
        ),
        (
            "codex",
            Arc::new(CodexProvider::new(missing_cli.clone())),
            ProviderType::Codex,
        ),
        (
            "pi",
            Arc::new(PiProvider::new(missing_cli)),
            ProviderType::Pi,
        ),
    ];

    let input = |provider_type: &ProviderType, role: AdapterRole| StreamingProviderInput {
        working_directory: None,
        baseline_tree: None,
        tool_policy: Some(marker_policy.clone()),
        audit_sink: None,
        provider_type: provider_type.clone(),
        role,
        prompt: "bootstrap marker probe".to_string(),
        working_dir: std::env::temp_dir(),
        workspace_session_id: None,
        resume_provider_session_id: None,
        permission_mode: crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
        structured_output_contract: None,
        env_vars: std::collections::BTreeMap::new(),
        timeout_secs: 5,
    };

    for (provider, adapter, provider_type) in adapters
        .iter()
        .map(|(name, adapter, provider_type)| (*name, adapter.clone(), provider_type.clone()))
    {
        // Executor + 完整 marker：守卫放行，失败必须是子进程启动而非守卫拒绝。
        match adapter
            .start(
                input(&provider_type, AdapterRole::Executor),
                CancellationToken::new(),
            )
            .await
        {
            Ok(_) => panic!("{provider}: missing CLI must fail the spawn after the guard"),
            Err(error) => {
                assert!(
                    !error.details.contains("tool policy guard"),
                    "{provider}: complete marker must pass the spawn guard, got: {}",
                    error.details
                );
            }
        }

        // 策略角色（Reviewer）携带 marker：spawn 前 PolicyRequired。
        match adapter
            .start(
                input(&provider_type, AdapterRole::Reviewer),
                CancellationToken::new(),
            )
            .await
        {
            Ok(_) => panic!("{provider}: reviewer with marker must be rejected before spawn"),
            Err(error) => {
                assert!(
                    error.details.contains("tool policy guard"),
                    "{provider}: reviewer must not substitute the marker for its policy, got: {}",
                    error.details
                );
            }
        }

        // Handoff 携带 marker：spawn 前 PolicyForbidden。
        match adapter
            .start(
                input(&provider_type, AdapterRole::Handoff),
                CancellationToken::new(),
            )
            .await
        {
            Ok(_) => panic!("{provider}: handoff with marker must be rejected before spawn"),
            Err(error) => {
                assert!(
                    error.details.contains("tool policy guard"),
                    "{provider}: handoff must not use the bootstrap channel, got: {}",
                    error.details
                );
            }
        }
    }
}
// ---------------------------------------------------------------------------
// Task 7(lcg_t07):tool-policy 双向 guard 的 LC validated 面与 root recipe
// marker 相位语义。
// ---------------------------------------------------------------------------

/// Task 7 的 LC guard fixture:以最小 gateway 装配产出真实
/// `ValidatedSessionLaunchPolicy`(Claude/Pi 可 validate;Codex 被 gateway
/// 路由级 danger 门阻断,由测试按「guard 先于 adapter 匹配」复用 Claude
/// policy 驱动)。
struct LcT07GuardFixture {
    _root: tempfile::TempDir,
    paths: crate::product::app_paths::ProductAppPaths,
}

impl LcT07GuardFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("lc t07 fixture root");
        let paths = crate::product::app_paths::ProductAppPaths::new(root.path().to_path_buf());
        let manifest = crate::product::logical_codebase::store::LogicalCodebaseManifest::new(
            "project_0001",
            root.path().to_path_buf(),
            vec![],
        );
        crate::product::logical_codebase::AggregatePolicyArtifactStore::new(paths.clone())
            .ensure_bootstrap(&manifest)
            .expect("install lc bootstrap policy");
        Self { _root: root, paths }
    }

    fn canonical_root(&self) -> std::path::PathBuf {
        std::fs::canonicalize(self.paths.root()).expect("lc t07 fixture root exists")
    }

    fn target_worktree(&self) -> std::path::PathBuf {
        let worktree = self.paths.root().join("member-worktree");
        std::fs::create_dir_all(&worktree).expect("create member worktree");
        worktree
    }

    fn coding_request(
        &self,
        provider_type: crate::product::logical_codebase::provider_gateway::ProviderRefType,
    ) -> crate::product::logical_codebase::SessionLaunchRequest {
        let provider_ref = crate::product::logical_codebase::provider_gateway::ProviderRef {
            provider_type,
            capability_snapshot_ref: "cap-lcg-t07".to_string(),
        };
        let worktree = self.target_worktree();
        crate::product::logical_codebase::SessionLaunchRequest {
            project_id: "project_0001".to_string(),
            provider: provider_ref,
            action: crate::product::logical_codebase::SessionPolicyAction::CodingTargetWrite,
            target: crate::product::logical_codebase::PolicyTarget::checkout(
                "logical_repo_0001",
                "checkout_0001",
                worktree.clone(),
            ),
            working_directory: self.canonical_root(),
            readable_roots: vec![self.canonical_root()],
            writable_roots: vec![worktree],
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        }
    }

    fn gateway(&self) -> crate::product::logical_codebase::LogicalCodebaseProviderGateway {
        use crate::product::logical_codebase::LogicalCodebaseProviderGateway;
        let registry = crate::cross_cutting::provider_registry::ProviderRegistry::new();
        LogicalCodebaseProviderGateway::with_audit(
            crate::product::logical_codebase::AggregatePolicyArtifactStore::new(self.paths.clone()),
            std::sync::Arc::new(LcT07StaticCapabilitySource),
            std::sync::Arc::new(LcT07TargetResolver),
            std::sync::Arc::new(registry),
            std::sync::Arc::new(LcT07NoopSyncAdapter),
            lc_t07_available_gate(),
            std::sync::Arc::new(crate::product::logical_codebase::GatewayRunAudit::new()),
            self.canonical_root(),
        )
    }
}

/// LC t07 fixture 的 capability 源:按 provider 返回 Confirmed 三格快照。
struct LcT07StaticCapabilitySource;

impl LcT07StaticCapabilitySource {
    fn capability(
        provider_type: crate::product::logical_codebase::provider_gateway::ProviderRefType,
    ) -> crate::product::logical_codebase::ProviderCapability {
        use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
        use crate::product::logical_codebase::policy::{ProviderDialect, ProviderWireDialect};
        use crate::product::logical_codebase::provider_capability_store::ProviderActionCapability;
        use crate::product::logical_codebase::provider_gateway::ProviderRefType;
        let (adapter_dialect, wire_dialect) = match provider_type {
            ProviderRefType::ClaudeCode => (
                ProviderDialect::ClaudeCodeCliV1,
                ProviderWireDialect::ClaudeCodeStreamJson,
            ),
            ProviderRefType::Codex => (
                ProviderDialect::CodexCliV1,
                ProviderWireDialect::CodexAppServerRpc,
            ),
            ProviderRefType::Pi => (ProviderDialect::PiRpcV1, ProviderWireDialect::PiRpc),
            ProviderRefType::KimiCode => (ProviderDialect::KimiAcpV1, ProviderWireDialect::KimiAcp),
        };
        crate::product::logical_codebase::ProviderCapability {
            provider_type,
            version: "1.0.0".to_string(),
            adapter_dialect,
            wire_dialect,
            capability_snapshot_ref: "cap-lcg-t07".to_string(),
            action_capability: ProviderActionCapability {
                action: crate::product::logical_codebase::SessionPolicyAction::CodingTargetWrite,
                launch: ProviderCapabilityEvidence::Confirmed,
                resume: ProviderCapabilityEvidence::Confirmed,
                write_boundary: ProviderCapabilityEvidence::Confirmed,
                projection_digest: "t07-projection-digest".to_string(),
                evidence_ref: "t07-evidence".to_string(),
            },
            trust: ProviderCapabilityEvidence::Confirmed,
        }
    }
}

impl crate::product::logical_codebase::ProviderCapabilitySource for LcT07StaticCapabilitySource {
    fn require_supported(
        &self,
        provider: &crate::product::logical_codebase::provider_gateway::ProviderRef,
        action: crate::product::logical_codebase::SessionPolicyAction,
    ) -> Result<
        crate::product::logical_codebase::ProviderCapability,
        crate::product::logical_codebase::ProviderGatewayError,
    > {
        assert_eq!(
            action,
            crate::product::logical_codebase::SessionPolicyAction::CodingTargetWrite
        );
        Ok(Self::capability(provider.provider_type))
    }

    fn require_resume_supported(
        &self,
        provider: &crate::product::logical_codebase::provider_gateway::ProviderRef,
        action: crate::product::logical_codebase::SessionPolicyAction,
    ) -> Result<
        crate::product::logical_codebase::ProviderCapability,
        crate::product::logical_codebase::ProviderGatewayError,
    > {
        self.require_supported(provider, action)
    }

    fn require_write_boundary(
        &self,
        provider: &crate::product::logical_codebase::provider_gateway::ProviderRef,
        action: crate::product::logical_codebase::SessionPolicyAction,
    ) -> Result<
        crate::product::logical_codebase::ProviderCapability,
        crate::product::logical_codebase::ProviderGatewayError,
    > {
        self.require_supported(provider, action)
    }

    fn require_root_recipe_supported(
        &self,
        provider: &crate::product::logical_codebase::provider_gateway::ProviderRef,
        _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
    ) -> Result<
        crate::product::logical_codebase::ProviderCapability,
        crate::product::logical_codebase::ProviderGatewayError,
    > {
        if provider.provider_type
            != crate::product::logical_codebase::provider_gateway::ProviderRefType::ClaudeCode
        {
            return Err(
                crate::product::logical_codebase::ProviderGatewayError::UnsupportedCapability(
                    "root recipe requires the fixed Claude provider".to_string(),
                ),
            );
        }
        Ok(Self::capability(
            crate::product::logical_codebase::provider_gateway::ProviderRefType::ClaudeCode,
        ))
    }
}

/// pass-through target resolver:原样返回请求冻结的 target。
struct LcT07TargetResolver;

impl crate::product::logical_codebase::PolicyTargetResolver for LcT07TargetResolver {
    fn resolve_and_revalidate(
        &self,
        request: &crate::product::logical_codebase::SessionLaunchRequest,
    ) -> Result<
        crate::product::logical_codebase::PolicyTarget,
        crate::product::logical_codebase::ProviderGatewayError,
    > {
        Ok(request.target.clone())
    }
}

/// 占位 sync adapter:guard 面测试不消费同步栈。
struct LcT07NoopSyncAdapter;

impl crate::cross_cutting::provider_adapter::ProviderAdapter for LcT07NoopSyncAdapter {
    fn run(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
    ) -> Result<
        crate::protocol::contracts::AdapterOutput,
        crate::cross_cutting::provider_adapter::ProviderAdapterError,
    > {
        Err(
            crate::cross_cutting::provider_adapter::ProviderAdapterError::provider_unavailable(
                "lc t07 fixture sync adapter is not consumed",
            ),
        )
    }
}

fn lc_t07_available_gate()
-> std::sync::Arc<crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate> {
    use crate::cross_cutting::provider_availability_gate::{
        ProviderAvailabilityGate, ProviderHealthSource,
    };
    use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
    use crate::product::models::ProviderName;
    use chrono::Utc;

    struct AlwaysHealthy(std::sync::Arc<ProviderHealthSnapshot>);
    impl ProviderHealthSource for AlwaysHealthy {
        fn snapshot(&self) -> std::sync::Arc<ProviderHealthSnapshot> {
            self.0.clone()
        }
        fn degraded(&self) -> bool {
            false
        }
    }

    let checked_at = Utc::now();
    let snapshot = std::sync::Arc::new(ProviderHealthSnapshot {
        schema_version: 1,
        generation: 1,
        checked_at,
        providers: [
            ProviderName::ClaudeCode,
            ProviderName::Pi,
            ProviderName::KimiCode,
        ]
        .into_iter()
        .map(|provider| ProviderHealthEntry {
            provider,
            command: "stub".to_string(),
            available: true,
            version: Some("1.0.0".to_string()),
            reason_code: None,
            reason: None,
            checked_at,
        })
        .collect(),
    });
    std::sync::Arc::new(ProviderAvailabilityGate::new(std::sync::Arc::new(
        AlwaysHealthy(snapshot),
    )))
}

/// 真实 process/extension seam 计数 CLI:每次执行向 marker 追加分类行——
/// `--version` 记 `version`;argv 携带 aria-ask extension 记 `extension`;
/// 其余记 `child`(session 子进程);session 内读到首行 stdin 记 `handshake`
/// (RPC/协议握手)。守卫必须在任何执行之前拒绝,marker 必须为空。
#[cfg(unix)]
fn write_lcg_t07_probe_cli(dir: &std::path::Path, marker: &std::path::Path) -> std::path::PathBuf {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt as _;
    let path = dir.join("lcg-t07-guard-probe-cli");
    let script = format!(
        "#!/usr/bin/env bash\nset -uo pipefail\nmarker={marker:?}\nif [[ \"${{1:-}}\" == \"--version\" ]]; then\n  echo version >> \"$marker\"\n  echo probe 1.0.0\n  exit 0\nfi\nif [[ \"$*\" == *aria-ask* ]]; then\n  echo extension >> \"$marker\"\nelse\n  echo child >> \"$marker\"\nfi\nif IFS= read -r _line; then\n  echo handshake >> \"$marker\"\nfi\nexit 0\n",
        marker = marker.display(),
    );
    let mut file = std::fs::File::create(&path).expect("create lcg t07 probe cli");
    file.write_all(script.as_bytes())
        .expect("write lcg t07 probe cli");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod lcg t07 probe cli");
    path
}

/// 读取 marker 分类计数(真实子进程执行 seam,非源码 grep)。
#[cfg(unix)]
fn lcg_t07_marker_counts(marker: &std::path::Path) -> (usize, usize, usize, usize) {
    let content = std::fs::read_to_string(marker).unwrap_or_default();
    let mut version = 0;
    let mut child = 0;
    let mut handshake = 0;
    let mut extension = 0;
    for line in content.lines() {
        match line.trim() {
            "version" => version += 1,
            "child" => child += 1,
            "handshake" => handshake += 1,
            "extension" => extension += 1,
            _ => {}
        }
    }
    (version, child, handshake, extension)
}

/// Task 7 Step 1(断言组 373-376 逐字):三家(Claude/Codex/Pi)×非法
/// role×policy 组合在 LC `start_validated` 首步被共用双向 guard 拒绝,
/// 早于版本探测/session child/extension——以真实 CLI 执行 marker 计数,
/// 任何执行面都是 0。Codex 的 validated policy 被 gateway 路由级 danger 门
/// 阻断无法经 validate 产出;guard 先于 adapter 匹配,复用 Claude policy
/// 驱动同一 guard 面。
#[cfg(unix)]
#[tokio::test]
async fn lcg_t07_invalid_role_policy_zero_child_and_zero_extension() {
    use crate::cross_cutting::claude_code_provider::ClaudeCodeProvider;
    use crate::cross_cutting::codex_provider::CodexProvider;
    use crate::cross_cutting::pi_provider::PiProvider;
    use crate::cross_cutting::session_launch::ValidatedStreamingProviderInput;
    use crate::protocol::contracts::{AdapterRole, ProviderType};

    let fixture = LcT07GuardFixture::new();
    let marker = fixture.paths.root().join("lcg-t07-guard-marker");
    let cli = write_lcg_t07_probe_cli(fixture.paths.root(), &marker);

    let claude_policy = fixture
        .gateway()
        .validate(fixture.coding_request(
            crate::product::logical_codebase::provider_gateway::ProviderRefType::ClaudeCode,
        ))
        .expect("claude lc launch validates");
    let pi_policy = fixture
        .gateway()
        .validate(fixture.coding_request(
            crate::product::logical_codebase::provider_gateway::ProviderRefType::Pi,
        ))
        .expect("pi lc launch validates");

    // 批2 Task 11：旧「策略角色缺 deny」臂退役（全角色 None 合法）；非法
    // 组合=非 Author 承载角色携带 deny 投影（Reviewer 只携窗口合同、
    // Coder 不带 deny、Handoff 不一概套 deny）。
    let cases: Vec<(&str, AdapterRole, Option<ProviderToolPolicy>)> = [
        AdapterRole::Reviewer,
        AdapterRole::Executor,
        AdapterRole::Handoff,
    ]
    .into_iter()
    .map(|role| {
        (
            "non-author role carrying deny",
            role,
            Some(ProviderToolPolicy::deny_file_write_builtins()),
        )
    })
    .collect();

    let launch_input = |provider_type: ProviderType,
                        role: AdapterRole,
                        tool_policy: Option<ProviderToolPolicy>| {
        StreamingProviderInput {
            working_directory: None,
            baseline_tree: None,
            tool_policy,
            audit_sink: None,
            provider_type,
            role,
            prompt: "lcg t07 invalid role policy probe".to_string(),
            working_dir: fixture.target_worktree(),
            workspace_session_id: Some("ws-lcg-t07".to_string()),
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Auto,
            structured_output_contract: None,
            env_vars: std::collections::BTreeMap::new(),
            timeout_secs: 5,
        }
    };

    let providers: Vec<(
        &str,
        std::sync::Arc<dyn StreamingProviderAdapter>,
        ProviderType,
        crate::product::logical_codebase::ValidatedSessionLaunchPolicy,
    )> = vec![
        (
            "claude",
            std::sync::Arc::new(ClaudeCodeProvider::new(cli.clone())),
            ProviderType::ClaudeCode,
            claude_policy.clone(),
        ),
        (
            "codex",
            std::sync::Arc::new(CodexProvider::new(cli.clone())),
            ProviderType::Codex,
            claude_policy.clone(),
        ),
        (
            "pi",
            std::sync::Arc::new(PiProvider::new(cli.clone())),
            ProviderType::Pi,
            pi_policy.clone(),
        ),
    ];

    let mut invalid_role_policy_result: Result<
        (),
        crate::cross_cutting::provider_adapter::ProviderAdapterError,
    > = Ok(());
    for (provider, adapter, provider_type, policy) in &providers {
        for (case, role, tool_policy) in &cases {
            let input = launch_input(provider_type.clone(), role.clone(), tool_policy.clone());
            let launch = ValidatedStreamingProviderInput::new(input, policy.clone());
            match adapter
                .start_validated(launch, CancellationToken::new())
                .await
            {
                Ok(_session) => {
                    panic!(
                        "{provider}/{case} ({role:?}): invalid role policy must be rejected before spawn"
                    )
                }
                Err(error) => {
                    assert!(
                        error.details.contains("tool policy guard"),
                        "{provider}/{case} ({role:?}): rejection must be the launch guard, got: {}",
                        error.details
                    );
                    invalid_role_policy_result = Err(error);
                }
            }
        }
    }

    let (_version_count, session_child_count, rpc_handshake_count, extension_start_count) =
        lcg_t07_marker_counts(&marker);
    assert!(invalid_role_policy_result.is_err());
    assert_eq!(session_child_count, 0);
    assert_eq!(rpc_handshake_count, 0);
    assert_eq!(extension_start_count, 0);
}

/// Task 7 Step 1(断言组 383-384 逐字):root recipe 的
/// `BootstrapExecutorMarker`(durable Running operation 派生凭据 + receipt
/// context 四要素完整)只对 root recipe 自举通道放行;同一 marker 到普通
/// Coder(无 root recipe 相位上下文)必须在创建子进程之前被拒绝——普通
/// Coder 不得借 marker 获取 root 写权。
#[test]
fn lcg_t07_root_recipe_marker_is_not_normal_coder_policy() {
    use crate::product::logical_codebase::aggregate_initialization::AggregateInitializationStepKind;
    use crate::product::logical_codebase::policy::SessionPolicyAction;
    use crate::product::logical_codebase::provider_admission_preflight::{
        BootstrapExecutorMarker, BootstrapPhaseCredential,
    };
    use crate::protocol::contracts::AdapterRole;

    use super::{ToolPolicyIntent, guard_lc_validated_launch};

    let canonical_root = std::path::PathBuf::from("/tmp/aria-lcg-t07-bootstrap-marker-root");
    let credential = BootstrapPhaseCredential::for_test(
        "project_0001",
        "logical_codebase_0001",
        "aggregate_initialization_0001",
        AggregateInitializationStepKind::PreCheck,
        "sha256:lcg-t07-input-digest",
        canonical_root.clone(),
    );
    let marker = BootstrapExecutorMarker::new(
        credential,
        SessionPolicyAction::CodingTargetWrite,
        canonical_root,
        "root-recipe:pre_check:command-1",
    )
    .expect("complete bootstrap executor marker");
    let marker_policy = ProviderToolPolicy {
        intent: ToolPolicyIntent::BootstrapExecutorMarker(marker),
    };

    // root recipe 通道(RootRecipe 相位:durable operation 凭据 + receipt
    // context 完整)→ 放行。
    let valid_recipe_marker_result = guard_lc_validated_launch(
        true,
        &AdapterRole::Executor,
        &AdapterRole::Executor,
        Some(&marker_policy),
    );
    // 同一 marker 到普通 Coder 启动(Normal 相位,无 root recipe durable
    // operation 上下文)→ 必须拒绝,不得借 marker 获取 root 写权。
    let same_marker_on_ordinary_coder = guard_lc_validated_launch(
        false,
        &AdapterRole::Executor,
        &AdapterRole::Executor,
        Some(&marker_policy),
    );

    assert!(valid_recipe_marker_result.is_ok());
    assert!(same_marker_on_ordinary_coder.is_err());
}

/// r61 kimi 例外映射:DenyFileWriteBuiltins(kimi 由 ClientServicePolicy
/// 等价承担,spec「kimi 既有对齐维持」例外)→ None;None 与其余 intent
/// 原样透传(marker 通道留给消费侧守卫 fail-closed,不在此吞掉)。
#[test]
fn lcg_kimi_exempted_tool_policy_maps_deny_intent_to_none_and_passes_others() {
    use super::{ProviderToolPolicy, ToolPolicyIntent, kimi_exempted_tool_policy};
    use crate::product::logical_codebase::aggregate_initialization::AggregateInitializationStepKind;
    use crate::product::logical_codebase::policy::SessionPolicyAction;
    use crate::product::logical_codebase::provider_admission_preflight::{
        BootstrapExecutorMarker, BootstrapPhaseCredential,
    };

    // 角色矩阵对策略角色的派生形态 → 例外映射为 None。
    assert_eq!(
        kimi_exempted_tool_policy(Some(ProviderToolPolicy::deny_file_write_builtins())),
        None
    );
    // 非策略路径(本来就 None)零变化。
    assert_eq!(kimi_exempted_tool_policy(None), None);

    // 其余 intent 原样透传:BootstrapExecutorMarker 不是 deny 投影,
    // 不得在映射层被吞——kimi 侧由其 ClientServicePolicy 通道级拒绝。
    let canonical_root = std::path::PathBuf::from("/tmp/aria-kimi-exempt-marker-root");
    let credential = BootstrapPhaseCredential::for_test(
        "project_0001",
        "logical_codebase_0001",
        "aggregate_initialization_0001",
        AggregateInitializationStepKind::PreCheck,
        "sha256:test-input-digest",
        canonical_root.clone(),
    );
    let marker = BootstrapExecutorMarker::new(
        credential,
        SessionPolicyAction::CodingTargetWrite,
        canonical_root,
        "root-recipe:pre_check:command-1",
    )
    .expect("complete bootstrap executor marker");
    let marker_policy = ProviderToolPolicy {
        intent: ToolPolicyIntent::BootstrapExecutorMarker(marker),
    };
    assert_eq!(
        kimi_exempted_tool_policy(Some(marker_policy.clone())),
        Some(marker_policy)
    );
}

// ===== Task 3.2(B0a2):LC 版本只读观测 60 秒 TTL/single-flight 合同 =====
//
// owner 2026-10-11 裁决:允许有界只读 `--version` 观测,60 秒非滑动 TTL
// (单调时钟,age>=60s 必再观测),同 key single-flight,失败/取消不缓存、
// 不回退过期成功;不同 key 独立不串行。观测口径①(version probe)与
// ②adapter start/③session child 分列,本文件只测①的缓存合同。

/// 手动单调时钟(注入 `LcVersionCache`;禁止 sleep 概率测试)。
struct AnchorT09ManualClock {
    now: std::sync::Mutex<std::time::Instant>,
}

impl AnchorT09ManualClock {
    fn new() -> Self {
        Self {
            now: std::sync::Mutex::new(std::time::Instant::now()),
        }
    }

    fn advance(&self, duration: Duration) {
        let mut now = self.now.lock().expect("anchor t09 clock");
        *now = now.checked_add(duration).expect("manual clock advance");
    }

    fn source(
        self: &std::sync::Arc<Self>,
    ) -> std::sync::Arc<dyn Fn() -> std::time::Instant + Send + Sync> {
        let clock = self.clone();
        std::sync::Arc::new(move || *clock.now.lock().expect("anchor t09 clock"))
    }
}

/// 可切换观测结果的计数 probe factory(观测口径①)。
#[derive(Clone, Default)]
struct AnchorT09ProbeFactory {
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    outcome: std::sync::Arc<std::sync::Mutex<AnchorT09ProbeOutcome>>,
}

#[derive(Clone)]
enum AnchorT09ProbeOutcome {
    Version(String),
    Fail(super::VersionProbeError),
}

impl Default for AnchorT09ProbeOutcome {
    fn default() -> Self {
        Self::Version("cli 1.4.0".to_string())
    }
}

impl AnchorT09ProbeFactory {
    fn probe(&self) -> super::LcVersionProbe {
        let calls = self.calls.clone();
        let outcome = self.outcome.clone();
        std::sync::Arc::new(move |_key| {
            let calls = calls.clone();
            let outcome = outcome.clone();
            Box::pin(async move {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                match outcome.lock().expect("anchor t09 probe").clone() {
                    AnchorT09ProbeOutcome::Version(version) => Ok(version),
                    AnchorT09ProbeOutcome::Fail(error) => Err(error),
                }
            })
        })
    }

    fn set_version(&self, version: &str) {
        *self.outcome.lock().expect("anchor t09 probe") =
            AnchorT09ProbeOutcome::Version(version.to_string());
    }

    fn set_failure(&self, error: super::VersionProbeError) {
        *self.outcome.lock().expect("anchor t09 probe") = AnchorT09ProbeOutcome::Fail(error);
    }

    fn calls(&self) -> usize {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

fn anchor_t09_key(
    provider: crate::product::models::ProviderName,
    executable_identity: &str,
) -> super::LcVersionProbeKey {
    super::LcVersionProbeKey {
        provider,
        canonical_executable: std::path::PathBuf::from("/opt/anchor-t09-fake-bin/cli"),
        executable_identity: executable_identity.to_string(),
        argv: vec!["--version".to_string()],
        supplier_scope: 1,
    }
}

/// 59.999s 命中/60.000s 边界再观测(非滑动)、same-key burst single-flight、
/// different-key 独立、失败不缓存、失败/过期不回退成功值。
#[tokio::test]
async fn anchor_t09_version_ttl_boundary_singleflight_and_failures() {
    use std::sync::Arc;

    let clock = Arc::new(AnchorT09ManualClock::new());
    let cache = Arc::new(super::LcVersionCache::with_clock(clock.source()));
    let probe = AnchorT09ProbeFactory::default();
    let cancel = CancellationToken::new();
    let key = anchor_t09_key(
        crate::product::models::ProviderName::ClaudeCode,
        "identity-1",
    );

    // same-key 并发 burst:barrier 同时释放 8 个 caller——single-flight 下
    // 全程只有 winner 的一次有界观测,其余 caller 等同一结果或命中缓存。
    let barrier = Arc::new(tokio::sync::Barrier::new(9));
    let mut callers = Vec::new();
    for _ in 0..8 {
        let cache = cache.clone();
        let probe = probe.probe();
        let cancel = cancel.clone();
        let key = key.clone();
        let barrier = barrier.clone();
        callers.push(tokio::spawn(async move {
            barrier.wait().await;
            cache
                .cached_lc_cli_version(key, probe, cancel)
                .await
                .expect("burst caller observes the single-flight result")
        }));
    }
    barrier.wait().await;
    for caller in callers {
        assert_eq!(
            caller
                .await
                .expect("burst caller join")
                .exact_version()
                .to_string(),
            "cli 1.4.0"
        );
    }
    let probe_calls_after_burst = probe.calls();
    assert_eq!(probe_calls_after_burst, 1);

    // age=59.999s:非滑动 TTL 命中,不触发新观测。
    clock.advance(Duration::from_millis(59_999));
    let hit = cache
        .cached_lc_cli_version(key.clone(), probe.probe(), cancel.clone())
        .await
        .expect("59.999s observation is still within the TTL");
    assert_eq!(hit.exact_version(), "cli 1.4.0");
    let probe_calls_at_59_999_ms = probe.calls();
    assert_eq!(probe_calls_at_59_999_ms, 1);

    // age=60.000s:边界到期(age>=60s 必再观测;若为滑动 TTL,59.999s 的
    // 命中会把续期后 age 拉回 0.001s——仍会命中,此处即红)。
    clock.advance(Duration::from_millis(1));
    let refreshed = cache
        .cached_lc_cli_version(key.clone(), probe.probe(), cancel.clone())
        .await
        .expect("60s boundary re-observes");
    assert_eq!(refreshed.exact_version(), "cli 1.4.0");
    let probe_calls_at_60_000_ms = probe.calls();
    assert_eq!(probe_calls_at_60_000_ms, 2);

    // 观测失败不缓存(独立 key,不受上面 key 的成功观测干扰):连续两次
    // 失败都真实再观测、都 typed Err——失败结果不进入缓存。
    let failure_key = anchor_t09_key(
        crate::product::models::ProviderName::KimiCode,
        "identity-failure",
    );
    probe.set_failure(super::VersionProbeError::Unavailable);
    let failed_first = cache
        .cached_lc_cli_version(failure_key.clone(), probe.probe(), cancel.clone())
        .await;
    assert!(matches!(
        failed_first,
        Err(super::VersionProbeError::Unavailable)
    ));
    let failed_second = cache
        .cached_lc_cli_version(failure_key.clone(), probe.probe(), cancel.clone())
        .await;
    assert!(matches!(
        failed_second,
        Err(super::VersionProbeError::Unavailable)
    ));
    assert_eq!(probe.calls(), 4);

    // 失败/过期不回退成功值:先成功并缓存,把时钟推过 TTL 后让观测失败——
    // 调用必须得到 typed 失败,绝不返回过期成功值。
    probe.set_version("cli 1.4.0");
    cache
        .cached_lc_cli_version(failure_key.clone(), probe.probe(), cancel.clone())
        .await
        .expect("successful re-observation");
    clock.advance(Duration::from_secs(61));
    probe.set_failure(super::VersionProbeError::Timeout);
    let expired = cache
        .cached_lc_cli_version(failure_key, probe.probe(), cancel.clone())
        .await;
    assert!(
        matches!(expired, Err(super::VersionProbeError::Timeout)),
        "expired success must never be served as a fallback"
    );
    assert_eq!(probe.calls(), 6);
    probe.set_version("cli 1.4.0");

    // different-key 独立不串行:key2 的观测阻塞在 gate 上,key3 仍能立即完成。
    let (key2_started_tx, key2_started_rx) = tokio::sync::oneshot::channel::<()>();
    let (key2_release_tx, key2_release_rx) = tokio::sync::oneshot::channel::<()>();
    let key2 = anchor_t09_key(crate::product::models::ProviderName::Codex, "identity-1");
    let key3 = anchor_t09_key(crate::product::models::ProviderName::Pi, "identity-1");
    let blocking_probe: super::LcVersionProbe = {
        let started = std::sync::Mutex::new(Some(key2_started_tx));
        let release = std::sync::Mutex::new(Some(key2_release_rx));
        std::sync::Arc::new(move |_key| {
            let _ = started.lock().unwrap().take().map(|tx| tx.send(()));
            let release = release.lock().unwrap().take();
            Box::pin(async move {
                if let Some(release) = release {
                    let _ = release.await;
                }
                Ok("codex 2.0.0".to_string())
            })
        })
    };
    let key2_caller = {
        let cache = cache.clone();
        let cancel = cancel.clone();
        let key = key2.clone();
        tokio::spawn(async move {
            cache
                .cached_lc_cli_version(key, blocking_probe, cancel)
                .await
                .expect("blocked key2 observation settles after release")
        })
    };
    key2_started_rx
        .await
        .expect("key2 probe has started and is parked on the gate");
    let key3_observation = tokio::time::timeout(
        TEST_TIMEOUT,
        cache.cached_lc_cli_version(key3, probe.probe(), cancel.clone()),
    )
    .await
    .expect("different-key observations must not serialize behind each other")
    .expect("key3 observation");
    assert_eq!(key3_observation.exact_version(), "cli 1.4.0");
    key2_release_tx
        .send(())
        .expect("release the blocked key2 observation");
    let key2_observation = key2_caller.await.expect("key2 caller join");
    assert_eq!(key2_observation.exact_version(), "codex 2.0.0");
}

/// 活缓存期间命令 canonical executable identity/路径/supplier scope 变化
/// 直接换 key:必须再观测;旧 key 命中不串值。
#[tokio::test]
async fn anchor_t09_command_identity_change_invalidates_live_cache() {
    let cache = std::sync::Arc::new(super::LcVersionCache::new());
    let probe = AnchorT09ProbeFactory::default();
    let old_key = anchor_t09_key(
        crate::product::models::ProviderName::ClaudeCode,
        "identity-v1",
    );
    let observation = cache
        .cached_lc_cli_version(old_key.clone(), probe.probe(), CancellationToken::new())
        .await
        .expect("initial observation");
    assert_eq!(observation.exact_version(), "cli 1.4.0");

    // executable identity 改变 → 新 key(路径相同也算不同命令)。
    let new_key = anchor_t09_key(
        crate::product::models::ProviderName::ClaudeCode,
        "identity-v2",
    );
    assert_ne!(new_key, old_key);
    // canonical executable 路径改变 → 新 key。
    let path_key = super::LcVersionProbeKey {
        canonical_executable: std::path::PathBuf::from("/opt/anchor-t09-other-bin/cli"),
        ..old_key.clone()
    };
    assert_ne!(path_key, old_key);
    // supplier 实例范围改变 → 新 key(测试 supplier 与真实 CLI 不串值)。
    let scoped_key = super::LcVersionProbeKey {
        supplier_scope: 2,
        ..old_key.clone()
    };
    assert_ne!(scoped_key, old_key);

    let probe_calls_before_identity_change = probe.calls();
    let reobserved = cache
        .cached_lc_cli_version(new_key.clone(), probe.probe(), CancellationToken::new())
        .await
        .expect("identity change must re-observe under the new key");
    assert_eq!(reobserved.exact_version(), "cli 1.4.0");
    let probe_calls_after_identity_change = probe.calls();
    assert_eq!(
        probe_calls_after_identity_change,
        probe_calls_before_identity_change + 1
    );

    // 旧 key 仍在 TTL 内命中,不与新 key 串值(不再观测)。
    let old_hit = cache
        .cached_lc_cli_version(old_key, probe.probe(), CancellationToken::new())
        .await
        .expect("old key still hits within its TTL");
    assert_eq!(old_hit.exact_version(), "cli 1.4.0");
    assert_eq!(probe.calls(), 2);
}

/// 进程(缓存实例)重建不能复用旧 observed_at/值:同 key 必须重新观测。
#[tokio::test]
async fn anchor_t09_process_restart_does_not_reuse_version_cache() {
    let clock = std::sync::Arc::new(AnchorT09ManualClock::new());
    let probe = AnchorT09ProbeFactory::default();
    let key = anchor_t09_key(
        crate::product::models::ProviderName::KimiCode,
        "identity-restart",
    );
    let first_cache = super::LcVersionCache::with_clock(clock.source());
    let original = first_cache
        .cached_lc_cli_version(key.clone(), probe.probe(), CancellationToken::new())
        .await
        .expect("original observation");
    assert_eq!(probe.calls(), 1);

    // 推进 5s(远小于 TTL):若跨实例复用旧值,新实例将命中而不观测。
    clock.advance(Duration::from_secs(5));
    let restarted = super::LcVersionCache::with_clock(clock.source());
    let reobserved = restarted
        .cached_lc_cli_version(key.clone(), probe.probe(), CancellationToken::new())
        .await
        .expect("restarted cache instance re-observes");
    let new_cache_instance_probe_calls = probe.calls() - 1;
    assert_eq!(new_cache_instance_probe_calls, 1);
    assert_ne!(
        reobserved.observed_at(),
        original.observed_at(),
        "restarted instance must not reuse the old observed_at"
    );

    // 旧实例自身的值未被新实例串改:原实例 TTL 内命中且不新增观测。
    let still = first_cache
        .cached_lc_cli_version(key, probe.probe(), CancellationToken::new())
        .await
        .expect("original instance still hits its own observation");
    assert_eq!(still.observed_at(), original.observed_at());
    assert_eq!(probe.calls(), 2);
}

/// probe 超时与 caller 取消各给 typed 失败;取消后的迟到成功不得入缓存、
/// 不得复活被取消的旧 run;释放 barrier 后旧 run 仍为零启动。
#[tokio::test]
async fn anchor_t09_probe_timeout_or_cancel_cannot_revive_old_run() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let clock = Arc::new(AnchorT09ManualClock::new());
    let cache = Arc::new(super::LcVersionCache::with_clock(clock.source()));
    let probe = AnchorT09ProbeFactory::default();

    // ① 超时:typed Err(Timeout),不缓存(再调必须再观测)。
    probe.set_failure(super::VersionProbeError::Timeout);
    let timeout_key = anchor_t09_key(
        crate::product::models::ProviderName::Codex,
        "identity-timeout",
    );
    let timed_out_observation = cache
        .cached_lc_cli_version(timeout_key.clone(), probe.probe(), CancellationToken::new())
        .await;
    assert!(matches!(
        timed_out_observation,
        Err(super::VersionProbeError::Timeout)
    ));
    let timed_out_again = cache
        .cached_lc_cli_version(timeout_key, probe.probe(), CancellationToken::new())
        .await;
    assert!(timed_out_again.is_err());
    assert_eq!(probe.calls(), 2);

    // ② caller 取消:probe 阻塞 → 取消 → typed Err;迟到成功不入缓存、
    //    不复活旧 run(barrier 释放后 old_run_started 仍为 false)。
    probe.set_version("cli 1.4.0");
    let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();
    let blocking_probe: super::LcVersionProbe = {
        let started = std::sync::Mutex::new(Some(started_tx));
        let release = std::sync::Mutex::new(Some(release_rx));
        std::sync::Arc::new(move |_key| {
            let _ = started.lock().unwrap().take().map(|tx| tx.send(()));
            let release = release.lock().unwrap().take();
            Box::pin(async move {
                if let Some(release) = release {
                    let _ = release.await;
                }
                Ok("cli 9.9.9-late".to_string())
            })
        })
    };
    let cancel = CancellationToken::new();
    let cancel_key = anchor_t09_key(crate::product::models::ProviderName::Pi, "identity-cancel");
    let old_run_started = Arc::new(AtomicBool::new(false));
    let cancelled_caller = {
        let cache = cache.clone();
        let cancel = cancel.clone();
        let key = cancel_key.clone();
        let old_run_started = old_run_started.clone();
        tokio::spawn(async move {
            let observation = cache
                .cached_lc_cli_version(key, blocking_probe, cancel)
                .await;
            // 只有观测成功才允许「启动旧 run」——取消路径必须到不了这里。
            if observation.is_ok() {
                old_run_started.store(true, Ordering::SeqCst);
            }
            observation
        })
    };
    started_rx
        .await
        .expect("blocking probe has started and is parked");
    cancel.cancel();
    let cancelled_observation = cancelled_caller
        .await
        .expect("cancelled caller joins")
        .map(|observation| observation.exact_version().to_string());
    assert!(
        cancelled_observation.is_err(),
        "cancelled caller must receive a typed failure"
    );

    // 释放 barrier:probe 的迟到成功(if any)不得复活旧 run/进入缓存。
    let late_release_succeeded = release_tx.send(()).is_ok();
    let _ = late_release_succeeded;
    let after_release = cache
        .cached_lc_cli_version(cancel_key, probe.probe(), CancellationToken::new())
        .await
        .expect("fresh caller re-observes after cancellation");
    assert_ne!(
        after_release.exact_version(),
        "cli 9.9.9-late",
        "late result of the cancelled probe must not leak into the cache"
    );
    assert!(
        !old_run_started.load(Ordering::SeqCst),
        "the cancelled old run must not start after barrier release"
    );
}

/// 审查 fix-loop R1(P2):winner panic 不得永久滞留 single-flight 槽——
/// unwind 只丢弃 winner 侧的 sender 克隆,若 HashMap 槽内的克隆随之保留,
/// watch 通道不关闭,同 key 后续 caller 走 Waiting 永久阻塞(无 typed
/// 错误、无日志直至重启)。panic 后同 key 新 caller 必须能重新观测。
#[tokio::test]
async fn anchor_t09_winner_panic_evicts_inflight_slot_for_next_caller() {
    use std::sync::Arc;

    let cache = Arc::new(super::LcVersionCache::new());
    let key = anchor_t09_key(
        crate::product::models::ProviderName::ClaudeCode,
        "identity-panic",
    );

    // winner 的 probe future 注入 panic:任务边界捕获(JoinError::is_panic),
    // 不让测试进程直接终止。
    let panicking_probe: super::LcVersionProbe = std::sync::Arc::new(|_key| {
        Box::pin(async {
            panic!("anchor t09 injected winner panic inside the probe future");
        })
    });
    let panicked_winner = {
        let cache = cache.clone();
        let key = key.clone();
        tokio::spawn(async move {
            cache
                .cached_lc_cli_version(key, panicking_probe, CancellationToken::new())
                .await
        })
    };
    let join_error = panicked_winner
        .await
        .expect_err("the panicking winner must surface as a JoinError, not a result");
    assert!(
        join_error.is_panic(),
        "expected a panic-carrying JoinError, got {join_error:?}"
    );

    // panic 卸载必须逐出 InFlight 槽:同 key 后续 caller 重新观测并成功,
    // 而不是在死通道上永久等待(timeout 守卫把「挂起」转为确定失败)。
    let probe = AnchorT09ProbeFactory::default();
    let next = tokio::time::timeout(
        TEST_TIMEOUT,
        cache.cached_lc_cli_version(key, probe.probe(), CancellationToken::new()),
    )
    .await
    .expect("next same-key caller must not hang on the panicked in-flight slot")
    .expect("re-observation after the panic succeeds");
    assert_eq!(next.exact_version(), "cli 1.4.0");
    assert_eq!(probe.calls(), 1);
}
