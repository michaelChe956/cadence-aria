// F3 Task 4.1（restrict-role-write-tools，GC12；修复轮 Minor3 收窄）：kimi 零变化回归。
// ①argv 冻结：仅 `acp`，不注入任何 tool-policy 物理片段（pi/claude/codex 的
//   denylist 片段不得串入 kimi）；
// ②kimi 不读 `tool_policy`/`audit_sink`：即使 input 误挂策略与传入的 sink，
//   provider 也不调用该 sink（零追加）——断言范围收窄为 provider 行为本身；
//   分区文件级断言（tool-policy-run-audit/ 永不因 kimi 产生）已由
//   coding/workspace 侧真实 LifecycleStore 根上的隔离测试覆盖；
// ③出站 request id 保持既有数字命名空间（initialize=1/session/new=2/
//   session/prompt=3，Task 2.2 只升级 codex peer 到 aria-<seq>）。

use std::collections::BTreeMap;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::json_rpc_peer::JsonRpcPeer;
use crate::cross_cutting::kimi_code_provider::session::run_kimi_session;
use crate::cross_cutting::kimi_code_provider::{
    KimiCodeProvider, tests::session_tests::fixture_command,
};
use crate::cross_cutting::streaming_provider::{
    ProviderEvent, ProviderPermissionMode, ProviderToolPolicy, StreamingProviderAdapter,
    StreamingProviderInput,
};
use crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink;
use crate::protocol::contracts::{AdapterRole, ProviderType};

fn kimi_input(role: AdapterRole) -> StreamingProviderInput {
    StreamingProviderInput {
        working_directory: None,
        baseline_tree: None,
        tool_policy: None,
        audit_sink: None,
        provider_type: ProviderType::KimiCode,
        role,
        prompt: "kimi zero change prompt".to_string(),
        working_dir: std::env::current_dir().expect("working directory"),
        workspace_session_id: None,
        resume_provider_session_id: None,
        permission_mode: ProviderPermissionMode::Auto,
        structured_output_contract: None,
        env_vars: BTreeMap::new(),
        timeout_secs: 10,
    }
}

async fn read_wire_request(
    reader: &mut BufReader<tokio::io::ReadHalf<tokio::io::DuplexStream>>,
) -> Value {
    let mut line = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        reader.read_line(&mut line),
    )
    .await
    .expect("wire line timeout")
    .expect("wire line");
    assert!(!line.trim().is_empty(), "unexpected empty wire line");
    serde_json::from_str(line.trim()).expect("wire json")
}

async fn write_wire_line(
    writer: &mut tokio::io::WriteHalf<tokio::io::DuplexStream>,
    value: &Value,
) {
    writer
        .write_all(value.to_string().as_bytes())
        .await
        .expect("write wire line");
    writer.write_all(b"\n").await.expect("write wire newline");
}

/// ①argv 冻结：kimi build_args 是完整向量等值 `"acp"`——不因本 change 混入
/// `--exclude-tools`/`--disallowedTools` 等策略片段（向量等值已覆盖，无需
/// 冗余 contains 检查）。
#[test]
fn kimi_build_args_stay_policy_free() {
    let provider = KimiCodeProvider::new("kimi".into());
    assert_eq!(provider.build_args(), vec!["acp".to_string()]);
}

/// ③出站 request id 保持数字：真实 `run_kimi_session` wire 级断言
/// initialize=1、session/new=2、session/prompt=3（JSON 数字，非 aria-<seq>）。
#[tokio::test]
async fn kimi_outbound_request_ids_stay_numeric() {
    let (client, server) = tokio::io::duplex(16 * 1024);
    let (reader, writer) = tokio::io::split(client);
    let peer = JsonRpcPeer::new(reader, writer);
    let (event_tx, mut event_rx) = mpsc::channel(32);
    let (_command_tx, command_rx) = mpsc::channel(8);

    let server_task = tokio::spawn(async move {
        let (server_reader, mut server_writer) = tokio::io::split(server);
        let mut reader = BufReader::new(server_reader);

        let initialize = read_wire_request(&mut reader).await;
        assert_eq!(initialize["method"], "initialize");
        assert_eq!(
            initialize["id"],
            json!(1),
            "kimi initialize id stays numeric 1"
        );
        write_wire_line(
            &mut server_writer,
            &json!({
                "jsonrpc":"2.0", "id": initialize["id"].clone(),
                "result": {"protocolVersion": 1, "agentCapabilities": {"loadSession": true}}
            }),
        )
        .await;

        // notifications/initialized：通知无 id。
        let initialized = read_wire_request(&mut reader).await;
        assert_eq!(initialized["method"], "notifications/initialized");
        assert!(initialized.get("id").is_none());

        let session_new = read_wire_request(&mut reader).await;
        assert_eq!(session_new["method"], "session/new");
        assert_eq!(
            session_new["id"],
            json!(2),
            "kimi session/new id stays numeric 2"
        );
        write_wire_line(
            &mut server_writer,
            &json!({
                "jsonrpc":"2.0", "id": session_new["id"].clone(),
                "result": {"sessionId": "kimi-numeric-ids"}
            }),
        )
        .await;

        let prompt = read_wire_request(&mut reader).await;
        assert_eq!(prompt["method"], "session/prompt");
        assert_eq!(
            prompt["id"],
            json!(3),
            "kimi session/prompt id stays numeric 3"
        );
        assert!(
            !prompt["id"].is_string(),
            "kimi outbound ids must never become aria-<seq> strings"
        );
        write_wire_line(
            &mut server_writer,
            &json!({
                "jsonrpc":"2.0", "method":"session/update",
                "params": {"sessionId":"kimi-numeric-ids", "update": {
                    "sessionUpdate":"agent_message_chunk",
                    "content": {"type":"text","text":"kimi numeric ids"}
                }}
            }),
        )
        .await;
        write_wire_line(
            &mut server_writer,
            &json!({
                "jsonrpc":"2.0", "id": prompt["id"].clone(),
                "result": {"stopReason": "end_turn"}
            }),
        )
        .await;
    });

    run_kimi_session(
        peer,
        command_rx,
        event_tx,
        kimi_input(AdapterRole::Orchestrator),
        CancellationToken::new(),
    )
    .await
    .expect("kimi session completes");
    server_task.await.expect("server task");

    loop {
        match tokio::time::timeout(std::time::Duration::from_secs(2), event_rx.recv())
            .await
            .expect("kimi event timeout")
            .expect("kimi event channel open")
        {
            ProviderEvent::Completed(completion) => {
                assert_eq!(completion.full_output, "kimi numeric ids");
                return;
            }
            ProviderEvent::Failed { message } => panic!("kimi session failed: {message}"),
            _ => continue,
        }
    }
}

