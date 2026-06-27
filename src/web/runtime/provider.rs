use std::fs;
use std::sync::Arc;

use serde_json::json;

use crate::cross_cutting::cli_adapter::{CliOutputChunk, ProviderOutputSink};
use crate::cross_cutting::provider_adapter::{
    FakeProviderAdapter, ProviderAdapter, ProviderAdapterError,
};
use crate::protocol::contracts::{AdapterInput, ProviderType};
use crate::task_run::orchestrator::TaskRunOrchestrator;
use crate::task_run::provider_factory::{
    real_routing_provider, real_routing_provider_with_output_sink,
};
use crate::task_run::types::{ProviderMode, TaskRunError, TaskRunRequest};
use crate::web::events::EventHub;
use crate::web::provider_availability::{
    host_real_workflow_ready, provider_type_available, provider_type_key,
    resolve_default_runtime_provider_type, resolve_runtime_provider_type,
};
use crate::web::redaction::redact_sensitive_lines;
use crate::web::runtime_store::WebRuntimeStore;
use crate::web::types::{ConfirmTaskResponse, ProviderInputPrepared, ProviderOutputChunk};

use super::WebRuntime;
use super::metadata::preserve_web_task_metadata;
use super::utils::{
    io_error, parse_confirm_provider_type, provider_input_ref_for_node,
    provider_node_id_for_schema, provider_run_id_for_chunk, read_optional_json, task_status_text,
};

struct ProviderOverrideAdapter<'a> {
    inner: &'a dyn ProviderAdapter,
    provider_type: ProviderType,
}

impl ProviderAdapter for ProviderOverrideAdapter<'_> {
    fn run(
        &self,
        input: &AdapterInput,
    ) -> Result<crate::protocol::contracts::AdapterOutput, ProviderAdapterError> {
        let mut input = input.clone();
        input.provider_type = self.provider_type.clone();
        self.inner.run(&input)
    }
}

impl WebRuntime {
    pub fn new_fake(workspace_root: std::path::PathBuf) -> Self {
        Self {
            workspace_root,
            next_projection_version: 1,
            real_provider: None,
            provider_availability: Arc::new(|_| true),
            enforce_real_provider_availability: false,
        }
    }

    pub fn new_fake_with_provider_availability<F>(
        workspace_root: std::path::PathBuf,
        provider_availability: F,
    ) -> Self
    where
        F: Fn(&ProviderType) -> bool + Send + Sync + 'static,
    {
        Self {
            workspace_root,
            next_projection_version: 1,
            real_provider: None,
            provider_availability: Arc::new(provider_availability),
            enforce_real_provider_availability: false,
        }
    }

    pub fn new_real(workspace_root: std::path::PathBuf) -> Result<Self, TaskRunError> {
        Ok(Self {
            workspace_root,
            next_projection_version: 1,
            real_provider: Some(Arc::new(real_routing_provider()?)),
            provider_availability: Arc::new(provider_type_available),
            enforce_real_provider_availability: true,
        })
    }

    pub fn new_real_with_events(
        workspace_root: std::path::PathBuf,
        events: EventHub,
    ) -> Result<Self, TaskRunError> {
        let output_sink: ProviderOutputSink = Arc::new(move |chunk: CliOutputChunk| {
            events.publish_provider_output(
                None,
                ProviderOutputChunk {
                    node_id: provider_node_id_for_schema(&chunk.output_schema).to_string(),
                    provider_run_id: provider_run_id_for_chunk(&chunk),
                    stream: chunk.stream,
                    text: chunk.text,
                    structured_output: None,
                    manual_gate: None,
                    retry_attempt: None,
                },
            );
        });
        Ok(Self {
            workspace_root,
            next_projection_version: 1,
            real_provider: Some(Arc::new(real_routing_provider_with_output_sink(Some(
                output_sink,
            ))?)),
            provider_availability: Arc::new(provider_type_available),
            enforce_real_provider_availability: true,
        })
    }

    pub fn new_with_provider(
        workspace_root: std::path::PathBuf,
        real_provider: Box<dyn ProviderAdapter + Send + Sync>,
    ) -> Self {
        Self {
            workspace_root,
            next_projection_version: 1,
            real_provider: Some(Arc::from(real_provider)),
            provider_availability: Arc::new(|_| true),
            enforce_real_provider_availability: false,
        }
    }

    pub fn enforces_real_provider_availability(&self) -> bool {
        self.enforce_real_provider_availability
    }

    pub fn provider_adapter(&self) -> Arc<dyn ProviderAdapter + Send + Sync> {
        if self.enforce_real_provider_availability {
            self.real_provider
                .clone()
                .unwrap_or_else(|| Arc::new(real_routing_provider().expect("real provider")))
        } else {
            Arc::new(FakeProviderAdapter)
        }
    }

