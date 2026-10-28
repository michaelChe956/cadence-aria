//! Task 3.2(B0a2):公共 prepare 合同——`prepare_lc_streaming_candidate`
//! 冻结 opaque `PreparedLcStreamingCandidate`(input/request/validated/
//! full projection/facts/run-bound audit)。
//!
//! 契约(计划 Interfaces 冻结):
//! 1. fake 正例 facts 真实可观察(TTL 缓存观测+同版本 projector),不能靠
//!    默认 typed unsupported 使正例虚绿;
//! 2. prepare 全程只读:三口径计数分列——①version probe 恰一次、
//!    ②adapter start=0、③session child=0;workspace durable 不变;
//! 3. 冻结+复验矩阵:命令 identity 变化后重观测 version 与冻结事实不一致
//!    → typed 拒(`PolicyDrift/provider_version`)、零 spawn、零新 claim;
//!    旧 basis(v1 指纹)不与 full candidate 混比 → StartNew 等待而非 resume;
//! 4. 未接入 adapter(默认 trait 实现)只得到 typed unsupported 拒绝。
use super::*;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::cross_cutting::provider_boundary::{ProviderBoundaryMode, ProviderBoundaryPlan};
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::streaming_provider::{
    LcVersionCache, LcVersionProbe, LcVersionProbeKey, StreamingProviderAdapter,
};
use crate::product::logical_codebase::policy::SessionPolicyAction;
use crate::product::logical_codebase::provider_projection::{
    ProviderLcPrepareFacts, ProviderPolicyProjection, ProviderPolicyProjector,
    ProviderProjectionError, ProviderProjectionInput, UnprovisionedProviderPolicyProjector,
};
use crate::product::logical_codebase::provider_trust::{
    ProviderTrustSource, ProviderTrustVerification,
};
use crate::product::models::ProviderName;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

/// anchor_t09 fake streaming adapter:实现真实 prepare 链——
/// `LcVersionCache`+计数 probe(口径①)、start/start_validated 计数
/// (口径②)、native child 计数(口径③);executable identity 与观测
/// version 可切换,驱动「冻结+复验矩阵」。
struct AnchorT09PrepareAdapter {
    cache: Arc<LcVersionCache>,
    probe_calls: Arc<AtomicUsize>,
    executable_identity: std::sync::Mutex<String>,
    observed_version: Arc<std::sync::Mutex<String>>,
    adapter_starts: AtomicUsize,
    session_children: AtomicUsize,
    prepare_calls: AtomicUsize,
}

impl AnchorT09PrepareAdapter {
    fn new(cache: Arc<LcVersionCache>) -> Self {
        Self {
            cache,
            probe_calls: Arc::new(AtomicUsize::new(0)),
            executable_identity: std::sync::Mutex::new(
                "sha256:anchor-t09-exe-identity-v1".to_string(),
            ),
            observed_version: Arc::new(std::sync::Mutex::new("1.4.0".to_string())),
            adapter_starts: AtomicUsize::new(0),
            session_children: AtomicUsize::new(0),
            prepare_calls: AtomicUsize::new(0),
        }
    }

    fn set_executable_identity(&self, identity: &str) {
        *self.executable_identity.lock().unwrap() = identity.to_string();
    }

    fn set_observed_version(&self, version: &str) {
        *self.observed_version.lock().unwrap() = version.to_string();
    }

    fn probe_calls(&self) -> usize {
        self.probe_calls.load(Ordering::SeqCst)
    }

    fn adapter_starts(&self) -> usize {
        self.adapter_starts.load(Ordering::SeqCst)
    }

    fn session_children(&self) -> usize {
        self.session_children.load(Ordering::SeqCst)
    }

    fn prepare_calls(&self) -> usize {
        self.prepare_calls.load(Ordering::SeqCst)
    }

