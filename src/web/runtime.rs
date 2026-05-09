use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::str::FromStr;

use serde_json::json;

use crate::interactive::models::{RuntimeCheckpoint, WebWorkspaceProjection};
use crate::interactive::policy::{
    ConfirmationDecision, NodeWriteClass, PolicyPreset, ProviderNodeMeta,
};
use crate::interactive::projection::build_workspace_projection;
use crate::interactive::web_projection::build_web_projection;
use crate::task_run::types::TaskRunError;
use crate::web::runtime_store::WebRuntimeStore;
use crate::web::types::{
    AdvanceTaskResponse, ArtifactContentResponse, ConfirmTaskRequest, ConfirmTaskResponse,
    CreateTaskRequest, CreateTaskResponse, FileContentResponse, FileDiffResponse,
    PendingProviderStepDto, StopTaskResponse, TaskListItem, TaskListResponse,
};

pub struct WebRuntime {
    workspace_root: PathBuf,
    next_projection_version: u64,
}

impl WebRuntime {
    pub fn new_fake(workspace_root: PathBuf) -> Self {
        Self {
            workspace_root,
            next_projection_version: 1,
        }
    }

    pub fn create_task(
        &mut self,
        request: CreateTaskRequest,
    ) -> Result<CreateTaskResponse, TaskRunError> {
        let task_id = "task_0001".to_string();
        let session_id = "sess_task_0001".to_string();
        let change_id = request.change_id.clone();
        let task_root = self
            .workspace_root
            .join(".aria/runtime/tasks")
            .join(&task_id);
        fs::create_dir_all(task_root.join("pending")).map_err(io_error)?;
        fs::create_dir_all(task_root.join("logs")).map_err(io_error)?;
        fs::write(
            task_root.join("state.json"),
            serde_json::to_vec_pretty(&json!({
                "task_id": task_id,
                "phase": "intake",
                "change_id": change_id,
                "current_node": "N16",
                "policy_preset": request.policy_preset,
                "provider_mode": request.provider_mode
            }))
            .map_err(json_error)?,
        )
        .map_err(io_error)?;
        Ok(CreateTaskResponse {
            task_id,
            session_id,
            change_id,
            phase: "intake".to_string(),
        })
    }

    pub fn advance_task(&mut self, task_id: &str) -> Result<AdvanceTaskResponse, TaskRunError> {
        let store = WebRuntimeStore::new(&self.workspace_root, task_id);
        let policy = policy_for_task(&store)?;
        run_internal_n00_if_needed(&store)?;
        write_checkpoint(
            &self.workspace_root,
            task_id,
            &store,
            self.next_projection_version,
        )?;
        let pending = pending_provider_step_for_policy(policy);
        let decision = policy.decision_for(&ProviderNodeMeta::new(
            pending.node_id.clone(),
            pending.provider_type.clone(),
            write_class_for_pending(&pending),
        ));
        match decision {
            ConfirmationDecision::PauseForConfirmation => {
                store.write_json("pending/provider-step.json", &pending)?;
                Ok(AdvanceTaskResponse::PausedForApproval {
                    pending_step: pending,
                })
            }
            ConfirmationDecision::RunAutomatically => {
                self.persist_provider_execution(
                    task_id,
                    &store,
                    pending.checkpoint_id,
                    pending.prompt,
                )?;
                Ok(AdvanceTaskResponse::Advanced {
                    projection_version: self.next_projection_version,
                })
            }
        }
    }

