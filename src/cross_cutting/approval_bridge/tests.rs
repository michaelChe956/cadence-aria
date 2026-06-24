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

use super::ApprovalBridge;
use super::guards::PendingPermissionGuard;

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
async fn approval_bridge_auto_emits_auto_approval_event() {
    let (event_tx, mut event_rx) = mpsc::channel(8);
    let bridge = ApprovalBridge::new(ProviderPermissionMode::Auto, event_tx);

    let decision = bridge
        .request_tool(
            "Bash",
            "cargo test --locked",
            RiskLevel::Medium,
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert!(decision.approved);
    assert_eq!(decision.reason, Some("auto_approved".to_string()));
    let event = tokio::time::timeout(TEST_TIMEOUT, event_rx.recv())
        .await
        .expect("auto approval event")
        .expect("event");
    match event {
        ProviderEvent::Execution(event) => {
            assert_eq!(event.title, "Auto approval");
            assert!(
                event
                    .detail
                    .as_deref()
                    .unwrap_or_default()
                    .contains("cargo test --locked")
            );
            assert!(
                event
                    .output
                    .as_deref()
                    .unwrap_or_default()
                    .contains("\"auto_approved\":true")
            );
        }
        other => panic!("unexpected event: {other:?}"),
    }
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
