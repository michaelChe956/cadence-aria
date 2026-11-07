// Task 3（REQ-WIGA-17）：取消与取锁的时序尾隙 + 取消结果不得谎称。
//
// 与 part_14 的 REQ-WIGA-15 用例共用同一夹具配方（bootstrap_confirmed_
// work_item_plan_group + 绑定 session + §1.5 假 run），但验的是不同的面：
//   - 用例①：completion 信号已到而 scope 共享 flock 尚未释放的时序尾隙下，
//     删除独占仍必须最终取得成功（不得 409 entity_busy）；
//   - 用例②：取消在兜底超时内未收口（收口失联）时，删除不得谎称已取消、
//     不得因取消失败被额外阻塞（仍按既有独占判定报 entity_busy）。

/// §1.6 用例①夹具：与 part_14 的 `spawn_fake_inflight_run` 同构，但把
/// 「释放共享 flock」与「发 completion 信号」的顺序**倒转**——假 run 任务
/// 被取消后**先** `finish_run` 发真实 completion，**再**延迟 `release_delay`
/// 释放共享 flock ⇒ 制造「completion 已到、锁还没放」的时序尾隙（生产中
/// 二者先后未验证：run 释放 scope 共享 flock 由 run task 全生命周期持有）。
async fn spawn_fake_inflight_run_completion_first(
    app_paths: &ProductAppPaths,
    manager: std::sync::Arc<WorkspaceSessionManager>,
    scope: EntityScope,
    release_delay: std::time::Duration,
) -> FakeInflightRun {
    let (_run_id, token, cancel_token, _cmd_rx, _node_id) = manager
        .start_run(ProviderRunKind::ReviewOnly, None)
        .await
        .expect("start fake in-flight run");
    let lease = EntityOperationStore::new(app_paths.clone())
        .acquire_run(&scope)
        .expect("fake run acquires scope shared lock");
    let shared_lease = std::sync::Arc::new(std::sync::Mutex::new(Some(lease)));
    let task_lease = shared_lease.clone();
    let cancel_in_task = cancel_token.clone();
    let completion_handle = tokio::spawn(async move {
        cancel_in_task.cancelled().await;
        // 先发 completion 信号（尾隙前半：等待解除 ≠ 共享锁已释放）。
        manager.finish_run(token).await;
        // 再延迟释放共享 flock（尾隙窗口 = release_delay，期间锁仍被持有）。
        tokio::time::sleep(release_delay).await;
        if let Some(lease) = task_lease.lock().unwrap().take() {
            drop(lease);
        }
    });
    FakeInflightRun {
        cancel_token,
        completion_handle,
        shared_lease,
    }
}

