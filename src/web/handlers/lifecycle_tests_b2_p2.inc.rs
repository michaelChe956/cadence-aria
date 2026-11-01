// 批 2 P2（REQ-WIGA-02 修订）定向测试：bound 创建/生成准入/确认投影按
// enrollment 冻结 plan_approval 策略互证。经 lifecycle.rs 的 `mod tests`
// include! 引入（large_file_guard 1200 行红线拆分，沿 n7/d2_2 惯例）；
// 共享 `use super::*` 的导入。

/// 批 2 P2（REQ-WIGA-02 修订）：bound 创建的 session run policy 取请求
/// 冻结值——auto 意图产出 AutoIfValid、manual/缺省保持 Interactive；
/// run policy 漂移按完整身份比较拒绝（与既有 options 漂移同口径）。
#[test]
fn prepare_bound_plan_honors_frozen_run_policy_from_request() {
    let root = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let (lifecycle, story_id, design_id) = seed_prepare_work_item_plan_fixture(&paths, 1);
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let mut request = plan_prepare_request(story_id, design_id);
    let ids = PreparedPlanIds {
        plan_id: "issue_work_item_plan_auto_policy_fixture".into(),
        session_id: "workspace_session_auto_policy_fixture".into(),
    };

    // auto 冻结意图：bound 会话取 AutoIfValid（红：现一律 Interactive）。
    request.run_policy = Some(RunPolicy::AutoIfValid);
    let first =
        prepare_plan_records(&state, PROJECT_ID, ISSUE_ID, request.clone(), Some(ids.clone()))
            .unwrap();
    assert_eq!(first.session.id, "workspace_session_auto_policy_fixture");
    assert_eq!(first.session.run_policy, RunPolicy::AutoIfValid);

    // 同 id 降级漂移（AutoIfValid → Interactive）：完整身份比较拒绝。
    let mut drifted = request.clone();
    drifted.run_policy = Some(RunPolicy::Interactive);
    assert!(
        prepare_plan_records(&state, PROJECT_ID, ISSUE_ID, drifted, Some(ids.clone())).is_err()
    );

    // 原请求重试幂等；durable 会话仍恰一条。
    let repeated = prepare_plan_records(&state, PROJECT_ID, ISSUE_ID, request, Some(ids)).unwrap();
    assert_eq!(repeated.session.id, first.session.id);
    assert_eq!(repeated.session.created_at, first.session.created_at);
    assert_eq!(
        lifecycle
            .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .len(),
        1
    );
}

/// 批 2 P2 集成（REQ-WIGA-02 修订）：真实 REST Enable 冻结 auto →
/// `ensure_enrolled_plan` 补偿创建 AutoIfValid 绑定会话 → 生成准入按
/// 冻结期望放行并派发 provider run（Running）。
#[tokio::test]
async fn auto_enrolled_plan_binds_auto_if_valid_and_generation_admits() {
    let inner =
        crate::web::handlers::automation_enrollment_test_support::seed_single_repository_fixture();
    init_real_main_checkout_for_bound_policy(&inner);
    let root_path = inner._root.path().to_path_buf();
    let state = WebAppState::new(root_path.clone(), WebRuntime::new_fake(root_path.clone()));
    let app = build_web_router(state.clone());

    let mut body =
        crate::web::handlers::automation_enrollment_test_support::single_repository_enable_body(
            &inner,
        );
    body["command"]["options"]["plan_approval"] = serde_json::json!("auto");
    // gate outline 无 integration 工作项：与单仓 fixture 同口径关闭
    // include_integration_tests（编译严格校验会拒绝缺项计划）。
    body["command"]["options"]["plan_options"]["include_integration_tests"] =
        serde_json::json!(false);
    let enable =
        crate::web::handlers::automation_enrollment_test_support::put_enrollment(&app, body).await;
    assert_eq!(enable.status(), StatusCode::OK);
    assert!(
        crate::web::handlers::automation_enrollment_test_support::response_json(enable)
            .await["enabled"]
            .as_bool()
            .unwrap()
    );

    let store =
        crate::product::issue_automation_store::IssueAutomationStore::new(inner.paths.clone());
    let enrollment = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
    assert_eq!(
        enrollment.options.plan_approval,
        crate::product::models::automation::PlanApprovalPolicy::Auto
    );

    // 补偿创建：auto 冻结 → AutoIfValid 绑定会话（红：现一律 Interactive）。
    let records =
        crate::web::handlers::lifecycle::plan_preparation::ensure_enrolled_plan(&state, &enrollment)
            .await
            .unwrap();
    assert_eq!(records.session.run_policy, RunPolicy::AutoIfValid);
    let bound = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
    assert_eq!(bound.session_id.as_deref(), Some(records.session.id.as_str()));

    // 生成准入：AutoIfValid 会话 + auto 冻结期望 → 派发（红：现 NeedsHuman
    // 且零 provider 启动）。
    let outcome = crate::web::plan_generation::start_plan_generation_once(&state, &bound)
        .await
        .unwrap();
    assert_eq!(
        outcome,
        crate::web::plan_generation::PlanGenerationOutcome::Running
    );
}

