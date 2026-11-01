// REQ-WPS-01/REQ-WPS-02（fix-cascade-deletion W 包）：单 plan 不变量 ——
// 重新 prepare 不产生第二个 plan；旧 plan 先删后建，且只留一条无内容墓碑。

/// 重新 prepare：旧 plan 先被完整删除，新 plan 建立后存活 plan 恰为 1。
#[tokio::test]
async fn reprepare_deletes_old_plan_and_keeps_single_live_plan() {
    let root = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let (lifecycle, story_spec_id, design_spec_id) = seed_single_physical_prepare_work_item_plan_fixture(&paths);
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state);

    let first = post_prepare_work_item_plan(&app, story_spec_id.clone(), design_spec_id.clone()).await;
    let first_status = first.status();
    let first_body = String::from_utf8_lossy(
        &axum::body::to_bytes(first.into_body(), usize::MAX)
            .await
            .expect("body"),
    )
    .to_string();
    assert_eq!(first_status, StatusCode::OK, "body={first_body}");
    let plans_after_first = lifecycle
        .list_issue_work_item_plans(PROJECT_ID, ISSUE_ID)
        .unwrap();
    assert_eq!(plans_after_first.len(), 1, "首次 prepare 后恰一个存活 plan");
    let first_plan_id = plans_after_first[0].id.clone();

    // 再次 prepare：旧 plan 必须先删（单 plan 不变量），新代不复用旧 id。
    let second = post_prepare_work_item_plan(&app, story_spec_id, design_spec_id).await;
    assert_eq!(second.status(), StatusCode::OK);
    let plans_after_second = lifecycle
        .list_issue_work_item_plans(PROJECT_ID, ISSUE_ID)
        .unwrap();
    assert_eq!(
        plans_after_second.len(),
        1,
        "同一 issue 存活 plan 必须恒为 1（REQ-WPS-01）"
    );
    assert_ne!(
        plans_after_second[0].id, first_plan_id,
        "新代 plan id 不得复用旧 plan id"
    );

    // 旧 plan 只留一条无内容删除事实（含触发原因，REQ-WPS-07）。
    let scope = crate::product::entity_deletion_store::EntityScope::new(
        PROJECT_ID,
        ISSUE_ID,
        crate::product::entity_deletion_store::DeletionEntityKind::Plan,
        &first_plan_id,
    );
    let tombstone = crate::product::entity_deletion_store::EntityDeletionStore::new(paths.clone())
        .get(&scope)
        .unwrap()
        .expect("旧 plan 删除事实");
    assert_eq!(
        tombstone.phase,
        crate::product::entity_deletion_store::DeletionPhase::Deleted
    );
    assert_eq!(tombstone.trigger_reason.as_deref(), Some("reprepare"));
    assert!(tombstone.deleted_at.is_some());
}

/// 旧 plan 的会话目录不得残留（墓碑不冒充存活 plan）。
#[tokio::test]
async fn reprepare_removes_old_plan_workspace_session() {
    let root = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let (lifecycle, story_spec_id, design_spec_id) = seed_single_physical_prepare_work_item_plan_fixture(&paths);
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state);

    let first = post_prepare_work_item_plan(&app, story_spec_id.clone(), design_spec_id.clone()).await;
    let first_status = first.status();
    let first_body = String::from_utf8_lossy(
        &axum::body::to_bytes(first.into_body(), usize::MAX)
            .await
            .expect("body"),
    )
    .to_string();
    assert_eq!(first_status, StatusCode::OK, "body={first_body}");
    let sessions_after_first = lifecycle
        .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
        .unwrap();
    assert_eq!(sessions_after_first.len(), 1);
    let first_session_id = sessions_after_first[0].id.clone();

    let second = post_prepare_work_item_plan(&app, story_spec_id, design_spec_id).await;
    assert_eq!(second.status(), StatusCode::OK);
    let sessions_after_second = lifecycle
        .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
        .unwrap();
    assert_eq!(sessions_after_second.len(), 1, "旧会话必须随旧 plan 删除");
    assert_ne!(sessions_after_second[0].id, first_session_id);
}

/// F4 回归：并发两次 prepare（同一 issue）→ 存活 plan 恒为 1。
///
/// 「删旧 + 建立」必须在同一 issue 级 plan-slot 互斥区内——否则两个并发
/// 触发可各建一个 plan（不变量复核发生在建立**之前**，捕获不到）。
#[tokio::test]
async fn concurrent_prepare_keeps_single_live_plan() {
    let root = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let (lifecycle, story_spec_id, design_spec_id) =
        seed_single_physical_prepare_work_item_plan_fixture(&paths);
    let state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let app = build_web_router(state);

    let (first, second) = tokio::join!(
        post_prepare_work_item_plan(&app, story_spec_id.clone(), design_spec_id.clone()),
        post_prepare_work_item_plan(&app, story_spec_id.clone(), design_spec_id.clone()),
    );
    let statuses = [first.status(), second.status()];
    assert!(
        statuses.contains(&StatusCode::OK),
        "并发 prepare 至少一次成功: {statuses:?}"
    );
    let plans = lifecycle
        .list_issue_work_item_plans(PROJECT_ID, ISSUE_ID)
        .unwrap();
    assert_eq!(
        plans.len(),
        1,
        "同一 issue 存活 plan 必须恒为 1（并发 prepare 不得各建一个 plan）"
    );
}
