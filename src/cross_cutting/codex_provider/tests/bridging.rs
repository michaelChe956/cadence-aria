// 桥接族（permission / request_user_input）：审批与用户输入桥接行为、
// requestUserInput 协议错误路径（bridge 失败 / 写失败）的 wire 级验证。
// 从 tests/mod.rs 拆出以满足 large_file_guard 的 1200 行上限。

use super::*;

#[tokio::test]
async fn codex_provider_bridges_permission_and_completes() {
    let fixture = executable_fixture("tests/fixtures/provider/codex_app_server_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Supervised);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let mut saw_text = false;
    let permission_id = loop {
        match session.events.recv().await.unwrap() {
            ProviderEvent::TextDelta { content } => {
                saw_text = content.contains("Codex fixture chunk");
            }
            ProviderEvent::PermissionRequest(data) => break data.id,
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
            other => panic!("unexpected event before permission: {other:?}"),
        }
    };
    assert!(saw_text);

    session
        .commands
        .send(ProviderCommand::PermissionResponse {
            id: permission_id,
            approved: true,
            reason: None,
        })
        .await
        .unwrap();

    let completed = recv_completed(&mut session.events).await;
    assert_eq!(completed, "Codex fixture chunk");
}

#[tokio::test]
async fn codex_provider_responds_to_current_command_approval_with_json_rpc_result() {
    let fixture = executable_fixture(
        "tests/fixtures/provider/codex_app_server_current_permission_fixture.sh",
    );
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Supervised);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let permission = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit current command approval")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::PermissionRequest(request) => break request,
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::ChoiceRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
            ProviderEvent::Completed(completion) => {
                let full_output = completion.full_output;
                panic!("provider completed before permission request: {full_output}")
            }
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error: {message}")
            }
            ProviderEvent::UsageReport(_)
            | ProviderEvent::ToolPolicyDecision(_)
            | ProviderEvent::ToolPolicyWarning(_)
            | ProviderEvent::ToolPolicyTerminated(_) => {}
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
        }
    };
    assert_eq!(permission.tool_name, "command");
    assert!(permission.description.contains("pnpm -C web install"));

    session
        .commands
        .send(ProviderCommand::PermissionResponse {
            id: permission.id,
            approved: true,
            reason: None,
        })
        .await
        .unwrap();

    let completed = recv_completed(&mut session.events).await;
    assert_eq!(completed, "permission accepted");
}

#[tokio::test]
async fn codex_provider_bridges_request_user_input_and_completes() {
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_user_input_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let choice = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit a choice request")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::ChoiceRequest(request) => break request,
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
            ProviderEvent::Completed(completion) => {
                let full_output = completion.full_output;
                panic!("provider completed before choice request: {full_output}")
            }
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error: {message}")
            }
            ProviderEvent::UsageReport(_)
            | ProviderEvent::ToolPolicyDecision(_)
            | ProviderEvent::ToolPolicyWarning(_)
            | ProviderEvent::ToolPolicyTerminated(_) => {}
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
        }
    };

    assert_eq!(choice.id, "77");
    assert_eq!(choice.prompt, "请选择复杂度");
    assert_eq!(choice.options[0].id, "O(n)");
    assert_eq!(choice.options[0].description.as_deref(), Some("线性复杂度"));

    session
        .commands
        .send(ProviderCommand::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec!["O(n)".to_string()],
            free_text: None,
            answers: vec![],
        })
        .await
        .unwrap();

    let completed = recv_completed(&mut session.events).await;
    assert_eq!(completed, "Codex received O(n)");
}