    fn minimal_session() -> crate::cross_cutting::streaming_provider::ProviderSession {
        let (_event_tx, events) = tokio::sync::mpsc::channel(1);
        let (commands, _command_rx) = tokio::sync::mpsc::channel(1);
        crate::cross_cutting::streaming_provider::ProviderSession {
            events,
            commands,
            native_session_id: None,
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for AnchorT09PrepareAdapter {
    async fn start(
        &self,
        _input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<crate::cross_cutting::streaming_provider::ProviderSession, ProviderAdapterError>
    {
        self.adapter_starts.fetch_add(1, Ordering::SeqCst);
        Ok(Self::minimal_session())
    }

    async fn start_validated(
        &self,
        _input: ValidatedStreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<crate::cross_cutting::streaming_provider::ProviderSession, ProviderAdapterError>
    {
        self.adapter_starts.fetch_add(1, Ordering::SeqCst);
        // 模拟策略会话握手后的 native child 创建(口径③):真实链路中
        // 错 ID 必须 kill/reap,不得冒称零 child;本测试全部负例都在
        // 此之前 fail-closed,该计数必须保持 0。
        self.session_children.fetch_add(1, Ordering::SeqCst);
        Ok(Self::minimal_session())
    }

    /// 真实 prepare 链:以本 adapter 的 TTL 缓存观测版本(口径①计数),
    /// 返回冻结 facts(exact_version/观测时刻/probe key/approval/MCP/
    /// boundary/同版本 projector)。
    async fn prepare_lc_facts(
        &self,
        _input: &crate::cross_cutting::streaming_provider::StreamingProviderInput,
        launch: &ValidatedSessionLaunchPolicy,
        cancel: CancellationToken,
    ) -> Result<ProviderLcPrepareFacts, ProviderAdapterError> {
        self.prepare_calls.fetch_add(1, Ordering::SeqCst);
        let key = LcVersionProbeKey {
            provider: ProviderName::ClaudeCode,
            canonical_executable: std::path::PathBuf::from("/opt/anchor-t09-gateway-bin/claude"),
            executable_identity: self.executable_identity.lock().unwrap().clone(),
            argv: vec!["--version".to_string()],
            supplier_scope: 7,
        };
        let probe_calls = self.probe_calls.clone();
        let observed_version = self.observed_version.clone();
        let probe: LcVersionProbe = Arc::new(move |_key| {
            let probe_calls = probe_calls.clone();
            let observed_version = observed_version.clone();
            Box::pin(async move {
                probe_calls.fetch_add(1, Ordering::SeqCst);
                Ok(observed_version.lock().unwrap().clone())
            })
        });
        let observation = self
            .cache
            .cached_lc_cli_version(key.clone(), probe, cancel)
            .await
            .map_err(|error| {
                ProviderAdapterError::execution_failed(
                    None,
                    String::new(),
                    format!("anchor t09 fake prepare: {error}"),
                    0,
                )
            })?;
        let envelope = launch.envelope();
        let boundary = ProviderBoundaryPlan::new(
            ProviderBoundaryMode::ReadOnly,
            envelope.working_directory.clone(),
            None,
            Vec::new(),
        );
        Ok(ProviderLcPrepareFacts::new(
            crate::product::logical_codebase::provider_gateway::ProviderRefType::ClaudeCode,
            envelope.provider_dialect,
            crate::product::logical_codebase::policy::ProviderWireDialect::ClaudeCodeStreamJson,
            observation.exact_version().to_string(),
            observation.observed_at(),
            key,
            "permission-prompt-tool=stdio".to_string(),
            "sha256:mcp-anchor-t09".to_string(),
            boundary,
            Arc::new(AnchorT09VersionedProjector {
                exact_version: observation.exact_version().to_string(),
            }),
        ))
    }
}

/// anchor_t09 版本化测试 projector:与 task28 `projection_from` 同形,
/// capability 摘要与 fixture 的 `StaticCapabilitySource` 对齐
/// (`projection-digest-{action:?}`),会话摘要为长度分隔+schema 前缀
/// 的 SHA-256;trust/MCP/approval/boundary 从 gateway 冻结输入透传。
struct AnchorT09VersionedProjector {
    exact_version: String,
}

fn anchor_t09_digest(parts: &[&str]) -> String {
    use sha2::Digest as _;
    let mut canonical = String::new();
    for part in parts {
        canonical.push_str(&format!("{}:{}", part.len(), part));
    }
    format!("sha256:{:x}", sha2::Sha256::digest(canonical.as_bytes()))
}

impl ProviderPolicyProjector for AnchorT09VersionedProjector {
    fn project(
        &self,
        input: &ProviderProjectionInput,
    ) -> Result<ProviderPolicyProjection, ProviderProjectionError> {
        let envelope = input.envelope();
        if input.provider().provider_type
            != crate::product::logical_codebase::provider_gateway::ProviderRefType::ClaudeCode
        {
            return Err(ProviderProjectionError::Unsupported(
                "anchor t09 projector only projects Claude Code".to_string(),
            ));
        }
        let _boundary = match input.boundary() {
            Some(plan) => plan.clone(),
            None => ProviderBoundaryPlan::new(
                ProviderBoundaryMode::ReadOnly,
                envelope.working_directory.clone(),
                None,
                Vec::new(),
            ),
        };
        let capability_projection_digest = format!("projection-digest-{:?}", envelope.action);
        let session_projection_digest = anchor_t09_digest(&[
            "anchor-t09-session-v1",
            self.exact_version.as_str(),
            capability_projection_digest.as_str(),
            input.trust_digest(),
            input.mcp_bundle_digest(),
            input.approval_policy(),
            &envelope.working_directory.to_string_lossy(),
            &envelope.config_digest,
        ]);
        Ok(ProviderPolicyProjection::new(
            crate::product::logical_codebase::provider_gateway::ProviderRefType::ClaudeCode,
            envelope.provider_dialect,
            crate::product::logical_codebase::policy::ProviderWireDialect::ClaudeCodeStreamJson,
            self.exact_version.clone(),
            envelope.action,
            input.role().clone(),
            input.permission_mode().clone(),
            input.tool_policy().cloned(),
            input.approval_policy().to_string(),
            "read-only".to_string(),
            envelope.working_directory.clone(),
            envelope.working_directory.clone(),
            envelope.target.clone(),
            envelope.readable_roots.clone(),
            envelope.writable_roots.clone(),
            input.trust_digest().to_string(),
            envelope.config_digest.clone(),
            input.mcp_bundle_digest().to_string(),
            String::new(),
            capability_projection_digest,
            session_projection_digest,
        ))
    }
}

/// anchor_t09 只读 trust source:Claude 无用户级 workspace trust 工件,
/// `verify_trusted`/`fingerprint_facts` 不应被触达;提供最小可信实现
/// 以注入 gateway(生产装配恒注入 readonly source)。
struct AnchorT09TrustedSource;

impl ProviderTrustSource for AnchorT09TrustedSource {
    fn verify_trusted(
        &self,
        _project_id: &str,
        _lc_id: &str,
        provider: &ProviderName,
        canonical_root: &Path,
    ) -> Result<ProviderTrustVerification, ProviderGatewayError> {
        Ok(ProviderTrustVerification {
            provider: provider.clone(),
            canonical_root: canonical_root.to_path_buf(),
            trust_key: String::new(),
            trusted: true,
            ownership: None,
            detail: String::new(),
            verified_at: String::new(),
        })
    }
}

fn anchor_t09_gateway(
    fixture: &GatewayFixture,
    adapter: Arc<AnchorT09PrepareAdapter>,
) -> LogicalCodebaseProviderGateway {
    let mut registry = ProviderRegistry::new();
    registry
        .register_gated(
            ProviderName::ClaudeCode,
            adapter,
            Arc::new(UnprovisionedProviderPolicyProjector),
            fixture.gate.clone(),
        )
        .expect("register gated anchor t09 adapter trio");
    let authority_root = std::fs::canonicalize(fixture.manifest().provider_context_root).unwrap();
    LogicalCodebaseProviderGateway::with_audit(
        fixture.policy_store(),
        fixture.capabilities(),
        fixture.targets(),
        Arc::new(registry),
        fixture.sync_adapter.clone(),
        fixture.gate.clone(),
        fixture.gateway_audit(),
        authority_root,
    )
    .with_readonly_lc_facts(
        crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
            fixture.paths.clone(),
            "logical_codebase_0001",
        ),
        Arc::new(AnchorT09TrustedSource),
        Some("logical_codebase_0001".to_string()),
    )
}

fn anchor_t09_planning_request(fixture: &GatewayFixture) -> SessionLaunchRequest {
    let worktree = fixture.real_worktree();
    let authority_root = std::fs::canonicalize(fixture.manifest().provider_context_root).unwrap();
    SessionLaunchRequest {
        project_id: fixture.manifest().project_id,
        provider: ProviderRef::claude_code("cap_claude_code_1_4_0"),
        action: SessionPolicyAction::PlanningReadOnly,
        target: PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree),
        // LC 会话 cwd = canonical LC root(root-cwd 契约,与 target 分离)。
        working_directory: authority_root.clone(),
        readable_roots: vec![fixture.paths.root().to_path_buf()],
        writable_roots: Vec::new(),
        config_artifact_ref: "sha256:managed-config-artifact".to_string(),
    }
}

fn anchor_t09_streaming_input(
    fixture: &GatewayFixture,
) -> crate::cross_cutting::streaming_provider::StreamingProviderInput {
    let authority_root = std::fs::canonicalize(fixture.manifest().provider_context_root).unwrap();
    crate::cross_cutting::streaming_provider::StreamingProviderInput {
        provider_type: crate::protocol::contracts::ProviderType::ClaudeCode,
        role: crate::protocol::contracts::AdapterRole::Orchestrator,
        prompt: "anchor t09 prepare contract".to_string(),
        working_dir: authority_root.clone(),
        working_directory: Some(authority_root),
        workspace_session_id: None,
        resume_provider_session_id: None,
        permission_mode: crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
        tool_policy: None,
        audit_sink: None,
        structured_output_contract: None,
        env_vars: Default::default(),
        timeout_secs: 5,
        baseline_tree: None,
    }
}

fn anchor_t09_audit_context(
    sink: Arc<crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink>,
) -> ProviderLaunchAuditContext {
    ProviderLaunchAuditContext {
        workspace_session_id: "ws-anchor-t09".to_string(),
        role_run_seq: 1,
        audit_sink: sink,
    }
}

#[tokio::test]
async fn anchor_t09_prepared_full_candidate_freezes_real_facts() {
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    let recording = Arc::new(
        crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink::default(),
    );
    let fake = Arc::new(AnchorT09PrepareAdapter::new(
        Arc::new(LcVersionCache::new()),
    ));
    let gateway = anchor_t09_gateway(&fixture, fake.clone());
    let request = anchor_t09_planning_request(&fixture);

    // durable/审计快照:prepare 必须零写(policy/trust/audit/claim 面)。
    let policy_before = fixture
        .policy_store()
        .get(&fixture.manifest().project_id)
        .unwrap()
        .unwrap();
    let durable_before = (
        policy_before.revision,
        policy_before.digest.clone(),
        fixture.gateway_audit().supersede_count(),
        fixture.gateway_audit().stream_launches(),
        fixture.gateway_audit().sync_launches(),
        recording.events().len(),
    );
    let adapter_starts_before_negative = fake.adapter_starts();
    let session_children_before_negative = fake.session_children();
    assert_eq!(adapter_starts_before_negative, 0);
    assert_eq!(session_children_before_negative, 0);

    let candidate = gateway
        .prepare_lc_streaming_candidate(
            anchor_t09_streaming_input(&fixture),
            request.clone(),
            anchor_t09_audit_context(recording.clone()),
            CancellationToken::new(),
        )
        .await
        .expect("prepare freezes an opaque full candidate from real adapter facts");

    // fake 正例 facts 真实可观察(非 default unsupported 虚绿):
    // TTL 缓存观测恰一次(口径①),观测时刻/键冻结在 facts 里。
    assert_eq!(candidate.facts().exact_version(), "1.4.0");
    assert_eq!(fake.probe_calls(), 1);
    assert_eq!(fake.prepare_calls(), 1);
    assert!(candidate.facts().version_observed_at() <= std::time::Instant::now());
    assert_eq!(
        candidate.facts().version_probe_key().executable_identity,
        "sha256:anchor-t09-exe-identity-v1"
    );

    // 冻结 full projection 与 facts 同源(同版本 projector 产出):
    assert_eq!(
        candidate.full_projection().exact_version(),
        candidate.facts().exact_version()
    );
    assert!(!candidate.full_projection().trust_digest().is_empty());
    assert!(!candidate.full_projection().projection_digest().is_empty());
    assert_eq!(
        candidate.full_projection().mcp_bundle_digest(),
        "sha256:mcp-anchor-t09"
    );

    // run-bound audit 绑定(prepare 合同:启动前绑定 sink 与会话身份)。
    assert_eq!(
        candidate
            .validated()
            .launch_audit()
            .map(|context| context.workspace_session_id.as_str()),
        Some("ws-anchor-t09")
    );
    assert_eq!(
        candidate.input().workspace_session_id.as_deref(),
        Some("ws-anchor-t09")
    );
    assert!(candidate.input().audit_sink.is_some());

    // 负例三口径/零写:prepare 全程 ②adapter start=0、③session child=0,
    // workspace durable/policy/trust audit 全不变。
    assert_eq!(fake.adapter_starts(), adapter_starts_before_negative);
    assert_eq!(fake.session_children(), session_children_before_negative);
    let policy_after = fixture
        .policy_store()
        .get(&fixture.manifest().project_id)
        .unwrap()
        .unwrap();
    let durable_after = (
        policy_after.revision,
        policy_after.digest,
        fixture.gateway_audit().supersede_count(),
        fixture.gateway_audit().stream_launches(),
        fixture.gateway_audit().sync_launches(),
        recording.events().len(),
    );
    assert_eq!(durable_after, durable_before);

    // full-matching resume:以候选冻结 full projection 重算的五参指纹与
    // 旧会话一致 → Resume(validated 携带完整上下文指纹)。
    let previous = SessionResumeFingerprint::from_envelope(
        candidate.validated().envelope(),
        candidate.full_projection(),
        candidate.validated().action_evidence_digest(),
        &canonical_target_git_identity(&candidate.validated().envelope().target.worktree),
    );
    let disposition = gateway
        .resume_or_start_with_projection(
            ResumeSessionLaunchRequest {
                launch: request.clone(),
                previous_fingerprint: previous,
                previous_session_id: "sess-native-anchor-t09".to_string(),
            },
            candidate.full_projection().clone(),
        )
        .expect("full-matching fingerprint resumes");
    assert!(matches!(disposition, GatewaySessionDisposition::Resume(_)));
    // 转换面(仅 full-matching resume / 明确 fresh 后):转换产物保留
    // prepare 绑定的 run-bound audit 与会话身份,供后续 B1/B2 消费。
    let (converted_input, converted_validated) = candidate
        .clone()
        .into_validated_streaming_input()
        .into_parts();
    assert_eq!(
        converted_input.workspace_session_id.as_deref(),
        Some("ws-anchor-t09")
    );
    assert!(converted_input.audit_sink.is_some());
    assert!(converted_validated.launch_audit().is_some());
    assert_eq!(fake.adapter_starts(), adapter_starts_before_negative);
    assert_eq!(fake.session_children(), session_children_before_negative);

    // 旧 basis 不与 full candidate 混比:v1/legacy 指纹永不匹配 →
    // StartNew 等待显式 fresh(审计 supersede),决策层零 spawn。
    let stale = gateway
        .resume_or_start_with_projection(
            ResumeSessionLaunchRequest {
                launch: request.clone(),
                previous_fingerprint: SessionResumeFingerprint {
                    digest: "sha256:legacy-v1-basis".to_string(),
                },
                previous_session_id: "sess-legacy-anchor-t09".to_string(),
            },
            candidate.full_projection().clone(),
        )
        .expect("legacy basis yields a StartNew waiting disposition");
    assert!(matches!(stale, GatewaySessionDisposition::StartNew { .. }));
    assert_eq!(fake.adapter_starts(), adapter_starts_before_negative);
    assert_eq!(fake.session_children(), session_children_before_negative);
    let claim_count_after_negative = fixture.gateway_audit().supersede_count();

    // 冻结+复验矩阵:活缓存期间命令 canonical executable identity 改变,
    // 重观测 version 与冻结事实不一致 → typed 拒 resume、不可 spawn。
    fake.set_executable_identity("sha256:anchor-t09-exe-identity-v2");
    fake.set_observed_version("1.4.1-replaced");
    let current = gateway
        .reobserve_lc_prepare_facts(&candidate, CancellationToken::new())
        .await
        .expect("reobservation re-probes under the new executable identity key");
    assert_eq!(fake.probe_calls(), 2);
    assert_eq!(current.exact_version(), "1.4.1-replaced");
    let typed_rejection = candidate
        .assert_frozen_version(&current)
        .expect_err("re-observed version must be typed-rejected against frozen facts");
    assert!(matches!(
        &typed_rejection,
        ProviderGatewayError::PolicyDrift { dimension } if dimension == "provider_version"
    ));
    assert_eq!(fake.adapter_starts(), adapter_starts_before_negative);
    assert_eq!(fake.session_children(), session_children_before_negative);
    assert_eq!(
        fixture.gateway_audit().supersede_count(),
        claim_count_after_negative
    );

    // 未接入 adapter(默认 trait 实现)只得到 typed unsupported 拒绝,
    // 且不触达 registry start(裸 fixture 的 CountingStreamingAdapter)。
    let bare = fixture.gateway();
    let bare_error = match bare
        .prepare_lc_streaming_candidate(
            anchor_t09_streaming_input(&fixture),
            anchor_t09_planning_request(&fixture),
            anchor_t09_audit_context(recording.clone()),
            CancellationToken::new(),
        )
        .await
    {
        Err(ProviderGatewayError::Adapter(adapter_error)) => adapter_error,
        Err(other) => panic!("unexpected rejection shape: {other}"),
        Ok(_) => panic!("default adapters must fail closed with a typed unsupported error"),
    };
    assert!(
        bare_error
            .stderr
            .contains("lc prepare facts is not supported"),
        "unexpected default rejection: {bare_error:?}"
    );
    assert_eq!(fixture.registry_start_count(), 0);
    let policy_final = fixture
        .policy_store()
        .get(&fixture.manifest().project_id)
        .unwrap()
        .unwrap();
    assert_eq!(policy_final.revision, durable_before.0);
    assert_eq!(policy_final.digest, durable_before.1);
    assert_eq!(
        fixture.gateway_audit().supersede_count(),
        claim_count_after_negative
    );
}
