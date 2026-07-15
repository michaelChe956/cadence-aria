use super::*;

#[tokio::test]
async fn repository_registration_runs_in_order_and_persists_once_after_success() {
    let temp = TempDir::new().unwrap();
    let git_root = temp.path().join("repository");
    std::fs::create_dir_all(&git_root).unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let repositories = Arc::new(RecordingRepositoryPersistence {
        calls: calls.clone(),
        created: AtomicUsize::new(0),
    });
    let gate = Arc::new(ProviderAvailabilityGate::new(Arc::new(AvailableHealth)));
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, Arc::new(FakeStreamingProvider));
    let host_calls = calls.clone();
    let coordinator = RepositoryRegistrationCoordinator::new(
        Arc::new(RecordingProjectLookup {
            calls: calls.clone(),
        }),
        repositories.clone(),
        gate,
        Arc::new(registry),
        Arc::new(RecordingCadence {
            calls: calls.clone(),
            source_root: temp.path().join("cadence-source"),
        }),
        Arc::new(move || {
            host_calls.lock().unwrap().push("host_ready");
            Ok(())
        }),
        Arc::new(RecordingRunner {
            calls: calls.clone(),
            root: git_root.clone(),
            status_calls: AtomicUsize::new(0),
        }),
        Arc::new(|| "2026-07-13T01:02:03Z".to_string()),
        Arc::new(RecordingInitializer {
            calls: calls.clone(),
        }),
        Duration::from_secs(1),
        Duration::from_secs(2),
    );

    let result = coordinator
        .register(
            RepositoryRegistrationInput {
                project_id: "project_0001".to_string(),
                name: "Repository".to_string(),
                path: git_root.clone(),
                default_policy_preset: None,
                default_provider_mode: None,
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();

    assert_eq!(repositories.created.load(Ordering::SeqCst), 1);
    assert_eq!(result.repository.path, git_root);
    assert_eq!(result.initialization.provider, "claude_code");
    assert_eq!(result.initialization.source_mode, "offline");
    assert_eq!(result.initialization.commands.len(), 4);
    assert_eq!(result.warnings, vec!["offline source"]);
    assert_eq!(result.changed_paths, vec!["generated.txt"]);
    assert_eq!(result.completed_at, "2026-07-13T01:02:03Z");
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        &[
            "project_get",
            "git_root",
            "repository_find",
            "repository_find",
            "host_ready",
            "cadence_prepare",
            "git_status",
            "initializer",
            "git_status",
            "repository_create",
        ]
    );
}

