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
    // A1b1（Task 2.3）：durable 投影正文每次读最新 record——出生 record 必须
    // 真实落盘；store 扫描只认 `workspace_session_*` ID，fixture 统一补前缀。
    let session_id = format!(
        "workspace_session_{}",
        session_id.strip_prefix("session_").unwrap_or(session_id)
    );
    let tmp = tempfile::tempdir().unwrap();
    let paths = crate::product::app_paths::ProductAppPaths::new(tmp.path().join(".aria"));
    FIXTURE_ROOTS.lock().unwrap().push(tmp);
    let mut record = crate::web::workspace_session::manager::test_session_record(&session_id);
    record.project_id = "project_claim".to_string();
    record.issue_id = "issue_claim".to_string();
    // A1b1：出生 record 落盘（读取失败按 typed 合同返回 ProtocolError）。
    crate::product::lifecycle_store::LifecycleStore::new(paths.clone())
        .create_workspace_session_with_id(
            crate::product::lifecycle_store::CreateWorkspaceSessionInput {
                project_id: record.project_id.clone(),
                issue_id: record.issue_id.clone(),
                entity_id: record.entity_id.clone(),
                workspace_type: record.workspace_type.clone(),
                author_provider: record.author_provider.clone(),
                reviewer_provider: record.reviewer_provider.clone(),
                review_rounds: record.review_rounds,
                superpowers_enabled: record.superpowers_enabled,
                openspec_enabled: record.openspec_enabled,
                work_item_plan_options: None,
            },
            record.id.clone(),
        )
        .expect("persist claim session record");
    let (engine_tx, _engine_rx) = mpsc::channel(1);
    let engine = crate::product::workspace_engine::WorkspaceEngine::new(
        Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
            paths.issue_lifecycle_root("project_claim", "issue_claim"),
        )),
        engine_tx,
        crate::product::workspace_engine::WorkspaceSession::from_record(record.clone()),
    );
    crate::web::workspace_session::manager::WorkspaceSessionManager::test_fixture_with_parts(
        &session_id,
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
    let (_token, _incarnation) = started_claim_run(&manager).await;
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

    // 旧 run 的 choice 应答必须在 claim 仲裁前拒（REQ-WIGA-05）：
    // 缺省绑定只认当前 run 挂起集中真实存在的 id；未登记即 None。
    assert!(
        manager
            .bind_current_run_request("choice-unknown", claim_two_answers())
            .is_none(),
        "stale/unregistered id must not bind to the current run"
    );

    // 登记当前 run 的挂起帧后：WS 缺省（不带 command_id/expected_run_id）
    // → 绑定当前唯一 run。
    manager.register_pending_choice_frame(
        crate::web::workspace_ws_types::WsOutMessage::ChoiceRequest {
            id: "choice-ws".to_string(),
            prompt: "ws default binding".to_string(),
            options: vec![],
            allow_multiple: false,
            allow_free_text: false,
            questions: vec![],
            source: "ask_user_question".to_string(),
        },
    );
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
    let mut frame = manager
        .durable_projection()
        .expect("durable fallback projection for run stamping")
        .session_state;
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

// ---- P1 WIGA Task 5：自动生成认领不得 supersede 活 run ----

fn bound_enrollment(
    enrollment_id: &str,
    plan_id: &str,
) -> crate::product::models::automation::IssueAutomationEnrollment {
    crate::product::models::automation::IssueAutomationEnrollment {
        enrollment_id: enrollment_id.to_string(),
        selection_key: "human-choice-1".to_string(),
        project_id: "project_claim".to_string(),
        issue_id: "issue_claim".to_string(),
        enabled: true,
        policy_revision: 2,
        source: crate::product::models::automation::EnrollmentSource {
            stories: Vec::new(),
            designs: Vec::new(),
        },
        options: crate::product::models::automation::EnrollmentOptions {
            author_provider: crate::product::models::ProviderName::Fake,
            reviewer_provider: crate::product::models::ProviderName::Fake,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            plan_approval: crate::product::models::automation::PlanApprovalPolicy::Manual,
            plan_options: crate::product::models::IssueWorkItemPlanOptions {
                include_integration_tests: true,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
        },
        target: Some(
            crate::product::logical_codebase::EnrollmentTarget::SingleRepository {
                repository_id: "repository_0001".to_string(),
            },
        ),
        prepare_intent_id: enrollment_id.to_string(),
        plan_id: Some(plan_id.to_string()),
        session_id: Some("session_auto_live".to_string()),
        created_at: "2026-09-27T00:00:00Z".to_string(),
        updated_at: "2026-09-27T00:00:00Z".to_string(),
        binding_history: None,
        command_ledger: Vec::new(),
        terminated_at: None,
        attention_facts: Vec::new(),
        voided_at: None,
        void_reason: None,
    }
}
async fn state_with_bound_enrollment(
    manager: &Arc<WorkspaceSessionManager>,
    _enrollment_id: &str,
    _plan_id: &str,
) -> crate::web::state::WebAppState {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().to_path_buf();
    FIXTURE_ROOTS.lock().unwrap().push(tmp);
    let state = crate::web::state::WebAppState::new(
        root.clone(),
        crate::web::runtime::WebRuntime::new_fake(root),
    );
    // 预注册 claim_manager 构造的 manager：start_plan_generation_once 经同一
    // registry 观察，不触发磁盘重建工厂。
    let preregistered = manager.clone();
    state
        .workspace_sessions
        .get_or_create("session_auto_live", || {
            let preregistered = preregistered.clone();
            async move { Ok(preregistered) }
        })
        .await
        .unwrap();
    state
}
/// REQ-WIGA-03：自动认领面对活 run 只观察（AlreadyActive），绝不调用会
/// supersede 的入口；run token/化身保持不变。
#[tokio::test]
async fn automation_generation_does_not_supersede_live_run() {
    let manager = claim_manager("session_auto_live");
    let (token, run_incarnation) = started_claim_run(&manager).await;
    let state = state_with_bound_enrollment(&manager, "enrollment_1", "plan_1").await;
    let outcome = crate::web::plan_generation::start_plan_generation_once(
        &state,
        &bound_enrollment("enrollment_1", "plan_1"),
    )
    .await
    .unwrap();
    assert!(matches!(
        outcome,
        crate::web::plan_generation::PlanGenerationOutcome::AlreadyActive
    ));
    let active = manager.active_run_ref_for_test().unwrap();
    assert_eq!(active.token, token);
    assert_eq!(active.run_incarnation, run_incarnation);
}

// ==== A1b1（Task 2.3）：最新 record 纯投影加载与错误合同 ====
// manager 存活期间 durable 恢复事实前进时，durable 投影每次以 session_id
// 读取最新 WorkspaceSessionRecord（出生 record 仅身份对照，不作正文）；
// ProjectionOnly 零恢复写；读取失败 typed 错误，绝不回退出生 record。

use crate::product::checkpoint_store::CheckpointStore;
use crate::product::lifecycle_store::CreateWorkspaceSessionInput;
use crate::product::models::{
    LcRecoveryBusinessContext, LcRecoveryRunContext, LcResumeRecoveryRecord, LcResumeRecoveryState,
    ProviderConversationRole, ProviderName, WorkspaceSessionRecord, WorkspaceSessionStatus,
};
use crate::product::workspace_engine::{WorkspaceEngine, WorkspaceSession};
use crate::web::workspace_session::manager::{
    DURABLE_PROJECTION_UNAVAILABLE_CODE, DurableProjectionError, DurableWorkspaceProjection,
    WorkspaceSessionIdentity, load_durable_workspace_projection,
};

/// 出生 record 经真实 LifecycleStore 落盘（生产 `WorkspaceSessionManager::create`
/// 同源），manager 绑定该 record。
fn anchored_durable_manager(
    project_id: &str,
) -> (
    Arc<WorkspaceSessionManager>,
    ProductAppPaths,
    WorkspaceSessionRecord,
) {
    let tmp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(tmp.path().join(".aria"));
    FIXTURE_ROOTS.lock().unwrap().push(tmp);
    let record = LifecycleStore::new(paths.clone())
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: project_id.to_string(),
            issue_id: "issue_anchor".to_string(),
            entity_id: "entity_anchor".to_string(),
            workspace_type: WorkspaceType::Story,
            author_provider: ProviderName::Fake,
            reviewer_provider: Some(ProviderName::Fake),
            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .expect("durable session record");
    let (engine_tx, _engine_rx) = mpsc::channel(1);
    let engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(
            paths.issue_lifecycle_root(&record.project_id, &record.issue_id),
        )),
        engine_tx,
        WorkspaceSession::from_record(record.clone()),
    );
    let manager = WorkspaceSessionManager::test_fixture_with_parts(
        &record.id,
        Arc::new(tokio::sync::Mutex::new(engine)),
        Arc::new(ProviderRegistry::new()),
        paths.clone(),
        record.clone(),
    );
    (manager, paths, record)
}

