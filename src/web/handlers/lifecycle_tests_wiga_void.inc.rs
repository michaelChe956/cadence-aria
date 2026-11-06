// REQ-WIGA-VOID-01（tasks 4.1/4.2）：删除工作项组 ⇒ 该 issue 的自动化登记
// 一并作废——经 lifecycle_tests.inc.rs 的 tests mod `include!` 引入。
//
// 覆盖：删除成功后作废（voided_at/void_reason、账本追加、登记文件保留）；
// 无登记幂等；绑定别的 plan 不作废（NotBoundToPlan）；作废失败不回滚已完成
// 的删除（仅 warn 留诊断事实）；删除失败不得作废（门禁拒绝 ⇒ voided_at 不写）。

/// ① 删除成功 ⇒ 绑定该 plan 的登记被标作废（在 ①.5 终止之上叠加作废
/// 维度，两维度并存可区分）；作废走锁内命令：账本追加一条、revision 再+1；
/// SHALL NOT 物理删除 `automation-enrollment.json`。
#[tokio::test]
async fn cascade_delete_plan_voids_bound_enrollment_with_ledger_entry() {
    let root = TempDir::new().expect("root");
    let app = build_test_router(root.path());
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = d2_2_seed_project_issue(&paths);
    d2_2_seed_plan(
        &lifecycle,
        D2_2_PLAN_A,
        Vec::new(),
        None,
        None,
        Vec::new(),
        crate::product::models::IssueWorkItemPlanStatus::Draft,
    );
    let seeded = f3_enable_automation(&paths, Some(D2_2_PLAN_A), f3_empty_source());

    let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
    assert_eq!(status, StatusCode::OK, "delete body: {body}");

    let enrollment = f3_read_enrollment(&paths);
    // ①.5 先终止（F3 既有语义不变），删除成功后作废——两维度并存。
    assert!(
        enrollment.terminated_at.is_some(),
        "F3 终止语义不变"
    );
    let voided_at = enrollment
        .voided_at
        .expect("删除成功后登记被标作废（REQ-WIGA-VOID-01）");
    assert!(!voided_at.is_empty());
    assert_eq!(
        enrollment.void_reason.as_deref(),
        Some("work_item_group_deleted")
    );
    // 终止 CAS +1、作废命令再 +1。
    assert_eq!(enrollment.policy_revision, seeded.policy_revision + 2);
    // 账本：终止条目（F3）之上追加一条作废条目（命令身份带被删 plan_id）。
    assert!(
        enrollment.command_ledger.iter().any(|entry| entry
            .command_id
            == format!("void-enrollment-{D2_2_PLAN_A}")),
        "作废条目入账本：{:?}",
        enrollment.command_ledger
    );
    // SHALL NOT 物理删除登记（留痕与历史保留）。
    assert!(
        paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("automation-enrollment.json")
            .exists()
    );
    assert!(
        lifecycle
            .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, D2_2_PLAN_A)
            .is_err(),
        "plan 记录随删除消失"
    );
}

/// ② 该 issue 无登记 ⇒ 删除成功且零副作用（NotFound 幂等——删 WIG 时没有
/// 登记是正常情况，不报错、不创建登记文件）。
#[tokio::test]
async fn cascade_delete_plan_without_enrollment_voids_nothing() {
    let root = TempDir::new().expect("root");
    let app = build_test_router(root.path());
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = d2_2_seed_project_issue(&paths);
    d2_2_seed_plan(
        &lifecycle,
        D2_2_PLAN_A,
        Vec::new(),
        None,
        None,
        Vec::new(),
        crate::product::models::IssueWorkItemPlanStatus::Draft,
    );

    let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
    assert_eq!(status, StatusCode::OK, "delete body: {body}");
    assert!(
        !paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("automation-enrollment.json")
            .exists(),
        "无登记时不得反向创建登记文件"
    );
}

/// ③ 登记绑定别的 plan ⇒ 不作废别人的（NotBoundToPlan 零副作用——该 issue
/// 的登记属于另一代工作项组）。
#[tokio::test]
async fn cascade_delete_plan_does_not_void_enrollment_bound_to_other_plan() {
    let root = TempDir::new().expect("root");
    let app = build_test_router(root.path());
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = d2_2_seed_project_issue(&paths);
    d2_2_seed_plan(
        &lifecycle,
        D2_2_PLAN_A,
        Vec::new(),
        None,
        None,
        Vec::new(),
        crate::product::models::IssueWorkItemPlanStatus::Draft,
    );
    let seeded = f3_enable_automation(&paths, Some("work_item_plan_other"), f3_empty_source());

    let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
    assert_eq!(status, StatusCode::OK, "delete body: {body}");

    let enrollment = f3_read_enrollment(&paths);
    assert!(
        enrollment.voided_at.is_none(),
        "不得作废绑定在别的 plan 上的登记"
    );
    assert_eq!(
        enrollment.policy_revision, seeded.policy_revision,
        "NotBoundToPlan 零副作用（未终止也未作废）"
    );
    assert!(enrollment.terminated_at.is_none());
}

/// ④ 作废失败（登记 JSON 损坏不可读）⇒ 删除仍已完成（作废失败 MUST NOT
/// 回滚已完成的删除）；失败以 warn 记录可诊断事实，不静默吞掉。
#[tokio::test]
async fn cascade_delete_plan_survives_void_failure_without_rollback() {
    let root = TempDir::new().expect("root");
    let app = build_test_router(root.path());
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = d2_2_seed_project_issue(&paths);
    d2_2_seed_plan(
        &lifecycle,
        D2_2_PLAN_A,
        Vec::new(),
        None,
        None,
        Vec::new(),
        crate::product::models::IssueWorkItemPlanStatus::Draft,
    );
    std::fs::write(
        paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("automation-enrollment.json"),
        "{ not valid enrollment json",
    )
    .expect("seed corrupt enrollment");

    let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "作废失败不得回滚已完成的删除：{body}"
    );
    assert!(
        lifecycle
            .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, D2_2_PLAN_A)
            .is_err(),
        "删除已完成"
    );
}

/// ⑤ 删除被门禁拒绝（coding workspace 存在 ⇒ 409）⇒ 登记不得作废——作废
/// 只在删除成功之后发生；①.5 的占用终止（F3 既有语义）不因本改动改变。
#[tokio::test]
async fn cascade_delete_plan_refused_by_gate_leaves_enrollment_unvoided() {
    let root = TempDir::new().expect("root");
    let app = build_test_router(root.path());
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = d2_2_seed_project_issue(&paths);
    d2_2_seed_plan(
        &lifecycle,
        D2_2_PLAN_A,
        Vec::new(),
        None,
        None,
        vec!["wi-void-1".to_string()],
        crate::product::models::IssueWorkItemPlanStatus::Draft,
    );
    d2_2_wi(&lifecycle, "wi-void-1", D2_2_PLAN_A);
    d2_2_seed_repository(&paths);
    f3_enable_automation(&paths, Some(D2_2_PLAN_A), f3_empty_source());
    let coding_store = CodingAttemptStore::new(paths.clone());
    coding_store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: "wi-void-1".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: None,
                review_rounds: 0,
                permission_modes: Default::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("seed attempt");

    let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "gate body: {body}");
    assert_eq!(body["code"], "coding_workspace_exists");

    let enrollment = f3_read_enrollment(&paths);
    assert!(
        enrollment.voided_at.is_none(),
        "删除失败 ⇒ 登记不得作废"
    );
}
