use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::protocol::contracts::{AdapterInput, AdapterRole, ProviderType};

#[derive(Debug, Clone)]
pub enum StreamChunk {
    Text(String),
    Done { full_output: String },
    Error(String),
}

pub struct StreamingRunHandle {
    pub receiver: mpsc::Receiver<StreamChunk>,
    pub cancel: CancellationToken,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderPermissionMode {
    Auto,
    Supervised,
}

#[derive(Debug, Clone)]
pub struct StreamingProviderInput {
    pub provider_type: ProviderType,
    pub role: AdapterRole,
    pub prompt: String,
    pub working_dir: PathBuf,
    pub session_id: Option<String>,
    pub permission_mode: ProviderPermissionMode,
    pub env_vars: BTreeMap<String, String>,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequestData {
    pub id: String,
    pub tool_name: String,
    pub description: String,
    pub risk_level: RiskLevel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderStatus {
    Starting,
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderEvent {
    TextDelta {
        content: String,
    },
    PermissionRequest(PermissionRequestData),
    StatusChanged(ProviderStatus),
    Completed {
        full_output: String,
        provider_session_id: Option<String>,
    },
    Failed {
        message: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderCommand {
    PermissionResponse {
        id: String,
        approved: bool,
        reason: Option<String>,
    },
    Abort,
}

pub struct ProviderSession {
    pub events: mpsc::Receiver<ProviderEvent>,
    pub commands: mpsc::Sender<ProviderCommand>,
}

const FAKE_STREAMING_STEP_DELAY: Duration = Duration::from_millis(10);

async fn fake_streaming_should_stop(
    cancel: &CancellationToken,
    command_rx: &mut mpsc::Receiver<ProviderCommand>,
    commands_open: &mut bool,
) -> bool {
    let delay = tokio::time::sleep(FAKE_STREAMING_STEP_DELAY);
    tokio::pin!(delay);

    loop {
        if *commands_open {
            tokio::select! {
                _ = cancel.cancelled() => return true,
                command = command_rx.recv() => {
                    match command {
                        Some(ProviderCommand::Abort) => return true,
                        Some(ProviderCommand::PermissionResponse { .. }) => {}
                        None => *commands_open = false,
                    }
                }
                _ = &mut delay => return false,
            }
        } else {
            tokio::select! {
                _ = cancel.cancelled() => return true,
                _ = &mut delay => return false,
            }
        }
    }
}

async fn fake_streaming_send_event(
    event_tx: &mpsc::Sender<ProviderEvent>,
    event: ProviderEvent,
    cancel: &CancellationToken,
    command_rx: &mut mpsc::Receiver<ProviderCommand>,
    commands_open: &mut bool,
) -> bool {
    loop {
        if *commands_open {
            tokio::select! {
                _ = cancel.cancelled() => return false,
                permit = event_tx.reserve() => {
                    match permit {
                        Ok(permit) => {
                            permit.send(event);
                            return true;
                        }
                        Err(_) => return false,
                    }
                }
                command = command_rx.recv() => {
                    match command {
                        Some(ProviderCommand::Abort) => return false,
                        Some(ProviderCommand::PermissionResponse { .. }) => {}
                        None => *commands_open = false,
                    }
                }
            }
        } else {
            tokio::select! {
                _ = cancel.cancelled() => return false,
                permit = event_tx.reserve() => {
                    match permit {
                        Ok(permit) => {
                            permit.send(event);
                            return true;
                        }
                        Err(_) => return false,
                    }
                }
            }
        }
    }
}

#[async_trait::async_trait]
pub trait StreamingProviderAdapter: Send + Sync {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "streaming provider start is not implemented",
            0,
        ))
    }

    async fn run_streaming(
        &self,
        input: &AdapterInput,
        cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError>;
}

pub struct FakeStreamingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for FakeStreamingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(32);
        let (command_tx, mut command_rx) = mpsc::channel(8);
        let prompt = input.prompt;

        tokio::spawn(async move {
            let words: Vec<&str> = prompt.split_whitespace().collect();
            let mut commands_open = true;

            for (i, word) in words.iter().enumerate() {
                if fake_streaming_should_stop(&cancel, &mut command_rx, &mut commands_open).await {
                    return;
                }

                let content = if i == 0 {
                    word.to_string()
                } else {
                    format!(" {word}")
                };
                if !fake_streaming_send_event(
                    &event_tx,
                    ProviderEvent::TextDelta { content },
                    &cancel,
                    &mut command_rx,
                    &mut commands_open,
                )
                .await
                {
                    return;
                }
            }

            if fake_streaming_should_stop(&cancel, &mut command_rx, &mut commands_open).await {
                return;
            }
            let _ = fake_streaming_send_event(
                &event_tx,
                ProviderEvent::Completed {
                    full_output: prompt,
                    provider_session_id: None,
                },
                &cancel,
                &mut command_rx,
                &mut commands_open,
            )
            .await;
        });

        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        input: &AdapterInput,
        cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        let working_dir = input.worktree_path.as_ref().map(PathBuf::from).unwrap_or(
            std::env::current_dir().map_err(|error| {
                ProviderAdapterError::execution_failed(None, String::new(), error.to_string(), 0)
            })?,
        );
        let provider_input = StreamingProviderInput {
            provider_type: input.provider_type.clone(),
            role: input.role.clone(),
            prompt: input.prompt.clone(),
            working_dir,
            session_id: None,
            permission_mode: ProviderPermissionMode::Auto,
            env_vars: BTreeMap::new(),
            timeout_secs: input.timeout,
        };
        let bridge_cancel = cancel.clone();
        let mut session = self.start(provider_input, cancel).await?;
        let (tx, rx) = mpsc::channel(32);

        tokio::spawn(async move {
            let _commands = session.commands;
            loop {
                let event = tokio::select! {
                    _ = bridge_cancel.cancelled() => return,
                    event = session.events.recv() => {
                        match event {
                            Some(event) => event,
                            None => return,
                        }
                    }
                };
                let chunk = match event {
                    ProviderEvent::TextDelta { content } => StreamChunk::Text(content),
                    ProviderEvent::Completed { full_output, .. } => {
                        StreamChunk::Done { full_output }
                    }
                    ProviderEvent::Failed { message } => StreamChunk::Error(message),
                    ProviderEvent::PermissionRequest(_) | ProviderEvent::StatusChanged(_) => {
                        continue;
                    }
                };
                tokio::select! {
                    _ = bridge_cancel.cancelled() => return,
                    send_result = tx.send(chunk) => {
                        if send_result.is_err() {
                            return;
                        }
                    }
                }
            }
        });

        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::contracts::AdapterInput;

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
            session_id: None,
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
        let input = make_input("hello world foo");

        let mut rx = provider.run_streaming(&input, cancel).await.unwrap();

        let mut texts = Vec::new();
        let mut done_output = None;

        while let Some(chunk) = rx.recv().await {
            match chunk {
                StreamChunk::Text(t) => texts.push(t),
                StreamChunk::Done { full_output } => {
                    done_output = Some(full_output);
                    break;
                }
                StreamChunk::Error(_) => panic!("unexpected error"),
            }
        }

        assert_eq!(texts, vec!["hello", " world", " foo"]);
        assert_eq!(done_output.unwrap(), "hello world foo");
    }

    #[tokio::test]
    async fn fake_streaming_provider_session_emits_text_and_completed() {
        let provider = FakeStreamingProvider;
        let cancel = CancellationToken::new();
        let input = make_provider_input("hello real stream");

        let mut session = provider.start(input, cancel).await.unwrap();
        let mut output = String::new();
        while let Some(event) = session.events.recv().await {
            match event {
                ProviderEvent::TextDelta { content } => output.push_str(&content),
                ProviderEvent::Completed { full_output, .. } => {
                    assert_eq!(full_output, "hello real stream");
                    break;
                }
                other => panic!("unexpected provider event: {other:?}"),
            }
        }
        assert_eq!(output, "hello real stream");
    }

    #[tokio::test]
    async fn fake_streaming_provider_abort_after_final_text_suppresses_completed() {
        let provider = FakeStreamingProvider;
        let cancel = CancellationToken::new();
        let input = make_provider_input("final");

        let mut session = provider.start(input, cancel).await.unwrap();
        let first = session.events.recv().await.unwrap();
        assert_eq!(
            first,
            ProviderEvent::TextDelta {
                content: "final".to_string()
            }
        );

        let _ = session.commands.send(ProviderCommand::Abort).await;

        while let Some(event) = tokio::time::timeout(TEST_TIMEOUT, session.events.recv())
            .await
            .expect("provider should close after abort")
        {
            assert_ne!(
                event,
                ProviderEvent::Completed {
                    full_output: "final".to_string(),
                    provider_session_id: None,
                },
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

        wait_for_buffer_len(&session.events, 32).await;
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

        wait_for_buffer_len(&rx, 32).await;
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
        assert!(matches!(first, StreamChunk::Text(_)));
        cancel.cancel();

        for _ in 0..9 {
            let Some(chunk) = tokio::time::timeout(TEST_TIMEOUT, rx.recv())
                .await
                .expect("provider should close after cancel")
            else {
                return;
            };
            assert!(
                !matches!(chunk, StreamChunk::Done { .. }),
                "cancelled provider should not emit a completion marker"
            );
        }

        panic!("cancelled provider should close before emitting the full stream");
    }
}