fn anchor_session_record_path(
    paths: &ProductAppPaths,
    record: &WorkspaceSessionRecord,
) -> std::path::PathBuf {
    paths
        .issue_lifecycle_root(&record.project_id, &record.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", record.id))
}

/// manager 存活期间 durable 恢复事实前进：最新 record 带 LC 恢复，
/// 出生 record（manager 绑定）保持无恢复。
fn overwrite_latest_record_with_recovery(
    paths: &ProductAppPaths,
    record: &WorkspaceSessionRecord,
    recovery_id: &str,
) {
    let mut latest = LifecycleStore::new(paths.clone())
        .get_workspace_session(&record.id)
        .expect("latest record");
    latest.lc_resume_recovery = Some(anchor_t04_recovery_record(recovery_id));
    std::fs::write(
        anchor_session_record_path(paths, record),
        serde_json::to_string(&latest).expect("serialize latest record"),
    )
    .expect("overwrite latest record");
}

fn anchor_t04_recovery_record(recovery_id: &str) -> LcResumeRecoveryRecord {
    LcResumeRecoveryRecord {
        schema_version: 1,
        recovery_id: recovery_id.to_string(),
        failed_provider_node_id: "node_anchor_0001".to_string(),
        reason_code: "resume_fingerprint_mismatch".to_string(),
        role: ProviderConversationRole::Author,
        provider: ProviderName::Codex,
        superseded_native_session_id: Some("native_anchor_0001".to_string()),
        previous_fingerprint_digest: Some("sha256:anchor_full_projection".to_string()),
        previous_session_status: WorkspaceSessionStatus::Running,
        previous_stage: "running".to_string(),
        run_context: LcRecoveryRunContext::Author {
            business: LcRecoveryBusinessContext {
                entity_id: "entity_anchor".to_string(),
                plan_id: None,
                workspace_type: WorkspaceType::Story,
                business_repository_physical_id: None,
                business_repository_logical_id: None,
                business_checkout_id: None,
                business_repository_path: None,
                source_story_ids: Vec::new(),
                source_design_ids: Vec::new(),
                plan_options_ref: None,
                artifact_refs: Vec::new(),
                review_refs: Vec::new(),
                author_provider: ProviderName::Codex,
                effective_reviewer_provider: None,
                original_node_id: Some("node_anchor_0001".to_string()),
                original_role_run_seq: Some(1),
            },
            original_content: Some("原 author prompt 内容".to_string()),
        },
        state: LcResumeRecoveryState::Waiting,
        operation_id: None,
        fresh_node_id: None,
        fresh_native_session_id: None,
        fresh_role_run_seq: None,
        fresh_operations: Vec::new(),
        detected_at: "2026-10-11T00:02:00Z".to_string(),
        updated_at: "2026-10-11T00:02:00Z".to_string(),
    }
}

/// durable 全量快照（projects 子树：session/timeline/turn 逐文件字节）。
fn durable_tree_snapshot(paths: &ProductAppPaths) -> Vec<(String, Vec<u8>)> {
    fn walk(dir: &std::path::Path, prefix: String, out: &mut Vec<(String, Vec<u8>)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let name = format!("{prefix}/{}", entry.file_name().to_string_lossy());
            if path.is_dir() {
                walk(&path, name, out);
            } else {
                out.push((name, std::fs::read(&path).expect("read durable file")));
            }
        }
    }
    let mut snapshot = Vec::new();
    walk(&paths.projects_root(), String::new(), &mut snapshot);
    snapshot
}

