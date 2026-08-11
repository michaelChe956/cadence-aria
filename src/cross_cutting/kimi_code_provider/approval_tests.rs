use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ProviderCommand, ProviderEvent, ProviderPermissionMode, StreamingProviderInput,
};
use crate::protocol::contracts::{AdapterRole, ProviderType};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::kimi_code_provider::session::run_kimi_session;
use crate::cross_cutting::kimi_code_provider::{KimiCodeProvider, StreamingProviderAdapter};

async fn terminal_events(
    session: &mut crate::cross_cutting::streaming_provider::ProviderSession,
) -> Vec<ProviderEvent> {
    let mut events = Vec::new();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(300);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout_at(deadline, session.events.recv()).await {
            Ok(Some(event)) => events.push(event),
            Ok(None) | Err(_) => break,
        }
    }
    events
}

fn fixture_command(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/provider")
        .join(name)
}

fn input(resume: Option<&str>, timeout_secs: u64) -> StreamingProviderInput {
    StreamingProviderInput {
        provider_type: ProviderType::KimiCode,
        role: AdapterRole::Orchestrator,
        prompt: "fixture prompt".to_string(),
        working_dir: std::env::current_dir().expect("working directory"),
        workspace_session_id: None,
        resume_provider_session_id: resume.map(ToString::to_string),
        permission_mode: ProviderPermissionMode::Auto,
        structured_output_contract: None,
        env_vars: BTreeMap::new(),
        timeout_secs,
    }
}

async fn read_request(
    reader: &mut tokio::io::BufReader<impl tokio::io::AsyncRead + Unpin>,
) -> Value {
    let mut line = String::new();
    reader.read_line(&mut line).await.expect("read request");
    serde_json::from_str(&line).expect("JSON-RPC request")
}

async fn send_message(writer: &mut (impl tokio::io::AsyncWrite + Unpin), value: Value) {
    writer
        .write_all(value.to_string().as_bytes())
        .await
        .expect("write message");
    writer.write_all(b"\n").await.expect("write newline");
    writer.flush().await.expect("flush message");
}

fn test_peer() -> (
    JsonRpcPeer<tokio::io::WriteHalf<tokio::io::DuplexStream>>,
    tokio::io::DuplexStream,
) {
    let (client, server) = tokio::io::duplex(16 * 1024);
    let (reader, writer) = tokio::io::split(client);
    (JsonRpcPeer::new(reader, writer), server)
}

async fn direct_session_events<W>(
    peer: JsonRpcPeer<W>,
    input: StreamingProviderInput,
) -> (
    mpsc::Sender<ProviderCommand>,
    mpsc::Receiver<ProviderEvent>,
    tokio::task::JoinHandle<Result<(), ProviderAdapterError>>,
)
where
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (commands, command_rx) = mpsc::channel(8);
    let (event_tx, events) = mpsc::channel(32);
    let run = tokio::spawn(run_kimi_session(
        peer,
        command_rx,
        event_tx,
        input,
        CancellationToken::new(),
    ));
    (commands, events, run)
}