/// ②kimi 忽略 tool_policy/audit_sink（修复轮 Minor3 收窄命名）：断言范围是
/// provider 不调用传入的 sink——Orchestrator（策略角色档）input 误挂 deny
/// 策略与 sink 时，会话行为零变化、sink 零追加。分区文件级断言已由
/// coding/workspace 侧真实 LifecycleStore 根上的隔离测试覆盖。
#[tokio::test]
async fn kimi_session_ignores_attached_policy_and_never_calls_passed_in_sink() {
    let sink = RecordingToolPolicyAuditSink::new();
    let provider = KimiCodeProvider::new(fixture_command("kimi_acp_text_fixture.sh"));
    let mut input = kimi_input(AdapterRole::Orchestrator);
    input.tool_policy = Some(ProviderToolPolicy::deny_file_write_builtins());
    input.audit_sink = Some(sink.clone().bound());

    let mut session = provider
        .start(input, CancellationToken::new())
        .await
        .expect("kimi starts even with a mistakenly attached policy");

    #[allow(unused_assignments)]
    let mut completed = String::new();
    loop {
        match tokio::time::timeout(std::time::Duration::from_secs(2), session.events.recv())
            .await
            .expect("kimi policy-attached event timeout")
            .expect("kimi policy-attached channel open")
        {
            ProviderEvent::Completed(completion) => {
                completed = completion.full_output;
                break;
            }
            ProviderEvent::Failed { message } => panic!("kimi session failed: {message}"),
            _ => continue,
        }
    }
    assert_eq!(
        completed, "Kimi fixture output",
        "attached policy must not change kimi behavior"
    );
    assert!(
        sink.events().is_empty(),
        "kimi must never append tool-policy canonical events (no provider_start, \
         no approval_decision, no protocol_warning, no session_terminated): {:?}",
        sink.events()
    );
}

// ==== Task 4c:Kimi LC 权限投影与统一 launch audit ====

use crate::cross_cutting::kimi_code_provider::projection::{
    KIMI_GENERIC_TOOL_POLICY_FORBIDDEN, KIMI_LC_APPROVAL_POLICY, KIMI_NATIVE_MCP_SOURCE,
    KimiPolicyProjector,
};
use crate::product::logical_codebase::policy::{
    PolicyTarget, ProviderDialect, ProviderWireDialect, SessionPolicyAction, SessionPolicyEnvelope,
};
use crate::product::logical_codebase::provider_gateway::ProviderRef;
use crate::product::logical_codebase::provider_projection::{
    ProviderPolicyProjector, ProviderProjectionInput,
};

fn lc_projection_envelope(
    action: SessionPolicyAction,
    target_worktree: PathBuf,
    config_artifact_ref: &str,
    config_digest: &str,
) -> SessionPolicyEnvelope {
    let writable_roots = match action {
        SessionPolicyAction::CodingTargetWrite => vec![target_worktree.clone()],
        SessionPolicyAction::PlanningReadOnly | SessionPolicyAction::ReviewReadOnly => Vec::new(),
    };
    SessionPolicyEnvelope {
        policy_id: "policy-lc-0001".to_string(),
        policy_revision: 1,
        policy_digest: "sha256:policy-lc-0001".to_string(),
        action,
        target: PolicyTarget::checkout("logical_repo_0001", "checkout_0001", target_worktree),
        working_directory: PathBuf::from("/lc/lc-root"),
        readable_roots: vec![PathBuf::from("/lc/lc-root")],
        writable_roots,
        provider_dialect: ProviderDialect::KimiAcpV1,
        config_artifact_ref: config_artifact_ref.to_string(),
        config_digest: config_digest.to_string(),
        created_at: "2026-10-03T00:00:00Z".to_string(),
        authority_root: PathBuf::from("/lc/lc-root"),
    }
}

fn lc_kimi_projection_input(
    envelope: SessionPolicyEnvelope,
    role: AdapterRole,
    tool_policy: Option<ProviderToolPolicy>,
    mcp_source: &str,
    trust_digest: &str,
) -> ProviderProjectionInput {
    ProviderProjectionInput::new(
        envelope.clone(),
        ProviderRef::kimi_code("cap_kimi_lc_fixture"),
        envelope.action,
        role,
        ProviderPermissionMode::Auto,
        tool_policy,
        KIMI_LC_APPROVAL_POLICY.to_string(),
        mcp_source.to_string(),
        "sha256:cfg-a".to_string(),
        trust_digest.to_string(),
        None,
    )
}

