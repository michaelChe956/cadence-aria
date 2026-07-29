use crate::cross_cutting::adapter_compatibility::default_compatibility_matrix;
use crate::cross_cutting::cli_adapter::{CliAdapterConfig, CliProviderAdapter, ProviderOutputSink};
use std::sync::Arc;

use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::cross_cutting::provider_availability_gate::{
    ProviderAvailabilityError, ProviderAvailabilityGate,
};
use crate::product::models::ProviderName;
use crate::protocol::contracts::{AdapterInput, AdapterOutput, ProviderType};
use crate::task_run::types::TaskRunError;

pub struct RoutingProviderAdapter {
    claude: Box<dyn ProviderAdapter + Send + Sync>,
    codex: Box<dyn ProviderAdapter + Send + Sync>,
}

struct GatedRoutingProviderAdapter {
    provider: ProviderName,
    inner: Box<dyn ProviderAdapter + Send + Sync>,
    gate: Arc<ProviderAvailabilityGate>,
}

impl ProviderAdapter for GatedRoutingProviderAdapter {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.gate.ensure_available(&self.provider).map_err(
            |error: ProviderAvailabilityError| {
                ProviderAdapterError::provider_unavailable(error.to_string())
            },
        )?;
        self.inner.run(input)
    }
}

impl RoutingProviderAdapter {
    pub fn new(
        claude: Box<dyn ProviderAdapter + Send + Sync>,
        codex: Box<dyn ProviderAdapter + Send + Sync>,
    ) -> Self {
        Self { claude, codex }
    }

    pub fn new_with_gate(
        claude: Box<dyn ProviderAdapter + Send + Sync>,
        codex: Box<dyn ProviderAdapter + Send + Sync>,
        gate: Arc<ProviderAvailabilityGate>,
    ) -> Self {
        Self::new(
            Box::new(GatedRoutingProviderAdapter {
                provider: ProviderName::ClaudeCode,
                inner: claude,
                gate: gate.clone(),
            }),
            Box::new(GatedRoutingProviderAdapter {
                provider: ProviderName::Codex,
                inner: codex,
                gate,
            }),
        )
    }
}

impl ProviderAdapter for RoutingProviderAdapter {
    fn run(&self, input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        match &input.provider_type {
            ProviderType::ClaudeCode => self.claude.run(input),
            ProviderType::Codex => self.codex.run(input),
            ProviderType::Fake => Err(ProviderAdapterError::incompatible_output(
                "task run routing provider does not execute fake provider inputs",
                String::new(),
                String::new(),
            )),
        }
    }
}

pub fn real_routing_provider(
    gate: Arc<ProviderAvailabilityGate>,
) -> Result<RoutingProviderAdapter, TaskRunError> {
    real_routing_provider_with_output_sink(gate, None)
}

pub fn real_routing_provider_with_host_readiness<F>(
    gate: Arc<ProviderAvailabilityGate>,
    host_readiness: F,
) -> Result<RoutingProviderAdapter, TaskRunError>
where
    F: FnOnce() -> Result<(), TaskRunError>,
{
    host_readiness()?;
    real_routing_provider(gate)
}

