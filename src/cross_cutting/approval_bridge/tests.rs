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

/// F3 Task 4.1（restrict-role-write-tools）：ApprovalBridge commandExecution
/// 既有链回归锁——Coder（非策略会话）的 commandExecution/fileChange 审批继续经
/// bridge `request_tool` 上抛，API 语义不因本 change 改变：
/// Auto 档即时批准并审计、Supervised 档等待 PermissionResponse（approve/deny
/// 原因回传）。codex session 的 `request.tool_name`（command/file_change）
/// 只是普通工具名入参，不得被策略分类改写。
#[tokio::test]
async fn approval_bridge_command_execution_chain_stays_unchanged_for_coder() {
    // —— Auto 档：commandExecution 形态立即放行并发出既有审计事件 ——
    let (event_tx, mut event_rx) = mpsc::channel(8);
    let bridge = ApprovalBridge::new(ProviderPermissionMode::Auto, event_tx);
    for tool_name in ["command", "file_change"] {
        let decision = bridge
            .request_tool(
                tool_name,
                "codex commandExecution requestApproval",
                RiskLevel::High,
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert!(decision.approved, "{tool_name} auto approval");
        assert_eq!(decision.reason.as_deref(), Some("auto_approved"));
    }
    for _ in 0..2 {
        match event_rx.recv().await.unwrap() {
            ProviderEvent::Execution(event) => assert_eq!(event.title, "Auto approval"),
            other => panic!("unexpected auto approval event: {other:?}"),
        }
    }

    // —— Supervised 档：上抛 PermissionRequest，approve→approved=true；deny→false ——
    let (event_tx, mut event_rx) = mpsc::channel(8);
    let bridge = ApprovalBridge::new(ProviderPermissionMode::Supervised, event_tx);
    let command_tx = bridge.command_sender();
    let supervised_bridge = std::sync::Arc::new(bridge);
    let approve_bridge = supervised_bridge.clone();
    let approve_task = tokio::spawn(async move {
        approve_bridge
            .request_tool(
                "command",
                "/bin/zsh -lc pnpm install",
                RiskLevel::High,
                CancellationToken::new(),
            )
            .await
            .unwrap()
    });
    let approved_id = receive_permission_request(&mut event_rx).await;
    assert_eq!(pending_len(&supervised_bridge).await, 1);
    command_tx
        .send(ProviderCommand::PermissionResponse {
            id: approved_id,
            approved: true,
            reason: None,
        })
        .await
        .unwrap();
    let approved = approve_task.await.unwrap();
    assert!(approved.approved);
    assert_eq!(approved.reason, None);

    let deny_bridge = supervised_bridge.clone();
    let deny_task = tokio::spawn(async move {
        deny_bridge
            .request_tool(
                "file_change",
                "写 src/main.rs",
                RiskLevel::High,
                CancellationToken::new(),
            )
            .await
            .unwrap()
    });
    let denied_id = receive_permission_request(&mut event_rx).await;
    command_tx
        .send(ProviderCommand::PermissionResponse {
            id: denied_id,
            approved: false,
            reason: Some("拒绝写面".to_string()),
        })
        .await
        .unwrap();
    let denied = deny_task.await.unwrap();
    assert!(!denied.approved);
    assert_eq!(denied.reason.as_deref(), Some("拒绝写面"));
    assert_eq!(pending_len(&supervised_bridge).await, 0);
}

// ---------------------------------------------------------------------------
// P0 1.3（REQ-WIGA-05）：choice 两层回执——mpsc 入队仅 Resolving；provider
// 等待者真正解出 ChoiceDecision 才 Delivered；结构拒绝为 Rejected。
// ---------------------------------------------------------------------------

fn choice_delivery_two_answers() -> Vec<crate::cross_cutting::streaming_provider::ChoiceAnswerData>
{
    use crate::cross_cutting::streaming_provider::ChoiceAnswerData;
    vec![
        ChoiceAnswerData {
            question_id: "q-1".to_string(),
            selected_option_ids: vec!["yes".to_string()],
            free_text: None,
        },
        ChoiceAnswerData {
            question_id: "q-2".to_string(),
            selected_option_ids: vec!["no".to_string()],
            free_text: None,
        },
    ]
}

async fn wait_for_choice_state(
    status: &mut tokio::sync::watch::Receiver<
        crate::cross_cutting::choice_delivery::ChoiceReplyState,
    >,
    expected: crate::cross_cutting::choice_delivery::ChoiceReplyState,
) {
    for _ in 0..200 {
        if *status.borrow() == expected {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    panic!(
        "choice receipt never reached {expected:?}, current: {:?}",
        *status.borrow()
    );
}

/// 受控 waiter：测试自持 pending entry 的 decision_rx，固定「命令已被
/// bridge 领取（Resolving）但 waiter 未解析」的状态边界——mpsc sender 成功
/// 不得冒充 Delivered。
#[tokio::test]
async fn choice_delivery_bridge_resolves_then_delivers_only_after_waiter_consumes() {
    use crate::cross_cutting::choice_delivery::{ChoiceDeliverySignal, ChoiceReplyState};
    use crate::cross_cutting::streaming_provider::ChoiceAnswerData;

    let (event_tx, _event_rx) = mpsc::channel(8);
    let bridge = ApprovalBridge::new(ProviderPermissionMode::Supervised, event_tx);
    let command_tx = bridge.command_sender();

    let (decision_tx, decision_rx) = oneshot::channel();
    bridge
        .pending_choices
        .lock()
        .await
        .insert("choice-1".to_string(), decision_tx);

    let (receipt, mut status) = ChoiceDeliverySignal::new();
    let two_answers = choice_delivery_two_answers();
    command_tx
        .send(ProviderCommand::ChoiceResponse {
            id: "choice-1".to_string(),
            selected_option_ids: vec!["a".to_string()],
            free_text: None,
            answers: two_answers.clone(),
            receipt: Some(receipt),
        })
        .await
        .unwrap();

    // bridge 领取命令 → Resolving；waiter 未解析时不得是 Delivered。
    wait_for_choice_state(&mut status, ChoiceReplyState::Resolving).await;
    assert_ne!(*status.borrow(), ChoiceReplyState::Delivered);

    // 释放 waiter（request_choice 同位点）：消费 decision、完整 answers 原样。
    let decision = decision_rx.await.expect("waiter decision");
    assert_eq!(decision.answers, two_answers);
    assert_eq!(
        decision.answers,
        vec![
            ChoiceAnswerData {
                question_id: "q-1".to_string(),
                selected_option_ids: vec!["yes".to_string()],
                free_text: None,
            },
            ChoiceAnswerData {
                question_id: "q-2".to_string(),
                selected_option_ids: vec!["no".to_string()],
                free_text: None,
            },
        ]
    );
    decision
        .receipt
        .as_ref()
        .expect("receipt forwarded to waiter")
        .deliver();
    assert_eq!(*status.borrow(), ChoiceReplyState::Delivered);
}

/// 真实 request_choice 集成：waiter 解出 decision 时内置推进 Delivered。
#[tokio::test]
async fn choice_delivery_request_choice_delivers_receipt_after_full_answers() {
    use crate::cross_cutting::choice_delivery::{ChoiceDeliverySignal, ChoiceReplyState};
    use crate::cross_cutting::streaming_provider::{ChoiceRequestData, ChoiceRequestSource};

    let (event_tx, mut event_rx) = mpsc::channel(8);
    let bridge = Arc::new(ApprovalBridge::new(
        ProviderPermissionMode::Supervised,
        event_tx,
    ));
    let command_tx = bridge.command_sender();

    let request = ChoiceRequestData {
        id: "choice-rt".to_string(),
        prompt: "选择部署策略".to_string(),
        options: Vec::new(),
        allow_multiple: false,
        allow_free_text: false,
        questions: Vec::new(),
        source: ChoiceRequestSource::ProviderChoice,
    };
    let wait_bridge = Arc::clone(&bridge);
    let waiting = tokio::spawn(async move {
        wait_bridge
            .request_choice(request, CancellationToken::new())
            .await
    });

    match tokio::time::timeout(TEST_TIMEOUT, event_rx.recv())
        .await
        .expect("choice request should be emitted")
        .expect("event channel open")
    {
        ProviderEvent::ChoiceRequest(request) => assert_eq!(request.id, "choice-rt"),
        other => panic!("unexpected provider event: {other:?}"),
    }

    let (receipt, mut status) = ChoiceDeliverySignal::new();
    let two_answers = choice_delivery_two_answers();
    command_tx
        .send(ProviderCommand::ChoiceResponse {
            id: "choice-rt".to_string(),
            selected_option_ids: vec![],
            free_text: None,
            answers: two_answers.clone(),
            receipt: Some(receipt),
        })
        .await
        .unwrap();

    let decision = tokio::time::timeout(TEST_TIMEOUT, waiting)
        .await
        .expect("request_choice should finish")
        .expect("waiter task must not panic")
        .expect("request_choice should succeed");
    assert_eq!(decision.answers, two_answers);
    wait_for_choice_state(&mut status, ChoiceReplyState::Delivered).await;
}

/// 无 pending 等待者（结构拒绝）→ Rejected；且不得推进 Delivered。
#[tokio::test]
async fn choice_delivery_unmatched_response_rejects_receipt() {
    use crate::cross_cutting::choice_delivery::{ChoiceDeliverySignal, ChoiceReplyState};

    let (event_tx, _event_rx) = mpsc::channel(8);
    let bridge = ApprovalBridge::new(ProviderPermissionMode::Supervised, event_tx);
    let command_tx = bridge.command_sender();

    let (receipt, mut status) = ChoiceDeliverySignal::new();
    command_tx
        .send(ProviderCommand::ChoiceResponse {
            id: "choice-no-waiter".to_string(),
            selected_option_ids: vec![],
            free_text: None,
            answers: Vec::new(),
            receipt: Some(receipt),
        })
        .await
        .unwrap();

    wait_for_choice_state(&mut status, ChoiceReplyState::Rejected).await;
    assert_ne!(*status.borrow(), ChoiceReplyState::Delivered);
}