    pub(super) fn resolve_real_confirm_provider(
        &self,
        provider_type: Option<&str>,
    ) -> Result<Option<ProviderType>, TaskRunError> {
        if !self.enforce_real_provider_availability {
            return provider_type.map(parse_confirm_provider_type).transpose();
        }
        host_real_workflow_ready()?;
        let is_available = |provider: &ProviderType| (self.provider_availability)(provider);
        match provider_type {
            Some(provider_type) => Ok(Some(
                resolve_runtime_provider_type(provider_type, is_available)?.provider,
            )),
            None => Ok(Some(
                resolve_default_runtime_provider_type(is_available)?.provider,
            )),
        }
    }

    pub(super) fn resolve_fake_confirm_provider(
        &self,
        provider_type: Option<&str>,
    ) -> Result<String, TaskRunError> {
        if let Some(provider_type) = provider_type {
            return parse_confirm_provider_type(provider_type)
                .map(|provider| provider_type_key(&provider).to_string());
        }
        let selected = resolve_default_runtime_provider_type(|provider| {
            (self.provider_availability)(provider)
        })?;
        Ok(provider_type_key(&selected.provider).to_string())
    }

    pub fn prepare_provider_input(
        &self,
        task_id: &str,
        prompt: &str,
    ) -> Result<ProviderInputPrepared, TaskRunError> {
        let store = WebRuntimeStore::new(&self.workspace_root, task_id);
        let state = read_optional_json(&store.task_root().join("state.json"))?;
        let pending = read_optional_json(&store.task_root().join("pending/provider-step.json"))?;
        let node_id = pending
            .get("node_id")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                state
                    .get("current_node")
                    .and_then(serde_json::Value::as_str)
            })
            .unwrap_or("N16");
        let effective_prompt = if prompt.trim().is_empty() {
            state
                .get("request_text")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
        } else {
            prompt
        };
        let input_ref = provider_input_ref_for_node(node_id);
        let input_value = json!({
            "node_id": node_id,
            "prompt": effective_prompt,
            "input_summary": {
                "node_id": node_id,
                "prompt_chars": effective_prompt.chars().count()
            }
        });
        store.write_json(&format!("provider-inputs/{input_ref}.json"), &input_value)?;
        let serialized = serde_json::to_string_pretty(&input_value)
            .map_err(|error| TaskRunError::new("web_runtime_json", error.to_string()))?;
        Ok(ProviderInputPrepared {
            node_id: node_id.to_string(),
            input_ref,
            input_summary: input_value["input_summary"].clone(),
            redaction_applied: redact_sensitive_lines(&serialized) != serialized,
        })
    }

    pub fn provider_command_diagnostic(
        &self,
        provider_type: &str,
        message: &str,
    ) -> serde_json::Value {
        json!({
            "category": "provider_error",
            "code": "provider_authorization_or_command_unavailable",
            "provider_type": provider_type,
            "message": format!("{provider_type} provider unavailable: {message}"),
            "details": {
                "action": "check provider CLI installation, authentication, and PATH"
            }
        })
    }

    pub(super) fn persist_real_provider_execution(
        &mut self,
        store: &WebRuntimeStore,
        state: &serde_json::Value,
        prompt: String,
        selected_provider_type: Option<ProviderType>,
    ) -> Result<ConfirmTaskResponse, TaskRunError> {
        let provider = self.real_provider.as_ref().ok_or_else(|| {
            TaskRunError::new(
                "web_real_provider_unavailable",
                "real provider is not configured for this web runtime",
            )
        })?;
        let change_id = state
            .get("change_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("aria-web-task")
            .to_string();
        let request_text = if prompt.trim().is_empty() {
            state
                .get("request_text")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string()
        } else {
            prompt
        };
        let timeout_secs = state
            .get("timeout_secs")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(crate::cross_cutting::provider_adapter::DEFAULT_PROVIDER_TIMEOUT_SECS);
        let task_id = state
            .get("task_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("task_0001")
            .to_string();
        let pending_path = store.task_root().join("pending/provider-step.json");
        if pending_path.exists() {
            fs::remove_file(pending_path).map_err(io_error)?;
        }
        let override_provider =
            selected_provider_type.map(|provider_type| ProviderOverrideAdapter {
                inner: provider.as_ref(),
                provider_type,
            });
        let provider_ref: &dyn ProviderAdapter = override_provider
            .as_ref()
            .map(|provider| provider as &dyn ProviderAdapter)
            .unwrap_or(provider.as_ref());
        let outcome = TaskRunOrchestrator::run_with_provider(
            TaskRunRequest {
                task_id: Some(task_id.clone()),
                workspace: self.workspace_root.clone(),
                request_text,
                change_id: change_id.clone(),
                provider_mode: ProviderMode::Real,
                non_interactive: true,
                timeout_secs,
            },
            provider_ref,
        );
        preserve_web_task_metadata(store, state, &task_id, &change_id, timeout_secs)?;
        let outcome = outcome?;
        self.next_projection_version += 1;
        Ok(ConfirmTaskResponse {
            status: task_status_text(&outcome.status).to_string(),
            node_id: "N16".to_string(),
            turn_id: format!("turn_{task_id}_real"),
        })
    }

    pub(super) fn persist_provider_execution(
        &mut self,
        _task_id: &str,
        store: &WebRuntimeStore,
        checkpoint_id: String,
        prompt: String,
        provider_type: &str,
    ) -> Result<ConfirmTaskResponse, TaskRunError> {
        store.append_event(
            "node_started",
            "N16",
            json!({"status":"running","provider_run_id":"run_n16_0001"}),
        )?;
        store.append_event(
            "provider_output",
            "N16",
            json!({"status":"running","provider_run_id":"run_n16_0001","stream":"stdout","text":"done"}),
        )?;
        store.append_jsonl(
            "logs/provider-output.jsonl",
            json!({
                "kind": "provider_output",
                "node_id": "N16",
                "provider_run_id": "run_n16_0001",
                "stream": "stdout",
                "text": "done"
            }),
        )?;
        store.write_json(
            "turns/turn_0001.json",
            &json!({
                "turn_id": "turn_0001",
                "session_id": "sess_task_0001",
                "node_id": "N16",
                "provider_type": provider_type,
                "prompt_snapshot": prompt.clone(),
                "input_summary": {"worktask_id":"work_wt_001"},
                "checkpoint_before": checkpoint_id.clone(),
                "provider_run_id": "run_n16_0001",
                "output_artifact_refs": ["coding_report_work_wt_001_0001"],
                "changed_files": ["src/fibonacciSquareSum.js"],
                "status": "completed",
                "dropped": false,
                "created_at": "2026-05-09T00:00:00Z",
                "updated_at": "2026-05-09T00:00:00Z"
            }),
        )?;
        store.write_json(
            "node-runs/nrun_0001.json",
            &json!({
                "node_run_id": "nrun_0001",
                "node_id": "N16",
                "turn_id": "turn_0001",
                "provider_run_id": "run_n16_0001",
                "input_refs": ["worktask:work_wt_001"],
                "output_schema": "schema://aria/artifacts/coding_report/v1",
                "artifact_refs": ["coding_report_work_wt_001_0001"],
                "status": "completed",
                "duration_ms": 42,
                "diagnostic_refs": [],
                "dropped": false,
                "created_at": "2026-05-09T00:00:00Z",
                "updated_at": "2026-05-09T00:00:00Z"
            }),
        )?;
        store.write_json(
            "provider-runs/run_n16_0001/run.json",
            &json!({
                "provider_run_id": "run_n16_0001",
                "provider_type": provider_type,
                "status": "completed",
                "prompt": prompt,
                "dropped": false
            }),
        )?;
        store.write_json(
            "artifacts/execution/0000.json",
            &json!({
                "artifact_ref": "coding_report_work_wt_001_0001",
                "artifact_kind": "coding_report",
                "producer_node": "N16",
                "changed_files": ["src/fibonacciSquareSum.js"],
                "dropped": false
            }),
        )?;
        store.write_json(
            "reports/provider-run-run_n16_0001.json",
            &json!({
                "report_id": "provider-run-run_n16_0001",
                "provider_run_id": "run_n16_0001",
                "provider_type": provider_type,
                "status": "completed",
                "dropped": false
            }),
        )?;
        store.append_event(
            "artifact_written",
            "N16",
            json!({"status":"completed","artifact_ref":"coding_report_work_wt_001_0001"}),
        )?;
        store.append_event(
            "node_completed",
            "N16",
            json!({
                "status":"completed",
                "checkpoint_id":checkpoint_id,
                "provider_run_id":"run_n16_0001",
                "duration_ms":42,
                "changed_files":["src/fibonacciSquareSum.js"]
            }),
        )?;
        let task_root = store.task_root();
        let pending_path = task_root.join("pending/provider-step.json");
        if pending_path.exists() {
            fs::remove_file(pending_path).map_err(io_error)?;
        }
        self.next_projection_version += 1;
        Ok(ConfirmTaskResponse {
            status: "provider_started".to_string(),
            node_id: "N16".to_string(),
            turn_id: "turn_0001".to_string(),
        })
    }
}
