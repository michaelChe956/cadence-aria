// P0 1.3（REQ-WIGA-05）：workspace choice claim 门面——REST/WS 共用仲裁、
// run 化身判旧、回执 Delivered 前不摘 pending 帧。Task 7 红绿锚点。

use crate::cross_cutting::choice_delivery::ChoiceReplyState;
use crate::cross_cutting::streaming_provider::{ChoiceAnswerData, ProviderCommand};
use crate::web::choice_reply::ChoiceResponseRequest;
use crate::web::workspace_session::ChoiceReplyError;
use crate::web::workspace_ws_types::WsOutMessage;

fn claim_two_answers() -> Vec<ChoiceAnswerData> {
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

fn claim_request(
    command_id: &str,
    run_incarnation: &str,
    answers: Vec<ChoiceAnswerData>,
) -> ChoiceResponseRequest {
    ChoiceResponseRequest {
        command_id: command_id.to_string(),
        expected_run_id: run_incarnation.to_string(),
        answers,
    }
}

/// fixture 临时目录需要活过 manager 生命周期——集中持有防止提前清理。
static FIXTURE_ROOTS: std::sync::Mutex<Vec<tempfile::TempDir>> = std::sync::Mutex::new(Vec::new());

fn claim_manager(session_id: &str) -> Arc<WorkspaceSessionManager> {
    // 复用 part_02 的 automation fixture 构造思路：每个测试独立 tmp 目录。
    let tmp = tempfile::tempdir().unwrap();
    let paths = crate::product::app_paths::ProductAppPaths::new(tmp.path().join(".aria"));
    FIXTURE_ROOTS.lock().unwrap().push(tmp);
    let mut record = crate::web::workspace_session::manager::test_session_record(session_id);
    record.project_id = "project_claim".to_string();
    record.issue_id = "issue_claim".to_string();
    let (engine_tx, _engine_rx) = mpsc::channel(1);
    let engine = crate::product::workspace_engine::WorkspaceEngine::new(
        Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
            paths.issue_lifecycle_root("project_claim", "issue_claim"),
        )),
        engine_tx,
        crate::product::workspace_engine::WorkspaceSession::from_record(record.clone()),
    );
    crate::web::workspace_session::manager::WorkspaceSessionManager::test_fixture_with_parts(
        session_id,
        Arc::new(tokio::sync::Mutex::new(engine)),
        Arc::new(crate::cross_cutting::provider_registry::ProviderRegistry::new()),
        paths,
        record,
    )
}

async fn started_claim_run(manager: &Arc<WorkspaceSessionManager>) -> (u64, String) {
    let (_id, token, _cancel, _command_rx, _node) = manager
        .start_run(
            crate::web::workspace_ws_handler::ProviderRunKind::ReviewOnly,
            None,
        )
        .await
        .expect("start run");
    let incarnation = manager
        .active_run_incarnation()
        .expect("run incarnation after start");
    (token, incarnation)
}

#[tokio::test]
async fn workspace_choice_claim_two_concurrent_claimants_single_winner() {
    let manager = claim_manager("session_claim_winner");
    let (_token, incarnation) = started_claim_run(&manager).await;
    manager.register_pending_choice_frame(provider_choice_frame("choice-1"));

    let request_a = claim_request("cmd-a", &incarnation, claim_two_answers());
    let request_b = claim_request("cmd-b", &incarnation, claim_two_answers());

    // HTTP/WS 两路并发（同一门面）：唯一赢家。
    let (status, won) = manager.claim_choice("choice-1", &request_a).unwrap();
    assert!(won);
    assert_eq!(status.state, ChoiceReplyState::Submitting);
    assert_eq!(status.choice_id, "choice-1");
    assert_eq!(status.expected_run_id, incarnation);

    // 另一 claimant（另一 command）→ Conflict。
    assert_eq!(
        manager.claim_choice("choice-1", &request_b).unwrap_err(),
        ChoiceReplyError::Conflict
    );

    // 同 command 同 payload → 原状态幂等返回、不二发。
    let (same, won_again) = manager.claim_choice("choice-1", &request_a).unwrap();
    assert!(!won_again);
    assert_eq!(same.state, ChoiceReplyState::Submitting);

    // 同 command 异 payload → Conflict。
    let mut mutated = claim_request("cmd-a", &incarnation, claim_two_answers());
    mutated.answers[0].selected_option_ids = vec!["no".to_string()];
    assert_eq!(
        manager.claim_choice("choice-1", &mutated).unwrap_err(),
        ChoiceReplyError::Conflict
    );

    // 状态查询：未知 command → Unknown。
    assert_eq!(
        manager
            .choice_status("choice-1", "cmd-unknown")
            .unwrap_err(),
        ChoiceReplyError::Unknown
    );
    assert_eq!(
        manager.choice_status("choice-1", "cmd-a").unwrap().state,
        ChoiceReplyState::Submitting
    );
}