#[tokio::test]
async fn anchor_t04_latest_record_pure_projection_while_engine_locked() {
    let (manager, paths, birth) = anchored_durable_manager("project_anchor_latest");

    // 最新 durable record 带 LC 恢复；出生 record 无恢复——出生 record
    // 不得再作投影正文。
    overwrite_latest_record_with_recovery(&paths, &birth, "latest-rec");

    // 纯投影前后 session/timeline/turn 零写的对照快照。
    let durable_before = durable_tree_snapshot(&paths);

    // engine 锁被长持（provider run 占用）——纯投影不得依赖 engine 锁。
    let _engine_guard = manager.lock_engine_for_test().await;

    // private 纯加载核心：最新 record 为正文，身份与 manager 绑定一致。
    let identity = WorkspaceSessionIdentity::from_bound_record(&birth);
    let projection: DurableWorkspaceProjection =
        load_durable_workspace_projection(&paths, &identity)
            .expect("pure projection of latest record");
    assert_eq!(projection.recovery_id(), Some("latest-rec"));

    // manager Result 出口（current_session_state 降级路径）同源最新 record。
    let frame = manager
        .durable_projection()
        .expect("manager durable projection of latest record")
        .session_state;
    let WsOutMessage::SessionState {
        lc_resume_recovery, ..
    } = &frame
    else {
        panic!("expected session state frame, got {frame:?}");
    };
    assert_eq!(
        lc_resume_recovery
            .as_ref()
            .map(|dto| dto.recovery_id.as_str()),
        Some("latest-rec"),
        "Result 出口必须投影最新 record 的恢复事实"
    );

    // ProjectionOnly：前后 session/timeline/turn 零写。
    let durable_after = durable_tree_snapshot(&paths);
    assert_eq!(
        durable_before, durable_after,
        "纯投影前后 session/timeline/turn 必须零写"
    );
}

