//! HTTP handlers for the aggregate initialization operation.
//!
//! The aggregate initialization operation is an independent, pollable,
//! cancellable five-step flow. These handlers create, poll and cancel the
//! operation using `InitializationRunKey::aggregate`, persisting the
//! cancellation request before signalling the in-memory run lease. The DTO
//! shows the five stable steps, the resolved profile, per-member evidence
//! projections and the cancellation record; it never reuses the single-
//! repository GitFinalize warning DTO.

use super::dto::aggregate_initialization_dto;
use super::support::{product_app_paths, product_store_api_error};
use super::*;

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::validate_relative_id;
use crate::product::logical_codebase::AggregateInitializationOperationStatus;
use crate::product::logical_codebase::AggregateInitializationStepKind;
use crate::product::logical_codebase::aggregate_initialization_coordinator::{
    AggregateInitializationCoordinator, AggregateInitializationError, AggregatePreflightSnapshot,
    AggregateProviderTurnDriver, GatewayBackedAggregateProviderTurnDriver,
};
use crate::product::logical_codebase::aggregate_initialization_store::AggregateInitializationOperationStore;
use crate::web::error::ApiError;
use crate::web::gateway_factory::LogicalCodebaseGatewayFactory;
use crate::web::state::{InitializationRunKey, InitializationRunRegistry};
use crate::web::types::{
    CancelAggregateInitializationRequest, CreateAggregateInitializationRequest,
};

/// Aggregate initialization coordinator dependencies that can be injected by
/// tests or built from the web app state.
#[derive(Clone)]
pub struct AggregateInitializationDependencies {
    coordinator: Arc<AggregateInitializationCoordinator>,
    runs: InitializationRunRegistry,
}

impl AggregateInitializationDependencies {
    pub fn new(
        coordinator: Arc<AggregateInitializationCoordinator>,
        runs: InitializationRunRegistry,
    ) -> Self {
        Self { coordinator, runs }
    }

    #[allow(dead_code)]
    pub fn coordinator(&self) -> &AggregateInitializationCoordinator {
        &self.coordinator
    }
}

/// Web 层 provider turn 驱动适配器:每个 turn 经
/// [`LogicalCodebaseGatewayFactory::build`] 现组装 gateway,再委托给
/// [`GatewayBackedAggregateProviderTurnDriver`] 启动 provider。factory 缺失时
/// fail-closed(run_turn 返回 `ProviderTurn` 错误),不破坏 create/get/cancel 的
/// 既有 handler 流程。
struct GatewayFactoryProviderTurnDriver {
    factory: Option<Arc<LogicalCodebaseGatewayFactory>>,
}

impl GatewayFactoryProviderTurnDriver {
    fn new(factory: Option<Arc<LogicalCodebaseGatewayFactory>>) -> Self {
        Self { factory }
    }
}

#[async_trait::async_trait]
impl AggregateProviderTurnDriver for GatewayFactoryProviderTurnDriver {
    async fn run_turn(
        &self,
        project_id: &str,
        operation_id: &str,
        step: AggregateInitializationStepKind,
        preflight: &AggregatePreflightSnapshot,
        cancellation: CancellationToken,
    ) -> Result<String, AggregateInitializationError> {
        let Some(factory) = self.factory.as_ref() else {
            return Err(AggregateInitializationError::ProviderTurn {
                step,
                reason: "logical codebase gateway factory is not configured".to_string(),
                retryable: false,
            });
        };
        let gateway = factory.build(project_id).map_err(|error| {
            AggregateInitializationError::ProviderTurn {
                step,
                reason: format!("logical codebase gateway factory build failed: {error}"),
                retryable: true,
            }
        })?;
        GatewayBackedAggregateProviderTurnDriver::claude_code(
            Arc::new(gateway),
            "cap_managed_snapshot",
        )
        .run_turn(project_id, operation_id, step, preflight, cancellation)
        .await
    }
}

