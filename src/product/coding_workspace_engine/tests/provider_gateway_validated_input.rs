//! Task 6: `validated_streaming_input_for_role` 三分支单测。
//!
//! - Legacy attempt（`target_snapshot` 为 `None`）→ `Ok(None)`。
//! - 逻辑 attempt 但引擎未注入 gateway → `Ok(None)`。
//! - 逻辑 attempt + 已注入 gateway + Coder → `Ok(Some)` 且 action 为
//!   `CodingTargetWrite`。
//! - Codex 名字 → gateway 路由级阻断 `codex_danger_full_access_unsupported`。

use super::*;

use std::sync::Arc;

use chrono::Utc;

use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::cross_cutting::provider_availability_gate::{
    ProviderAvailabilityGate, ProviderHealthSource,
};
use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::product::coding_models::AttemptTargetSnapshot;
use crate::product::logical_codebase::provider_gateway::ResumeEvidenceState;
use crate::product::logical_codebase::{
    AggregatePolicyArtifactStore, GatewayRunAudit, LogicalCodebaseManifest,
    LogicalCodebaseProviderGateway, LogicalRepositoryId, PolicyTarget, PolicyTargetResolver,
    ProviderCapability, ProviderCapabilitySource, ProviderDialect, ProviderGatewayError,
    ProviderRef, ProviderRefType, RepositoryCheckoutId, SessionLaunchRequest, SessionPolicyAction,
};
use crate::protocol::contracts::{AdapterInput, AdapterOutput, TimeoutStatus};

/// pass-through target resolver:直接返回请求中冻结的 target。
struct PassThroughTargetResolver;

impl PolicyTargetResolver for PassThroughTargetResolver {
    fn resolve_and_revalidate(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError> {
        Ok(request.target.clone())
    }
}

/// 测试 capability source:按 provider ref 返回对应 dialect 的 capability。
/// Codex 的 danger-full-access 阻断由 gateway 自身的路由级硬门施加,不在此处。
struct StaticCapabilitySource;

impl ProviderCapabilitySource for StaticCapabilitySource {
    fn require_supported(
        &self,
        provider: &ProviderRef,
        _action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        let adapter_dialect = match provider.provider_type {
            ProviderRefType::ClaudeCode => ProviderDialect::ClaudeCodeCliV1,
            ProviderRefType::Codex => ProviderDialect::CodexCliV1,
        };
        Ok(ProviderCapability {
            provider_type: provider.provider_type,
            version: "1.0.0".to_string(),
            adapter_dialect,
            capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
            resume_evidence: ResumeEvidenceState::Confirmed,
        })
    }
}

/// 同步 adapter stub:`validate` 不触达 sync adapter,仅满足 gateway 构造签名。
struct StubSyncAdapter;

impl ProviderAdapter for StubSyncAdapter {
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

/// 为 `project_id` 组装一个 bootstrap 政策就绪的 gateway。
fn build_gateway(paths: &ProductAppPaths, project_id: &str) -> LogicalCodebaseProviderGateway {
    let manifest = LogicalCodebaseManifest::new(project_id, paths.root().to_path_buf(), vec![]);
    let policies = AggregatePolicyArtifactStore::new(paths.clone());
    policies
        .ensure_bootstrap(&manifest)
        .expect("bootstrap policy");
    LogicalCodebaseProviderGateway::with_audit(
        policies,
        Arc::new(StaticCapabilitySource),
        Arc::new(PassThroughTargetResolver),
        Arc::new(ProviderRegistry::new()),
        Arc::new(StubSyncAdapter),
        always_available_gate(),
        Arc::new(GatewayRunAudit::new()),
    )
}

/// 覆写 attempt 的 `target_snapshot` 为逻辑代码库 target 并落盘。
fn with_target_snapshot(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> CodingExecutionAttempt {
    let worktree = attempt
        .worktree_path
        .clone()
        .expect("running attempt must have a worktree");
    let mut logical = attempt.clone();
    logical.target_snapshot = Some(AttemptTargetSnapshot {
        logical_repository_id: LogicalRepositoryId(uuid::Uuid::nil()),
        checkout_id: RepositoryCheckoutId(uuid::Uuid::nil()),
        physical_repository_id: "repository_0001".to_string(),
        canonical_path: worktree,
        git_dir_identity: "git-dir-identity".to_string(),
        revision: None,
        policy_digest: String::new(),
        membership_revision: 1,
        captured_at: "2026-08-09T00:00:00Z".to_string(),
        capture_source: "test".to_string(),
    });
    let attempt_path = store
        .paths()
        .issue_lifecycle_root(&logical.project_id, &logical.issue_id)
        .join("coding-attempts")
        .join(format!("{}.json", logical.id));
    crate::product::json_store::write_json(&attempt_path, &logical)
        .expect("write attempt with target snapshot");
    logical
}

fn engine(
    store: &CodingAttemptStore,
    gateway: Option<LogicalCodebaseProviderGateway>,
) -> CodingWorkspaceEngine {
    let (tx, _rx) = mpsc::channel(32);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    match gateway {
        Some(gateway) => engine.with_logical_provider_gateway(Arc::new(gateway)),
        None => engine,
    }
}

fn streaming_input(working_dir: PathBuf, provider_type: ProviderType) -> StreamingProviderInput {
    StreamingProviderInput {
        provider_type,
        role: AdapterRole::Executor,
        prompt: "probe".to_string(),
        working_dir,
        workspace_session_id: None,
        resume_provider_session_id: None,
        permission_mode: ProviderPermissionMode::Auto,
        structured_output_contract: None,
        env_vars: Default::default(),
        timeout_secs: 1,
    }
}

#[test]
fn validated_streaming_input_for_role_returns_none_for_legacy_attempt() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let _root = root;
    let gateway = build_gateway(&store.paths(), &attempt.project_id);
    let engine = engine(&store, Some(gateway));
    let input = streaming_input(
        attempt.worktree_path.clone().expect("worktree"),
        ProviderType::ClaudeCode,
    );

    let validated = engine
        .validated_streaming_input_for_role(&attempt, CodingProviderRole::Coder, input)
        .expect("helper returns Ok");

    assert!(
        validated.is_none(),
        "legacy attempt must not produce validated input"
    );
}

#[test]
fn validated_streaming_input_for_role_returns_none_without_injected_gateway() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let _root = root;
    let logical_attempt = with_target_snapshot(&store, &attempt);
    let engine = engine(&store, None);
    let input = streaming_input(
        logical_attempt.worktree_path.clone().expect("worktree"),
        ProviderType::ClaudeCode,
    );

