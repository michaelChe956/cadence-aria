#[tokio::test]
async fn repository_initialization_post_returns_202_then_get_returns_completed_six_step_result() {
    let root = tempdir().unwrap();
    let repo = git_repo();
    let provider = Arc::new(ScriptedClaude::new(vec![TurnScript::Complete; 4]));
    let app = build_web_router(integration_state(root.path(), provider.clone(), None, None));
    create_project(app.clone()).await;

    let (status, accepted) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;

    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let operation_id = accepted["operation_id"].as_str().unwrap();
    assert_eq!(accepted["status"], "created");
    assert_eq!(accepted["steps"].as_array().unwrap().len(), 6);
    assert_eq!(
        accepted["steps"][5]["step_id"],
        "git_finalize",
        "git finalize must be the sixth and final initialization step"
    );
    assert!(
        accepted["steps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|step| step["status"] == "pending")
    );

    let completed = get_operation_until_terminal(&app, "project_0001", operation_id).await;
    let canonical_repository_path = repo
        .path()
        .canonicalize()
        .expect("canonical repository path");
    let canonical_repository_path = canonical_repository_path.to_string_lossy();
    assert_eq!(completed["status"], "completed");
    assert!(
        completed["steps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|step| step["status"] == "completed")
    );
    assert_eq!(
        completed["result"]["repository"]["repository_id"],
        "repository_0001"
    );
    assert_eq!(completed["result"]["repository"]["path"], "<path>");
    assert_eq!(completed["result"]["repository"]["runtime_root"], "<path>");
    let serialized_completed =
        serde_json::to_string(&completed).expect("serialize completed operation response");
    assert!(
        !serialized_completed.contains(canonical_repository_path.as_ref()),
        "completed operation response leaked repository path: {serialized_completed}"
    );
    assert_eq!(
        completed["result"]["initialization"]["commands"][0]["command"],
        "/pre-check --no-interrupt",
    );

    let inputs = provider.inputs.lock().expect("inputs");
    assert_eq!(
        inputs
            .iter()
            .map(|input| input.prompt.as_str())
            .collect::<Vec<_>>(),
        vec![
            "/pre-check --no-interrupt",
            "/rule-config --no-interrupt",
            "/mcp-configuration --no-interrupt",
            "/project-rules-examples --no-interrupt"
        ]
    );
    for input in inputs.iter() {
        assert_eq!(input.working_dir, repo.path().canonicalize().unwrap());
        assert_eq!(input.permission_mode, ProviderPermissionMode::Auto);
        assert!(input.workspace_session_id.is_none());
        assert!(input.resume_provider_session_id.is_none());
    }
}

#[tokio::test]
async fn repository_initialization_completed_operation_get_sanitizes_persisted_result_paths() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let repository_path = repo
        .path()
        .canonicalize()
        .expect("canonical repository path");
    let runtime_root = repository_path.join(".aria/runtime");
    let app = build_web_router(integration_state(
        root.path(),
        Arc::new(ScriptedClaude::new(Vec::new())),
        None,
        None,
    ));
    create_project(app.clone()).await;

    let operation_id = "repository_initialization_completed_sanitization";
    let operation_store =
        cadence_aria::product::repository_store::RepositoryInitializationOperationStore::new(
            ProductAppPaths::new(root.path().join(".aria")),
        );
    operation_store
        .create(RepositoryInitializationOperation::new(
            operation_id.to_string(),
            "project_0001".to_string(),
            RepositoryInitializationOperationInput {
                name: "Repo".to_string(),
                git_root: repository_path.clone(),
                default_policy_preset: Some("manual-write".to_string()),
                default_provider_mode: Some("claude_code".to_string()),
            },
            "2026-07-14T03:00:00Z".to_string(),
        ))
        .expect("create completed operation");
    operation_store
        .mark_running(
            "project_0001",
            operation_id,
            "2026-07-14T03:00:01Z".to_string(),
        )
        .expect("mark operation running");
    let success = cadence_aria::product::repository_store::RepositoryRegistrationSuccess {
        repository: RepositoryRecord {
            id: "repository_0001".to_string(),
            project_id: "project_0001".to_string(),
            name: "Repo".to_string(),
            path: repository_path.clone(),
            repo_hash: "repo_hash".to_string(),
            runtime_root: runtime_root.clone(),
            default_policy_preset: "manual-write".to_string(),
            default_provider_mode: "claude_code".to_string(),
            created_at: "2026-07-14T03:00:12Z".to_string(),
            updated_at: "2026-07-14T03:00:12Z".to_string(),
        },
        cadence_skills:
            cadence_aria::product::repository_store::CadenceSkillsPreparationSummary {
                source_mode: "offline".to_string(),
                source_root: PathBuf::from("/private/cadence-skills"),
                skills_root: PathBuf::from("/private/repo/.claude/skills"),
                git_updated: false,
                link_sync_status: "synchronized".to_string(),
                warnings: Vec::new(),
            },
        initialization:
            cadence_aria::product::repository_store::RepositoryInitializationSummary {
                provider: "claude_code".to_string(),
                source: PathBuf::from("/private/cadence-skills"),
                source_mode: "offline".to_string(),
                skills_root: PathBuf::from("/private/repo/.claude/skills"),
                git_updated: false,
                link_sync_status: "synchronized".to_string(),
                commands: vec![RepositoryInitializationCommandSummary {
                    command_index: 1,
                    command: "/pre-check --no-interrupt".to_string(),
                    status: "completed".to_string(),
                    output_summary: None,
                }],
            },
        warnings: Vec::new(),
        changed_paths: vec![
            "/private/repo/generated".to_string(),
            ".claude/rules/project.md".to_string(),
            "src/monkey.rs".to_string(),
        ],
        git_finalize_warning: None,
        completed_at: "2026-07-14T03:00:12Z".to_string(),
    };
    for (index, step) in RepositoryInitializationStepKind::ALL
        .into_iter()
        .enumerate()
    {
        operation_store
            .mark_step_running(
                "project_0001",
                operation_id,
                step,
                format!("2026-07-14T03:00:{:02}Z", index * 2 + 2),
            )
            .expect("mark operation step running");
        if step == RepositoryInitializationStepKind::GitFinalize {
            operation_store
                .checkpoint_git_finalize_result("project_0001", operation_id, success.clone())
                .expect("checkpoint git finalize result");
        }
        operation_store
            .mark_step_completed(
                "project_0001",
                operation_id,
                step,
                format!("2026-07-14T03:00:{:02}Z", index * 2 + 3),
            )
            .expect("mark operation step completed");
    }
    operation_store
        .finish_completed(
            "project_0001",
            operation_id,
            success,
            "2026-07-14T03:00:12Z".to_string(),
        )
        .expect("finish completed operation");

    let (status, completed) = request_json(
        app,
        Method::GET,
        &format!("/api/projects/project_0001/repository-initializations/{operation_id}"),
        json!({}),
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{completed}");
    assert_eq!(completed["status"], "completed");
    assert_eq!(
        completed["result"]["initialization"]["changed_paths"],
        json!(["<path>", ".claude/rules/project.md", "src/monkey.rs"])
    );
    assert_eq!(completed["result"]["repository"]["path"], "<path>");
    assert_eq!(completed["result"]["repository"]["runtime_root"], "<path>");

    let serialized = serde_json::to_string(&completed).expect("serialize completed operation");
    assert!(
        !serialized.contains("/private/repo/generated"),
        "completed operation response leaked changed path: {serialized}"
    );
    assert!(
        !serialized.contains(repository_path.to_string_lossy().as_ref()),
        "completed operation response leaked repository path: {serialized}"
    );
    assert!(
        !serialized.contains(runtime_root.to_string_lossy().as_ref()),
        "completed operation response leaked runtime root: {serialized}"
    );
}

#[tokio::test]
async fn repository_initialization_failures_stop_and_leave_no_repository_record() {
    for fail_at in 1..=4 {
        let root = tempdir().expect("root");
        let repo = git_repo();
        let mut scripts = vec![TurnScript::Complete; fail_at - 1];
        scripts.push(TurnScript::Fail);
        let provider = Arc::new(ScriptedClaude::new(scripts));
        let app = build_web_router(integration_state(root.path(), provider.clone(), None, None));
        create_project(app.clone()).await;
        let (status, accepted) = request_json(
            app.clone(),
            Method::POST,
            "/api/projects/project_0001/repositories",
            json!({"name":"Repo","path":repo.path()}),
        )
        .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
        let operation_id = accepted["operation_id"].as_str().expect("operation id");
        let failed = get_operation_until_terminal(&app, "project_0001", operation_id).await;
        assert_eq!(failed["status"], "failed");
        assert_eq!(failed["failed_step"], command_step_id(fail_at));
        assert_eq!(failed["error"]["code"], "repository_init_command_failed");
        assert_eq!(
            failed["error"]["details"]["command"],
            command_summaries()[fail_at - 1].command
        );
        assert_eq!(provider.inputs.lock().expect("inputs").len(), fail_at);
        assert!(failed["error"]["details"]["retryable"].as_bool().unwrap());
        assert!(failed["error"]["details"]["action"].is_string());
        let (_, repositories) = request_json(
            app,
            Method::GET,
            "/api/projects/project_0001/repositories",
            json!({}),
        )
        .await;
        assert!(repositories["repositories"].as_array().unwrap().is_empty());
    }
}

#[tokio::test]
async fn repository_initialization_interaction_aborts_and_does_not_persist() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let provider = Arc::new(ScriptedClaude::new(vec![TurnScript::Interaction]));
    let app = build_web_router(integration_state(root.path(), provider.clone(), None, None));
    create_project(app.clone()).await;
    let (status, accepted) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let operation_id = accepted["operation_id"].as_str().expect("operation id");
    let failed = get_operation_until_terminal(&app, "project_0001", operation_id).await;
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["failed_step"], "pre_check");
    assert_eq!(
        failed["error"]["code"],
        "repository_init_interaction_required"
    );
    for _ in 0..10 {
        if provider
            .commands
            .lock()
            .expect("commands")
            .contains(&ProviderCommand::Abort)
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(
        provider
            .commands
            .lock()
            .expect("commands")
            .contains(&ProviderCommand::Abort)
    );
    assert_eq!(provider.inputs.lock().expect("inputs").len(), 1);
    let (_, repositories) = request_json(
        app,
        Method::GET,
        "/api/projects/project_0001/repositories",
        json!({}),
    )
    .await;
    assert!(repositories["repositories"].as_array().unwrap().is_empty());
}

struct FailingPersistence;

impl RepositoryPersistence for FailingPersistence {
    fn find_by_path(
        &self,
        _project_id: &str,
        _path: &Path,
    ) -> Result<Option<RepositoryRecord>, ProductStoreError> {
        Ok(None)
    }

    fn create_repository(
        &self,
        _input: CreateRepositoryInput,
    ) -> Result<RepositoryRecord, ProductStoreError> {
        Err(ProductStoreError::Io(
            "scripted persistence failure".to_string(),
        ))
    }
}

struct BlockingInitializer {
    started: Arc<Semaphore>,
    release: Arc<Semaphore>,
}

#[async_trait::async_trait]
impl RepositoryInitializer for BlockingInitializer {
    async fn initialize_repository(
        &self,
        _git_root: &Path,
        _command_timeout: Duration,
        _cancellation: CancellationToken,
        progress: Arc<dyn RepositoryInitializationProgress>,
    ) -> Result<Vec<RepositoryInitializationCommandSummary>, RepositoryRegistrationError> {
        self.started.add_permits(1);
        let permit = self.release.acquire().await.expect("release");
        permit.forget();
        let summaries: Result<_, Box<RepositoryRegistrationError>> =
            RepositoryInitializationStepKind::ALL
                .into_iter()
                .filter_map(|step| step.command().map(|command| (step, command)))
                .enumerate()
                .map(|(offset, (step, command))| {
                    progress.step_started(step)?;
                    progress.step_completed(step)?;
                    Ok(RepositoryInitializationCommandSummary {
                        command_index: offset + 1,
                        command: command.to_string(),
                        status: "completed".to_string(),
                        output_summary: None,
                    })
                })
                .collect();
        summaries.map_err(|error| *error)
    }
}

#[tokio::test]
async fn repository_initialization_persist_failure_and_same_path_lock_are_transactional() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let provider = Arc::new(ScriptedClaude::new(vec![TurnScript::Complete; 4]));
    let app = build_web_router(integration_state(
        root.path(),
        provider,
        Some(Arc::new(FailingPersistence)),
        None,
    ));
    create_project(app.clone()).await;
    let (status, accepted) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let operation_id = accepted["operation_id"].as_str().expect("operation id");
    let failed = get_operation_until_terminal(&app, "project_0001", operation_id).await;
    assert_eq!(failed["status"], "failed");
    assert!(failed["steps"].as_array().unwrap()[..5]
        .iter()
        .all(|step| step["status"] == "completed")
    );
    assert_eq!(
        failed["steps"][5]["status"],
        "pending",
        "repository persistence fails before git finalize starts"
    );
    assert_eq!(failed["failed_step"], Value::Null);
    assert_eq!(
        failed["error"]["details"]["reason_code"],
        "repository_persist_failed"
    );

    let root = tempdir().expect("root");
    let repo = git_repo();
    let started = Arc::new(Semaphore::new(0));
    let release = Arc::new(Semaphore::new(0));
    let initializer = Arc::new(BlockingInitializer {
        started: started.clone(),
        release: release.clone(),
    });
    let provider = Arc::new(ScriptedClaude::new(Vec::new()));
    let app = build_web_router(integration_state(
        root.path(),
        provider,
        None,
        Some(initializer),
    ));
    create_project(app.clone()).await;
    let (status, accepted) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let operation_id = accepted["operation_id"].as_str().expect("operation id");
    let permit = started.acquire().await.expect("started");
    permit.forget();
    let (status, running) = get_operation(app.clone(), "project_0001", operation_id).await;
    assert_eq!(status, StatusCode::OK, "{running}");
    assert_eq!(running["status"], "running");
    let (status, in_progress) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(in_progress["code"], "repository_initialization_in_progress");
    release.add_permits(1);
    let completed = get_operation_until_terminal(&app, "project_0001", operation_id).await;
    assert_eq!(completed["status"], "completed");
    let (status, already_registered) = request_json(
        app,
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(already_registered["code"], "repository_already_registered");
}

#[tokio::test]
async fn repository_initialization_operation_unknown_and_cross_project_are_not_found() {
    let root = tempdir().expect("root");
    let repo = git_repo();
    let provider = Arc::new(ScriptedClaude::new(vec![TurnScript::Complete; 4]));
    let app = build_web_router(integration_state(root.path(), provider, None, None));
    create_project(app.clone()).await;
    let (status, unknown) = get_operation(
        app.clone(),
        "project_0001",
        "repository_initialization_unknown",
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{unknown}");
    assert_eq!(
        unknown["code"],
        "repository_initialization_operation_not_found"
    );

    let (status, accepted) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/repositories",
        json!({"name":"Repo","path":repo.path()}),
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");
    let operation_id = accepted["operation_id"].as_str().expect("operation id");
    let (status, cross_project) = get_operation(app, "project_0002", operation_id).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{cross_project}");
    assert_eq!(
        cross_project["code"],
        "repository_initialization_operation_not_found"
    );
}

#[tokio::test]
async fn repository_initialization_stale_records_are_recovered_before_new_launch() {
    for status in [
        RepositoryInitializationOperationStatus::Created,
        RepositoryInitializationOperationStatus::Running,
    ] {
        let root = tempdir().expect("root");
        let repo = git_repo();
        let initial_provider = Arc::new(ScriptedClaude::new(Vec::new()));
        let initial_app =
            build_web_router(integration_state(root.path(), initial_provider, None, None));
        create_project(initial_app).await;
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let store =
            cadence_aria::product::repository_store::RepositoryInitializationOperationStore::new(
                paths,
            );
        let operation_id = format!("repository_initialization_stale_{status:?}").to_lowercase();
        let stale = RepositoryInitializationOperation::new(
            operation_id.clone(),
            "project_0001".to_string(),
            RepositoryInitializationOperationInput {
                name: "Stale".to_string(),
                git_root: repo.path().canonicalize().expect("canonical git root"),
                default_policy_preset: None,
                default_provider_mode: None,
            },
            "2026-07-14T03:00:00Z".to_string(),
        );
        store.create(stale).expect("stale operation");
        if status == RepositoryInitializationOperationStatus::Running {
            store
                .mark_running(
                    "project_0001",
                    &operation_id,
                    "2026-07-14T03:01:00Z".to_string(),
                )
                .expect("mark stale operation running");
            store
                .mark_step_running(
                    "project_0001",
                    &operation_id,
                    RepositoryInitializationStepKind::CadenceSkills,
                    "2026-07-14T03:01:00Z".to_string(),
                )
                .expect("mark stale operation step running");
        }

        let provider = Arc::new(ScriptedClaude::new(vec![TurnScript::Complete; 4]));
        let app = build_web_router(integration_state(root.path(), provider, None, None));
        let (get_status, recovered) =
            get_operation(app.clone(), "project_0001", &operation_id).await;
        assert_eq!(get_status, StatusCode::OK, "{recovered}");
        assert_eq!(recovered["status"], "failed");
        assert_eq!(
            recovered["error"]["details"]["reason_code"],
            "repository_initialization_interrupted"
        );

        let (post_status, accepted) = request_json(
            app,
            Method::POST,
            "/api/projects/project_0001/repositories",
            json!({"name":"Repo","path":repo.path()}),
        )
        .await;
        assert_eq!(post_status, StatusCode::ACCEPTED, "{accepted}");
    }
}
