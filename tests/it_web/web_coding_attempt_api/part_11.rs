#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn group_then_single_creation_is_serialized_by_work_item_guard() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let group_state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let group_app = build_web_router(group_state.clone());
    bootstrap_confirmed_work_item_plan_group(group_app.clone(), repo.path()).await;
    let single_app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let group_pause = group_state
        .test_controls
        .pause_next_group_attempt_after_worktree_acquire();

    let mut group_request = tokio::spawn(async move {
        request_json(
            group_app,
            Method::POST,
            "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
            json!({}),
        )
        .await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        group_pause.wait_until_paused(),
    )
    .await
    .expect("group request did not pause after worktree acquire");

    let mut single_request = tokio::spawn(async move {
        request_json(
            single_app,
            Method::POST,
            "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
            json!({}),
        )
        .await
    });
    let early_single = tokio::time::timeout(
        std::time::Duration::from_millis(150),
        &mut single_request,
    )
    .await;
    if let Ok(result) = early_single {
        group_pause.resume();
        panic!("single request bypassed paused group creation: {result:?}");
    }

    group_pause.resume();
    let (group_status, group) = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        &mut group_request,
    )
    .await
    .expect("group request did not finish")
    .expect("group request task");
    let (single_status, single) = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        &mut single_request,
    )
    .await
    .expect("single request did not finish")
    .expect("single request task");

    assert_eq!(group_status, StatusCode::OK, "{group}");
    assert_eq!(single_status, StatusCode::CONFLICT, "{single}");
    assert_eq!(single["code"], "coding_attempt_active");
    let winner_id = assert_global_attempt_id(&group);
    assert_creation_winner_state(root.path(), &winner_id, true);
}

#[tokio::test]
async fn schema_v2_single_creation_requires_group_without_persisting_attempt() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    bootstrap_confirmed_work_item_plan_group(app.clone(), repo.path()).await;

    let (single_status, single) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-items/work_item_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(single_status, StatusCode::BAD_REQUEST, "{single}");
    assert_eq!(single["code"], "schema_v2_group_coding_required");

    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    assert!(
        CodingAttemptStore::new(app_paths)
            .list_attempts_for_work_item("project_0001", "issue_0001", "work_item_0001")
            .expect("list attempts after rejected single request")
            .is_empty(),
        "a rejected Schema v2 single request must not create an attempt"
    );

    let (group_status, group) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts",
        json!({}),
    )
    .await;
    assert_eq!(group_status, StatusCode::OK, "{group}");
    let winner_id = assert_global_attempt_id(&group);
    assert_creation_winner_state(root.path(), &winner_id, true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn group_retry_and_delete_are_serialized_by_initialization_arbitration() {
    use cadence_aria::product::coding_attempt_store::CodingGroupInitializationPhase;
    use cadence_aria::web::test_controls::GroupAttemptInitializationCheckpoint;

    let root = tempdir().expect("root");
    let repo = git_repo();
    let interrupted_state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    interrupted_state
        .test_controls
        .fail_next_group_attempt_initialization_at(
            GroupAttemptInitializationCheckpoint::PersistedBeforeBind,
        );
    let interrupted_app = build_web_router(interrupted_state);
    bootstrap_confirmed_work_item_plan_group(interrupted_app.clone(), repo.path()).await;
    let create_path = "/api/projects/project_0001/issues/issue_0001/work-item-plans/work_item_plan_0001/coding-attempts";
    let (interrupted_status, interrupted) =
        request_json(interrupted_app, Method::POST, create_path, json!({})).await;
    assert_eq!(interrupted_status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        interrupted["code"],
        "coding_group_initialization_interrupted"
    );

    let app_paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = CodingAttemptStore::new(app_paths.clone());
    let journal = store
        .get_group_initialization("project_0001", "issue_0001", "work_item_plan_0001")
        .expect("attempt persisted journal");
    assert_eq!(
        journal.phase,
        CodingGroupInitializationPhase::AttemptPersisted
    );
    let attempt_id = journal.attempt.id.clone();

    let replay_state = WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    );
    let replay_pause = replay_state
        .test_controls
        .pause_next_group_attempt_after_worktree_acquire();
    let replay_app = build_web_router(replay_state);
    let mut replay_request = tokio::spawn(async move {
        request_json(replay_app, Method::POST, create_path, json!({})).await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(3),
        replay_pause.wait_until_paused(),
    )
    .await
    .expect("group replay did not pause while holding initialization arbitration");

    let delete_app = build_web_router(WebAppState::new(
        root.path().to_path_buf(),
        WebRuntime::new_fake(root.path().to_path_buf()),
    ));
    let delete_path = scoped_attempt_uri(&attempt_id, "");
    let mut delete_request = tokio::spawn(async move {
        request_json(delete_app, Method::DELETE, &delete_path, json!({})).await
    });
    let early_delete = tokio::time::timeout(
        std::time::Duration::from_millis(250),
        &mut delete_request,
    )
    .await;
    if let Ok(result) = early_delete {
        replay_pause.resume();
        panic!("delete bypassed paused group replay: {result:?}");
    }

    replay_pause.resume();
    let (replay_status, replay) = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        &mut replay_request,
    )
    .await
    .expect("group replay did not finish")
    .expect("group replay task");
    assert_eq!(replay_status, StatusCode::OK, "{replay}");
    assert_eq!(replay["attempt_id"], attempt_id);

    let (delete_status, delete) = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        &mut delete_request,
    )
    .await
    .expect("group delete did not finish")
    .expect("group delete task");
    assert_eq!(delete_status, StatusCode::NO_CONTENT, "{delete}");

    assert!(
        store
            .get_attempt("project_0001", "issue_0001", &attempt_id)
            .is_err()
    );
    let attempts_root = root
        .path()
        .join(".aria/projects/project_0001/issues/issue_0001/coding-attempts");
    assert!(!attempts_root.join(format!("{attempt_id}.json")).exists());
    assert!(!attempts_root.join(&attempt_id).exists());
    assert!(
        !attempts_root
            .join("group-initializations/work_item_plan_0001.json")
            .exists()
    );
    let shared = LifecycleStore::new(app_paths)
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("shared worktree lookup after delete");
    // DELETE 后该 issue 无其他 attempt 记录时，shared-worktree.json 一并被条件清理
    // （spec `harden-coding-attempt-deletion`）。无论是「json 已删」还是「json 保留但
    // lock 已释放」，都视为删除路径正确收敛。
    match shared {
        None => {}
        Some(record) => {
            assert_eq!(record.current_active_work_item_id, None);
            assert_eq!(record.current_lock_owner_id, None);
        }
    }
}

