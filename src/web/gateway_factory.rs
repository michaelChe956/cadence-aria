//! Web 层 `LogicalCodebaseGatewayFactory`:为指定 project 组装 gateway。

use std::sync::Arc;

use crate::cross_cutting::provider_adapter::ProviderAdapter;
use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::product::app_paths::ProductAppPaths;
use crate::product::logical_codebase::{
    AggregatePolicyArtifactStore, GatewayRunAudit, LogicalCodebaseProviderGateway,
    LogicalCodebaseStore, ProductionPolicyTargetResolver, ProviderCapabilityStore,
    ProviderGatewayError, StoreBackedProviderCapabilitySource,
};

/// 为指定 project 组装 `LogicalCodebaseProviderGateway` 的 Web 层工厂。
///
/// 持有持久化路径、provider registry、同步 adapter、availability gate 与共享启动
/// 审计。`build` 会先 bootstrap policy 与 capability,再用 `with_audit` 组装 gateway,
/// 使同一 factory 构造出的多个 gateway 共享同一份 `GatewayRunAudit`。
pub struct LogicalCodebaseGatewayFactory {
    paths: ProductAppPaths,
    registry: Arc<ProviderRegistry>,
    sync_adapter: Arc<dyn ProviderAdapter + Send + Sync>,
    availability_gate: Arc<ProviderAvailabilityGate>,
    audit: Arc<GatewayRunAudit>,
}

impl LogicalCodebaseGatewayFactory {
    pub fn new(
        paths: ProductAppPaths,
        registry: Arc<ProviderRegistry>,
        sync_adapter: Arc<dyn ProviderAdapter + Send + Sync>,
        availability_gate: Arc<ProviderAvailabilityGate>,
    ) -> Self {
        Self {
            paths,
            registry,
            sync_adapter,
            availability_gate,
            audit: Arc::new(GatewayRunAudit::new()),
        }
    }

    pub fn audit(&self) -> Arc<GatewayRunAudit> {
        self.audit.clone()
    }