#[tokio::test]
async fn supervised_tool_approval_approved_maps_to_allow_once() {
    let (peer, server) = test_peer();
    let mut session_input = input(None, 10);
    session_input.permission_mode = ProviderPermissionMode::Supervised;
    let (commands, mut events, run) = direct_session_events(peer, session_input).await;
    let server_task = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = tokio::io::BufReader::new(reader);
        let initialize = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({
                "jsonrpc":"2.0", "id":initialize["id"],
                "result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}
            })).await;
        let _initialized = read_request(&mut reader).await;
        let new = read_request(&mut reader).await;
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":new["id"], "result":{"sessionId":"approval_fixture"}
            }),
        )
        .await;
        let prompt = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({
                "jsonrpc":"2.0", "id":"approval-request", "method":"session/request_permission",
                "params":{
                    "options":[
                        {"optionId":"allow-once","name":"Allow once","kind":"allow_once"},
                        {"optionId":"allow-always","name":"Allow always","kind":"allow_always"},
                        {"optionId":"reject-once","name":"Reject once","kind":"reject_once"}
                    ],
                    "toolCall":{"toolCallId":"tool-approval","title":"Bash","content":{"type":"text","text":"pwd"}}
                }
            })).await;
        let reply = read_request(&mut reader).await;
        assert_eq!(reply["id"], "approval-request");
        assert_eq!(
            reply["result"]["outcome"],
            serde_json::json!({"outcome":"selected","optionId":"allow-once"})
        );
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":prompt["id"], "result":{"stopReason":"end_turn"}
            }),
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    });

    let permission_id = match tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
        .await
        .expect("permission request event")
        .expect("permission request value")
    {
        ProviderEvent::StatusChanged(_) => match events.recv().await.expect("permission event") {
            ProviderEvent::PermissionRequest(request) => {
                assert_eq!(request.tool_name, "Bash");
                assert_eq!(request.description, "pwd");
                request.id
            }
            other => panic!("unexpected provider event: {other:?}"),
        },
        ProviderEvent::PermissionRequest(request) => request.id,
        other => panic!("unexpected provider event: {other:?}"),
    };
    commands
        .send(ProviderCommand::PermissionResponse {
            id: permission_id,
            approved: true,
            reason: None,
        })
        .await
        .expect("approval response");
    let mut completed = false;
    while let Some(event) = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
        .await
        .expect("terminal event")
    {
        assert!(!matches!(event, ProviderEvent::Failed { .. }));
        if matches!(event, ProviderEvent::Completed(_)) {
            completed = true;
            break;
        }
    }
    assert!(completed);
    assert!(run.await.expect("run join").is_ok());
    server_task.await.expect("server task");
}

#[tokio::test]
async fn supervised_tool_approval_rejected_maps_to_reject_once_and_continues() {
    let (peer, server) = test_peer();
    let mut session_input = input(None, 10);
    session_input.permission_mode = ProviderPermissionMode::Supervised;
    let (commands, mut events, run) = direct_session_events(peer, session_input).await;
    let server_task = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = tokio::io::BufReader::new(reader);
        let initialize = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({
                "jsonrpc":"2.0", "id":initialize["id"],
                "result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}
            })).await;
        let _initialized = read_request(&mut reader).await;
        let new = read_request(&mut reader).await;
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":new["id"], "result":{"sessionId":"rejection_fixture"}
            }),
        )
        .await;
        let prompt = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({
                "jsonrpc":"2.0", "id":42, "method":"session/request_permission",
                "params":{
                    "options":[
                        {"optionId":"allow-once","name":"Allow once","kind":"allow_once"},
                        {"optionId":"reject-once","name":"Reject once","kind":"reject_once"}
                    ],
                    "toolCall":{"toolCallId":"tool-rejection","title":"Write","content":{"type":"text","text":"update file"}}
                }
            })).await;
        let reply = read_request(&mut reader).await;
        assert_eq!(reply["id"], 42);
        assert_eq!(
            reply["result"]["outcome"],
            serde_json::json!({"outcome":"selected","optionId":"reject-once"})
        );
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":prompt["id"], "result":{"stopReason":"end_turn"}
            }),
        )
        .await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    });

    let permission_id = loop {
        match tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
            .await
            .expect("permission request event")
            .expect("permission request value")
        {
            ProviderEvent::PermissionRequest(request) => break request.id,
            ProviderEvent::StatusChanged(_) => {}
            other => panic!("unexpected provider event: {other:?}"),
        }
    };
    commands
        .send(ProviderCommand::PermissionResponse {
            id: permission_id,
            approved: false,
            reason: Some("not now".to_string()),
        })
        .await
        .expect("rejection response");
    let mut completed = false;
    while let Some(event) = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
        .await
        .expect("terminal event")
    {
        assert!(!matches!(event, ProviderEvent::Failed { .. }));
        if matches!(event, ProviderEvent::Completed(_)) {
            completed = true;
            break;
        }
    }
    assert!(completed, "rejection must let the session continue");
    assert!(run.await.expect("run join").is_ok());
    server_task.await.expect("server task");
}

