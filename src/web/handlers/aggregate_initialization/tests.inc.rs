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
                multi_repo: true,
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
            Arc::new(GatewayFactoryProviderTurnDriver::new(factory));
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
                multi_repo: true,
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
        let shared_run = InitializationRunKey::aggregate("project_0001", "shared-run");
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
                multi_repo: true,
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

        dependencies
            .coordinator()
            .execute("project_0001", &operation_id, CancellationToken::new())
            .await
            .expect("aggregate initialization should execute through the gateway factory");

        // 三个 provider turn 经 factory 组装出的 gateway 启动,audit 计数为 3;
        // machine_skills / aggregate_preflight 是确定性 Cadence 代码,不产生启动。
        assert_eq!(factory.audit().stream_launches(), 3);
    }
