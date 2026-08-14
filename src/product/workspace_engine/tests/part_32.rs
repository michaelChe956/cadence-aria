// T10b:规划 review via-gateway 骨架的 lib 层单测。
//
// 覆盖:
// - 注入 fake-registry gateway 后调用 `drive_review_session_via_gateway` →
//   会话驱动正常(reviewer pass → HumanConfirm)+ gateway audit `stream_launches` +1。
// - 既有 `drive_review_session`(Legacy,未注入 gateway)路径在其余 part 中保持
//   零变化,本文件不重复覆盖。

use crate::cross_cutting::provider_adapter::ProviderAdapter;
use crate::cross_cutting::provider_availability_gate::{
    ProviderAvailabilityGate, ProviderHealthSource,
};
use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::product::logical_codebase::provider_gateway::ResumeEvidenceState;
use crate::product::logical_codebase::{
    AggregatePolicyArtifactStore, GatewayRunAudit, LogicalCodebaseManifest,
    LogicalCodebaseProviderGateway, PolicyTarget, PolicyTargetResolver, ProviderCapability,
    ProviderCapabilitySource, ProviderDialect, ProviderGatewayError, ProviderRef, ProviderRefType,
    SessionLaunchRequest, SessionPolicyAction,
};
use crate::protocol::contracts::{AdapterOutput, TimeoutStatus};

/// 同步 adapter stub:`run` 返回最小成功输出,供 gateway 构造。
struct ReviewStubSyncAdapter;

impl ProviderAdapter for ReviewStubSyncAdapter {
    fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
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

/// 测试用 capability source:返回固定 capability,resume 证据恒为 `Confirmed`,
/// 使 review repair 的 resume 启动也能通过 spawn 前复验。
struct ReviewStaticCapabilitySource;

impl ProviderCapabilitySource for ReviewStaticCapabilitySource {
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
            version: "1.4.0".to_string(),
            adapter_dialect,
            capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
            resume_evidence: ResumeEvidenceState::Confirmed,
        })
    }
}

/// 测试用 target resolver:直接透传 request target。aggregate_root target 的
/// worktree 与 input.working_dir 相同,spawn 前 canonicalize 复验可通过对等比较。
struct ReviewPassThroughTargetResolver;

impl PolicyTargetResolver for ReviewPassThroughTargetResolver {
    fn resolve_and_revalidate(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError> {
        Ok(request.target.clone())
    }
}

fn review_always_available_gate() -> Arc<ProviderAvailabilityGate> {
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

struct ReviewGatewayFixture {
    _root: tempfile::TempDir,
    paths: ProductAppPaths,
    gateway: Arc<LogicalCodebaseProviderGateway>,
    audit: Arc<GatewayRunAudit>,
    worktree: std::path::PathBuf,
}

fn review_gateway_fixture() -> ReviewGatewayFixture {
    let root = tempfile::tempdir().expect("temporary product root");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).expect("create review worktree");

    let manifest = LogicalCodebaseManifest::new("project_0001", worktree.clone(), Vec::new());
    let policy_store = AggregatePolicyArtifactStore::new(paths.clone());
    policy_store
        .ensure_bootstrap(&manifest)
        .expect("bootstrap aggregate policy");

    // fake registry:ClaudeCode 映射到「输出 pass verdict」的 streaming adapter。
    let mut registry = ProviderRegistry::new();
    registry.register(
        ProviderName::ClaudeCode,
        Arc::new(ReviewVerdictStreamingProvider {
            output: "审核通过。\n\n```json\n{\"verdict\":\"pass\",\"summary\":\"可以确认\"}\n```",
            provider_type: Arc::new(Mutex::new(None)),
            prompt: Arc::new(Mutex::new(None)),
        }),
    );

    let audit = Arc::new(GatewayRunAudit::new());
    let gateway = Arc::new(LogicalCodebaseProviderGateway::with_audit(
        policy_store,
        Arc::new(ReviewStaticCapabilitySource),
        Arc::new(ReviewPassThroughTargetResolver),
        Arc::new(registry),
        Arc::new(ReviewStubSyncAdapter),
        review_always_available_gate(),
        audit.clone(),
        worktree.clone(),
    ));

    ReviewGatewayFixture {
        _root: root,
        paths,
        gateway,
        audit,
        worktree,
    }
}

#[tokio::test]
async fn drive_review_session_via_gateway_records_audit_and_completes() {
    let fixture = review_gateway_fixture();
    let audit = fixture.audit.clone();
    assert_eq!(audit.stream_launches(), 0);

    let (event_tx, _event_rx) = mpsc::channel(64);
    let mut session = make_session("sess_review_via_gateway");
    session.review_rounds = 2;
    session.reviewer_provider = Some(ProviderName::ClaudeCode);
    session.artifact = Some(artifact_payload("# Artifact\n\n可以确认"));
    session.repository_path = Some(fixture.worktree.clone());

    let mut engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(
            fixture.paths.root().join("checkpoints"),
        )),
        event_tx,
        session,
    )
    .with_logical_provider_gateway(fixture.gateway.clone());

    engine.start_review_or_skip().await;
    engine
        .drive_review_session_via_gateway(empty_provider_commands())
        .await;

    assert_eq!(
        audit.stream_launches(),
        1,
        "logical review session must start via gateway once"
    );
    assert_eq!(
        engine.session().stage,
        WorkspaceStage::HumanConfirm,
        "pass verdict must enter human confirm"
    );
    assert!(
        engine.timeline_nodes.iter().any(|node| {
            node.node_type == TimelineNodeType::ReviewerRun
                && node.status == TimelineNodeStatus::Completed
                && node.summary.as_deref() == Some("可以确认")
        }),
        "reviewer run node must be completed with pass summary"
    );
}
