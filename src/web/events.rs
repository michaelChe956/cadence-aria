use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::sync::broadcast;

use crate::web::types::{ProviderOutputChunk, WebEvent};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WebEventType {
    ProjectionUpdated,
    NodeStarted,
    NodeCompleted,
    NodeFailed,
    PausedForApproval,
    ProviderInputPrepared,
    ProviderOutput,
    ArtifactWritten,
    GateBlocked,
    CheckpointCreated,
    RollbackPreviewed,
    RollbackCompleted,
    Error,
}

impl WebEventType {
    pub fn all() -> Vec<Self> {
        vec![
            Self::ProjectionUpdated,
            Self::NodeStarted,
            Self::NodeCompleted,
            Self::NodeFailed,
            Self::PausedForApproval,
            Self::ProviderInputPrepared,
            Self::ProviderOutput,
            Self::ArtifactWritten,
            Self::GateBlocked,
            Self::CheckpointCreated,
            Self::RollbackPreviewed,
            Self::RollbackCompleted,
            Self::Error,
        ]
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProjectionUpdated => "projection_updated",
            Self::NodeStarted => "node_started",
            Self::NodeCompleted => "node_completed",
            Self::NodeFailed => "node_failed",
            Self::PausedForApproval => "paused_for_approval",
            Self::ProviderInputPrepared => "provider.input_prepared",
            Self::ProviderOutput => "provider_output",
            Self::ArtifactWritten => "artifact_written",
            Self::GateBlocked => "gate_blocked",
            Self::CheckpointCreated => "checkpoint_created",
            Self::RollbackPreviewed => "rollback_previewed",
            Self::RollbackCompleted => "rollback_completed",
            Self::Error => "error",
        }
    }
}

#[derive(Clone)]
pub struct EventHub {
    inner: Arc<Mutex<EventHubInner>>,
    tx: broadcast::Sender<WebEvent>,
}

struct EventHubInner {
    cursor: u64,
    replay: VecDeque<WebEvent>,
}

impl EventHub {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(256);
        Self {
            inner: Arc::new(Mutex::new(EventHubInner {
                cursor: 0,
                replay: VecDeque::new(),
            })),
            tx,
        }
    }

    pub fn publish(
        &self,
        event_type: impl Into<String>,
        task_id: Option<&str>,
        payload: Value,
    ) -> WebEvent {
        let mut inner = self.inner.lock().expect("event hub lock");
        inner.cursor += 1;
        let event = WebEvent {
            cursor: inner.cursor,
            event_type: event_type.into(),
            task_id: task_id.map(str::to_string),
            payload,
        };
        inner.replay.push_back(event.clone());
        while inner.replay.len() > 512 {
            inner.replay.pop_front();
        }
        let _ = self.tx.send(event.clone());
        event
    }

    pub fn publish_provider_output(
        &self,
        task_id: Option<&str>,
        chunk: ProviderOutputChunk,
    ) -> WebEvent {
        let payload = serde_json::to_value(chunk).expect("provider output chunk");
        self.publish(WebEventType::ProviderOutput.as_str(), task_id, payload)
    }

    pub fn replay_after(&self, cursor: u64) -> Vec<WebEvent> {
        let inner = self.inner.lock().expect("event hub lock");
        inner
            .replay
            .iter()
            .filter(|event| event.cursor > cursor)
            .cloned()
            .collect()
    }

    pub fn subscribe_with_replay_after(
        &self,
        cursor: u64,
    ) -> (Vec<WebEvent>, broadcast::Receiver<WebEvent>) {
        let inner = self.inner.lock().expect("event hub lock");
        let replay = inner
            .replay
            .iter()
            .filter(|event| event.cursor > cursor)
            .cloned()
            .collect();
        let receiver = self.tx.subscribe();
        (replay, receiver)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<WebEvent> {
        self.tx.subscribe()
    }
}

impl Default for EventHub {
    fn default() -> Self {
        Self::new()
    }
}