    /// 为指定 project 构造 gateway:ensure_bootstrap(policy + capability) 后
    /// `with_audit` 组装。缺失 manifest 时 fail-closed 为 `PolicyMissing`。
    pub fn build(
        &self,
        project_id: &str,
    ) -> Result<LogicalCodebaseProviderGateway, ProviderGatewayError> {
        let manifest = LogicalCodebaseStore::new(self.paths.clone())
            .load_manifest(project_id)?
            .ok_or_else(|| ProviderGatewayError::PolicyMissing(project_id.to_string()))?;

        let policies = AggregatePolicyArtifactStore::new(self.paths.clone());
        policies.ensure_bootstrap(&manifest)?;

        ProviderCapabilityStore::new(self.paths.clone()).ensure_bootstrap(project_id)?;

        Ok(LogicalCodebaseProviderGateway::with_audit(
            policies,
            Arc::new(StoreBackedProviderCapabilitySource::new(
                self.paths.clone(),
                project_id.to_string(),
            )),
            Arc::new(ProductionPolicyTargetResolver::new(self.paths.clone())),
            self.registry.clone(),
            self.sync_adapter.clone(),
            self.availability_gate.clone(),
            self.audit.clone(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;

    use chrono::Utc;
    use tempfile::tempdir;

    use crate::cross_cutting::provider_adapter::ProviderAdapter;
    use crate::cross_cutting::provider_availability_gate::{
        ProviderAvailabilityGate, ProviderHealthSource,
    };
    use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
    use crate::cross_cutting::provider_registry::ProviderRegistry;
    use crate::cross_cutting::streaming_provider::{
        ProviderSession, StreamingProviderAdapter, StreamingProviderInput,
    };
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::{
        LogicalCodebaseManifest, LogicalCodebaseStore, PolicyTarget, ProviderGatewayError,
        ProviderRef, SessionLaunchRequest,
    };
    use crate::product::models::ProviderName;
    use crate::protocol::contracts::{AdapterOutput, TimeoutStatus};

    struct StubSyncAdapter;

    impl ProviderAdapter for StubSyncAdapter {
        fn run(
            &self,
            _input: &crate::protocol::contracts::AdapterInput,
        ) -> Result<AdapterOutput, crate::cross_cutting::provider_adapter::ProviderAdapterError>
        {
            Ok(AdapterOutput {
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
                structured_output: None,
                files_modified: Vec::new(),
                duration_ms: 0,
                timeout_status: TimeoutStatus::NotTimedOut,
            })
        }
    }

    struct NoopStreamingAdapter;

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for NoopStreamingAdapter {
        async fn start(
            &self,
            _input: StreamingProviderInput,
            _cancel: tokio_util::sync::CancellationToken,
        ) -> Result<ProviderSession, crate::cross_cutting::provider_adapter::ProviderAdapterError>
        {
            let (_event_tx, events) = tokio::sync::mpsc::channel(1);
            let (commands, _command_rx) = tokio::sync::mpsc::channel(1);
            Ok(ProviderSession { events, commands })
        }
    }

    fn fake_registry() -> Arc<ProviderRegistry> {
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::ClaudeCode, Arc::new(NoopStreamingAdapter));
        Arc::new(registry)
    }

    fn always_available_gate() -> Arc<ProviderAvailabilityGate> {
        struct AlwaysHealthy(Arc<ProviderHealthSnapshot>);

        impl ProviderHealthSource for AlwaysHealthy {
            fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
                self.0.clone()
            }

            fn degraded(&self) -> bool {
                false
            }
        }

        let checked_at = Utc::now();
        let snapshot = Arc::new(ProviderHealthSnapshot {
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
        });
        Arc::new(ProviderAvailabilityGate::new(Arc::new(AlwaysHealthy(
            snapshot,
        ))))
    }

    fn factory_fixture() -> (
        tempfile::TempDir,
        ProductAppPaths,
        Arc<LogicalCodebaseGatewayFactory>,
    ) {
        let root = tempdir().expect("temporary product root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let factory = Arc::new(LogicalCodebaseGatewayFactory::new(
            paths.clone(),
            fake_registry(),
            Arc::new(StubSyncAdapter),
            always_available_gate(),
        ));
        (root, paths, factory)
    }

    fn register_manifest(paths: &ProductAppPaths, project_id: &str) {
        let manifest = LogicalCodebaseManifest::new(project_id, paths.root().to_path_buf(), vec![]);
        LogicalCodebaseStore::new(paths.clone())
            .save_manifest(project_id, &manifest)
            .expect("save manifest");
    }

    #[test]
    fn build_assembles_gateway_and_shares_audit_instance() {
        let (_root, paths, factory) = factory_fixture();
        register_manifest(&paths, "project_0001");

        let gateway = factory.build("project_0001").unwrap();

        let audit = factory.audit();
        assert!(Arc::ptr_eq(&audit, &gateway.audit()));

        let aggregate_root = paths.root().join("aggregate");
        std::fs::create_dir_all(&aggregate_root).expect("create aggregate root");
        let request = SessionLaunchRequest::planning(
            "project_0001",
            ProviderRef::claude_code("cap_managed_snapshot"),
            PolicyTarget::aggregate_root(aggregate_root),
            vec![paths.root().to_path_buf()],
            "sha256:managed-config-artifact",
        );
        let validated = gateway.validate(request).unwrap();
        assert_eq!(validated.envelope().policy_revision, 1);
    }

    #[test]
    fn build_returns_policy_missing_for_unregistered_project() {
        let (_root, _paths, factory) = factory_fixture();

        let error = match factory.build("project_missing") {
            Err(error) => error,
            Ok(_) => panic!("expected PolicyMissing, got a gateway"),
        };

        assert!(matches!(
            error,
            ProviderGatewayError::PolicyMissing(ref id) if id == "project_missing"
        ));
    }

    #[test]
    fn web_app_state_installs_factory_and_supports_injection() {
        let root = tempdir().expect("root");
        let state = crate::web::state::WebAppState::new(
            root.path().to_path_buf(),
            crate::web::runtime::WebRuntime::new_fake(root.path().to_path_buf()),
        );
        assert!(state.gateway_factory().is_some());

        let injected = Arc::new(LogicalCodebaseGatewayFactory::new(
            ProductAppPaths::new(root.path().join(".aria")),
            fake_registry(),
            Arc::new(StubSyncAdapter),
            always_available_gate(),
        ));
        let state = state.with_gateway_factory(injected.clone());
        assert!(Arc::ptr_eq(
            state.gateway_factory().expect("injected factory"),
            &injected
        ));
    }

    #[test]
    fn web_app_state_injected_registry_rebuilds_logical_gateway_factory() {
        let root = tempdir().expect("root");
        let injected = fake_registry();
        let state = crate::web::state::WebAppState::with_events_and_provider_registry(
            root.path().to_path_buf(),
            crate::web::runtime::WebRuntime::new_fake(root.path().to_path_buf()),
            crate::web::events::EventHub::new(),
            injected.clone(),
        );

        let factory = state.gateway_factory().expect("factory").clone();
        assert!(Arc::ptr_eq(&factory.registry, &injected));
    }
}
