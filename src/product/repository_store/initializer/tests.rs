use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::ClaudeRepositoryInitializer;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::provider_availability_gate::{
    ProviderAvailabilityGate, ProviderHealthSource,
};
use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::streaming_provider::{
    ChoiceRequestData, ChoiceRequestSource, PermissionRequestData, ProviderCommand,
    ProviderCompletion, ProviderEvent, ProviderPermissionMode, ProviderSession, ProviderStatus,
    RiskLevel, StreamingProviderAdapter, StreamingProviderInput,
};
use crate::product::models::ProviderName;
use crate::product::repository_store::{
    RepositoryInitializationProgress, RepositoryInitializationStepKind, RepositoryRegistrationError,
};

struct FixedHealthSource(Arc<ProviderHealthSnapshot>);

impl ProviderHealthSource for FixedHealthSource {
    fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
        self.0.clone()
    }

    fn degraded(&self) -> bool {
        false
    }
}

struct MutableHealthSource {
    available: Arc<AtomicBool>,
}

impl ProviderHealthSource for MutableHealthSource {
    fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
        Arc::new(ProviderHealthSnapshot {
            schema_version: 1,
            generation: 1,
            checked_at: chrono::Utc::now(),
            providers: vec![ProviderHealthEntry {
                provider: ProviderName::ClaudeCode,
                command: "claude --version".to_string(),
                available: self.available.load(Ordering::SeqCst),
                reason_code: None,
                reason: Some("mutable availability".to_string()),
                version: Some("1.0.0".to_string()),
                checked_at: chrono::Utc::now(),
            }],
        })
    }

    fn degraded(&self) -> bool {
        false
    }
}

#[derive(Default)]
struct RecordingProvider {
    inputs: Mutex<Vec<StreamingProviderInput>>,
}

#[derive(Default)]
struct RecordingProgress {
    events: Mutex<Vec<(RepositoryInitializationStepKind, &'static str)>>,
}

impl RepositoryInitializationProgress for RecordingProgress {
    fn step_started(
        &self,
        step: RepositoryInitializationStepKind,
    ) -> Result<(), Box<RepositoryRegistrationError>> {
        self.events.lock().unwrap().push((step, "started"));
        Ok(())
    }

    fn step_completed(
        &self,
        step: RepositoryInitializationStepKind,
    ) -> Result<(), Box<RepositoryRegistrationError>> {
        self.events.lock().unwrap().push((step, "completed"));
        Ok(())
    }
}

enum SessionScript {
    Events(Vec<ProviderEvent>),
    Pending,
}

struct ScriptedProvider {
    inputs: Mutex<Vec<StreamingProviderInput>>,
    scripts: Mutex<Vec<SessionScript>>,
    commands: Arc<Mutex<Vec<ProviderCommand>>>,
    pending_senders: Mutex<Vec<mpsc::Sender<ProviderEvent>>>,
    disable_after_first_start: Option<Arc<AtomicBool>>,
}

struct FailingStartProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for FailingStartProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            format!("API_TOKEN=secret {}", "x".repeat(128)),
            0,
        ))
    }
}

struct FullCommandChannelProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for FullCommandChannelProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let (event_tx, event_rx) = mpsc::channel(1);
        let (command_tx, command_rx) = mpsc::channel(1);
        tokio::spawn(async move {
            std::future::pending::<()>().await;
            drop(command_rx);
        });
        command_tx.send(ProviderCommand::Abort).await.unwrap();
        event_tx
            .send(ProviderEvent::PermissionRequest(PermissionRequestData {
                id: "permission-1".to_string(),
                tool_name: "write".to_string(),
                description: "approve".to_string(),
                risk_level: RiskLevel::High,
            }))
            .await
            .unwrap();
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

impl ScriptedProvider {
    fn new(scripts: Vec<SessionScript>) -> Self {
        Self {
            inputs: Mutex::new(Vec::new()),
            scripts: Mutex::new(scripts),
            commands: Arc::new(Mutex::new(Vec::new())),
            pending_senders: Mutex::new(Vec::new()),
            disable_after_first_start: None,
        }
    }