#[tokio::test]
async fn anchor_t04_projection_read_failure_never_uses_birth_record() {
    let (manager, paths, birth) = anchored_durable_manager("project_anchor_unreadable");
    let identity = WorkspaceSessionIdentity::from_bound_record(&birth);
    let record_path = anchor_session_record_path(&paths, &birth);

    // 1) 最新 record 身份漂移（entity 不再匹配 manager 绑定）→
    //    IdentityMismatch，不回出生 record。
    let mut drifted = LifecycleStore::new(paths.clone())
        .get_workspace_session(&birth.id)
        .expect("latest record before drift");
    drifted.entity_id = "entity_drifted".to_string();
    std::fs::write(
        &record_path,
        serde_json::to_string(&drifted).expect("serialize drifted record"),
    )
    .expect("write drifted record");
    assert!(matches!(
        load_durable_workspace_projection(&paths, &identity),
        Err(DurableProjectionError::IdentityMismatch { .. })
    ));

    // 2) record 文件损坏（JSON）→ typed Store 错误；manager Result 出口
    //    typed 上抛（不出生 record/default 假态），wire 面 ProtocolError
    //    转帧由 anchor_t05 广播/attach 断言。code 取稳定常量（单点锚定）。
    assert_eq!(
        DURABLE_PROJECTION_UNAVAILABLE_CODE,
        "workspace_durable_projection_unavailable"
    );
    std::fs::write(&record_path, "{ not json").unwrap();
    let unreadable = load_durable_workspace_projection(&paths, &identity);
    assert!(matches!(unreadable, Err(DurableProjectionError::Store(_))));
    assert!(matches!(
        manager.durable_projection(),
        Err(DurableProjectionError::Store(_))
    ));

    // 3) record 缺失（NotFound）→ 同样 typed Store，无出生 record 假态。
    std::fs::remove_file(&record_path).unwrap();
    assert!(matches!(
        load_durable_workspace_projection(&paths, &identity),
        Err(DurableProjectionError::Store(_))
    ));
    assert!(matches!(
        manager.durable_projection(),
        Err(DurableProjectionError::Store(_))
    ));
}

// ---------------------------------------------------------------------------
// A1b2（Task 2.4，设计 §4.4）：router/attach/socket 全 caller 错误闭包。
// ---------------------------------------------------------------------------

/// 有界等待一帧出站文本并解析 JSON（超时即视为广播未接线）。
async fn anchor_t05_recv_frame(rx: &mut mpsc::Receiver<OutboundControl>) -> serde_json::Value {
    let control = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .expect("frame must arrive within 5s")
        .expect("outbound channel stays open");
    let OutboundControl::Text(json) = control else {
        panic!("router must deliver text frames, got {control:?}");
    };
    serde_json::from_str(&json).expect("wire frame json")
}

/// 断言帧为 session_state 并取出 lc_resume_recovery 投影。
fn anchor_t05_recovery_of(frame: &serde_json::Value) -> &serde_json::Value {
    assert_eq!(
        frame["type"], "session_state",
        "expect authoritative session_state frame: {frame}"
    );
    frame
        .get("lc_resume_recovery")
        .unwrap_or_else(|| panic!("session_state must carry lc_resume_recovery: {frame}"))
}

/// A1b2 验收（设计 §4.4）：同一存活 manager 持锁时 durable 恢复前进——
/// RecoveryChanged 立即广播（不等 engine 锁，忙走最新 record 纯投影），
/// driver/observer/新 attach 看到相同 recovery_id/reason/allowed_actions；
/// 广播帧沿 journal event_seq。
#[tokio::test]
async fn anchor_t05_live_manager_locked_broadcast_attach_recovery() {
    let (manager, paths, birth) = anchored_durable_manager("project_anchor_t05_live");
    // 最新 durable record 带 LC 恢复（出生 record 无恢复）。
    overwrite_latest_record_with_recovery(&paths, &birth, "latest-rec");

    // driver/observer：同一存活 manager 的两个已激活连接。
    let (driver_tx, mut driver_rx) = mpsc::channel(16);
    let (observer_tx, mut observer_rx) = mpsc::channel(16);
    manager.register_attachment("conn_t05_driver", driver_tx);
    manager.register_attachment("conn_t05_observer", observer_tx);
    manager.activate_attachment("conn_t05_driver");
    manager.activate_attachment("conn_t05_observer");

    // engine 锁长持（provider run 占用）——RecoveryChanged 广播不得等锁。
    let _engine_guard = manager.lock_engine_for_test().await;

    // router 真实广播接线（A2.2 移交）：事件经 event router 触发 authoritative 帧。
    let (router_tx, router_rx) = mpsc::channel::<EngineEvent>(8);
    manager.spawn_event_router(
        router_rx,
        crate::web::state::WorkspaceRunRegistry::default(),
    );
    router_tx
        .send(EngineEvent::LcResumeRecoveryChanged)
        .await
        .expect("feed RecoveryChanged into router");

    let driver = anchor_t05_recv_frame(&mut driver_rx).await;
    let observer = anchor_t05_recv_frame(&mut observer_rx).await;
    let driver_recovery = anchor_t05_recovery_of(&driver);
    let observer_recovery = anchor_t05_recovery_of(&observer);

    assert_eq!(driver_recovery["recovery_id"], "latest-rec");
    assert_eq!(
        driver_recovery["reason_code"],
        "resume_fingerprint_mismatch"
    );
    assert_eq!(
        driver_recovery["allowed_actions"].as_array().map(Vec::len),
        Some(0),
        "A 波未接齐 B0：allowed_actions 恒空"
    );
    // driver/observer 同 recovery 投影；event_seq 沿 journal 同序。
    assert_eq!(
        driver_recovery["recovery_id"],
        observer_recovery["recovery_id"]
    );
    assert_eq!(
        driver_recovery["reason_code"],
        observer_recovery["reason_code"]
    );
    assert_eq!(
        driver_recovery["allowed_actions"],
        observer_recovery["allowed_actions"]
    );
    let event_seq = driver["event_seq"].as_u64().expect("event_seq stamped");
    assert!(event_seq > 0);
    assert_eq!(observer["event_seq"].as_u64(), Some(event_seq));

    let journal_entries: Vec<(u64, String)> = {
        let state = manager
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.journal.entries.iter().cloned().collect()
    };
    assert!(
        journal_entries
            .iter()
            .any(|(seq, json)| *seq == event_seq && json.contains("latest-rec")),
        "RecoveryChanged 广播帧必须沿 journal event_seq：{journal_entries:?}"
    );

    // 新 attach（engine 锁仍持有 → durable fallback）同 recovery/原因/动作。
    let (attach_frame, _) = manager
        .attached_session_state()
        .await
        .expect("attached durable fallback projects latest record");
    let WsOutMessage::SessionState {
        lc_resume_recovery, ..
    } = &attach_frame
    else {
        panic!("attach fallback must be session_state, got {attach_frame:?}");
    };
    let attach_recovery = lc_resume_recovery
        .as_ref()
        .expect("attach fallback carries latest recovery");
    assert_eq!(attach_recovery.recovery_id, "latest-rec");
    assert_eq!(attach_recovery.reason_code, "resume_fingerprint_mismatch");
    assert!(attach_recovery.allowed_actions.is_empty());
}

