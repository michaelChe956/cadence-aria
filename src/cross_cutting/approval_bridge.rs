use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::streaming_provider::{
    ChoiceRequestData, PermissionRequestData, ProviderCommand, ProviderEvent,
    ProviderPermissionMode, RiskLevel,
};

static NEXT_PERMISSION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionDecision {
    pub approved: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoiceDecision {
    pub selected_option_ids: Vec<String>,
    pub free_text: Option<String>,
}

type PendingPermissions =
    Arc<Mutex<HashMap<String, (oneshot::Sender<PermissionDecision>, Instant)>>>;
type PendingChoices = Arc<Mutex<HashMap<String, oneshot::Sender<ChoiceDecision>>>>;

#[cfg(not(test))]
const PERMISSION_CLEANUP_INTERVAL: Duration = Duration::from_secs(60);
#[cfg(test)]
const PERMISSION_CLEANUP_INTERVAL: Duration = Duration::from_millis(10);
#[cfg(not(test))]
const PERMISSION_TIMEOUT: Duration = Duration::from_secs(900);
#[cfg(test)]
const PERMISSION_TIMEOUT: Duration = Duration::from_millis(30);

struct PendingPermissionGuard {
    id: Option<String>,
    pending: PendingPermissions,
}

impl PendingPermissionGuard {
    fn new(id: String, pending: PendingPermissions) -> Self {
        Self {
            id: Some(id),
            pending,
        }
    }

    async fn remove_now(&mut self) {
        let Some(id) = self.id.as_ref().cloned() else {
            return;
        };
        self.pending.lock().await.remove(&id);
        self.id = None;
    }
}

impl Drop for PendingPermissionGuard {
    fn drop(&mut self) {
        let Some(id) = self.id.take() else {
            return;
        };
        let pending = Arc::clone(&self.pending);
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                pending.lock().await.remove(&id);
            });
        }
    }
}

struct PendingChoiceGuard {
    id: Option<String>,
    pending: PendingChoices,
}

impl PendingChoiceGuard {
    fn new(id: String, pending: PendingChoices) -> Self {
        Self {
            id: Some(id),
            pending,
        }
    }

    async fn remove_now(&mut self) {
        let Some(id) = self.id.as_ref().cloned() else {
            return;
        };
        self.pending.lock().await.remove(&id);
        self.id = None;
    }
}

impl Drop for PendingChoiceGuard {
    fn drop(&mut self) {
        let Some(id) = self.id.take() else {
            return;
        };
        let pending = Arc::clone(&self.pending);
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                pending.lock().await.remove(&id);
            });
        }
    }
}

pub struct ApprovalBridge {
    mode: ProviderPermissionMode,
    event_tx: mpsc::Sender<ProviderEvent>,
    command_tx: mpsc::Sender<ProviderCommand>,
    pending: PendingPermissions,
    pending_choices: PendingChoices,
    cleanup_cancel: CancellationToken,
}

impl ApprovalBridge {
    pub fn new(mode: ProviderPermissionMode, event_tx: mpsc::Sender<ProviderEvent>) -> Self {
        let (command_tx, command_rx) = mpsc::channel(8);
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let pending_choices: PendingChoices = Arc::new(Mutex::new(HashMap::new()));
        let cleanup_cancel = CancellationToken::new();

        tokio::spawn(listen_for_permission_commands(
            command_rx,
            Arc::clone(&pending),
            Arc::clone(&pending_choices),
            event_tx.clone(),
        ));
        tokio::spawn(cleanup_pending_permissions(
            Arc::clone(&pending),
            event_tx.clone(),
            cleanup_cancel.clone(),
        ));

        Self {
            mode,
            event_tx,
            command_tx,
            pending,
            pending_choices,
            cleanup_cancel,
        }
    }

    pub fn command_sender(&self) -> mpsc::Sender<ProviderCommand> {
        self.command_tx.clone()
    }

