use std::time::Duration;

use axum::Json;
use axum::extract::{Path, State};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;

use crate::web::state::WebAppState;

use super::{TestControls, WorkspaceSocketControl};

#[derive(Debug, Deserialize)]
pub struct WsTimeoutRequest {
    pub server_idle_timeout_ms: Option<u64>,
    pub client_idle_timeout_ms: Option<u64>,
    pub suppress_server_messages: Option<bool>,
    pub session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct WsRejectRequest {
    pub count: u32,
}

pub async fn drop_workspace_socket(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
) -> Json<serde_json::Value> {
    let dropped = state
        .test_controls
        .drop_workspace_socket_when_registered(&session_id, Duration::from_secs(2))
        .await;
    Json(json!({"status": "ok", "dropped": dropped}))
}

pub async fn reject_next_workspace_sockets(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
    Json(request): Json<WsRejectRequest>,
) -> Json<serde_json::Value> {
    state
        .test_controls
        .reject_next_workspace_sockets(session_id, request.count)
        .await;
    Json(json!({"status": "ok"}))
}

pub async fn set_ws_timeout(
    State(state): State<WebAppState>,
    Json(request): Json<WsTimeoutRequest>,
) -> Json<serde_json::Value> {
    if let Some(timeout_ms) = request.server_idle_timeout_ms {
        state
            .test_controls
            .set_server_idle_timeout(Duration::from_millis(timeout_ms))
            .await;
    }
    let _ = (
        request.client_idle_timeout_ms,
        request.suppress_server_messages,
        request.session_id,
    );
    Json(json!({"status": "ok"}))
}

impl TestControls {
    pub async fn register_workspace_socket(
        &self,
        session_id: String,
        sender: mpsc::Sender<WorkspaceSocketControl>,
    ) {
        self.inner
            .workspace_sockets
            .lock()
            .expect("test controls workspace socket lock")
            .entry(session_id)
            .or_default()
            .push(sender);
    }

    pub async fn reject_next_workspace_sockets(&self, session_id: String, count: u32) {
        if count == 0 {
            self.inner
                .workspace_socket_rejects
                .lock()
                .expect("test controls workspace socket rejects lock")
                .remove(&session_id);
            return;
        }
        self.inner
            .workspace_socket_rejects
            .lock()
            .expect("test controls workspace socket rejects lock")
            .insert(session_id, count);
    }

    pub async fn consume_workspace_socket_reject(&self, session_id: &str) -> bool {
        let mut rejects = self
            .inner
            .workspace_socket_rejects
            .lock()
            .expect("test controls workspace socket rejects lock");
        let Some(count) = rejects.get_mut(session_id) else {
            return false;
        };
        if *count <= 1 {
            rejects.remove(session_id);
        } else {
            *count -= 1;
        }
        true
    }

    pub async fn drop_workspace_socket(&self, session_id: &str) -> bool {
        let senders = self
            .inner
            .workspace_sockets
            .lock()
            .expect("test controls workspace socket lock")
            .remove(session_id)
            .unwrap_or_default();

        let mut dropped = false;
        for sender in senders {
            if sender
                .send(WorkspaceSocketControl::CloseForTestDrop)
                .await
                .is_ok()
            {
                dropped = true;
            }
        }
        dropped
    }

    pub async fn drop_workspace_socket_when_registered(
        &self,
        session_id: &str,
        timeout: Duration,
    ) -> bool {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if self.drop_workspace_socket(session_id).await {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    }

    pub fn server_idle_timeout(&self) -> Duration {
        self.inner
            .server_idle_timeout
            .lock()
            .expect("test controls server idle timeout lock")
            .unwrap_or(Duration::from_secs(90))
    }

    pub async fn set_server_idle_timeout(&self, timeout: Duration) {
        *self
            .inner
            .server_idle_timeout
            .lock()
            .expect("test controls server idle timeout lock") = Some(timeout);
    }
}
