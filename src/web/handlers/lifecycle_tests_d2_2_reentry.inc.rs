// 经 lifecycle_tests.inc.rs 的 tests mod `include!` 引入（large_file_guard
// 1200 行红线拆分，纯移动自 lifecycle_tests_d2_2.inc.rs 1053-1370 行，逐字不变）。
/// 失败重入与根/pending 缺失后的 scope 反查（REQ-DEL-07/02）：子依赖 failpoint 中断
/// 保留重试入口；重入续清；根删后 complete 前中断凭固定来源收口；Deleted 幂等。
#[tokio::test]
async fn cascade_d2_2_failure_reentry() {
    // A) 子依赖损坏 JSON → 删除失败上抛；墓碑+pending 落盘、根与未清子依赖保留；
    //    修复后重入一次收口、恰一条记录。
    {
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
            vec!["wi-a-1".to_string()],
            crate::product::models::IssueWorkItemPlanStatus::Draft,
        );
        d2_2_wi(&lifecycle, "wi-a-1", D2_2_PLAN_A);
        let plan_session = d2_2_session(&lifecycle, D2_2_PLAN_A, WorkspaceType::WorkItemPlan);
        let review = d2_2_review_for_session(&lifecycle, &plan_session.id);
        d2_2_advance_closure(&paths, D2_2_PLAN_A);
        let review_dir = paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("provider-review-rounds");
        let review_path = review_dir.join(format!("{}.json", review.id));
        // failpoint：review 目录置只读——采集（只读）可过、purge ①（删除）必败，
        // 真实 IO 错误在墓碑之后上抛（不当空、不假记完成）。
        std::fs::set_permissions(
            &review_dir,
            std::os::unix::fs::PermissionsExt::from_mode(0o555),
        )
        .unwrap();

        let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
        assert_ne!(status, StatusCode::OK, "failpoint 必须上抛: {body}");
        let scope = EntityScope::new(
            PROJECT_ID,
            ISSUE_ID,
            crate::product::entity_deletion_store::DeletionEntityKind::Plan,
            D2_2_PLAN_A,
        );
        let deletion_store = EntityDeletionStore::new(paths.clone());
        let record = deletion_store.get(&scope).unwrap().expect("墓碑在位");
        assert_eq!(record.phase, DeletionPhase::Purging);
        assert!(
            d2_2_pending_path(&paths, "plan", D2_2_PLAN_A).exists(),
            "pending manifest 落盘（重入入口）"
        );
        assert!(
            lifecycle
                .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, D2_2_PLAN_A)
                .is_ok(),
            "根最后：失败时根保留"
        );
        assert!(
            paths
                .issue_root(PROJECT_ID, ISSUE_ID)
                .join("work-items")
                .join("wi-a-1.json")
                .exists(),
            "未清到的子依赖保留"
        );

        // 修复 failpoint 后重入：pending+owner 索引续清，一次收口。
        std::fs::set_permissions(
            &review_dir,
            std::os::unix::fs::PermissionsExt::from_mode(0o755),
        )
        .unwrap();
        let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
        assert_eq!(status, StatusCode::OK, "重入收口: {body}");
        assert!(
            !review_path.exists()
                && !paths
                    .issue_root(PROJECT_ID, ISSUE_ID)
                    .join("work-items")
                    .join("wi-a-1.json")
                    .exists()
                && !lifecycle
                    .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, D2_2_PLAN_A)
                    .is_ok(),
            "重入清完整闭包"
        );
        let record: EntityDeletionRecord = crate::product::json_store::read_json(
            &d2_2_deletion_dir(&paths, "plan").join(format!("{D2_2_PLAN_A}.json")),
        )
        .unwrap();
        assert_eq!(record.phase, DeletionPhase::Deleted);
        assert!(
            !d2_2_pending_path(&paths, "plan", D2_2_PLAN_A).exists(),
            "成功后 pending 移除"
        );
        let records: Vec<_> = std::fs::read_dir(d2_2_deletion_dir(&paths, "plan"))
            .unwrap()
            .flatten()
            .collect();
        assert_eq!(records.len(), 1, "重试不重复留痕");
    }

    // B) 根已删、pending 已移除、complete 前中断（模拟崩溃）→ 重入凭 §0.5 固定来源
    //    反查收敛，根缺不 404；Deleted 幂等 200。
    {
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
            vec!["wi-a-1".to_string()],
            crate::product::models::IssueWorkItemPlanStatus::Draft,
        );
        d2_2_wi(&lifecycle, "wi-a-1", D2_2_PLAN_A);
        let (status, _) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
        assert_eq!(status, StatusCode::OK);

        // 模拟 complete 前崩溃：记录回置 purging（pending 已移除、根已删）。
        let record_path = d2_2_deletion_dir(&paths, "plan").join(format!("{D2_2_PLAN_A}.json"));
        let mut record: EntityDeletionRecord =
            crate::product::json_store::read_json(&record_path).unwrap();
        record.phase = DeletionPhase::Purging;
        record.deleted_at = None;
        crate::product::json_store::write_json(&record_path, &record).unwrap();

        let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
        assert_eq!(status, StatusCode::OK, "根缺不 404，重入收口: {body}");
        let record: EntityDeletionRecord =
            crate::product::json_store::read_json(&record_path).unwrap();
        assert_eq!(record.phase, DeletionPhase::Deleted, "完成单条事实");
        assert!(record.deleted_at.is_some());

        // Deleted 幂等：重复请求 200、记录不再变化。
        let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
        assert_eq!(status, StatusCode::OK, "Deleted 幂等: {body}");
        assert_eq!(body["status"], "deleted");
        let after: EntityDeletionRecord =
            crate::product::json_store::read_json(&record_path).unwrap();
        assert_eq!(after, record, "幂等重试不新增记录、不改完成时间");
    }

    // C) Story 闭包：被 Design 引用拒删 → 先删后置 → 上游放行全清（REQ-DEL-01/03）。
    {
        let root = TempDir::new().expect("root");
        let app = build_test_router(root.path());
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let lifecycle = d2_2_seed_project_issue(&paths);
        let story = lifecycle
            .create_story_spec(CreateStorySpecInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: REPOSITORY_ID.to_string(),
                title: "上游 story".to_string(),
                aggregate_codebase: None,
            })
            .unwrap();
        let design = lifecycle
            .create_design_spec(CreateDesignSpecInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                story_spec_ids: vec![story.id.clone()],
                title: "后置 design".to_string(),
                aggregate_codebase: None,
            })
            .unwrap();
        let story_session = d2_2_session(&lifecycle, &story.id, WorkspaceType::Story);
        let story_review = d2_2_review_for_session(&lifecycle, &story_session.id);
        lifecycle
            .append_version(crate::product::lifecycle_store::AppendSpecVersionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: story.id.clone(),
                markdown: "# story v1".to_string(),
                provider_run_refs: Vec::new(),
                review_refs: Vec::new(),
                confirmed_by: None,
            })
            .unwrap();

        let (status, body) = d2_2_delete(&app, "story-specs", &story.id, None).await;
        assert_eq!(status, StatusCode::CONFLICT, "story body: {body}");
        assert_eq!(body["code"], "entity_referenced");
        assert_eq!(body["details"]["references"][0]["kind"], "design_spec");

        let (status, _) = d2_2_delete(&app, "design-specs", &design.id, None).await;
        assert_eq!(status, StatusCode::OK, "先删后置 design");
        let (status, body) = d2_2_delete(&app, "story-specs", &story.id, None).await;
        assert_eq!(status, StatusCode::OK, "后删上游: {body}");

        assert!(
            !paths
                .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
                .join("story-specs")
                .join(format!("{}.json", story.id))
                .exists()
                && !paths
                    .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
                    .join("design-specs")
                    .join(format!("{}.json", design.id))
                    .exists()
        );
        assert!(
            !paths
                .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
                .join("versions")
                .join(&story.id)
                .exists(),
            "story versions 全清"
        );
        let sessions_root = paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("workspace-sessions");
        assert!(
            !sessions_root
                .join(format!("{}.json", story_session.id))
                .exists()
        );
        assert!(
            !paths
                .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
                .join("provider-review-rounds")
                .join(format!("{}.json", story_review.id))
                .exists(),
            "story review 随闭包清"
        );
        let record: EntityDeletionRecord = crate::product::json_store::read_json(
            &d2_2_deletion_dir(&paths, "story").join(format!("{}.json", story.id)),
        )
        .unwrap();
        assert_eq!(record.phase, DeletionPhase::Deleted);
    }
}

