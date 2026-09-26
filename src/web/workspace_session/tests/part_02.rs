// ---------------------------------------------------------------------------
// F-24（choice 卡不送达用户）：挂起 provider choice 的可靠重投面
// ---------------------------------------------------------------------------

fn provider_choice_frame(id: &str) -> crate::web::workspace_ws_types::WsOutMessage {
    use crate::web::workspace_ws_types::{ChoiceOption, WsOutMessage};

    WsOutMessage::ChoiceRequest {
        id: id.to_string(),
        prompt: "验收口径歧义需要用户裁定".to_string(),
        options: vec![ChoiceOption {
            id: "option-a".to_string(),
            label: "按全局口径".to_string(),
            description: None,
        }],
        allow_multiple: false,
        allow_free_text: false,
        questions: Vec::new(),
        source: "provider_choice".to_string(),
    }
}

fn frames_contain_choice(frames: &[String], id: &str) -> bool {
    frames.iter().any(|json| {
        let value: serde_json::Value =
            serde_json::from_str(json).unwrap_or(serde_json::Value::Null);
        value["type"] == "choice_request" && value["id"] == id
    })
}

/// F-24 现场锚（0484）：满队列 attachment 降级期间广播的 choice 帧被 try_send 丢弃，
/// 恢复只补 session_state 基线（不含 choice）→ 用户全程在线也永远看不到卡。
/// 修复后：degraded 恢复必须在基线之外补发挂起 choice 帧。
#[tokio::test]
async fn degraded_attachment_recovery_redelivers_pending_provider_choice() {
    use crate::web::workspace_ws_types::WsProviderStatus;

    let manager = WorkspaceSessionManager::test_fixture("session_choice_degraded_redelivery");
    let (slow_tx, mut slow_rx) = mpsc::channel(1);
    manager.attach("slow", slow_tx).await;
    manager
        .broadcast_test_event(WsProviderStatus::Starting)
        .await;
    manager
        .broadcast_test_event(WsProviderStatus::Running)
        .await;
    assert!(
        manager.attachment_is_degraded("slow"),
        "满队列 attachment 必须被标为 degraded"
    );

    manager
        .start_run(ProviderRunKind::ReviewOnly, None)
        .await
        .expect("active run for pending choice");
    manager.register_pending_choice_frame(provider_choice_frame("choice_degraded_1"));

    let _first = slow_rx.recv().await.expect("first queued event");
    manager
        .broadcast_test_event(WsProviderStatus::Completed)
        .await;
    assert!(!manager.attachment_is_degraded("slow"));

    let mut saw_baseline = false;
    let mut saw_choice = false;
    for _ in 0..4 {
        match tokio::time::timeout(std::time::Duration::from_millis(500), slow_rx.recv()).await {
            Ok(Some(OutboundControl::Text(json))) => {
                let value: serde_json::Value =
                    serde_json::from_str(&json).expect("recovery frame JSON");
                match value["type"].as_str() {
                    Some("session_state") => saw_baseline = true,
                    Some("choice_request") if value["id"] == "choice_degraded_1" => {
                        saw_choice = true;
                    }
                    _ => {}
                }
            }
            _ => break,
        }
    }
    assert!(saw_baseline, "degraded 恢复必须先发 session_state 基线");
    assert!(
        saw_choice,
        "degraded 恢复必须补发挂起 choice 卡（F-24：卡片丢失即 run 永久楔死）"
    );
}

/// F-24：初帧激活与 cursor 重订阅（snapshot 分支）都必须补发活跃 run 的挂起
/// choice——`pending_author_choice_request_message` 只覆盖 TextFallback 面。
#[tokio::test]
async fn attach_and_cursor_resubscribe_redeliver_pending_provider_choices() {
    let manager = WorkspaceSessionManager::test_fixture("session_choice_attach_redelivery");
    manager
        .start_run(ProviderRunKind::ReviewOnly, None)
        .await
        .expect("active run");
    manager.register_pending_choice_frame(provider_choice_frame("choice_attach_1"));

    let (session_state, _text_fallback) = manager.attached_session_state().await;
    let frames = manager.activate_attachment_with_initial_frames(
        "conn-attach",
        session_state,
        None,
        manager.attach_baseline_seq(),
    );
    assert!(
        frames_contain_choice(&frames, "choice_attach_1"),
        "初帧激活必须补发挂起 provider choice：{frames:?}"
    );

    let (cursor_tx, mut cursor_rx) = mpsc::channel(16);
    manager.register_attachment("conn-cursor", cursor_tx.clone());
    manager.resubscribe(&cursor_tx, "conn-cursor", 9_999).await;
    let mut resubscribed = Vec::new();
    while let Ok(control) = cursor_rx.try_recv() {
        let OutboundControl::Text(json) = control else {
            panic!("resubscribe must only send text frames");
        };
        resubscribed.push(json);
    }
    assert!(
        frames_contain_choice(&resubscribed, "choice_attach_1"),
        "cursor snapshot 重订阅必须补发挂起 provider choice：{resubscribed:?}"
    );
}