#[tokio::test]
async fn repository_registration_rejects_project_path_git_and_duplicate_preconditions() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("repository");
    std::fs::create_dir_all(&root).unwrap();

    let missing_repositories = Arc::new(ConfigRepositoryPersistence::new(vec![], false));
    let missing = coordinator(
        Arc::new(ConfigProjectLookup { exists: false }),
        missing_repositories.clone(),
        Arc::new(ProviderAvailabilityGate::new(Arc::new(AvailableHealth))),
        Arc::new(StaticCadence {
            failure: None,
            count: AtomicUsize::new(0),
            source_root: temp.path().join("cadence"),
        }),
        Arc::new(|| Ok(())),
        Arc::new(CwdRootRunner),
        Arc::new(StaticInitializer {
            fail: false,
            count: AtomicUsize::new(0),
        }),
    )
    .register(input(root.clone()), CancellationToken::new())
    .await
    .unwrap_err();
    assert_eq!(missing.reason_code, "repository_project_not_found");
    assert_eq!(missing_repositories.create_count.load(Ordering::SeqCst), 0);

    let invalid_repositories = Arc::new(ConfigRepositoryPersistence::new(vec![], false));
    let invalid = coordinator(
        Arc::new(ConfigProjectLookup { exists: true }),
        invalid_repositories.clone(),
        Arc::new(ProviderAvailabilityGate::new(Arc::new(AvailableHealth))),
        Arc::new(StaticCadence {
            failure: None,
            count: AtomicUsize::new(0),
            source_root: temp.path().join("cadence"),
        }),
        Arc::new(|| Ok(())),
        Arc::new(CwdRootRunner),
        Arc::new(StaticInitializer {
            fail: false,
            count: AtomicUsize::new(0),
        }),
    )
    .register(
        input(temp.path().join("does-not-exist")),
        CancellationToken::new(),
    )
    .await
    .unwrap_err();
    assert_eq!(invalid.reason_code, "repository_path_invalid");
    assert_eq!(invalid_repositories.create_count.load(Ordering::SeqCst), 0);

    let not_git_repositories = Arc::new(ConfigRepositoryPersistence::new(vec![], false));
    let not_git_runner = Arc::new(ConfigRunner {
        root: root.clone(),
        rev_parse: Mutex::new(Some(command_result(Some(128), "", "not a git repository"))),
        statuses: Mutex::new(VecDeque::new()),
        call_count: AtomicUsize::new(0),
    });
    let not_git = coordinator(
        Arc::new(ConfigProjectLookup { exists: true }),
        not_git_repositories.clone(),
        Arc::new(ProviderAvailabilityGate::new(Arc::new(AvailableHealth))),
        Arc::new(StaticCadence {
            failure: None,
            count: AtomicUsize::new(0),
            source_root: temp.path().join("cadence"),
        }),
        Arc::new(|| Ok(())),
        not_git_runner,
        Arc::new(StaticInitializer {
            fail: false,
            count: AtomicUsize::new(0),
        }),
    )
    .register(input(root.clone()), CancellationToken::new())
    .await
    .unwrap_err();
    assert_eq!(not_git.reason_code, "repository_not_git");
    assert_eq!(not_git_repositories.create_count.load(Ordering::SeqCst), 0);

    let duplicate_record = repository_record("project_0001", root.clone());
    let duplicate_repositories = Arc::new(ConfigRepositoryPersistence::new(
        vec![Some(duplicate_record)],
        false,
    ));
    let duplicate = coordinator(
        Arc::new(ConfigProjectLookup { exists: true }),
        duplicate_repositories.clone(),
        Arc::new(ProviderAvailabilityGate::new(Arc::new(AvailableHealth))),
        Arc::new(StaticCadence {
            failure: None,
            count: AtomicUsize::new(0),
            source_root: temp.path().join("cadence"),
        }),
        Arc::new(|| Ok(())),
        Arc::new(CwdRootRunner),
        Arc::new(StaticInitializer {
            fail: false,
            count: AtomicUsize::new(0),
        }),
    )
    .register(input(root), CancellationToken::new())
    .await
    .unwrap_err();
    assert_eq!(duplicate.reason_code, "repository_already_registered");
    assert_eq!(
        duplicate_repositories.create_count.load(Ordering::SeqCst),
        0
    );
}

