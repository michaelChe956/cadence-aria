//! Task 12a:#8 政策发布依赖面——缺发布 fail-closed、正文/revision 漂移 spawn
//! 前阻断(实施计划 Task 12 段 513-541 行;REQ-LCG-02/07)。
//!
//! receipt 链 digest 漂移**不**阻断:2026-10-27 的政策链改动(提交 85497679)
//! 把政策权威收敛为「当前已发布政策」(artifact),回执只证明「已发布」这一
//! 事实存在 → 见
//! `lcg_t12_receipt_digest_drift_is_tolerated_because_published_policy_is_authoritative`。
//!
//! 消费契约(Global Constraints #8;#8 已于 2026-10-03 c4706aa9 收口交付):
//! gateway 只消费最终 `artifact.policy_text` 原始 UTF-8 字节(= canonical
//! root 下 `policy_id` locator 文件原字节),消费侧零发布、零 fixture seed
//! 物化——本文件的漂移 fixture 是「已发布后」状态构造(计划 Step 2:缺件/
//! 漂移负向 case 不依赖 #8 真实重跑,可先行),不是给成功 case 供假的 seed。
//!
//! `assert_policy_publication_chain` 是计划 Interfaces 面
//! `LiveLcGatewayHarness::assert_policy_publication_chain()->Result<(),LiveMatrixFailure>`
//! 的文件内等价实现(harness.rs 归 Task11Step3 独占,本任务禁触;live 现场骨架
//! `lcg_live_fresh_lc_policy_publication_recipe_and_direct_comparison` 消费本 fn,
//! controller 可在收口后整体搬移上 harness)。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use cadence_aria::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use cadence_aria::cross_cutting::provider_availability_gate::{
    ProviderAvailabilityGate, ProviderHealthSource,
};
use cadence_aria::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
use cadence_aria::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
use cadence_aria::cross_cutting::provider_registry::ProviderRegistry;
use cadence_aria::cross_cutting::session_launch::ValidatedStreamingProviderInput;
use cadence_aria::cross_cutting::streaming_provider::{
    ProviderPermissionMode, ProviderSession, StreamingProviderAdapter, StreamingProviderInput,
};
use cadence_aria::cross_cutting::tool_policy_audit::{
    DurableToolPolicyEvent, ToolPolicyAuditError, ToolPolicyAuditSink,
};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::logical_codebase::aggregate_initialization_store::root_recipe_command_index;
use cadence_aria::product::logical_codebase::policy::{
    AggregatePolicyArtifact, PolicyTarget, ProviderDialect, ProviderWireDialect,
    SessionPolicyAction,
};
use cadence_aria::product::logical_codebase::provider_capability_store::ProviderActionCapability;
use cadence_aria::product::logical_codebase::provider_gateway::ProviderLaunchAuditContext;
use cadence_aria::product::logical_codebase::provider_trust::{
    ProviderTrustSource, ProviderTrustVerification,
};
use cadence_aria::product::logical_codebase::root_recipe_receipt::RootRecipeFilesystemAuditor;
use cadence_aria::product::logical_codebase::{
    AggregatePolicyArtifactStore, GatewayRunAudit, LogicalCodebaseProviderGateway,
    PolicyTargetResolver, ProviderCapability, ProviderCapabilitySource, ProviderGatewayError,
    ProviderRef, ProviderRefType, RootRecipeReceiptStore, SessionLaunchRequest,
};
use cadence_aria::product::models::ProviderName;
use cadence_aria::protocol::contracts::{AdapterOutput, AdapterRole, ProviderType, TimeoutStatus};
use chrono::Utc;
use sha2::{Digest, Sha256};
use tempfile::{TempDir, tempdir};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub(crate) const T12_PROJECT_ID: &str = "project_0001";
const T12_LC_SCOPE_ID: &str = "lc_0001";
const T12_LOGICAL_CODEBASE_UUID: &str = "00000000-0000-0000-0000-000000000012";
const T12_CONFIG_ARTIFACT_REF: &str = "sha256:t12-managed-config-artifact";

// ---------------------------------------------------------------------------
// Task 12 共享测试替身(供 direct_comparison.rs 两类 direct 对照复用;
// 模式同 tests/it_web/provider_gateway_envelope.rs 的公共面替身)
// ---------------------------------------------------------------------------

