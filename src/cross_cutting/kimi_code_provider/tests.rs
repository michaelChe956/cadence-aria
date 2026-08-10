#[cfg(unix)]
mod session_tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use serde_json::Value;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    use tokio::sync::mpsc;

    use crate::cross_cutting::streaming_provider::{
        ProviderCommand, ProviderEvent, ProviderPermissionMode, StreamingProviderInput,
    };
    use crate::protocol::contracts::{AdapterRole, ProviderType};

    use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
    use crate::cross_cutting::provider_adapter::ProviderAdapterError;
    use tokio_util::sync::CancellationToken;

    use super::super::session::run_kimi_session;
    use super::super::{KimiCodeProvider, StreamingProviderAdapter};

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

    fn input_with_env(
        resume: Option<&str>,
        timeout_secs: u64,
        env_vars: BTreeMap<String, String>,
    ) -> StreamingProviderInput {
        StreamingProviderInput {
            env_vars,
            ..input(resume, timeout_secs)
        }
    }

    async fn terminal_events(
        session: &mut crate::cross_cutting::streaming_provider::ProviderSession,
    ) -> Vec<ProviderEvent> {
        let mut events = Vec::new();
        while let Some(event) =
            tokio::time::timeout(std::time::Duration::from_secs(3), session.events.recv())
                .await
                .expect("provider event")
        {
            let terminal = matches!(
                event,
                ProviderEvent::Completed(_) | ProviderEvent::Failed { .. }
            );
            events.push(event);
            if terminal {
                break;
            }
        }
        events
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

    async fn direct_session_events(
        peer: JsonRpcPeer<tokio::io::WriteHalf<tokio::io::DuplexStream>>,
        input: StreamingProviderInput,
    ) -> (
        mpsc::Sender<ProviderCommand>,
        mpsc::Receiver<ProviderEvent>,
        tokio::task::JoinHandle<Result<(), ProviderAdapterError>>,
    ) {
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
    async fn text_turn_completes_with_full_output() {
        let provider = KimiCodeProvider::new(fixture_command("kimi_acp_text_fixture.sh"));
        let mut session = provider
            .start(input(None, 10), CancellationToken::new())
            .await
            .expect("start");
        let events = terminal_events(&mut session).await;
        assert!(events.iter().any(|event| matches!(event, ProviderEvent::TextDelta { content } if content == "Kimi fixture output")));
        let completion = events
            .iter()
            .find_map(|event| match event {
                ProviderEvent::Completed(completion) => Some(completion),
                _ => None,
            })
            .expect("completion");
        assert_eq!(completion.full_output, "Kimi fixture output");
        assert_eq!(
            completion.provider_session_id.as_deref(),
            Some("kimi_text_fixture")
        );
    }

    #[tokio::test]
    async fn tool_call_emits_toolcall_then_toolresult_once() {
        let provider = KimiCodeProvider::new(fixture_command("kimi_acp_tool_fixture.sh"));
        let mut session = provider
            .start(input(None, 10), CancellationToken::new())
            .await
            .expect("start");
        let events = terminal_events(&mut session).await;
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, ProviderEvent::ToolCall(_)))
                .count(),
            2
        );
        let results = events
            .iter()
            .filter_map(|event| match event {
                ProviderEvent::ToolResult(result) => Some(result),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|result| result.tool_use_id == "tool_1"
            && !result.is_error
            && result.output.contains("tmp")));
        assert!(
            results
                .iter()
                .any(|result| result.tool_use_id == "tool_2" && result.is_error)
        );
    }

    #[tokio::test]
    async fn direct_load_error_does_not_send_prompt() {
        let (peer, server) = test_peer();
        let (_commands, _events, run) =
            direct_session_events(peer, input(Some("old-session"), 10)).await;
        let server_task = tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(server);
            let mut reader = tokio::io::BufReader::new(reader);
            let initialize = read_request(&mut reader).await;
            send_message(&mut writer, serde_json::json!({
                "jsonrpc":"2.0", "id":initialize["id"],
                "result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}
            })).await;
            let _initialized = read_request(&mut reader).await;
            let load = read_request(&mut reader).await;
            assert_eq!(load["method"], "session/load");
            send_message(
                &mut writer,
                serde_json::json!({
                    "jsonrpc":"2.0", "id":load["id"],
                    "error":{"code":-32001,"message":"missing session"}
                }),
            )
            .await;
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            assert!(
                tokio::time::timeout(
                    std::time::Duration::from_millis(100),
                    read_request(&mut reader)
                )
                .await
                .is_err(),
                "load error must not send session/new or session/prompt"
            );
        });
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(1), run)
                .await
                .expect("session task")
                .expect("session join")
                .is_err()
        );
        server_task.await.expect("server task");
    }

    #[tokio::test]
    async fn terminal_response_drains_prebuffered_updates_without_sleep() {
        let provider = KimiCodeProvider::new(fixture_command("kimi_acp_fast_fixture.sh"));
        let mut session = provider
            .start(input(None, 10), CancellationToken::new())
            .await
            .expect("start");
        let events = terminal_events(&mut session).await;
        let completion = events
            .iter()
            .find_map(|event| match event {
                ProviderEvent::Completed(completion) => Some(completion),
                _ => None,
            })
            .expect("completion");
        assert_eq!(completion.full_output, "first second");
        assert!(
            events.iter().any(
                |event| matches!(event, ProviderEvent::ToolCall(call) if call.id == "fast_tool")
            )
        );
        assert!(events.iter().any(|event| matches!(event, ProviderEvent::ToolResult(result) if result.tool_use_id == "fast_tool" && result.output == "ok")));
    }

    #[tokio::test]
    async fn resume_load_failure_never_falls_back_to_new_or_prompt() {
        let provider = KimiCodeProvider::new(fixture_command("kimi_acp_load_failure_fixture.sh"));
        let mut session = provider
            .start(input(Some("missing_session"), 10), CancellationToken::new())
            .await
            .expect("start");
        let events = terminal_events(&mut session).await;
        let failures = events
            .iter()
            .filter(|event| matches!(event, ProviderEvent::Failed { .. }))
            .collect::<Vec<_>>();
        assert_eq!(failures.len(), 1);
        assert!(
            matches!(failures[0], ProviderEvent::Failed { message } if message.contains("session/load failed"))
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, ProviderEvent::Completed(_)))
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn resume_uses_session_load_when_resume_id_present() {
        let provider = KimiCodeProvider::new(fixture_command("kimi_acp_resume_fixture.sh"));
        let mut session = provider
            .start(
                input(Some("existing_session"), 10),
                CancellationToken::new(),
            )
            .await
            .expect("start");
        let events = terminal_events(&mut session).await;
        let completion = events
            .iter()
            .find_map(|event| match event {
                ProviderEvent::Completed(completion) => Some(completion),
                _ => None,
            })
            .expect("completion");
        assert_eq!(
            completion.provider_session_id.as_deref(),
            Some("resumed_kimi_fixture")
        );
    }

    #[tokio::test]
    async fn early_exit_codes_have_stable_distinct_messages() {
        for (code, phrase) in [
            ("0", "code 0 before terminal prompt result"),
            ("1", "code 1 (non-retryable failure)"),
            ("75", "code 75 (temporary failure)"),
        ] {
            let mut env_vars = BTreeMap::new();
            env_vars.insert("KIMI_FIXTURE_EXIT_CODE".to_string(), code.to_string());
            let provider = KimiCodeProvider::new(fixture_command("kimi_acp_exit_fixture.sh"));
            let mut session = provider
                .start(input_with_env(None, 10, env_vars), CancellationToken::new())
                .await
                .expect("start");
            let events = terminal_events(&mut session).await;
            assert_eq!(
                events
                    .iter()
                    .filter(|event| matches!(event, ProviderEvent::Failed { .. }))
                    .count(),
                1
            );
            assert!(events.iter().any(|event| matches!(event, ProviderEvent::Failed { message } if message.contains(phrase))), "exit {code} missing {phrase}: {events:?}");
        }
    }

    #[tokio::test]
    async fn abort_sends_session_cancel_before_process_termination() {
        let marker = tempfile::NamedTempFile::new().expect("marker");
        let marker_path = marker.path().to_path_buf();
        std::fs::remove_file(&marker_path).expect("remove marker placeholder");
        let mut env_vars = BTreeMap::new();
        env_vars.insert(
            "KIMI_CANCEL_MARKER".to_string(),
            marker_path.display().to_string(),
        );
        let provider = KimiCodeProvider::new(fixture_command("kimi_acp_hanging_fixture.sh"));
        let mut session = provider
            .start(input_with_env(None, 10, env_vars), CancellationToken::new())
            .await
            .expect("start");
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        session
            .commands
            .send(ProviderCommand::Abort)
            .await
            .expect("abort command");
        loop {
            let event =
                tokio::time::timeout(std::time::Duration::from_secs(3), session.events.recv())
                    .await
                    .expect("event")
                    .expect("event value");
            if matches!(
                event,
                ProviderEvent::StatusChanged(
                    crate::cross_cutting::streaming_provider::ProviderStatus::Aborted
                )
            ) {
                break;
            }
        }
        assert!(
            marker_path.exists(),
            "ACP session/cancel must be written before process termination fallback"
        );
    }

    #[tokio::test]
    async fn abort_emits_aborted_without_failed() {
        let provider = KimiCodeProvider::new(fixture_command("kimi_acp_hanging_fixture.sh"));
        let cancel = CancellationToken::new();
        let mut session = provider
            .start(input(None, 10), cancel.clone())
            .await
            .expect("start");
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        session
            .commands
            .send(ProviderCommand::Abort)
            .await
            .expect("abort command");
        let mut saw_aborted = false;
        while let Some(event) =
            tokio::time::timeout(std::time::Duration::from_secs(3), session.events.recv())
                .await
                .expect("event")
        {
            if matches!(
                event,
                ProviderEvent::StatusChanged(
                    crate::cross_cutting::streaming_provider::ProviderStatus::Aborted
                )
            ) {
                saw_aborted = true;
                break;
            }
            assert!(!matches!(event, ProviderEvent::Failed { .. }));
        }
        assert!(saw_aborted);
    }
}
