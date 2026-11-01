// Task 13: 逻辑代码库 fail-closed 入口×状态回归矩阵（it_web 端到端）。
// 目的：锁住各 Web 入口在 manifest/selection/member 不一致状态下的 fail-closed 行为
// （稳定错误码 repository_routing_* + 4xx），防止未来回归静默降级物理仓库。
//
// 产品行为已在 Task 1-11 实现，这些测试是回归保护（写即 PASS，锁住正确行为）。
//
// 矩阵覆盖说明：
//   - 入口=group 创建是最直接的 fail-closed HTTP 入口：routing 在到达 identity/revision
//     解析链之前就经 RepositoryRouting::classify 三态判定，因此 (Some,None) / (None,Some)
//     等不一致状态可在 it_web 直接断言稳定错误码，无需完整 identity registry fixture。
//   - 多 target 无唯一（repository_routing_ambiguous）已由 part_01.rs
//     create_group_attempt_multi_target_non_unique_is_ambiguous_4xx 覆盖，此处不重复。
//   - 单成员 (Some,Some) 走 authority 成功路径、target 不在 selection、selection invalidated
//     等需完整 identity registry + 物理 checkout + revision 链（见 batch_accept_all_runs
//     的 100+ 行 setup），其 classify/resolve 行为由 lib 单测覆盖：
//       * repository_routing::tests（classify 三态 + FailClosed 稳定错误码）
//       * workspace_repository 的 routing_error/routing_error_for_target_error
//       * group.rs routing_api_error 的 code 映射
//     it_web 端到端完整 logical 成功链路属后续 Plan（WP5b 多仓执行层）范围。

#[tokio::test]
async fn logical_group_create_manifest_without_selection_is_fail_closed_4xx() {
    // 入口=group 创建 × 状态=(Some,None) 有 manifest 无 selection
    // → RepositoryRouting::classify 返回 FailClosed(TargetMissing)
    // → handler 返回稳定错误码 repository_routing_target_missing + 4xx，不静默降级。
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    // 叠加 manifest 但故意不写 selection → (Some, None)
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let aggregate_root = app_paths.root().join("aggregate-root");
    LogicalCodebaseStore::new(app_paths)
        .save_manifest(
            "project_0001",
            &LogicalCodebaseManifest::new(
                "project_0001",
                aggregate_root,
                vec![LogicalRepositoryId(uuid::Uuid::from_u128(1))],
            ),
        )
        .expect("manifest without selection");

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert!(
        status.is_client_error(),
        "(Some,None) manifest-without-selection must fail-closed 4xx: {body}"
    );
    assert_eq!(body["code"], "repository_routing_target_missing");
}

#[tokio::test]
async fn logical_group_create_orphaned_selection_is_fail_closed_4xx() {
    // 入口=group 创建 × 状态=(None,Some) 无 manifest 有 selection（孤立 selection/数据损坏）
    // → RepositoryRouting::classify 返回 FailClosed(OrphanedSelection)
    // → handler 映射为稳定错误码 repository_routing_inconsistent + 4xx，不静默降级。
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;
    // 故意只写 selection 不写 manifest → (None, Some)
    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    IssueCodebaseSelectionStore::new(app_paths)
        .save(&IssueCodebaseSelection::explicit(
            "project_0001",
            "issue_0001",
            vec![LogicalRepositoryId(uuid::Uuid::from_u128(1))],
            Vec::new(),
            vec![LogicalRepositoryId(uuid::Uuid::from_u128(1))],
            None,
        ))
        .expect("orphaned selection without manifest");

    let (status, body) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;

    assert!(
        status.is_client_error(),
        "(None,Some) orphaned-selection must fail-closed 4xx: {body}"
    );
    assert_eq!(body["code"], "repository_routing_inconsistent");
}