/// A1b2 验收（设计 §4.4）：durable 纯读失败——broadcast/attach/resubscribe
/// 全 caller typed 错误闭包，稳定 code
/// `workspace_durable_projection_unavailable`；零伪 SessionState（无旧可
/// 启动 baseline/假 snapshot）。
#[tokio::test]
async fn anchor_t05_snapshot_failure_returns_typed_error_without_baseline() {
    let (manager, paths, birth) = anchored_durable_manager("project_anchor_t05_unavailable");
    std::fs::write(anchor_session_record_path(&paths, &birth), "{ not json")
        .expect("corrupt latest record");
    let _engine_guard = manager.lock_engine_for_test().await;

    // 广播面：broadcast_current_session_state 广播 typed ProtocolError。
    let (conn_tx, mut conn_rx) = mpsc::channel(16);
    manager.register_attachment("conn_t05_fail", conn_tx);
    manager.activate_attachment("conn_t05_fail");
    manager.broadcast_current_session_state();
    let error_frame = anchor_t05_recv_frame(&mut conn_rx).await;
    assert_eq!(error_frame["type"], "protocol_error");
    assert_eq!(error_frame["code"], DURABLE_PROJECTION_UNAVAILABLE_CODE);
    assert!(
        error_frame["event_seq"].as_u64().is_some_and(|seq| seq > 0),
        "typed 错误帧沿 journal event_seq：{error_frame}"
    );

    // attach 面：typed Err 上抛，不产伪 snapshot。
    assert!(
        matches!(
            manager.attached_session_state().await,
            Err(DurableProjectionError::Store(_))
        ),
        "attached fallback 读取失败必须 typed 上抛"
    );

    // 重订阅面（cursor 越界走 snapshot 分支）：发 typed ProtocolError，
    // 不激活伪 snapshot。
    let (resub_tx, mut resub_rx) = mpsc::channel(16);
    manager.register_attachment("conn_t05_resub", resub_tx.clone());
    manager
        .resubscribe(&resub_tx, "conn_t05_resub", 9_999)
        .await;
    let mut session_state_frames = 0usize;
    let mut typed_error_frames = 0usize;
    while let Ok(control) = resub_rx.try_recv() {
        let OutboundControl::Text(json) = control else {
            panic!("resubscribe must deliver text frames only");
        };
        let frame: serde_json::Value = serde_json::from_str(&json).expect("frame json");
        match frame["type"].as_str() {
            Some("protocol_error") => {
                assert_eq!(frame["code"], DURABLE_PROJECTION_UNAVAILABLE_CODE);
                typed_error_frames += 1;
            }
            Some("session_state") => session_state_frames += 1,
            other => panic!("unexpected resubscribe frame type: {other:?}"),
        }
    }
    assert_eq!(
        typed_error_frames, 1,
        "重订阅读失败发一帧 typed ProtocolError"
    );
    assert_eq!(session_state_frames, 0, "published_fake_snapshots 必须为 0");

    // journal 无任何伪可启动 SessionState 基线。
    let journal_entries: Vec<(u64, String)> = {
        let state = manager
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.journal.entries.iter().cloned().collect()
    };
    assert!(
        journal_entries
            .iter()
            .all(|(_, json)| !json.contains("\"session_state\"")),
        "读取失败不得向 journal 发布伪 SessionState：{journal_entries:?}"
    );
}

// ---------------------------------------------------------------------------
// SG7a（plan v1.1 Task SG7a Step 1）：manager 纯投影幂等/零写与结构
// sync 不误覆盖 runtime 锚点。
// ---------------------------------------------------------------------------