/// REQ-WIGA-17 场景 ①：取消已发出且该 run 已真实收口（completion 信号已
/// 到），但其 scope 共享 flock 在 completion 之后才延迟释放——删除独占仍
/// SHALL 最终取得成功（不依赖「completion ⇒ 锁已释放」的未验证时序假设），
/// 删除 SHALL NOT 以 `entity_busy` 失败，SHALL NOT 在锁未释放时开始破坏性
/// 清理。
#[tokio::test]
async fn delete_work_item_plan_succeeds_when_scope_lock_releases_after_completion_signal() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state.clone());
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());

    // ①/② 绑定创建 entity_id == work_item_plan_0001 的 WorkItemPlan 会话。
    let session = lifecycle
        .create_workspace_session_bound(
            CreateWorkspaceSessionInput {
                project_id: GROUP_PROJECT_ID.to_string(),
                issue_id: GROUP_ISSUE_ID.to_string(),
                entity_id: GROUP_PLAN_ID.to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Fake,
                reviewer_provider: Some(ProviderName::Fake),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            },
            "workspace_session_9301".to_string(),
        )
        .expect("create bound plan workspace session");

    // ③ manager 挂进 registry（先例 web_lifecycle_api/part_05.rs:373-383）。
    let factory_state = state.clone();
    let factory_session_id = session.id.clone();
    state
        .workspace_sessions
        .get_or_create(&session.id, move || {
            let state = factory_state.clone();
            let session_id = factory_session_id.clone();
            async move { WorkspaceSessionManager::create(&state, &session_id).await }
        })
        .await
        .expect("plan manager attached");
    let manager = state
        .workspace_sessions
        .get(&session.id)
        .await
        .expect("plan manager in registry");

    // ④ 假 in-flight run：被取消后先发 completion、延迟 250ms 才放共享
    //    flock——「completion 已到、锁还没放」的确定性尾隙。
    let scope = EntityScope::new(
        GROUP_PROJECT_ID,
        GROUP_ISSUE_ID,
        DeletionEntityKind::Plan,
        GROUP_PLAN_ID,
    );
    let fake = spawn_fake_inflight_run_completion_first(
        &app_paths,
        manager,
        scope.clone(),
        std::time::Duration::from_millis(250),
    )
    .await;

    // ⑤ DELETE：取消已真实收口，删除独占必须最终取得成功。
    let (status, body) = request_json(app, Method::DELETE, group_plan_uri(), json!({})).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "completion 已到而共享 flock 尚未释放的时序尾隙下，删除必须补足有界等待并最终成功（不得 409 entity_busy）；body: {body}"
    );
    assert_eq!(body["status"], "deleted");

    // 取消确实发生且已真实收口；删除返回时共享 flock 必须已释放——删除
    // 独占只能在锁释放后取得，成功响应本身即「未在锁未释放时开始破坏性
    // 清理」的证据。
    assert!(
        fake.cancel_token.is_cancelled(),
        "删除路径必须先取消该实体在途 run 的 CancellationToken"
    );
    fake.completion_handle
        .await
        .expect("假 run 任务必须已收口（真实 completion）");
    assert!(
        fake.shared_lease.lock().unwrap().is_none(),
        "删除返回时 scope 共享 flock 必须已释放"
    );

    // 既有删除语义未被削弱：plan 已删净（无残留半清理状态）。
    assert!(
        lifecycle
            .get_issue_work_item_plan(GROUP_PROJECT_ID, GROUP_ISSUE_ID, GROUP_PLAN_ID)
            .is_err(),
        "plan 必须已被删除"
    );
}