fn d2_2_owned_index_path(paths: &ProductAppPaths, kind: &str, id: &str) -> std::path::PathBuf {
    paths
        .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
        .join("entity-operations")
        .join("owned-index")
        .join(kind)
        .join(format!("{id}.json"))
}

/// X 修复：cascade 完成后 owned-index 不再指向已删会话。Plan 的 owned-index
/// 只保留 worktree 墓碑 claim 事实（work_item_ids/target_repository_ids），
/// session/review/run 等已 purge 集合整段清空；身分绑定（D1.0 纯 ID 准入
/// 事实）不受影响，list-by-owner 仍可反查（墓碑语义保持）。
#[tokio::test]
async fn cascade_d2_2_owned_index_pruned_after_completion() {
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
        vec!["wi-x-prune-1".to_string()],
        crate::product::models::IssueWorkItemPlanStatus::Confirmed,
    );
    d2_2_wi(&lifecycle, "wi-x-prune-1", D2_2_PLAN_A);
    let plan_session = d2_2_session(&lifecycle, D2_2_PLAN_A, WorkspaceType::WorkItemPlan);
    // 显式 id 会话的等价现场：删除前已写身分绑定（D1.0 纯 ID 准入事实）。
    let scope = EntityScope::new(
        PROJECT_ID,
        ISSUE_ID,
        crate::product::entity_deletion_store::DeletionEntityKind::Plan,
        D2_2_PLAN_A,
    );
    EntityOperationStore::new(paths.clone())
        .bind_workspace_session_identity(&plan_session.id, &scope)
        .expect("bind session identity");


    let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, Some(true)).await;
    assert_eq!(status, StatusCode::OK, "delete body: {body}");

    // 墓碑已完成。
    let record: EntityDeletionRecord = crate::product::json_store::read_json(
        &d2_2_deletion_dir(&paths, "plan").join(format!("{D2_2_PLAN_A}.json")),
    )
    .unwrap();
    assert_eq!(record.phase, DeletionPhase::Deleted);

    // owned-index：不指向已删会话（红：当前残留 session_ids/review_ids）。
    let owned_index = d2_2_owned_index_path(&paths, "plan", D2_2_PLAN_A);
    let indexed: crate::product::entity_deletion_store::EntityDeletionManifest =
        crate::product::json_store::read_json(&owned_index).unwrap();
    assert!(
        indexed.session_ids.is_empty(),
        "完成后 owned-index 不得指向已删会话: {:?}",
        indexed.session_ids
    );
    assert!(indexed.review_ids.is_empty());
    assert!(indexed.provider_run_ids.is_empty());
    // 墓碑 claim 事实保留：WI 反查/资源 claim 不受清理影响。
    assert_eq!(indexed.work_item_ids, vec!["wi-x-prune-1".to_string()]);

    // 身分绑定是长期准入事实：list-by-owner 仍反查得到已删会话（语义保持），
    // 但 owned-index 读取方（list_owned_artifact_ids）不再返回已删会话。
    let operations = EntityOperationStore::new(paths.clone());
    assert!(operations
        .list_workspace_session_ids_by_owner(&scope)
        .unwrap()
        .contains(&plan_session.id));
    assert!(EntityDeletionStore::new(paths.clone())
        .list_owned_artifact_ids(&scope)
        .unwrap()
        .session_ids
        .is_empty());
}