#[tokio::test]
async fn auto_mode_skips_permission_request_for_normal_tools() {
    let provider = KimiCodeProvider::new(fixture_command("kimi_acp_auto_permission_fixture.sh"));
    let mut session = provider
        .start(input(None, 10), CancellationToken::new())
        .await
        .expect("start");
    let events = terminal_events(&mut session).await;
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, ProviderEvent::PermissionRequest(_)))
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, ProviderEvent::Completed(_)))
    );
}

#[tokio::test]
async fn askuserquestion_maps_to_choice_request() {
    let (peer, server) = test_peer();
    let (commands, mut events, run) = direct_session_events(peer, input(None, 10)).await;
    let server_task = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = tokio::io::BufReader::new(reader);
        let initialize = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({
                "jsonrpc":"2.0", "id":initialize["id"],
                "result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}
            })).await;
        let _initialized = read_request(&mut reader).await;
        let new = read_request(&mut reader).await;
        send_message(
            &mut writer,
            serde_json::json!({
                "jsonrpc":"2.0", "id":new["id"], "result":{"sessionId":"ask_fixture"}
            }),
        )
        .await;
        let _prompt = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({
                "jsonrpc":"2.0", "id":"ask-1", "method":"session/request_permission",
                "params":{
                    "options":[
                        {"optionId":"yes","name":"Yes","kind":"allow_once"},
                        {"optionId":"no","name":"No","kind":"reject_once"}
                    ],
                    "toolCall":{"toolCallId":"question-tool","title":"AskUserQuestion","content":{"type":"text","text":"Continue?"}}
                }
            })).await;
    });
    loop {
        match tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
            .await
            .expect("choice event")
            .expect("choice event value")
        {
            ProviderEvent::ChoiceRequest(request) => {
                assert_eq!(request.id, "ask-1");
                assert_eq!(request.prompt, "Continue?");
                assert_eq!(
                    request.source,
                    crate::cross_cutting::streaming_provider::ChoiceRequestSource::AskUserQuestion
                );
                assert_eq!(request.options.len(), 2);
                assert!(request.allow_free_text);
                assert!(!request.allow_multiple);
                commands
                    .send(ProviderCommand::ChoiceResponse {
                        id: request.id,
                        selected_option_ids: vec!["yes".to_string()],
                        free_text: None,
                        answers: vec![],
                    })
                    .await
                    .expect("choice response");
                break;
            }
            ProviderEvent::StatusChanged(_) => {}
            other => panic!("unexpected provider event: {other:?}"),
        }
    }
    tokio::time::timeout(std::time::Duration::from_secs(1), server_task)
        .await
        .expect("server must receive choice response")
        .expect("server task");
    drop(commands);
    assert!(run.await.expect("run join").is_err());
}

#[tokio::test]
async fn askuserquestion_select_option_returns_selected_and_continues() {
    let (peer, server) = test_peer();
    let (commands, mut events, run) = direct_session_events(peer, input(None, 10)).await;
    let server_task = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = tokio::io::BufReader::new(reader);
        let initialize = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":initialize["id"],"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}})).await;
        let _initialized = read_request(&mut reader).await;
        let new = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":new["id"],"result":{"sessionId":"ask_select_fixture"}})).await;
        let prompt = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":"ask-select","method":"session/request_permission","params":{"options":[{"optionId":"choice-a","name":"Choice A","kind":"allow_once"}],"toolCall":{"toolCallId":"question-tool","title":"AskUserQuestion","content":{"type":"text","text":"Pick one"}}}})).await;
        let reply = read_request(&mut reader).await;
        assert_eq!(reply["id"], "ask-select");
        assert_eq!(reply["result"]["outcome"], serde_json::json!({"outcome":"selected","optionId":"choice-a"}));
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":prompt["id"],"result":{"stopReason":"end_turn"}})).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    });
    let choice = loop {
        match tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
            .await
            .expect("choice event")
            .expect("event")
        {
            ProviderEvent::ChoiceRequest(request) => break request,
            ProviderEvent::StatusChanged(_) => {}
            other => panic!("unexpected provider event: {other:?}"),
        }
    };
    commands
        .send(ProviderCommand::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec!["choice-a".to_string()],
            free_text: None,
            answers: vec![],
        })
        .await
        .expect("choice response");
    let mut completed = false;
    while let Some(event) = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
        .await
        .expect("terminal event")
    {
        assert!(!matches!(event, ProviderEvent::Failed { .. }));
        if matches!(event, ProviderEvent::Completed(_)) {
            completed = true;
            break;
        }
    }
    assert!(completed);
    assert!(run.await.expect("run join").is_ok());
    server_task.await.expect("server task");
}