// ---------------------------------------------------------------------------
// P0 1.2（REQ-WIGA-08）：durable automation 归属投影——manager/summary/durable
// 三面同源，未绑定/缺 enrollment 恒 client，读取失败不伪 client。
// ---------------------------------------------------------------------------

fn automation_test_record(
    session_id: &str,
    entity_id: &str,
) -> crate::product::models::WorkspaceSessionRecord {
    use crate::product::models::WorkspaceRolePermissionModes;
    use crate::product::models::{
        ProviderName, WorkspaceSessionRecord, WorkspaceSessionStatus, WorkspaceType,
    };
    use crate::product::work_item_plan_policy::{RunHistory, RunPolicy, WorkItemPlanFlowKind};

    WorkspaceSessionRecord {
        id: session_id.to_string(),
        project_id: "project_auto".to_string(),
        issue_id: "issue_auto".to_string(),
        entity_id: entity_id.to_string(),
        // Story/Legacy 与 manager::test_session_record 同构：归属投影与
        // workspace_type 正交，ephemeral engine 无需 lifecycle_store。
        workspace_type: WorkspaceType::Story,
        status: WorkspaceSessionStatus::Open,
        author_provider: ProviderName::Fake,
        reviewer_provider: ProviderName::Fake,
        review_rounds: 1,
        permission_modes: WorkspaceRolePermissionModes::default(),
        provisional_reviewer_provider: None,
        reviewer_enabled_at_start: None,
        superpowers_enabled: false,
        openspec_enabled: false,
        flow_kind: WorkItemPlanFlowKind::Legacy,
        run_policy: RunPolicy::Interactive,
        run_history: RunHistory::default(),
        review_invocation_scope: None,
        human_gate_snapshot: None,
        repair_reservation: None,
        human_gate_reservation: None,
        policy_diagnostics: Vec::new(),
        provider_start_ledger: Vec::new(),
        single_candidate_phase: None,
        work_item_plan_source_revision_ref: None,
        plan_candidate_ir_ref: None,
        mechanical_report_ref: None,
        publication_provenance_ref: None,
        approval_attempt_id: None,
        approved_at: None,
        compile_reservation: None,
        work_item_runtime_binding: None,
        provider_conversations: Vec::new(),
        messages: Vec::new(),
        created_at: "2026-09-26T00:00:00Z".to_string(),
        updated_at: "2026-09-26T00:00:00Z".to_string(),
    }
}