#[tokio::test]
async fn repository_registration_preserves_provider_host_and_cadence_failure_codes() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("repository");
    std::fs::create_dir_all(&root).unwrap();

    let unavailable_repositories =
        Arc::new(ConfigRepositoryPersistence::new(vec![None, None], false));
    let unavailable = coordinator(
        Arc::new(ConfigProjectLookup { exists: true }),
        unavailable_repositories.clone(),
        Arc::new(ProviderAvailabilityGate::new(Arc::new(UnavailableHealth))),
        Arc::new(StaticCadence {
            failure: None,
            count: AtomicUsize::new(0),
            source_root: temp.path().join("cadence"),
        }),
        Arc::new(|| Ok(())),
        Arc::new(CwdRootRunner),
        Arc::new(StaticInitializer {
            fail: false,
            count: AtomicUsize::new(0),
        }),
    )
    .register(input(root.clone()), CancellationToken::new())
    .await
    .unwrap_err();
    assert_eq!(unavailable.reason_code, "provider_unavailable");
    assert_eq!(unavailable.provider.as_deref(), Some("claude_code"));
    assert_eq!(
        unavailable_repositories.create_count.load(Ordering::SeqCst),
        0
    );

    let host_repositories = Arc::new(ConfigRepositoryPersistence::new(vec![None, None], false));
    let host = coordinator(
        Arc::new(ConfigProjectLookup { exists: true }),
        host_repositories.clone(),
        Arc::new(ProviderAvailabilityGate::new(Arc::new(AvailableHealth))),
        Arc::new(StaticCadence {
            failure: None,
            count: AtomicUsize::new(0),
            source_root: temp.path().join("cadence"),
        }),
        Arc::new(|| Err("host blocked API_KEY=secret".to_string())),
        Arc::new(CwdRootRunner),
        Arc::new(StaticInitializer {
            fail: false,
            count: AtomicUsize::new(0),
        }),
    )
    .register(input(root.clone()), CancellationToken::new())
    .await
    .unwrap_err();
    assert_eq!(host.reason_code, "host_real_workflow_blocked");
    assert_eq!(host.provider.as_deref(), Some("claude_code"));
    assert!(!host.stderr_summary.as_deref().unwrap().contains("secret"));
    assert!(
        host.stderr_summary
            .as_deref()
            .unwrap()
            .contains("[REDACTED]")
    );
    assert_eq!(host_repositories.create_count.load(Ordering::SeqCst), 0);

    for code in [
        "cadence_skills_unavailable",
        "cadence_skills_update_failed",
        "cadence_skills_sync_failed",
    ] {
        let repositories = Arc::new(ConfigRepositoryPersistence::new(vec![None, None], false));
        let error = coordinator(
            Arc::new(ConfigProjectLookup { exists: true }),
            repositories.clone(),
            Arc::new(ProviderAvailabilityGate::new(Arc::new(AvailableHealth))),
            Arc::new(StaticCadence {
                failure: Some(code),
                count: AtomicUsize::new(0),
                source_root: temp.path().join(format!("cadence-{code}")),
            }),
            Arc::new(|| Ok(())),
            Arc::new(CwdRootRunner),
            Arc::new(StaticInitializer {
                fail: false,
                count: AtomicUsize::new(0),
            }),
        )
        .register(input(root.clone()), CancellationToken::new())
        .await
        .unwrap_err();
        assert_eq!(error.reason_code, code);
        assert_eq!(repositories.create_count.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn repository_registration_rechecks_duplicate_inside_lock() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("repository");
    std::fs::create_dir_all(&root).unwrap();
    let repositories = Arc::new(ConfigRepositoryPersistence::new(
        vec![None, Some(repository_record("project_0001", root.clone()))],
        false,
    ));
    let cadence = Arc::new(StaticCadence {
        failure: None,
        count: AtomicUsize::new(0),
        source_root: temp.path().join("cadence"),
    });

    let error = coordinator(
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
    )
    .register(input(root), CancellationToken::new())
    .await
    .unwrap_err();

    assert_eq!(error.reason_code, "repository_already_registered");
    assert_eq!(cadence.count.load(Ordering::SeqCst), 0);
    assert_eq!(repositories.create_count.load(Ordering::SeqCst), 0);
}

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
    let first_coordinator = coordinator.clone();
    let first_input = input(root.clone());
    let first = tokio::spawn(async move {
        first_coordinator
            .register(first_input, CancellationToken::new())
            .await
    });
    entered.acquire().await.unwrap().forget();

    let second = coordinator
        .register(input(root.clone()), CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(second.reason_code, "repository_initialization_in_progress");
    assert_eq!(cadence.count.load(Ordering::SeqCst), 1);

    release.add_permits(1);
    first.await.unwrap().unwrap();
    assert_eq!(repositories.create_count.load(Ordering::SeqCst), 1);

    release.add_permits(1);
    let third = coordinator
        .register(input(root), CancellationToken::new())
        .await
        .unwrap();
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
    let first = {
        let coordinator = coordinator.clone();
        tokio::spawn(async move {
            coordinator
                .register(input(first_root), CancellationToken::new())
                .await
        })
    };
    let second = {
        let coordinator = coordinator.clone();
        tokio::spawn(async move {
            coordinator
                .register(input(second_root), CancellationToken::new())
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
    let before = coordinator(
        Arc::new(ConfigProjectLookup { exists: true }),
        before_repositories.clone(),
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
    )
    .register(input(root.clone()), CancellationToken::new())
    .await
    .unwrap_err();
    assert_eq!(before.reason_code, "repository_git_state_failed");
    assert_eq!(before_initializer.count.load(Ordering::SeqCst), 0);
    assert_eq!(before_repositories.create_count.load(Ordering::SeqCst), 0);

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
    let after = coordinator(
        Arc::new(ConfigProjectLookup { exists: true }),
        after_repositories.clone(),
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
    )
    .register(input(root.clone()), CancellationToken::new())
    .await
    .unwrap_err();
    assert_eq!(after.reason_code, "repository_git_state_failed");
    assert_eq!(after.changed_paths, None);
    assert!(after.action.contains("inspect the repository manually"));
    assert_eq!(after_repositories.create_count.load(Ordering::SeqCst), 0);

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
