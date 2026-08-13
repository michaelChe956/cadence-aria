use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::cross_cutting::provider_health::{
    ProviderHealthEntry, ProviderHealthReasonCode, ProviderHealthSnapshot,
};
use crate::product::models::ProviderName;
use crate::web::state::WebAppState;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderStatusResponse {
    pub schema_version: u32,
    pub generation: u64,
    pub checked_at: DateTime<Utc>,
    pub state_status: &'static str,
    pub state_error: Option<String>,
    pub real_workflow_blocked: bool,
    pub test_provider_enabled: bool,
    pub providers: Vec<ProviderStatusDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ProviderStatusDto {
    pub provider: &'static str,
    pub display_name: &'static str,
    pub available: bool,
    pub version: Option<String>,
    pub reason_code: Option<&'static str>,
    pub reason: Option<String>,
    pub checked_at: DateTime<Utc>,
    pub install_hint: &'static str,
}

pub async fn providers_status(State(state): State<WebAppState>) -> Json<ProviderStatusResponse> {
    Json(response_from_state(&state))
}

pub async fn providers_recheck(State(state): State<WebAppState>) -> Json<ProviderStatusResponse> {
    if state.test_provider_enabled {
        return Json(response_from_state(&state));
    }
    let snapshot = state.refresh_provider_health().await;
    Json(response_from_snapshot(&state, snapshot))
}

fn response_from_state(state: &WebAppState) -> ProviderStatusResponse {
    response_from_snapshot(state, state.provider_health.latest_diagnostic())
}

fn response_from_snapshot(
    state: &WebAppState,
    snapshot: Arc<ProviderHealthSnapshot>,
) -> ProviderStatusResponse {
    let degraded = state.provider_health.degraded();
    ProviderStatusResponse {
        schema_version: snapshot.schema_version,
        generation: snapshot.generation,
        checked_at: snapshot.checked_at,
        state_status: if degraded { "degraded" } else { "ready" },
        state_error: state
            .provider_health_error()
            .or_else(|| degraded.then(|| "provider health state is degraded".to_string())),
        real_workflow_blocked: degraded || snapshot.real_workflow_blocked(),
        test_provider_enabled: state.test_provider_enabled,
        providers: [
            ProviderName::ClaudeCode,
            ProviderName::Codex,
            ProviderName::Pi,
            ProviderName::KimiCode,
        ]
        .iter()
        .filter_map(|provider| snapshot.entry(provider))
        .map(provider_dto)
        .collect(),
    }
}

fn provider_dto(entry: &ProviderHealthEntry) -> ProviderStatusDto {
    let (provider, display_name, install_hint) = match entry.provider {
        ProviderName::ClaudeCode => (
            "claude_code",
            "Claude Code",
            "Install Claude Code CLI and ensure `claude` is available on PATH.",
        ),
        ProviderName::Codex => (
            "codex",
            "Codex",
            "Install Codex CLI and ensure `codex` is available on PATH.",
        ),
        ProviderName::Pi => (
            "pi",
            "Pi",
            "Install Pi CLI and ensure `pi` is available on PATH.",
        ),
        ProviderName::KimiCode => (
            "kimi_code",
            "Kimi Code",
            "Install Kimi Code CLI and ensure `kimi` is available on PATH.",
        ),
        ProviderName::Fake => unreachable!("Fake provider is not part of real health status"),
    };
    ProviderStatusDto {
        provider,
        display_name,
        available: entry.available,
        version: entry.version.clone(),
        reason_code: entry.reason_code.map(reason_code),
        reason: entry.reason.clone(),
        checked_at: entry.checked_at,
        install_hint,
    }
}

fn reason_code(code: ProviderHealthReasonCode) -> &'static str {
    match code {
        ProviderHealthReasonCode::CommandMissing => "command_missing",
        ProviderHealthReasonCode::Timeout => "timeout",
        ProviderHealthReasonCode::NonZeroExit => "non_zero_exit",
        ProviderHealthReasonCode::VersionUnparseable => "version_unparseable",
        ProviderHealthReasonCode::VersionTooLow => "version_too_low",
        ProviderHealthReasonCode::IoError => "io_error",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use axum::extract::State;
    use chrono::{DateTime, Utc};
    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    use super::{providers_recheck, providers_status, response_from_state};
    use crate::cross_cutting::aria_state_paths::AriaStatePaths;
    use crate::cross_cutting::bounded_command_runner::{
        BoundedCommandError, BoundedCommandRequest, BoundedCommandResult, BoundedCommandRunner,
    };
    use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
    use crate::cross_cutting::provider_health::{ProviderHealthClock, ProviderHealthService};
    use crate::web::runtime::WebRuntime;
    use crate::web::state::WebAppState;

    struct FixedClock(DateTime<Utc>);

    impl ProviderHealthClock for FixedClock {
        fn now(&self) -> DateTime<Utc> {
            self.0
        }
    }

    struct ScriptedRunner {
        results: Mutex<VecDeque<Result<BoundedCommandResult, BoundedCommandError>>>,
        active: AtomicUsize,
        max_active: AtomicUsize,
        delay: Duration,
    }

    impl ScriptedRunner {
        fn new(
            results: Vec<Result<BoundedCommandResult, BoundedCommandError>>,
            delay: Duration,
        ) -> Self {
            Self {
                results: Mutex::new(results.into()),
                active: AtomicUsize::new(0),
                max_active: AtomicUsize::new(0),
                delay,
            }
        }

        fn max_active(&self) -> usize {
            self.max_active.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl BoundedCommandRunner for ScriptedRunner {
        async fn run(
            &self,
            _request: BoundedCommandRequest,
        ) -> Result<BoundedCommandResult, BoundedCommandError> {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_active.fetch_max(active, Ordering::SeqCst);
            if !self.delay.is_zero() {
                tokio::time::sleep(self.delay).await;
            }
            self.active.fetch_sub(1, Ordering::SeqCst);
            self.results
                .lock()
                .expect("scripted results")
                .pop_front()
                .expect("scripted result")
        }
    }

    fn success(version: &str) -> Result<BoundedCommandResult, BoundedCommandError> {
        Ok(BoundedCommandResult {
            exit_code: Some(0),
            stdout: format!("provider {version}"),
            stderr: String::new(),
            timed_out: false,
            cancelled: false,
            stdout_truncated: false,
            stderr_truncated: false,
            duration_ms: 1,
        })
    }

    fn missing(program: &str) -> Result<BoundedCommandResult, BoundedCommandError> {
        Err(BoundedCommandError::CommandMissing {
            executable: program.to_string(),
            details: "not found".to_string(),
        })
    }

    fn service(root: &Path, runner: Arc<dyn BoundedCommandRunner>) -> Arc<ProviderHealthService> {
        Arc::new(ProviderHealthService::with_dependencies(
            AriaStatePaths::from_workspace_root(root),
            runner,
            Arc::new(FixedClock(Utc::now())),
            Duration::from_secs(1),
            4096,
        ))
    }

    fn state(
        root: &Path,
        health: Arc<ProviderHealthService>,
        runner: Arc<dyn BoundedCommandRunner>,
    ) -> WebAppState {
        let gate = Arc::new(ProviderAvailabilityGate::new(health.clone()));
        let mut state =
            WebAppState::new(root.to_path_buf(), WebRuntime::new_fake(root.to_path_buf()))
                .with_provider_health(health, gate, runner);
        state.test_provider_enabled = false;
        state
    }

    #[tokio::test]
    async fn providers_status_includes_kimi_when_available() {
        let root = tempdir().expect("root");
        let runner = Arc::new(ScriptedRunner::new(
            vec![
                success("1.0"),
                success("2.0"),
                success("0.83.0"),
                success("0.34.0"),
            ],
            Duration::ZERO,
        ));
        let health = service(root.path(), runner.clone());
        health
            .refresh(CancellationToken::new())
            .await
            .expect("refresh");
        let response = response_from_state(&state(root.path(), health, runner));
        let kimi = response
            .providers
            .iter()
            .find(|provider| provider.provider == "kimi_code")
            .expect("Kimi provider status");

        assert_eq!(kimi.display_name, "Kimi Code");
        assert!(kimi.available);
        assert_eq!(
            kimi.install_hint,
            "Install Kimi Code CLI and ensure `kimi` is available on PATH."
        );
    }

    #[tokio::test]
    async fn providers_status_includes_pi_when_available() {
        let root = tempdir().expect("root");
        let runner = Arc::new(ScriptedRunner::new(
            vec![
                success("1.0"),
                success("2.0"),
                success("0.83.0"),
                success("0.34.0"),
            ],
            Duration::ZERO,
        ));
        let health = service(root.path(), runner.clone());
        health
            .refresh(CancellationToken::new())
            .await
            .expect("refresh");
        let response = response_from_state(&state(root.path(), health, runner));

        assert_eq!(response.providers.len(), 4);
        let pi = response
            .providers
            .iter()
            .find(|dto| dto.provider == "pi")
            .expect("status API 应返回 pi 条目");
        assert_eq!(pi.display_name, "Pi");
        assert!(pi.available);
        assert!(pi.install_hint.contains("pi"));
    }

    #[tokio::test]
    async fn providers_status_maps_all_availability_states_and_complete_fields() {
        for (results, expected_available, expected_blocked) in [
            (
                vec![
                    success("1.0"),
                    success("2.0"),
                    success("0.83.0"),
                    success("0.34.0"),
                ],
                vec![true, true, true, true],
                false,
            ),
            (
                vec![
                    missing("claude"),
                    success("2.0"),
                    success("0.83.0"),
                    success("0.34.0"),
                ],
                vec![false, true, true, true],
                false,
            ),
            (
                vec![
                    missing("claude"),
                    missing("codex"),
                    missing("pi"),
                    missing("kimi"),
                ],
                vec![false, false, false, false],
                true,
            ),
        ] {
            let root = tempdir().expect("root");
            let runner = Arc::new(ScriptedRunner::new(results, Duration::ZERO));
            let health = service(root.path(), runner.clone());
            health
                .refresh(CancellationToken::new())
                .await
                .expect("refresh");
            let response = response_from_state(&state(root.path(), health, runner));

            assert_eq!(response.schema_version, 1);
            assert_eq!(response.generation, 1);
            assert_eq!(response.state_status, "ready");
            assert_eq!(response.state_error, None);
            assert_eq!(response.real_workflow_blocked, expected_blocked);
            assert!(!response.test_provider_enabled);
            assert_eq!(response.providers.len(), 4);
            assert_eq!(response.providers[0].provider, "claude_code");
            assert_eq!(response.providers[1].provider, "codex");
            assert_eq!(response.providers[2].provider, "pi");
            assert_eq!(response.providers[3].provider, "kimi_code");
            assert_eq!(
                response
                    .providers
                    .iter()
                    .map(|provider| provider.available)
                    .collect::<Vec<_>>(),
                expected_available
            );
            for provider in response.providers {
                assert!(!provider.display_name.is_empty());
                assert!(!provider.checked_at.to_rfc3339().is_empty());
                assert!(!provider.install_hint.is_empty());
                if provider.available {
                    assert!(provider.version.is_some());
                    assert_eq!(provider.reason_code, None);
                    assert_eq!(provider.reason, None);
                } else {
                    assert_eq!(provider.version, None);
                    assert_eq!(provider.reason_code, Some("command_missing"));
                    assert_eq!(provider.reason.as_deref(), Some("not found"));
                }
            }
        }
    }

    #[tokio::test]
    async fn providers_status_reports_degraded_storage_without_http_error() {
        let root = tempdir().expect("root");
        let blocked_root = root.path().join("not-a-directory");
        std::fs::write(&blocked_root, "blocked").expect("blocked root");
        let runner = Arc::new(ScriptedRunner::new(
            vec![
                success("1.0"),
                success("2.0"),
                success("0.83.0"),
                success("0.34.0"),
            ],
            Duration::ZERO,
        ));
        let health = service(&blocked_root, runner.clone());
        let state = state(&blocked_root, health, runner);
        state.refresh_provider_health().await;

        let response = providers_status(State(state)).await.0;

        assert_eq!(response.state_status, "degraded");
        let state_error = response.state_error.expect("state error");
        assert!(state_error.contains("failed to persist provider health snapshot"));
        assert_ne!(state_error, "provider health state is degraded");
        assert!(response.real_workflow_blocked);
        assert_eq!(response.generation, 1);
        assert_eq!(response.providers.len(), 4);
    }

    #[tokio::test]
    async fn providers_status_excludes_fake_and_only_exposes_test_mode_flag() {
        let root = tempdir().expect("root");
        let runner = Arc::new(ScriptedRunner::new(Vec::new(), Duration::ZERO));
        let health = service(root.path(), runner.clone());
        let mut state = state(root.path(), health, runner);
        state.test_provider_enabled = true;

        let response = response_from_state(&state);

        assert!(response.test_provider_enabled);
        assert_eq!(response.providers.len(), 4);
        assert!(
            response
                .providers
                .iter()
                .all(|provider| provider.provider != "fake")
        );
    }

    #[tokio::test]
    async fn providers_status_recheck_serializes_refresh_and_returns_each_generation() {
        let root = tempdir().expect("root");
        let runner = Arc::new(ScriptedRunner::new(
            vec![
                missing("claude"),
                missing("codex"),
                missing("pi"),
                missing("kimi"),
                success("1.0"),
                success("2.0"),
                success("0.83.0"),
                success("0.34.0"),
            ],
            Duration::from_millis(20),
        ));
        let health = service(root.path(), runner.clone());
        let state = state(root.path(), health, runner.clone());

        let (first, second) = tokio::join!(
            providers_recheck(State(state.clone())),
            providers_recheck(State(state))
        );
        let mut responses = [first.0, second.0];
        responses.sort_by_key(|response| response.generation);

        assert_eq!(
            responses
                .iter()
                .map(|response| response.generation)
                .collect::<Vec<_>>(),
            [1, 2]
        );
        assert_eq!(runner.max_active(), 4);
        assert!(
            responses[0]
                .providers
                .iter()
                .all(|provider| !provider.available)
        );
        assert!(
            responses[1]
                .providers
                .iter()
                .all(|provider| provider.available)
        );
    }

    #[tokio::test]
    async fn providers_status_recheck_can_change_available_providers_to_unavailable() {
        let root = tempdir().expect("root");
        let runner = Arc::new(ScriptedRunner::new(
            vec![
                success("1.0"),
                success("2.0"),
                success("0.83.0"),
                success("0.34.0"),
                missing("claude"),
                missing("codex"),
                missing("pi"),
                missing("kimi"),
            ],
            Duration::ZERO,
        ));
        let health = service(root.path(), runner.clone());
        let state = state(root.path(), health, runner);

        let available = providers_recheck(State(state.clone())).await.0;
        let unavailable = providers_recheck(State(state)).await.0;

        assert_eq!(available.generation, 1);
        assert!(!available.real_workflow_blocked);
        assert!(
            available
                .providers
                .iter()
                .all(|provider| provider.available)
        );
        assert_eq!(unavailable.generation, 2);
        assert!(unavailable.real_workflow_blocked);
        assert!(
            unavailable
                .providers
                .iter()
                .all(|provider| !provider.available)
        );
    }

    #[tokio::test]
    async fn providers_status_recheck_fake_mode_does_not_execute_real_probe() {
        let root = tempdir().expect("root");
        let runner = Arc::new(ScriptedRunner::new(Vec::new(), Duration::ZERO));
        let health = service(root.path(), runner.clone());
        let mut state = state(root.path(), health, runner.clone());
        state.test_provider_enabled = true;

        let response = providers_recheck(State(state)).await.0;

        assert_eq!(runner.max_active(), 0);
        assert_eq!(response.generation, 0);
        assert!(response.test_provider_enabled);
    }
}