/// REQ-WIGA-17 场景 ②：取消已发出但该 run 在兜底超时内未收口（收口失联：
/// 既不发 completion 也不释放共享 flock）——删除 SHALL NOT 谎称已取消该
/// run、SHALL NOT 因取消失败被额外阻塞（仍按既有独占判定推进：拿不到独占
/// 按既有语义报出占用者），且必须在有限时间内结束。
///
/// 有限时间上界（5s）远低于生产 30s 兜底：若实现把「取消失败」变成长阻塞
/// （兜底固定 30s 且不可注入），本用例在此上界处见红。
#[tokio::test]
async fn delete_work_item_plan_does_not_claim_cancel_when_completion_times_out() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state.clone());
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());

    // 绑定创建 entity_id == work_item_plan_0001 的 WorkItemPlan 会话并挂进
    // registry。
    let session = lifecycle
        .create_workspace_session_bound(
            CreateWorkspaceSessionInput {
                project_id: GROUP_PROJECT_ID.to_string(),
                issue_id: GROUP_ISSUE_ID.to_string(),
                entity_id: GROUP_PLAN_ID.to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Fake,
                reviewer_provider: Some(ProviderName::Fake),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            },
            "workspace_session_9302".to_string(),
        )
        .expect("create bound plan workspace session");
    let factory_state = state.clone();
    let factory_session_id = session.id.clone();
    state
        .workspace_sessions
        .get_or_create(&session.id, move || {
            let state = factory_state.clone();
            let session_id = factory_session_id.clone();
            async move { WorkspaceSessionManager::create(&state, &session_id).await }
        })
        .await
        .expect("plan manager attached");
    let manager = state
        .workspace_sessions
        .get(&session.id)
        .await
        .expect("plan manager in registry");

    // 兜底时长注入为可测级（200ms；生产默认仍 30s——不得为跑得快删掉
    // 兜底语义，只经 set_run_completion_wait_timeout 注入）。
    manager.set_run_completion_wait_timeout(std::time::Duration::from_millis(200));

    // 假 in-flight run：收口失联——被取消后既不发 completion 信号，也不
    // 释放 scope 共享 flock（run「卡死」仍占删除独占的对手锁）。
    let scope = EntityScope::new(
        GROUP_PROJECT_ID,
        GROUP_ISSUE_ID,
        DeletionEntityKind::Plan,
        GROUP_PLAN_ID,
    );
    let (_run_id, _token, cancel_token, _cmd_rx, _node_id) = manager
        .start_run(ProviderRunKind::ReviewOnly, None)
        .await
        .expect("start fake in-flight run");
    let cancel_for_assert = cancel_token.clone();
    let _unresponsive_run = tokio::spawn(async move {
        cancel_token.cancelled().await;
        // 收口失联：既不 finish_run（无 completion），也永不释放共享 flock。
        std::future::pending::<()>().await;
    });
    // 共享 flock 由本用例持有（模拟 run 任务全生命周期持锁），测试结束随
    // TempDir 与 lease drop 释放。
    let _scope_shared_lease = EntityOperationStore::new(app_paths.clone())
        .acquire_run(&scope)
        .expect("fake run acquires scope shared lock");

    let started = std::time::Instant::now();
    // 有限时间上界：删除不得因取消未收口被额外阻塞（兜底不可注入 ⇒ 30s
    // 长阻塞在本处见红）。
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        request_json(app, Method::DELETE, group_plan_uri(), json!({})),
    )
    .await;
    let elapsed = started.elapsed();
    let (status, body) = response.unwrap_or_else(|_| {
        panic!(
            "删除不得因取消未收口被额外阻塞（{elapsed:?} 内未返回；\
             兜底时长固定 30s 且不可注入 ⇒ 缺可注入兜底与三态取消结果）"
        )
    });

    // 拿不到独占：既有语义逐字不变——409 + entity_busy + 既有占用文案。
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "取消未收口时删除仍按既有独占判定推进（共享锁仍被持有 ⇒ 409）；body: {body}"
    );
    assert_eq!(body["code"], "entity_busy", "body: {body}");
    assert_eq!(
        body["message"],
        "实体存在进行中的运行或并发写入，请等待收口后重试删除",
        "既有 entity_busy 文案逐字不变；body: {body}"
    );

    // 不得谎称：响应体不得出现与事实相反的「已取消/已停止」表述。
    let body_text = body.to_string();
    assert!(
        !body_text.contains("已取消"),
        "取消未真正收口，响应体不得谎称已取消：{body_text}"
    );
    assert!(
        !body_text.contains("已停止"),
        "取消未真正收口，响应体不得谎称已停止：{body_text}"
    );

    // 有限时间内结束（秒级，远低于 30s 兜底）。
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "用例必须在有限时间内结束（实测 {elapsed:?}）"
    );

    // 取消确实发出过（token 已触发）——「没谎称」不是「没取消」。
    assert!(
        cancel_for_assert.is_cancelled(),
        "删除路径必须已发出取消（CancellationToken 已触发）"
    );

    // 未开始破坏性清理：实体仍在、无删除墓碑。
    assert!(
        lifecycle
            .get_issue_work_item_plan(GROUP_PROJECT_ID, GROUP_ISSUE_ID, GROUP_PLAN_ID)
            .is_ok(),
        "取消未收口、独占未取得时不得开始破坏性清理（plan 必须仍在）"
    );
    assert!(
        cadence_aria::product::entity_deletion_store::EntityDeletionStore::new(
            app_paths.clone()
        )
        .get(&scope)
        .expect("read deletion record")
        .is_none(),
        "取消未收口、独占未取得时不得立起删除墓碑"
    );
}

// ---------------------------------------------------------------------------
// Task 6（Review Focus ⑤）：既有删除语义护栏 —— 「取消在途 run」不得削弱
// 既有删除语义与账本一次性。
//
//   - 用例①：删除成功 ⇒ 该实体的自动化登记**只终止一次**（terminated_at
//     只写一次、终止账本条目恰好一条、既有账本条目逐字未变、revision 只被
//     终止(+1)与作废(+1)两次递增），登记记录**未被物理删除**
//     （REQ-DRG-18 / REQ-WIGA-VOID-01）。
//   - 用例②：门禁拒绝（存在 coding attempt ⇒ coding_workspace_exists）
//     ⇒ **零破坏**：墓碑未立起、实体仍在、coding 产物仍在、别的登记未被
//     终止/作废/改写。
// ---------------------------------------------------------------------------