#[tokio::test]
async fn workspace_choice_claim_completes_while_engine_mutex_held() {
    let manager = claim_manager("session_claim_mutex");
    let (_token, incarnation) = started_claim_run(&manager).await;

    // provider run 长持 engine 锁——claim 必须在短临界区立即完成。
    let _engine_guard = manager.engine.lock().await;
    let request = claim_request("cmd-hold", &incarnation, claim_two_answers());
    let (_status, won) = manager.claim_choice("choice-1", &request).unwrap();
    assert!(won, "claim 不得等待 engine mutex");
}

#[tokio::test]
async fn workspace_choice_claim_run_finish_expires_old_commands() {
    let manager = claim_manager("session_claim_expire");
    let (token, incarnation) = started_claim_run(&manager).await;
    let request = claim_request("cmd-old", &incarnation, claim_two_answers());
    let (_, won) = manager.claim_choice("choice-1", &request).unwrap();
    assert!(won);

    manager.finish_run(token).await;

    // 旧 expected_run_id 再 claim → Expired；同 command 状态仍可查（有界）。
    assert_eq!(
        manager.claim_choice("choice-1", &request).unwrap_err(),
        ChoiceReplyError::Expired
    );
    assert_eq!(
        manager.choice_status("choice-1", "cmd-old").unwrap().state,
        ChoiceReplyState::Expired
    );

    // 重启后的新 run：旧 run 化身仍 Expired，绝不把新 run 当旧 run。
    let (_token2, incarnation2) = started_claim_run(&manager).await;
    assert_ne!(incarnation, incarnation2, "run 化身绝不复用");
    assert_eq!(
        manager.claim_choice("choice-1", &request).unwrap_err(),
        ChoiceReplyError::Expired
    );
    let new_request = claim_request("cmd-new", &incarnation2, claim_two_answers());
    let (_, won) = manager.claim_choice("choice-1", &new_request).unwrap();
    assert!(won, "新 run 化身可重新认领同一 choice");
}

