use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: &str = "aria.repl.v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    Request,
    Response,
    Event,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Command {
    Hello,
    Attach,
    Subscribe,
    NewTask,
    GetStatus,
    ListArtifacts,
    ApproveGate,
    RejectGate,
    ReplyGate,
    Detach,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WireError {
    pub code: String,
    pub message: String,
    pub details: Option<Value>,
}

impl std::fmt::Display for WireError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for WireError {}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RequestEnvelope {
    pub protocol_version: String,
    pub message_type: MessageType,
    pub request_id: String,
    pub command: Command,
    pub sent_at: String,
    pub payload: Value,
}

impl RequestEnvelope {
    pub fn new<T: Serialize>(
        request_id: impl Into<String>,
        command: Command,
        payload: T,
    ) -> Result<Self, serde_json::Error> {
        Ok(Self {
            protocol_version: PROTOCOL_VERSION.to_string(),
            message_type: MessageType::Request,
            request_id: request_id.into(),
            command,
            sent_at: now_iso8601(),
            payload: serde_json::to_value(payload)?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ResponseEnvelope {
    pub protocol_version: String,
    pub message_type: MessageType,
    pub request_id: String,
    pub command: Command,
    pub sent_at: String,
    pub ok: bool,
    pub payload: Option<Value>,
    pub error: Option<WireError>,
}

impl ResponseEnvelope {
    pub fn success<T: Serialize>(
        request_id: impl Into<String>,
        command: Command,
        payload: T,
    ) -> Result<Self, serde_json::Error> {
        Ok(Self {
            protocol_version: PROTOCOL_VERSION.to_string(),
            message_type: MessageType::Response,
            request_id: request_id.into(),
            command,
            sent_at: now_iso8601(),
            ok: true,
            payload: Some(serde_json::to_value(payload)?),
            error: None,
        })
    }

    pub fn failure(request_id: impl Into<String>, command: Command, error: WireError) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION.to_string(),
            message_type: MessageType::Response,
            request_id: request_id.into(),
            command,
            sent_at: now_iso8601(),
            ok: false,
            payload: None,
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EventEnvelope {
    pub protocol_version: String,
    pub message_type: MessageType,
    pub event_id: u64,
    pub event_type: String,
    pub occurred_at: String,
    pub payload: Value,
}

impl EventEnvelope {
    pub fn new(
        event_id: u64,
        event_type: impl Into<String>,
        occurred_at: impl Into<String>,
        payload: Value,
    ) -> Result<Self, WireError> {
        let event_type = event_type.into();
        validate_event_payload(&event_type, &payload)?;
        Ok(Self {
            protocol_version: PROTOCOL_VERSION.to_string(),
            message_type: MessageType::Event,
            event_id,
            event_type,
            occurred_at: occurred_at.into(),
            payload,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HelloRequest {
    pub last_seen_event_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HelloResponse {
    pub daemon_session_id: String,
    pub protocol_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AttachRequest {
    pub reconnect_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct AttachResponse {
    pub reconnect_token: String,
    pub replay_cursor: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SubscribeRequest {
    pub event_types: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SubscribeResponse {
    pub subscription_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NewTaskRequest {
    pub request_text: String,
    pub requested_change_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NewTaskResponse {
    pub task_id: String,
    pub phase: String,
    pub intake_ref: String,
    pub change_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GetStatusRequest {
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GetStatusResponse {
    pub session_id: String,
    pub tasks: Vec<Value>,
    pub latest_event_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ListArtifactsRequest {
    pub task_id: String,
    pub artifact_kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ListArtifactsResponse {
    pub artifacts: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ApproveGateRequest {
    pub gate_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RejectGateRequest {
    pub gate_id: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ReplyGateRequest {
    pub gate_id: String,
    pub reply_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct GateResolutionResponse {
    pub gate_id: String,
    pub resolution: String,
    pub next_route: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DetachRequest {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DetachResponse {
    pub detached: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventSchema {
    pub event_type: &'static str,
    pub required_fields: &'static [&'static str],
}

pub fn event_registry() -> &'static [EventSchema] {
    &[
        EventSchema {
            event_type: "task.created",
            required_fields: &["task_id", "phase"],
        },
        EventSchema {
            event_type: "task.phase_changed",
            required_fields: &["task_id", "from_phase", "to_phase"],
        },
        EventSchema {
            event_type: "artifact.materialized",
            required_fields: &["artifact_ref", "artifact_type", "producer_node"],
        },
        EventSchema {
            event_type: "projection.compiled",
            required_fields: &["projection_ref", "projection_kind", "source_artifact_ref"],
        },
        EventSchema {
            event_type: "constraint_bundle.compiled",
            required_fields: &["constraint_bundle_ref", "change_id", "bundle_status"],
        },
        EventSchema {
            event_type: "traceability.updated",
            required_fields: &["binding_refs", "coverage_status"],
        },
        EventSchema {
            event_type: "gate.opened",
            required_fields: &["gate_id", "gate_type", "blocking_node"],
        },
        EventSchema {
            event_type: "gate.resolved",
            required_fields: &["gate_id", "resolution", "next_route"],
        },
        EventSchema {
            event_type: "provider_run.started",
            required_fields: &["provider_run_id", "node_id", "provider_type"],
        },
        EventSchema {
            event_type: "provider_run.completed",
            required_fields: &["provider_run_id", "exit_code", "duration_ms"],
        },
        EventSchema {
            event_type: "provider_run.failed",
            required_fields: &["provider_run_id", "error_code", "retryable"],
        },
        EventSchema {
            event_type: "policy_mode.degraded",
            required_fields: &["task_id", "requested_mode", "effective_mode", "reason"],
        },
        EventSchema {
            event_type: "worktree.lease_acquired",
            required_fields: &["lease_id", "worktask_id", "worktree_path", "base_ref"],
        },
        EventSchema {
            event_type: "openspec.rollback",
            required_fields: &["change_id", "rollback_ref", "reason", "old_bundle_ref"],
        },
    ]
}

pub fn validate_event_payload(event_type: &str, payload: &Value) -> Result<(), WireError> {
    let schema = event_registry()
        .iter()
        .find(|candidate| candidate.event_type == event_type)
        .ok_or_else(|| WireError {
            code: "unknown_event_type".to_string(),
            message: format!("event type {event_type} is not registered"),
            details: None,
        })?;

    for field in schema.required_fields {
        if payload.get(field).is_none() {
            return Err(WireError {
                code: "invalid_event_payload".to_string(),
                message: format!("event {event_type} is missing required field {field}"),
                details: Some(serde_json::json!({ "missing_field": field })),
            });
        }
    }

    Ok(())
}

fn now_iso8601() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}