/// 批 2 P2 集成（REQ-WIGA-02/03 修订）：auto 绑定会话的正常批准经引擎
/// AutoIfValid 语义直达 Confirmed（无人工 approve、编排器无自动 approve
/// 循环）；`plan_confirmed_info` 按冻结策略互证派生成功投影；错绑
///（manual 冻结对 AutoIfValid 会话）不投影。
#[tokio::test]
async fn auto_enrolled_sc_confirms_via_engine_and_projects_confirmed_info() {
    let inner =
        crate::web::handlers::automation_enrollment_test_support::seed_single_repository_fixture();
    init_real_main_checkout_for_bound_policy(&inner);
    let root_path = inner._root.path().to_path_buf();
    let state = WebAppState::new(root_path.clone(), WebRuntime::new_fake(root_path.clone()));
    let app = build_web_router(state.clone());

    let mut body =
        crate::web::handlers::automation_enrollment_test_support::single_repository_enable_body(
            &inner,
        );
    body["command"]["options"]["plan_approval"] = serde_json::json!("auto");
    body["command"]["options"]["plan_options"]["include_integration_tests"] =
        serde_json::json!(false);
    let enable =
        crate::web::handlers::automation_enrollment_test_support::put_enrollment(&app, body).await;
    assert_eq!(enable.status(), StatusCode::OK);

    let store =
        crate::product::issue_automation_store::IssueAutomationStore::new(inner.paths.clone());
    let enrollment = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
    let records =
        crate::web::handlers::lifecycle::plan_preparation::ensure_enrolled_plan(&state, &enrollment)
            .await
            .unwrap();
    assert_eq!(records.session.run_policy, RunPolicy::AutoIfValid);
    let session_id = records.session.id.clone();
    let plan_id = records.plan.id.clone();

    // 引擎面铺编译底座（accepted drafts + active index + Approval 相位，
    // 与审批门 fixture 同源），再经 AutoIfValid 的 policy-valid compile
    // 直达 Confirmed——本测试全程零 approve/人工门动作。
    let manager = state
        .workspace_sessions
        .get_or_create(&session_id, || {
            crate::web::workspace_session::WorkspaceSessionManager::create(&state, &session_id)
        })
        .await
        .unwrap();
    let engine_arc = manager.engine();
    {
        let mut engine = engine_arc.lock().await;
        engine.session.artifact = Some(crate::web::wiga_gate_fixture::gate_outline_candidate_payload());
        let plan_store =
            crate::product::work_item_plan_store::WorkItemPlanStore::new(inner.paths.clone());
        let draft_a =
            crate::web::wiga_gate_fixture::gate_draft_record(&plan_id, "outline_a", "draft_outline_a");
        let mut draft_b =
            crate::web::wiga_gate_fixture::gate_draft_record(&plan_id, "outline_b", "draft_outline_b");
        let mut required = crate::product::work_item_contract::canonical_contract_fixture("unused")
            .input_contracts
            .remove(0);
        required.provider_logical_work_item_id = "wi_a".to_string();
        required.contract_id = "contract.canonical".to_string();
        required.required_capabilities = vec!["stable_hash".to_string()];
        draft_b
            .candidate
            .canonical_contract_candidate
            .input_contracts
            .push(required);
        draft_b
            .candidate
            .canonical_contract_candidate
            .handoff_contract
            .provided_contract_refs
            .clear();
        plan_store.put_draft_record(&draft_a).unwrap();
        plan_store.put_draft_record(&draft_b).unwrap();
        plan_store
            .save_active_index(&crate::web::wiga_gate_fixture::gate_active_index(&plan_id))
            .unwrap();
        crate::product::workspace_engine::tests::single_candidate_recovery::single_candidate_recovery_record(
            &inner.lifecycle,
            &mut engine,
            crate::product::models::SingleCandidatePhase::Approval,
            RunPolicy::AutoIfValid,
        );
        engine.enter_policy_valid_work_item_plan_compile().await;
    }

    // 正常批准经引擎 AutoIfValid 语义：durable Confirmed + Completed。
    let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(inner.paths.clone());
    let durable = lifecycle.get_workspace_session(&session_id).unwrap();
    assert_eq!(
        durable.status,
        crate::product::models::WorkspaceSessionStatus::Confirmed
    );
    assert_eq!(
        durable.single_candidate_phase,
        Some(crate::product::models::SingleCandidatePhase::Completed)
    );

    // 确认投影：按冻结 auto 策略互证 → 派生（红：现拒非 Interactive）。
    let bound = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
    let info = crate::web::plan_confirmed_info::plan_confirmed_info(&inner.paths, &bound)
        .unwrap()
        .expect("auto 冻结 + AutoIfValid Confirmed 会话必须投影成功信息");
    assert_eq!(info.session_id, session_id);
    assert_eq!(info.plan_id, plan_id);
    assert_eq!(info.title, "Work Item Plan 已确认");

    // 错绑对照：manual 冻结（期望 Interactive）对同一 AutoIfValid 会话
    // 不投影——投影接缝互证冻结策略，不一致即拒绝。
    let mut misbound = bound.clone();
    misbound.options.plan_approval = crate::product::models::automation::PlanApprovalPolicy::Manual;
    assert!(
        crate::web::plan_confirmed_info::plan_confirmed_info(&inner.paths, &misbound)
            .unwrap()
            .is_none()
    );
}