fn lc_projection_input(
    envelope: SessionPolicyEnvelope,
    role: AdapterRole,
    trust_digest: &str,
) -> ProviderProjectionInput {
    lc_kimi_projection_input(envelope, role, None, KIMI_NATIVE_MCP_SOURCE, trust_digest)
}

/// Task 4c Step 1(断言组 300-301 的 Kimi 形态):client 读面与 target 写面
/// 分离——host root(canonical LC root)保持只读读面,target worktree 是唯一
/// 可写 root;进程 cwd 与 ACP 协议 cwd 都保持 root(terminal 实际 cwd 不改
/// provider 进程 cwd,写面经 boundary plan 由宿主 handler 消费)。
#[test]
fn lcg_t04_kimi_projection_separates_client_read_root_from_target_write_root() {
    let projector = KimiPolicyProjector::new("kimi 0.34.0-lc-fixture");

    let coding = projector
        .project(&lc_projection_input(
            lc_projection_envelope(
                SessionPolicyAction::CodingTargetWrite,
                PathBuf::from("/lc/member-a"),
                "sha256:cfg-a",
                "sha256:cfg-digest-a",
            ),
            AdapterRole::Executor,
            "sha256:trust-1",
        ))
        .expect("kimi lc coding projection is produced");

    // 300/301 形态:target 写允许;root 与其它成员不可写。
    let target_write_allowed = coding.writable_roots() == [PathBuf::from("/lc/member-a")];
    let root_or_other_member_write_allowed = coding.writable_roots().iter().any(|root| {
        root == &PathBuf::from("/lc/lc-root") || root == &PathBuf::from("/lc/member-b")
    });
    assert!(target_write_allowed);
    assert!(!root_or_other_member_write_allowed);

    // 宿主 client 读面:host root 只读可见;进程 cwd 与 ACP 协议 cwd 都冻结
    // 为 canonical root(target 不替代任何 cwd)。
    assert_eq!(coding.readable_roots(), [PathBuf::from("/lc/lc-root")]);
    assert_eq!(
        coding.working_directory(),
        std::path::Path::new("/lc/lc-root")
    );
    assert_eq!(
        coding.protocol_working_directory(),
        std::path::Path::new("/lc/lc-root")
    );

    // 302 形态:通用 tool policy 恒 None;wire/exact version/approval 冻结。
    assert!(coding.tool_policy().is_none());
    assert_eq!(coding.wire_dialect(), ProviderWireDialect::KimiAcp);
    assert_eq!(coding.exact_version(), "kimi 0.34.0-lc-fixture");
    assert_eq!(coding.approval_policy(), KIMI_LC_APPROVAL_POLICY);
    assert_eq!(coding.sandbox(), "target-write-only");
    assert!(!coding.boundary_evidence_ref().is_empty());

    // read-only action:无可写面,boundary 引用为空。
    for action in [
        SessionPolicyAction::PlanningReadOnly,
        SessionPolicyAction::ReviewReadOnly,
    ] {
        let read_only = projector
            .project(&lc_projection_input(
                lc_projection_envelope(
                    action,
                    PathBuf::from("/lc/member-a"),
                    "sha256:cfg-a",
                    "sha256:cfg-digest-a",
                ),
                AdapterRole::Orchestrator,
                "sha256:trust-1",
            ))
            .expect("kimi lc read-only projection is produced");
        assert!(read_only.writable_roots().is_empty(), "{action:?}");
        assert_eq!(read_only.sandbox(), "read-only");
        assert!(read_only.boundary_evidence_ref().is_empty());
        assert_eq!(
            read_only.working_directory(),
            std::path::Path::new("/lc/lc-root")
        );
    }
}

