//! Task 12a:#8 政策发布依赖面——缺发布 fail-closed、正文/revision/receipt
//! 漂移 spawn 前阻断(实施计划 Task 12 段 513-541 行;REQ-LCG-02/07)。
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
use std::time::Duration;

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
use cadence_aria::product::logical_codebase::aggregate_initialization::AggregateInitializationStepKind;
use cadence_aria::product::logical_codebase::aggregate_initialization_store::root_recipe_command_index;
use cadence_aria::product::logical_codebase::policy::{
    AggregatePolicyArtifact, PolicyTarget, ProviderDialect, ProviderWireDialect,
    SessionPolicyAction,
};
use cadence_aria::product::logical_codebase::provider_capability_store::ProviderActionCapability;
use cadence_aria::product::logical_codebase::provider_gateway::{
    ProviderLaunchAuditContext, verify_published_policy_body,
};
use cadence_aria::product::logical_codebase::provider_trust::{
    ProviderTrustSource, ProviderTrustVerification,
};
use cadence_aria::product::logical_codebase::root_recipe_receipt::{
    RootRecipeFilesystemAuditor, RootRecipeReceipt,
};
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

use super::harness::LiveMatrixFailure;

const T12_PROJECT_ID: &str = "project_0001";
const T12_LC_SCOPE_ID: &str = "lc_0001";
const T12_LOGICAL_CODEBASE_UUID: &str = "00000000-0000-0000-0000-000000000012";
const T12_CONFIG_ARTIFACT_REF: &str = "sha256:t12-managed-config-artifact";
/// 计划 519 行:固定 Claude recipe 五步四命令(Task 1.5 冻结布局)。
const T12_RECIPE_STEP_COUNT: usize = 5;
const T12_RECIPE_COMMAND_COUNT: usize = 4;

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
        if provider.provider_type != ProviderRefType::ClaudeCode {
            return Err(ProviderGatewayError::UnsupportedCapability(
                cadence_aria::product::logical_codebase::provider_gateway::PROVIDER_ROOT_RECIPE_REQUIRES_FIXED_CLAUDE
                    .to_string(),
            ));
        }
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
/// spawn 之前 fail-closed;receipt 链 digest 漂移在 validate 阶段即拒。
#[tokio::test]
async fn lcg_t12_body_revision_receipt_drift_blocks_before_spawn() {
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

    // ---- 场景 E:receipt 链漂移(policy_digest 与 artifact.digest 脱钩)
    //      → validate 阶段 policy_digest_chain 即拒,spawn 无从发生 ----
    {
        let chain = build_published_chain(Some("sha256:t12-receipt-chain-drift"));
        let fixture = t12_gateway(
            chain.policies(),
            Some(chain.receipts()),
            &chain.canonical_root,
        );
        let error = fixture
            .gateway
            .prepare_streaming_launch(
                t12_streaming_input(&chain.canonical_root),
                t12_planning_request(
                    ProviderRef::claude_code("t12-cap-claude"),
                    &chain.canonical_root,
                ),
                t12_launch_audit_context(),
            )
            .expect_err("receipt 链 digest 漂移必须在 validate 阶段拒绝");
        assert!(
            matches!(&error, ProviderGatewayError::PolicyDrift { dimension } if dimension == "policy_digest_chain"),
            "receipt 链漂移判别维度必须是 policy_digest_chain(得到 {error:?})"
        );
        assert_eq!(
            fixture
                .streaming_spawn_probe
                .validated_starts
                .load(Ordering::SeqCst),
            0,
            "receipt 链漂移被拒时 adapter 不得被 spawn"
        );
    }
}

// ---------------------------------------------------------------------------
// 只读验收:assert_policy_publication_chain(计划 Interfaces 面;
// live 骨架消费,生产 #8 链事实校验零副作用)
// ---------------------------------------------------------------------------

/// #8 发布链观测事实(全部由公共只读 API 采集,见 live 骨架)。
pub(crate) struct PolicyPublicationChainObservation<'a> {
    pub(crate) canonical_root: &'a Path,
    pub(crate) artifact: &'a AggregatePolicyArtifact,
    pub(crate) receipt: &'a RootRecipeReceipt,
    /// recipe 驱动 provider(生产固定 Claude)。
    pub(crate) recipe_provider: ProviderName,
}

fn chain_failure(message: String) -> LiveMatrixFailure {
    LiveMatrixFailure {
        reason_code: "policy_publication_chain_violated".to_string(),
        message,
        stage: Some("publication_chain".to_string()),
    }
}