fn assert_creation_winner_state(
    root: &std::path::Path,
    winner_attempt_id: &str,
    expect_group_journal: bool,
) {
    let app_paths = ProductAppPaths::new(root.join(".aria"));
    let store = CodingAttemptStore::new(app_paths.clone());
    let active_attempts = store
        .list_attempts_for_work_item("project_0001", "issue_0001", "work_item_0001")
        .expect("attempts")
        .into_iter()
        .filter(|attempt| attempt.status.is_active())
        .collect::<Vec<_>>();
    assert_eq!(active_attempts.len(), 1);
    assert_eq!(active_attempts[0].id, winner_attempt_id);

    let attempts_root = root
        .join(".aria/projects/project_0001/issues/issue_0001/coding-attempts");
    let provider_configs = fs::read_dir(&attempts_root)
        .expect("coding attempts root")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().join("role-provider-config.json").is_file())
        .count();
    assert_eq!(provider_configs, 1, "only the winner may persist provider config");

    let journal_path = attempts_root
        .join("group-initializations/work_item_plan_0001.json");
    assert_eq!(journal_path.is_file(), expect_group_journal);
    if expect_group_journal {
        let journal = store
            .get_group_initialization(
                "project_0001",
                "issue_0001",
                "work_item_plan_0001",
            )
            .expect("group initialization journal");
        assert_eq!(journal.attempt.id, winner_attempt_id);
    }

    let shared = LifecycleStore::new(app_paths)
        .get_issue_shared_worktree("project_0001", "issue_0001")
        .expect("issue shared worktree")
        .expect("issue shared worktree");
    assert_eq!(
        shared.current_active_work_item_id.as_deref(),
        Some("work_item_0001")
    );
    assert_eq!(
        shared.current_lock_owner_id.as_deref(),
        Some(winner_attempt_id)
    );
}