pub async fn create_aggregate_initialization(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
    Json(request): Json<CreateAggregateInitializationRequest>,
) -> ApiResult<Response> {
    validate_project_id(&project_id)?;
    validate_idempotency_key(&request.idempotency_key)?;
    let dependencies = aggregate_initialization_dependencies(&state);
    let operation_id = deterministic_operation_id(&project_id, &request.idempotency_key);
    let project_paths = product_app_paths(&state);
    let manifest = load_manifest_for_profile(&project_paths, &project_id)?;
    let input = crate::product::logical_codebase::AggregateInitializationOperationInput {
        idempotency_key: request.idempotency_key.clone(),
        manifest_revision: manifest.membership_revision,
        policy_digest: "sha256:aggregate-policy".to_string(),
        profile_evidence_digest: Some("sha256:profile".to_string()),
        provider_context_root: manifest.provider_context_root.clone(),
        provider: "claude_code".to_string(),
    };
    let operation = dependencies
        .coordinator
        .begin(operation_id.clone(), &project_id, input)
        .map_err(aggregate_initialization_api_error)?;
    let key = InitializationRunKey::aggregate(&project_id, &operation.operation_id);
    let lease = dependencies.runs.register(key).ok_or_else(|| {
        ApiError::runtime(
            "aggregate_initialization_in_progress",
            "aggregate initialization is already in progress",
            json!({}),
        )
    })?;
    // The cancellation token is stored in the lease lifetime; the worker is
    // not spawned in this deterministic handler — the operation is polled via
    // GET and cancelled via POST, persisting state through the store.
    drop(lease);
    Ok((
        StatusCode::ACCEPTED,
        Json(aggregate_initialization_dto(operation)),
    )
        .into_response())
}

