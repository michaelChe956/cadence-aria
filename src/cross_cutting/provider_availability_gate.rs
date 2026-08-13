use std::sync::Arc;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::cross_cutting::provider_health::{
    ProviderHealthReasonCode, ProviderHealthService, ProviderHealthSnapshot,
};
use crate::cross_cutting::streaming_provider::{
    ProviderSession, StreamChunk, StreamingProviderAdapter, StreamingProviderInput,
};
use crate::product::models::ProviderName;
use crate::protocol::contracts::{AdapterInput, AdapterOutput};

pub trait ProviderHealthSource: Send + Sync {
    fn snapshot(&self) -> Arc<ProviderHealthSnapshot>;
    fn degraded(&self) -> bool;
}

impl ProviderHealthSource for ProviderHealthService {
    fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
        ProviderHealthService::snapshot(self)
    }

    fn degraded(&self) -> bool {
        ProviderHealthService::degraded(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("provider_unavailable: {provider:?}: {reason}")]
pub struct ProviderAvailabilityError {
    provider: ProviderName,
    reason: String,
}

impl ProviderAvailabilityError {
    pub fn unavailable(provider: ProviderName, reason: impl Into<String>) -> Self {
        Self {
            provider,
            reason: reason.into(),
        }
    }

    pub fn code(&self) -> &'static str {
        "provider_unavailable"
    }

    pub fn provider(&self) -> &ProviderName {
        &self.provider
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }

    fn into_adapter_error(self) -> ProviderAdapterError {
        ProviderAdapterError::provider_unavailable(self.to_string())
    }
}

pub trait ProviderHostReadiness: Send + Sync {
    fn ensure_ready(&self, provider: &ProviderName) -> Result<(), ProviderAvailabilityError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct AlwaysReadyProviderHost;

impl ProviderHostReadiness for AlwaysReadyProviderHost {
    fn ensure_ready(&self, _provider: &ProviderName) -> Result<(), ProviderAvailabilityError> {
        Ok(())
    }
}

pub struct ProviderAvailabilityGate {
    health: Arc<dyn ProviderHealthSource>,
    host_readiness: Arc<dyn ProviderHostReadiness>,
}

impl ProviderAvailabilityGate {
    pub fn new(health: Arc<dyn ProviderHealthSource>) -> Self {
        Self::with_host_readiness(health, Arc::new(AlwaysReadyProviderHost))
    }

    pub fn with_host_readiness(
        health: Arc<dyn ProviderHealthSource>,
        host_readiness: Arc<dyn ProviderHostReadiness>,
    ) -> Self {
        Self {
            health,
            host_readiness,
        }
    }

    pub fn ensure_available(
        &self,
        provider: &ProviderName,
    ) -> Result<(), ProviderAvailabilityError> {
        if provider == &ProviderName::Fake {
            return Ok(());
        }
        if self.health.degraded() {
            return Err(ProviderAvailabilityError::unavailable(
                provider.clone(),
                "provider health state is degraded",
            ));
        }

        let snapshot = self.health.snapshot();
        let Some(entry) = snapshot.entry(provider) else {
            return Err(ProviderAvailabilityError::unavailable(
                provider.clone(),
                "provider health entry is missing",
            ));
        };
        if !entry.available {
            let reason_code = entry.reason_code.map(reason_code_str).unwrap_or("io_error");
            let reason = entry.reason.as_deref().unwrap_or("provider unavailable");
            return Err(ProviderAvailabilityError::unavailable(
                provider.clone(),
                format!("{reason_code}: {reason}"),
            ));
        }

        self.host_readiness.ensure_ready(provider)
    }
}

fn reason_code_str(reason_code: ProviderHealthReasonCode) -> &'static str {
    match reason_code {
        ProviderHealthReasonCode::CommandMissing => "command_missing",
        ProviderHealthReasonCode::Timeout => "timeout",
        ProviderHealthReasonCode::NonZeroExit => "non_zero_exit",
        ProviderHealthReasonCode::VersionUnparseable => "version_unparseable",
        ProviderHealthReasonCode::VersionTooLow => "version_too_low",
        ProviderHealthReasonCode::IoError => "io_error",
    }
}

pub struct GatedProviderAdapter {
    provider: ProviderName,
    inner: Arc<dyn ProviderAdapter>,
    gate: Arc<ProviderAvailabilityGate>,
}

impl GatedProviderAdapter {
    pub fn new(
        provider: ProviderName,
        inner: Arc<dyn ProviderAdapter>,
        gate: Arc<ProviderAvailabilityGate>,
    ) -> Self {
        Self {
            provider,
            inner,
            gate,
        }
    }
}