/// Task 4c Step 1(断言组 305 逐字 + 300-301 写面的 Kimi 形态):target/role/
/// config 任一漂移都会改变会话全投影 digest;capability profile 摘要按
/// 「证据摘要分层」不随单次 role/config 变化。tool 维度的 Kimi 形态:通用
/// tool policy 恒 None,非空通用策略直接拒绝(稳定码),不存在 Some↔None
/// 漂移;mcp 来源(native ↔ Aria bundle)漂移两层 digest 都变。
#[test]
fn lcg_t04_projection_digest_changes_on_target_role_tool_or_config() {
    let projector = KimiPolicyProjector::new("kimi 0.34.0-lc-fixture");

    let baseline = projector
        .project(&lc_projection_input(
            lc_projection_envelope(
                SessionPolicyAction::PlanningReadOnly,
                PathBuf::from("/lc/member-a"),
                "sha256:cfg-a",
                "sha256:cfg-digest-a",
            ),
            AdapterRole::Reviewer,
            "sha256:trust-1",
        ))
        .expect("kimi lc planning projection is produced");
    let original_projection_digest = baseline.projection_digest().to_string();
    let original_capability_digest = baseline.capability_projection_digest().to_string();

    // digest 形状:sha256: 前缀 + 64 位小写 hex(与 2c shape validator 同构)。
    assert_eq!(original_projection_digest.len(), 71);
    assert!(original_projection_digest.starts_with("sha256:"));
    assert_eq!(original_capability_digest.len(), 71);
    assert!(original_capability_digest.starts_with("sha256:"));
    assert!(baseline.tool_policy().is_none());

    // 1) target 漂移 → 会话 digest 变化(305 逐字)。
    let changed = projector
        .project(&lc_projection_input(
            lc_projection_envelope(
                SessionPolicyAction::PlanningReadOnly,
                PathBuf::from("/lc/member-b"),
                "sha256:cfg-a",
                "sha256:cfg-digest-a",
            ),
            AdapterRole::Reviewer,
            "sha256:trust-1",
        ))
        .expect("projection with drifted target");
    assert_ne!(original_projection_digest, changed.projection_digest());

    // 2) role 漂移 → 会话 digest 变化;profile 摘要不随单次 role 变化。
    let changed = projector
        .project(&lc_projection_input(
            lc_projection_envelope(
                SessionPolicyAction::PlanningReadOnly,
                PathBuf::from("/lc/member-a"),
                "sha256:cfg-a",
                "sha256:cfg-digest-a",
            ),
            AdapterRole::Orchestrator,
            "sha256:trust-1",
        ))
        .expect("projection with drifted role");
    assert_ne!(original_projection_digest, changed.projection_digest());
    assert_eq!(
        original_capability_digest,
        changed.capability_projection_digest()
    );

    // 3) tool 维度(Kimi 形态):非空通用策略拒绝(稳定码),None 稳定。
    let denied = projector.project(&lc_kimi_projection_input(
        lc_projection_envelope(
            SessionPolicyAction::PlanningReadOnly,
            PathBuf::from("/lc/member-a"),
            "sha256:cfg-a",
            "sha256:cfg-digest-a",
        ),
        AdapterRole::Reviewer,
        Some(ProviderToolPolicy::deny_file_write_builtins()),
        KIMI_NATIVE_MCP_SOURCE,
        "sha256:trust-1",
    ));
    let denial = denied.expect_err("kimi projector must reject a non-empty generic tool policy");
    assert!(
        denial
            .to_string()
            .contains(KIMI_GENERIC_TOOL_POLICY_FORBIDDEN),
        "unexpected denial: {denial}"
    );

    // 4) config 漂移 → 会话 digest 变化;profile 摘要不含 config,保持不变。
    let changed = projector
        .project(&lc_projection_input(
            lc_projection_envelope(
                SessionPolicyAction::PlanningReadOnly,
                PathBuf::from("/lc/member-a"),
                "sha256:cfg-b",
                "sha256:cfg-digest-b",
            ),
            AdapterRole::Reviewer,
            "sha256:trust-1",
        ))
        .expect("projection with drifted config");
    assert_ne!(original_projection_digest, changed.projection_digest());
    assert_eq!(
        original_capability_digest,
        changed.capability_projection_digest()
    );

    // 5) mcp 来源漂移(native ↔ Aria bundle digest)→ 会话与 profile 两层
    //    digest 都变(MCP 控制规范属于 profile)。
    let changed = projector
        .project(&lc_kimi_projection_input(
            lc_projection_envelope(
                SessionPolicyAction::PlanningReadOnly,
                PathBuf::from("/lc/member-a"),
                "sha256:cfg-a",
                "sha256:cfg-digest-a",
            ),
            AdapterRole::Reviewer,
            None,
            "sha256:aria-mcp-bundle-1",
            "sha256:trust-1",
        ))
        .expect("projection with drifted mcp source");
    assert_ne!(original_projection_digest, changed.projection_digest());
    assert_ne!(
        original_capability_digest,
        changed.capability_projection_digest()
    );

    // Coding 投影(断言组 300-301):恰一个可写 root=target worktree;
    // boundary 引用非空;profile 摘要随 boundary 模式整体变化。
    let coding = projector
        .project(&lc_projection_input(
            lc_projection_envelope(
                SessionPolicyAction::CodingTargetWrite,
                PathBuf::from("/lc/member-a"),
                "sha256:cfg-a",
                "sha256:cfg-digest-a",
            ),
            AdapterRole::Executor,
            "sha256:trust-1",
        ))
        .expect("kimi lc coding projection is produced");
    assert_eq!(coding.writable_roots(), [PathBuf::from("/lc/member-a")]);
    assert!(!coding.boundary_evidence_ref().is_empty());
    assert_eq!(coding.sandbox(), "target-write-only");
    assert_ne!(
        original_capability_digest,
        coding.capability_projection_digest()
    );
}

// ==== Task 4c:LC validated launch 测试 fixture ====
use std::path::PathBuf;
// 经真实 `LogicalCodebaseProviderGateway::validate` 链产出
// `ValidatedStreamingProviderInput`(envelope 由 bootstrap 政策冻结),
// 供 `start_validated` 的 LC 审计/native 会话测试消费(与 4a/4b 的
// LcLaunchFixture 同构,Kimi 特有:capability dialect=KimiAcpV1)。

use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::cross_cutting::session_launch::ValidatedStreamingProviderInput;
use crate::cross_cutting::tool_policy_audit::{DurableToolPolicyEvent, ToolPolicyAuditSink};
use crate::product::app_paths::ProductAppPaths;
use crate::product::logical_codebase::policy::AggregatePolicyArtifactStore;
use crate::product::logical_codebase::provider_capability_store::ProviderActionCapability;
use crate::product::logical_codebase::provider_gateway::{
    GatewayRunAudit, LogicalCodebaseProviderGateway, PolicyTargetResolver, ProviderCapability,
    ProviderCapabilitySource, ProviderGatewayError, ProviderRefType, SessionLaunchRequest,
};
use crate::product::logical_codebase::store::LogicalCodebaseManifest;
use crate::product::models::ProviderName;
use crate::protocol::contracts::{AdapterOutput, TimeoutStatus};