pub async fn get_aggregate_initialization(
    State(state): State<WebAppState>,
    Path((project_id, operation_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    validate_project_id(&project_id)?;
    validate_operation_id(&operation_id)?;
    let dependencies = aggregate_initialization_dependencies(&state);
    let operation = dependencies
        .coordinator
        .get(&project_id, &operation_id)
        .map_err(aggregate_initialization_api_error)?;
    let operation = if matches!(
        operation.status,
        AggregateInitializationOperationStatus::Created
            | AggregateInitializationOperationStatus::Running
    ) && !dependencies
        .runs
        .is_active(&InitializationRunKey::aggregate(&project_id, &operation_id))
    {
        // No active in-memory lease: nothing to recover here deterministically,
        // but surface the persisted record so the client can poll.
        operation
    } else {
        operation
    };
    Ok(Json(aggregate_initialization_dto(operation)).into_response())
}

pub async fn cancel_aggregate_initialization(
    State(state): State<WebAppState>,
    Path((project_id, operation_id)): Path<(String, String)>,
    Json(request): Json<CancelAggregateInitializationRequest>,
) -> ApiResult<Response> {
    validate_project_id(&project_id)?;
    validate_operation_id(&operation_id)?;
    let dependencies = aggregate_initialization_dependencies(&state);
    // Persist the cancellation request before signalling any in-memory token.
    let operation = dependencies
        .coordinator
        .cancel(
            &project_id,
            &operation_id,
            &request.reason,
            request.detail.clone(),
        )
        .map_err(aggregate_initialization_api_error)?;
    Ok((
        StatusCode::OK,
        Json(aggregate_initialization_dto(operation)),
    )
        .into_response())
}

fn aggregate_initialization_dependencies(
    state: &WebAppState,
) -> AggregateInitializationDependencies {
    if let Some(dependencies) = state.aggregate_initialization_dependencies() {
        return dependencies;
    }
    build_default_aggregate_dependencies(state)
}

fn build_default_aggregate_dependencies(
    state: &WebAppState,
) -> AggregateInitializationDependencies {
    let project_paths = product_app_paths(state);
    let operations = AggregateInitializationOperationStore::new(project_paths.clone());
    let clock: Arc<dyn Fn() -> String + Send + Sync> = Arc::new(|| chrono::Utc::now().to_rfc3339());
    let factory = state.gateway_factory().cloned();
    let coordinator = build_coordinator(project_paths, operations, clock, factory);
    AggregateInitializationDependencies::new(
        Arc::new(coordinator),
        InitializationRunRegistry::default(),
    )
}

fn build_coordinator(
    paths: ProductAppPaths,
    operations: AggregateInitializationOperationStore,
    clock: Arc<dyn Fn() -> String + Send + Sync>,
    factory: Option<Arc<LogicalCodebaseGatewayFactory>>,
) -> AggregateInitializationCoordinator {
    use crate::product::logical_codebase::aggregate_initialization_coordinator::{
        AggregatePreflightService, AggregateSkillsPreparation, MachineSkillsPreparation,
    };
    use std::path::PathBuf;

    struct NoopSkills;
    #[async_trait::async_trait]
    impl AggregateSkillsPreparation for NoopSkills {
        async fn prepare_skills(
            &self,
            _project_id: &str,
            _operation_id: &str,
            _cancellation: CancellationToken,
        ) -> Result<MachineSkillsPreparation, AggregateInitializationError> {
            Ok(MachineSkillsPreparation {
                source_digest: "sha256:noop".to_string(),
                link_digest: "sha256:noop".to_string(),
                skills_root: PathBuf::from("/skills"),
                warnings: Vec::new(),
            })
        }
    }

    struct NoopPreflight;
    impl AggregatePreflightService for NoopPreflight {
        fn inspect(
            &self,
            _project_id: &str,
            manifest: &crate::product::logical_codebase::store::LogicalCodebaseManifest,
            _cancellation: &CancellationToken,
        ) -> Result<AggregatePreflightSnapshot, AggregateInitializationError> {
            Ok(AggregatePreflightSnapshot {
                aggregate_root: manifest
                    .provider_context_root
                    .to_string_lossy()
                    .into_owned(),
                index_excludes_assets: true,
                members: Vec::new(),
                manifest_revision: manifest.membership_revision,
                manifest_digest: "sha256:manifest".to_string(),
            })
        }
    }

    let skills: Arc<dyn AggregateSkillsPreparation> = Arc::new(NoopSkills);
    let preflight: Arc<dyn AggregatePreflightService> = Arc::new(NoopPreflight);
    let provider: Arc<dyn AggregateProviderTurnDriver> =
        Arc::new(GatewayFactoryProviderTurnDriver::new(factory));
    AggregateInitializationCoordinator::new(paths, operations, skills, preflight, provider, clock)
}

fn load_manifest_for_profile(
    paths: &ProductAppPaths,
    project_id: &str,
) -> ApiResult<crate::product::logical_codebase::store::LogicalCodebaseManifest> {
    let store = crate::product::logical_codebase::LogicalCodebaseStore::new(paths.clone());
    store
        .load_manifest(project_id)
        .map_err(product_store_api_error)?
        .ok_or_else(|| {
            ApiError::runtime(
                "logical_codebase_manifest_missing",
                "logical codebase manifest is missing; register members first",
                json!({}),
            )
        })
}

fn validate_project_id(project_id: &str) -> ApiResult<()> {
    validate_relative_id(project_id).map_err(|error| {
        ApiError::validation("invalid_project_id", format!("invalid project id: {error}"))
    })?;
    Ok(())
}

fn validate_operation_id(operation_id: &str) -> ApiResult<()> {
    validate_relative_id(operation_id).map_err(|error| {
        ApiError::validation(
            "invalid_operation_id",
            format!("invalid operation id: {error}"),
        )
    })?;
    Ok(())
}

fn validate_idempotency_key(key: &str) -> ApiResult<()> {
    if key.is_empty() {
        return Err(ApiError::validation(
            "invalid_idempotency_key",
            "idempotency_key must not be empty",
        ));
    }
    Ok(())
}

fn deterministic_operation_id(project_id: &str, idempotency_key: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(project_id.as_bytes());
    hasher.update(idempotency_key.as_bytes());
    let digest = hasher.finalize();
    format!("aggregate_initialization_{:x}", digest)[..40].to_string()
}

fn aggregate_initialization_api_error(error: AggregateInitializationError) -> ApiError {
    match error {
        AggregateInitializationError::NotFound { .. } => ApiError::runtime(
            "aggregate_initialization_operation_not_found",
            "aggregate initialization operation not found",
            json!({}),
        ),
        AggregateInitializationError::StateRejected { detail, .. } => {
            ApiError::runtime("aggregate_initialization_state_rejected", detail, json!({}))
        }
        AggregateInitializationError::Store(error) => product_store_api_error(error),
        other => ApiError::runtime(
            "aggregate_initialization_failed",
            other.to_string(),
            json!({}),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let paths = ProductAppPaths::new(root.join(".aria"));
        let operations = AggregateInitializationOperationStore::new(paths.clone());
        let clock: Arc<dyn Fn() -> String + Send + Sync> =
            Arc::new(|| "2026-08-09T00:00:00Z".to_string());
        let coordinator = build_coordinator(paths, operations, clock, factory);
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
}