use cadence_aria::product::logical_codebase::EnrollmentTarget;
use cadence_aria::product::models::automation::{
    EnrollmentOptions, EnrollmentSource, EnrollmentWriteCommand, IssueAutomationEnrollment,
    OperationState, PlanApprovalPolicy, SourceRevisionRef,
};

/// 播种一条 issue 级自动化登记（Enable ＋ 绑定 plan/session）：删除路径的
/// 终止/作废面都以它为对象。
///
/// `command_id` 显式给出 ⇒ 首条 Enable 进命令账本（后续「旧条目逐字未变」
/// 的断言需要至少一条既有条目）。`plan_id`/`session_id` 决定该登记是否
/// **绑定**被删实体（绑定面才会被终止/作废）。
fn seed_guardrail_enrollment(
    app_paths: &ProductAppPaths,
    plan_id: &str,
    session_id: &str,
    command_id: &str,
) -> IssueAutomationEnrollment {
    let store = IssueAutomationStore::new(app_paths.clone());
    let enabled = store
        .compare_and_set(
            GROUP_PROJECT_ID,
            GROUP_ISSUE_ID,
            None,
            EnrollmentWriteCommand::Enable {
                command_id: Some(command_id.to_string()),
                selection_key: "guardrail-selection-1".to_string(),
                source: EnrollmentSource {
                    stories: vec![SourceRevisionRef {
                        id: "story_spec_0001".to_string(),
                        version: 1,
                    }],
                    designs: vec![SourceRevisionRef {
                        id: "design_spec_0001".to_string(),
                        version: 1,
                    }],
                },
                options: EnrollmentOptions {
                    author_provider: ProviderName::Fake,
                    reviewer_provider: ProviderName::Fake,
                    review_rounds: 1,
                    superpowers_enabled: false,
                    openspec_enabled: false,
                    plan_approval: PlanApprovalPolicy::Manual,
                    plan_options: IssueWorkItemPlanOptions {
                        include_integration_tests: true,
                        include_e2e_tests: false,
                        force_frontend_backend_split: true,
                        require_execution_plan_confirm: false,
                    },
                },
                target: EnrollmentTarget::SingleRepository {
                    repository_id: "repository_0001".to_string(),
                },
            },
        )
        .expect("seed enrollment enable");
    store
        .bind_plan(
            GROUP_PROJECT_ID,
            GROUP_ISSUE_ID,
            enabled.policy_revision,
            plan_id,
            session_id,
        )
        .expect("bind enrollment to plan/session")
}

/// 绑定该实体的 WorkItemPlan 会话并把 manager 挂进 registry（删除路径据此
/// 找到该实体的在途 run 并取消它）。返回该实体的 session manager。
async fn bind_plan_session_with_manager(
    state: &WebAppState,
    lifecycle: &LifecycleStore,
    session_id: &str,
) -> std::sync::Arc<WorkspaceSessionManager> {
    let session = lifecycle
        .create_workspace_session_bound(
            CreateWorkspaceSessionInput {
                project_id: GROUP_PROJECT_ID.to_string(),
                issue_id: GROUP_ISSUE_ID.to_string(),
                entity_id: GROUP_PLAN_ID.to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Fake,
                reviewer_provider: Some(ProviderName::Fake),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            },
            session_id.to_string(),
        )
        .expect("create bound plan workspace session");
    let factory_state = state.clone();
    let factory_session_id = session.id.clone();
    state
        .workspace_sessions
        .get_or_create(&session.id, move || {
            let state = factory_state.clone();
            let session_id = factory_session_id.clone();
            async move { WorkspaceSessionManager::create(&state, &session_id).await }
        })
        .await
        .expect("plan manager attached");
    state
        .workspace_sessions
        .get(&session.id)
        .await
        .expect("plan manager in registry")
}