    pub async fn request_tool(
        &self,
        tool_name: &str,
        description: &str,
        risk_level: RiskLevel,
        cancel: CancellationToken,
    ) -> Result<PermissionDecision, ProviderAdapterError> {
        if self.mode == ProviderPermissionMode::Auto {
            return Ok(PermissionDecision {
                approved: true,
                reason: None,
            });
        }

        let id = next_permission_id();
        let (decision_tx, decision_rx) = oneshot::channel();
        self.pending
            .lock()
            .await
            .insert(id.clone(), (decision_tx, Instant::now()));
        let mut pending_guard = PendingPermissionGuard::new(id.clone(), Arc::clone(&self.pending));

        let request = ProviderEvent::PermissionRequest(PermissionRequestData {
            id: id.clone(),
            tool_name: tool_name.to_string(),
            description: description.to_string(),
            risk_level,
        });

        let send_result = tokio::select! {
            _ = cancel.cancelled() => {
                pending_guard.remove_now().await;
                return Err(permission_bridge_error("permission request cancelled"));
            }
            result = self.event_tx.send(request) => result,
        };

        if send_result.is_err() {
            pending_guard.remove_now().await;
            return Err(permission_bridge_error(
                "permission request event receiver closed",
            ));
        }

        tokio::select! {
            _ = cancel.cancelled() => {
                pending_guard.remove_now().await;
                Err(permission_bridge_error("permission request cancelled"))
            }
            _ = self.event_tx.closed() => {
                pending_guard.remove_now().await;
                Err(permission_bridge_error("permission request event receiver closed"))
            }
            decision = decision_rx => {
                pending_guard.remove_now().await;
                decision.map_err(|_| permission_bridge_error("permission response channel closed"))
            }
        }
    }

    pub async fn request_choice(
        &self,
        request: ChoiceRequestData,
        cancel: CancellationToken,
    ) -> Result<ChoiceDecision, ProviderAdapterError> {
        let id = request.id.clone();
        let (decision_tx, decision_rx) = oneshot::channel();
        self.pending_choices
            .lock()
            .await
            .insert(id.clone(), decision_tx);
        let mut pending_guard = PendingChoiceGuard::new(id, Arc::clone(&self.pending_choices));

        let send_result = tokio::select! {
            _ = cancel.cancelled() => {
                pending_guard.remove_now().await;
                return Err(permission_bridge_error("choice request cancelled"));
            }
            result = self.event_tx.send(ProviderEvent::ChoiceRequest(request)) => result,
        };

        if send_result.is_err() {
            pending_guard.remove_now().await;
            return Err(permission_bridge_error(
                "choice request event receiver closed",
            ));
        }

        tokio::select! {
            _ = cancel.cancelled() => {
                pending_guard.remove_now().await;
                Err(permission_bridge_error("choice request cancelled"))
            }
            _ = self.event_tx.closed() => {
                pending_guard.remove_now().await;
                Err(permission_bridge_error("choice request event receiver closed"))
            }
            decision = decision_rx => {
                pending_guard.remove_now().await;
                decision.map_err(|_| permission_bridge_error("choice response channel closed"))
            }
        }
    }
}

impl Drop for ApprovalBridge {
    fn drop(&mut self) {
        self.cleanup_cancel.cancel();
    }
}

async fn listen_for_permission_commands(
    mut command_rx: mpsc::Receiver<ProviderCommand>,
    pending: PendingPermissions,
    pending_choices: PendingChoices,
    event_tx: mpsc::Sender<ProviderEvent>,
) {
    while let Some(command) = command_rx.recv().await {
        match command {
            ProviderCommand::PermissionResponse {
                id,
                approved,
                reason,
            } => {
                tracing::info!(permission_id = %id, approved, "bridge received permission response");
                if let Some((decision_tx, _created_at)) = pending.lock().await.remove(&id) {
                    tracing::info!(permission_id = %id, "bridge dispatched decision to pending");
                    let _ = decision_tx.send(PermissionDecision { approved, reason });
                } else {
                    tracing::warn!(permission_id = %id, "bridge: no pending entry for id");
                    let _ = event_tx
                        .send(ProviderEvent::ProtocolError {
                            code: "PERMISSION_ID_UNMATCHED".to_string(),
                            message: format!("PermissionResponse id={id} not found in pending"),
                            context: Some(serde_json::json!({ "permission_id": id })),
                        })
                        .await;
                }
            }
            ProviderCommand::ChoiceResponse {
                id,
                selected_option_ids,
                free_text,
            } => {
                tracing::info!(choice_id = %id, "bridge received choice response");
                if let Some(decision_tx) = pending_choices.lock().await.remove(&id) {
                    let _ = decision_tx.send(ChoiceDecision {
                        selected_option_ids,
                        free_text,
                    });
                } else {
                    tracing::warn!(choice_id = %id, "bridge: no pending choice entry for id");
                    let _ = event_tx
                        .send(ProviderEvent::ProtocolError {
                            code: "CHOICE_ID_UNMATCHED".to_string(),
                            message: format!("ChoiceResponse id={id} not found in pending"),
                            context: Some(serde_json::json!({ "choice_id": id })),
                        })
                        .await;
                }
            }
            ProviderCommand::Abort => {
                let mut pending_permissions = pending.lock().await;
                for (_, (decision_tx, _created_at)) in pending_permissions.drain() {
                    let _ = decision_tx.send(PermissionDecision {
                        approved: false,
                        reason: Some("aborted".to_string()),
                    });
                }
                let mut pending_choices = pending_choices.lock().await;
                for (_, decision_tx) in pending_choices.drain() {
                    let _ = decision_tx.send(ChoiceDecision {
                        selected_option_ids: Vec::new(),
                        free_text: Some("aborted".to_string()),
                    });
                }
            }
            ProviderCommand::ToolResult(_) => {}
        }
    }
}