#[tokio::test]
async fn workspace_choice_claim_pending_frame_removed_only_after_delivered() {
    let manager = claim_manager("session_claim_deliver");
    let (_token, incarnation) = started_claim_run(&manager).await;
    manager.register_pending_choice_frame(provider_choice_frame("choice-d"));

    // 拿到 run 的 command 通道以扮演 provider 等待者。
    let (_id, _token, _cancel, mut command_rx, _node) = manager
        .start_run(
            crate::web::workspace_ws_handler::ProviderRunKind::ReviewOnly,
            None,
        )
        .await
        .expect("restart run for command channel");
    let incarnation = manager.active_run_incarnation().unwrap();
    manager.register_pending_choice_frame(provider_choice_frame("choice-d"));

    let request = claim_request("cmd-deliver", &incarnation, claim_two_answers());
    let (_, won) = manager.claim_choice("choice-d", &request).unwrap();
    assert!(won);
    let status = manager
        .submit_claimed_choice("choice-d", &request)
        .await
        .unwrap();
    assert_eq!(status.state, ChoiceReplyState::Resolving);

    // provider 等待者领取命令：完整 answers 原样、receipt 在场。
    let command = tokio::time::timeout(std::time::Duration::from_secs(2), command_rx.recv())
        .await
        .expect("provider command")
        .expect("command channel open");
    let ProviderCommand::ChoiceResponse {
        answers, receipt, ..
    } = command
    else {
        panic!("expected choice response command");
    };
    assert_eq!(answers, claim_two_answers());
    let receipt = receipt.expect("receipt forwarded to provider waiter");

    // 回执未 Delivered：pending 帧不删除、状态停在 Resolving。
    receipt.mark_resolving();
    let still = manager
        .wait_choice_receipt(
            "choice-d",
            "cmd-deliver",
            std::time::Duration::from_millis(50),
        )
        .await
        .unwrap();
    assert_eq!(still.state, ChoiceReplyState::Resolving);
    assert!(
        manager.pending_choice_frames().iter().any(
            |frame| matches!(frame, WsOutMessage::ChoiceRequest { id, .. } if id == "choice-d")
        ),
        "pending 帧在 Delivered 前不得删除"
    );

    // Delivered：帧收敛、状态终态可查。
    receipt.deliver();
    let delivered = manager
        .wait_choice_receipt("choice-d", "cmd-deliver", std::time::Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(delivered.state, ChoiceReplyState::Delivered);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while manager
        .pending_choice_frames()
        .iter()
        .any(|frame| matches!(frame, WsOutMessage::ChoiceRequest { id, .. } if id == "choice-d"))
    {
        assert!(
            std::time::Instant::now() < deadline,
            "看护任务必须摘除 pending 帧"
        );
        tokio::task::yield_now().await;
    }
    assert_eq!(
        manager
            .choice_status("choice-d", "cmd-deliver")
            .unwrap()
            .state,
        ChoiceReplyState::Delivered
    );
}

#[tokio::test]
async fn workspace_choice_claim_ws_default_binding_uses_current_unique_run() {
    let manager = claim_manager("session_claim_ws_default");
    let (_token, incarnation) = started_claim_run(&manager).await;

    // WS 缺省：不带 command_id/expected_run_id → 绑定当前唯一 run。
    let request = manager
        .bind_current_run_request("choice-ws", claim_two_answers())
        .expect("current run binding");
    assert_eq!(request.expected_run_id, incarnation);
    assert!(!request.command_id.is_empty());

    let (_, won) = manager.claim_choice("choice-ws", &request).unwrap();
    assert!(won);

    // 无活跃 run（run 结束）→ None，调用方保留 TextFallback 语义。
    manager.abort_active_run().await;
    assert!(
        manager
            .bind_current_run_request("choice-ws", claim_two_answers())
            .is_none()
    );
}

/// P0 1.3（真实链 2026-09-26 发现）：provider run 长期持有 engine 锁时，
/// attach/广播全部走 durable 降级投影——若降级帧不注入 run 化身，驾驶舱
/// （无 driver）拿不到 `expected_run_id`，REST 作答恒 410。契约：降级帧的
/// pending choice 同样携带当前 active run 化身；无活跃 run 保持 None。
#[tokio::test]
async fn durable_fallback_pending_choice_carries_active_run_incarnation() {
    use crate::web::workspace_ws_types::WsPendingChoiceRequest;

    let manager = claim_manager("session_durable_stamp");
    let (_token, incarnation) = started_claim_run(&manager).await;

    // 取真实 durable 降级帧（attach/current_session_state 在 run 持锁时的
    // 数据源），手工挂一条 pending（fixture 无 provider，登记簿为空）。
    let (mut frame, _) = manager.durable_projection();
    frame_session_state_pending(
        &mut frame,
        WsPendingChoiceRequest {
            id: "choice-durable".to_string(),
            prompt: "降级帧也必须携带 run 化身".to_string(),
            options: Vec::new(),
            allow_multiple: false,
            allow_free_text: false,
            questions: Vec::new(),
            source: "ask_user_question".to_string(),
            created_at_ms: None,
            role: "author".to_string(),
            expected_run_id: None,
        },
    );

    manager.stamp_pending_choice_run_ids(&mut frame);

    let WsOutMessage::SessionState {
        pending_choice_requests,
        ..
    } = &frame
    else {
        panic!("session state frame");
    };
    let stamped = pending_choice_requests
        .iter()
        .find(|request| request.id == "choice-durable")
        .expect("pending entry survives stamping");
    assert_eq!(
        stamped.expected_run_id.as_deref(),
        Some(incarnation.as_str()),
        "durable 降级帧的 pending choice 必须携带 active run 化身（驾驶舱 REST 作答绑定）"
    );
}

fn frame_session_state_pending(
    frame: &mut WsOutMessage,
    request: crate::web::workspace_ws_types::WsPendingChoiceRequest,
) {
    if let WsOutMessage::SessionState {
        pending_choice_requests,
        ..
    } = frame
    {
        pending_choice_requests.push(request);
    }
}
