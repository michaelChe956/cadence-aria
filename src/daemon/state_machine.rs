use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::daemon::checkpoint::{RiskRegistrySnapshot, RuntimeSnapshot};
use crate::daemon::recovery::{EventLogIndex, ReplayDecision, ReplayWindow};
use crate::daemon::task_registry::{TaskRuntimeState, TaskSummary};
use crate::protocol::nodes::{N00, N01, N02, N03};
use crate::protocol::policies::{OpenSpecBootstrapStatus, PolicyMode};
use crate::protocol::repl_wire::{
    ApproveGateRequest, AttachRequest, AttachResponse, Command, DetachResponse,
    GateResolutionResponse, GetStatusRequest, GetStatusResponse, HelloRequest, HelloResponse,
    ListArtifactsRequest, ListArtifactsResponse, NewTaskRequest, NewTaskResponse,
    RejectGateRequest, ReplyGateRequest, RequestEnvelope, ResponseEnvelope, SubscribeRequest,
    SubscribeResponse, WireError, PROTOCOL_VERSION,
};

#[derive(Debug)]
pub struct DaemonState {
    workspace_root: PathBuf,
    daemon_session_id: String,
    latest_event_id: u64,
    first_retained_event_id: u64,
    next_task_sequence: u64,
    tasks: BTreeMap<String, TaskRuntimeState>,
    task_artifacts: BTreeMap<String, Vec<Value>>,
    events: Vec<crate::protocol::repl_wire::EventEnvelope>,
}

impl DaemonState {
    pub fn bootstrap(workspace_root: &Path) -> anyhow::Result<Self> {
        Ok(Self {
            workspace_root: workspace_root.to_path_buf(),
            daemon_session_id: format!("sess_{}", uuid::Uuid::new_v4().simple()),
            latest_event_id: 0,
            first_retained_event_id: 1,
            next_task_sequence: 1,
            tasks: BTreeMap::new(),
            task_artifacts: BTreeMap::new(),
            events: Vec::new(),
        })
    }

    pub fn task(&self, task_id: &str) -> Option<&TaskRuntimeState> {
        self.tasks.get(task_id)
    }

    pub fn events(&self) -> &[crate::protocol::repl_wire::EventEnvelope] {
        &self.events
    }

    pub fn set_replay_floor_for_test(&mut self, first_retained_event_id: u64) {
        self.first_retained_event_id = first_retained_event_id;
    }

    pub fn persist_checkpoint(&self) -> anyhow::Result<()> {
        let runtime_dir = self.workspace_root.join(".aria/runtime");
        fs::create_dir_all(&runtime_dir)?;
        let visible_tasks: Vec<TaskSummary> = self.tasks.values().map(TaskSummary::from).collect();
        let checkpoint = SessionCheckpoint {
            daemon_session_id: self.daemon_session_id.clone(),
            latest_event_id: self.latest_event_id,
            attached_clients: vec![],
            open_gates: vec![],
            visible_tasks,
            tasks: self.tasks.values().cloned().collect(),
            timestamp: now_iso8601(),
        };
        fs::write(
            runtime_dir.join("session.json"),
            serde_json::to_vec_pretty(&checkpoint)?,
        )?;
        Ok(())
    }

    pub fn recover(workspace_root: &Path) -> anyhow::Result<Self> {
        let session_path = workspace_root.join(".aria/runtime/session.json");
        let checkpoint: SessionCheckpoint = serde_json::from_slice(&fs::read(session_path)?)?;
        let tasks: BTreeMap<String, TaskRuntimeState> = checkpoint
            .tasks
            .into_iter()
            .map(|task| (task.task_id.clone(), task))
            .collect();

        let first_retained_event_id = load_event_log_index(workspace_root)?
            .map(|index| index.first_retained_event_id)
            .unwrap_or(1);

        Ok(Self {
            workspace_root: workspace_root.to_path_buf(),
            daemon_session_id: checkpoint.daemon_session_id,
            latest_event_id: checkpoint.latest_event_id,
            first_retained_event_id,
            next_task_sequence: tasks.len() as u64 + 1,
            tasks,
            task_artifacts: BTreeMap::new(),
            events: Vec::new(),
        })
    }

    pub fn new_task(&mut self, request: NewTaskRequest) -> Result<NewTaskResponse, WireError> {
        self.new_task_with_policy(
            &request.request_text,
            request.requested_change_id,
            PolicyMode::Conservative,
        )
    }

