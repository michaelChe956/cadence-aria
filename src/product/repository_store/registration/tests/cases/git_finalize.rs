#[tokio::test]
async fn repository_registration_finalizes_staged_changes_after_persisting_repository() {
    let temp = TempDir::new().unwrap();
    let git_root = temp.path().join("repository");
    std::fs::create_dir_all(&git_root).unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let repositories = Arc::new(RecordingRepositoryPersistence {
        calls: calls.clone(),
        created: AtomicUsize::new(0),
    });
    let operations = RepositoryInitializationOperationStore::new(ProductAppPaths::new(
        temp.path().join(".aria"),
    ));
    let coordinator = RepositoryRegistrationCoordinator::new_with_operations(
        Arc::new(RecordingProjectLookup {
            calls: calls.clone(),
        }),
        repositories.clone(),
        operations.clone(),
        Arc::new(ProviderAvailabilityGate::new(Arc::new(AvailableHealth))),
        registry(),
        Arc::new(RecordingCadence {
            calls: calls.clone(),
            source_root: temp.path().join("cadence-source"),
        }),
        Arc::new(|| Ok(())),
        Arc::new(GitFinalizeRunner {
            calls: calls.clone(),
            root: git_root.clone(),
            status_calls: AtomicUsize::new(0),
        }),
        Arc::new(|| "2026-07-25T00:00:00Z".to_string()),
        Arc::new(RecordingInitializer {
            calls: calls.clone(),
        }),
        Duration::from_secs(1),
        Duration::from_secs(1),
    );

    let launch = coordinator
        .begin_initialization(input(git_root), CancellationToken::new())
        .await
        .unwrap();
    let operation_id = launch.operation_id().to_string();
    let completed = coordinator
        .execute_initialization(launch, CancellationToken::new())
        .await
        .unwrap();
    let operation = operations.get("project_0001", &operation_id).unwrap();

    assert_eq!(repositories.created.load(Ordering::SeqCst), 1);
    assert_eq!(operation.status, RepositoryInitializationOperationStatus::Completed);
    assert!(operation
        .steps
        .iter()
        .all(|step| step.status == RepositoryInitializationStepStatus::Completed));
    assert_eq!(
        completed
            .result
            .as_ref()
            .and_then(|result| result.git_finalize_warning.as_ref()),
        None,
    );
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        &[
            "project_get",
            "git_root",
            "repository_find",
            "repository_find",
            "cadence_prepare",
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
async fn git_finalize_handles_no_change_push_skips_and_failures() {
    struct Case {
        name: &'static str,
        responses: Vec<(Vec<String>, BoundedCommandResult)>,
        expected_warning: Option<&'static str>,
        expected_error: Option<&'static str>,
        expected_next_argv: Option<Vec<String>>,
    }

    let add = || vec!["add".to_string(), "-A".to_string()];
    let diff = || {
        vec![
            "diff".to_string(),
            "--cached".to_string(),
            "--quiet".to_string(),
        ]
    };
    let commit = || {
        vec![
            "commit".to_string(),
            "-m".to_string(),
            "初始化cadence-aria 代码库".to_string(),
        ]
    };
    let upstream = || {
        vec![
            "rev-parse".to_string(),
            "--abbrev-ref".to_string(),
            "--symbolic-full-name".to_string(),
            "@{u}".to_string(),
        ]
    };
    let cases = vec![
        Case {
            name: "no staged change",
            responses: vec![
                (add(), command_result(Some(0), "", "")),
                (diff(), command_result(Some(0), "", "")),
            ],
            expected_warning: None,
            expected_error: None,
            expected_next_argv: None,
        },
        Case {
            name: "add failure stops git finalization",
            responses: vec![
                (add(), command_result(Some(1), "", "index locked")),
                (diff(), command_result(Some(1), "", "")),
            ],
            expected_warning: None,
            expected_error: Some("git_finalize_add"),
            expected_next_argv: Some(diff()),
        },
        Case {
            name: "no remote",
            responses: vec![
                (add(), command_result(Some(0), "", "")),
                (diff(), command_result(Some(1), "", "")),
                (commit(), command_result(Some(0), "", "")),
                (vec!["remote".to_string()], command_result(Some(0), "", "")),
            ],
            expected_warning: Some("无 remote"),
            expected_error: None,
            expected_next_argv: None,
        },
        Case {
            name: "no upstream",
            responses: vec![
                (add(), command_result(Some(0), "", "")),
                (diff(), command_result(Some(1), "", "")),
                (commit(), command_result(Some(0), "", "")),
                (
                    vec!["remote".to_string()],
                    command_result(Some(0), "origin\n", ""),
                ),
                (upstream(), command_result(Some(128), "", "no upstream")),
            ],
            expected_warning: Some("无 upstream"),
            expected_error: None,
            expected_next_argv: None,
        },
        Case {
            name: "commit failure",
            responses: vec![
                (add(), command_result(Some(0), "", "")),
                (diff(), command_result(Some(1), "", "")),
                (commit(), command_result(Some(1), "", "identity unknown")),
            ],
            expected_warning: None,
            expected_error: Some("git_finalize_commit"),
            expected_next_argv: None,
        },
        Case {
            name: "push failure",
            responses: vec![
                (add(), command_result(Some(0), "", "")),
                (diff(), command_result(Some(1), "", "")),
                (commit(), command_result(Some(0), "", "")),
                (
                    vec!["remote".to_string()],
                    command_result(Some(0), "origin\n", ""),
                ),
                (upstream(), command_result(Some(0), "origin/main\n", "")),
                (vec!["push".to_string()], command_result(Some(1), "", "permission denied")),
            ],
            expected_warning: None,
            expected_error: Some("git_finalize_push"),
            expected_next_argv: None,
        },
    ];

    for case in cases {
        let temp = TempDir::new().unwrap();
        let runner = Arc::new(ScriptedGitRunner::new(case.responses));
        let coordinator = coordinator(
            Arc::new(ConfigProjectLookup { exists: true }),
            Arc::new(ConfigRepositoryPersistence::new(vec![], false)),
            Arc::new(ProviderAvailabilityGate::new(Arc::new(AvailableHealth))),
            Arc::new(StaticCadence {
                failure: None,
                count: AtomicUsize::new(0),
                source_root: temp.path().join("cadence"),
            }),
            Arc::new(|| Ok(())),
            runner.clone(),
            Arc::new(StaticInitializer {
                fail: false,
                count: AtomicUsize::new(0),
            }),
        );

        let result = coordinator
            .git_finalize(temp.path(), CancellationToken::new())
            .await;
        match (case.expected_warning, case.expected_error) {
            (Some(expected_warning), None) => assert!(
                result
                    .expect(case.name)
                    .as_deref()
                    .is_some_and(|warning| warning.contains(expected_warning)),
                "{}",
                case.name
            ),
            (None, None) => assert_eq!(result.expect(case.name), None, "{}", case.name),
            (None, Some(expected_error)) => assert!(
                result
                    .expect_err(case.name)
                    .contains(expected_error),
                "{}",
                case.name
            ),
            (Some(_), Some(_)) => unreachable!(),
        }
        match case.expected_next_argv {
            Some(expected_next_argv) => {
                let responses = runner.responses.lock().unwrap();
                assert_eq!(responses.len(), 1, "{} must stop after git add", case.name);
                assert_eq!(
                    responses.front().map(|(argv, _)| argv),
                    Some(&expected_next_argv),
                    "{} must not invoke diff/commit/push after git add fails",
                    case.name
                );
                assert_eq!(runner.calls.lock().unwrap().as_slice(), &[add()]);
            }
            None => assert!(
                runner.responses.lock().unwrap().is_empty(),
                "{} left unconsumed Git responses",
                case.name
            ),
        }
    }
}

#[tokio::test]
async fn repository_registration_keeps_completed_result_when_git_finalize_push_fails() {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("repository");
    std::fs::create_dir_all(&root).unwrap();
    let operations = RepositoryInitializationOperationStore::new(ProductAppPaths::new(
        temp.path().join(".aria"),
    ));
    let repositories = Arc::new(ConfigRepositoryPersistence::new(vec![], false));
    let runner = Arc::new(ScriptedGitRunner::new(vec![
        (
            vec!["rev-parse".to_string(), "--show-toplevel".to_string()],
            command_result(Some(0), &format!("{}\n", root.display()), ""),
        ),
        (
            vec![
                "status".to_string(),
                "--porcelain=v1".to_string(),
                "-z".to_string(),
                "--untracked-files=all".to_string(),
            ],
            command_result(Some(0), "", ""),
        ),
        (
            vec![
                "status".to_string(),
                "--porcelain=v1".to_string(),
                "-z".to_string(),
                "--untracked-files=all".to_string(),
            ],
            command_result(Some(0), "?? generated.txt\0", ""),
        ),
        (vec!["add".to_string(), "-A".to_string()], command_result(Some(0), "", "")),
        (
            vec!["diff".to_string(), "--cached".to_string(), "--quiet".to_string()],
            command_result(Some(1), "", ""),
        ),
        (
            vec![
                "commit".to_string(),
                "-m".to_string(),
                "初始化cadence-aria 代码库".to_string(),
            ],
            command_result(Some(0), "", ""),
        ),
        (vec!["remote".to_string()], command_result(Some(0), "origin\n", "")),
        (
            vec![
                "rev-parse".to_string(),
                "--abbrev-ref".to_string(),
                "--symbolic-full-name".to_string(),
                "@{u}".to_string(),
            ],
            command_result(Some(0), "origin/main\n", ""),
        ),
        (vec!["push".to_string()], command_result(Some(1), "", "permission denied")),
    ]));
    let coordinator = coordinator_with_operations(
        Arc::new(ConfigProjectLookup { exists: true }),
        repositories.clone(),
        operations.clone(),
        Arc::new(ProviderAvailabilityGate::new(Arc::new(AvailableHealth))),
        Arc::new(StaticCadence {
            failure: None,
            count: AtomicUsize::new(0),
            source_root: temp.path().join("cadence"),
        }),
        Arc::new(|| Ok(())),
        runner.clone(),
        Arc::new(StaticInitializer {
            fail: false,
            count: AtomicUsize::new(0),
        }),
    );

    let launch = coordinator
        .begin_initialization(input(root), CancellationToken::new())
        .await
        .unwrap();
    let operation_id = launch.operation_id().to_string();
    let completed = coordinator
        .execute_initialization(launch, CancellationToken::new())
        .await
        .unwrap();
    let operation = operations.get("project_0001", &operation_id).unwrap();

    assert_eq!(repositories.create_count.load(Ordering::SeqCst), 1);
    assert_eq!(operation.status, RepositoryInitializationOperationStatus::Completed);
    assert!(operation.error.is_none());
    assert_eq!(
        operation
            .steps
            .iter()
            .map(|step| step.status)
            .collect::<Vec<_>>(),
        vec![
            RepositoryInitializationStepStatus::Completed,
            RepositoryInitializationStepStatus::Completed,
            RepositoryInitializationStepStatus::Completed,
            RepositoryInitializationStepStatus::Completed,
            RepositoryInitializationStepStatus::Completed,
            RepositoryInitializationStepStatus::Failed,
        ]
    );
    assert_eq!(
        operation.failed_step,
        Some(RepositoryInitializationStepKind::GitFinalize)
    );
    let warning = completed
        .result
        .as_ref()
        .and_then(|result| result.git_finalize_warning.as_deref())
        .expect("completed operation keeps the git finalize warning");
    assert!(warning.contains("push"));
    assert!(warning.contains("手动提交推送"));
    assert!(operation.result.is_some());
    assert!(runner.responses.lock().unwrap().is_empty());
}
