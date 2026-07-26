#[tokio::test]
async fn repository_registration_persists_real_step_states_and_only_creates_repository_at_completion()
 {
    let fixture = operation_coordinator_fixture(false, false);
    let launch = fixture
        .coordinator
        .begin_initialization(fixture.input.clone(), CancellationToken::new())
        .await
        .unwrap();
    let operation_id = launch.operation_id().to_string();

    let initial = fixture
        .operations
        .get("project_0001", &operation_id)
        .unwrap();
    assert_eq!(
        initial.status,
        RepositoryInitializationOperationStatus::Created
    );
    assert!(
        initial
            .steps
            .iter()
            .all(|step| step.status == RepositoryInitializationStepStatus::Pending)
    );
    assert_eq!(fixture.repositories.create_count.load(Ordering::SeqCst), 0);

    fixture
        .coordinator
        .execute_initialization(launch, CancellationToken::new())
        .await
        .unwrap();
    let completed = fixture
        .operations
        .get("project_0001", &operation_id)
        .unwrap();
    assert_eq!(
        completed.status,
        RepositoryInitializationOperationStatus::Completed
    );
    assert!(
        completed
            .steps
            .iter()
            .all(|step| step.status == RepositoryInitializationStepStatus::Completed)
    );
    assert!(completed.result.is_some());
    assert_eq!(fixture.repositories.create_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn repository_registration_persists_step_boundaries_for_execution_failures() {
    struct Case {
        name: &'static str,
        cadence_fails: bool,
        initializer_fails: bool,
        persistence_fails: bool,
        expected_failed_step: Option<RepositoryInitializationStepKind>,
        expected_steps: [RepositoryInitializationStepStatus; 6],
        expected_error: &'static str,
    }

    let cases = [
        Case {
            name: "cadence failure",
            cadence_fails: true,
            initializer_fails: false,
            persistence_fails: false,
            expected_failed_step: Some(RepositoryInitializationStepKind::CadenceSkills),
            expected_steps: [
                RepositoryInitializationStepStatus::Failed,
                RepositoryInitializationStepStatus::Pending,
                RepositoryInitializationStepStatus::Pending,
                RepositoryInitializationStepStatus::Pending,
                RepositoryInitializationStepStatus::Pending,
                RepositoryInitializationStepStatus::Pending,
            ],
            expected_error: "cadence_skills_unavailable",
        },
        Case {
            name: "second Claude command failure",
            cadence_fails: false,
            initializer_fails: true,
            persistence_fails: false,
            expected_failed_step: Some(RepositoryInitializationStepKind::RuleConfig),
            expected_steps: [
                RepositoryInitializationStepStatus::Completed,
                RepositoryInitializationStepStatus::Completed,
                RepositoryInitializationStepStatus::Failed,
                RepositoryInitializationStepStatus::Pending,
                RepositoryInitializationStepStatus::Pending,
                RepositoryInitializationStepStatus::Pending,
            ],
            expected_error: "repository_init_command_failed",
        },
        Case {
            name: "repository persistence failure",
            cadence_fails: false,
            initializer_fails: false,
            persistence_fails: true,
            expected_failed_step: None,
            expected_steps: [
                RepositoryInitializationStepStatus::Completed,
                RepositoryInitializationStepStatus::Completed,
                RepositoryInitializationStepStatus::Completed,
                RepositoryInitializationStepStatus::Completed,
                RepositoryInitializationStepStatus::Completed,
                RepositoryInitializationStepStatus::Pending,
            ],
            expected_error: "repository_persist_failed",
        },
    ];

    for case in cases {
        let fixture = operation_coordinator_fixture(case.cadence_fails, case.initializer_fails);
        fixture
            .repositories
            .fail_create
            .store(case.persistence_fails, Ordering::SeqCst);
        let launch = fixture
            .coordinator
            .begin_initialization(fixture.input.clone(), CancellationToken::new())
            .await
            .unwrap();
        let operation_id = launch.operation_id().to_string();

        let error = fixture
            .coordinator
            .execute_initialization(launch, CancellationToken::new())
            .await
            .unwrap_err();
        let failed = fixture
            .operations
            .get("project_0001", &operation_id)
            .unwrap();

        assert_eq!(
            failed.status,
            RepositoryInitializationOperationStatus::Failed,
            "{}",
            case.name
        );
        assert_eq!(
            failed.failed_step, case.expected_failed_step,
            "{}",
            case.name
        );
        assert_eq!(
            failed
                .steps
                .iter()
                .map(|step| step.status)
                .collect::<Vec<_>>(),
            case.expected_steps,
            "{}",
            case.name
        );
        assert_eq!(
            failed
                .error
                .as_ref()
                .map(|error| error.reason_code.as_str()),
            Some(case.expected_error),
            "{}",
            case.name
        );
        assert_eq!(error.reason_code, case.expected_error, "{}", case.name);
        assert_eq!(
            fixture.repositories.create_count.load(Ordering::SeqCst),
            usize::from(case.persistence_fails),
            "{}",
            case.name
        );
        if case.persistence_fails {
            assert_eq!(
                failed
                    .error
                    .as_ref()
                    .and_then(|error| error.changed_paths.clone()),
                Some(vec!["generated.txt".to_string()]),
                "{}",
                case.name
            );
        } else {
            assert_eq!(
                fixture.repositories.create_count.load(Ordering::SeqCst),
                0,
                "{}",
                case.name
            );
        }
    }
}

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
    let operations = RepositoryInitializationOperationStore::new(ProductAppPaths::new(
        temp.path().join(".aria"),
    ));
    let coordinator = RepositoryRegistrationCoordinator::new_with_operations(
        Arc::new(RecordingProjectLookup {
            calls: calls.clone(),
        }),
        repositories.clone(),
        operations.clone(),
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

    let input = RepositoryRegistrationInput {
        project_id: "project_0001".to_string(),
        name: "Repository".to_string(),
        path: git_root.clone(),
        default_policy_preset: None,
        default_provider_mode: None,
    };
    let launch = coordinator
        .begin_initialization(input, CancellationToken::new())
        .await
        .unwrap();
    let operation_id = launch.operation_id().to_string();
    let result = coordinator
        .execute_initialization(launch, CancellationToken::new())
        .await
        .unwrap()
        .result
        .expect("completed operation result");

    assert_eq!(repositories.created.load(Ordering::SeqCst), 1);
    assert_eq!(result.repository.path, git_root);
    assert_eq!(result.initialization.provider, "claude_code");
    assert_eq!(result.initialization.source_mode, "offline");
    assert_eq!(result.initialization.commands.len(), 4);
    assert_eq!(result.warnings, vec!["offline source"]);
    assert_eq!(result.changed_paths, vec!["generated.txt"]);
    assert_eq!(result.completed_at, "2026-07-13T01:02:03Z");
    let operation = operations.get("project_0001", &operation_id).unwrap();
    assert_eq!(
        operation.status,
        RepositoryInitializationOperationStatus::Completed
    );
    assert!(
        operation
            .steps
            .iter()
            .all(|step| step.status == RepositoryInitializationStepStatus::Completed)
    );
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        &[
            "project_get",
            "git_root",
            "repository_find",
            "repository_find",
            "cadence_prepare",
            "host_ready",
            "git_status",
            "initializer",
            "git_status",
            "repository_create",
            "git_finalize_add",
            "git_finalize_diff",
            "git_finalize_commit",
            "git_finalize_remote",
            "git_finalize_upstream",
            "git_finalize_push",
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