#[tokio::test]
async fn codex_provider_bridges_all_request_user_input_questions() {
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_multi_user_input_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .unwrap();

    let choice = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit a choice request")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::ChoiceRequest(request) => break request,
            ProviderEvent::StatusChanged(_)
            | ProviderEvent::Execution(_)
            | ProviderEvent::TextDelta { .. }
            | ProviderEvent::PermissionRequest(_)
            | ProviderEvent::ToolCall(_)
            | ProviderEvent::ToolResult(_) => {}
            ProviderEvent::Completed(completion) => {
                let full_output = completion.full_output;
                panic!("provider completed before choice request: {full_output}")
            }
            ProviderEvent::Failed { message } => panic!("provider failed: {message}"),
            ProviderEvent::ProtocolError { message, .. } => {
                panic!("provider protocol error: {message}")
            }
            ProviderEvent::UsageReport(_)
            | ProviderEvent::ToolPolicyDecision(_)
            | ProviderEvent::ToolPolicyWarning(_)
            | ProviderEvent::ToolPolicyTerminated(_) => {}
            ProviderEvent::PermissionTimeout { permission_id } => {
                panic!("provider permission timed out: {permission_id}")
            }
        }
    };

    assert_eq!(choice.id, "91");
    assert_eq!(choice.questions.len(), 3);
    assert_eq!(choice.questions[0].id, "startup");
    assert_eq!(choice.questions[0].prompt, "启动自检策略？");
    assert_eq!(choice.questions[1].id, "scope");
    assert_eq!(choice.questions[1].prompt, "影响范围？");
    assert_eq!(choice.questions[2].id, "mcp_events");
    assert_eq!(choice.questions[2].prompt, "MCP 事件输出？");

    session
        .commands
        .send(ProviderCommand::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec![],
            free_text: None,
            answers: vec![
                ChoiceAnswerData {
                    question_id: "startup".to_string(),
                    selected_option_ids: vec!["每次启动都自检".to_string()],
                    free_text: None,
                },
                ChoiceAnswerData {
                    question_id: "scope".to_string(),
                    selected_option_ids: vec!["Story/Design/Work Item 共享链路".to_string()],
                    free_text: None,
                },
                ChoiceAnswerData {
                    question_id: "mcp_events".to_string(),
                    selected_option_ids: vec!["输出 MCP 事件".to_string()],
                    free_text: None,
                },
            ],
        })
        .await
        .unwrap();

    let completed = recv_completed(&mut session.events).await;
    assert_eq!(completed, "Codex received all answers");
}

#[tokio::test]
async fn codex_provider_request_user_input_emits_protocol_error_on_bridge_failure() {
    let fixture =
        executable_fixture("tests/fixtures/provider/codex_app_server_user_input_fixture.sh");
    let provider = CodexProvider::new(fixture);
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let cancel = CancellationToken::new();
    let mut session = provider.start(input, cancel.clone()).await.unwrap();

    let _choice = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should emit choice")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::ChoiceRequest(choice) => break choice,
            ProviderEvent::Failed { message } => {
                panic!("provider failed before choice: {message}")
            }
            _ => {}
        }
    };

    // 取消 provider 以强制 bridge 失败。
    cancel.cancel();

    let mut saw_protocol_error = false;
    while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
        .await
        .unwrap_or(None)
    {
        match event {
            ProviderEvent::ProtocolError {
                code,
                message,
                context,
            } if code == "request_user_input_unresolved" => {
                assert!(
                    message.contains("requestUserInput"),
                    "unexpected message: {message}"
                );
                assert!(
                    message.contains("unresolved"),
                    "unexpected message: {message}"
                );
                let context = context.expect("protocol error should include context");
                assert_eq!(
                    context.get("question_id").and_then(Value::as_str),
                    Some("complexity")
                );
                saw_protocol_error = true;
                break;
            }
            ProviderEvent::Completed(_) => {
                panic!("expected protocol error before completion")
            }
            ProviderEvent::Failed { message } => {
                panic!("expected protocol error before failure: {message}")
            }
            _ => {}
        }
    }
    assert!(
        saw_protocol_error,
        "expected request_user_input_unresolved protocol error after bridge failure"
    );
}

