use std::fs;
use std::str::FromStr;

use serde_json::json;

use crate::interactive::checkpoint::{
    CheckpointService, RollbackPreviewRequest as CoreRollbackPreviewRequest,
    RollbackRequest as CoreRollbackRequest,
};
use crate::interactive::policy::{ConfirmationDecision, PolicyPreset, ProviderNodeMeta};
use crate::task_run::store::allocate_next_task_id;
use crate::task_run::types::TaskRunError;
use crate::web::runtime_store::WebRuntimeStore;
use crate::web::types::{
    AdvanceTaskResponse, ConfirmTaskRequest, ConfirmTaskResponse, CreateTaskRequest,
    CreateTaskResponse, RollbackPreviewResponse, RollbackResponse, StopTaskResponse, TaskListItem,
    TaskListResponse,
};

use super::WebRuntime;
use super::metadata::{
    pending_provider_step_for_policy, policy_for_task, run_internal_n00_if_needed,
    write_checkpoint, write_class_for_pending,
};
use super::utils::{io_error, json_error, read_optional_json};

impl WebRuntime {
    pub fn create_task(
        &mut self,
        request: CreateTaskRequest,
    ) -> Result<CreateTaskResponse, TaskRunError> {
        if !matches!(request.provider_mode.as_str(), "fake" | "real") {
            return Err(TaskRunError::new(
                "unsupported_provider_mode",
                format!("unsupported provider_mode: {}", request.provider_mode),
            ));
        }
        let task_id = allocate_next_task_id(&self.workspace_root)?;
        let session_id = format!("sess_{task_id}");
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
                "provider_mode": request.provider_mode,
                "request_text": request.request_text,
                "timeout_secs": request.timeout_secs
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
        let state = read_optional_json(&store.task_root().join("state.json"))?;
        run_internal_n00_if_needed(&store)?;
        write_checkpoint(
            &self.workspace_root,
            task_id,
            &store,
            self.next_projection_version,
        )?;
        let pending = pending_provider_step_for_policy(policy, &state);
        let decision = policy.decision_for(&ProviderNodeMeta::new(
            pending.node_id.clone(),
            pending.provider_type.clone(),
            write_class_for_pending(&pending),
        ));
        match decision {
            ConfirmationDecision::PauseForConfirmation => {
                store.write_json("pending/provider-step.json", &pending)?;
                Ok(AdvanceTaskResponse::PausedForApproval {
                    pending_step: Box::new(pending),
                })
            }
            ConfirmationDecision::RunAutomatically => {
                self.persist_provider_execution(
                    task_id,
                    &store,
                    pending.checkpoint_id,
                    pending.prompt,
                    &pending.provider_type,
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
        let state = read_optional_json(&store.task_root().join("state.json"))?;
        if state
            .get("provider_mode")
            .and_then(serde_json::Value::as_str)
            == Some("real")
        {
            let provider_type =
                self.resolve_real_confirm_provider(request.provider_type.as_deref())?;
            return self.persist_real_provider_execution(
                &store,
                &state,
                request.prompt,
                provider_type,
            );
        }
        self.persist_provider_execution(
            task_id,
            &store,
            request.checkpoint_id,
            request.prompt,
            &self.resolve_fake_confirm_provider(request.provider_type.as_deref())?,
        )
    }

    pub fn stop_task(&mut self, task_id: &str) -> Result<StopTaskResponse, TaskRunError> {
        Ok(StopTaskResponse {
            status: "stop_requested".to_string(),
            task_id: task_id.to_string(),
        })
    }

    pub fn rollback_preview(
        &self,
        task_id: &str,
        checkpoint_id: &str,
    ) -> Result<RollbackPreviewResponse, TaskRunError> {
        let preview = CheckpointService::new(&self.workspace_root, task_id).preview_rollback(
            CoreRollbackPreviewRequest {
                checkpoint_id: checkpoint_id.to_string(),
            },
        )?;
        Ok(preview.into())
    }

    pub fn rollback(
        &mut self,
        task_id: &str,
        checkpoint_id: &str,
        force_when_dirty: bool,
    ) -> Result<RollbackResponse, TaskRunError> {
        CheckpointService::new(&self.workspace_root, task_id).rollback(CoreRollbackRequest {
            checkpoint_id: checkpoint_id.to_string(),
            force_when_dirty,
        })?;
        let store = WebRuntimeStore::new(&self.workspace_root, task_id);
        let policy = policy_for_task(&store)?;
        let state = read_optional_json(&store.task_root().join("state.json"))?;
        let pending = pending_provider_step_for_policy(policy, &state);
        let decision = policy.decision_for(&ProviderNodeMeta::new(
            pending.node_id.clone(),
            pending.provider_type.clone(),
            write_class_for_pending(&pending),
        ));
        if matches!(decision, ConfirmationDecision::PauseForConfirmation) {
            store.write_json("pending/provider-step.json", &pending)?;
        }
        self.next_projection_version += 1;
        Ok(RollbackResponse {
            status: "rollback_completed".to_string(),
            checkpoint_id: checkpoint_id.to_string(),
        })
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
}