#[tokio::test]
async fn askuserquestion_free_text_takes_priority_over_selected() {
    let (peer, server) = test_peer();
    let (commands, mut events, run) = direct_session_events(peer, input(None, 10)).await;
    let server_task = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = tokio::io::BufReader::new(reader);
        let initialize = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":initialize["id"],"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}})).await;
        let _initialized = read_request(&mut reader).await;
        let new = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":new["id"],"result":{"sessionId":"ask_text_fixture"}})).await;
        let _first_prompt = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":"ask-text","method":"session/request_permission","params":{"options":[{"optionId":"selected","name":"Selected","kind":"allow_once"}],"toolCall":{"toolCallId":"question-tool","title":"AskUserQuestion","content":{"type":"text","text":"What?"}}}})).await;
        let cancel = read_request(&mut reader).await;
        assert_eq!(cancel["id"], "ask-text");
        assert_eq!(cancel["result"]["outcome"], serde_json::json!({"outcome":"cancelled"}));
        let second_prompt = read_request(&mut reader).await;
        assert_eq!(second_prompt["method"], "session/prompt");
        assert_eq!(
            second_prompt["params"]["prompt"][0]["text"],
            "custom answer"
        );
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"ask_text_fixture","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"final answer"}}}})).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":second_prompt["id"],"result":{"stopReason":"end_turn"}})).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    });
    let choice = loop {
        match tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
            .await
            .expect("choice event")
            .expect("event")
        {
            ProviderEvent::ChoiceRequest(request) => break request,
            ProviderEvent::StatusChanged(_) => {}
            other => panic!("unexpected provider event: {other:?}"),
        }
    };
    commands
        .send(ProviderCommand::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec!["selected".to_string()],
            free_text: Some(" custom answer ".to_string()),
            answers: vec![],
        })
        .await
        .expect("choice response");
    let mut terminal_events = Vec::new();
    while let Some(event) = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
        .await
        .expect("terminal event")
    {
        terminal_events.push(event.clone());
        if matches!(event, ProviderEvent::Completed(_)) {
            break;
        }
    }
    assert_eq!(
        terminal_events
            .iter()
            .filter(|event| matches!(
                event,
                ProviderEvent::Completed(_) | ProviderEvent::Failed { .. }
            ))
            .count(),
        1
    );
    assert!(terminal_events.iter().any(|event| matches!(event, ProviderEvent::Completed(completion) if completion.full_output == "final answer")));
    assert!(run.await.expect("run join").is_ok());
    server_task.await.expect("server task");
}

#[tokio::test]
async fn askuserquestion_free_text_only_no_selected() {
    let (peer, server) = test_peer();
    let (commands, mut events, run) = direct_session_events(peer, input(None, 10)).await;
    let server_task = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = tokio::io::BufReader::new(reader);
        let initialize = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":initialize["id"],"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}})).await;
        let _initialized = read_request(&mut reader).await;
        let new = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":new["id"],"result":{"sessionId":"ask_text_only_fixture"}})).await;
        let _first_prompt = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":"ask-text-only","method":"session/request_permission","params":{"options":[],"toolCall":{"toolCallId":"question-tool","title":"AskUserQuestion","content":{"type":"text","text":"What?"}}}})).await;
        let cancel = read_request(&mut reader).await;
        assert_eq!(cancel["result"]["outcome"], serde_json::json!({"outcome":"cancelled"}));
        let second_prompt = read_request(&mut reader).await;
        assert_eq!(
            second_prompt["params"]["prompt"][0]["text"],
            "free text only"
        );
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":second_prompt["id"],"result":{"stopReason":"end_turn"}})).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    });
    let choice = loop {
        match tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
            .await
            .expect("choice event")
            .expect("event")
        {
            ProviderEvent::ChoiceRequest(request) => break request,
            ProviderEvent::StatusChanged(_) => {}
            other => panic!("unexpected provider event: {other:?}"),
        }
    };
    commands
        .send(ProviderCommand::ChoiceResponse {
            id: choice.id,
            selected_option_ids: vec![],
            free_text: Some("free text only".to_string()),
            answers: vec![],
        })
        .await
        .expect("choice response");
    while let Some(event) = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
        .await
        .expect("terminal event")
    {
        if matches!(event, ProviderEvent::Completed(_)) {
            break;
        }
    }
    assert!(run.await.expect("run join").is_ok());
    server_task.await.expect("server task");
}