/// SG7a：结构聚合状态经 durable record 注入（SG5.3/SG6a.1 同款
/// read-modify-write fixture；grant/checkpoint producer 归 SG6a.3）。
#[tokio::test]
async fn sg_07a_projection_is_pure_and_preserves_anchor() {
    use crate::product::json_store::write_json;
    use crate::product::models::StructuralLaunchEvidence;
    use crate::product::models::StructuralLaunchState;
    use crate::product::models::{
        ResolvedStructureFact, StructuralCheckpoint, StructuralCheckpointStatus,
        StructuralGovernanceState, StructuralGrantPhase, StructuralRevisionGrant, UserBudgetHold,
        UserBudgetReceipt, UserBudgetReceiptKind,
    };
    use crate::product::work_item_plan_policy::ProviderStartLedgerEntry;
    use crate::web::workspace_ws_types::project_structural_governance_snapshot;

    let (manager, paths, birth) = anchored_durable_manager("project_anchor_sg07a");

    // durable 前进：结构聚合状态（grant attempt1 + 活动 hold + 历史
    // receipt/checkpoint/resolved facts + actual start 观察）——出生
    // record 保持无聚合状态（旧记录）。
    let mut latest = LifecycleStore::new(paths.clone())
        .get_workspace_session(&birth.id)
        .expect("latest record");
    latest.structural_governance_state = Some(StructuralGovernanceState {
        current_grant: Some(StructuralRevisionGrant {
            grant_id: "structural_grant_turn_sg07a_anchor".to_string(),
            authorization_hash: "sha256:sg07a-authorization".to_string(),
            turn_id: Some("turn_sg07a_anchor".to_string()),
            command_id: Some("cmd_sg07a_anchor".to_string()),
            candidate_source_revision_ref: Some("sg07a-source-ref".to_string()),
            plan_candidate_ir_ref: Some("sg07a-ir-ref".to_string()),
            mechanical_report_ref: Some("sg07a-report-ref".to_string()),
            phase: Some(StructuralGrantPhase::Attempt2Claimed),
            attempt_no: 2,
            prompt_ref: Some("sg07a-frozen-prompt-ref".to_string()),
            prompt_digest: Some("sha256:sg07a-prompt".to_string()),
            instruction_digest: Some("sha256:sg07a-instruction".to_string()),
            failing_source_hash: Some("sha256:sg07a-failing-source".to_string()),
            review_identity: None,
            granted_at: "2026-10-14T00:00:00Z".to_string(),
        }),
        user_budget_total: 3,
        user_holds: vec![UserBudgetHold {
            turn_id: "turn_sg07a_anchor".to_string(),
            command_id: "cmd_sg07a_anchor".to_string(),
            source_hash: "sha256:sg07a-failing-source".to_string(),
            held_at: "2026-10-14T00:00:00Z".to_string(),
        }],
        user_receipts: vec![UserBudgetReceipt {
            kind: UserBudgetReceiptKind::ReleasedStructural,
            turn_id: "turn_sg07a_prev".to_string(),
            command_id: "cmd_sg07a_prev".to_string(),
            attempt_reason: None,
            source_revision_ref: None,
            plan_candidate_ir_ref: None,
            structure_report_ref: None,
            context_digest: None,
            reason: Some("structural exhausted before delivery".to_string()),
            settled_at: "2026-10-14T00:00:00Z".to_string(),
        }],
        structure_checkpoints: vec![StructuralCheckpoint {
            checkpoint_id: "sg07a-anchor-checkpoint".to_string(),
            source_revision_ref: Some("sg07a-source-ref".to_string()),
            plan_candidate_ir_ref: Some("sg07a-ir-ref".to_string()),
            structure_report_ref: Some("sg07a-report-ref".to_string()),
            artifact_version: Some(1),
            source_hash: Some("sha256:sg07a-failing-source".to_string()),
            context_digest: Some("sha256:sg07a-context".to_string()),
            authorization_hash: Some("sha256:sg07a-authorization".to_string()),
            receipt_ref: None,
            review_scope: Some("sg07a-review-scope".to_string()),
            status: StructuralCheckpointStatus::Active,
            recorded_at: "2026-10-14T00:00:00Z".to_string(),
        }],
        resolved_structure_facts: vec![ResolvedStructureFact {
            predicate: "dependency_cycle_resolved".to_string(),
            evidence_refs: vec!["sg07a-report-ref".to_string()],
            source_hash: None,
            resolved_at: "2026-10-14T00:00:00Z".to_string(),
        }],
        current_structure_report_ref: Some("sg07a-report-ref".to_string()),
        current_context_digest: Some("sha256:sg07a-context".to_string()),
        current_turn_summary: Some("attempt2 机器修复窗口".to_string()),
        launch_reason: Some("structural repair attempt".to_string()),
        launch_evidence: Some(StructuralLaunchEvidence {
            launch_state: StructuralLaunchState::Started,
            launch_observed_at: Some("2026-10-14T00:00:00Z".to_string()),
            launch_evidence_ref: Some("sg07a-launch-evidence".to_string()),
        }),
    });
    latest.work_item_plan_source_revision_ref = Some("sg07a-source-ref".to_string());
    latest.plan_candidate_ir_ref = Some("sg07a-ir-ref".to_string());
    latest.mechanical_report_ref = Some("sg07a-report-ref".to_string());
    latest.provider_start_ledger.push(ProviderStartLedgerEntry {
        provider_start_idempotency_key: "sg07a:start:anchor-actual".to_string(),
        started: true,
        provider: Some("fake".to_string()),
        started_at: Some("2026-10-14T00:00:00Z".to_string()),
        provider_attempt_key: None,
    });
    write_json(&anchor_session_record_path(&paths, &birth), &latest)
        .expect("persist structural governance record");
    let ledger_len = latest.provider_start_ledger.len();
    let diagnostics_len = latest.policy_diagnostics.len();
    // engine 锁长持（attach 期间的极端形态）——manager Result 出口纯投影
    // 不得依赖 engine 锁，且前后 durable 字节零写。
    let mut engine_guard = manager.lock_engine_for_test().await;
    let bytes_before = durable_tree_snapshot(&paths);
    let frame = manager
        .durable_projection()
        .expect("manager durable projection with structural governance")
        .session_state;
    let bytes_after = durable_tree_snapshot(&paths);
    assert_eq!(bytes_before, bytes_after, "纯投影必须零写 durable");
    let snapshot = match &frame {
        WsOutMessage::SessionState {
            structural_governance: Some(snapshot),
            ..
        } => snapshot,
        WsOutMessage::SessionState {
            structural_governance: None,
            ..
        } => panic!("结构聚合状态必须投影到 SessionState"),
        other => panic!("expected SessionState, got {other:?}"),
    };
    let state = latest.structural_governance_state.as_ref().unwrap();
    // 与 SG7 纯函数逐字段同值（同一单源输入）。
    let expected = project_structural_governance_snapshot(
        state,
        latest.human_gate_snapshot.as_ref(),
        &latest.provider_start_ledger,
        latest.work_item_plan_source_revision_ref.as_deref(),
        latest.plan_candidate_ir_ref.as_deref(),
        latest.mechanical_report_ref.as_deref(),
    );
    assert_eq!(snapshot, &expected, "manager 投影必须与纯函数同值");
    assert_eq!(
        snapshot.available_user_budget, 2,
        "3 − 0 计费(released 不实扣) − 1 hold"
    );
    assert_eq!(snapshot.actual_provider_starts, 1);
    assert_eq!(snapshot.structure_checkpoints.len(), 1);

    // 幂等：同输入重复投影的结构快照逐字段同值（帧级时间戳字段如
    // 派生 timeline 节点 started_at 不在结构快照域）；claim/start/预算
    // 事件计数不变。
    let again = manager
        .durable_projection()
        .expect("repeatable pure projection")
        .session_state;
    let again_snapshot = match &again {
        WsOutMessage::SessionState {
            structural_governance: Some(snapshot),
            ..
        } => snapshot.clone(),
        other => panic!("expected structural snapshot, got {other:?}"),
    };
    assert_eq!(snapshot, &again_snapshot, "结构纯投影幂等");
    let reloaded_latest = LifecycleStore::new(paths.clone())
        .get_workspace_session(&birth.id)
        .expect("latest unchanged");
    assert_eq!(reloaded_latest.provider_start_ledger.len(), ledger_len);
    assert_eq!(reloaded_latest.policy_diagnostics.len(), diagnostics_len);

    // 结构 sync 不误覆盖 runtime 锚点：engine 内存 session 先注入非默认
    // 锚点（repository_path/lc_resume_recovery/fullmap/tombstone/stage），
    // 再以同 scope record sync——结构镜像更新、锚点原样。
    {
        let engine = &mut *engine_guard;
        engine.session.repository_path = Some(std::path::PathBuf::from("/tmp/sg07a-anchor-repo"));
        engine.session.lc_resume_recovery =
            Some(anchor_t04_recovery_record("recovery_sg07a_anchor"));
        engine.session.logical_launch_fingerprints =
            vec![crate::product::models::LogicalLaunchFingerprintRecord {
                schema_version: 1,
                basis: crate::product::models::LogicalLaunchBasis::FullLcProjectionV2,
                role: ProviderConversationRole::Author,
                provider: ProviderName::Fake,
                role_run_seq: 7,
                node_id: "node_sg07a_anchor".to_string(),
                native_session_id: None,
                fingerprint_digest: "sha256:sg07a-fullmap".to_string(),
                planning_access_fingerprint: "sha256:sg07a-access".to_string(),
                state: crate::product::models::LogicalLaunchFingerprintState::Prepared,
                created_at: "2026-10-14T00:00:00Z".to_string(),
                updated_at: "2026-10-14T00:00:00Z".to_string(),
            }];
        engine.session.superseded_native_sessions =
            vec![crate::product::models::SupersededNativeSessionRecord {
                role: ProviderConversationRole::Author,
                provider: ProviderName::Fake,
                native_session_id: "native_sg07a_old".to_string(),
                reason_code: "resume_fingerprint_mismatch".to_string(),
                recovery_id: "recovery_sg07a_anchor".to_string(),
                superseded_at: "2026-10-14T00:00:00Z".to_string(),
            }];
        engine.session.stage = crate::product::workspace_engine::WorkspaceStage::HumanConfirm;

        engine
            .session
            .sync_structural_governance_from_record(&latest)
            .expect("same-scope structural sync");
        assert_eq!(
            engine.session.structural_governance_state, latest.structural_governance_state,
            "结构镜像必须吸收最新 record 聚合状态"
        );
        assert_eq!(
            engine.session.repository_path,
            Some(std::path::PathBuf::from("/tmp/sg07a-anchor-repo")),
            "结构 sync 不得覆盖 repository_path"
        );
        assert_eq!(
            engine
                .session
                .lc_resume_recovery
                .as_ref()
                .map(|recovery| recovery.recovery_id.as_str()),
            Some("recovery_sg07a_anchor"),
            "结构 sync 不得覆盖 lc_resume_recovery"
        );
        assert_eq!(
            engine.session.logical_launch_fingerprints.len(),
            1,
            "结构 sync 不得覆盖 fullmap 指纹"
        );
        assert_eq!(
            engine.session.superseded_native_sessions.len(),
            1,
            "结构 sync 不得覆盖 tombstone"
        );
        assert_eq!(
            engine.session.stage,
            crate::product::workspace_engine::WorkspaceStage::HumanConfirm,
            "结构 sync 不得覆盖 stage"
        );
        assert_eq!(
            engine.session.human_gate_snapshot, latest.human_gate_snapshot,
            "同 record 原 gate 快照随 sync 对齐"
        );
        assert_eq!(
            engine.session.provider_start_ledger.len(),
            ledger_len,
            "ledger 沿 record 原样复制，不重算"
        );

        // reload_session_from_record：结构镜像随 durable 对齐且保留
        // repository_path 模式（C-1 集成修复语义不回归）。
        engine.reload_session_from_record(latest.clone());
        assert_eq!(
            engine.session.structural_governance_state, latest.structural_governance_state,
            "reload 后结构镜像必须对齐"
        );
        assert_eq!(
            engine.session.repository_path,
            Some(std::path::PathBuf::from("/tmp/sg07a-anchor-repo")),
            "reload 必须保留 repository_path"
        );
    }
    drop(engine_guard);

    // 出生旧 record（无聚合状态）不产生快照（无新许可）——出生 record
    // 的 from_record 镜像保持 None；anchor_t04/t05 既有负例不受影响。
    assert_eq!(birth.structural_governance_state, None);
    let birth_runtime = WorkspaceSession::from_record(birth.clone());
    assert_eq!(birth_runtime.structural_governance_state, None);
}