    fn disabling_after_first_start(available: Arc<AtomicBool>) -> Self {
        Self {
            disable_after_first_start: Some(available),
            ..Self::new(vec![SessionScript::Events(vec![ProviderEvent::Completed(
                ProviderCompletion::plain("completed", None),
            )])])
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ScriptedProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.inputs.lock().unwrap().push(input);
        let script = self.scripts.lock().unwrap().remove(0);
        let (event_tx, event_rx) = mpsc::channel(16);
        let (command_tx, mut command_rx) = mpsc::channel(16);
        let commands = self.commands.clone();
        tokio::spawn(async move {
            while let Some(command) = command_rx.recv().await {
                commands.lock().unwrap().push(command);
            }
        });
        match script {
            SessionScript::Events(events) => {
                for event in events {
                    event_tx.send(event).await.unwrap();
                }
            }
            SessionScript::Pending => {
                self.pending_senders.lock().unwrap().push(event_tx);
            }
        }
        if self.inputs.lock().unwrap().len() == 1
            && let Some(available) = &self.disable_after_first_start
        {
            available.store(false, Ordering::SeqCst);
        }
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for RecordingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.inputs.lock().unwrap().push(input);
        let (event_tx, event_rx) = mpsc::channel(2);
        let (command_tx, _command_rx) = mpsc::channel(2);
        event_tx
            .send(ProviderEvent::Completed(ProviderCompletion::plain(
                "completed",
                Some("provider-session".to_string()),
            )))
            .await
            .unwrap();
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

fn available_gate() -> Arc<ProviderAvailabilityGate> {
    Arc::new(ProviderAvailabilityGate::new(Arc::new(FixedHealthSource(
        Arc::new(ProviderHealthSnapshot {
            schema_version: 1,
            generation: 1,
            checked_at: chrono::Utc::now(),
            providers: vec![ProviderHealthEntry {
                provider: ProviderName::ClaudeCode,
                command: "claude --version".to_string(),
                available: true,
                reason_code: None,
                reason: None,
                version: Some("1.0.0".to_string()),
                checked_at: chrono::Utc::now(),
            }],
        }),
    ))))
}

fn scripted_initializer(
    provider: Arc<ScriptedProvider>,
    output_limit: usize,
) -> ClaudeRepositoryInitializer {
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, provider);
    ClaudeRepositoryInitializer::new(available_gate(), Arc::new(registry), output_limit)
}

#[tokio::test]
async fn repository_initializer_runs_four_independent_claude_turns_in_strict_order() {
    let provider = Arc::new(RecordingProvider::default());
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, provider.clone());
    let initializer = ClaudeRepositoryInitializer::new(available_gate(), Arc::new(registry), 1024);
    let git_root = PathBuf::from("/tmp/example-repository");
    let progress = Arc::new(RecordingProgress::default());

    let summaries = initializer
        .initialize(
            &git_root,
            Duration::from_secs(1),
            CancellationToken::new(),
            progress.clone(),
        )
        .await
        .unwrap();

    let inputs = provider.inputs.lock().unwrap();
    assert_eq!(inputs.len(), 4);
    assert_eq!(
        inputs
            .iter()
            .map(|input| input.prompt.as_str())
            .collect::<Vec<_>>(),
        vec![
            "/rule-config --no-interrupt",
            "/pre-check --no-interrupt",
            "/mcp-configuration --no-interrupt",
            "/project-rules-examples --no-interrupt",
        ]
    );
    for input in inputs.iter() {
        assert_eq!(
            input.provider_type,
            crate::protocol::contracts::ProviderType::ClaudeCode
        );
        assert_eq!(
            input.role,
            crate::protocol::contracts::AdapterRole::Executor
        );
        assert_eq!(input.working_dir, git_root);
        assert_eq!(input.permission_mode, ProviderPermissionMode::Auto);
        assert_eq!(input.resume_provider_session_id, None);
        assert_eq!(input.workspace_session_id, None);
        assert_eq!(input.env_vars, BTreeMap::new());
    }
    assert_eq!(
        summaries
            .iter()
            .map(|summary| (
                summary.command_index,
                summary.command.as_str(),
                summary.status.as_str()
            ))
            .collect::<Vec<_>>(),
        vec![
            (1, "/rule-config --no-interrupt", "completed"),
            (2, "/pre-check --no-interrupt", "completed"),
            (3, "/mcp-configuration --no-interrupt", "completed"),
            (4, "/project-rules-examples --no-interrupt", "completed"),
        ]
    );
    assert_eq!(
        progress.events.lock().unwrap().as_slice(),
        &[
            (RepositoryInitializationStepKind::RuleConfig, "started"),
            (RepositoryInitializationStepKind::RuleConfig, "completed"),
            (RepositoryInitializationStepKind::PreCheck, "started"),
            (RepositoryInitializationStepKind::PreCheck, "completed"),
            (
                RepositoryInitializationStepKind::McpConfiguration,
                "started"
            ),
            (
                RepositoryInitializationStepKind::McpConfiguration,
                "completed"
            ),
            (
                RepositoryInitializationStepKind::ProjectRulesExamples,
                "started"
            ),
            (
                RepositoryInitializationStepKind::ProjectRulesExamples,
                "completed"
            ),
        ],
    );
}

#[tokio::test]
async fn repository_initializer_reports_only_started_steps_when_second_turn_fails() {
    let provider = Arc::new(ScriptedProvider::new(vec![
        SessionScript::Events(vec![ProviderEvent::Completed(ProviderCompletion::plain(
            "completed",
            None,
        ))]),
        SessionScript::Events(vec![ProviderEvent::Failed {
            message: "failed".to_string(),
        }]),
    ]));
    let progress = Arc::new(RecordingProgress::default());

    let error = scripted_initializer(provider.clone(), 1024)
        .initialize(
            &PathBuf::from("/tmp/repository"),
            Duration::from_secs(1),
            CancellationToken::new(),
            progress.clone(),
        )
        .await
        .unwrap_err();

    assert_eq!(error.command_index, Some(2));
    assert_eq!(provider.inputs.lock().unwrap().len(), 2);
    assert_eq!(
        progress.events.lock().unwrap().as_slice(),
        &[
            (RepositoryInitializationStepKind::RuleConfig, "started"),
            (RepositoryInitializationStepKind::RuleConfig, "completed"),
            (RepositoryInitializationStepKind::PreCheck, "started"),
        ],
    );
}

#[tokio::test]
async fn repository_initializer_stops_after_terminal_failure_events() {
    let cases = vec![
        ProviderEvent::Failed {
            message: "failed".to_string(),
        },
        ProviderEvent::ProtocolError {
            code: "bad_protocol".to_string(),
            message: "broken".to_string(),
            context: None,
        },
        ProviderEvent::PermissionTimeout {
            permission_id: "permission-1".to_string(),
        },
        ProviderEvent::StatusChanged(ProviderStatus::Failed),
        ProviderEvent::StatusChanged(ProviderStatus::Aborted),
    ];

    for event in cases {
        let provider = Arc::new(ScriptedProvider::new(vec![SessionScript::Events(vec![
            event,
        ])]));
        let error = scripted_initializer(provider.clone(), 1024)
            .initialize(
                &PathBuf::from("/tmp/repository"),
                Duration::from_secs(1),
                CancellationToken::new(),
                Arc::new(RecordingProgress::default()),
            )
            .await
            .unwrap_err();
        assert_eq!(error.reason_code, "repository_init_command_failed");
        assert_eq!(error.command_index, Some(1));
        assert_eq!(provider.inputs.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn repository_initializer_rejects_stream_close_without_completed() {
    let provider = Arc::new(ScriptedProvider::new(vec![SessionScript::Events(
        Vec::new(),
    )]));
    let error = scripted_initializer(provider.clone(), 1024)
        .initialize(
            &PathBuf::from("/tmp/repository"),
            Duration::from_secs(1),
            CancellationToken::new(),
            Arc::new(RecordingProgress::default()),
        )
        .await
        .unwrap_err();

    assert_eq!(error.reason_code, "repository_init_command_failed");
    assert!(
        error
            .stderr_summary
            .unwrap()
            .contains("closed before completion")
    );
    assert_eq!(provider.inputs.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn repository_initializer_times_out_and_aborts_the_current_turn() {
    let provider = Arc::new(ScriptedProvider::new(vec![SessionScript::Pending]));
    let error = scripted_initializer(provider.clone(), 1024)
        .initialize(
            &PathBuf::from("/tmp/repository"),
            Duration::from_millis(20),
            CancellationToken::new(),
            Arc::new(RecordingProgress::default()),
        )
        .await
        .unwrap_err();
    tokio::task::yield_now().await;

    assert_eq!(error.reason_code, "repository_init_command_failed");
    assert!(error.stderr_summary.unwrap().contains("timed out"));
    assert_eq!(
        provider.commands.lock().unwrap().as_slice(),
        &[ProviderCommand::Abort]
    );
}

#[tokio::test]
async fn repository_initializer_aborts_unexpected_permission_and_choice_requests() {
    let interaction_events = vec![
        ProviderEvent::PermissionRequest(PermissionRequestData {
            id: "permission-1".to_string(),
            tool_name: "write".to_string(),
            description: "approve".to_string(),
            risk_level: RiskLevel::High,
        }),
        ProviderEvent::ChoiceRequest(ChoiceRequestData {
            id: "choice-1".to_string(),
            prompt: "choose".to_string(),
            options: Vec::new(),
            allow_multiple: false,
            allow_free_text: false,
            questions: Vec::new(),
            source: ChoiceRequestSource::ProviderChoice,
        }),
    ];

    for event in interaction_events {
        let provider = Arc::new(ScriptedProvider::new(vec![SessionScript::Events(vec![
            event,
        ])]));
        let progress = Arc::new(RecordingProgress::default());
        let error = scripted_initializer(provider.clone(), 1024)
            .initialize(
                &PathBuf::from("/tmp/repository"),
                Duration::from_secs(1),
                CancellationToken::new(),
                progress.clone(),
            )
            .await
            .unwrap_err();
        tokio::task::yield_now().await;

        assert_eq!(error.reason_code, "repository_init_interaction_required");
        assert_eq!(
            provider.commands.lock().unwrap().as_slice(),
            &[ProviderCommand::Abort]
        );
        assert_eq!(provider.inputs.lock().unwrap().len(), 1);
        assert_eq!(
            progress.events.lock().unwrap().as_slice(),
            &[(RepositoryInitializationStepKind::RuleConfig, "started")],
        );
    }
}

#[tokio::test]
async fn repository_initializer_sanitizes_secrets_controls_and_long_output() {
    let provider = Arc::new(ScriptedProvider::new(vec![SessionScript::Events(vec![
        ProviderEvent::TextDelta {
            content: format!("\u{1b}[31mAPI_KEY=secret\u{0} {}", "x".repeat(128)),
        },
        ProviderEvent::Failed {
            message: "failed".to_string(),
        },
    ])]));
    let error = scripted_initializer(provider, 32)
        .initialize(
            &PathBuf::from("/tmp/repository"),
            Duration::from_secs(1),
            CancellationToken::new(),
            Arc::new(RecordingProgress::default()),
        )
        .await
        .unwrap_err();
    let summary = error.stderr_summary.unwrap();

    assert!(!summary.contains("secret"));
    assert!(!summary.contains('\u{1b}'));
    assert!(!summary.contains('\u{0}'));
    assert!(summary.contains("[REDACTED]"));
    assert!(summary.contains("[truncated]"));
}

#[tokio::test]
async fn repository_initializer_rechecks_claude_gate_before_every_turn() {
    let available = Arc::new(AtomicBool::new(true));
    let provider = Arc::new(ScriptedProvider::disabling_after_first_start(
        available.clone(),
    ));
    let gate = Arc::new(ProviderAvailabilityGate::new(Arc::new(
        MutableHealthSource { available },
    )));
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, provider.clone());
    let initializer = ClaudeRepositoryInitializer::new(gate, Arc::new(registry), 1024);

    let error = initializer
        .initialize(
            &PathBuf::from("/tmp/repository"),
            Duration::from_secs(1),
            CancellationToken::new(),
            Arc::new(RecordingProgress::default()),
        )
        .await
        .unwrap_err();

    assert_eq!(error.reason_code, "provider_unavailable");
    assert_eq!(error.command_index, Some(2));
    assert_eq!(provider.inputs.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn repository_initializer_bounds_and_sanitizes_session_start_failures() {
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, Arc::new(FailingStartProvider));
    let initializer = ClaudeRepositoryInitializer::new(available_gate(), Arc::new(registry), 32);

    let error = initializer
        .initialize(
            &PathBuf::from("/tmp/repository"),
            Duration::from_secs(1),
            CancellationToken::new(),
            Arc::new(RecordingProgress::default()),
        )
        .await
        .unwrap_err();
    let summary = error.stderr_summary.unwrap();

    assert!(!summary.contains("secret"));
    assert!(summary.contains("[REDACTED]"));
    assert!(summary.contains("[truncated]"));
    assert!(summary.len() < 80);
}

#[tokio::test]
async fn repository_initializer_does_not_block_when_best_effort_abort_channel_is_full() {
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(FullCommandChannelProvider),
    );
    let initializer = ClaudeRepositoryInitializer::new(available_gate(), Arc::new(registry), 1024);

    let error = tokio::time::timeout(
        Duration::from_millis(100),
        initializer.initialize(
            &PathBuf::from("/tmp/repository"),
            Duration::from_secs(1),
            CancellationToken::new(),
            Arc::new(RecordingProgress::default()),
        ),
    )
    .await
    .expect("best-effort abort must not block")
    .unwrap_err();

    assert_eq!(error.reason_code, "repository_init_interaction_required");
}