/// 与 `wiga_gate_fixture::p2_campaign::init_real_main_checkout` 同款的真实
/// main 分支 git 仓（空提交 + 本地 bare origin）：Final Compile 的
/// plan_baseline_tree 基线解析与 advance 分支三面同源验证需要真实 git
/// 现场（该函数为 fixture 私有，本文件按同口径复制，不改 fixture）。
fn init_real_main_checkout_for_bound_policy(
    fixture: &crate::web::handlers::automation_enrollment_test_support::Fixture,
) {
    fn git(repo: &std::path::Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .expect("spawn git");
        assert!(
            output.status.success(),
            "git {args:?} in {repo:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let repo = fixture._root.path().join("repo-1");
    git(&repo, &["init"]);
    git(&repo, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    git(&repo, &["config", "user.email", "aria@example.com"]);
    git(&repo, &["config", "user.name", "Aria Test"]);
    git(&repo, &["commit", "--allow-empty", "-m", "advance base"]);
    let origin = fixture._root.path().join("origin-bound-policy.git");
    let _ = std::fs::remove_dir_all(&origin);
    git(
        &repo,
        &["init", "--bare", origin.to_str().expect("origin path")],
    );
    git(
        &repo,
        &["remote", "add", "origin", origin.to_str().expect("origin path")],
    );
    git(&repo, &["push", "origin", "main"]);
}