/// Task 6 护栏①（既有语义不被削弱）：有在途 run 时删除该工作项组成功后，
/// 绑定它的自动化登记必须**只终止一次**（terminated_at 只写一次、终止账本
/// 条目恰好一条、既有账本条目逐字未变、revision 只被终止(+1)与作废(+1)
/// 两次递增），且登记记录**未被物理删除**（REQ-DRG-18 / REQ-WIGA-VOID-01）。
#[tokio::test]
async fn delete_work_item_plan_with_inflight_run_terminates_enrollment_once() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state.clone());
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());

    // 绑定创建 entity_id == work_item_plan_0001 的 WorkItemPlan 会话并挂
    // 进 registry（删除路径据此取消该实体的在途 run）。
    let manager =
        bind_plan_session_with_manager(&state, &lifecycle, "workspace_session_9401").await;

    // 播种绑定该 plan 的自动化登记（Enable ＋ bind plan/session）。
    seed_guardrail_enrollment(
        &app_paths,
        GROUP_PLAN_ID,
        "workspace_session_9401",
        "guardrail-enable-1",
    );
    let before = IssueAutomationStore::new(app_paths.clone())
        .get(GROUP_PROJECT_ID, GROUP_ISSUE_ID)
        .expect("read seeded enrollment")
        .expect("seeded enrollment exists");
    assert!(before.enabled, "播种的登记必须处于 enabled");
    assert!(before.terminated_at.is_none(), "播种的登记不得已终止");
    assert_eq!(
        before.plan_id.as_deref(),
        Some(GROUP_PLAN_ID),
        "播种的登记必须绑定被删 plan（否则删除路径不认它是占用者）"
    );
    assert!(
        !before.command_ledger.is_empty(),
        "播种的 Enable 必须留一条账本条目（护栏需要「旧条目」快照）"
    );

    // 假 in-flight run（复用 part_14 的 spawn_fake_inflight_run —— 跨 part
    // 同模块可见，不重写一份）。
    let scope = EntityScope::new(
        GROUP_PROJECT_ID,
        GROUP_ISSUE_ID,
        DeletionEntityKind::Plan,
        GROUP_PLAN_ID,
    );
    let fake = spawn_fake_inflight_run(&app_paths, manager, scope.clone()).await;

    // DELETE：取消在途 run 后删除成功（既有删除语义未被削弱）。
    let (status, body) = request_json(app, Method::DELETE, group_plan_uri(), json!({})).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "取消在途 run 后删除必须成功（409 entity_busy 即缺本修法）；body: {body}"
    );
    assert_eq!(body["status"], "deleted");
    assert!(
        fake.cancel_token.is_cancelled(),
        "删除路径必须先取消该实体在途 run 的 CancellationToken"
    );
    fake.completion_handle
        .await
        .expect("假 run 任务必须已收口（真实 completion）");

    // 登记记录仍在（未被物理删除——删除工作项组只作废登记，不删登记文件）。
    let enrollment_path = group_issue_root(&app_paths).join("automation-enrollment.json");
    assert!(
        enrollment_path.exists(),
        "登记记录不得被物理删除：{}",
        enrollment_path.display()
    );
    let after = IssueAutomationStore::new(app_paths.clone())
        .get(GROUP_PROJECT_ID, GROUP_ISSUE_ID)
        .expect("read enrollment after delete")
        .expect("登记记录删除后仍在（未被物理删除）");

    // 终止恰好一次：已终止（不得停留在「未终止」）且不得重复终止。
    assert!(
        after.terminated_at.is_some(),
        "删除成功后登记必须被终止一次（terminated_at 非空）"
    );
    assert!(!after.enabled, "终止后 enabled 必须为 false");
    let terminate_entries: Vec<_> = after
        .command_ledger
        .iter()
        .filter(|entry| entry.command_id.starts_with("entity-delete-terminate-"))
        .collect();
    assert_eq!(
        terminate_entries.len(),
        1,
        "终止账本条目必须恰好一条（不得重复终止）：{:?}",
        after
            .command_ledger
            .iter()
            .map(|entry| entry.command_id.as_str())
            .collect::<Vec<_>>()
    );
    assert_eq!(
        terminate_entries[0].command_id,
        format!(
            "entity-delete-terminate-{}-r{}",
            before.enrollment_id, before.policy_revision
        ),
        "终止命令身份必须绑定终止当时的 revision"
    );
    assert_eq!(
        terminate_entries[0].state,
        OperationState::Accepted,
        "终止账本条目必须是已接受"
    );

    // 作废条目（REQ-WIGA-VOID-01）：删除成功后登记被作废，且只作废一次。
    let void_command_id = format!("void-enrollment-{GROUP_PLAN_ID}");
    let void_entries: Vec<_> = after
        .command_ledger
        .iter()
        .filter(|entry| entry.command_id == void_command_id)
        .collect();
    assert_eq!(
        void_entries.len(),
        1,
        "作废账本条目必须恰好一条（不得重复作废）"
    );
    assert!(
        after.voided_at.is_some(),
        "删除成功后登记必须被作废（voided_at 非空）"
    );
    assert_eq!(
        after.void_reason.as_deref(),
        Some("work_item_group_deleted"),
        "作废原因必须是既有稳定码"
    );

    // revision 恰好来自既有两条命令：终止(+1) ＋ 作废(+1)——再多即重复写入。
    assert_eq!(
        after.policy_revision,
        before.policy_revision + 2,
        "policy_revision 只允许终止(+1)与作废(+1)两次递增（重复终止/作废会多涨）"
    );

    // 既有账本条目逐字未变，且恰好追加两条（终止一条 ＋ 作废一条）。
    assert_eq!(
        after.command_ledger.len(),
        before.command_ledger.len() + 2,
        "命令账本必须恰好追加两条（终止一条 + 作废一条）"
    );
    for (old, kept) in before.command_ledger.iter().zip(after.command_ledger.iter()) {
        assert_eq!(old, kept, "既有账本条目必须逐字未变");
    }
}