// 写失败注入确定性化（RR-3 单例销账，wave4-passive-flaky）：原形态=真实进程 fixture
// （codex_request_user_input_peer_closes_fixture.sh 关闭 stdin 读端→EPIPE）叠加
// TEST_TIMEOUT 墙钟窗口——全量首跑在冷缓存/高并发负载下事件窗口可被饿死（历史
// 首跑红/单跑绿，stage4-c3-t5 §3.6/w1 登记）。验证方式迁移（d7525220 kimi terminal
// 同款先例）：受控内存流（tokio duplex）直接驱动 run_codex_session_loop，stdin
// 读端在触发写的 ChoiceResponse 发出之前 drop——写失败由因果定序保证，无真实
// 进程/无墙钟竞态；断言与原形态逐字一致（code/message/question_id）。
#[tokio::test]
async fn codex_provider_request_user_input_emits_protocol_error_on_write_failure() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let (mut server_stdout, provider_stdout) = tokio::io::duplex(64 * 1024);
    let (provider_stdin, server_stdin) = tokio::io::duplex(64 * 1024);
    let peer = JsonRpcPeer::new(provider_stdout, provider_stdin)
        .with_outbound_id_namespace(OutboundIdNamespace::Aria);

    let (event_tx, mut session_events) = mpsc::channel(32);
    let bridge = ApprovalBridge::new(ProviderPermissionMode::Auto, event_tx.clone());
    let commands = bridge.command_sender();
    let input = streaming_input(ProviderType::Codex, ProviderPermissionMode::Auto);
    let cancel = CancellationToken::new();

    let session_loop = tokio::spawn(async move {
        run_codex_session_loop(
            peer,
            bridge,
            event_tx,
            input,
            cancel,
            CodexSessionHandshake {
                resume_session_id: None,
                thread_id: Some("codex_input_thread".to_string()),
            },
        )
        .await
    });

    // 服务端泵：应答 turn/start（复用出站原生 id），随后回放 requestUserInput 请求
    // （id=88/question=confirm/options=是|否——逐字取自已退役的 peer_closes 真实
    // fixture，保证 parse 路径与原形态一致）。
    let mut server_stdin_reader = BufReader::new(server_stdin);
    let mut turn_start_line = String::new();
    server_stdin_reader
        .read_line(&mut turn_start_line)
        .await
        .expect("provider should send turn/start");
    let turn_start_id = serde_json::from_str::<Value>(&turn_start_line)
        .expect("turn/start should be valid JSON")
        .get("id")
        .cloned()
        .expect("turn/start should carry an id");
    let turn_start_response = json!({
        "jsonrpc": "2.0",
        "id": turn_start_id,
        "result": { "turn": { "id": "codex_input_turn", "status": "inProgress" } },
    });
    server_stdout
        .write_all(format!("{turn_start_response}\n").as_bytes())
        .await
        .expect("server should answer turn/start");

    let request_user_input = json!({
        "jsonrpc": "2.0",
        "id": 88,
        "method": "item/tool/requestUserInput",
        "params": {
            "threadId": "codex_input_thread",
            "turnId": "codex_input_turn",
            "itemId": "ask_1",
            "questions": [{
                "id": "confirm",
                "header": "确认",
                "question": "继续？",
                "options": [{"label": "是"}, {"label": "否"}]
            }]
        }
    });
    server_stdout
        .write_all(format!("{request_user_input}\n").as_bytes())
        .await
        .expect("server should emit requestUserInput");

    let choice = loop {
        match tokio::time::timeout(TEST_TIMEOUT, session_events.recv())
            .await
            .expect("provider should emit choice")
            .expect("provider event channel should stay open")
        {
            ProviderEvent::ChoiceRequest(choice) => break choice,
            ProviderEvent::Failed { message } => {
                panic!("provider failed before choice: {message}")
            }
            _ => {}
        }
    };

    // 确定性写失败注入：drop stdin 读端严格先于发出触发写的 ChoiceResponse——
    // provider 侧写失败时序由因果定序锁定（duplex 对端 drop 后写返回 BrokenPipe，
    // 与真实管道 EPIPE 同语义），不再依赖进程调度竞态。
    drop(server_stdin_reader);

    commands
        .send(ProviderCommand::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec!["是".to_string()],
            free_text: None,
            answers: vec![],
        })
        .await
        .expect("send choice response");

    let mut saw_protocol_error = false;
    while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session_events.recv())
        .await
        .expect("provider should emit events")
    {
        match event {
            ProviderEvent::ProtocolError {
                code,
                message,
                context,
            } if code == "request_user_input_unresolved" => {
                assert!(
                    message.contains("requestUserInput"),
                    "unexpected message: {message}"
                );
                assert!(
                    message.contains("unresolved"),
                    "unexpected message: {message}"
                );
                let context = context.expect("protocol error should include context");
                assert_eq!(
                    context.get("question_id").and_then(Value::as_str),
                    Some("confirm")
                );
                saw_protocol_error = true;
                break;
            }
            ProviderEvent::Completed(_) => {
                panic!("expected protocol error before completion")
            }
            ProviderEvent::Failed { message } => {
                panic!("expected protocol error before failure: {message}")
            }
            _ => {}
        }
    }
    assert!(
        saw_protocol_error,
        "expected request_user_input_unresolved protocol error when JSON-RPC response write fails"
    );

    // 写失败必须以错误终止会话循环（原形态经 provider.start 包装任务发 Failed；
    // 迁移后直接断言会话循环返回 Err，覆盖同一契约）。
    let session_result = tokio::time::timeout(TEST_TIMEOUT, session_loop)
        .await
        .expect("session loop should finish after write failure")
        .expect("session task should join");
    assert!(
        session_result.is_err(),
        "write failure must terminate the session loop with an error"
    );
}
