#[cfg(unix)]
mod session_tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use tokio_util::sync::CancellationToken;

    use crate::cross_cutting::streaming_provider::{
        ProviderCommand, ProviderEvent, ProviderPermissionMode, StreamingProviderInput,
    };
    use crate::protocol::contracts::{AdapterRole, ProviderType};

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