struct LcKimiLaunchFixture {
    _root: tempfile::TempDir,
    paths: ProductAppPaths,
    audit: std::sync::Arc<GatewayRunAudit>,
}

impl LcKimiLaunchFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("lc fixture root");
        let paths = ProductAppPaths::new(root.path());
        let manifest =
            LogicalCodebaseManifest::new("project_0001", root.path().to_path_buf(), vec![]);
        AggregatePolicyArtifactStore::new(paths.clone())
            .ensure_bootstrap(&manifest)
            .expect("install lc bootstrap policy");
        let fixture = Self {
            _root: root,
            paths,
            audit: std::sync::Arc::new(GatewayRunAudit::new()),
        };
        fixture.target_worktree();
        fixture
    }

    /// canonical LC root(= manifest provider_context_root)。
    fn canonical_root(&self) -> PathBuf {
        std::fs::canonicalize(self.paths.root()).expect("lc fixture root exists")
    }

    /// 唯一可写 target(成员 worktree;与 canonical root 分离)。
    fn target_worktree(&self) -> PathBuf {
        let worktree = self.paths.root().join("member-worktree");
        std::fs::create_dir_all(&worktree).expect("create member worktree");
        worktree
    }

    /// 以注入 registry 组装 gateway(Task 7:注册真实 Kimi adapter 观测
    /// gateway 分流;默认仍用占位 adapter)。
    fn gateway_with_registry(
        &self,
        registry: crate::cross_cutting::provider_registry::ProviderRegistry,
    ) -> LogicalCodebaseProviderGateway {
        LogicalCodebaseProviderGateway::with_audit(
            AggregatePolicyArtifactStore::new(self.paths.clone()),
            std::sync::Arc::new(LcStaticCapabilitySource),
            std::sync::Arc::new(LcTargetResolver),
            std::sync::Arc::new(registry),
            std::sync::Arc::new(LcNoopSyncAdapter),
            lc_available_gate(),
            self.audit.clone(),
            self.canonical_root(),
        )
    }

    fn gateway(&self) -> LogicalCodebaseProviderGateway {
        let mut registry = ProviderRegistry::new();
        registry.register(
            ProviderName::KimiCode,
            std::sync::Arc::new(LcNoopStreamingAdapter),
        );
        self.gateway_with_registry(registry)
    }

    /// LC coding 请求:cwd=canonical root,target=成员 worktree,恰一个
    /// 可写 root=target。
    fn coding_request(&self) -> SessionLaunchRequest {
        let manifest =
            LogicalCodebaseManifest::new("project_0001", self.paths.root().to_path_buf(), vec![]);
        let worktree = self.target_worktree();
        SessionLaunchRequest {
            project_id: manifest.project_id,
            provider: ProviderRef::kimi_code("cap_kimi_lc_fixture"),
            action: SessionPolicyAction::CodingTargetWrite,
            target: PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree.clone()),
            working_directory: self.canonical_root(),
            readable_roots: vec![self.canonical_root()],
            writable_roots: vec![worktree],
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        }
    }

    /// LC streaming input(Kimi 通用 tool policy 恒 None;无策略角色档)。
    fn lc_streaming_input(
        &self,
        role: AdapterRole,
        tool_policy: Option<ProviderToolPolicy>,
        audit_sink: Option<std::sync::Arc<dyn ToolPolicyAuditSink>>,
        resume_id: Option<String>,
    ) -> StreamingProviderInput {
        StreamingProviderInput {
            working_directory: Some(self.canonical_root()),
            baseline_tree: None,
            tool_policy,
            audit_sink,
            provider_type: ProviderType::KimiCode,
            role,
            prompt: "Run the LC fixture provider".to_string(),
            working_dir: self.target_worktree(),
            workspace_session_id: Some("ws-lc-fixture-1".to_string()),
            resume_provider_session_id: resume_id,
            permission_mode: ProviderPermissionMode::Auto,
            structured_output_contract: None,
            env_vars: BTreeMap::new(),
            timeout_secs: 60,
        }
    }

    /// 经真实 gateway validate 产出 coding validated input。
    fn validated_coding_input(
        &self,
        raw: StreamingProviderInput,
    ) -> ValidatedStreamingProviderInput {
        let validated = self
            .gateway()
            .validate(self.coding_request())
            .expect("lc coding launch validates");
        ValidatedStreamingProviderInput::new(raw, validated)
    }
}

/// LC fixture 的 capability 源:仅返回 kimi 的已实测快照(三格 Confirmed 的
/// fixture 事实;真实 probe/evidence 归 6c/2d)。
struct LcStaticCapabilitySource;

fn lc_kimi_capability() -> ProviderCapability {
    ProviderCapability {
        provider_type: ProviderRefType::KimiCode,
        version: "kimi 0.34.0-lc-fixture".to_string(),
        adapter_dialect: ProviderDialect::KimiAcpV1,
        wire_dialect: ProviderWireDialect::KimiAcp,
        capability_snapshot_ref: "cap_kimi_lc_fixture".to_string(),
        action_capability: ProviderActionCapability {
            action: SessionPolicyAction::CodingTargetWrite,
            launch:
                crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed,
            resume:
                crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed,
            write_boundary:
                crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed,
            projection_digest: String::new(),
            evidence_ref: String::new(),
        },
        trust: crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed,
    }
}

