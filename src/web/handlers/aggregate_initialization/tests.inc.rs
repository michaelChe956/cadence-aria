    use crate::web::app::build_web_router;
    use crate::web::runtime::WebRuntime;
    use crate::web::state::WebAppState;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tempfile::tempdir;
    use tower::ServiceExt;

    fn aggregate_init_test_app() -> axum::Router {
        let root = tempdir().expect("root");
        let root_path = root.path().to_path_buf();
        // Persist a minimal manifest so create can load it.
        let paths = ProductAppPaths::new(root_path.join(".aria"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "Aggregate initialization test".to_string(),
                description: None,
            })
            .unwrap();
        let aggregate_root = root_path.join("aggregate-root");
        std::fs::create_dir_all(&aggregate_root).unwrap();
        let manifest = crate::product::logical_codebase::store::LogicalCodebaseManifest::new(
            "project_0001",
            aggregate_root,
            Vec::new(),
        );
        crate::product::logical_codebase::LogicalCodebaseStore::new(paths)
            .save_manifest("project_0001", &manifest)
            .unwrap();

        let runtime_root = root_path.clone();
        let factory = fake_registry_gateway_factory(ProductAppPaths::new(root_path.join(".aria")));
        let state = WebAppState::new(root_path.clone(), WebRuntime::new_fake(runtime_root))
            .with_aggregate_initialization_dependencies(build_test_dependencies(
                root_path,
                Some(factory),
            ));
        // Leak the temp dir for the test duration so the manifest stays on disk.
        std::mem::forget(root);
        build_web_router(state)
    }

    fn build_test_dependencies(
        root: std::path::PathBuf,
        factory: Option<Arc<LogicalCodebaseGatewayFactory>>,
    ) -> AggregateInitializationDependencies {
        struct TestSkills;
        #[async_trait::async_trait]
        impl AggregateSkillsPreparation for TestSkills {
            async fn prepare_skills(
                &self,
                _project_id: &str,
                _operation_id: &str,
                _cancellation: CancellationToken,
            ) -> Result<MachineSkillsPreparation, AggregateInitializationError> {
                Ok(MachineSkillsPreparation {
                    source_digest: "sha256:test-source".to_string(),
                    link_digest: "sha256:test-links".to_string(),
                    skills_root: PathBuf::from("/test-skills"),
                    warnings: Vec::new(),
                })
            }
        }
        let paths = ProductAppPaths::new(root.join(".aria"));
        let operations = AggregateInitializationOperationStore::new(paths.clone());
        let clock: Arc<dyn Fn() -> String + Send + Sync> =
            Arc::new(|| "2026-08-09T00:00:00Z".to_string());
        let preflight: Arc<dyn AggregatePreflightService> =
            Arc::new(DeterministicAggregatePreflightService::new(paths.clone()));
        let provider: Arc<dyn AggregateProviderTurnDriver> =
            Arc::new(GatewayFactoryProviderTurnDriver::new(factory, None));
        let coordinator = AggregateInitializationCoordinator::new(
            paths,
            operations,
            Arc::new(TestSkills),
            preflight,
            provider,
            clock,
        );
        AggregateInitializationDependencies::new(
            Arc::new(coordinator),
            InitializationRunRegistry::default(),
        )
    }

    /// 测试用 gateway factory:fake streaming provider + 恒可用 gate + stub sync
    /// adapter。ClaudeCode 路由到 `FakeStreamingProvider`,使 provider turn 经
    /// gateway 启动时无需真实 claude 二进制。
    fn fake_registry_gateway_factory(paths: ProductAppPaths) -> Arc<LogicalCodebaseGatewayFactory> {
        struct StubSyncAdapter;
        impl crate::cross_cutting::provider_adapter::ProviderAdapter for StubSyncAdapter {
            fn run(
                &self,
                _input: &crate::protocol::contracts::AdapterInput,
            ) -> Result<
                crate::protocol::contracts::AdapterOutput,
                crate::cross_cutting::provider_adapter::ProviderAdapterError,
            > {
                Ok(crate::protocol::contracts::AdapterOutput {
                    exit_code: Some(0),
                    stdout: String::new(),
                    stderr: String::new(),
                    structured_output: None,
                    files_modified: Vec::new(),
                    duration_ms: 0,
                    timeout_status: crate::protocol::contracts::TimeoutStatus::NotTimedOut,
                })
            }
        }

        struct AlwaysHealthy(Arc<crate::cross_cutting::provider_health::ProviderHealthSnapshot>);
        impl crate::cross_cutting::provider_availability_gate::ProviderHealthSource for AlwaysHealthy {
            fn snapshot(
                &self,
            ) -> Arc<crate::cross_cutting::provider_health::ProviderHealthSnapshot> {
                self.0.clone()
            }

            fn degraded(&self) -> bool {
                false
            }
        }

        use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
        use crate::cross_cutting::streaming_provider::FakeStreamingProvider;
        use crate::product::models::ProviderName;
        let checked_at = chrono::Utc::now();
        let gate = Arc::new(
            crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate::new(
                Arc::new(AlwaysHealthy(Arc::new(ProviderHealthSnapshot {
                    schema_version: 1,
                    generation: 1,
                    checked_at,
                    providers: [ProviderName::ClaudeCode, ProviderName::Codex]
                        .into_iter()
                        .map(|provider| ProviderHealthEntry {
                            provider,
                            command: "stub".to_string(),
                            available: true,
                            version: Some("1.0".to_string()),
                            reason_code: None,
                            reason: None,
                            checked_at,
                        })
                        .collect(),
                }))),
            ),
        );

        let mut registry = crate::cross_cutting::provider_registry::ProviderRegistry::new();
        registry.register(ProviderName::ClaudeCode, Arc::new(FakeStreamingProvider));

        Arc::new(LogicalCodebaseGatewayFactory::new(
            paths,
            Arc::new(registry),
            Arc::new(StubSyncAdapter),
            gate,
        ))
    }

    async fn post_json(
        app: &axum::Router,
        uri: &str,
        body: serde_json::Value,
    ) -> axum::http::Response<Body> {
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn production_dependencies_prepare_real_skills_and_reject_invalid_aggregate_root() {
        let root = tempdir().expect("root");
        let root_path = root.path().to_path_buf();
        let paths = ProductAppPaths::new(root_path.join(".aria"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "Aggregate initialization production test".to_string(),
                description: None,
            })
            .unwrap();
        let aggregate_root = root_path.join("aggregate-root");
        std::fs::create_dir_all(&aggregate_root).unwrap();
        let skills_source = root_path.join(".agents/Cadence-skills/cadence-init/skills/demo");
        std::fs::create_dir_all(&skills_source).unwrap();
        std::fs::write(skills_source.join("SKILL.md"), "# demo\n").unwrap();
        let manifest = crate::product::logical_codebase::store::LogicalCodebaseManifest::new(
            "project_0001",
            aggregate_root.join("missing-root"),
            Vec::new(),
        );
        crate::product::logical_codebase::LogicalCodebaseStore::new(paths)
            .save_manifest("project_0001", &manifest)
            .unwrap();
        let state = WebAppState::new(
            root_path.clone(),
            WebRuntime::new_fake(root_path.clone()),
        );
        let dependencies = AggregateInitializationDependencies::production(&state).unwrap();
        let shared_dependencies = state.aggregate_initialization_dependencies();
        let second_dependencies = state.aggregate_initialization_dependencies();
        assert!(Arc::ptr_eq(
            &shared_dependencies.coordinator,
            &second_dependencies.coordinator,
        ));
        let shared_run = InitializationRunKey::aggregate("project_0001", "lc_0001", "shared-run");
        let lease = shared_dependencies.runs.register(shared_run.clone()).unwrap();
        assert!(second_dependencies.runs.is_active(&shared_run));
        drop(lease);
        let input = crate::product::logical_codebase::AggregateInitializationOperationInput {
            idempotency_key: "production-test".to_string(),
            manifest_revision: 0,
            policy_digest: "sha256:policy".to_string(),
            profile_evidence_digest: None,
            provider_context_root: aggregate_root.join("missing-root"),
            provider: "claude_code".to_string(),
        };
        let operation = dependencies
            .coordinator()
            .begin("operation_0001".to_string(), "project_0001", input)
            .unwrap();
        let result = dependencies
            .coordinator()
            .execute("project_0001", &operation.operation_id, CancellationToken::new())
            .await;
        assert!(result.is_err());
        let saved = dependencies
            .coordinator()
            .get("project_0001", &operation.operation_id)
            .unwrap();
        assert_ne!(
            saved.steps[0].output_artifact_ref.as_deref(),
            Some("sha256:noop")
        );
        let machine_skills: MachineSkillsPreparation = serde_json::from_slice(
            &std::fs::read(
                root_path
                    .join(
                        ".aria/projects/logical-codebase/aggregate-initializations/operation_0001/machine_skills.json",
                    ),
            )
            .unwrap(),
        )
        .unwrap();
        assert_ne!(machine_skills.source_digest, "sha256:noop");
        assert_ne!(machine_skills.link_digest, "sha256:noop");
    }

    #[tokio::test]
    async fn aggregate_initialization_route_returns_pollable_operation_and_cancel_is_persistent() {
        let app = aggregate_init_test_app();
        let created = post_json(
            &app,
            "/api/projects/project_0001/logical-codebase/initializations",
            serde_json::json!({"idempotency_key":"init-1"}),
        )
        .await;
        assert_eq!(created.status(), StatusCode::ACCEPTED);
        let body = axum::body::to_bytes(created.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let operation_id = value["operation_id"]
            .as_str()
            .expect("operation_id")
            .to_string();
        assert_eq!(value["steps"].as_array().unwrap().len(), 5);

        let cancel_uri = format!(
            "/api/projects/project_0001/logical-codebase/initializations/{operation_id}/cancel"
        );
        let cancelled = post_json(
            &app,
            &cancel_uri,
            serde_json::json!({"reason":"user_cancelled"}),
        )
        .await;
        assert_eq!(cancelled.status(), StatusCode::OK);
        let body = axum::body::to_bytes(cancelled.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["status"], "cancelled");
        assert_eq!(value["cancellation"]["reason_code"], "user_cancelled");
    }

    #[tokio::test]
    async fn get_aggregate_initialization_polls_existing_operation() {
        let app = aggregate_init_test_app();
        let created = post_json(
            &app,
            "/api/projects/project_0001/logical-codebase/initializations",
            serde_json::json!({"idempotency_key":"init-2"}),
        )
        .await;
        assert_eq!(created.status(), StatusCode::ACCEPTED);
        let body = axum::body::to_bytes(created.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let operation_id = value["operation_id"]
            .as_str()
            .expect("operation_id")
            .to_string();

        let get_uri =
            format!("/api/projects/project_0001/logical-codebase/initializations/{operation_id}");
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(&get_uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn aggregate_initialization_provider_turn_launches_through_gateway_factory() {
        let root = tempdir().expect("root");
        let root_path = root.path().to_path_buf();
        let paths = ProductAppPaths::new(root_path.join(".aria"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "Aggregate initialization test".to_string(),
                description: None,
            })
            .unwrap();
        let aggregate_root = root_path.join("aggregate-root");
        std::fs::create_dir_all(&aggregate_root).unwrap();
        let manifest = crate::product::logical_codebase::store::LogicalCodebaseManifest::new(
            "project_0001",
            aggregate_root,
            Vec::new(),
        );
        crate::product::logical_codebase::LogicalCodebaseStore::new(paths.clone())
            .save_manifest("project_0001", &manifest)
            .unwrap();

        let factory = fake_registry_gateway_factory(paths);
        let dependencies = build_test_dependencies(root_path.clone(), Some(factory.clone()));
        let state = WebAppState::new(root_path.clone(), WebRuntime::new_fake(root_path.clone()))
            .with_aggregate_initialization_dependencies(dependencies.clone());
        std::mem::forget(root);
        let app = build_web_router(state);

        let created = post_json(
            &app,
            "/api/projects/project_0001/logical-codebase/initializations",
            serde_json::json!({"idempotency_key":"init-provider-turn"}),
        )
        .await;
        assert_eq!(created.status(), StatusCode::ACCEPTED);
        let body = axum::body::to_bytes(created.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let operation_id = value["operation_id"]
            .as_str()
            .expect("operation_id")
            .to_string();

        // Task 1.4 起 provider turn 会真实消费会话事件（等待完成的真实
        // 挂起点），POST 的后台 worker 与显式 execute 不再可能是同任务内
        // 的串行双跑——测试改为观察 durable operation 收敛，provider 启动
        // 仍由同一 gateway factory 驱动并经 audit 断言。
        let mut operation = dependencies
            .coordinator()
            .get("project_0001", &operation_id)
            .expect("operation is durable");
        for _ in 0..200 {
            if !matches!(
                operation.status,
                crate::product::logical_codebase::AggregateInitializationOperationStatus::Created
                    | crate::product::logical_codebase::AggregateInitializationOperationStatus::Running
            ) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            operation = dependencies
                .coordinator()
                .get("project_0001", &operation_id)
                .expect("operation is durable");
        }
        assert_eq!(
            operation.status,
            crate::product::logical_codebase::AggregateInitializationOperationStatus::Completed,
            "background worker must finish the five-step operation: {:?}",
            operation.error
        );

        // 三个 provider turn 经 factory 组装出的 gateway 启动,audit 计数为 3;
        // machine_skills / aggregate_preflight 是确定性 Cadence 代码,不产生启动。
        assert_eq!(factory.audit().stream_launches(), 3);
    }

    // -------------------------------------------------------------------
    // C4 Task 6：GET 纯投影 + canonical bootstrap GET/action 端点。
    // -------------------------------------------------------------------

    struct BootstrapTestApp {
        app: axum::Router,
        lc_id: String,
        root: std::path::PathBuf,
        events: crate::web::events::EventHub,
    }

    fn bootstrap_test_app() -> BootstrapTestApp {
        let root = tempdir().expect("root");
        let root_path = root.path().to_path_buf();
        let paths = ProductAppPaths::new(root_path.join(".aria"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "Bootstrap projection test".to_string(),
                description: None,
            })
            .unwrap();
        std::fs::create_dir_all(root_path.join("aggregate-root")).unwrap();
        let lc_id = crate::product::logical_codebase::LogicalCodebaseStore::new(paths.clone())
            .create(
                "project_0001",
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "bootstrap-lc".to_string(),
                    aggregate_root: root_path.join("aggregate-root"),
                },
            )
            .unwrap()
            .id;

        let factory = fake_registry_gateway_factory(ProductAppPaths::new(root_path.join(".aria")));
        // Task 9：EventHub 由测试持有句柄——订阅/无订阅两种投递形态都可观察。
        let events = crate::web::events::EventHub::new();
        let state =
            WebAppState::with_events(root_path.clone(), WebRuntime::new_fake(root_path.clone()), events.clone())
                .with_aggregate_initialization_dependencies(build_test_dependencies(
                    root_path.clone(),
                    Some(factory),
                ));
        std::mem::forget(root);
        BootstrapTestApp {
            app: build_web_router(state),
            lc_id,
            root: root_path,
            events,
        }
    }

    fn create_running_initialization(
        paths: &ProductAppPaths,
        lc_id: &str,
        operation_id: &str,
    ) {
        let store = AggregateInitializationOperationStore::for_lc(paths.clone(), lc_id);
        let operation =
            crate::product::logical_codebase::aggregate_initialization::AggregateInitializationOperation::new(
                operation_id.to_string(),
                "project_0001".to_string(),
                crate::product::logical_codebase::aggregate_initialization::AggregateInitializationOperationInput {
                    idempotency_key: format!("idem-{operation_id}"),
                    manifest_revision: 1,
                    policy_digest: "sha256:policy".to_string(),
                    profile_evidence_digest: None,
                    provider_context_root: paths
                        .logical_codebase_root("project_0001")
                        .join("aggregate-root"),
                    provider: "claude_code".to_string(),
                },
                "2026-09-29T00:00:00Z".to_string(),
            );
        store.create_idempotent(operation).unwrap();
        store
            .mark_running("project_0001", operation_id, "2026-09-29T00:01:00Z".to_string())
            .unwrap();
        // 按既有 V1 协议顺序推进：MachineSkills 完成后 preflight Running，
        // 模拟进程在 deterministic preflight 步骤中断。
        store
            .mark_step_running(
                "project_0001",
                operation_id,
                AggregateInitializationStepKind::MachineSkills,
                "sha256:skills-input".to_string(),
                "2026-09-29T00:01:30Z".to_string(),
            )
            .unwrap();
        store
            .checkpoint_step_output(
                "project_0001",
                operation_id,
                AggregateInitializationStepKind::MachineSkills,
                "aggregate-initializations/op/machine_skills.json".to_string(),
                "2026-09-29T00:01:40Z".to_string(),
            )
            .unwrap();
        store
            .mark_step_completed(
                "project_0001",
                operation_id,
                AggregateInitializationStepKind::MachineSkills,
                "2026-09-29T00:01:45Z".to_string(),
            )
            .unwrap();
        store
            .mark_step_running(
                "project_0001",
                operation_id,
                AggregateInitializationStepKind::AggregatePreflight,
                "sha256:preflight-input".to_string(),
                "2026-09-29T00:02:00Z".to_string(),
            )
            .unwrap();
    }

    async fn get_json(
        app: &axum::Router,
        uri: &str,
    ) -> (StatusCode, serde_json::Value) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        (status, serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null))
    }

    async fn response_json(response: axum::http::Response<Body>) -> (StatusCode, serde_json::Value) {
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        (status, serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null))
    }

    fn aria_inventory(root: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        let mut inventory = std::collections::BTreeMap::new();
        let mut stack = vec![root.join(".aria")];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let relative = path.strip_prefix(root).unwrap().to_path_buf();
                    inventory.insert(
                        relative.to_string_lossy().into_owned(),
                        std::fs::read(&path).unwrap_or_default(),
                    );
                }
            }
        }
        inventory
    }

    #[tokio::test]
    async fn get_running_initialization_does_not_recover_or_delete_staging() {
        let fixture = bootstrap_test_app();
        let paths = ProductAppPaths::new(fixture.root.join(".aria"));
        let operation_id = "aggregate_initialization_get_purity_0001".to_string();
        create_running_initialization(&paths, &fixture.lc_id, &operation_id);

        // 放置中断残留 staging：显式恢复动作应清理，GET 不得清理。
        let store = AggregateInitializationOperationStore::for_lc(
            paths.clone(),
            &fixture.lc_id,
        );
        let staging_root = store.staging_path("project_0001", &operation_id).unwrap();
        std::fs::create_dir_all(&staging_root).unwrap();
        std::fs::write(staging_root.join("partial-member-projection.json"), "{}").unwrap();

        let uri = format!(
            "/api/projects/project_0001/logical-codebases/{}/initializations/{operation_id}",
            fixture.lc_id
        );
        // 连续 GET 两次：operation 必须仍是 Running、staging 原样保留。
        for _ in 0..2 {
            let (status, body) = get_json(&fixture.app, &uri).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body["status"], "running", "GET must not recover the operation");
        }
        assert!(staging_root.join("partial-member-projection.json").exists());
        let operation = store.get("project_0001", &operation_id).unwrap();
        assert_eq!(
            operation.status,
            crate::product::logical_codebase::AggregateInitializationOperationStatus::Running
        );
        let preflight = operation
            .steps
            .iter()
            .find(|step| step.step_id == AggregateInitializationStepKind::AggregatePreflight)
            .unwrap();
        assert_eq!(
            preflight.status,
            crate::product::logical_codebase::aggregate_initialization::AggregateInitializationStepStatus::Running
        );

        // 显式 POST bootstrap action Continue 才把中断事实显式落盘，并返回
        // 允许的 retry 动作。
        let action_uri = format!(
            "/api/projects/project_0001/logical-codebases/{}/bootstrap/actions",
            fixture.lc_id
        );
        let response = post_json(
            &fixture.app,
            &action_uri,
            serde_json::json!({
                "command_id": "cmd-get-purity-continue-1",
                "step": "member_index",
                "action": "continue",
                "expected_revision": null,
                "expected_object_id": operation_id,
            }),
        )
        .await;
        let (status, body) = response_json(response).await;
        assert_eq!(status, StatusCode::OK, "explicit continue action: {body}");
        assert_eq!(body["outcome"], "accepted");
        let member_index = body["projection"]["steps"]
            .as_array()
            .unwrap()
            .iter()
            .find(|step| step["step"] == "member_index")
            .unwrap()
            .clone();
        assert_eq!(member_index["status"], "failed");
        assert_eq!(
            member_index["failure"]["reason_code"],
            "aggregate_initialization_interrupted"
        );
        assert!(
            member_index["allowed_actions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|action| action == "retry"),
            "explicit recovery must surface the retry action: {member_index}"
        );

        let operation = store.get("project_0001", &operation_id).unwrap();
        assert_eq!(
            operation.status,
            crate::product::logical_codebase::AggregateInitializationOperationStatus::Failed
        );
        assert_eq!(
            operation.error.as_ref().unwrap().reason_code,
            "aggregate_initialization_interrupted"
        );
        // 显式恢复按 recover_interrupted 语义清理 staging。
        assert!(!staging_root.exists());
    }

    #[tokio::test]
    async fn bootstrap_action_rejects_stale_revision_and_replays_same_command() {
        let fixture = bootstrap_test_app();
        let paths = ProductAppPaths::new(fixture.root.join(".aria"));
        let operation_id = "aggregate_initialization_replay_0001".to_string();
        create_running_initialization(&paths, &fixture.lc_id, &operation_id);

        // manifest 提供 durable membership revision。
        let manifest = crate::product::logical_codebase::store::LogicalCodebaseManifest::new(
            "project_0001",
            fixture.root.join("aggregate-root"),
            Vec::new(),
        );
        let lc_store = crate::product::logical_codebase::LogicalCodebaseStore::for_lc(
            paths.clone(),
            &fixture.lc_id,
        );
        lc_store.save_manifest("project_0001", &manifest).unwrap();
        let revision = lc_store
            .load_lc_manifest("project_0001", &fixture.lc_id)
            .unwrap()
            .expect("manifest persisted")
            .membership_revision;

        let action_uri = format!(
            "/api/projects/project_0001/logical-codebases/{}/bootstrap/actions",
            fixture.lc_id
        );
        let store = AggregateInitializationOperationStore::for_lc(
            paths.clone(),
            &fixture.lc_id,
        );

        // 过期 revision：409，不产生任何新 durable 事实。
        let stale = post_json(
            &fixture.app,
            &action_uri,
            serde_json::json!({
                "command_id": "cmd-stale-1",
                "step": "member_index",
                "action": "continue",
                "expected_revision": revision + 5,
                "expected_object_id": operation_id,
            }),
        )
        .await;
        let (status, body) = response_json(stale).await;
        assert_eq!(status, StatusCode::CONFLICT, "stale revision must 409: {body}");
        assert_eq!(body["code"], "bootstrap_stale_revision");
        let operation = store.get("project_0001", &operation_id).unwrap();
        assert_eq!(
            operation.status,
            crate::product::logical_codebase::AggregateInitializationOperationStatus::Running
        );

        // 正确 revision + 同 command 两次：第二次 outcome=replayed，durable
        // 状态不被第二次重复推进。
        let body = serde_json::json!({
            "command_id": "cmd-bootstrap-replay-1",
            "step": "member_index",
            "action": "continue",
            "expected_revision": revision,
            "expected_object_id": operation_id,
        });
        let first = response_json(post_json(&fixture.app, &action_uri, body.clone()).await).await;
        assert_eq!(first.0, StatusCode::OK, "{}", first.1);
        assert_eq!(first.1["outcome"], "accepted");

        let second = response_json(post_json(&fixture.app, &action_uri, body).await).await;
        assert_eq!(second.0, StatusCode::OK, "{}", second.1);
        assert_eq!(second.1["outcome"], "replayed");
        assert_eq!(
            second.1["projection"]["steps"]
                .as_array()
                .unwrap()
                .iter()
                .find(|step| step["step"] == "member_index")
                .unwrap()["object_id"],
            first.1["projection"]["steps"]
                .as_array()
                .unwrap()
                .iter()
                .find(|step| step["step"] == "member_index")
                .unwrap()["object_id"]
        );
        // 第二次同 command 不再次推进 durable 状态（仍 Failed，未被 reopen）。
        let operation = store.get("project_0001", &operation_id).unwrap();
        assert_eq!(
            operation.status,
            crate::product::logical_codebase::AggregateInitializationOperationStatus::Failed
        );
    }

    #[tokio::test]
    async fn get_details_members_policy_and_index_never_start_side_effects() {
        let fixture = bootstrap_test_app();
        let paths = ProductAppPaths::new(fixture.root.join(".aria"));
        let operation_id = "aggregate_initialization_get_effects_0001".to_string();
        create_running_initialization(&paths, &fixture.lc_id, &operation_id);

        // 真实成员仓 + member/checkout + policy + Building index：GET 只读这些
        // durable 事实，不得启动 CLI/provider 或写任何 store。
        let repo = fixture.root.join("aggregate-root").join("repo-a");
        std::fs::create_dir_all(&repo).unwrap();
        let output = std::process::Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(&repo)
            .output()
            .unwrap();
        assert!(output.status.success());
        std::fs::write(repo.join("README.md"), "# member\n").unwrap();

        let git = |arguments: &[&str]| {
            let output = std::process::Command::new("git")
                .args(arguments)
                .current_dir(&repo)
                .output()
                .unwrap();
            assert!(output.status.success(), "git {arguments:?} failed");
            String::from_utf8_lossy(&output.stdout).into_owned()
        };
        git(&["config", "user.email", "get-purity@test.local"]);
        git(&["config", "user.name", "GetPurity Test"]);
        git(&["add", "."]);
        git(&["commit", "-m", "init"]);
        let head_before = git(&["rev-parse", "HEAD"]);

        let canonical = std::fs::canonicalize(&repo).unwrap();
        let source = crate::product::repository_store::resolve_repository_source(&canonical).unwrap();
        let member_id = crate::product::logical_codebase::LogicalRepositoryId(uuid::Uuid::new_v4());
        let checkout_id =
            crate::product::logical_codebase::RepositoryCheckoutId(uuid::Uuid::new_v4());
        let lc_store = crate::product::logical_codebase::LogicalCodebaseStore::for_lc(
            paths.clone(),
            &fixture.lc_id,
        );
        let mut manifest = crate::product::logical_codebase::store::LogicalCodebaseManifest::new(
            "project_0001",
            fixture.root.join("aggregate-root"),
            Vec::new(),
        );
        manifest.member_ids = vec![member_id];
        lc_store.save_manifest("project_0001", &manifest).unwrap();
        lc_store
            .save_member(
                "project_0001",
                &crate::product::logical_codebase::types::CodebaseMemberRecord {
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_member_a".to_string(),
                    alias: "repo-a".to_string(),
                    role: "member".to_string(),
                    ordinal: 1,
                    source_identity: source.clone(),
                    repo_type: crate::product::logical_codebase::types::RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_id],
                    status: crate::product::logical_codebase::types::MemberStatus::Active,
                    created_at: "2026-09-29T00:00:00Z".to_string(),
                    updated_at: "2026-09-29T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        lc_store
            .save_checkout(
                "project_0001",
                &crate::product::logical_codebase::types::RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_member_a".to_string(),
                    kind: crate::product::logical_codebase::types::CheckoutKind::Main,
                    canonical_path: canonical.clone(),
                    checkout_path_hash: crate::product::id::repo_hash_for_path(
                        canonical.to_string_lossy().as_ref(),
                    ),
                    git_dir_identity: source.git_dir_identity(),
                    revision: None,
                    availability: crate::product::logical_codebase::types::CheckoutAvailability::Available,
                    observed_at: "2026-09-29T00:00:00Z".to_string(),
                    created_at: "2026-09-29T00:00:00Z".to_string(),
                    updated_at: "2026-09-29T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::for_lc(
            paths.clone(),
            &fixture.lc_id,
        )
        .save(
            "project_0001",
            &crate::product::logical_codebase::policy::AggregatePolicyArtifact::bootstrap(
                "project_0001",
                &manifest.logical_codebase_id.to_string(),
                "2026-09-29T00:00:00Z".to_string(),
            ),
        )
        .unwrap();
        crate::product::logical_codebase::aggregate_index::AggregateIndexStore::for_lc(
            paths.clone(),
            &fixture.lc_id,
        )
        .create(
            "project_0001",
            crate::product::logical_codebase::aggregate_index::AggregateIndexRecord::building(
                "aggregate_index_get_effects_0001".to_string(),
                "project_0001".to_string(),
                manifest.membership_revision,
                Vec::new(),
                "2026-09-29T00:00:00Z".to_string(),
            ),
        )
        .unwrap();

        let before = aria_inventory(&fixture.root);
        let base = format!(
            "/api/projects/project_0001/logical-codebases/{}",
            fixture.lc_id
        );
        for uri in [
            base.clone(),
            format!("{base}/members"),
            format!("{base}/bootstrap"),
            format!("{base}/aggregate-indexes/active"),
            format!("{base}/initializations/{operation_id}"),
        ] {
            for _ in 0..2 {
                let (status, body) = get_json(&fixture.app, &uri).await;
                assert_eq!(status, StatusCode::OK, "GET {uri} must be readable: {body}");
            }
        }
        let after = aria_inventory(&fixture.root);
        assert_eq!(
            before, after,
            "GET details/members/bootstrap/index/initialization must not write durable bytes"
        );

        // 成员仓 HEAD/dirty 不变。
        let head_after = git(&["rev-parse", "HEAD"]);
        assert_eq!(head_before, head_after);
        let dirty = git(&["status", "--porcelain"]);
        assert!(dirty.trim().is_empty(), "member repo must stay clean: {dirty}");
    }

    /// Task 9 Step 1：把 durable 的 index 首建 Failed generation 放到磁盘。
    fn create_failed_aggregate_index(paths: &ProductAppPaths, lc_id: &str, index_id: &str) {
        let mut record =
            crate::product::logical_codebase::aggregate_index::AggregateIndexRecord::building(
                index_id.to_string(),
                "project_0001".to_string(),
                1,
                Vec::new(),
                "2026-09-29T00:00:00Z".to_string(),
            );
        record.status = crate::product::logical_codebase::aggregate_index::AggregateIndexStatus::Failed;
        record.warning = Some("first build failed: coverage assertion rejected".to_string());
        record.command_id = Some(format!("cmd-index-{index_id}"));
        crate::product::logical_codebase::aggregate_index::AggregateIndexStore::for_lc(
            paths.clone(),
            lc_id,
        )
        .create("project_0001", record)
        .unwrap();
    }

    #[tokio::test]
    async fn durable_bootstrap_failure_is_readable_when_event_delivery_fails() {
        let fixture = bootstrap_test_app();
        let paths = ProductAppPaths::new(fixture.root.join(".aria"));
        let index_id = "aggregate_index_delivery_failure_0001".to_string();
        create_failed_aggregate_index(&paths, &fixture.lc_id, &index_id);

        // EventHub 无订阅者：broadcast 投递失败。显式 Retry 动作仍必须完成
        // durable 语义（同 command replay），事实不被投递失败回滚。
        let action_uri = format!(
            "/api/projects/project_0001/logical-codebases/{}/bootstrap/actions",
            fixture.lc_id
        );
        let action = serde_json::json!({
            "command_id": format!("cmd-index-{index_id}"),
            "step": "aggregate_index_active",
            "action": "retry",
            "expected_object_id": index_id,
        });
        let (status, body) =
            response_json(post_json(&fixture.app, &action_uri, action).await).await;
        assert_eq!(status, StatusCode::OK, "delivery failure must not roll back: {body}");
        let posted_notices = body["projection"]["notices"].clone();
        assert!(
            posted_notices.as_array().is_some_and(|notices| !notices.is_empty()),
            "failed index must project a durable notice: {posted_notices}"
        );

        // GET 补读：与 POST 响应携带同一 notice key/reason/object/allowed
        // actions；重复 GET 结果一致（同 durable 事实，无重执行）。
        let bootstrap_uri = format!(
            "/api/projects/project_0001/logical-codebases/{}/bootstrap",
            fixture.lc_id
        );
        let before = aria_inventory(&fixture.root);
        let (first_status, first_body) = get_json(&fixture.app, &bootstrap_uri).await;
        assert_eq!(first_status, StatusCode::OK);
        assert_eq!(first_body["notices"], posted_notices);
        let (second_status, second_body) = get_json(&fixture.app, &bootstrap_uri).await;
        assert_eq!(second_status, StatusCode::OK);
        assert_eq!(second_body["notices"], first_body["notices"]);
        assert_eq!(second_body["planning_ready"], false);
        let after = aria_inventory(&fixture.root);
        assert_eq!(before, after, "GET re-reads must not re-execute any step");

        // durable generation 保持 Failed（通知投递失败≠可重试成功）。
        let record = crate::product::logical_codebase::aggregate_index::AggregateIndexStore::for_lc(
            paths,
            &fixture.lc_id,
        )
        .get("project_0001", &index_id)
        .unwrap()
        .expect("failed generation persisted");
        assert_eq!(
            record.status,
            crate::product::logical_codebase::aggregate_index::AggregateIndexStatus::Failed
        );
    }

    #[tokio::test]
    async fn event_is_published_after_durable_failure_and_contains_action_context() {
        let fixture = bootstrap_test_app();
        let paths = ProductAppPaths::new(fixture.root.join(".aria"));
        let index_id = "aggregate_index_publish_context_0001".to_string();
        create_failed_aggregate_index(&paths, &fixture.lc_id, &index_id);

        // 先订阅，再触发失败步骤上的显式动作。
        let mut receiver = fixture.events.subscribe();
        let action_uri = format!(
            "/api/projects/project_0001/logical-codebases/{}/bootstrap/actions",
            fixture.lc_id
        );
        let (status, body) = response_json(
            post_json(
                &fixture.app,
                &action_uri,
                serde_json::json!({
                    "command_id": format!("cmd-index-{index_id}"),
                    "step": "aggregate_index_active",
                    "action": "retry",
                    "expected_object_id": index_id,
                }),
            )
            .await,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");

        // 顺序契约：durable failure 先可读，通知后到。
        let record = crate::product::logical_codebase::aggregate_index::AggregateIndexStore::for_lc(
            paths.clone(),
            &fixture.lc_id,
        )
        .get("project_0001", &index_id)
        .unwrap()
        .expect("failed generation persisted before event");
        assert_eq!(
            record.status,
            crate::product::logical_codebase::aggregate_index::AggregateIndexStatus::Failed
        );

        let event = tokio::time::timeout(std::time::Duration::from_secs(2), receiver.recv())
            .await
            .expect("event must be published")
            .expect("subscription alive");
        assert_eq!(
            event.event_type,
            crate::web::events::WebEventType::ProjectionUpdated.as_str()
        );
        let payload = event.payload;
        assert_eq!(payload["scope"], "logical_codebase_bootstrap");
        assert_eq!(payload["logical_codebase_id"], fixture.lc_id);
        assert_eq!(payload["outcome"], body["outcome"]);
        assert_eq!(payload["planning_ready"], false);
        let notice = &payload["notice"];
        assert_eq!(notice["step"], "aggregate_index_active");
        assert_eq!(notice["object_id"], index_id);
        assert_eq!(notice["reason_code"], "aggregate_index_failed");
        assert_eq!(
            notice["allowed_actions"],
            serde_json::json!(["retry"]),
            "notice must carry the durable allowed actions"
        );
        // aggregate_index_active 是最后一步：没有 next_step 可宣称。
        assert!(notice["next_step"].is_null());
        // 通知不携带任何未持久化的成功宣称（durable 记录仍 Failed）。
        assert_ne!(payload["outcome"], "completed");
        assert_eq!(body["projection"]["notices"][0]["reason_code"], "aggregate_index_failed");
    }