    let validated = engine
        .validated_streaming_input_for_role(&logical_attempt, CodingProviderRole::Coder, input)
        .expect("helper returns Ok");

    assert!(
        validated.is_none(),
        "missing gateway must not produce validated input"
    );
}

#[test]
fn validated_streaming_input_for_role_produces_coding_target_write_for_coder() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let _root = root;
    let logical_attempt = with_target_snapshot(&store, &attempt);
    let gateway = build_gateway(&store.paths(), &logical_attempt.project_id);
    let engine = engine(&store, Some(gateway));
    let worktree = logical_attempt.worktree_path.clone().expect("worktree");
    let input = streaming_input(worktree.clone(), ProviderType::ClaudeCode);

    let validated = engine
        .validated_streaming_input_for_role(&logical_attempt, CodingProviderRole::Coder, input)
        .expect("helper returns Ok")
        .expect("logical attempt + gateway must produce validated input");

    let (_input, policy) = validated.into_parts();
    assert_eq!(
        policy.envelope().action,
        SessionPolicyAction::CodingTargetWrite
    );
    assert_eq!(policy.envelope().writable_roots, vec![worktree]);
}

#[test]
fn validated_streaming_input_for_role_rejects_codex_danger_full_access() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let _root = root;
    let logical_attempt = with_target_snapshot(&store, &attempt);
    let gateway = build_gateway(&store.paths(), &logical_attempt.project_id);
    let engine = engine(&store, Some(gateway));
    let input = streaming_input(
        logical_attempt.worktree_path.clone().expect("worktree"),
        ProviderType::Codex,
    );

    let error = engine
        .validated_streaming_input_for_role(&logical_attempt, CodingProviderRole::Coder, input)
        .expect_err("Codex must be rejected");

    assert!(
        error
            .to_string()
            .contains("codex_danger_full_access_unsupported"),
        "expected codex_danger_full_access_unsupported, got: {error}"
    );
}