impl ProviderCapabilitySource for LcStaticCapabilitySource {
    fn require_supported(
        &self,
        _provider: &ProviderRef,
        _action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(lc_kimi_capability())
    }

    fn require_resume_supported(
        &self,
        _provider: &ProviderRef,
        _action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(lc_kimi_capability())
    }

    fn require_write_boundary(
        &self,
        _provider: &ProviderRef,
        _action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(lc_kimi_capability())
    }

    fn require_root_recipe_supported(
        &self,
        _provider: &ProviderRef,
        _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Err(ProviderGatewayError::UnsupportedCapability(
            "lc fixture has no root recipe facts".to_string(),
        ))
    }
}

/// LC fixture 的 target resolver:直接返回请求冻结的 target。
struct LcTargetResolver;

impl PolicyTargetResolver for LcTargetResolver {
    fn resolve_and_revalidate(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError> {
        Ok(request.target.clone())
    }
}

/// 占位 streaming adapter:validate 不触 registry,仅为 gateway 构造提供槽位。
struct LcNoopStreamingAdapter;

#[async_trait::async_trait]
impl crate::cross_cutting::streaming_provider::StreamingProviderAdapter for LcNoopStreamingAdapter {}

/// 占位 sync adapter:gateway 构造参数,LC validated 测试不调用。
struct LcNoopSyncAdapter;

impl crate::cross_cutting::provider_adapter::ProviderAdapter for LcNoopSyncAdapter {
    fn run(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
    ) -> Result<AdapterOutput, crate::cross_cutting::provider_adapter::ProviderAdapterError> {
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

fn lc_available_gate() -> std::sync::Arc<ProviderAvailabilityGate> {
    use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
    use chrono::Utc;

    let checked_at = Utc::now();
    let snapshot = std::sync::Arc::new(ProviderHealthSnapshot {
        schema_version: 1,
        generation: 1,
        checked_at,
        providers: [ProviderName::KimiCode]
            .into_iter()
            .map(|provider| ProviderHealthEntry {
                provider,
                command: "stub".to_string(),
                available: true,
                version: Some("0.34.0".to_string()),
                reason_code: None,
                reason: None,
                checked_at,
            })
            .collect(),
    });
    std::sync::Arc::new(ProviderAvailabilityGate::new(std::sync::Arc::new(
        AlwaysHealthy(snapshot),
    )))
}

struct AlwaysHealthy(std::sync::Arc<crate::cross_cutting::provider_health::ProviderHealthSnapshot>);

impl crate::cross_cutting::provider_availability_gate::ProviderHealthSource for AlwaysHealthy {
    fn snapshot(
        &self,
    ) -> std::sync::Arc<crate::cross_cutting::provider_health::ProviderHealthSnapshot> {
        self.0.clone()
    }
    fn degraded(&self) -> bool {
        false
    }
}

/// LC cwd fixture(fake kimi ACP):`--version` 打印兼容版本;ACP 循环内
/// initialize/session/new/session/prompt 正常应答,session/new 时把进程
/// `pwd -P` 与请求 `cwd` 参数分别写入 marker(进程 cwd 与 ACP 协议 cwd
/// 双观测),session/prompt 后 end_turn 退出。
#[cfg(unix)]
fn write_kimi_lc_fixture(
    dir: &std::path::Path,
    marker: &std::path::Path,
    acp_marker: &std::path::Path,
) -> PathBuf {
    write_executable(
        dir,
        "fake-kimi-lc-acp",
        &format!(
            "#!/usr/bin/env bash\nset -uo pipefail\nif [[ \"${{1:-}}\" == \"--version\" ]]; then\n  echo \"kimi 0.34.0\"\n  exit 0\nfi\nwhile IFS= read -r line; do\n  id=\"$(printf '%s' \"$line\" | sed -n 's/.*\"id\"[[:space:]]*:[[:space:]]*\\([0-9][0-9]*\\).*/\\1/p')\"\n  if [[ \"$line\" == *'\"initialize\"'* ]]; then\n    echo \"{{\\\"jsonrpc\\\":\\\"2.0\\\",\\\"id\\\":${{id:-1}},\\\"result\\\":{{\\\"protocolVersion\\\":1,\\\"agentCapabilities\\\":{{\\\"loadSession\\\":true,\\\"sessionCapabilities\\\":{{\\\"resume\\\":{{}}}}}}}}}}\"\n  elif [[ \"$line\" == *'\"session/new\"'* ]]; then\n    pwd -P > {marker}\n    printf '%s' \"$line\" | sed -n 's/.*\"cwd\"[[:space:]]*:[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p' > {acp_marker}\n    echo \"{{\\\"jsonrpc\\\":\\\"2.0\\\",\\\"id\\\":${{id:-2}},\\\"result\\\":{{\\\"sessionId\\\":\\\"kimi_lc_native_fixture\\\"}}}}\"\n  elif [[ \"$line\" == *'\"session/prompt\"'* ]]; then\n    echo \"{{\\\"jsonrpc\\\":\\\"2.0\\\",\\\"id\\\":${{id:-3}},\\\"result\\\":{{\\\"stopReason\\\":\\\"end_turn\\\"}}}}\"\n    exit 0\n  fi\ndone\n",
            marker = marker.display(),
            acp_marker = acp_marker.display(),
        ),
    )
}

/// 版本探测失败的 fake kimi(`--version` 打印不兼容的 0.0.0):用于证明
/// 非空通用策略的拒绝先于 version 兼容门(若拒绝在后,错误会是
/// incompatible 而非策略稳定码)。
#[cfg(unix)]
fn write_kimi_lc_version_broken_fixture(dir: &std::path::Path) -> PathBuf {
    write_executable(
        dir,
        "fake-kimi-lc-version-broken",
        "#!/usr/bin/env bash\nif [[ \"${1:-}\" == \"--version\" ]]; then\n  echo \"kimi 0.0.0\"\n  exit 0\nfi\nwhile IFS= read -r line; do :; done\n",
    )
}

#[cfg(unix)]
fn write_executable(dir: &std::path::Path, name: &str, script: &str) -> PathBuf {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt as _;
    let path = dir.join(name);
    let mut file = std::fs::File::create(&path).expect("create fixture script");
    file.write_all(script.as_bytes())
        .expect("write fixture script");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod fixture script");
    path
}

/// 有界等待 marker 落盘(2s 超时即 panic)。
#[cfg(unix)]
fn wait_for_marker(marker: &std::path::Path) -> String {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if let Ok(content) = std::fs::read_to_string(marker) {
            return content;
        }
        if std::time::Instant::now() >= deadline {
            panic!("marker {marker:?} was never written by the fixture child");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// Task 4c Step 1(断言组 302-304 逐字):无通用 tool policy 的 LC Coding
/// 启动(Executor)同样执行 exact version probe(真实 `--version`)、
/// native session/handshake(ACP `session/new`)与统一
/// `ProviderStartAudit.lc_projection` 落盘——不以 `tool_policy=None` 早退;
/// 进程 cwd 与 ACP 协议 cwd 都冻结为 canonical LC root(299)。
#[cfg(unix)]
#[tokio::test]
async fn lcg_t04_no_generic_tool_policy_still_records_version_audit_and_native_session() {
    let fixture = LcKimiLaunchFixture::new();
    let sink = RecordingToolPolicyAuditSink::new();
    let marker = fixture.paths.root().join("lc-cwd-marker");
    let acp_marker = fixture.paths.root().join("lc-acp-cwd-marker");
    let mut raw = fixture.lc_streaming_input(
        AdapterRole::Executor,
        None,
        Some(sink.clone().bound()),
        None,
    );
    raw.env_vars
        .insert("LC_CWD_MARKER".to_string(), marker.display().to_string());
    raw.env_vars.insert(
        "LC_ACP_CWD_MARKER".to_string(),
        acp_marker.display().to_string(),
    );
    // 302 逐字观测源:LC 会话输入不携带通用 tool policy。
    let kimi_generic_tool_policy = raw.tool_policy.clone();

    let provider = KimiCodeProvider::new(write_kimi_lc_fixture(
        fixture.paths.root(),
        &marker,
        &acp_marker,
    ));
    let session = provider
        .start_validated(
            fixture.validated_coding_input(raw),
            CancellationToken::new(),
        )
        .await
        .expect("lc validated start succeeds without generic tool policy");

    // 304 逐字:native session id 来自 ACP session/new 握手。
    let kimi_native_session_id = session
        .native_session_id
        .clone()
        .expect("kimi lc validated session carries the handshake native session id");
    let kimi_handshake_session_id = "kimi_lc_native_fixture";
    assert_eq!(kimi_native_session_id, kimi_handshake_session_id);

    let events = sink.events();
    assert_eq!(events.len(), 1, "exactly one provider_start is written");
    let DurableToolPolicyEvent::ProviderStart(record) = &events[0] else {
        panic!("expected provider_start");
    };
    assert_eq!(record.provider, "kimi-code");
    assert_eq!(record.provider_version, "kimi 0.34.0");
    assert_eq!(record.adapter_dialect, "kimi-acp");
    assert_eq!(record.role, "executor");
    assert_eq!(record.workspace_session_id, "ws-lc-fixture-1");
    assert_eq!(record.provider_session_id, kimi_native_session_id);
    // argv 冻结:LC 与 direct 同源,仅 `acp`,无任何策略物理片段。
    assert_eq!(record.argv, vec!["acp".to_string()]);
    // 无通用 tool policy 也有 canonical digest(空 token 序列的 canonical 形态)。
    assert!(!record.tool_policy_canonical_digest.is_empty());

    // 统一 lc_projection 落盘:分层双 digest + wire/action/boundary 引用。
    let lc_projection = record
        .lc_projection
        .as_ref()
        .expect("lc projection audit is recorded even without generic tool policy");
    assert_eq!(lc_projection.action, "coding_target_write");
    assert_eq!(lc_projection.wire_dialect, "kimi-acp");
    assert!(lc_projection.projection_digest.starts_with("sha256:"));
    assert!(
        lc_projection
            .capability_projection_digest
            .starts_with("sha256:")
    );
    assert!(!lc_projection.boundary_evidence_ref.is_empty());

    // 302-303 逐字断言组:通用 tool policy 为 None;version probe(真实
    // `--version` 探测)与 provider_start audit 双双落盘,不以 None 早退。
    assert!(kimi_generic_tool_policy.is_none());
    let kimi_version_probe_recorded = record.provider_version == "kimi 0.34.0";
    let kimi_provider_start_audit_recorded = record.lc_projection.is_some() && events.len() == 1;
    assert!(kimi_version_probe_recorded && kimi_provider_start_audit_recorded);

    // 299:provider 进程 cwd = canonical LC root(不是 target worktree);
    // ACP 协议 cwd 同样保持 root(target 写面经 boundary plan 由宿主
    // handler 消费,不进协议 cwd)。
    let provider_cwd = wait_for_marker(&marker).trim().to_string();
    assert_eq!(
        provider_cwd,
        fixture.canonical_root().to_string_lossy(),
        "provider process must spawn at the canonical LC root"
    );
    let acp_cwd = wait_for_marker(&acp_marker).trim().to_string();
    assert_eq!(
        acp_cwd,
        fixture.canonical_root().to_string_lossy(),
        "acp protocol cwd must stay at the canonical LC root"
    );

    // 非空通用策略在 version/child 之前拒绝:版本探测故意不兼容(0.0.0)
    // 时错误仍是策略稳定码——证明策略拒绝先于 version 兼容门;零 audit。
    let denied_sink = RecordingToolPolicyAuditSink::new();
    let denied_raw = fixture.lc_streaming_input(
        AdapterRole::Orchestrator,
        Some(ProviderToolPolicy::deny_file_write_builtins()),
        Some(denied_sink.clone().bound()),
        None,
    );
    let denied_provider =
        KimiCodeProvider::new(write_kimi_lc_version_broken_fixture(fixture.paths.root()));
    let error = match denied_provider
        .start_validated(
            fixture.validated_coding_input(denied_raw),
            CancellationToken::new(),
        )
        .await
    {
        Ok(_session) => {
            panic!("non-empty generic tool policy must be rejected before version/child")
        }
        Err(error) => error,
    };
    assert!(
        error.details.contains(KIMI_GENERIC_TOOL_POLICY_FORBIDDEN),
        "unexpected rejection: {}",
        error.details
    );
    assert!(
        denied_sink.events().is_empty(),
        "rejected launch must not write any provider_start audit"
    );
}

/// Kimi 拒绝探针 CLI(Task 7):`--version` 记 `version` 并打印不兼容版本
/// 0.0.0(若拒绝晚于版本门,错误会变成 incompatible 而非策略稳定码);
/// ACP 会话执行记 `child`,读到首行 stdin 记 `handshake` 后退出。
#[cfg(unix)]
fn write_kimi_t07_reject_probe(dir: &std::path::Path, marker: &std::path::Path) -> PathBuf {
    write_executable(
        dir,
        "fake-kimi-t07-reject-probe",
        &format!(
            "#!/usr/bin/env bash\nset -uo pipefail\nif [[ \"${{1:-}}\" == \"--version\" ]]; then\n  echo version >> {marker}\n  echo \"kimi 0.0.0\"\n  exit 0\nfi\necho child >> {marker}\nif IFS= read -r _line; then\n  echo handshake >> {marker}\nfi\nexit 0\n",
            marker = marker.display(),
        ),
    )
}

/// Task 7 Step 1(断言组 382 逐字):经 gateway `start_streaming` 启动的 LC
/// 会话携带非空通用策略时,Kimi 以稳定码
/// `provider_generic_tool_policy_forbidden` 在版本探测与 session child 之前
/// 拒绝。BASE 的 gateway prepared/raw 过渡分叉会把未经 prepare 的 validated
/// 构造分派到裸 `start`——Kimi 直连不读通用策略、直接 spawn child,稳定码
/// 拒绝消失(本测试红);Task 7 收口 validated-only 后由 `start_validated`
/// 首步拒绝(绿),marker 全零证明拒绝先于 version/child/handshake。
#[cfg(unix)]
#[tokio::test]
async fn lcg_t07_kimi_generic_policy_rejected_before_version_child() {
    let fixture = LcKimiLaunchFixture::new();
    let marker = fixture.paths.root().join("t07-kimi-reject-marker");
    let provider =
        KimiCodeProvider::new(write_kimi_t07_reject_probe(fixture.paths.root(), &marker));
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::KimiCode, std::sync::Arc::new(provider));
    let gateway = fixture.gateway_with_registry(registry);

    let raw = fixture.lc_streaming_input(
        AdapterRole::Orchestrator,
        Some(ProviderToolPolicy::deny_file_write_builtins()),
        Some(RecordingToolPolicyAuditSink::new().bound()),
        None,
    );
    // 未 prepare 的存量 validated 构造:分叉收口前走裸 start(红点)。
    let validated = {
        let policy = gateway
            .validate(fixture.coding_request())
            .expect("lc coding launch validates");
        ValidatedStreamingProviderInput::new(raw, policy)
    };

    let kimi_reason = match gateway
        .start_streaming(validated, CancellationToken::new())
        .await
    {
        Ok(_session) => String::new(),
        Err(error) => {
            let text = error.to_string();
            if text.contains(KIMI_GENERIC_TOOL_POLICY_FORBIDDEN) {
                KIMI_GENERIC_TOOL_POLICY_FORBIDDEN.to_string()
            } else {
                text
            }
        }
    };

    // 真实执行面计数:拒绝必须发生在任何 version 探测/child/握手之前。
    let content = std::fs::read_to_string(&marker).unwrap_or_default();
    let kimi_version_probe_count = content.matches("version").count();
    let kimi_child_spawn_count = content.matches("child").count();
    let kimi_handshake_count = content.matches("handshake").count();
    assert_eq!(kimi_version_probe_count, 0);
    assert_eq!(kimi_child_spawn_count, 0);
    assert_eq!(kimi_handshake_count, 0);
    assert_eq!(kimi_reason, "provider_generic_tool_policy_forbidden");
}