/// 计划 521-529 行断言集的只读实现:locator 原字节/digest 三方一致/
/// receipt 根与 revision 链/固定 Claude recipe 五步四命令/独立 rule digest。
/// 违反即 `Err(LiveMatrixFailure)`,绝不物化/修复任何事实。
pub(crate) fn assert_policy_publication_chain(
    observation: &PolicyPublicationChainObservation<'_>,
) -> Result<(), LiveMatrixFailure> {
    let PolicyPublicationChainObservation {
        canonical_root,
        artifact,
        receipt,
        recipe_provider,
    } = observation;

    // 1. locator 原字节(#8:policy_id 文件原字节 == artifact.policy_text)。
    let locator = canonical_root.join(&artifact.policy_id);
    let raw_body = std::fs::read(&locator).map_err(|error| {
        chain_failure(format!(
            "read policy locator {}: {error}",
            locator.display()
        ))
    })?;
    if raw_body != artifact.policy_text.as_bytes() {
        return Err(chain_failure(format!(
            "locator raw body != artifact.policy_text at {}",
            locator.display()
        )));
    }

    // 2. raw SHA-256 == artifact.digest(#8 消费契约:raw UTF-8 字节摘要)。
    let body_digest = format!("sha256:{:x}", Sha256::digest(&raw_body));
    if body_digest != artifact.digest {
        return Err(chain_failure(format!(
            "sha256(raw body) {body_digest} != artifact.digest {}",
            artifact.digest
        )));
    }

    // 3. receipt 链:policy_digest == artifact.digest;canonical_root 一致。
    if receipt.policy_digest != artifact.digest {
        return Err(chain_failure(format!(
            "receipt.policy_digest {} != artifact.digest {}",
            receipt.policy_digest, artifact.digest
        )));
    }
    let canonical_receipt_root = receipt.canonical_root.canonicalize().map_err(|error| {
        chain_failure(format!(
            "canonicalize receipt root {}: {error}",
            receipt.canonical_root.display()
        ))
    })?;
    let canonical_observed_root = canonical_root.canonicalize().map_err(|error| {
        chain_failure(format!(
            "canonicalize canonical root {}: {error}",
            canonical_root.display()
        ))
    })?;
    if canonical_receipt_root != canonical_observed_root {
        return Err(chain_failure(format!(
            "receipt.canonical_root {} != canonical root {}",
            receipt.canonical_root.display(),
            canonical_root.display()
        )));
    }

    // 4. locator 携带的 revision == artifact.revision(policy_id 尾段)。
    let policy_revision_in_locator = artifact
        .policy_id
        .rsplit('/')
        .next()
        .and_then(|revision| revision.parse::<u64>().ok())
        .ok_or_else(|| {
            chain_failure(format!(
                "policy_id {} 不携带可解析 revision 尾段",
                artifact.policy_id
            ))
        })?;
    if policy_revision_in_locator != artifact.revision {
        return Err(chain_failure(format!(
            "locator revision {policy_revision_in_locator} != artifact.revision {}",
            artifact.revision
        )));
    }

    // 5. 独立 rule digest:AGENTS.md 原字节 SHA-256 == receipt.rule_digest。
    let rule_entry = canonical_root.join("AGENTS.md");
    let rule_bytes = std::fs::read(&rule_entry).map_err(|error| {
        chain_failure(format!(
            "read root rule entry {}: {error}",
            rule_entry.display()
        ))
    })?;
    let rule_digest = format!("sha256:{:x}", Sha256::digest(&rule_bytes));
    if receipt.rule_digest != rule_digest {
        return Err(chain_failure(format!(
            "receipt.rule_digest {} != sha256(AGENTS.md) {rule_digest}",
            receipt.rule_digest
        )));
    }

    // 6. 固定 Claude recipe:五步四命令(计划 527-529 行)。
    if *recipe_provider != ProviderName::ClaudeCode {
        return Err(chain_failure(format!(
            "recipe provider 必须是 ClaudeCode(得到 {recipe_provider:?})"
        )));
    }
    let recipe_step_count = AggregateInitializationStepKind::V1.len();
    if recipe_step_count != T12_RECIPE_STEP_COUNT {
        return Err(chain_failure(format!(
            "固定 recipe 步数必须是 {T12_RECIPE_STEP_COUNT}(得到 {recipe_step_count})"
        )));
    }
    let frozen_index = root_recipe_command_index();
    let recipe_command_count = frozen_index.len();
    if recipe_command_count != T12_RECIPE_COMMAND_COUNT {
        return Err(chain_failure(format!(
            "固定 recipe 命令数必须是 {T12_RECIPE_COMMAND_COUNT}(得到 {recipe_command_count})"
        )));
    }
    if receipt.commands.len() != T12_RECIPE_COMMAND_COUNT {
        return Err(chain_failure(format!(
            "receipt.commands 必须逐条对应四命令(得到 {})",
            receipt.commands.len()
        )));
    }
    for (summary, (step, command_index, command)) in receipt.commands.iter().zip(&frozen_index) {
        if summary.command_index != *command_index
            || summary.step != *step
            || summary.command != *command
        {
            return Err(chain_failure(format!(
                "receipt 命令摘要与冻结命令索引不一致:idx {} 期望 ({step:?}, {command})",
                summary.command_index
            )));
        }
    }

    // 7. 公共只读校验面二次复核(#8 delivered 实现,交叉一致性)。
    verify_published_policy_body(canonical_root, artifact, receipt)
        .map_err(|error| chain_failure(format!("verify_published_policy_body 拒绝: {error}")))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// 真实 live 骨架:新 fresh LC + 真实 Claude recipe + #8 发布链只读验收
// ---------------------------------------------------------------------------

/// 计划 519/538 行 `lcg_live_fresh_lc_policy_publication_recipe_and_
/// direct_comparison`:fresh fixture 完全由产品 API 组装(项目/manifest 落盘
/// + 真实生产依赖图 + 真实 Claude recipe),发布链观测全部来自公共只读
/// store;无手工 seed/物化。真实现场执行归 controller:
/// `LC_GATEWAY_E2E=1 cargo test --locked --test it_web
///  lcg_live_fresh_lc_policy_publication_recipe_and_direct_comparison
///  -- --ignored --nocapture --test-threads=1`。
#[tokio::test]
#[ignore = "真实 E2E:需要 LC_GATEWAY_E2E=1 与真实 claude CLI(recipe 固定 Claude)"]
async fn lcg_live_fresh_lc_policy_publication_recipe_and_direct_comparison() {
    super::live_matrix::require_lc_gateway_e2e_switch();

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use cadence_aria::product::logical_codebase::AggregateInitializationOperationStatus;
    use cadence_aria::product::logical_codebase::AggregateInitializationOperationStore;
    use cadence_aria::product::logical_codebase::LogicalCodebaseManifest;
    use cadence_aria::product::logical_codebase::LogicalCodebaseStore;
    use cadence_aria::product::project_store::{CreateProjectInput, ProjectStore};
    use cadence_aria::web::app::build_web_router;
    use cadence_aria::web::runtime::WebRuntime;
    use cadence_aria::web::state::WebAppState;
    use tower::ServiceExt;

    // ---- fresh fixture:零 seed(tempdir 持有至断言结束)----
    let root_guard = tempdir().expect("live fresh root");
    let root_path = root_guard.path().to_path_buf();
    let paths = ProductAppPaths::new(root_path.join(".aria"));
    ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "lcg t12 live fresh publication recipe".to_string(),
            description: None,
        })
        .expect("create project fixture");
    let aggregate_root = root_path.join("aggregate-root");
    std::fs::create_dir_all(&aggregate_root).expect("fresh aggregate root");
    let manifest = LogicalCodebaseManifest::new(T12_PROJECT_ID, aggregate_root.clone(), Vec::new());
    LogicalCodebaseStore::new(paths.clone())
        .save_manifest(T12_PROJECT_ID, &manifest)
        .expect("save fresh manifest");

    // ---- 真实生产依赖图(默认装配,不注入任何 fake)----
    let state = WebAppState::new(
        root_path.clone(),
        WebRuntime::new_real(root_path.clone()).expect("real web runtime"),
    );
    let app = build_web_router(state);

    // 公共 HTTP JSON 请求辅助(live 骨架内嵌,不依赖 harness 私有面)。
    async fn json_request(
        app: &axum::Router,
        method: axum::http::Method,
        uri: &str,
        body: serde_json::Value,
    ) -> (axum::http::StatusCode, serde_json::Value) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("build json request"),
            )
            .await
            .expect("json request");
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("json request body");
        let value = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, value)
    }

    // ---- provider health 预热(公共端点;矩阵 harness 同语义先例)----
    // `WebRuntime::new_real` 的健康源需要主动 recheck 刷新 + 有界轮询;
    // 未就绪时五步 init 的 provider turn spawn 复验会以
    // provider_gateway_unavailable(health degraded)拒绝 pre_check
    // (2026-10-09 集成现场:pre_check_failed/spawn_revalidation_drift)。
    // 此处经 /api/providers/recheck + /api/providers/status 等价实现,
    // 不复制 harness 私有面;claude(recipe 固定驱动)ready 才放行 POST。
    let health_deadline = std::time::Instant::now()
        + Duration::from_secs(
            std::env::var("LCG_T12_HEALTH_TIMEOUT_SECS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(180),
        );
    loop {
        let _ = json_request(
            &app,
            axum::http::Method::POST,
            "/api/providers/recheck",
            serde_json::json!({}),
        )
        .await;
        let (status, health_body) = json_request(
            &app,
            axum::http::Method::GET,
            "/api/providers/status",
            serde_json::json!({}),
        )
        .await;
        let claude_ready = status.is_success()
            && health_body["state_status"] == "ready"
            && health_body["providers"].as_array().is_some_and(|entries| {
                entries
                    .iter()
                    .any(|entry| entry["provider"] == "claude_code" && entry["available"] == true)
            });
        if claude_ready {
            break;
        }
        assert!(
            std::time::Instant::now() < health_deadline,
            "provider health 未就绪(claude_code):{health_body} (环境不可运行须报告 BLOCKED)"
        );
        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    // ---- 驱动真实五步 aggregate initialization(固定 Claude recipe)----
    let (create_status, value) = json_request(
        &app,
        axum::http::Method::POST,
        &format!("/api/projects/{T12_PROJECT_ID}/logical-codebase/initializations"),
        serde_json::json!({"idempotency_key": "lcg-t12-live-fresh"}),
    )
    .await;
    assert_eq!(create_status, StatusCode::ACCEPTED);
    assert_eq!(
        value["steps"].as_array().expect("steps").len(),
        T12_RECIPE_STEP_COUNT,
        "五步 operation 布局必须冻结"
    );
    let operation_id = value["operation_id"]
        .as_str()
        .expect("operation_id")
        .to_string();

    // ---- 轮询 durable operation 至终态(默认 15 分钟,可 env 覆盖)----
    let operations = AggregateInitializationOperationStore::new(paths.clone());
    let deadline = std::time::Instant::now()
        + Duration::from_secs(
            std::env::var("LCG_T12_LIVE_TIMEOUT_SECS")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(900),
        );
    let operation = loop {
        let operation = operations
            .get(T12_PROJECT_ID, &operation_id)
            .expect("read durable operation");
        match operation.status {
            AggregateInitializationOperationStatus::Completed => break operation,
            AggregateInitializationOperationStatus::Failed => {
                panic!("真实 aggregate initialization 失败: {:?}", operation.error)
            }
            AggregateInitializationOperationStatus::Created
            | AggregateInitializationOperationStatus::Running => {}
            AggregateInitializationOperationStatus::Cancelled => {
                panic!("真实 aggregate initialization 被取消({operation_id})")
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "真实 aggregate initialization 超时未达终态({operation_id})"
        );
        tokio::time::sleep(Duration::from_secs(2)).await;
    };
    assert_eq!(
        operation.input.provider, "claude_code",
        "生产 recipe 固定 Claude 驱动"
    );

    // ---- 只读采集 #8 发布链事实(公共 store,零副作用)----
    let manifest = LogicalCodebaseStore::new(paths.clone())
        .load_manifest(T12_PROJECT_ID)
        .expect("load manifest")
        .expect("manifest 存在");
    let canonical_root = manifest
        .provider_context_root
        .canonicalize()
        .expect("canonical");
    let artifact = AggregatePolicyArtifactStore::new(paths.clone())
        .get(T12_PROJECT_ID)
        .expect("read published artifact")
        .expect("真实发布后必须存在政策 artifact");
    assert!(
        artifact.revision >= 2,
        "发布链 artifact 必须是非自举升级版(revision>=2,得到 {})",
        artifact.revision
    );
    let receipt = RootRecipeReceiptStore::new(paths.clone())
        .latest_finalized(T12_PROJECT_ID)
        .expect("read finalized receipt")
        .expect("真实 recipe 完成后必须存在 finalized receipt");

    // ---- 只读验收:发布链 + 固定 Claude recipe 五步四命令 ----
    assert_policy_publication_chain(&PolicyPublicationChainObservation {
        canonical_root: &canonical_root,
        artifact: &artifact,
        receipt: &receipt,
        recipe_provider: ProviderName::ClaudeCode,
    })
    .expect("fresh #8 发布链只读验收");

    // ---- direct 对照段(计划 531-533 行;确定性拓扑观测,与
    //      direct_comparison.rs 单测同源;真实 argv/cwd/output 基线归
    //      Task 10/11 live 矩阵 cell)----
    let sync_before = super::direct_comparison::capture_sync_direct_topology();
    let streaming_before =
        super::direct_comparison::capture_workspace_streaming_legacy_shape().await;
    let sync_after = super::direct_comparison::capture_sync_direct_topology();
    let streaming_after =
        super::direct_comparison::capture_workspace_streaming_legacy_shape().await;
    assert_eq!(sync_after, sync_before, "计划 531 行:sync direct 拓扑不变");
    assert_eq!(
        streaming_after, streaming_before,
        "计划 532 行:workspace streaming legacy 四家形态不变"
    );
}