/// Task 6 护栏②（门禁拒绝 ⇒ 零破坏）：本 group 存在 coding attempt 时删除
/// 必被 coding 门禁拒绝（既有稳定码与文案逐字不变），且**零破坏**——墓碑
/// 未立起、实体与其 work item 仍在、coding 产物仍在、登记未被物理删除，
/// 绑定**别的**工作项组的登记不得被终止/作废/改写。
///
/// ★在途 run 叠加★：取消动作不是破坏性清理——门禁拒绝后不得留下任何
/// 删除痕迹（墓碑/pending/实体/产物全在）。
#[tokio::test]
async fn delete_work_item_plan_gate_rejection_keeps_zero_destruction() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state.clone());
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());

    // 门禁构造：真实 group attempt（其记录存在 ⇒ coding 门禁拒删）。
    let (create_status, created) =
        request_json(app.clone(), Method::POST, group_attempt_uri(), json!({})).await;
    assert_eq!(
        create_status,
        StatusCode::OK,
        "group attempt create: {created}"
    );
    let attempt_id = assert_global_attempt_id(&created);

    // 绑定创建 entity_id == work_item_plan_0001 的 WorkItemPlan 会话并挂
    // 进 registry（删除路径据此取消该实体的在途 run）。
    let manager =
        bind_plan_session_with_manager(&state, &lifecycle, "workspace_session_9402").await;

    // 播种**绑定别的**工作项组的登记（plan_id 与 session_id 都不指向本实体）：
    // 门禁拒绝时删除路径不得越界终止/作废它。
    seed_guardrail_enrollment(
        &app_paths,
        "work_item_plan_0002",
        "workspace_session_9403",
        "guardrail-enable-2",
    );
    let before = IssueAutomationStore::new(app_paths.clone())
        .get(GROUP_PROJECT_ID, GROUP_ISSUE_ID)
        .expect("read seeded enrollment")
        .expect("seeded enrollment exists");
    assert!(before.enabled && before.terminated_at.is_none());

    // 假 in-flight run（复用 part_14 的 spawn_fake_inflight_run）。
    let scope = EntityScope::new(
        GROUP_PROJECT_ID,
        GROUP_ISSUE_ID,
        DeletionEntityKind::Plan,
        GROUP_PLAN_ID,
    );
    let fake = spawn_fake_inflight_run(&app_paths, manager, scope.clone()).await;

    // DELETE：coding 门禁拒绝（既有稳定码与文案逐字不变）。
    let (status, body) = request_json(app, Method::DELETE, group_plan_uri(), json!({})).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "存在 coding attempt 时删除必须被门禁拒绝；body: {body}"
    );
    assert_eq!(body["code"], "coding_workspace_exists", "body: {body}");
    assert_eq!(
        body["message"],
        "存在 coding workspace，请先删除 coding workspace 再删除 work item group",
        "既有 coding_workspace_exists 文案逐字不变；body: {body}"
    );

    // 取消动作不是破坏性清理：在途 run 仍被取消（既有取消语义），但删除
    // 未执行 ⇒ 无任何删除痕迹。
    assert!(
        fake.cancel_token.is_cancelled(),
        "删除路径必须已发出取消（CancellationToken 已触发）"
    );
    fake.completion_handle
        .await
        .expect("假 run 任务必须已收口（真实 completion）");

    // 零破坏①：墓碑未立起（entity_deletions/plan/ 下无该实体的记录）。
    assert!(
        cadence_aria::product::entity_deletion_store::EntityDeletionStore::new(app_paths.clone())
            .get(&scope)
            .expect("read deletion record")
            .is_none(),
        "门禁拒绝时不得立起删除墓碑"
    );

    // 零破坏②：实体与其 work item 仍在（未被部分删除）。
    assert!(
        lifecycle
            .get_issue_work_item_plan(GROUP_PROJECT_ID, GROUP_ISSUE_ID, GROUP_PLAN_ID)
            .is_ok(),
        "门禁拒绝时工作项组必须仍在（不得部分删除）"
    );
    let work_items = lifecycle
        .list_work_items(GROUP_PROJECT_ID, GROUP_ISSUE_ID)
        .expect("list work items after rejected delete");
    assert!(
        work_items.iter().any(|item| item.id == "work_item_0001")
            && work_items.iter().any(|item| item.id == "work_item_0002"),
        "门禁拒绝时所属 work item 必须仍在（不得部分删除）"
    );

    // 零破坏③：coding 产物仍在（attempt 主体 json 未被清理）。
    let attempt_json = app_paths
        .issue_lifecycle_root(GROUP_PROJECT_ID, GROUP_ISSUE_ID)
        .join("coding-attempts")
        .join(format!("{attempt_id}.json"));
    assert!(
        attempt_json.exists(),
        "门禁拒绝时 coding attempt 产物必须仍在（不得有部分删除痕迹）：{}",
        attempt_json.display()
    );

    // 零破坏④：登记未被物理删除，且**未被终止/作废/改写**（门禁拒绝时删除
    // 路径不得越界动别的登记）。
    let enrollment_path = group_issue_root(&app_paths).join("automation-enrollment.json");
    assert!(
        enrollment_path.exists(),
        "登记记录不得被物理删除：{}",
        enrollment_path.display()
    );
    let after = IssueAutomationStore::new(app_paths.clone())
        .get(GROUP_PROJECT_ID, GROUP_ISSUE_ID)
        .expect("read enrollment after rejected delete")
        .expect("登记记录仍在（未被物理删除）");
    assert!(
        after.terminated_at.is_none(),
        "门禁拒绝时不得终止任何登记（terminated_at 必须仍为空）"
    );
    assert!(
        after.voided_at.is_none(),
        "门禁拒绝时不得作废任何登记（voided_at 必须仍为空）"
    );
    assert!(
        after.enabled,
        "门禁拒绝时不得停用登记（enabled 必须仍为 true）"
    );
    assert_eq!(
        after.policy_revision,
        before.policy_revision,
        "门禁拒绝时登记 revision 不得变（零写入）"
    );
    assert_eq!(
        after.command_ledger, before.command_ledger,
        "门禁拒绝时命令账本不得变（零写入）"
    );
}
