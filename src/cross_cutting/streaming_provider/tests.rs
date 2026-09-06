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
async fn run_streaming_bridge_injects_engine_provided_sink_for_policy_roles() {
    use crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink;
    use std::sync::atomic::Ordering;

    let provider = LegacySinkHookProvider {
        sink: RecordingToolPolicyAuditSink::new(),
        saw_policy: std::sync::atomic::AtomicBool::new(false),
        saw_sink: std::sync::atomic::AtomicBool::new(false),
    };
    // make_input 的 role=Orchestrator（策略角色）：默认 bridge 必须派生策略并
    // 同时注入 hook 提供的 sink，不得让真实 adapter 在 legacy 直连上缺 sink
    // 运行时 fail-closed。
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
        provider.saw_policy.load(Ordering::SeqCst),
        "legacy bridge must derive the deny policy for policy roles"
    );
    assert!(
        provider.saw_sink.load(Ordering::SeqCst),
        "legacy bridge must inject the engine-provided audit sink alongside the derived policy"
    );
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
    assert_eq!(codex.tokens, frozen, "codex canonical tokens must be the exact frozen sequence");

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

// ---- Task 3.1（REQ-ENV-09）：三 adapter 双向 spawn 前守卫 ----

#[test]
fn adapter_tool_policy_guard_is_bidirectional_for_every_role() {
    use super::validate_tool_policy_for_role;
    use crate::protocol::contracts::AdapterRole;

    let deny = ProviderToolPolicy::deny_file_write_builtins();
    for role in [
        AdapterRole::Orchestrator,
        AdapterRole::WorkItemSplitter,
        AdapterRole::Reviewer,
    ] {
        assert!(
            validate_tool_policy_for_role(&role, None).is_err(),
            "policy role {role:?} without policy must be rejected"
        );
        assert!(
            validate_tool_policy_for_role(&role, Some(&deny)).is_ok(),
            "policy role {role:?} with deny policy must be accepted"
        );
    }
    for role in [AdapterRole::Executor, AdapterRole::Handoff] {
        assert!(
            validate_tool_policy_for_role(&role, None).is_ok(),
            "non-policy role {role:?} without policy must be accepted"
        );
        assert!(
            validate_tool_policy_for_role(&role, Some(&deny)).is_err(),
            "non-policy role {role:?} carrying policy must be rejected"
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

    // 策略角色缺失策略：spawn 前拒绝。
    expect_guard_rejection(
        "pi",
        &PiProvider::new(missing_cli.clone()),
        base_input(ProviderType::Pi, AdapterRole::Orchestrator, None),
    )
    .await;
    expect_guard_rejection(
        "claude",
        &ClaudeCodeProvider::new(missing_cli.clone()),
        base_input(ProviderType::ClaudeCode, AdapterRole::Reviewer, None),
    )
    .await;
    expect_guard_rejection(
        "codex",
        &CodexProvider::new(missing_cli.clone()),
        base_input(ProviderType::Codex, AdapterRole::WorkItemSplitter, None),
    )
    .await;

    // 非策略角色误带策略：spawn 前拒绝。
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
