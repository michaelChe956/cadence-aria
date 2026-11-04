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

