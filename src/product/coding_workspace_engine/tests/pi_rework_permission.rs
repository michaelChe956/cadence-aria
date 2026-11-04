use super::*;
use crate::product::coding_models::{
    CodingProviderPermissionMode, CodingRolePermissionModes, CodingRoleProviderConfigSnapshot,
};
use std::sync::Arc;

// REQ-ROUTE-01：逻辑 target（`target_snapshot.is_some()`）的 provider run 只能经
// `LogicalCodebaseProviderGateway` 启动；本用例的 coder 是 Pi，而共享 gateway
// 夹具只登记 ClaudeCode/Codex 的 health entry——Pi 会在 availability 复验处
// 被拒。故这里按同一装配面构造只为 Pi 放行的本地 gateway 夹具。

use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::cross_cutting::provider_availability_gate::{
    ProviderAvailabilityGate, ProviderHealthSource,
};
use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::product::logical_codebase::{
    AggregatePolicyArtifactStore, GatewayRunAudit, LogicalCodebaseManifest,
    LogicalCodebaseProviderGateway, LogicalCodebaseStore, PolicyTarget, PolicyTargetResolver,
    ProviderCapability, ProviderCapabilitySource, ProviderDialect, ProviderGatewayError,
    ProviderRef, ProviderRefType, SessionLaunchRequest, SessionPolicyAction,
};
use crate::protocol::contracts::{AdapterInput, AdapterOutput, TimeoutStatus};

/// pass-through target resolver：直接返回请求中冻结的 target。
struct PiPassthroughTargetResolver;

impl PolicyTargetResolver for PiPassthroughTargetResolver {
    fn resolve_and_revalidate(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError> {
        Ok(request.target.clone())
    }
}

/// 测试 capability source：ClaudeCode/Pi 两方言（本用例 coder=Pi）恒 Confirmed。
struct PiCapabilitySource;

impl PiCapabilitySource {
    fn capability(provider: &ProviderRef, action: SessionPolicyAction) -> ProviderCapability {
        use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
        use crate::product::logical_codebase::policy::ProviderWireDialect;
        use crate::product::logical_codebase::provider_capability_store::ProviderActionCapability;
        let (adapter_dialect, wire_dialect) = match provider.provider_type {
            ProviderRefType::ClaudeCode => (
                ProviderDialect::ClaudeCodeCliV1,
                ProviderWireDialect::ClaudeCodeStreamJson,
            ),
            ProviderRefType::Pi => (ProviderDialect::PiRpcV1, ProviderWireDialect::PiRpc),
            ProviderRefType::Codex => (
                ProviderDialect::CodexCliV1,
                ProviderWireDialect::CodexAppServerRpc,
            ),
            ProviderRefType::KimiCode => (ProviderDialect::KimiAcpV1, ProviderWireDialect::KimiAcp),
        };
        ProviderCapability {
            provider_type: provider.provider_type,
            version: "1.0.0".to_string(),
            adapter_dialect,
            wire_dialect,
            capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
            action_capability: ProviderActionCapability {
                action,
                launch: ProviderCapabilityEvidence::Confirmed,
                resume: ProviderCapabilityEvidence::Confirmed,
                write_boundary: ProviderCapabilityEvidence::Confirmed,
                projection_digest: format!("projection-digest-{action:?}"),
                evidence_ref: format!("probe://{action:?}"),
            },
            trust: ProviderCapabilityEvidence::Confirmed,
        }
    }
}

impl ProviderCapabilitySource for PiCapabilitySource {
    fn require_supported(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(Self::capability(provider, action))
    }

    fn require_resume_supported(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(Self::capability(provider, action))
    }

    fn require_write_boundary(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(Self::capability(provider, action))
    }

    fn require_root_recipe_supported(
        &self,
        _provider: &ProviderRef,
        _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Err(ProviderGatewayError::UnsupportedCapability(
            "pi rework fixture has no root recipe facts".to_string(),
        ))
    }
}

/// 同步 adapter stub：gateway 构造签名要求，不参与本用例。
struct PiStubSyncAdapter;

impl ProviderAdapter for PiStubSyncAdapter {
    fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            structured_output: None,
            files_modified: Vec::new(),
            duration_ms: 0,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}

/// availability gate：Pi（+ ClaudeCode）恒可用。
fn pi_available_gate() -> Arc<ProviderAvailabilityGate> {
    struct AlwaysHealthy(Arc<ProviderHealthSnapshot>);

    impl ProviderHealthSource for AlwaysHealthy {
        fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
            self.0.clone()
        }

        fn degraded(&self) -> bool {
            false
        }
    }

    let checked_at = chrono::Utc::now();
    let snapshot = Arc::new(ProviderHealthSnapshot {
        schema_version: 1,
        generation: 1,
        checked_at,
        providers: [ProviderName::Pi, ProviderName::ClaudeCode]
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

/// 以已播种 LC 权威为基座组装 Pi 可用 gateway，并把 adapter 注册进 registry。
fn pi_gateway_engine(
    store: &CodingAttemptStore,
    event_tx: mpsc::Sender<CodingWsOutMessage>,
    adapter: Arc<dyn StreamingProviderAdapter>,
) -> CodingWorkspaceEngine {
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::Pi,
        Arc::new(super::provider_gateway_validated_input::ValidatedPassthroughAdapter(adapter)),
    );
    let manifest: LogicalCodebaseManifest = LogicalCodebaseStore::new(store.paths())
        .load_manifest("project_0001")
        .expect("load manifest")
        .expect("seeded manifest");
    let policies = AggregatePolicyArtifactStore::new(store.paths());
    policies
        .ensure_bootstrap(&manifest)
        .expect("bootstrap policy");
    let gateway = LogicalCodebaseProviderGateway::with_audit(
        policies,
        Arc::new(PiCapabilitySource),
        Arc::new(PiPassthroughTargetResolver),
        Arc::new(registry),
        Arc::new(PiStubSyncAdapter),
        pi_available_gate(),
        Arc::new(GatewayRunAudit::new()),
        store
            .paths()
            .root()
            .parent()
            .expect("workspace root")
            .to_path_buf(),
    );
    CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx)
        .with_logical_provider_gateway(Arc::new(gateway))
}

#[tokio::test]
async fn coder_rework_normalizes_pi_supervised_mode_to_auto() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    store
        .update_role_provider_config_snapshot(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingRoleProviderConfigSnapshot {
                coder: ProviderName::Pi,
                code_reviewer: Some(ProviderName::ClaudeCode),
                internal_reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: CodingRolePermissionModes {
                    coder: CodingProviderPermissionMode::Supervised,
                    code_reviewer: CodingProviderPermissionMode::Auto,
                    internal_reviewer: CodingProviderPermissionMode::Auto,
                },
            },
        )
        .expect("set Pi role config");

    let provider = Arc::new(provider_driven::ReviewerDrivenReworkProvider::default());
    let (event_tx, _event_rx) = mpsc::channel(16);
    let engine = pi_gateway_engine(&store, event_tx, provider.clone());
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    engine
        .execute_coder_fix_from_review(
            &attempt,
            &provider_driven::review_report_requesting_changes(&attempt),
            &CodingExecutionContext::default(),
            provider.as_ref(),
            &mut command_rx,
        )
        .await
        .expect("execute Pi coder rework");

    let input = provider.recorded_input();
    assert_eq!(input.provider_type, ProviderType::Pi);
    assert_eq!(
        input.permission_mode,
        ProviderPermissionMode::Auto,
        "Pi coder rework must normalize persisted Supervised to Auto"
    );
}