#[tokio::test]
async fn multiquestion_serial_one_at_a_time() {
    let (peer, server) = test_peer();
    let (commands, mut events, run) = direct_session_events(peer, input(None, 10)).await;
    let server_task = tokio::spawn(async move {
        let (reader, mut writer) = tokio::io::split(server);
        let mut reader = tokio::io::BufReader::new(reader);
        let initialize = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":initialize["id"],"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}})).await;
        let _initialized = read_request(&mut reader).await;
        let new = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":new["id"],"result":{"sessionId":"multi_question_fixture"}})).await;
        let prompt = read_request(&mut reader).await;
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":"ask-q1","method":"session/request_permission","params":{"options":[{"optionId":"q1-a","name":"Option A","kind":"allow_once"}],"toolCall":{"toolCallId":"ask-q1-tool","title":"AskUserQuestion","content":{"type":"text","text":"Question 1?"}}}})).await;
        let first_reply = read_request(&mut reader).await;
        assert_eq!(first_reply["result"]["outcome"]["optionId"], "q1-a");
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":"ask-q2","method":"session/request_permission","params":{"options":[{"optionId":"q2-b","name":"Option B","kind":"allow_once"}],"toolCall":{"toolCallId":"ask-q2-tool","title":"AskUserQuestion","content":{"type":"text","text":"Question 2?"}}}})).await;
        let second_reply = read_request(&mut reader).await;
        assert_eq!(second_reply["result"]["outcome"]["optionId"], "q2-b");
        send_message(&mut writer, serde_json::json!({"jsonrpc":"2.0","id":prompt["id"],"result":{"stopReason":"end_turn"}})).await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    });

    let first_choice = loop {
        match tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
            .await
            .expect("first choice event")
            .expect("event")
        {
            ProviderEvent::ChoiceRequest(request) => break request,
            ProviderEvent::StatusChanged(_) => {}
            other => panic!("unexpected provider event: {other:?}"),
        }
    };
    assert_eq!(first_choice.prompt, "Question 1?");
    assert!(!first_choice.allow_multiple);
    commands
        .send(ProviderCommand::ChoiceResponse {
            id: first_choice.id,
            selected_option_ids: vec!["q1-a".to_string()],
            free_text: None,
            answers: vec![],
        })
        .await
        .expect("first response");

    let second_choice = match tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
        .await
        .expect("second choice event")
        .expect("event")
    {
        ProviderEvent::ChoiceRequest(request) => request,
        other => panic!("second question must be a distinct ChoiceRequest, got {other:?}"),
    };
    assert_eq!(second_choice.prompt, "Question 2?");
    assert!(!second_choice.allow_multiple);
    commands
        .send(ProviderCommand::ChoiceResponse {
            id: second_choice.id,
            selected_option_ids: vec!["q2-b".to_string()],
            free_text: None,
            answers: vec![],
        })
        .await
        .expect("second response");

    while let Some(event) = tokio::time::timeout(std::time::Duration::from_secs(1), events.recv())
        .await
        .expect("terminal event")
    {
        if matches!(event, ProviderEvent::Completed(_)) {
            break;
        }
    }
    assert!(run.await.expect("run join").is_ok());
    server_task.await.expect("server task");
}
