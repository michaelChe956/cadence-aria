use cadence_aria::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use cadence_aria::protocol::contracts::{
    AdapterInput, AdapterOutput, AdapterRole, ProviderType, TimeoutStatus,
};
use cadence_aria::task_run::provider_factory::RoutingProviderAdapter;
use serde_json::json;
use std::sync::{Arc, Mutex};

#[test]
fn routes_provider_calls_by_adapter_input_provider_type() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let provider = RoutingProviderAdapter::new(
        Box::new(RecordingProvider::new(
            ProviderType::ClaudeCode,
            seen.clone(),
        )),
        Box::new(RecordingProvider::new(ProviderType::Codex, seen.clone())),
    );

    provider
        .run(&adapter_input(ProviderType::ClaudeCode))
        .expect("claude run");
    provider
        .run(&adapter_input(ProviderType::Codex))
        .expect("codex run");

    assert_eq!(
        *seen.lock().expect("seen"),
        vec![ProviderType::ClaudeCode, ProviderType::Codex]
    );
}

#[derive(Debug)]
struct RecordingProvider {
    provider_type: ProviderType,
    seen: Arc<Mutex<Vec<ProviderType>>>,
}

impl RecordingProvider {
    fn new(provider_type: ProviderType, seen: Arc<Mutex<Vec<ProviderType>>>) -> Self {
        Self {
            provider_type,
            seen,
        }
    }
}

impl ProviderAdapter for RecordingProvider {
    fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.seen
            .lock()
            .expect("seen")
            .push(self.provider_type.clone());
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            structured_output: Some(json!({"artifact_kind": "clarification_record"})),
            files_modified: Vec::new(),
            duration_ms: 1,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}

fn adapter_input(provider_type: ProviderType) -> AdapterInput {
    AdapterInput {
        provider_type,
        role: AdapterRole::Orchestrator,
        worktree_path: None,
        prompt: "fixture prompt".to_string(),
        context_files: Vec::new(),
        output_schema: "schema://aria/artifacts/clarification_record/v1".to_string(),
        timeout: 3,
        max_retries: 1,
    }
}
