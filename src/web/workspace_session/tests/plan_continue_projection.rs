/// REQ-HGC-04：`plan_continue_available` 的 WS 投影测试（块1 片5）。
///
/// 钉住两个失效形态：
/// ① 帧缺该字段（`None`）→ 门卡按「无真实续跑入口」处理，不得给假指引；
/// ② 各取帧面取值不一致（活引擎帧 vs durable 投影帧）→ 重连后门卡忽然变
///    假指引。
/// 取帧面上的 `plan_continue_available`（非 SessionState 帧即测试失败）。
fn plan_continue_of(
    frame: &crate::web::workspace_ws_types::WsOutMessage,
) -> Option<bool> {
    match frame {
        crate::web::workspace_ws_types::WsOutMessage::SessionState {
            plan_continue_available,
            ..
        } => *plan_continue_available,
        other => panic!("expected session_state frame, got {other:?}"),
    }
}

/// 可续跑停点现场：enabled + 精确绑定的 WorkItemPlan 会话（durable
/// `stopped_needs_human` + resumable 门快照）→ 服务端确有「继续自动化」入口。
struct PlanContinueFixture {
    _tmp: tempfile::TempDir,
    paths: ProductAppPaths,
    record: crate::product::models::WorkspaceSessionRecord,
    engine: Arc<tokio::sync::Mutex<crate::product::workspace_engine::WorkspaceEngine>>,
    manager: Arc<WorkspaceSessionManager>,
}

fn plan_continue_enable_command() -> crate::product::models::automation::EnrollmentWriteCommand {
    use crate::product::models::automation::{
        EnrollmentOptions, EnrollmentSource, EnrollmentWriteCommand, PlanApprovalPolicy,
    };
    use crate::product::models::lifecycle::IssueWorkItemPlanOptions;
    use crate::product::models::provider::ProviderName;

    EnrollmentWriteCommand::Enable {
        command_id: None,
        selection_key: "plan-continue-projection".into(),
        source: EnrollmentSource {
            stories: Vec::new(),
            designs: Vec::new(),
        },
        options: EnrollmentOptions {
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Fake,
            review_rounds: 0,
            superpowers_enabled: false,
            openspec_enabled: false,
            plan_approval: PlanApprovalPolicy::Auto,
            plan_options: IssueWorkItemPlanOptions {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
        },
        target: crate::product::logical_codebase::EnrollmentTarget::SingleRepository {
            repository_id: "repository_0001".to_string(),
        },
    }
}

/// `persist = true` 落盘 durable 会话记录；`false` 时**不落盘**（模拟读取
/// 失败：会话记录读不到）。
fn plan_continue_fixture(persist: bool) -> PlanContinueFixture {
    const PROJECT: &str = "project_0001";
    const ISSUE: &str = "issue_0001";
    const SESSION: &str = "workspace_session_plan_continue_1";
    const PLAN: &str = "issue_work_item_plan_continue_1";

    let tmp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(tmp.path().join(".aria"));
    let store = crate::product::issue_automation_store::IssueAutomationStore::new(paths.clone());
    let enrollment = store
        .compare_and_set(PROJECT, ISSUE, None, plan_continue_enable_command())
        .expect("enable enrollment");

    let mut record = super::test_session_record(SESSION);
    record.project_id = PROJECT.to_string();
    record.issue_id = ISSUE.to_string();
    record.entity_id = PLAN.to_string();
    record.workspace_type = WorkspaceType::WorkItemPlan;
    record.status = crate::product::models::WorkspaceSessionStatus::StoppedNeedsHuman;
    record.single_candidate_phase = Some(SingleCandidatePhase::Evaluate);
    record.human_gate_snapshot = Some(HumanGateSnapshot {
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 1,
        trigger: HumanReason::NativeHumanRequired,
        resumable: true,
        accepted_feedback_turns: Some(0),
    });

    if persist {
        crate::product::json_store::write_json(
            &paths
                .issue_lifecycle_root(PROJECT, ISSUE)
                .join("workspace-sessions")
                .join(format!("{SESSION}.json")),
            &record,
        )
        .expect("persist durable session record");
        store
            .bind_plan(PROJECT, ISSUE, enrollment.policy_revision, PLAN, SESSION)
            .expect("bind plan");
    }

    let checkpoint_store = Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
        tmp.path().join("checkpoints"),
    ));
    let (tx, _rx) = mpsc::channel(1);
    // `plan_continue_available` 经 `WorkspaceEngine::revision_store()` 读
    // lifecycle store——持久引擎缺它即 panic（`persistent WorkspaceEngine
    // requires lifecycle_store`），故夹具必须用持久形态构造引擎。
    let engine = Arc::new(tokio::sync::Mutex::new(
        crate::product::workspace_engine::WorkspaceEngine::new_persistent(
            checkpoint_store,
            crate::product::lifecycle_store::LifecycleStore::new(paths.clone()),
            tx,
            crate::product::workspace_engine::WorkspaceSession::from_record(record.clone()),
        ),
    ));
    let manager = WorkspaceSessionManager::test_fixture_with_parts(
        SESSION,
        engine.clone(),
        Arc::new(ProviderRegistry::new()),
        paths.clone(),
        record.clone(),
    );
    PlanContinueFixture {
        _tmp: tmp,
        paths,
        record,
        engine,
        manager,
    }
}

/// REQ-HGC-04：WS `SessionState` 帧必须携带 `plan_continue_available`，且
/// 活引擎取帧面（attach 首帧 / 广播帧）与 durable 投影面（observer 重建 /
/// 引擎锁忙时的降级帧）取值一致（**非 None**）——缺字段即「假指引」缺陷形态。
#[tokio::test]
async fn plan_continue_available_is_projected_on_attach_reconnect_and_observer_rebuild() {
    let fixture = plan_continue_fixture(true);

    // 活引擎取帧面：attach 首帧 / 广播帧 / 修订补帧共用 manager 投影。
    let mut live_frame = fixture.engine.lock().await.build_session_state();
    fixture
        .manager
        .project_plan_continue_available(&mut live_frame);
    assert_eq!(
        plan_continue_of(&live_frame),
        Some(true),
        "live session_state frame must carry the durable continue fact"
    );

    // durable 投影面：observer 重建 / 刷新时引擎锁被 run 持有的降级帧。
    let projection = super::manager::load_durable_workspace_projection(
        &fixture.paths,
        &super::manager::WorkspaceSessionIdentity::from_bound_record(&fixture.record),
    )
    .expect("durable projection");
    assert_eq!(
        plan_continue_of(&projection.session_state),
        Some(true),
        "durable projection frame must carry the same continue fact"
    );

    // 两面取值必须一致（重连后不得忽然变假指引）。
    assert_eq!(
        plan_continue_of(&live_frame),
        plan_continue_of(&projection.session_state),
        "live and durable frames must agree on the continue fact"
    );
}

/// REQ-HGC-04：读取失败（durable 会话记录读不到）→ `None`（**不猜**），
/// 绝不伪造成「确定没有入口」以外的事实，也绝不伪造成「有入口」。
#[tokio::test]
async fn plan_continue_available_is_none_when_read_fails() {
    let fixture = plan_continue_fixture(false);

    let mut live_frame = fixture.engine.lock().await.build_session_state();
    fixture
        .manager
        .project_plan_continue_available(&mut live_frame);
    assert_eq!(
        plan_continue_of(&live_frame),
        None,
        "unreadable durable record must stay unknown (None), never guessed"
    );
}