    pub fn new_task_with_policy(
        &mut self,
        request_text: &str,
        requested_change_id: Option<String>,
        requested_policy: PolicyMode,
    ) -> Result<NewTaskResponse, WireError> {
        if request_text.trim().is_empty() {
            return Err(invalid_request("request_text must not be empty"));
        }

        let task_id = format!("task_{:04}", self.next_task_sequence);
        self.next_task_sequence += 1;

        let change_id = match requested_change_id {
            Some(change_id) => {
                if is_valid_change_id(&change_id) {
                    change_id
                } else {
                    return Err(invalid_request("requested_change_id is invalid"));
                }
            }
            None => format!("chg_{task_id}"),
        };

        let effective_policy = PolicyMode::Conservative;
        if requested_policy != PolicyMode::Conservative {
            self.emit_event(
                "policy_mode.degraded",
                json!({
                    "task_id": task_id,
                    "requested_mode": requested_policy,
                    "effective_mode": effective_policy,
                    "reason": "phase1_forces_conservative_policy"
                }),
            )?;
        }

        let intake_ref = self.materialize_intake_brief(&task_id, request_text)?;
        let risk_registry_ref = format!("riskreg_{task_id}_v0001");

        let task = TaskRuntimeState {
            task_id: task_id.clone(),
            phase: "intake".to_string(),
            change_id: change_id.clone(),
            effective_policy,
            intake_ref: intake_ref.clone(),
            risk_registry_ref,
            openspec_bootstrap_status: OpenSpecBootstrapStatus::BootstrapPending,
            protocol_steps: vec![
                N00.to_string(),
                N01.to_string(),
                N02.to_string(),
                N03.to_string(),
            ],
        };
        self.tasks.insert(task_id.clone(), task);

        self.emit_event(
            "task.created",
            json!({
                "task_id": task_id,
                "phase": "intake"
            }),
        )?;
        let task = self
            .tasks
            .get(&task_id)
            .expect("task was just inserted")
            .clone();
        self.write_task_state(&task)?;
        self.write_empty_risk_registry(&task)?;
        self.write_protocol_step_snapshots(&task)?;
        self.persist_checkpoint().map_err(internal_error)?;

        Ok(NewTaskResponse {
            task_id,
            phase: "intake".to_string(),
            intake_ref,
            change_id,
        })
    }