async fn cleanup_pending_permissions(
    pending: PendingPermissions,
    event_tx: mpsc::Sender<ProviderEvent>,
    cancel: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = tokio::time::sleep(PERMISSION_CLEANUP_INTERVAL) => {}
        }
        let now = Instant::now();
        let expired_ids: Vec<String> = {
            let guard = pending.lock().await;
            guard
                .iter()
                .filter(|(_, (_, created_at))| now.duration_since(*created_at) > PERMISSION_TIMEOUT)
                .map(|(id, _)| id.clone())
                .collect()
        };
        let timed_out_ids: Vec<String> = {
            let mut guard = pending.lock().await;
            expired_ids
                .into_iter()
                .filter_map(|id| {
                    guard.remove(&id).map(|(decision_tx, _created_at)| {
                        drop(decision_tx);
                        id
                    })
                })
                .collect()
        };

        for id in timed_out_ids {
            tokio::select! {
                _ = cancel.cancelled() => return,
                result = event_tx.send(ProviderEvent::PermissionTimeout { permission_id: id }) => {
                    let _ = result;
                }
            }
        }
    }
}

fn next_permission_id() -> String {
    let id = NEXT_PERMISSION_ID.fetch_add(1, Ordering::Relaxed);
    format!("permission_{id}")
}