impl ProviderAdapter for GatedProviderAdapter {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.gate
            .ensure_available(&self.provider)
            .map_err(ProviderAvailabilityError::into_adapter_error)?;
        self.inner.run(input)
    }
}

pub struct GatedStreamingProviderAdapter {
    provider: ProviderName,
    inner: Arc<dyn StreamingProviderAdapter>,
    gate: Arc<ProviderAvailabilityGate>,
}

impl GatedStreamingProviderAdapter {
    pub fn new(
        provider: ProviderName,
        inner: Arc<dyn StreamingProviderAdapter>,
        gate: Arc<ProviderAvailabilityGate>,
    ) -> Self {
        Self {
            provider,
            inner,
            gate,
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for GatedStreamingProviderAdapter {
    fn supports_tool_calls(&self) -> bool {
        self.inner.supports_tool_calls()
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.gate
            .ensure_available(&self.provider)
            .map_err(ProviderAvailabilityError::into_adapter_error)?;
        self.inner.start(input, cancel).await
    }

    async fn run_streaming(
        &self,
        input: &AdapterInput,
        cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        self.gate
            .ensure_available(&self.provider)
            .map_err(ProviderAvailabilityError::into_adapter_error)?;
        self.inner.run_streaming(input, cancel).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use chrono::Utc;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use super::{
        GatedProviderAdapter, GatedStreamingProviderAdapter, ProviderAvailabilityError,
        ProviderAvailabilityGate, ProviderHealthSource, ProviderHostReadiness,
    };
    use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
    use crate::cross_cutting::provider_health::{
        ProviderHealthEntry, ProviderHealthReasonCode, ProviderHealthSnapshot,
    };
    use crate::cross_cutting::streaming_provider::{
        ProviderSession, StreamingProviderAdapter, StreamingProviderInput,
    };
    use crate::product::models::ProviderName;
    use crate::protocol::contracts::{
        AdapterInput, AdapterOutput, AdapterRole, ProviderType, TimeoutStatus,
    };
    use crate::protocol::provider_errors::ProviderErrorCode;

    struct FixedHealthSource {
        snapshot: Arc<ProviderHealthSnapshot>,
        degraded: bool,
    }

    impl ProviderHealthSource for FixedHealthSource {
        fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
            self.snapshot.clone()
        }

        fn degraded(&self) -> bool {
            self.degraded
        }
    }

    fn snapshot(claude_available: bool, codex_available: bool) -> Arc<ProviderHealthSnapshot> {
        let checked_at = Utc::now();
        Arc::new(ProviderHealthSnapshot {
            schema_version: 1,
            generation: 3,
            checked_at,
            providers: vec![
                ProviderHealthEntry {
                    provider: ProviderName::ClaudeCode,
                    command: "claude --version".to_string(),
                    available: claude_available,
                    version: claude_available.then(|| "1.0".to_string()),
                    reason_code: (!claude_available)
                        .then_some(ProviderHealthReasonCode::CommandMissing),
                    reason: (!claude_available).then(|| "not found".to_string()),
                    checked_at,
                },
                ProviderHealthEntry {
                    provider: ProviderName::Codex,
                    command: "codex --version".to_string(),
                    available: codex_available,
                    version: codex_available.then(|| "1.0".to_string()),
                    reason_code: (!codex_available)
                        .then_some(ProviderHealthReasonCode::CommandMissing),
                    reason: (!codex_available).then(|| "not found".to_string()),
                    checked_at,
                },
            ],
        })
    }

    fn gate(
        claude_available: bool,
        codex_available: bool,
        degraded: bool,
    ) -> Arc<ProviderAvailabilityGate> {
        Arc::new(ProviderAvailabilityGate::new(Arc::new(FixedHealthSource {
            snapshot: snapshot(claude_available, codex_available),
            degraded,
        })))
    }

    #[test]
    fn provider_availability_gate_rejects_explicit_unavailable_provider() {
        let gate = gate(false, true, false);

        let error = gate
            .ensure_available(&ProviderName::ClaudeCode)
            .expect_err("Claude must be rejected");

        assert_eq!(error.code(), "provider_unavailable");
        assert!(error.to_string().contains("command_missing"));
        assert!(gate.ensure_available(&ProviderName::Codex).is_ok());
    }

    #[test]
    fn provider_availability_gate_fails_closed_when_health_storage_is_degraded() {
        let gate = gate(true, true, true);

        let error = gate
            .ensure_available(&ProviderName::Codex)
            .expect_err("degraded storage must fail closed");

        assert_eq!(error.code(), "provider_unavailable");
        assert!(error.to_string().contains("degraded"));
    }

    #[test]
    fn provider_availability_gate_always_allows_fake_for_tests() {
        let gate = gate(false, false, true);

        assert!(gate.ensure_available(&ProviderName::Fake).is_ok());
    }

    struct BlockedHost;

    impl ProviderHostReadiness for BlockedHost {
        fn ensure_ready(&self, provider: &ProviderName) -> Result<(), ProviderAvailabilityError> {
            Err(ProviderAvailabilityError::unavailable(
                provider.clone(),
                "host_not_ready",
            ))
        }
    }

    #[test]
    fn provider_availability_gate_composes_host_readiness_seam() {
        let source = Arc::new(FixedHealthSource {
            snapshot: snapshot(true, true),
            degraded: false,
        });
        let gate = ProviderAvailabilityGate::with_host_readiness(source, Arc::new(BlockedHost));

        let error = gate
            .ensure_available(&ProviderName::ClaudeCode)
            .expect_err("host readiness must participate");

        assert!(error.to_string().contains("host_not_ready"));
    }

    struct RecordingSyncProvider(AtomicUsize);

    impl ProviderAdapter for RecordingSyncProvider {
        fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(AdapterOutput {
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
                structured_output: None,
                files_modified: Vec::new(),
                duration_ms: 0,
                timeout_status: TimeoutStatus::NotTimedOut,
            })
        }
    }

    fn adapter_input() -> AdapterInput {
        AdapterInput {
            provider_type: ProviderType::ClaudeCode,
            role: AdapterRole::Executor,
            worktree_path: None,
            provider_stream_log_dir: None,
            prompt: "probe".to_string(),
            context_files: Vec::new(),
            output_schema: String::new(),
            timeout: 1,
            max_retries: 0,
        }
    }

    #[test]
    fn provider_availability_gate_wraps_sync_provider_adapter() {
        let inner = Arc::new(RecordingSyncProvider(AtomicUsize::new(0)));
        let allowed = GatedProviderAdapter::new(
            ProviderName::ClaudeCode,
            inner.clone(),
            gate(true, true, false),
        );
        allowed.run(&adapter_input()).expect("allowed run");
        assert_eq!(inner.0.load(Ordering::SeqCst), 1);

        let blocked = GatedProviderAdapter::new(
            ProviderName::ClaudeCode,
            inner.clone(),
            gate(false, true, false),
        );
        let error = blocked
            .run(&adapter_input())
            .expect_err("blocked run must fail");
        assert_eq!(error.code, ProviderErrorCode::ProviderUnavailable);
        assert_eq!(error.code.as_str(), "provider_unavailable");
        assert_eq!(inner.0.load(Ordering::SeqCst), 1);
    }

    struct RecordingStreamingProvider(AtomicUsize);

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for RecordingStreamingProvider {
        fn supports_tool_calls(&self) -> bool {
            true
        }

        async fn start(
            &self,
            _input: StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            let (_event_tx, events) = mpsc::channel(1);
            let (commands, _command_rx) = mpsc::channel(1);
            Ok(ProviderSession { events, commands })
        }
    }

    fn streaming_input() -> StreamingProviderInput {
        StreamingProviderInput {
            provider_type: ProviderType::Codex,
            role: AdapterRole::Executor,
            prompt: "probe".to_string(),
            working_dir: std::env::current_dir().expect("cwd"),
            workspace_session_id: None,
            resume_provider_session_id: None,
            permission_mode: crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
            structured_output_contract: None,
            env_vars: Default::default(),
            timeout_secs: 1,
        }
    }

    #[tokio::test]
    async fn provider_availability_gate_wraps_streaming_provider_adapter() {
        let inner = Arc::new(RecordingStreamingProvider(AtomicUsize::new(0)));
        let allowed = GatedStreamingProviderAdapter::new(
            ProviderName::Codex,
            inner.clone(),
            gate(true, true, false),
        );
        assert!(allowed.supports_tool_calls());
        allowed
            .start(streaming_input(), CancellationToken::new())
            .await
            .expect("allowed start");
        assert_eq!(inner.0.load(Ordering::SeqCst), 1);

        let blocked = GatedStreamingProviderAdapter::new(
            ProviderName::Codex,
            inner.clone(),
            gate(true, false, false),
        );
        let error = blocked
            .start(streaming_input(), CancellationToken::new())
            .await
            .err()
            .expect("blocked start must fail");
        assert_eq!(error.code, ProviderErrorCode::ProviderUnavailable);
        assert_eq!(error.code.as_str(), "provider_unavailable");
        assert_eq!(inner.0.load(Ordering::SeqCst), 1);
    }
}