fn automation_enable_command() -> crate::product::models::automation::EnrollmentWriteCommand {
    use crate::product::logical_codebase::LogicalRepositoryId;
    use crate::product::models::automation::{
        EnrollmentOptions, EnrollmentSource, EnrollmentWriteCommand, SourceRevisionRef,
    };
    use crate::product::models::lifecycle::IssueWorkItemPlanOptions;
    use crate::product::models::provider::ProviderName;

    EnrollmentWriteCommand::Enable {
        selection_key: "human-choice-auto".into(),
        source: EnrollmentSource {
            stories: vec![SourceRevisionRef {
                id: "story_auto".into(),
                version: 1,
            }],
            designs: vec![SourceRevisionRef {
                id: "design_auto".into(),
                version: 1,
            }],
        },
        options: EnrollmentOptions {
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Fake,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            plan_options: IssueWorkItemPlanOptions {
                include_integration_tests: true,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
        },
        logical_repository_id: LogicalRepositoryId(uuid::Uuid::nil()),
    }
}

fn automation_ownership_store(
    paths: &ProductAppPaths,
) -> crate::product::issue_automation_store::IssueAutomationStore {
    crate::product::issue_automation_store::IssueAutomationStore::new(paths.clone())
}

fn automation_of(
    frame: &crate::web::workspace_ws_types::WsOutMessage,
) -> Option<crate::product::models::automation::AutomationOwnership> {
    match frame {
        crate::web::workspace_ws_types::WsOutMessage::SessionState { automation, .. } => {
            automation.clone()
        }
        other => panic!("expected session_state frame, got {other:?}"),
    }
}

#[test]
fn automation_ownership_store_projects_only_precisely_bound_session() {
    use crate::product::issue_automation_store::IssueAutomationStore;
    use crate::product::models::automation::{
        AutomationOwner, AutomationOwnership, EnrollmentWriteCommand,
    };

    let tmp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(tmp.path().join(".aria"));
    let store = IssueAutomationStore::new(paths.clone());
    let bound = automation_test_record("session_auto_bound", "plan_auto");
    let other = automation_test_record("session_auto_other", "plan_other");

    // 默认 off：无 enrollment 即 client/manual。
    assert_eq!(
        store.ownership_for_session(&bound).unwrap(),
        AutomationOwnership {
            owner: AutomationOwner::Client,
            enrollment_id: None,
            policy_revision: None,
            enabled: false,
        }
    );

    let enrollment = store
        .compare_and_set(
            "project_auto",
            "issue_auto",
            None,
            automation_enable_command(),
        )
        .unwrap();
    // bind_plan 首绑按 CAS 契约推进 revision（+1）；归属以绑定态真值为准。
    let bound_enrollment = store
        .bind_plan(
            "project_auto",
            "issue_auto",
            enrollment.policy_revision,
            "plan_auto",
            "session_auto_bound",
        )
        .unwrap();

    // 仅精确绑定的会话投影 server。
    let bound_ownership = store.ownership_for_session(&bound).unwrap();
    assert_eq!(bound_ownership.owner, AutomationOwner::Server);
    assert_eq!(
        bound_ownership.enrollment_id.as_deref(),
        Some(bound_enrollment.enrollment_id.as_str())
    );
    assert_eq!(
        bound_ownership.policy_revision,
        Some(bound_enrollment.policy_revision)
    );
    assert!(bound_ownership.enabled);

    // 同 issue 其他会话恒 client。
    assert_eq!(
        store.ownership_for_session(&other).unwrap(),
        AutomationOwnership {
            owner: AutomationOwner::Client,
            enrollment_id: None,
            policy_revision: None,
            enabled: false,
        }
    );

    // 关闭：绑定仍在但 owner 退位 client、enabled=false、revision+1。
    store
        .compare_and_set(
            "project_auto",
            "issue_auto",
            Some(bound_enrollment.policy_revision),
            EnrollmentWriteCommand::Disable,
        )
        .unwrap();
    let disabled = store.ownership_for_session(&bound).unwrap();
    assert_eq!(disabled.owner, AutomationOwner::Client);
    assert_eq!(
        disabled.policy_revision,
        Some(bound_enrollment.policy_revision + 1)
    );
    assert!(!disabled.enabled);

    // 删除 enrollment 文件 → 老 session 回 client 默认（不残留内存归属）。
    std::fs::remove_file(
        paths
            .issue_root("project_auto", "issue_auto")
            .join("automation-enrollment.json"),
    )
    .unwrap();
    assert_eq!(
        store.ownership_for_session(&bound).unwrap(),
        AutomationOwnership {
            owner: AutomationOwner::Client,
            enrollment_id: None,
            policy_revision: None,
            enabled: false,
        }
    );

    // 损坏文件 → 显式 projection error，绝不伪作 client。
    std::fs::write(
        paths
            .issue_root("project_auto", "issue_auto")
            .join("automation-enrollment.json"),
        "{ not json",
    )
    .unwrap();
    assert!(store.ownership_for_session(&bound).is_err());
}

fn automation_manager_fixture(
    paths: &ProductAppPaths,
    record: crate::product::models::WorkspaceSessionRecord,
) -> Arc<WorkspaceSessionManager> {
    use crate::product::checkpoint_store::CheckpointStore;
    use crate::product::workspace_engine::{WorkspaceEngine, WorkspaceSession};

    let (engine_tx, _engine_rx) = mpsc::channel(1);
    let checkpoint_store = Arc::new(CheckpointStore::new(
        paths.issue_lifecycle_root(&record.project_id, &record.issue_id),
    ));
    let engine = WorkspaceEngine::new(
        checkpoint_store,
        engine_tx,
        WorkspaceSession::from_record(record.clone()),
    );
    let engine = Arc::new(tokio::sync::Mutex::new(engine));
    let session_id = record.id.clone();
    WorkspaceSessionManager::test_fixture_with_parts(
        &session_id,
        engine,
        Arc::new(ProviderRegistry::new()),
        paths.clone(),
        record,
    )
}

#[tokio::test]
async fn automation_ownership_injected_into_manager_session_state_frames() {
    use crate::product::models::automation::{AutomationOwner, EnrollmentWriteCommand};

    let tmp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(tmp.path().join(".aria"));
    let store = automation_ownership_store(&paths);

    let enrollment = store
        .compare_and_set(
            "project_auto",
            "issue_auto",
            None,
            automation_enable_command(),
        )
        .unwrap();
    let bound_enrollment = store
        .bind_plan(
            "project_auto",
            "issue_auto",
            enrollment.policy_revision,
            "plan_auto",
            "session_auto_bound",
        )
        .unwrap();

    let bound = automation_manager_fixture(
        &paths,
        automation_test_record("session_auto_bound", "plan_auto"),
    );
    let other = automation_manager_fixture(
        &paths,
        automation_test_record("session_auto_other", "plan_other"),
    );

    // 活跃 engine 初帧（attach 路径）与 durable 投影同源：只绑定者 server。
    let (frame, _) = bound.attached_session_state().await;
    let ownership = automation_of(&frame).expect("bound session projects server ownership");
    assert_eq!(ownership.owner, AutomationOwner::Server);
    assert_eq!(
        ownership.policy_revision,
        Some(bound_enrollment.policy_revision)
    );
    assert!(ownership.enabled);

    let (frame, _) = other.attached_session_state().await;
    let ownership = automation_of(&frame).expect("unbound session projects client ownership");
    assert_eq!(ownership.owner, AutomationOwner::Client);
    assert_eq!(ownership.enrollment_id, None);
    assert!(!ownership.enabled);

    let (durable, _) = bound.durable_projection();
    assert_eq!(
        automation_of(&durable).unwrap().owner,
        AutomationOwner::Server
    );

    // summary DTO 同形：绑定者带 revision，未绑定者 client/null。
    let bound_summary = crate::product::models::WorkspaceSessionSummaryRecord {
        id: "session_auto_bound".into(),
        project_id: "project_auto".into(),
        issue_id: "issue_auto".into(),
        entity_id: "plan_auto".into(),
        workspace_type: crate::product::models::WorkspaceType::WorkItemPlan,
        status: crate::product::models::WorkspaceSessionStatus::Open,
        author_provider: crate::product::models::ProviderName::Fake,
        reviewer_provider: crate::product::models::ProviderName::Fake,
        review_rounds: 1,
        superpowers_enabled: false,
        openspec_enabled: false,
    };
    let dto = crate::web::handlers::dto::workspace_session_summary_dto(
        &bound_summary,
        store
            .ownership_for_session(&automation_test_record("session_auto_bound", "plan_auto"))
            .unwrap(),
    );
    assert_eq!(dto.automation.owner, AutomationOwner::Server);
    assert_eq!(
        dto.automation.policy_revision,
        Some(bound_enrollment.policy_revision)
    );

    // 关闭后同 manager 新快照：client/enabled=false/revision+1。
    store
        .compare_and_set(
            "project_auto",
            "issue_auto",
            Some(bound_enrollment.policy_revision),
            EnrollmentWriteCommand::Disable,
        )
        .unwrap();
    let (frame, _) = bound.durable_projection();
    let ownership = automation_of(&frame).unwrap();
    assert_eq!(ownership.owner, AutomationOwner::Client);
    assert_eq!(
        ownership.policy_revision,
        Some(bound_enrollment.policy_revision + 1)
    );
    assert!(!ownership.enabled);

    // 损坏文件 → 显式投影失败：automation 保持未知（None），不伪作 client。
    std::fs::write(
        paths
            .issue_root("project_auto", "issue_auto")
            .join("automation-enrollment.json"),
        "{ not json",
    )
    .unwrap();
    let (frame, _) = bound.durable_projection();
    assert!(
        automation_of(&frame).is_none(),
        "corrupted enrollment must surface unknown ownership, not fake client"
    );
}
