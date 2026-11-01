use super::*;

/// D①（红→绿）：abort_attempt 取消 runner token 时必须打点名取消者
/// （attempt_key + run_id + 触发原因）——biased-select masking 下，这是唯一
/// 不被握手错误文案掩盖的信号源。
#[tokio::test]
async fn abort_attempt_logs_cancellation_site_with_attempt_and_run_ids() {
    use crate::cross_cutting::tracing_capture::SharedTracingCapture;

    let registry = CodingRunRegistry::default();
    let attempt = CodingAttemptRunKey::new("project_diag", "issue_diag", "coding_attempt_diag");
    let (command_tx, command_rx) = mpsc::channel(1);
    let run_id = registry
        .insert_cancellable(&attempt, command_tx)
        .expect("diag runner")
        .run_id;
    drop(command_rx); // 接收端关闭：abort 无需等待移除即完成

    let capture = SharedTracingCapture::global();
    let sent = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        registry.abort_attempt(&attempt),
    )
    .await
    .expect("abort completes with closed receiver");

    assert_eq!(sent, 1);
    let logs = capture.captured();
    assert!(logs.contains("registry_abort_attempt"), "logs: {logs}");
    assert!(logs.contains("project_diag"), "logs: {logs}");
    assert!(logs.contains("issue_diag"), "logs: {logs}");
    assert!(logs.contains("coding_attempt_diag"), "logs: {logs}");
    assert!(logs.contains(&format!("run_id={run_id}")), "logs: {logs}");
}

#[tokio::test]
async fn aborts_all_runs_for_attempt_and_removes_them() {
    let registry = CodingRunRegistry::default();
    let attempt = CodingAttemptRunKey::new("project_0001", "issue_0001", "coding_attempt_0001");
    let other = CodingAttemptRunKey::new("project_0001", "issue_0001", "coding_attempt_0002");
    let (first_tx, mut first_rx) = mpsc::channel(1);
    let (second_tx, mut second_rx) = mpsc::channel(1);
    let (other_tx, mut other_rx) = mpsc::channel(1);

    let first_run_id = registry
        .insert_cancellable(&attempt, first_tx)
        .expect("first runner")
        .run_id;
    let second_run_id = registry
        .insert_cancellable(&attempt, second_tx)
        .expect("second runner")
        .run_id;
    registry
        .insert_cancellable(&other, other_tx)
        .expect("other runner");

    assert_eq!(registry.runner_count(&attempt), 2);
    let registry_for_abort = registry.clone();
    let attempt_for_abort = attempt.clone();
    let abort =
        tokio::spawn(async move { registry_for_abort.abort_attempt(&attempt_for_abort).await });
    assert_eq!(
        first_rx.recv().await.expect("first abort"),
        CodingRunnerCommand::AbortAttempt
    );
    assert_eq!(
        second_rx.recv().await.expect("second abort"),
        CodingRunnerCommand::AbortAttempt
    );
    tokio::task::yield_now().await;
    assert!(!abort.is_finished(), "abort must wait for runner removal");
    assert_eq!(registry.runner_count(&attempt), 2);
    registry.remove(&attempt, first_run_id);
    assert!(!abort.is_finished(), "abort must wait for every runner");
    registry.remove(&attempt, second_run_id);
    assert_eq!(abort.await.expect("abort task"), 2);
    assert_eq!(registry.runner_count(&attempt), 0);
    assert_eq!(registry.runner_count(&other), 1);
    assert!(other_rx.try_recv().is_err());
}

#[tokio::test]
async fn abort_retires_attempt_and_revokes_pending_reservation() {
    let registry = CodingRunRegistry::default();
    let attempt = CodingAttemptRunKey::new("project_0001", "issue_0001", "coding_attempt_0001");
    let reservation = registry
        .try_reserve_attempt(&attempt)
        .expect("pending reservation");

    assert_eq!(registry.abort_attempt(&attempt).await, 0);
    let (late_tx, _late_rx) = mpsc::channel(1);
    assert!(reservation.activate_cancellable(late_tx.clone()).is_none());
    assert!(registry.insert_cancellable(&attempt, late_tx).is_none());
    assert!(registry.try_reserve_attempt(&attempt).is_none());
    assert!(!registry.attempt_is_reserved_or_running(&attempt));
}

