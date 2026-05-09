use std::fs;
use std::path::PathBuf;
use std::process::Command;

use serde_json::json;

use crate::interactive::models::{RuntimeCheckpoint, WebWorkspaceProjection};
use crate::interactive::projection::build_workspace_projection;
use crate::interactive::web_projection::build_web_projection;
use crate::task_run::types::TaskRunError;
use crate::web::runtime_store::WebRuntimeStore;
use crate::web::types::{
    AdvanceTaskResponse, ConfirmTaskRequest, ConfirmTaskResponse, CreateTaskRequest,
    CreateTaskResponse, PendingProviderStepDto,
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
                "change_id": request.change_id,
                "current_node": "N16"
            }))
            .map_err(json_error)?,
        )
        .map_err(io_error)?;
        Ok(CreateTaskResponse {
            task_id,
            session_id,
            change_id: request.change_id,
            phase: "intake".to_string(),
        })
    }

    pub fn advance_task(&mut self, task_id: &str) -> Result<AdvanceTaskResponse, TaskRunError> {
        let store = WebRuntimeStore::new(&self.workspace_root, task_id);
        store.write_json(
            "checkpoints/state@ckpt_0001.json",
            &read_optional_json(&store.task_root().join("state.json"))?,
        )?;
        store.write_json(
            "checkpoints/projection@ckpt_0001.json",
            &json!({"projection_version": self.next_projection_version}),
        )?;
        store.write_json(
            "checkpoints/ckpt_0001.json",
            &RuntimeCheckpoint {
                checkpoint_id: "ckpt_0001".to_string(),
                task_id: task_id.to_string(),
                session_id: "sess_task_0001".to_string(),
                turn_id: Some("turn_0001".to_string()),
                git_head: git_head(&self.workspace_root),
                dirty_summary: json!({}),
                state_snapshot_ref: "state@ckpt_0001.json".to_string(),
                projection_snapshot_ref: "projection@ckpt_0001.json".to_string(),
                artifact_boundary: 0,
                provider_run_boundary: 0,
                node_run_boundary: 0,
                created_at: "2026-05-09T00:00:00Z".to_string(),
            },
        )?;
        let pending = PendingProviderStepDto {
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
        };
        store.write_json("pending/provider-step.json", &pending)?;
        Ok(AdvanceTaskResponse::PausedForApproval {
            pending_step: pending,
        })
    }

    pub fn confirm_task(
        &mut self,
        task_id: &str,
        request: ConfirmTaskRequest,
    ) -> Result<ConfirmTaskResponse, TaskRunError> {
        let store = WebRuntimeStore::new(&self.workspace_root, task_id);
        let checkpoint_id = request.checkpoint_id.clone();
        let prompt = request.prompt.clone();
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
                "prompt_snapshot": prompt,
                "input_summary": {"worktask_id":"work_wt_001"},
                "checkpoint_before": checkpoint_id,
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
                "prompt": request.prompt,
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
                "checkpoint_id":request.checkpoint_id,
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