/// 四家恒 Confirmed 的能力源:使政策/发布面成为唯一被测维度。
pub(crate) struct AllFourCapabilitySource;

impl AllFourCapabilitySource {
    fn capability(provider: &ProviderRef, action: SessionPolicyAction) -> ProviderCapability {
        let (adapter_dialect, wire_dialect) = match provider.provider_type {
            ProviderRefType::ClaudeCode => (
                ProviderDialect::ClaudeCodeCliV1,
                ProviderWireDialect::ClaudeCodeStreamJson,
            ),
            ProviderRefType::Codex => (
                ProviderDialect::CodexCliV1,
                ProviderWireDialect::CodexAppServerRpc,
            ),
            ProviderRefType::Pi => (ProviderDialect::PiRpcV1, ProviderWireDialect::PiRpc),
            ProviderRefType::KimiCode => (ProviderDialect::KimiAcpV1, ProviderWireDialect::KimiAcp),
        };
        ProviderCapability {
            provider_type: provider.provider_type,
            version: "1.0.0-t12".to_string(),
            adapter_dialect,
            wire_dialect,
            capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
            action_capability: ProviderActionCapability {
                action,
                launch: ProviderCapabilityEvidence::Confirmed,
                resume: ProviderCapabilityEvidence::Confirmed,
                write_boundary: ProviderCapabilityEvidence::Confirmed,
                projection_digest: format!("projection-digest-t12-{action:?}"),
                evidence_ref: format!("probe://t12/{action:?}"),
            },
            trust: ProviderCapabilityEvidence::Confirmed,
        }
    }
}

impl ProviderCapabilitySource for AllFourCapabilitySource {
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
        provider: &ProviderRef,
        _credential: &cadence_aria::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(Self::capability(
            provider,
            SessionPolicyAction::PlanningReadOnly,
        ))
    }
}

/// pass-through target resolver:t12 只测政策/发布面,不重复
/// `ProductionPolicyTargetResolver` 三层身份复验(lib T2/T6 已覆盖)。
pub(crate) struct PassThroughTargetResolver;

impl PolicyTargetResolver for PassThroughTargetResolver {
    fn resolve_and_revalidate(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError> {
        Ok(request.target.clone())
    }
}

/// 四家恒可用健康源(t12 单测;真实可用性门是 live 面)。
struct AlwaysHealthy(Arc<ProviderHealthSnapshot>);

impl ProviderHealthSource for AlwaysHealthy {
    fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
        self.0.clone()
    }

    fn degraded(&self) -> bool {
        false
    }
}