#[tokio::test]
async fn abort_completes_when_command_receiver_is_closed() {
    let registry = CodingRunRegistry::default();
    let attempt = CodingAttemptRunKey::new(
        "project_0001",
        "issue_0001",
        "coding_attempt_closed_receiver",
    );
    let (command_tx, command_rx) = mpsc::channel(1);
    registry
        .insert_cancellable(&attempt, command_tx)
        .expect("closed receiver runner");
    drop(command_rx);

    let sent = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        registry.abort_attempt(&attempt),
    )
    .await
    .expect("abort must not wait forever after command receiver closes");

    assert_eq!(sent, 1);
    assert_eq!(registry.runner_count(&attempt), 0);
}

#[tokio::test]
async fn cancellable_abort_ignores_full_command_channel_and_waits_for_removal() {
    let registry = CodingRunRegistry::default();
    let attempt = CodingAttemptRunKey::new(
        "project_0001",
        "issue_0001",
        "coding_attempt_cancellable_backpressure",
    );
    let (command_tx, _command_rx) = mpsc::channel(1);
    command_tx
        .try_send(CodingRunnerCommand::AbortAttempt)
        .expect("fill command channel");
    let registration = registry
        .insert_cancellable(&attempt, command_tx)
        .expect("cancellable runner");
    let cancellation = registration.cancellation.clone();
    let run_id = registration.run_id;
    let registry_for_runner = registry.clone();
    let attempt_for_runner = attempt.clone();
    let runner = tokio::spawn(async move {
        cancellation.cancelled().await;
        registry_for_runner.remove(&attempt_for_runner, run_id);
    });

    let cancelled = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        registry.abort_attempt(&attempt),
    )
    .await
    .expect("abort must not wait for command channel capacity");

    runner.await.expect("runner cancellation observer");
    assert_eq!(cancelled, 1);
    assert_eq!(registry.runner_count(&attempt), 0);
}

#[test]
fn scopes_reservations_by_attempt_identity() {
    let registry = CodingRunRegistry::default();
    let first = CodingAttemptRunKey::new("project_0001", "issue_0001", "coding_attempt_0001");
    let second = CodingAttemptRunKey::new("project_0001", "issue_0002", "coding_attempt_0001");

    let first_reservation = registry
        .try_reserve_attempt(&first)
        .expect("first reservation");
    let second_reservation = registry
        .try_reserve_attempt(&second)
        .expect("second scoped reservation");

    assert!(registry.has_active_recovery_reservation(&first));
    assert!(registry.has_active_recovery_reservation(&second));
    first_reservation.release();
    second_reservation.release();
}

#[tokio::test]
async fn aborts_only_exact_attempt_identity() {
    let registry = CodingRunRegistry::default();
    let first = CodingAttemptRunKey::new("project_0001", "issue_0001", "coding_attempt_0001");
    let second = CodingAttemptRunKey::new("project_0001", "issue_0002", "coding_attempt_0001");
    let (first_tx, mut first_rx) = mpsc::channel(1);
    let (second_tx, mut second_rx) = mpsc::channel(1);
    registry
        .insert_cancellable(&first, first_tx)
        .expect("first runner");
    let second_run_id = registry
        .insert_cancellable(&second, second_tx)
        .expect("second runner")
        .run_id;

    let registry_for_abort = registry.clone();
    let second_for_abort = second.clone();
    let abort =
        tokio::spawn(async move { registry_for_abort.abort_attempt(&second_for_abort).await });
    assert_eq!(
        second_rx.recv().await.expect("second abort"),
        CodingRunnerCommand::AbortAttempt
    );
    tokio::task::yield_now().await;
    assert!(
        !abort.is_finished(),
        "abort must wait for exact runner removal"
    );
    assert_eq!(registry.runner_count(&first), 1);
    assert_eq!(registry.runner_count(&second), 1);
    assert!(first_rx.try_recv().is_err());
    registry.remove(&second, second_run_id);
    assert_eq!(abort.await.expect("abort task"), 1);
    assert_eq!(registry.runner_count(&second), 0);
}

