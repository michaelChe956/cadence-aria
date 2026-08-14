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
use crate::cross_cutting::session_launch::ValidatedStreamingProviderInput;
use crate::product::cadence_skills::routing_reference::RoutingReferenceContext;
use crate::product::coding_models::AttemptTargetSnapshot;
use crate::product::coding_models::provider_config::{
    CodingRolePermissionModes, CodingRoleProviderConfigSnapshot,
};
use crate::product::logical_codebase::provider_gateway::ResumeEvidenceState;
use crate::product::logical_codebase::{
    AggregatePolicyArtifactStore, CheckoutAvailability, CheckoutKind, GatewayRunAudit,
    LogicalCodebaseManifest, LogicalCodebaseProviderGateway, LogicalCodebaseStore,
    LogicalRepositoryId, PolicyTarget, PolicyTargetResolver, ProviderCapability,
    ProviderCapabilitySource, ProviderDialect, ProviderGatewayError, ProviderRef, ProviderRefType,
    RepositoryCheckoutId, RepositoryCheckoutRecord, SessionLaunchRequest, SessionPolicyAction,
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
    build_gateway_with_registry(
        paths,
        project_id,
        Arc::new(ProviderRegistry::new()),
        Arc::new(GatewayRunAudit::new()),
    )
}

/// 用注入的 registry 与 audit 组装 gateway,供 engine 级测试注册 fake streaming
/// adapter 并断言 `GatewayRunAudit` 的 Stream 启动计数。
fn build_gateway_with_registry(
    paths: &ProductAppPaths,
    project_id: &str,
    registry: Arc<ProviderRegistry>,
    audit: Arc<GatewayRunAudit>,
) -> LogicalCodebaseProviderGateway {
    let manifest = LogicalCodebaseManifest::new(project_id, paths.root().to_path_buf(), vec![]);
    let policies = AggregatePolicyArtifactStore::new(paths.clone());
    policies
        .ensure_bootstrap(&manifest)
        .expect("bootstrap policy");
    LogicalCodebaseProviderGateway::with_audit(
        policies,
        Arc::new(StaticCapabilitySource),
        Arc::new(PassThroughTargetResolver),
        registry,
        Arc::new(StubSyncAdapter),
        always_available_gate(),
        audit,
        manifest.provider_context_root.clone(),
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

/// Task 7 专用:为逻辑 attempt 播种 LogicalCodebaseStore 的 manifest 与主 checkout,
/// 使 provider run 启动前的跨仓越界 baseline 采集(Task 14)能成功。
fn seed_logical_codebase_checkout(store: &CodingAttemptStore, attempt: &CodingExecutionAttempt) {
    let logical_store = LogicalCodebaseStore::new(store.paths());
    let repository_id = LogicalRepositoryId(uuid::Uuid::nil());
    let worktree = attempt.worktree_path.clone().expect("worktree");
    let manifest = LogicalCodebaseManifest::new(
        &attempt.project_id,
        store.paths().root().to_path_buf(),
        vec![repository_id],
    );
    logical_store
        .save_manifest(&attempt.project_id, &manifest)
        .expect("save manifest");
    logical_store
        .save_checkout(
            &attempt.project_id,
            &RepositoryCheckoutRecord {
                checkout_id: RepositoryCheckoutId(uuid::Uuid::nil()),
                logical_repository_id: repository_id,
                physical_repository_id: "repository_0001".to_string(),
                kind: CheckoutKind::Main,
                canonical_path: worktree,
                checkout_path_hash: "checkout-path-hash".to_string(),
                git_dir_identity: "git-dir-identity".to_string(),
                revision: None,
                availability: CheckoutAvailability::Available,
                observed_at: "2026-08-09T00:00:00Z".to_string(),
                created_at: "2026-08-09T00:00:00Z".to_string(),
                updated_at: "2026-08-09T00:00:00Z".to_string(),
            },
        )
        .expect("save checkout");
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

/// 把 attempt 的 Coder role provider 覆盖为 ClaudeCode,避免 fixture 默认 author=Codex
/// 触发 gateway 的 Codex danger-full-access 路由级硬门。
fn override_coder_to_claude_code(store: &CodingAttemptStore, attempt: &CodingExecutionAttempt) {
    store
        .update_role_provider_config_snapshot(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingRoleProviderConfigSnapshot {
                coder: ProviderName::ClaudeCode,
                code_reviewer: ProviderName::ClaudeCode,
                internal_reviewer: ProviderName::ClaudeCode,
                review_rounds: 1,
                permission_modes: CodingRolePermissionModes::default(),
            },
        )
        .expect("override coder to ClaudeCode");
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

/// Task 7:注册进 gateway registry 的 review streaming adapter。`start` 立即完成
/// 并输出固定 review JSON,使 engine 级 internal review 能唯一经 gateway 跑通。
struct ReviewStreamingAdapter {
    output: String,
}

impl ReviewStreamingAdapter {
    fn new(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ReviewStreamingAdapter {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let structured_output_contract = input.structured_output_contract.clone();
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(4);
        let output = self.output.clone();
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed(
                    crate::cross_cutting::streaming_provider::ProviderCompletion::from_output(
                        output,
                        structured_output_contract.as_ref(),
                        None,
                    ),
                ))
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[test]
fn validated_streaming_input_for_role_produces_review_read_only_for_review_roles() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let _root = root;
    let logical_attempt = with_target_snapshot(&store, &attempt);
    let gateway = build_gateway(&store.paths(), &logical_attempt.project_id);
    let engine = engine(&store, Some(gateway));

    for role in [
        CodingProviderRole::CodeReviewer,
        CodingProviderRole::InternalReviewer,
    ] {
        let input = streaming_input(
            logical_attempt.worktree_path.clone().expect("worktree"),
            ProviderType::ClaudeCode,
        );
        let policy = engine
            .resolve_launch_policy_for_role(&logical_attempt, role, &input.working_dir)
            .expect("helper returns Ok")
            .expect("logical attempt + gateway must produce policy");
        let (_input, policy) = ValidatedStreamingProviderInput::new(input, policy).into_parts();
        assert_eq!(
            policy.envelope().action,
            SessionPolicyAction::ReviewReadOnly
        );
        assert!(policy.envelope().writable_roots.is_empty());
    }
}

#[tokio::test]
async fn logical_internal_review_launches_through_gateway() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let _root = root;
    init_test_git_repo(attempt.worktree_path.as_ref().unwrap());
    let logical_attempt = with_target_snapshot(&store, &attempt);
    seed_logical_codebase_checkout(&store, &logical_attempt);

    store
        .save_review_request(
            &logical_attempt,
            &ReviewRequest {
                id: "review_request_0001".to_string(),
                attempt_id: logical_attempt.id.clone(),
                kind: ReviewRequestKind::GitBranchOnly,
                remote_kind: RemoteKind::GenericGit,
                remote: "origin".to_string(),
                base_branch: logical_attempt.base_branch.clone(),
                branch_name: logical_attempt.branch_name.clone(),
                commit_sha: git_stdout(
                    logical_attempt.worktree_path.as_ref().unwrap(),
                    &["rev-parse", "HEAD"],
                ),
                push_status: PushStatus::Pushed,
                external_url: None,
                manual_instructions: Vec::new(),
                created_at: "2026-08-09T00:00:00Z".to_string(),
                updated_at: "2026-08-09T00:00:00Z".to_string(),
                push_error: None,
                owner_kind: ReviewRequestOwnerKind::Attempt,
                pointer_publication_id: None,
                revoked: false,
            },
        )
        .expect("review request");

    let audit = Arc::new(GatewayRunAudit::new());
    let adapter = Arc::new(ReviewStreamingAdapter::new(
        serde_json::json!({
            "verdict": "approve",
            "summary": "logical review ok",
            "findings": []
        })
        .to_string(),
    ));
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::ClaudeCode, adapter.clone());
    let gateway = build_gateway_with_registry(
        &store.paths(),
        &logical_attempt.project_id,
        Arc::new(registry),
        audit.clone(),
    );

    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx)
        .with_logical_provider_gateway(Arc::new(gateway));

    let review = engine
        .execute_internal_pr_review(&logical_attempt, adapter.as_ref())
        .await
        .expect("logical internal review must launch through gateway");

    assert_eq!(review.verdict, ReviewVerdict::Approve);
    assert_eq!(audit.stream_launches(), 1);
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

    let policy = engine
        .resolve_launch_policy_for_role(&attempt, CodingProviderRole::Coder, &input.working_dir)
        .expect("helper returns Ok");

    assert!(
        policy.is_none(),
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

    let policy = engine
        .resolve_launch_policy_for_role(
            &logical_attempt,
            CodingProviderRole::Coder,
            &input.working_dir,
        )
        .expect("helper returns Ok");

    assert!(
        policy.is_none(),
        "missing gateway must not produce validated input"
    );
}

#[test]
fn validated_streaming_input_for_role_produces_coding_target_write_for_coder() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let _root = root;
    let logical_attempt = with_target_snapshot(&store, &attempt);
    override_coder_to_claude_code(&store, &logical_attempt);
    let gateway = build_gateway(&store.paths(), &logical_attempt.project_id);
    let engine = engine(&store, Some(gateway));
    let worktree = logical_attempt.worktree_path.clone().expect("worktree");
    let input = streaming_input(worktree.clone(), ProviderType::ClaudeCode);

    let policy = engine
        .resolve_launch_policy_for_role(
            &logical_attempt,
            CodingProviderRole::Coder,
            &input.working_dir,
        )
        .expect("helper returns Ok")
        .expect("logical attempt + gateway must produce policy");

    let (_input, policy) = ValidatedStreamingProviderInput::new(input, policy).into_parts();
    assert_eq!(
        policy.envelope().action,
        SessionPolicyAction::CodingTargetWrite
    );
    assert_eq!(policy.envelope().writable_roots, vec![worktree]);
}

#[test]
fn validated_streaming_input_targets_coding_worktree_not_primary_checkout() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let _root = root;
    let logical_attempt = with_target_snapshot(&store, &attempt);

    // 覆盖 target_snapshot.canonical_path 为主 checkout（与嵌套 coding worktree 不同），
    // 验证 CodingTargetWrite 的 target worktree 取 input.working_dir（嵌套 worktree）而非
    // 主 checkout——与 C-1 多仓 worktree 路由（checkout/.worktrees/aria-issues/{issue}）一致。
    let nested_worktree = logical_attempt
        .worktree_path
        .clone()
        .expect("worktree path");
    let primary_checkout = nested_worktree.parent().expect("parent").join("checkout");
    let mut logical = logical_attempt.clone();
    logical.target_snapshot = Some(AttemptTargetSnapshot {
        logical_repository_id: LogicalRepositoryId(uuid::Uuid::nil()),
        checkout_id: RepositoryCheckoutId(uuid::Uuid::nil()),
        physical_repository_id: "repository_0001".to_string(),
        canonical_path: primary_checkout,
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
        .expect("write attempt with nested-worktree snapshot");

    let gateway = build_gateway(&store.paths(), &logical.project_id);
    let engine = engine(&store, Some(gateway));
    override_coder_to_claude_code(&store, &logical);
    let input = streaming_input(nested_worktree.clone(), ProviderType::ClaudeCode);

    let policy = engine
        .resolve_launch_policy_for_role(&logical, CodingProviderRole::Coder, &input.working_dir)
        .expect("helper returns Ok")
        .expect("logical attempt + gateway must produce policy");

    let (_input, policy) = ValidatedStreamingProviderInput::new(input, policy).into_parts();
    assert_eq!(
        policy.envelope().action,
        SessionPolicyAction::CodingTargetWrite
    );
    assert_eq!(policy.envelope().target.worktree, nested_worktree);
    assert_eq!(policy.envelope().writable_roots, vec![nested_worktree]);
}

#[test]
fn validated_streaming_input_for_role_rejects_codex_danger_full_access() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let _root = root;
    let logical_attempt = with_target_snapshot(&store, &attempt);
    let gateway = build_gateway(&store.paths(), &logical_attempt.project_id);
    let engine = engine(&store, Some(gateway));
    let worktree = logical_attempt.worktree_path.clone().expect("worktree");

    let error = engine
        .resolve_launch_policy_for_role(&logical_attempt, CodingProviderRole::Coder, &worktree)
        .expect_err("Codex must be rejected");

    assert!(
        error
            .to_string()
            .contains("codex_danger_full_access_unsupported"),
        "expected codex_danger_full_access_unsupported, got: {error}"
    );
}

#[test]
fn routing_reference_context_from_policy_maps_envelope_fields() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let _root = root;
    let logical_attempt = with_target_snapshot(&store, &attempt);
    override_coder_to_claude_code(&store, &logical_attempt);
    let gateway = build_gateway(&store.paths(), &logical_attempt.project_id);
    let engine = engine(&store, Some(gateway));
    let worktree = logical_attempt.worktree_path.clone().expect("worktree");

    let policy = engine
        .resolve_launch_policy_for_role(&logical_attempt, CodingProviderRole::Coder, &worktree)
        .expect("helper returns Ok")
        .expect("logical attempt + gateway must produce policy");

    let envelope = policy.envelope();
    let context = routing_reference_context_from_policy(&policy);
    match context {
        RoutingReferenceContext::Logical(logical) => {
            assert_eq!(logical.policy_id, envelope.policy_id);
            assert_eq!(logical.policy_revision, envelope.policy_revision);
            assert_eq!(logical.policy_digest, envelope.policy_digest);
            assert_eq!(
                logical.authority_root,
                envelope.authority_root.to_string_lossy()
            );
        }
        RoutingReferenceContext::Legacy => panic!("expected Logical routing reference context"),
    }
}