    pub fn confirm_task(
        &mut self,
        task_id: &str,
        request: ConfirmTaskRequest,
    ) -> Result<ConfirmTaskResponse, TaskRunError> {
        if let Some(policy_override) = request.policy_override.as_deref() {
            PolicyPreset::from_str(policy_override)
                .map_err(|error| TaskRunError::new("web_runtime_policy", error))?;
        }
        let store = WebRuntimeStore::new(&self.workspace_root, task_id);
        self.persist_provider_execution(task_id, &store, request.checkpoint_id, request.prompt)
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

    pub fn stop_task(&mut self, task_id: &str) -> Result<StopTaskResponse, TaskRunError> {
        Ok(StopTaskResponse {
            status: "stop_requested".to_string(),
            task_id: task_id.to_string(),
        })
    }

    fn persist_provider_execution(
        &mut self,
        _task_id: &str,
        store: &WebRuntimeStore,
        checkpoint_id: String,
        prompt: String,
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
                "provider_type": "codex",
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
                "provider_type": "codex",
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
                "provider_type": "codex",
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

    pub fn projection(
        &self,
        task_id: Option<&str>,
        selected_node_id: Option<&str>,
    ) -> Result<WebWorkspaceProjection, TaskRunError> {
        let base = build_workspace_projection(&self.workspace_root, task_id)?;
        build_web_projection(&self.workspace_root, base, selected_node_id)
    }

    pub fn list_tasks(&self) -> Result<TaskListResponse, TaskRunError> {
        let tasks_root = self.workspace_root.join(".aria/runtime/tasks");
        let entries = match fs::read_dir(&tasks_root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(TaskListResponse { tasks: Vec::new() });
            }
            Err(error) => return Err(io_error(error)),
        };
        let mut tasks = Vec::new();
        for entry in entries {
            let entry = entry.map_err(io_error)?;
            if !entry.file_type().map_err(io_error)?.is_dir() {
                continue;
            }
            let task_id = entry.file_name().to_string_lossy().to_string();
            let state = read_optional_json(&entry.path().join("state.json"))?;
            tasks.push(TaskListItem {
                task_id,
                change_id: state
                    .get("change_id")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                phase: state
                    .get("phase")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                updated_at: None,
            });
        }
        tasks.sort_by(|left, right| left.task_id.cmp(&right.task_id));
        Ok(TaskListResponse { tasks })
    }

    pub fn artifact_content(
        &self,
        artifact_ref: &str,
    ) -> Result<ArtifactContentResponse, TaskRunError> {
        let projection = self.projection(None, None)?;
        let entry = projection
            .artifact_index
            .iter()
            .find(|entry| entry.artifact_ref == artifact_ref)
            .ok_or_else(|| {
                TaskRunError::new(
                    "artifact_not_found",
                    format!("artifact not found: {artifact_ref}"),
                )
            })?;
        let path = self.workspace_root.join(&entry.path);
        let content = fs::read_to_string(path).map_err(io_error)?;
        Ok(ArtifactContentResponse {
            artifact_ref: entry.artifact_ref.clone(),
            artifact_kind: entry.artifact_kind.clone(),
            producer_node: entry.producer_node.clone(),
            path: entry.path.clone(),
            content_type: format!("{:?}", entry.content_type).to_lowercase(),
            content,
        })
    }

    pub fn file_content(&self, path: &str) -> Result<FileContentResponse, TaskRunError> {
        let safe = safe_workspace_path(&self.workspace_root, path)?;
        Ok(FileContentResponse {
            path: path.to_string(),
            content_type: content_type_for_path(path),
            content: fs::read_to_string(safe).map_err(io_error)?,
        })
    }

    pub fn file_diff(
        &self,
        base_checkpoint: &str,
        path: &str,
    ) -> Result<FileDiffResponse, TaskRunError> {
        let _ = safe_workspace_path(&self.workspace_root, path)?;
        let diff = Command::new("git")
            .args(["diff", base_checkpoint, "--", path])
            .current_dir(&self.workspace_root)
            .output()
            .map_err(|error| TaskRunError::new("git_command_failed", error.to_string()))?;
        Ok(FileDiffResponse {
            base_checkpoint: base_checkpoint.to_string(),
            path: path.to_string(),
            diff: String::from_utf8_lossy(&diff.stdout).to_string(),
        })
    }
}

fn io_error(error: std::io::Error) -> TaskRunError {
    TaskRunError::new("web_runtime_io", error.to_string())
}

fn json_error(error: serde_json::Error) -> TaskRunError {
    TaskRunError::new("web_runtime_json", error.to_string())
}

fn read_optional_json(path: &std::path::Path) -> Result<serde_json::Value, TaskRunError> {
    match fs::File::open(path) {
        Ok(file) => serde_json::from_reader(file)
            .map_err(|error| TaskRunError::new("web_runtime_json", error.to_string())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(json!({})),
        Err(error) => Err(io_error(error)),
    }
}

fn safe_workspace_path(
    root: &std::path::Path,
    path: &str,
) -> Result<std::path::PathBuf, TaskRunError> {
    if path.contains("..") || path.starts_with('/') || path.starts_with('\\') {
        return Err(TaskRunError::new(
            "invalid_file_path",
            format!("unsafe path: {path}"),
        ));
    }
    Ok(root.join(path))
}

fn content_type_for_path(path: &str) -> String {
    if path.ends_with(".md") {
        "markdown".to_string()
    } else if path.ends_with(".json") {
        "json".to_string()
    } else if path.contains("/tests/") || path.contains(".test.") || path.contains(".spec.") {
        "test".to_string()
    } else if path.ends_with(".log") || path.ends_with(".jsonl") {
        "log".to_string()
    } else {
        "source".to_string()
    }
}

fn git_head(workspace_root: &std::path::Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(workspace_root)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|head| !head.is_empty())
}

fn policy_for_task(store: &WebRuntimeStore) -> Result<PolicyPreset, TaskRunError> {
    let state = read_optional_json(&store.task_root().join("state.json"))?;
    let policy = state
        .get("policy_preset")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("manual-write");
    PolicyPreset::from_str(policy).map_err(|error| TaskRunError::new("web_runtime_policy", error))
}

fn run_internal_n00_if_needed(store: &WebRuntimeStore) -> Result<(), TaskRunError> {
    if node_event_exists(store, "N00")? {
        return Ok(());
    }
    store.append_event("node_started", "N00", json!({"status":"running"}))?;
    store.write_json(
        "node-runs/nrun_n00.json",
        &json!({
            "node_run_id": "nrun_n00",
            "node_id": "N00",
            "turn_id": null,
            "provider_run_id": null,
            "input_refs": [],
            "output_schema": null,
            "artifact_refs": ["internal_n00"],
            "status": "completed",
            "duration_ms": 1,
            "diagnostic_refs": [],
            "dropped": false,
            "created_at": "2026-05-09T00:00:00Z",
            "updated_at": "2026-05-09T00:00:00Z"
        }),
    )?;
    store.write_json(
        "artifacts/internal/n00.json",
        &json!({
            "artifact_ref": "internal_n00",
            "artifact_kind": "internal_step",
            "producer_node": "N00",
            "summary": "runtime bootstrap",
            "dropped": false
        }),
    )?;
    store.append_event(
        "artifact_written",
        "N00",
        json!({"status":"completed","artifact_ref":"internal_n00"}),
    )?;
    store.append_event("node_completed", "N00", json!({"status":"completed"}))
}

fn node_event_exists(store: &WebRuntimeStore, node_id: &str) -> Result<bool, TaskRunError> {
    let path = store.task_root().join("logs/node-events.jsonl");
    match fs::read_to_string(path) {
        Ok(events) => Ok(events.contains(&format!("\"node_id\":\"{node_id}\""))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error(error)),
    }
}

fn write_checkpoint(
    workspace_root: &std::path::Path,
    task_id: &str,
    store: &WebRuntimeStore,
    projection_version: u64,
) -> Result<(), TaskRunError> {
    store.write_json(
        "checkpoints/state@ckpt_0001.json",
        &read_optional_json(&store.task_root().join("state.json"))?,
    )?;
    store.write_json(
        "checkpoints/projection@ckpt_0001.json",
        &json!({"projection_version": projection_version}),
    )?;
    store.write_json(
        "checkpoints/ckpt_0001.json",
        &RuntimeCheckpoint {
            checkpoint_id: "ckpt_0001".to_string(),
            task_id: task_id.to_string(),
            session_id: "sess_task_0001".to_string(),
            turn_id: Some("turn_0001".to_string()),
            git_head: git_head(workspace_root),
            dirty_summary: json!({}),
            state_snapshot_ref: "state@ckpt_0001.json".to_string(),
            projection_snapshot_ref: "projection@ckpt_0001.json".to_string(),
            artifact_boundary: 0,
            provider_run_boundary: 0,
            node_run_boundary: 0,
            created_at: "2026-05-09T00:00:00Z".to_string(),
        },
    )?;
    Ok(())
}

fn pending_provider_step_for_policy(policy: PolicyPreset) -> PendingProviderStepDto {
    match policy {
        PolicyPreset::ManualAll => PendingProviderStepDto {
            node_id: "N04".to_string(),
            provider_type: "claude_code".to_string(),
            runtime_role: "orchestrator".to_string(),
            adapter_role: "orchestrator".to_string(),
            prompt: "执行 N04".to_string(),
            input_summary: json!({"node_id":"N04"}),
            canonical_input_refs: vec!["task:task_0001".to_string()],
            context_files: vec!["openspec/changes/aria-fibonacci-square/proposal.md".to_string()],
            output_schema: "schema://aria/artifacts/planning_report/v1".to_string(),
            allowed_write_scope: vec![".aria/runtime/".to_string(), "openspec/".to_string()],
            forbidden_actions: vec!["修改 cadence/project-rules".to_string()],
            verification_commands: vec!["cargo check --locked".to_string()],
            checkpoint_id: "ckpt_0001".to_string(),
        },
        PolicyPreset::ManualWrite | PolicyPreset::AutoReview | PolicyPreset::NonInteractive => {
            PendingProviderStepDto {
                node_id: "N16".to_string(),
                provider_type: "codex".to_string(),
                runtime_role: "executor".to_string(),
                adapter_role: "executor".to_string(),
                prompt: "实现 Fibonacci square sum".to_string(),
                input_summary: json!({"worktask_id":"work_wt_001"}),
                canonical_input_refs: vec!["worktask:work_wt_001".to_string()],
                context_files: vec!["openspec/changes/aria-fibonacci-square/tasks.md".to_string()],
                output_schema: "schema://aria/artifacts/coding_report/v1".to_string(),
                allowed_write_scope: vec!["src/".to_string(), "tests/".to_string()],
                forbidden_actions: vec!["修改 cadence/project-rules".to_string()],
                verification_commands: vec!["cargo test --locked -j 1".to_string()],
                checkpoint_id: "ckpt_0001".to_string(),
            }
        }
    }
}

fn write_class_for_pending(pending: &PendingProviderStepDto) -> NodeWriteClass {
    match pending.node_id.as_str() {
        "N16" | "N19" => NodeWriteClass::WritesWorkspace,
        "N04" | "N05" | "N07" | "N09" | "N10" | "N11" | "N12" | "N25" | "N26" | "N27" => {
            NodeWriteClass::WritesRuntime
        }
        _ => NodeWriteClass::ReadOnly,
    }
}