fn permission_bridge_error(message: &'static str) -> ProviderAdapterError {
    ProviderAdapterError::permission_denied(message, String::new(), String::new())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::future::Future;
    use std::sync::Arc;
    use std::task::Poll;
    use std::time::{Duration, Instant};

    use futures_util::task::noop_waker_ref;
    use tokio::sync::{Mutex, mpsc, oneshot};
    use tokio_util::sync::CancellationToken;

    use crate::cross_cutting::streaming_provider::{
        ProviderCommand, ProviderEvent, ProviderPermissionMode, RiskLevel,
    };
    use crate::protocol::provider_errors::ProviderErrorCode;

    use super::{ApprovalBridge, PendingPermissionGuard};

    const TEST_TIMEOUT: Duration = Duration::from_secs(1);

    async fn receive_permission_request(event_rx: &mut mpsc::Receiver<ProviderEvent>) -> String {
        match tokio::time::timeout(TEST_TIMEOUT, event_rx.recv())
            .await
            .expect("permission request should be emitted")
            .expect("permission event channel should stay open")
        {
            ProviderEvent::PermissionRequest(request) => request.id,
            other => panic!("unexpected provider event: {other:?}"),
        }
    }

    async fn pending_len(bridge: &ApprovalBridge) -> usize {
        bridge.pending.lock().await.len()
    }

    async fn wait_for_pending_len(bridge: &ApprovalBridge, expected_len: usize) {
        for _ in 0..200 {
            let actual_len = pending_len(bridge).await;
            if actual_len == expected_len {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!(
            "pending permission count did not reach {expected_len}; actual len is {}",
            pending_len(bridge).await
        );
    }

    #[tokio::test]
    async fn approval_bridge_auto_approves_without_emitting_request() {
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let bridge = ApprovalBridge::new(ProviderPermissionMode::Auto, event_tx);

        let decision = bridge
            .request_tool(
                "git status",
                "查看当前工作区状态",
                RiskLevel::Low,
                CancellationToken::new(),
            )
            .await
            .unwrap();

        assert!(decision.approved);
        assert_eq!(decision.reason, None);
        assert!(event_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn approval_bridge_supervised_waits_for_permission_response() {
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let bridge = ApprovalBridge::new(ProviderPermissionMode::Supervised, event_tx);
        let command_tx = bridge.command_sender();

        let decision_task = tokio::spawn(async move {
            bridge
                .request_tool(
                    "cargo test",
                    "运行完整测试套件",
                    RiskLevel::High,
                    CancellationToken::new(),
                )
                .await
                .unwrap()
        });

        let request_id = match event_rx.recv().await.unwrap() {
            ProviderEvent::PermissionRequest(request) => {
                assert_eq!(request.tool_name, "cargo test");
                assert_eq!(request.description, "运行完整测试套件");
                assert_eq!(request.risk_level, RiskLevel::High);
                request.id
            }
            other => panic!("unexpected provider event: {other:?}"),
        };

        command_tx
            .send(ProviderCommand::PermissionResponse {
                id: request_id,
                approved: false,
                reason: Some("命令范围过大".to_string()),
            })
            .await
            .unwrap();

        let decision = decision_task.await.unwrap();
        assert!(!decision.approved);
        assert_eq!(decision.reason, Some("命令范围过大".to_string()));
    }

    #[tokio::test]
    async fn approval_bridge_returns_error_when_event_receiver_closes_after_request() {
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let bridge = Arc::new(ApprovalBridge::new(
            ProviderPermissionMode::Supervised,
            event_tx,
        ));
        let wait_bridge = Arc::clone(&bridge);

        let decision_task = tokio::spawn(async move {
            wait_bridge
                .request_tool(
                    "bash",
                    "Run cargo test",
                    RiskLevel::Medium,
                    CancellationToken::new(),
                )
                .await
        });

        let _request_id = receive_permission_request(&mut event_rx).await;
        wait_for_pending_len(&bridge, 1).await;
        drop(event_rx);

        let error = tokio::time::timeout(TEST_TIMEOUT, decision_task)
            .await
            .expect("event channel close should finish permission request")
            .expect("permission request task should not panic")
            .unwrap_err();
        assert_eq!(error.code, ProviderErrorCode::ProviderPermissionDenied);
        assert_eq!(error.details, "permission request event receiver closed");
        wait_for_pending_len(&bridge, 0).await;
    }

    #[tokio::test]
    async fn approval_bridge_cancel_cleans_pending_and_preserves_error_reason() {
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let bridge = Arc::new(ApprovalBridge::new(
            ProviderPermissionMode::Supervised,
            event_tx,
        ));
        let wait_bridge = Arc::clone(&bridge);
        let cancel = CancellationToken::new();
        let request_cancel = cancel.clone();

        let decision_task = tokio::spawn(async move {
            wait_bridge
                .request_tool("bash", "Run cargo test", RiskLevel::Medium, request_cancel)
                .await
        });

        let _request_id = receive_permission_request(&mut event_rx).await;
        wait_for_pending_len(&bridge, 1).await;
        cancel.cancel();

        let error = tokio::time::timeout(TEST_TIMEOUT, decision_task)
            .await
            .expect("cancel should finish permission request")
            .expect("permission request task should not panic")
            .unwrap_err();
        assert_eq!(error.code, ProviderErrorCode::ProviderPermissionDenied);
        assert_eq!(error.details, "permission request cancelled");
        wait_for_pending_len(&bridge, 0).await;
    }

    #[tokio::test]
    async fn approval_bridge_abort_drains_pending_as_rejected_decision() {
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let bridge = Arc::new(ApprovalBridge::new(
            ProviderPermissionMode::Supervised,
            event_tx,
        ));
        let command_tx = bridge.command_sender();
        let wait_bridge = Arc::clone(&bridge);

        let decision_task = tokio::spawn(async move {
            wait_bridge
                .request_tool(
                    "bash",
                    "Run cargo test",
                    RiskLevel::Medium,
                    CancellationToken::new(),
                )
                .await
                .unwrap()
        });

        let _request_id = receive_permission_request(&mut event_rx).await;
        wait_for_pending_len(&bridge, 1).await;

        command_tx.send(ProviderCommand::Abort).await.unwrap();

        let decision = tokio::time::timeout(TEST_TIMEOUT, decision_task)
            .await
            .expect("abort should finish permission request")
            .expect("permission request task should not panic");
        assert!(!decision.approved);
        assert_eq!(decision.reason.as_deref(), Some("aborted"));
        wait_for_pending_len(&bridge, 0).await;
    }

    #[tokio::test]
    async fn approval_bridge_unmatched_response_does_not_complete_request() {
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let bridge = Arc::new(ApprovalBridge::new(
            ProviderPermissionMode::Supervised,
            event_tx,
        ));
        let command_tx = bridge.command_sender();
        let wait_bridge = Arc::clone(&bridge);

        let decision_task = tokio::spawn(async move {
            wait_bridge
                .request_tool(
                    "bash",
                    "Run cargo test",
                    RiskLevel::Medium,
                    CancellationToken::new(),
                )
                .await
                .unwrap()
        });

        let request_id = receive_permission_request(&mut event_rx).await;
        command_tx
            .send(ProviderCommand::PermissionResponse {
                id: "permission_not_pending".to_string(),
                approved: true,
                reason: Some("wrong request".to_string()),
            })
            .await
            .unwrap();
        match tokio::time::timeout(TEST_TIMEOUT, event_rx.recv())
            .await
            .expect("unmatched response should emit protocol_error")
            .expect("event channel should stay open")
        {
            ProviderEvent::ProtocolError { code, context, .. } => {
                assert_eq!(code, "PERMISSION_ID_UNMATCHED");
                assert_eq!(
                    context
                        .as_ref()
                        .and_then(|value| value.get("permission_id"))
                        .and_then(|value| value.as_str()),
                    Some("permission_not_pending")
                );
            }
            other => panic!("unexpected provider event: {other:?}"),
        }
        command_tx
            .send(ProviderCommand::PermissionResponse {
                id: request_id,
                approved: false,
                reason: Some("matched request".to_string()),
            })
            .await
            .unwrap();

        let decision = tokio::time::timeout(TEST_TIMEOUT, decision_task)
            .await
            .expect("matching response should finish permission request")
            .expect("permission request task should not panic");
        assert!(!decision.approved);
        assert_eq!(decision.reason.as_deref(), Some("matched request"));
        wait_for_pending_len(&bridge, 0).await;
    }

    #[tokio::test]
    async fn approval_bridge_times_out_pending_permission_without_denying_provider() {
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let bridge = Arc::new(ApprovalBridge::new(
            ProviderPermissionMode::Supervised,
            event_tx,
        ));
        let wait_bridge = Arc::clone(&bridge);

        let decision_task = tokio::spawn(async move {
            wait_bridge
                .request_tool(
                    "bash",
                    "Run cargo test",
                    RiskLevel::Medium,
                    CancellationToken::new(),
                )
                .await
        });

        let request_id = receive_permission_request(&mut event_rx).await;

        match tokio::time::timeout(TEST_TIMEOUT, event_rx.recv())
            .await
            .expect("timeout event should be emitted")
            .expect("event channel should stay open")
        {
            ProviderEvent::PermissionTimeout { permission_id } => {
                assert_eq!(permission_id, request_id);
            }
            other => panic!("unexpected provider event: {other:?}"),
        }

        let decision = tokio::time::timeout(TEST_TIMEOUT, decision_task)
            .await
            .expect("permission request should finish after timeout")
            .expect("permission request task should not panic");
        assert!(decision.is_err());
        wait_for_pending_len(&bridge, 0).await;
    }

    #[tokio::test]
    async fn approval_bridge_dropping_request_future_cleans_pending() {
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let bridge = Arc::new(ApprovalBridge::new(
            ProviderPermissionMode::Supervised,
            event_tx,
        ));
        let wait_bridge = Arc::clone(&bridge);

        let decision_task = tokio::spawn(async move {
            wait_bridge
                .request_tool(
                    "bash",
                    "Run cargo test",
                    RiskLevel::Medium,
                    CancellationToken::new(),
                )
                .await
        });

        let _request_id = receive_permission_request(&mut event_rx).await;
        wait_for_pending_len(&bridge, 1).await;

        decision_task.abort();
        let _ = decision_task.await;

        wait_for_pending_len(&bridge, 0).await;
    }

    #[tokio::test]
    async fn approval_bridge_pending_guard_drop_cleans_when_remove_now_future_is_dropped_while_waiting_for_lock()
     {
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (decision_tx, _decision_rx) = oneshot::channel();
        pending
            .lock()
            .await
            .insert("permission_test".to_string(), (decision_tx, Instant::now()));
        let pending_lock = pending.lock().await;
        let mut guard =
            PendingPermissionGuard::new("permission_test".to_string(), Arc::clone(&pending));
        let mut remove = Box::pin(guard.remove_now());
        let waker = noop_waker_ref();
        let mut context = std::task::Context::from_waker(waker);

        assert!(matches!(
            Future::poll(remove.as_mut(), &mut context),
            Poll::Pending
        ));

        drop(remove);
        drop(guard);
        drop(pending_lock);

        for _ in 0..200 {
            if pending.lock().await.is_empty() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("pending permission should be cleaned after guard drop");
    }
}