// ------------------------------------------------------------------
// P0 1.3（Task 9）：coding choice 应答 claim 门面——与 workspace 侧
// （Task 7）同语义：短锁仲裁唯一赢家、同 command 幂等、异 payload 冲突、
// run 终态后旧命令 Expired；Delivered 只能由 provider 等待者推进。
// ------------------------------------------------------------------

use crate::cross_cutting::choice_delivery::ChoiceReplyState;
use crate::cross_cutting::streaming_provider::ChoiceAnswerData;
use crate::web::choice_reply::ChoiceResponseRequest;
use crate::web::workspace_session::ChoiceReplyError;

fn choice_answers() -> Vec<ChoiceAnswerData> {
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

fn choice_request(command_id: &str, incarnation: &str) -> ChoiceResponseRequest {
    ChoiceResponseRequest {
        command_id: command_id.to_string(),
        expected_run_id: incarnation.to_string(),
        answers: choice_answers(),
    }
}

#[tokio::test]
async fn coding_choice_reply_claim_arbitrates_and_delivers_exactly_once() {
    let registry = CodingRunRegistry::default();
    let key = CodingAttemptRunKey::new("project_1", "issue_1", "attempt_0001");
    let (command_tx, mut command_rx) = mpsc::channel(8);
    let registration = registry
        .insert_cancellable(&key, command_tx)
        .expect("register runner");
    let incarnation = registry
        .active_run_incarnation(&key)
        .expect("active run incarnation");

    let request = choice_request("cmd-one", &incarnation);
    let (first, won) = registry
        .claim_choice(&key, "ch-1", &request)
        .expect("first claim");
    assert!(won);
    assert_eq!(first.state, ChoiceReplyState::Submitting);
    assert_eq!(first.choice_id, "ch-1");

    // 同 command 同 payload：幂等返回、不再二发。
    let (retry, won_again) = registry
        .claim_choice(&key, "ch-1", &request)
        .expect("idempotent claim");
    assert!(!won_again);
    assert_eq!(retry.state, ChoiceReplyState::Submitting);

    // 同 command 异 payload → Conflict；另一 command → Conflict。
    let mut divergent = choice_request("cmd-one", &incarnation);
    divergent.answers[0].selected_option_ids = vec!["no".to_string()];
    assert_eq!(
        registry.claim_choice(&key, "ch-1", &divergent).unwrap_err(),
        ChoiceReplyError::Conflict
    );
    assert_eq!(
        registry
            .claim_choice(&key, "ch-1", &choice_request("cmd-two", &incarnation))
            .unwrap_err(),
        ChoiceReplyError::Conflict
    );

    // 提交：runner 收到恰一条命令，answers 原样 + receipt 在场。
    registry
        .submit_claimed_choice(&key, "ch-1", &request)
        .await
        .expect("submit");
    match command_rx.recv().await.expect("runner command") {
        CodingRunnerCommand::ChoiceResponse {
            id,
            answers,
            receipt: Some(receipt),
            ..
        } => {
            assert_eq!(id, "ch-1");
            assert_eq!(answers.len(), 2);
            assert_eq!(answers[1].question_id, "q-2");
            // mpsc 入队 ≠ Delivered：等待者解析前 wait 只见 Resolving。
            receipt.mark_resolving();
            let paused = registry
                .wait_choice_receipt(
                    &key,
                    "ch-1",
                    "cmd-one",
                    std::time::Duration::from_millis(80),
                )
                .await
                .expect("paused wait");
            assert_eq!(paused.state, ChoiceReplyState::Resolving);
            receipt.deliver();
        }
        other => panic!("expected ChoiceResponse command, got {other:?}"),
    }
    assert!(command_rx.try_recv().is_err(), "不得二发 runner command");

    // Delivered 后 wait 收敛；同 command 查询持久可见。
    let delivered = registry
        .wait_choice_receipt(
            &key,
            "ch-1",
            "cmd-one",
            std::time::Duration::from_millis(500),
        )
        .await
        .expect("delivered wait");
    assert_eq!(delivered.state, ChoiceReplyState::Delivered);
    let status = registry
        .choice_status(&key, "ch-1", "cmd-one")
        .expect("status query");
    assert_eq!(status.state, ChoiceReplyState::Delivered);

    registry.remove(&key, registration.run_id());
}

#[tokio::test]
async fn coding_choice_reply_rejects_missing_or_retired_run() {
    let registry = CodingRunRegistry::default();
    let key = CodingAttemptRunKey::new("project_1", "issue_1", "attempt_0002");

    // 无 runner → Expired（410 语义）。
    assert_eq!(
        registry
            .claim_choice(&key, "ch-1", &choice_request("cmd-x", "any"))
            .unwrap_err(),
        ChoiceReplyError::Expired
    );

    let (command_tx, _command_rx) = mpsc::channel(8);
    let registration = registry
        .insert_cancellable(&key, command_tx)
        .expect("register runner");
    let incarnation = registry.active_run_incarnation(&key).unwrap();
    let request = choice_request("cmd-one", &incarnation);
    assert!(registry.claim_choice(&key, "ch-1", &request).unwrap().1);

    // 旧 expected_run_id（异 incarnation）→ Expired。
    assert_eq!(
        registry
            .claim_choice(
                &key,
                "ch-1",
                &choice_request("cmd-old", "stale-incarnation")
            )
            .unwrap_err(),
        ChoiceReplyError::Expired
    );

    // run 结束：旧 claim 置 Expired，同 command 查询见终态。
    registry.remove(&key, registration.run_id());
    assert_eq!(
        registry.claim_choice(&key, "ch-1", &request).unwrap_err(),
        ChoiceReplyError::Expired
    );
    assert_eq!(
        registry
            .choice_status(&key, "ch-1", "cmd-one")
            .unwrap()
            .state,
        ChoiceReplyState::Expired
    );

    // 未知 command → Unknown（404 语义）。
    assert_eq!(
        registry
            .choice_status(&key, "ch-1", "cmd-unknown")
            .unwrap_err(),
        ChoiceReplyError::Unknown
    );
}

#[tokio::test]
async fn coding_choice_reply_concurrent_claims_have_single_winner() {
    let registry = CodingRunRegistry::default();
    let key = CodingAttemptRunKey::new("project_1", "issue_1", "attempt_0003");
    let (command_tx, mut command_rx) = mpsc::channel(8);
    let registration = registry
        .insert_cancellable(&key, command_tx)
        .expect("register runner");
    let incarnation = registry.active_run_incarnation(&key).unwrap();

    // REST/WS 两路并发同 command+payload：唯一赢家，另一路幂等复看。
    let registry_b = registry.clone();
    let incarnation_b = incarnation.clone();
    let key_b = key.clone();
    let second = tokio::spawn(async move {
        let request = choice_request("cmd-race", &incarnation_b);
        registry_b
            .claim_choice(&key_b, "ch-race", &request)
            .expect("second claim")
    });
    let request = choice_request("cmd-race", &incarnation);
    let (first_status, first_won) = registry
        .claim_choice(&key, "ch-race", &request)
        .expect("first claim");
    let (second_status, second_won) = second.await.expect("join claim");
    assert!(
        first_won ^ second_won,
        "恰好一个赢家：{first_won}/{second_won}"
    );
    assert_eq!(first_status.command_id, second_status.command_id);

    // 唯一赢家提交后 runner 只收到一条命令。
    registry
        .submit_claimed_choice(&key, "ch-race", &request)
        .await
        .expect("submit");
    assert!(command_rx.recv().await.is_some());
    assert!(command_rx.try_recv().is_err(), "并发下不得二发");
    registry.remove(&key, registration.run_id());
}