fn always_available_gate() -> Arc<ProviderAvailabilityGate> {
    let checked_at = Utc::now();
    let snapshot = Arc::new(ProviderHealthSnapshot {
        schema_version: 1,
        generation: 1,
        checked_at,
        providers: [
            ProviderName::ClaudeCode,
            ProviderName::Codex,
            ProviderName::Pi,
            ProviderName::KimiCode,
        ]
        .into_iter()
        .map(|provider| ProviderHealthEntry {
            provider,
            command: "t12-stub".to_string(),
            available: true,
            version: Some("1.0.0-t12".to_string()),
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

/// 只读恒信任源:`with_readonly_lc_facts` 装配参数;t12 用 Claude(不触发
/// Codex/Kimi 的 workspace trust 复验),本替身保证任何查询都 trusted。
struct AlwaysTrustedSource;

impl ProviderTrustSource for AlwaysTrustedSource {
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
            trust_key: "t12-trust".to_string(),
            trusted: true,
            ownership: None,
            detail: "t12 read-only trust fixture".to_string(),
            verified_at: Utc::now().to_rfc3339(),
        })
    }
}

/// 计数同步 adapter:LC 同步桥 spawn 证据(被拒启动不得触达)。
#[derive(Default)]
pub(crate) struct CountingSyncAdapter {
    pub(crate) runs: AtomicUsize,
}

impl ProviderAdapter for CountingSyncAdapter {
    fn run(
        &self,
        _input: &cadence_aria::protocol::contracts::AdapterInput,
    ) -> Result<AdapterOutput, ProviderAdapterError> {
        self.runs.fetch_add(1, Ordering::SeqCst);
        Ok(AdapterOutput {
            exit_code: Some(0),
            stdout: "t12-sync-ok".to_string(),
            stderr: String::new(),
            structured_output: None,
            files_modified: Vec::new(),
            duration_ms: 0,
            timeout_status: TimeoutStatus::NotTimedOut,
        })
    }
}

/// 计数流式 adapter:registry 中 Claude 的真实槽位替身,
/// `start_validated` 计数即「LC validated spawn 是否发生」。
#[derive(Default)]
pub(crate) struct CountingStreamingAdapter {
    pub(crate) validated_starts: AtomicUsize,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for CountingStreamingAdapter {
    async fn start_validated(
        &self,
        _input: ValidatedStreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.validated_starts.fetch_add(1, Ordering::SeqCst);
        let (_event_tx, events) = mpsc::channel(1);
        let (commands, _command_rx) = mpsc::channel(1);
        Ok(ProviderSession {
            events,
            commands,
            native_session_id: None,
        })
    }
}

/// t12 no-op 审计 sink:`ProviderLaunchAuditContext.audit_sink` 装配参数。
struct NoopAuditSink;

impl ToolPolicyAuditSink for NoopAuditSink {
    fn append(
        &self,
        _workspace_session_id: &str,
        _role_run_seq: u64,
        _event: DurableToolPolicyEvent,
    ) -> Result<(), ToolPolicyAuditError> {
        Ok(())
    }
}

/// t12 网关 fixture:空政策 store(缺发布面)或已发布链(漂移面)。
pub(crate) struct T12GatewayFixture {
    pub(crate) policies: AggregatePolicyArtifactStore,
    pub(crate) audit: Arc<GatewayRunAudit>,
    pub(crate) gateway: LogicalCodebaseProviderGateway,
    pub(crate) streaming_spawn_probe: Arc<CountingStreamingAdapter>,
    pub(crate) sync_spawn_probe: Arc<CountingSyncAdapter>,
}

pub(crate) fn t12_gateway(
    policies: AggregatePolicyArtifactStore,
    receipts: Option<RootRecipeReceiptStore>,
    authority_root: &Path,
) -> T12GatewayFixture {
    let mut registry = ProviderRegistry::new();
    let streaming_spawn_probe = Arc::new(CountingStreamingAdapter::default());
    registry.register(ProviderName::ClaudeCode, streaming_spawn_probe.clone());
    let sync_spawn_probe = Arc::new(CountingSyncAdapter::default());
    let audit = Arc::new(GatewayRunAudit::new());
    let gateway = LogicalCodebaseProviderGateway::with_audit(
        policies.clone(),
        Arc::new(AllFourCapabilitySource),
        Arc::new(PassThroughTargetResolver),
        Arc::new(registry),
        sync_spawn_probe.clone(),
        always_available_gate(),
        audit.clone(),
        authority_root.to_path_buf(),
    );
    let gateway = match receipts {
        Some(receipts) => gateway.with_readonly_lc_facts(
            receipts,
            Arc::new(AlwaysTrustedSource),
            Some(T12_LC_SCOPE_ID.to_string()),
        ),
        None => gateway,
    };
    T12GatewayFixture {
        policies,
        audit,
        gateway,
        streaming_spawn_probe,
        sync_spawn_probe,
    }
}

/// planning 只读启动请求(聚合根 target;cwd=canonical root)。
pub(crate) fn t12_planning_request(
    provider: ProviderRef,
    canonical_root: &Path,
) -> SessionLaunchRequest {
    SessionLaunchRequest::planning(
        T12_PROJECT_ID,
        provider,
        PolicyTarget::aggregate_root(canonical_root.to_path_buf()),
        vec![canonical_root.to_path_buf()],
        T12_CONFIG_ARTIFACT_REF,
    )
}

/// t12 流式 input(Executor + 无策略;cwd=canonical root)。
pub(crate) fn t12_streaming_input(canonical_root: &Path) -> StreamingProviderInput {
    StreamingProviderInput {
        provider_type: ProviderType::ClaudeCode,
        role: AdapterRole::Executor,
        prompt: "t12 publication-chain probe".to_string(),
        working_dir: canonical_root.to_path_buf(),
        working_directory: Some(canonical_root.to_path_buf()),
        workspace_session_id: Some("t12_workspace_session_0001".to_string()),
        resume_provider_session_id: None,
        permission_mode: ProviderPermissionMode::Auto,
        tool_policy: None,
        audit_sink: None,
        structured_output_contract: None,
        env_vars: BTreeMap::new(),
        timeout_secs: 5,
        baseline_tree: None,
    }
}

fn t12_launch_audit_context() -> ProviderLaunchAuditContext {
    ProviderLaunchAuditContext {
        workspace_session_id: "t12_workspace_session_0001".to_string(),
        role_run_seq: 1,
        audit_sink: Arc::new(NoopAuditSink),
    }
}

// ---------------------------------------------------------------------------
// 已发布链 fixture(漂移负向 case 的「发布后」状态;非成功 case 的 seed)
// ---------------------------------------------------------------------------

struct PublishedChainFixture {
    _root_guard: TempDir,
    canonical_root: PathBuf,
    paths: ProductAppPaths,
    artifact: AggregatePolicyArtifact,
    locator_path: PathBuf,
    rule_entry_path: PathBuf,
}

impl PublishedChainFixture {
    fn policies(&self) -> AggregatePolicyArtifactStore {
        AggregatePolicyArtifactStore::for_lc(self.paths.clone(), T12_LC_SCOPE_ID)
    }

    fn receipts(&self) -> RootRecipeReceiptStore {
        RootRecipeReceiptStore::for_lc(self.paths.clone(), T12_LC_SCOPE_ID)
    }
}

/// 构造「#8 已发布」固定链:非自举 artifact(revision 2)+ canonical root
/// locator 原字节 + AGENTS.md 根规则 + 四命令 Allowed receipt + finalize。
/// `receipt_policy_digest_override` 注入与 artifact.digest 不一致的
/// policy_digest 时,产出 receipt 链漂移变体(validate 阶段即拒)。
fn build_published_chain(receipt_policy_digest_override: Option<&str>) -> PublishedChainFixture {
    let root_guard = tempdir().expect("t12 published-chain root");
    let canonical_root = root_guard.path().canonicalize().expect("canonical root");
    let rule_entry_path = canonical_root.join("AGENTS.md");
    let rule_bytes =
        b"# Aggregate root rule entry (t12 fixture)\nSingle root policy entry for the four providers.\n"
            .to_vec();
    std::fs::write(&rule_entry_path, &rule_bytes).expect("write AGENTS.md");
    let rule_digest = format!("sha256:{:x}", Sha256::digest(&rule_bytes));

    let paths = ProductAppPaths::new(canonical_root.join(".aria"));

    // 非自举 artifact:bootstrap(revision 1)→ with_revised_policy(revision 2,
    // digest 由正文重算,不接受外部传入)。
    let created_at = Utc::now().to_rfc3339();
    let bootstrap = AggregatePolicyArtifact::bootstrap(
        T12_PROJECT_ID,
        T12_LOGICAL_CODEBASE_UUID,
        created_at.clone(),
    );
    let artifact = bootstrap.with_revised_policy(
        "# Aggregate policy (t12 published)\n\nPublished by the #8 chain for drift acceptance.\n",
        created_at,
    );
    AggregatePolicyArtifactStore::for_lc(paths.clone(), T12_LC_SCOPE_ID)
        .save(T12_PROJECT_ID, &artifact)
        .expect("save published artifact");

    // locator:canonical root 下 policy_id 原字节(#8 消费契约)。
    let locator_path = canonical_root.join(&artifact.policy_id);
    if let Some(parent) = locator_path.parent() {
        std::fs::create_dir_all(parent).expect("locator parents");
    }
    std::fs::write(&locator_path, artifact.policy_text.as_bytes()).expect("write locator raw body");

    // 四命令 Allowed receipt + finalize(receipt 漂移变体注入错 policy_digest)。
    let auditor = RootRecipeFilesystemAuditor::new();
    let receipts = RootRecipeReceiptStore::for_lc(paths.clone(), T12_LC_SCOPE_ID);
    let operation_id = "operation_0001";
    for (step, command_index, command) in root_recipe_command_index() {
        let watch = auditor
            .before_command(operation_id, &canonical_root, step, command_index, command)
            .expect("before_command snapshot");
        let receipt = auditor
            .after_command(watch, Utc::now().to_rfc3339())
            .expect("after_command receipt");
        receipts
            .append_command(T12_PROJECT_ID, receipt)
            .expect("append command receipt");
    }
    let frozen_policy_digest = receipt_policy_digest_override.unwrap_or(artifact.digest.as_str());
    receipts
        .finalize(
            T12_PROJECT_ID,
            operation_id,
            frozen_policy_digest,
            &rule_digest,
            Utc::now().to_rfc3339(),
        )
        .expect("finalize root recipe receipt");

    PublishedChainFixture {
        _root_guard: root_guard,
        canonical_root,
        paths,
        artifact,
        locator_path,
        rule_entry_path,
    }
}

// ---------------------------------------------------------------------------
// Step 1 测试 1:缺发布阻塞——无 fixture seed 时四家全部 fail-closed
// ---------------------------------------------------------------------------

/// 计划 519 行 `lcg_t12_missing_publication_blocks_every_provider_without_
/// fixture_seed`:空政策 store(零 seed 物化)下,四家 validate 全部
/// `PolicyMissing`,且网关不自行物化任何政策/发布事实、零启动记录。
#[test]
fn lcg_t12_missing_publication_blocks_every_provider_without_fixture_seed() {
    let root_guard = tempdir().expect("t12 empty root");
    let canonical_root = root_guard.path().canonicalize().expect("canonical root");
    let paths = ProductAppPaths::new(canonical_root.join(".aria"));
    let fixture = t12_gateway(
        AggregatePolicyArtifactStore::for_lc(paths.clone(), T12_LC_SCOPE_ID),
        None,
        &canonical_root,
    );

    let providers = [
        ProviderRef::claude_code("t12-cap-claude"),
        ProviderRef::codex("t12-cap-codex"),
        ProviderRef::pi("t12-cap-pi"),
        ProviderRef::kimi_code("t12-cap-kimi"),
    ];
    for provider in &providers {
        let error = fixture
            .gateway
            .validate(t12_planning_request(provider.clone(), &canonical_root))
            .expect_err("缺发布时 validate 必须 fail-closed");
        assert!(
            matches!(&error, ProviderGatewayError::PolicyMissing(project) if project == T12_PROJECT_ID),
            "provider {:?} 缺发布必须 PolicyMissing(得到 {error:?})",
            provider.provider_type
        );
    }

    // fail-closed 不是「顺手 seed」:store 必须仍然为空,启动审计必须零记录。
    assert!(
        fixture
            .policies
            .get(T12_PROJECT_ID)
            .expect("read policy store")
            .is_none(),
        "缺发布阻塞不得物化任何政策 artifact(无 fixture seed 语义)"
    );
    assert_eq!(
        fixture.audit.sync_launches() + fixture.audit.stream_launches(),
        0,
        "缺发布阶段不得有任何 sync/stream 启动记录"
    );
    assert_eq!(
        fixture
            .streaming_spawn_probe
            .validated_starts
            .load(Ordering::SeqCst),
        0,
        "缺发布阶段 registry adapter 不得被触达"
    );
    assert_eq!(
        fixture.sync_spawn_probe.runs.load(Ordering::SeqCst),
        0,
        "缺发布阶段同步桥 adapter 不得被触达"
    );
}

// ---------------------------------------------------------------------------
// Step 1 测试 2:正文/revision/receipt 漂移在 spawn 之前阻断
// ---------------------------------------------------------------------------

/// 计划 519 行 `lcg_t12_body_revision_receipt_drift_blocks_before_spawn`:
/// validate→spawn 之间任一发布链维度漂移(正文篡改/正文缺失/政策升级/
/// 根规则篡改),`start_streaming` 必须在 registry lookup 与真实 adapter
/// spawn 之前 fail-closed。
///
/// 注:原计划把「receipt 链 digest 漂移」也列为本用例的阻断维度;2026-10-27
/// 的政策链改动(提交 85497679「集成测试适配 verify_published_policy_body 新
/// 签名」)已把政策权威收敛为**当前已发布政策**(artifact),回执只证明
/// 「已发布」这一事实存在 → receipt digest 漂移不再阻断,独立成例
/// `lcg_t12_receipt_digest_drift_is_tolerated_because_published_policy_is_authoritative`。
#[tokio::test]
async fn lcg_t12_body_revision_drift_blocks_before_spawn() {
    // ---- 场景 A:正文漂移(locator 原字节被篡改)→ policy_body ----
    {
        let chain = build_published_chain(None);
        let fixture = t12_gateway(
            chain.policies(),
            Some(chain.receipts()),
            &chain.canonical_root,
        );
        let launch = fixture
            .gateway
            .prepare_streaming_launch(
                t12_streaming_input(&chain.canonical_root),
                t12_planning_request(
                    ProviderRef::claude_code("t12-cap-claude"),
                    &chain.canonical_root,
                ),
                t12_launch_audit_context(),
            )
            .expect("发布链一致时 prepare 必须通过");
        std::fs::write(&chain.locator_path, b"tampered policy body (t12)")
            .expect("tamper locator raw body");
        let error = match fixture
            .gateway
            .start_streaming(launch, CancellationToken::new())
            .await
        {
            Ok(_session) => panic!("正文漂移必须在 spawn 前阻断(不得返回会话)"),
            Err(error) => error,
        };
        assert!(
            matches!(&error, ProviderGatewayError::PolicyDrift { dimension } if dimension == "policy_body"),
            "正文漂移判别维度必须是 policy_body(得到 {error:?})"
        );
        assert_eq!(
            fixture
                .streaming_spawn_probe
                .validated_starts
                .load(Ordering::SeqCst),
            0,
            "正文漂移被拒时 adapter 不得被 spawn"
        );
        assert_eq!(fixture.audit.stream_launches(), 0, "漂移被拒不落启动审计");
    }

    // ---- 场景 B:正文缺失(locator 被删)→ 零 spawn(计划 530 行) ----
    {
        let chain = build_published_chain(None);
        let fixture = t12_gateway(
            chain.policies(),
            Some(chain.receipts()),
            &chain.canonical_root,
        );
        let launch = fixture
            .gateway
            .prepare_streaming_launch(
                t12_streaming_input(&chain.canonical_root),
                t12_planning_request(
                    ProviderRef::claude_code("t12-cap-claude"),
                    &chain.canonical_root,
                ),
                t12_launch_audit_context(),
            )
            .expect("prepare 必须通过");
        std::fs::remove_file(&chain.locator_path).expect("remove locator body");
        let spawn_error = match fixture
            .gateway
            .start_streaming(launch, CancellationToken::new())
            .await
        {
            Ok(_session) => panic!("正文缺失必须阻断 spawn(不得返回会话)"),
            Err(error) => error,
        };
        assert!(
            matches!(&spawn_error, ProviderGatewayError::Target(message) if message.contains("re-read policy locator")),
            "正文缺失必须在 locator 重读处 fail-closed(得到 {spawn_error:?})"
        );
        let spawn_count_with_missing_body = fixture
            .streaming_spawn_probe
            .validated_starts
            .load(Ordering::SeqCst);
        assert_eq!(
            spawn_count_with_missing_body, 0,
            "计划 530 行:正文缺失零 spawn"
        );
    }

    // ---- 场景 C:revision 漂移(validate 后政策升级)→ policy_revision ----
    {
        let chain = build_published_chain(None);
        let fixture = t12_gateway(
            chain.policies(),
            Some(chain.receipts()),
            &chain.canonical_root,
        );
        let launch = fixture
            .gateway
            .prepare_streaming_launch(
                t12_streaming_input(&chain.canonical_root),
                t12_planning_request(
                    ProviderRef::claude_code("t12-cap-claude"),
                    &chain.canonical_root,
                ),
                t12_launch_audit_context(),
            )
            .expect("prepare 必须通过");
        chain
            .policies()
            .save(
                T12_PROJECT_ID,
                &chain.artifact.with_revised_policy(
                    "# Aggregate policy (t12 revised)\n",
                    Utc::now().to_rfc3339(),
                ),
            )
            .expect("publish revised artifact");
        let error = match fixture
            .gateway
            .start_streaming(launch, CancellationToken::new())
            .await
        {
            Ok(_session) => panic!("revision 漂移必须在 spawn 前阻断(不得返回会话)"),
            Err(error) => error,
        };
        assert!(
            matches!(&error, ProviderGatewayError::PolicyDrift { dimension } if dimension == "policy_revision"),
            "revision 漂移判别维度必须是 policy_revision(得到 {error:?})"
        );
        assert_eq!(
            fixture
                .streaming_spawn_probe
                .validated_starts
                .load(Ordering::SeqCst),
            0,
            "revision 漂移被拒时 adapter 不得被 spawn"
        );
    }

    // ---- 场景 D:根规则漂移(AGENTS.md 原字节篡改)→ rule_digest ----
    {
        let chain = build_published_chain(None);
        let fixture = t12_gateway(
            chain.policies(),
            Some(chain.receipts()),
            &chain.canonical_root,
        );
        let launch = fixture
            .gateway
            .prepare_streaming_launch(
                t12_streaming_input(&chain.canonical_root),
                t12_planning_request(
                    ProviderRef::claude_code("t12-cap-claude"),
                    &chain.canonical_root,
                ),
                t12_launch_audit_context(),
            )
            .expect("prepare 必须通过");
        std::fs::write(&chain.rule_entry_path, b"# tampered root rule entry\n")
            .expect("tamper AGENTS.md");
        let error = match fixture
            .gateway
            .start_streaming(launch, CancellationToken::new())
            .await
        {
            Ok(_session) => panic!("根规则漂移必须在 spawn 前阻断(不得返回会话)"),
            Err(error) => error,
        };
        assert!(
            matches!(&error, ProviderGatewayError::PolicyDrift { dimension } if dimension == "rule_digest"),
            "根规则漂移判别维度必须是 rule_digest(得到 {error:?})"
        );
        assert_eq!(
            fixture
                .streaming_spawn_probe
                .validated_starts
                .load(Ordering::SeqCst),
            0,
            "根规则漂移被拒时 adapter 不得被 spawn"
        );
    }

    // ---- 场景 E(receipt 链 digest 漂移)已独立成例 ----
    //      见 `lcg_t12_receipt_digest_drift_is_tolerated_because_published_policy_is_authoritative`
    //      (政策权威=当前已发布政策;回执只证明「已发布」存在,不参与 digest 权威)。
}

/// 计划 519 行 `lcg_t12_body_revision_receipt_drift_blocks_before_spawn` 的
/// receipt 链 digest 漂移维度(独立成例):回执里冻结的 policy_digest 与
/// artifact.digest 脱钩时,`prepare_streaming_launch` **必须成功**,且冻结的
/// locator `policy_digest` 必须等于 `artifact.digest`(不得采用被篡改的回执
/// digest)。
///
/// 依据(这是**产品行为**,不是缺陷——勿按缺陷改回):
/// - 提交 85497679(2026-10-27,「fix(test): 集成测试适配
///   verify_published_policy_body 新签名」)把政策链的权威收敛为**当前已发布
///   政策**:该函数的签名从 `(authority_root, artifact, receipt)` 收敛为
///   `(authority_root, artifact)`,只校验 artifact 身份 / 磁盘正文 ==
//   artifact 正文 / `artifact.digest == sha256(正文)` / 根规则摘要。
/// - `provider_gateway.rs` 的 `assemble_validated` 里
///   `receipts.latest_finalized(...)` 的**回执值被丢弃**(`|_receipt|`),只用于
///   判断「回执存在」;冻结的 `policy_digest` 直接取 `artifact.digest`。
///
/// 即:回执仅证明「已发布」这一事实存在,digest 权威=当前已发布政策——回执
/// digest 漂移被容忍,且产出的 locator digest 必须是 artifact.digest。
#[tokio::test]
async fn lcg_t12_receipt_digest_drift_is_tolerated_because_published_policy_is_authoritative() {
    let chain = build_published_chain(Some("sha256:t12-receipt-chain-drift"));
    let fixture = t12_gateway(
        chain.policies(),
        Some(chain.receipts()),
        &chain.canonical_root,
    );
    let launch = fixture
        .gateway
        .prepare_streaming_launch(
            t12_streaming_input(&chain.canonical_root),
            t12_planning_request(
                ProviderRef::claude_code("t12-cap-claude"),
                &chain.canonical_root,
            ),
            t12_launch_audit_context(),
        )
        .expect("回执 digest 漂移不被拒:政策权威=当前已发布政策(artifact)");
    let (policy_digest, _rule_digest) = launch
        .policy_locator_digests()
        .expect("已发布链必须冻结 policy locator digest 事实");
    assert_eq!(
        policy_digest,
        chain.artifact.digest.as_str(),
        "冻结的 policy_digest 必须是当前已发布政策的 digest(不得采用被篡改的回执 digest)"
    );
    assert_ne!(
        policy_digest, "sha256:t12-receipt-chain-drift",
        "被篡改的回执 digest 不得进入 policy locator"
    );
    assert_eq!(
        fixture
            .streaming_spawn_probe
            .validated_starts
            .load(Ordering::SeqCst),
        0,
        "prepare 阶段不得触达 adapter spawn"
    );
}