pub fn real_routing_provider_with_output_sink(
    gate: Arc<ProviderAvailabilityGate>,
    output_sink: Option<ProviderOutputSink>,
) -> Result<RoutingProviderAdapter, TaskRunError> {
    let matrix = default_compatibility_matrix();
    let claude = matrix
        .entry_for(ProviderType::ClaudeCode)
        .cloned()
        .ok_or_else(|| TaskRunError::new("provider_matrix_missing", "missing claude entry"))?;
    let codex = matrix
        .entry_for(ProviderType::Codex)
        .cloned()
        .ok_or_else(|| TaskRunError::new("provider_matrix_missing", "missing codex entry"))?;

    Ok(RoutingProviderAdapter::new_with_gate(
        Box::new(CliProviderAdapter::new(CliAdapterConfig {
            compatibility: claude,
            expected_artifact_kind: None,
            output_sink: output_sink.clone(),
        })),
        Box::new(CliProviderAdapter::new(CliAdapterConfig {
            compatibility: codex,
            expected_artifact_kind: None,
            output_sink,
        })),
        gate,
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, RwLock};

    use chrono::Utc;

    use super::*;
    use crate::cross_cutting::provider_availability_gate::{
        ProviderAvailabilityGate, ProviderHealthSource,
    };
    use crate::cross_cutting::provider_health::{
        ProviderHealthEntry, ProviderHealthReasonCode, ProviderHealthSnapshot,
    };
    use crate::product::models::ProviderName;
    use crate::protocol::provider_errors::ProviderErrorCode;

    struct MutableHealth {
        snapshot: RwLock<Arc<ProviderHealthSnapshot>>,
        degraded: AtomicBool,
    }

    impl ProviderHealthSource for MutableHealth {
        fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
            self.snapshot.read().expect("snapshot").clone()
        }

        fn degraded(&self) -> bool {
            self.degraded.load(Ordering::SeqCst)
        }
    }

    fn snapshot(claude_available: bool, codex_available: bool) -> Arc<ProviderHealthSnapshot> {
        let checked_at = Utc::now();
        Arc::new(ProviderHealthSnapshot {
            schema_version: 1,
            generation: 1,
            checked_at,
            providers: vec![
                entry(ProviderName::ClaudeCode, claude_available, checked_at),
                entry(ProviderName::Codex, codex_available, checked_at),
            ],
        })
    }

    fn entry(
        provider: ProviderName,
        available: bool,
        checked_at: chrono::DateTime<Utc>,
    ) -> ProviderHealthEntry {
        ProviderHealthEntry {
            provider,
            command: "provider --version".to_string(),
            available,
            version: available.then(|| "1.0".to_string()),
            reason_code: (!available).then_some(ProviderHealthReasonCode::CommandMissing),
            reason: (!available).then(|| "not found".to_string()),
            checked_at,
        }
    }

    fn gate(health: Arc<MutableHealth>) -> Arc<ProviderAvailabilityGate> {
        Arc::new(ProviderAvailabilityGate::new(health))
    }

    struct RecordingProvider(Arc<AtomicUsize>);

    impl ProviderAdapter for RecordingProvider {
        fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(AdapterOutput {
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
                structured_output: None,
                files_modified: Vec::new(),
                duration_ms: 0,
                timeout_status: crate::protocol::contracts::TimeoutStatus::NotTimedOut,
            })
        }
    }

    fn input(provider_type: ProviderType) -> AdapterInput {
        AdapterInput {
            provider_type,
            role: crate::protocol::contracts::AdapterRole::Executor,
            worktree_path: None,
            provider_stream_log_dir: None,
            prompt: "test".to_string(),
            context_files: Vec::new(),
            output_schema: String::new(),
            timeout: 1,
            max_retries: 0,
        }
    }

    #[test]
    fn routing_provider_rejects_unavailable_provider_before_dispatch() {
        let health = Arc::new(MutableHealth {
            snapshot: RwLock::new(snapshot(false, true)),
            degraded: AtomicBool::new(false),
        });
        let claude_calls = Arc::new(AtomicUsize::new(0));
        let provider = RoutingProviderAdapter::new_with_gate(
            Box::new(RecordingProvider(claude_calls.clone())),
            Box::new(RecordingProvider(Arc::new(AtomicUsize::new(0)))),
            gate(health),
        );

        let error = provider
            .run(&input(ProviderType::ClaudeCode))
            .expect_err("unavailable Claude must be rejected");

        assert_eq!(error.code, ProviderErrorCode::ProviderUnavailable);
        assert_eq!(claude_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn routing_provider_fails_closed_when_health_is_degraded() {
        let health = Arc::new(MutableHealth {
            snapshot: RwLock::new(snapshot(true, true)),
            degraded: AtomicBool::new(true),
        });
        let codex_calls = Arc::new(AtomicUsize::new(0));
        let provider = RoutingProviderAdapter::new_with_gate(
            Box::new(RecordingProvider(Arc::new(AtomicUsize::new(0)))),
            Box::new(RecordingProvider(codex_calls.clone())),
            gate(health),
        );

        let error = provider
            .run(&input(ProviderType::Codex))
            .expect_err("degraded health must reject real provider");

        assert_eq!(error.code, ProviderErrorCode::ProviderUnavailable);
        assert_eq!(codex_calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn routing_provider_rechecks_health_before_each_dispatch() {
        let health = Arc::new(MutableHealth {
            snapshot: RwLock::new(snapshot(true, true)),
            degraded: AtomicBool::new(false),
        });
        let claude_calls = Arc::new(AtomicUsize::new(0));
        let provider = RoutingProviderAdapter::new_with_gate(
            Box::new(RecordingProvider(claude_calls.clone())),
            Box::new(RecordingProvider(Arc::new(AtomicUsize::new(0)))),
            gate(health.clone()),
        );

        provider
            .run(&input(ProviderType::ClaudeCode))
            .expect("initial healthy dispatch");
        *health.snapshot.write().expect("snapshot") = snapshot(false, true);

        let error = provider
            .run(&input(ProviderType::ClaudeCode))
            .expect_err("health changes must block the next dispatch");

        assert_eq!(error.code, ProviderErrorCode::ProviderUnavailable);
        assert_eq!(claude_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn routing_provider_rejects_fake_inputs() {
        let health = Arc::new(MutableHealth {
            snapshot: RwLock::new(snapshot(true, true)),
            degraded: AtomicBool::new(false),
        });
        let provider = RoutingProviderAdapter::new_with_gate(
            Box::new(RecordingProvider(Arc::new(AtomicUsize::new(0)))),
            Box::new(RecordingProvider(Arc::new(AtomicUsize::new(0)))),
            gate(health),
        );

        let error = provider
            .run(&input(ProviderType::Fake))
            .expect_err("task run must reject Fake provider");

        assert_eq!(error.code, ProviderErrorCode::ProviderIncompatibleOutput);
    }

    #[test]
    fn routing_provider_rejects_unready_host_before_real_construction() {
        let health = Arc::new(MutableHealth {
            snapshot: RwLock::new(snapshot(true, true)),
            degraded: AtomicBool::new(false),
        });
        let error = real_routing_provider_with_host_readiness(gate(health), || {
            Err(TaskRunError::new(
                "host_real_workflow_blocked",
                "host is not ready",
            ))
        })
        .err()
        .expect("host readiness must block real provider construction");

        assert_eq!(error.code, "host_real_workflow_blocked");
    }
}
