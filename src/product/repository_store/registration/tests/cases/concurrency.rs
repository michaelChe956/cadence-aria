#[tokio::test]
async fn repository_registration_allows_only_one_same_path_initialization_and_releases_lock() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("repository");
    std::fs::create_dir_all(&root).unwrap();
    let repositories = Arc::new(ConfigRepositoryPersistence::new(vec![], false));
    let entered = Arc::new(Semaphore::new(0));
    let release = Arc::new(Semaphore::new(0));
    let cadence = Arc::new(BlockingCadence {
        entered: entered.clone(),
        release: release.clone(),
        count: AtomicUsize::new(0),
        source_root: temp.path().join("cadence"),
    });
    let coordinator = Arc::new(coordinator(
        Arc::new(ConfigProjectLookup { exists: true }),
        repositories.clone(),
        Arc::new(ProviderAvailabilityGate::new(Arc::new(AvailableHealth))),
        cadence.clone(),
        Arc::new(|| Ok(())),
        Arc::new(CwdRootRunner),
        Arc::new(StaticInitializer {
            fail: false,
            count: AtomicUsize::new(0),
        }),
    ));
    let first_launch = coordinator
        .begin_initialization(input(root.clone()), CancellationToken::new())
        .await
        .unwrap();
    let first_coordinator = coordinator.clone();
    let first = tokio::spawn(async move {
        first_coordinator
            .execute_initialization(first_launch, CancellationToken::new())
            .await
    });
    entered.acquire().await.unwrap().forget();

    let second = match coordinator
        .begin_initialization(input(root.clone()), CancellationToken::new())
        .await
    {
        Ok(_) => panic!("same-path initialization must be rejected while the first run is active"),
        Err(error) => error,
    };
    assert_eq!(second.reason_code, "repository_initialization_in_progress");
    assert_eq!(cadence.count.load(Ordering::SeqCst), 1);

    release.add_permits(1);
    first.await.unwrap().unwrap();
    assert_eq!(repositories.create_count.load(Ordering::SeqCst), 1);

    let third_launch = coordinator
        .begin_initialization(input(root), CancellationToken::new())
        .await
        .unwrap();
    let third_coordinator = coordinator.clone();
    let third = tokio::spawn(async move {
        third_coordinator
            .execute_initialization(third_launch, CancellationToken::new())
            .await
    });
    entered.acquire().await.unwrap().forget();
    release.add_permits(1);
    let third = third
        .await
        .unwrap()
        .unwrap()
        .result
        .expect("completed operation result");
    assert_eq!(third.repository.project_id, "project_0001");
    assert_eq!(repositories.create_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn repository_registration_allows_different_paths_to_enter_in_parallel() {
    let temp = TempDir::new().unwrap();
    let first_root = temp.path().join("first");
    let second_root = temp.path().join("second");
    std::fs::create_dir_all(&first_root).unwrap();
    std::fs::create_dir_all(&second_root).unwrap();
    let repositories = Arc::new(ConfigRepositoryPersistence::new(vec![], false));
    let entered = Arc::new(Semaphore::new(0));
    let release = Arc::new(Semaphore::new(0));
    let cadence = Arc::new(BlockingCadence {
        entered: entered.clone(),
        release: release.clone(),
        count: AtomicUsize::new(0),
        source_root: temp.path().join("cadence"),
    });
    let coordinator = Arc::new(coordinator(
        Arc::new(ConfigProjectLookup { exists: true }),
        repositories.clone(),
        Arc::new(ProviderAvailabilityGate::new(Arc::new(AvailableHealth))),
        cadence,
        Arc::new(|| Ok(())),
        Arc::new(CwdRootRunner),
        Arc::new(StaticInitializer {
            fail: false,
            count: AtomicUsize::new(0),
        }),
    ));
    let first_launch = coordinator
        .begin_initialization(input(first_root), CancellationToken::new())
        .await
        .unwrap();
    let second_launch = coordinator
        .begin_initialization(input(second_root), CancellationToken::new())
        .await
        .unwrap();
    let first = {
        let coordinator = coordinator.clone();
        tokio::spawn(async move {
            coordinator
                .execute_initialization(first_launch, CancellationToken::new())
                .await
        })
    };
    let second = {
        let coordinator = coordinator.clone();
        tokio::spawn(async move {
            coordinator
                .execute_initialization(second_launch, CancellationToken::new())
                .await
        })
    };

    entered.acquire_many(2).await.unwrap().forget();
    release.add_permits(2);
    first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
    assert_eq!(repositories.create_count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn repository_registration_releases_same_path_lock_after_failure() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("repository");
    std::fs::create_dir_all(&root).unwrap();
    let repositories = Arc::new(ConfigRepositoryPersistence::new(vec![], false));
    let cadence = Arc::new(FailOnceCadence {
        count: AtomicUsize::new(0),
        source_root: temp.path().join("cadence"),
    });
    let coordinator = coordinator(
        Arc::new(ConfigProjectLookup { exists: true }),
        repositories.clone(),
        Arc::new(ProviderAvailabilityGate::new(Arc::new(AvailableHealth))),
        cadence,
        Arc::new(|| Ok(())),
        Arc::new(CwdRootRunner),
        Arc::new(StaticInitializer {
            fail: false,
            count: AtomicUsize::new(0),
        }),
    );

    let first = coordinator
        .register(input(root.clone()), CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(first.reason_code, "cadence_skills_unavailable");

    let second = coordinator
        .register(input(root), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(second.repository.project_id, "project_0001");
    assert_eq!(repositories.create_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn repository_registration_reports_git_initializer_and_persist_failures_without_early_create()
{
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("repository");
    std::fs::create_dir_all(&root).unwrap();

    let before_repositories = Arc::new(ConfigRepositoryPersistence::new(vec![], false));
    let before_initializer = Arc::new(StaticInitializer {
        fail: false,
        count: AtomicUsize::new(0),
    });
    let before_operations = RepositoryInitializationOperationStore::new(ProductAppPaths::new(
        temp.path().join("before-operations"),
    ));
    let before_coordinator = coordinator_with_operations(
        Arc::new(ConfigProjectLookup { exists: true }),
        before_repositories.clone(),
        before_operations.clone(),
        Arc::new(ProviderAvailabilityGate::new(Arc::new(AvailableHealth))),
        Arc::new(StaticCadence {
            failure: None,
            count: AtomicUsize::new(0),
            source_root: temp.path().join("cadence-before"),
        }),
        Arc::new(|| Ok(())),
        Arc::new(ConfigRunner {
            root: root.clone(),
            rev_parse: Mutex::new(None),
            statuses: Mutex::new(vec![command_result(Some(1), "", "status failed")].into()),
            call_count: AtomicUsize::new(0),
        }),
        before_initializer.clone(),
    );
    let before_launch = before_coordinator
        .begin_initialization(input(root.clone()), CancellationToken::new())
        .await
        .unwrap();
    let before_operation_id = before_launch.operation_id().to_string();
    let before = before_coordinator
        .execute_initialization(before_launch, CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(before.reason_code, "repository_git_state_failed");
    assert_eq!(before_initializer.count.load(Ordering::SeqCst), 0);
    assert_eq!(before_repositories.create_count.load(Ordering::SeqCst), 0);
    let before_operation = before_operations
        .get("project_0001", &before_operation_id)
        .unwrap();
    assert_eq!(
        before_operation.status,
        RepositoryInitializationOperationStatus::Failed
    );
    assert_eq!(
        before_operation.failed_step,
        Some(RepositoryInitializationStepKind::RuleConfig)
    );
    assert_eq!(
        before_operation.steps[1].status,
        RepositoryInitializationStepStatus::Failed
    );
    assert_eq!(
        before_operation
            .error
            .as_ref()
            .map(|error| error.reason_code.as_str()),
        Some("repository_git_state_failed")
    );

    let init_repositories = Arc::new(ConfigRepositoryPersistence::new(vec![], false));
    let init = coordinator(
        Arc::new(ConfigProjectLookup { exists: true }),
        init_repositories.clone(),
        Arc::new(ProviderAvailabilityGate::new(Arc::new(AvailableHealth))),
        Arc::new(StaticCadence {
            failure: None,
            count: AtomicUsize::new(0),
            source_root: temp.path().join("cadence-init"),
        }),
        Arc::new(|| Ok(())),
        Arc::new(ConfigRunner {
            root: root.clone(),
            rev_parse: Mutex::new(None),
            statuses: Mutex::new(
                vec![
                    command_result(Some(0), "", ""),
                    command_result(Some(0), "?? generated.txt\0", ""),
                ]
                .into(),
            ),
            call_count: AtomicUsize::new(0),
        }),
        Arc::new(StaticInitializer {
            fail: true,
            count: AtomicUsize::new(0),
        }),
    )
    .register(input(root.clone()), CancellationToken::new())
    .await
    .unwrap_err();
    assert_eq!(init.reason_code, "repository_init_command_failed");
    assert_eq!(init.changed_paths, Some(vec!["generated.txt".to_string()]));
    assert_eq!(init_repositories.create_count.load(Ordering::SeqCst), 0);

    let after_repositories = Arc::new(ConfigRepositoryPersistence::new(vec![], false));
    let after_operations = RepositoryInitializationOperationStore::new(ProductAppPaths::new(
        temp.path().join("after-operations"),
    ));
    let after_coordinator = coordinator_with_operations(
        Arc::new(ConfigProjectLookup { exists: true }),
        after_repositories.clone(),
        after_operations.clone(),
        Arc::new(ProviderAvailabilityGate::new(Arc::new(AvailableHealth))),
        Arc::new(StaticCadence {
            failure: None,
            count: AtomicUsize::new(0),
            source_root: temp.path().join("cadence-after"),
        }),
        Arc::new(|| Ok(())),
        Arc::new(ConfigRunner {
            root: root.clone(),
            rev_parse: Mutex::new(None),
            statuses: Mutex::new(
                vec![
                    command_result(Some(0), "", ""),
                    command_result(Some(1), "", "final status failed"),
                ]
                .into(),
            ),
            call_count: AtomicUsize::new(0),
        }),
        Arc::new(StaticInitializer {
            fail: false,
            count: AtomicUsize::new(0),
        }),
    );
    let after_launch = after_coordinator
        .begin_initialization(input(root.clone()), CancellationToken::new())
        .await
        .unwrap();
    let after_operation_id = after_launch.operation_id().to_string();
    let after = after_coordinator
        .execute_initialization(after_launch, CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(after.reason_code, "repository_git_state_failed");
    assert_eq!(after.changed_paths, None);
    assert!(after.action.contains("inspect the repository manually"));
    assert_eq!(after_repositories.create_count.load(Ordering::SeqCst), 0);
    let after_operation = after_operations
        .get("project_0001", &after_operation_id)
        .unwrap();
    assert_eq!(
        after_operation.status,
        RepositoryInitializationOperationStatus::Failed
    );
    assert_eq!(after_operation.failed_step, None);
    assert!(
        after_operation
            .steps
            .iter()
            .all(|step| step.status == RepositoryInitializationStepStatus::Completed)
    );
    assert_eq!(
        after_operation
            .error
            .as_ref()
            .map(|error| error.reason_code.as_str()),
        Some("repository_git_state_failed")
    );

    let persist_repositories = Arc::new(ConfigRepositoryPersistence::new(vec![], true));
    let persist = coordinator(
        Arc::new(ConfigProjectLookup { exists: true }),
        persist_repositories.clone(),
        Arc::new(ProviderAvailabilityGate::new(Arc::new(AvailableHealth))),
        Arc::new(StaticCadence {
            failure: None,
            count: AtomicUsize::new(0),
            source_root: temp.path().join("cadence-persist"),
        }),
        Arc::new(|| Ok(())),
        Arc::new(ConfigRunner {
            root: root.clone(),
            rev_parse: Mutex::new(None),
            statuses: Mutex::new(
                vec![
                    command_result(Some(0), "", ""),
                    command_result(Some(0), "?? z.txt\0?? a.txt\0?? z.txt\0", ""),
                ]
                .into(),
            ),
            call_count: AtomicUsize::new(0),
        }),
        Arc::new(StaticInitializer {
            fail: false,
            count: AtomicUsize::new(0),
        }),
    )
    .register(input(root), CancellationToken::new())
    .await
    .unwrap_err();
    assert_eq!(persist.reason_code, "repository_persist_failed");
    assert_eq!(
        persist.changed_paths,
        Some(vec!["a.txt".to_string(), "z.txt".to_string()])
    );
    assert_eq!(persist_repositories.create_count.load(Ordering::SeqCst), 1);
}