    pub fn handle_request(
        &mut self,
        request: RequestEnvelope,
    ) -> Result<ResponseEnvelope, WireError> {
        match request.command {
            Command::Hello => {
                let payload = decode_payload::<HelloRequest>(&request.payload)?;
                let replay_window = ReplayWindow {
                    latest_event_id: self.latest_event_id,
                    first_retained_event_id: self.first_retained_event_id,
                };
                if matches!(
                    replay_window.decide(payload.last_seen_event_id),
                    ReplayDecision::WindowLost
                ) {
                    return Ok(ResponseEnvelope::failure(
                        request.request_id,
                        request.command,
                        replay_window_lost(self.first_retained_event_id, self.latest_event_id),
                    ));
                }
                response_success(
                    &request.request_id,
                    request.command,
                    HelloResponse {
                        daemon_session_id: self.daemon_session_id.clone(),
                        protocol_version: PROTOCOL_VERSION.to_string(),
                    },
                )
            }
            Command::Attach => {
                decode_payload::<AttachRequest>(&request.payload)?;
                response_success(
                    &request.request_id,
                    request.command,
                    AttachResponse {
                        reconnect_token: format!("reconnect_{}", self.daemon_session_id),
                        replay_cursor: Some(self.latest_event_id),
                    },
                )
            }
            Command::Subscribe => {
                decode_payload::<SubscribeRequest>(&request.payload)?;
                response_success(
                    &request.request_id,
                    request.command,
                    SubscribeResponse {
                        subscription_id: format!("sub_{}", self.daemon_session_id),
                    },
                )
            }
            Command::NewTask => {
                let payload = decode_payload::<NewTaskRequest>(&request.payload)?;
                match self.new_task(payload) {
                    Ok(response) => {
                        response_success(&request.request_id, request.command, response)
                    }
                    Err(error) => Ok(ResponseEnvelope::failure(
                        request.request_id,
                        request.command,
                        error,
                    )),
                }
            }
            Command::GetStatus => {
                let payload = decode_payload::<GetStatusRequest>(&request.payload)?;
                let tasks: Vec<Value> = self
                    .tasks
                    .values()
                    .filter(|task| {
                        payload
                            .task_id
                            .as_ref()
                            .map_or(true, |expected| expected == &task.task_id)
                    })
                    .map(TaskSummary::from)
                    .map(|summary| serde_json::to_value(summary_to_json(summary)).unwrap())
                    .collect();

                response_success(
                    &request.request_id,
                    request.command,
                    GetStatusResponse {
                        session_id: self.daemon_session_id.clone(),
                        tasks,
                        latest_event_id: self.latest_event_id,
                    },
                )
            }
            Command::ListArtifacts => {
                let payload = decode_payload::<ListArtifactsRequest>(&request.payload)?;
                let artifacts = self
                    .task_artifacts
                    .get(&payload.task_id)
                    .into_iter()
                    .flatten()
                    .filter(|artifact| {
                        payload.artifact_kind.as_ref().map_or(true, |expected| {
                            artifact
                                .get("artifact_kind")
                                .and_then(Value::as_str)
                                .is_some_and(|actual| actual == expected)
                        })
                    })
                    .cloned()
                    .collect();
                response_success(
                    &request.request_id,
                    request.command,
                    ListArtifactsResponse { artifacts },
                )
            }
            Command::ApproveGate => {
                let payload = decode_payload::<ApproveGateRequest>(&request.payload)?;
                self.resolve_gate(
                    &request.request_id,
                    request.command,
                    payload.gate_id,
                    "approved",
                )
            }
            Command::RejectGate => {
                let payload = decode_payload::<RejectGateRequest>(&request.payload)?;
                self.resolve_gate(
                    &request.request_id,
                    request.command,
                    payload.gate_id,
                    "rejected",
                )
            }
            Command::ReplyGate => {
                let payload = decode_payload::<ReplyGateRequest>(&request.payload)?;
                self.resolve_gate(
                    &request.request_id,
                    request.command,
                    payload.gate_id,
                    "reply_recorded",
                )
            }
            Command::Detach => {
                decode_payload::<crate::protocol::repl_wire::DetachRequest>(&request.payload)?;
                response_success(
                    &request.request_id,
                    request.command,
                    DetachResponse { detached: true },
                )
            }
        }
    }

    fn materialize_intake_brief(
        &mut self,
        task_id: &str,
        request_text: &str,
    ) -> Result<String, WireError> {
        let artifact_id = format!("art_intake_brief_{task_id}_0001");
        let artifact_ref_id = format!("ref_{artifact_id}_v0001");
        let artifact_dir = self
            .workspace_root
            .join(".aria/runtime/tasks")
            .join(task_id)
            .join("artifacts/intake_brief");
        let artifact_path = artifact_dir.join(format!("{artifact_id}_v0001.json"));
        fs::create_dir_all(&artifact_dir).map_err(io_error)?;

        let content = json!({
            "intake_id": format!("intake_{task_id}_0001"),
            "request_text": request_text,
            "origin_type": "user_repl",
            "created_at": now_iso8601(),
            "task_id": task_id
        });
        let bytes = serde_json::to_vec_pretty(&content).map_err(internal_error)?;
        fs::write(&artifact_path, &bytes).map_err(io_error)?;

        let sha256 = hex::encode(Sha256::digest(&bytes));
        let artifact_record = json!({
            "artifact_ref_id": artifact_ref_id,
            "artifact_id": artifact_id,
            "version": "0001",
            "artifact_kind": "intake_brief",
            "status": "active",
            "path": artifact_path.to_string_lossy(),
            "sha256": sha256
        });
        fs::write(
            artifact_dir.join("artifact_index.json"),
            serde_json::to_vec_pretty(&json!({
                "task_id": task_id,
                "artifacts": [artifact_record.clone()]
            }))
            .map_err(internal_error)?,
        )
        .map_err(io_error)?;
        fs::write(
            artifact_dir.join("latest.json"),
            serde_json::to_vec_pretty(&json!({
                "active_ref": artifact_ref_id
            }))
            .map_err(internal_error)?,
        )
        .map_err(io_error)?;

        self.task_artifacts
            .entry(task_id.to_string())
            .or_default()
            .push(artifact_record);

        self.emit_event(
            "artifact.materialized",
            json!({
                "artifact_ref": artifact_ref_id,
                "artifact_type": "intake_brief",
                "producer_node": N01
            }),
        )?;

        Ok(artifact_ref_id)
    }