/// T22 r3(b45 现场 r2/r3 根因⑤):SC 修订 run 全程持有 engine 锁,run 内
/// 发出的 `HumanGateOpened` 曾令 router 以 `engine.lock().await` 阻塞等待
/// ——router 停止消费后 engine 事件通道(容量 64)被后续流帧填满,run 的
/// send 也阻塞,构成循环等待:WS 零帧 90min(r3 帧日志实证:reviewer 已
/// `StatusChanged(Completed)`+结构化产出,而连接零出帧含 pong)。钉测:
/// engine 锁被持(run 形态)时,HumanGateOpened 之后的后续事件仍必须被
/// router 持续消费——通道不楔死、发送方不悬停。
#[tokio::test]
async fn human_gate_opened_routing_survives_engine_mutex_held_by_run() {
    let manager = claim_manager("session_gate_open_lock");
    let (engine_tx, engine_rx) =
        mpsc::channel::<crate::product::workspace_engine::EngineEvent>(4);
    manager.spawn_event_router(
        engine_rx,
        crate::web::state::WorkspaceRunRegistry::default(),
    );

    // 模拟 provider run:任务全期持有 engine 锁(provider_run.rs run 任务形态)。
    let _engine_guard = manager.engine.lock().await;

    manager_set_gate_frame_probe(&manager).await;
    engine_tx
        .send(crate::product::workspace_engine::EngineEvent::HumanGateOpened {
            stage: "human_confirm".to_string(),
        })
        .await
        .expect("send gate opened");

    // 超越通道容量继续发送:router 若阻塞在 HumanGateOpened 臂,通道填满
    // 后这些 send 将楔死(每发限时 2s,阻塞形态=超时断言失败)。
    for index in 0..64u32 {
        let sent = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            engine_tx.send(crate::product::workspace_engine::EngineEvent::StreamChunk {
                role: "reviewer".to_string(),
                content: format!("chunk-{index}"),
                node_id: None,
            }),
        )
        .await;
        assert!(
            sent.is_ok(),
            "router 必须在 engine 锁被持时持续消费(第 {index} 个流帧楔死=死锁形态)"
        );
    }
}

/// 广播目的地探针:挂一个 attachment 出站通道,断言帧可投递(锁持形态下
/// 经 durable 投影广播;不要求具体帧型,只保证 router 存活)。
async fn manager_set_gate_frame_probe(manager: &Arc<WorkspaceSessionManager>) {
    let (outbound_tx, mut outbound_rx) =
        mpsc::channel::<crate::web::workspace_ws_handler::OutboundControl>(64);
    manager.attach("probe-connection", outbound_tx).await;
    // 消费侧保持活到测试尾,防广播 try_send 满仓退化为 resync。
    tokio::spawn(async move {
        while outbound_rx.recv().await.is_some() {}
    });
}