    fn emit_event(&mut self, event_type: &str, payload: Value) -> Result<(), WireError> {
        self.latest_event_id += 1;
        let event = crate::protocol::repl_wire::EventEnvelope::new(
            self.latest_event_id,
            event_type,
            now_iso8601(),
            payload,
        )?;
        self.append_event_log(&event)?;
        self.events.push(event);
        Ok(())
    }

    fn resolve_gate(
        &mut self,
        request_id: &str,
        command: Command,
        gate_id: String,
        resolution: &str,
    ) -> Result<ResponseEnvelope, WireError> {
        self.emit_event(
            "gate.resolved",
            json!({
                "gate_id": gate_id,
                "resolution": resolution,
                "next_route": Value::Null
            }),
        )?;
        response_success(
            request_id,
            command,
            GateResolutionResponse {
                gate_id,
                resolution: resolution.to_string(),
                next_route: None,
            },
        )
    }

    fn append_event_log(
        &self,
        event: &crate::protocol::repl_wire::EventEnvelope,
    ) -> Result<(), WireError> {
        let event_dir = self.workspace_root.join(".aria/runtime/events");
        fs::create_dir_all(&event_dir).map_err(io_error)?;
        let event_path = event_dir.join(format!("{}.jsonl", self.daemon_session_id));
        let index_path = event_dir.join("index.json");
        let disk_event = json!({
            "event_id": event.event_id,
            "event_type": event.event_type,
            "created_at": event.occurred_at,
            "payload": event.payload
        });
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(event_path)
            .map_err(io_error)?;
        writeln!(
            file,
            "{}",
            serde_json::to_string(&disk_event).map_err(internal_error)?
        )
        .map_err(io_error)?;

        let mut first_retained_event_id_by_task = if index_path.exists() {
            fs::read(&index_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
                .and_then(|value| value.get("first_retained_event_id_by_task").cloned())
                .and_then(|value| value.as_object().cloned())
                .unwrap_or_default()
        } else {
            serde_json::Map::new()
        };
        if let Some(task_id) = event
            .payload
            .get("task_id")
            .and_then(Value::as_str)
            .map(str::to_string)
        {
            first_retained_event_id_by_task
                .entry(task_id)
                .or_insert_with(|| json!(event.event_id));
        }

        let index = json!({
            "daemon_session_id": self.daemon_session_id,
            "latest_event_id": self.latest_event_id,
            "first_retained_event_id": self.first_retained_event_id,
            "first_retained_event_id_by_task": first_retained_event_id_by_task,
            "compacted_segments": []
        });
        fs::write(
            index_path,
            serde_json::to_vec_pretty(&index).map_err(internal_error)?,
        )
        .map_err(io_error)?;
        Ok(())
    }

    fn write_task_state(&self, task: &TaskRuntimeState) -> Result<(), WireError> {
        let task_dir = self
            .workspace_root
            .join(".aria/runtime/tasks")
            .join(&task.task_id);
        fs::create_dir_all(&task_dir).map_err(io_error)?;
        fs::write(
            task_dir.join("task.json"),
            serde_json::to_vec_pretty(task).map_err(internal_error)?,
        )
        .map_err(io_error)?;
        Ok(())
    }

    fn write_empty_risk_registry(&self, task: &TaskRuntimeState) -> Result<(), WireError> {
        let risk_registry_dir = self
            .workspace_root
            .join(".aria/runtime/tasks")
            .join(&task.task_id)
            .join("risk-registry");
        let ref_dir = risk_registry_dir.join("refs");
        fs::create_dir_all(&ref_dir).map_err(io_error)?;

        let registry = RiskRegistrySnapshot {
            risk_registry_ref: task.risk_registry_ref.clone(),
            risk_ids: vec![],
            risks: vec![],
        };
        fs::write(
            risk_registry_dir.join("registry.json"),
            serde_json::to_vec_pretty(&registry).map_err(internal_error)?,
        )
        .map_err(io_error)?;

        fs::write(
            ref_dir.join(format!("{}.json", task.risk_registry_ref)),
            serde_json::to_vec_pretty(&json!({
                "risk_registry_ref": task.risk_registry_ref,
                "task_id": task.task_id,
                "registry_path": "risk-registry/registry.json",
                "risk_count": 0
            }))
            .map_err(internal_error)?,
        )
        .map_err(io_error)?;
        Ok(())
    }

    fn write_protocol_step_snapshots(&self, task: &TaskRuntimeState) -> Result<(), WireError> {
        let snapshot_dir = self
            .workspace_root
            .join(".aria/runtime/tasks")
            .join(&task.task_id)
            .join("snapshots");
        fs::create_dir_all(&snapshot_dir).map_err(io_error)?;

        for node_id in &task.protocol_steps {
            let snapshot = RuntimeSnapshot {
                snapshot_id: format!("snap_{}_{}", task.task_id, node_id.to_lowercase()),
                session_id: self.daemon_session_id.clone(),
                task_id: task.task_id.clone(),
                node_id: node_id.clone(),
                phase: task.phase.clone(),
                timestamp: now_iso8601(),
                effective_policy: task.effective_policy.clone(),
                artifact_refs: vec![task.intake_ref.clone()],
                provider_run_refs: vec![],
                worktree_ref: None,
                rework_counter: 0,
                risk_registry: RiskRegistrySnapshot {
                    risk_registry_ref: task.risk_registry_ref.clone(),
                    risk_ids: vec![],
                    risks: vec![],
                },
                loop_counters: BTreeMap::new(),
                superseded_artifact_refs: vec![],
                node_specific_fields: json!({
                    "openspec_bootstrap_status": task.openspec_bootstrap_status
                }),
                projection_refs: vec![],
                constraint_bundle_refs: vec![],
            };
            snapshot.validate().map_err(|message| WireError {
                code: "invalid_runtime_snapshot".to_string(),
                message,
                details: None,
            })?;
            fs::write(
                snapshot_dir.join(format!("{node_id}.json")),
                serde_json::to_vec_pretty(&snapshot).map_err(internal_error)?,
            )
            .map_err(io_error)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct SessionCheckpoint {
    daemon_session_id: String,
    latest_event_id: u64,
    #[serde(default)]
    attached_clients: Vec<Value>,
    #[serde(default)]
    open_gates: Vec<Value>,
    visible_tasks: Vec<TaskSummary>,
    tasks: Vec<TaskRuntimeState>,
    timestamp: String,
}

fn load_event_log_index(workspace_root: &Path) -> anyhow::Result<Option<EventLogIndex>> {
    let index_path = workspace_root.join(".aria/runtime/events/index.json");
    if !index_path.exists() {
        return Ok(None);
    }

    Ok(Some(serde_json::from_slice(&fs::read(index_path)?)?))
}

fn decode_payload<T: serde::de::DeserializeOwned>(payload: &Value) -> Result<T, WireError> {
    serde_json::from_value(payload.clone()).map_err(|error| WireError {
        code: "invalid_request".to_string(),
        message: error.to_string(),
        details: None,
    })
}

fn response_success<T: serde::Serialize>(
    request_id: &str,
    command: Command,
    payload: T,
) -> Result<ResponseEnvelope, WireError> {
    ResponseEnvelope::success(request_id, command, payload).map_err(internal_error)
}

fn summary_to_json(summary: TaskSummary) -> Value {
    json!({
        "task_id": summary.task_id,
        "phase": summary.phase,
        "change_id": summary.change_id,
        "effective_policy": summary.effective_policy
    })
}

fn is_valid_change_id(change_id: &str) -> bool {
    change_id.starts_with("chg_")
        && change_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        })
}

fn invalid_request(message: impl Into<String>) -> WireError {
    WireError {
        code: "invalid_request".to_string(),
        message: message.into(),
        details: None,
    }
}

fn replay_window_lost(first_retained_event_id: u64, latest_event_id: u64) -> WireError {
    WireError {
        code: "replay_window_lost".to_string(),
        message: "event replay window has been compacted".to_string(),
        details: Some(json!({
            "first_retained_event_id": first_retained_event_id,
            "latest_event_id": latest_event_id
        })),
    }
}

fn io_error(error: std::io::Error) -> WireError {
    WireError {
        code: "io_error".to_string(),
        message: error.to_string(),
        details: None,
    }
}

fn internal_error(error: impl std::fmt::Display) -> WireError {
    WireError {
        code: "internal_error".to_string(),
        message: error.to_string(),
        details: None,
    }
}

fn now_iso8601() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
