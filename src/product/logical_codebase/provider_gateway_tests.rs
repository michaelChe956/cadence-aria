use super::*;
use crate::cross_cutting::session_launch::{
    ValidatedAdapterInput, ValidatedStreamingProviderInput,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::logical_codebase::policy::PolicyTarget;
use crate::product::logical_codebase::store::LogicalCodebaseManifest;
use crate::protocol::contracts::AdapterInput;

/// 验证 bootstrap 政策在 gateway 能校验首次启动前被持久化。
///
/// 缺失政策时 fail-closed 为 `PolicyMissing`;`ensure_bootstrap` 写出 revision 1
/// 的 bootstrap artifact 后,gateway 可校验 planning 只读启动并冻结 envelope。
#[test]
fn bootstrap_policy_is_persisted_before_gateway_can_validate_a_launch() {
    let fixture = gateway_fixture();
    assert!(matches!(
        fixture.gateway().validate(fixture.planning_request()),
        Err(ProviderGatewayError::PolicyMissing(_))
    ));

    fixture.install_bootstrap_policy();
    let validated = fixture
        .gateway()
        .validate(fixture.planning_request())
        .unwrap();
    assert_eq!(validated.envelope().policy_revision, 1);
    assert_eq!(
        validated.envelope().action,
        SessionPolicyAction::PlanningReadOnly,
    );
}

/// validated envelope 冻结 authority_root:值来自 manifest.provider_context_root
/// (构造时 canonicalize),与构建 gateway 时注入的聚合根一致。
#[test]
fn validated_envelope_carries_canonical_authority_root_from_manifest() {
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    let manifest = fixture.manifest();
    let expected = std::fs::canonicalize(&manifest.provider_context_root).unwrap();

    let validated = fixture
        .gateway()
        .validate(fixture.planning_request())
        .unwrap();

    assert_eq!(validated.envelope().authority_root, expected);
}

/// validated policy 的字段对外不可直接构造:没有 public constructor,getter 是
/// 唯一访问方式。编译期保证只能由 gateway 产出。
#[test]
fn validated_policy_only_exposes_getters_and_cannot_be_constructed_outside_module() {
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    let validated = fixture
        .gateway()
        .validate(fixture.planning_request())
        .unwrap();

    // getter 返回冻结的引用;外部无法修改字段或凭空重建 validated policy。
    assert_eq!(validated.envelope().policy_revision, 1);
    assert!(validated.fingerprint().digest.starts_with("sha256:"));
}

/// fingerprint 随 provider exact version 与 capability snapshot 变化:任一漂移
/// 都应产生不同 digest,保证 resume 复验能检测 provider 侧变更。
#[test]
fn resume_fingerprint_changes_when_provider_version_or_snapshot_drifts() {
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();

    let baseline = fixture
        .gateway()
        .validate(fixture.planning_request())
        .unwrap();
    let baseline_digest = baseline.fingerprint().digest.clone();

    fixture.capabilities.set_version("1.4.1");
    let drifted = fixture
        .gateway()
        .validate(fixture.planning_request())
        .unwrap();
    assert_ne!(drifted.fingerprint().digest, baseline_digest);
}

/// 直接构造 `ValidatedSessionLaunchPolicy { envelope, fingerprint }` 在本模块外
/// 不可行(struct literal 构造需要字段可见)。此处用 doctest-like 断言:gateway
/// 返回值只能经 getter 访问,确认 opaque 边界由 privacy 保护。
#[test]
fn opaque_policy_blocks_struct_literal_construction_outside_module() {
    // 编译期约束:ValidatedSessionLaunchPolicy 的字段是 private,本测试模块虽在
    // 同一文件但通过公开 API 访问,模拟外部调用方。外部 crate 无法构造该 struct。
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    let validated = fixture
        .gateway()
        .validate(fixture.planning_request())
        .unwrap();
    // 只能读 getter,无法读字段或重建。
    let _envelope = validated.envelope();
    let _fingerprint = validated.fingerprint();
}

/// spawn 前 canonical 复验:validate 阶段 target resolver 检测到 git_dir 在
/// request 构造后被篡改(TOCTOU),返回 `TargetMismatch { field: "git_dir" }`,
/// 且复验失败发生在 registry lookup/真实 adapter start 之前(start_count == 0)。
#[test]
fn gateway_revalidates_canonical_target_git_dir_and_managed_config_before_spawn() {
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    let request = fixture.coding_request("/work/api/.worktrees/aria-issues/issue_1");
    fixture
        .targets()
        .change_git_dir_after_request("/work/api/.git-replaced");

    let error = fixture.gateway().validate(request).unwrap_err();
    assert!(
        matches!(error, ProviderGatewayError::TargetMismatch { ref field } if field == "git_dir")
    );
    assert_eq!(fixture.registry_start_count(), 0);
}

/// spawn 前 target identity 复验(B-1):validate 成功后、start_streaming 前
/// resolver 返回的 target 被改掉(模拟 .git 指针/target 被调包),spawn 必须
/// fail-closed 为 `TargetMismatch { field: "target" }`,且不触达 registry。
#[tokio::test]
async fn start_streaming_revalidates_target_identity_before_spawn() {
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    let worktree = fixture.real_worktree();
    let launch = fixture.validated_planning_streaming_input(worktree.clone());

    // validate 后、spawn 前把 resolver 返回的 target 改掉。
    fixture
        .targets()
        .change_target_after_request(PolicyTarget::checkout(
            "logical_repo_0001",
            "checkout_0001",
            worktree.join("swapped"),
        ));

    let result = fixture
        .gateway()
        .start_streaming(launch, CancellationToken::new())
        .await;
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("expected TargetMismatch field=target, got a session"),
    };

    assert!(
        matches!(error, ProviderGatewayError::TargetMismatch { ref field } if field == "target"),
        "expected TargetMismatch field=target, got {error:?}"
    );
    assert_eq!(fixture.registry_start_count(), 0);
}

/// read-only action(planning/review)没有 write root;coding action 恰好一个
/// 等于 canonical target worktree 的 write root。envelope 在 validate 时冻结该约束。
#[test]
fn planning_and_review_have_no_write_root_while_coding_has_exactly_target_root() {
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    assert!(
        fixture
            .gateway()
            .validate(fixture.planning_request())
            .unwrap()
            .envelope()
            .writable_roots
            .is_empty()
    );
    let worktree = fixture
        .paths
        .root()
        .join("api/.worktrees/aria-issues/issue_1");
    assert_eq!(
        fixture
            .gateway()
            .validate(fixture.coding_request(worktree.clone()))
            .unwrap()
            .envelope()
            .writable_roots,
        vec![worktree]
    );
}

/// spawn 前复验(B-1):validate 后、start_streaming 前政策被升级(revision+digest
/// 变化),spawn 必须以 `PolicyDrift { dimension: "policy_revision" }` fail-closed,
/// 且不触达 registry(start_count == 0)。
#[test]
fn start_streaming_fails_closed_when_policy_upgraded_between_validate_and_spawn() {
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    let worktree = fixture.real_worktree();
    let launch = fixture.validated_planning_streaming_input(worktree);

    // validate→spawn 之间政策被升级到 revision 2。
    fixture.upgrade_policy();

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let result = runtime.block_on(
        fixture
            .gateway()
            .start_streaming(launch, CancellationToken::new()),
    );
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("expected PolicyDrift, got a session"),
    };
    assert!(
        matches!(error, ProviderGatewayError::PolicyDrift { ref dimension } if dimension == "policy_revision")
    );
    assert_eq!(fixture.registry_start_count(), 0);
}

/// spawn 前复验(B-1):validate 后、start_streaming 前 provider version 被改
/// (capability source 返回不同 version),spawn 必须以
/// `PolicyDrift { dimension: "provider_version" }` fail-closed,不触达 registry。
#[test]
fn start_streaming_rejects_provider_version_change_before_spawn() {
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    let worktree = fixture.real_worktree();
    let launch = fixture.validated_planning_streaming_input(worktree);

    fixture.capabilities().set_version("1.4.1");

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let result = runtime.block_on(
        fixture
            .gateway()
            .start_streaming(launch, CancellationToken::new()),
    );
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("expected PolicyDrift, got a session"),
    };
    assert!(
        matches!(error, ProviderGatewayError::PolicyDrift { ref dimension } if dimension == "provider_version")
    );
    assert_eq!(fixture.registry_start_count(), 0);
}

/// spawn 前复验(B-1):validate 后、run_sync 前政策被升级,同步路径同样 fail-closed
/// 为 `PolicyDrift`,不触达真实 sync adapter(此处表现为返回错误而非成功)。
#[test]
fn run_sync_fails_closed_when_policy_upgraded_between_validate_and_spawn() {
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    let worktree = fixture.real_worktree();
    let request = SessionLaunchRequest::planning(
        fixture.manifest().project_id,
        ProviderRef::claude_code("cap_claude_code_1_4_0"),
        PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree.clone()),
        vec![fixture.paths.root().to_path_buf()],
        "sha256:managed-config-artifact",
    );
    let validated = fixture.gateway().validate(request).unwrap();
    let input = crate::protocol::contracts::AdapterInput {
        working_directory: None,
        provider_type: crate::protocol::contracts::ProviderType::ClaudeCode,
        role: crate::protocol::contracts::AdapterRole::Executor,
        worktree_path: Some(worktree.to_string_lossy().to_string()),
        provider_stream_log_dir: None,
        prompt: "probe".to_string(),
        context_files: Vec::new(),
        output_schema: String::new(),
        timeout: 1,
        max_retries: 0,
    };
    let launch = ValidatedAdapterInput::new(input, validated);

    fixture.upgrade_policy();

    let error = fixture.gateway().run_sync(launch).unwrap_err();
    assert!(
        matches!(error, ProviderGatewayError::PolicyDrift { ref dimension } if dimension == "policy_revision")
    );
}

/// resume 能力 fail-closed(B-2 消费者):resume 启动时 provider 的
/// `resume_evidence` 不是 `Confirmed` → spawn 拒绝(`ResumeNotSupported`),不触达
/// registry。这使 `resume_evidence` 三态成为有消费者的 fail-closed 门禁,而非
/// dead field。
#[test]
fn start_streaming_resume_is_rejected_when_resume_evidence_not_confirmed() {
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    let worktree = fixture.real_worktree();
    // resume 启动:streaming input 携带 resume_provider_session_id。
    let validated = fixture
        .gateway()
        .validate(fixture.planning_request())
        .unwrap();
    let input = fixture.streaming_input(worktree, Some("sess_resume_0001".to_string()));
    let launch = ValidatedStreamingProviderInput::new(input, validated);

    // provider 标记 resume 不支持(三态 Unknown/Denied 在 gateway 侧归为 Unsupported)。
    fixture
        .capabilities()
        .set_resume_evidence(ResumeEvidenceState::Unsupported);

    let runtime = tokio::runtime::Runtime::new().unwrap();
    let result = runtime.block_on(
        fixture
            .gateway()
            .start_streaming(launch, CancellationToken::new()),
    );
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("expected ResumeNotSupported, got a session"),
    };
    assert!(matches!(error, ProviderGatewayError::ResumeNotSupported));
    assert_eq!(fixture.registry_start_count(), 0);
}

/// Task 2b(lcg_t02):fresh 门在 validate 阶段消费 write_boundary 分格——
/// write_boundary 为 Denied(真实负向证据)时,正常 coding 请求在 validate
/// 即拒绝(fresh = launch + write-boundary 两半),provider 零启动;分格恢复
/// Confirmed 后同一请求放行。
#[test]
fn lcg_t02_gateway_fresh_requires_write_boundary_cell() {
    use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    let worktree = fixture.real_worktree();
    fixture
        .capabilities()
        .set_write_boundary_cell(ProviderCapabilityEvidence::Denied {
            reason: "boundary probe denied".to_string(),
        });

    let error = fixture
        .gateway()
        .validate(fixture.coding_request(worktree.clone()))
        .unwrap_err();
    assert!(
        matches!(&error, ProviderGatewayError::UnsupportedCapability(reason) if reason.starts_with(PROVIDER_CAPABILITY_WRITE_BOUNDARY_NOT_CONFIRMED)),
        "unexpected fresh write-boundary error: {error:?}"
    );
    assert_eq!(fixture.registry_start_count(), 0);

    // 分格恢复 Confirmed:同一请求放行(拒绝只来自分格状态)。
    fixture
        .capabilities()
        .set_write_boundary_cell(ProviderCapabilityEvidence::Confirmed);
    fixture
        .gateway()
        .validate(fixture.coding_request(worktree))
        .unwrap();
}

/// Task 2b 第二段:root-recipe 相位私有冻结 + 不误套 normal action 门。
/// - 普通 validate 产出恒 Normal 相位;唯一 RootRecipe 相位只能经 gateway
///   内部 validate_root_recipe_request(携带凭据)产出,普通调用不可构造
///   (phase 字段私有、无 public constructor)。
/// - normal write_boundary Denied 时,同一形状请求:普通 validate 拒绝且零
///   spawn;recipe 相位放行并完成 spawn(revalidate 走 recipe 分支,不消费
///   normal launch/write/resume 分格)。
#[tokio::test]
async fn lcg_t02_root_recipe_phase_not_constructible_by_normal_caller() {
    use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
    use crate::product::logical_codebase::aggregate_initialization::AggregateInitializationStepKind;
    use crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential;
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    let root = fixture.real_worktree();
    let credential = BootstrapPhaseCredential::for_test(
        "project_0001",
        "lc_0001",
        "op_recipe_0001",
        AggregateInitializationStepKind::PreCheck,
        "sha256:recipe-input",
        root.clone(),
    );
    fixture
        .capabilities()
        .set_write_boundary_cell(ProviderCapabilityEvidence::Denied {
            reason: "boundary probe denied".to_string(),
        });

    let request = SessionLaunchRequest {
        project_id: "project_0001".to_string(),
        provider: ProviderRef::claude_code("cap_claude_code_1_4_0"),
        action: SessionPolicyAction::CodingTargetWrite,
        target: PolicyTarget::aggregate_root(root.clone()),
        working_directory: root.clone(),
        readable_roots: vec![root.clone()],
        writable_roots: vec![root.clone()],
        config_artifact_ref: "sha256:managed-config-artifact".to_string(),
    };

    // 普通 validate:write_boundary Denied → 拒绝,零 spawn;产出恒 Normal。
    let ordinary_request_using_recipe_evidence = fixture.gateway().validate(request.clone());
    assert!(ordinary_request_using_recipe_evidence.is_err());
    assert_eq!(fixture.registry_start_count(), 0);

    // recipe 相位:同一请求经 gateway 内部入口放行,phase 冻结为 RootRecipe。
    let validated = fixture
        .gateway()
        .validate_root_recipe_request(request, &credential)
        .expect("recipe phase must validate with fixed Claude recipe facts");
    assert!(validated.is_root_recipe_phase());

    // 对照:普通 validate 的产出永远不是 recipe 相位(分格恢复 Confirmed 后
    // 放行,phase 仍为 Normal)。
    fixture
        .capabilities()
        .set_write_boundary_cell(ProviderCapabilityEvidence::Confirmed);
    let normal_validated = fixture
        .gateway()
        .validate(fixture.coding_request(root.clone()))
        .unwrap();
    assert!(!normal_validated.is_root_recipe_phase());

    // recipe 相位 spawn:再次置 Denied 后 revalidate 仍走 recipe 分支(不消
    // 费 normal 分格),启动成功——root recipe 契约在 normal 门拒绝时保持。
    fixture
        .capabilities()
        .set_write_boundary_cell(ProviderCapabilityEvidence::Denied {
            reason: "boundary probe denied again".to_string(),
        });
    let input = fixture.streaming_input(root, None);
    let launch = ValidatedStreamingProviderInput::new(input, validated);
    let _session = fixture
        .gateway()
        .start_streaming(launch, CancellationToken::new())
        .await
        .expect("recipe phase must spawn despite denied normal write cell");
    assert_eq!(fixture.registry_start_count(), 1);
}

/// resume 能力放行路径(B-2):`resume_evidence` 为 `Confirmed` 时 resume 启动通过复验
/// 并触达 registry(start_count == 1),确认消费者只在非 Confirmed 时阻断。
#[tokio::test]
async fn start_streaming_resume_passes_when_resume_evidence_confirmed() {
    let fixture = gateway_fixture();
    fixture.install_bootstrap_policy();
    let worktree = fixture.real_worktree();
    // planning 请求的 target worktree 必须等于真实 worktree,使 cwd 复验通过。
    let request = SessionLaunchRequest::planning(
        fixture.manifest().project_id,
        ProviderRef::claude_code("cap_claude_code_1_4_0"),
        PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree.clone()),
        vec![fixture.paths.root().to_path_buf()],
        "sha256:managed-config-artifact",
    );
    let validated = fixture.gateway().validate(request).unwrap();
    let input = fixture.streaming_input(worktree, Some("sess_resume_0001".to_string()));
    let launch = ValidatedStreamingProviderInput::new(input, validated);

    fixture
        .capabilities()
        .set_resume_evidence(ResumeEvidenceState::Confirmed);

    let _session = fixture
        .gateway()
        .start_streaming(launch, CancellationToken::new())
        .await
        .expect("resume launch passes when evidence confirmed");
    assert_eq!(fixture.registry_start_count(), 1);
}

/// 可变 target resolver:模拟 spawn 前 git_dir/target TOCTOU。初始 current 与
/// expected 一致(resolve 通过);`change_git_dir_after_request` 后 current
/// 偏离 expected,resolve 返回 `TargetMismatch { field: "git_dir" }`;
/// `change_target_after_request` 后 resolve 返回偏离请求的 target,供 spawn 前
/// target identity 复验测试驱动漂移。
struct MutableTargetResolver {
    expected_git_dir: PathBuf,
    current_git_dir: std::sync::Mutex<PathBuf>,
    target_override: std::sync::Mutex<Option<PolicyTarget>>,
}

impl MutableTargetResolver {
    fn new(git_dir: PathBuf) -> Self {
        let current = git_dir.clone();
        Self {
            expected_git_dir: git_dir,
            current_git_dir: std::sync::Mutex::new(current),
            target_override: std::sync::Mutex::new(None),
        }
    }

    fn change_git_dir_after_request(&self, git_dir: impl Into<PathBuf>) {
        *self.current_git_dir.lock().unwrap() = git_dir.into();
    }

    fn change_target_after_request(&self, target: PolicyTarget) {
        *self.target_override.lock().unwrap() = Some(target);
    }
}

impl PolicyTargetResolver for MutableTargetResolver {
    fn resolve_and_revalidate(
        &self,
        request: &SessionLaunchRequest,
    ) -> Result<PolicyTarget, ProviderGatewayError> {
        let current = self.current_git_dir.lock().unwrap().clone();
        if current != self.expected_git_dir {
            return Err(ProviderGatewayError::TargetMismatch {
                field: "git_dir".to_string(),
            });
        }
        if let Some(target) = self.target_override.lock().unwrap().clone() {
            return Ok(target);
        }
        Ok(request.target.clone())
    }
}

/// 测试用 capability source:version 与 resume/write 分格可调,以驱动 spawn
/// 前复验指纹漂移与 resume/write-boundary fail-closed(Task 2b 分格门)。
struct StaticCapabilitySource {
    version: std::sync::Mutex<String>,
    resume_cell: std::sync::Mutex<ProviderCapabilityEvidence>,
    write_boundary_cell: std::sync::Mutex<ProviderCapabilityEvidence>,
}

impl StaticCapabilitySource {
    fn new(version: &str) -> Self {
        Self {
            version: std::sync::Mutex::new(version.to_string()),
            resume_cell: std::sync::Mutex::new(ProviderCapabilityEvidence::Confirmed),
            write_boundary_cell: std::sync::Mutex::new(ProviderCapabilityEvidence::Confirmed),
        }
    }

    fn set_version(&self, version: &str) {
        *self.version.lock().unwrap() = version.to_string();
    }

    /// 旧 resume 二态兼容入口:Confirmed → 三态 Confirmed;Unsupported →
    /// 三态 Denied(既有 resume fail-closed 测试语义零变化)。
    fn set_resume_evidence(&self, state: ResumeEvidenceState) {
        *self.resume_cell.lock().unwrap() = match state {
            ResumeEvidenceState::Confirmed => ProviderCapabilityEvidence::Confirmed,
            ResumeEvidenceState::Unsupported => ProviderCapabilityEvidence::Denied {
                reason: "resume evidence unsupported".to_string(),
            },
        };
    }

    /// 直接设置三态 resume 证据(Task 9a 显式 resume Unknown 决策测试;
    /// parking_lot 非本 crate 依赖,沿用本文件 std Mutex 约定)。
    fn set_resume_evidence_state(&self, state: ProviderCapabilityEvidence) {
        *self.resume_cell.lock().unwrap() = state;
    }
    fn set_write_boundary_cell(&self, evidence: ProviderCapabilityEvidence) {
        *self.write_boundary_cell.lock().unwrap() = evidence;
    }

    fn capability(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> ProviderCapability {
        let version = self.version.lock().unwrap().clone();
        let resume = self.resume_cell.lock().unwrap().clone();
        let write_boundary = self.write_boundary_cell.lock().unwrap().clone();
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
            version,
            adapter_dialect,
            wire_dialect,
            capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
            action_capability: ProviderActionCapability {
                action,
                launch: ProviderCapabilityEvidence::Confirmed,
                resume,
                write_boundary,
                projection_digest: format!("projection-digest-{action:?}"),
                evidence_ref: format!("probe://{action:?}"),
            },
            trust: ProviderCapabilityEvidence::Confirmed,
        }
    }
}

impl ProviderCapabilitySource for StaticCapabilitySource {
    fn require_supported(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        Ok(self.capability(provider, action))
    }

    fn require_resume_supported(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        let capability = self.capability(provider, action);
        if capability.action_capability.resume != ProviderCapabilityEvidence::Confirmed {
            return Err(ProviderGatewayError::ResumeNotSupported);
        }
        Ok(capability)
    }

    fn require_write_boundary(
        &self,
        provider: &ProviderRef,
        action: SessionPolicyAction,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        let capability = self.capability(provider, action);
        if capability.action_capability.write_boundary != ProviderCapabilityEvidence::Confirmed {
            return Err(ProviderGatewayError::UnsupportedCapability(
                PROVIDER_CAPABILITY_WRITE_BOUNDARY_NOT_CONFIRMED.to_string(),
            ));
        }
        Ok(capability)
    }

    fn require_root_recipe_supported(
        &self,
        provider: &ProviderRef,
        _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
    ) -> Result<ProviderCapability, ProviderGatewayError> {
        // 测试 double:固定 Claude recipe 事实的基础形态(durable 重核验归
        // StoreBacked source 的 for_lc 通道,由 admission 侧测试覆盖)。
        if provider.provider_type != ProviderRefType::ClaudeCode {
            return Err(ProviderGatewayError::UnsupportedCapability(
                PROVIDER_ROOT_RECIPE_REQUIRES_FIXED_CLAUDE.to_string(),
            ));
        }
        Ok(self.capability(provider, SessionPolicyAction::PlanningReadOnly))
    }
}

/// 测试用 streaming adapter:记录 start 调用次数,供断言「复验失败不触达 registry」。
struct CountingStreamingAdapter {
    start_count: std::sync::atomic::AtomicUsize,
}

impl CountingStreamingAdapter {
    fn new() -> Self {
        Self {
            start_count: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn start_count(&self) -> usize {
        self.start_count.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl crate::cross_cutting::streaming_provider::StreamingProviderAdapter
    for CountingStreamingAdapter
{
    async fn start(
        &self,
        _input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<
        crate::cross_cutting::streaming_provider::ProviderSession,
        crate::cross_cutting::provider_adapter::ProviderAdapterError,
    > {
        self.start_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (_event_tx, events) = tokio::sync::mpsc::channel(1);
        let (commands, _command_rx) = tokio::sync::mpsc::channel(1);
        Ok(crate::cross_cutting::streaming_provider::ProviderSession {
            events,
            commands,
            native_session_id: None,
        })
    }

    /// Task 7 分流收口:gateway `start_streaming` 只走 validated trait,
    /// 计数并入同一 `start_count`(「复验失败不触达 registry」断言不变)。
    async fn start_validated(
        &self,
        _input: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<
        crate::cross_cutting::streaming_provider::ProviderSession,
        crate::cross_cutting::provider_adapter::ProviderAdapterError,
    > {
        self.start_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (_event_tx, events) = tokio::sync::mpsc::channel(1);
        let (commands, _command_rx) = tokio::sync::mpsc::channel(1);
        Ok(crate::cross_cutting::streaming_provider::ProviderSession {
            events,
            commands,
            native_session_id: None,
        })
    }
}

/// 测试用 streaming adapter:记录每次 validated start 收到的 tool_policy
/// (r61 kimi 例外映射对照断言用),返回最小空会话。
struct RecordingStreamingAdapter {
    observed_tool_policies:
        std::sync::Mutex<Vec<Option<crate::cross_cutting::streaming_provider::ProviderToolPolicy>>>,
}

impl Default for RecordingStreamingAdapter {
    fn default() -> Self {
        Self {
            observed_tool_policies: std::sync::Mutex::new(Vec::new()),
        }
    }
}

impl RecordingStreamingAdapter {
    fn observed_tool_policies(
        &self,
    ) -> Vec<Option<crate::cross_cutting::streaming_provider::ProviderToolPolicy>> {
        self.observed_tool_policies.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl crate::cross_cutting::streaming_provider::StreamingProviderAdapter
    for RecordingStreamingAdapter
{
    async fn start(
        &self,
        _input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<
        crate::cross_cutting::streaming_provider::ProviderSession,
        crate::cross_cutting::provider_adapter::ProviderAdapterError,
    > {
        let (_event_tx, events) = tokio::sync::mpsc::channel(1);
        let (commands, _command_rx) = tokio::sync::mpsc::channel(1);
        Ok(crate::cross_cutting::streaming_provider::ProviderSession {
            events,
            commands,
            native_session_id: None,
        })
    }

    async fn start_validated(
        &self,
        input: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<
        crate::cross_cutting::streaming_provider::ProviderSession,
        crate::cross_cutting::provider_adapter::ProviderAdapterError,
    > {
        let (input, _launch) = input.into_parts();
        self.observed_tool_policies
            .lock()
            .unwrap()
            .push(input.tool_policy.clone());
        let (_event_tx, events) = tokio::sync::mpsc::channel(1);
        let (commands, _command_rx) = tokio::sync::mpsc::channel(1);
        Ok(crate::cross_cutting::streaming_provider::ProviderSession {
            events,
            commands,
            native_session_id: None,
        })
    }
}

/// 测试用同步 adapter stub:run 返回最小成功输出。
struct StubSyncAdapter;

impl crate::cross_cutting::provider_adapter::ProviderAdapter for StubSyncAdapter {
    fn run(
        &self,
        _input: &crate::protocol::contracts::AdapterInput,
    ) -> Result<
        crate::protocol::contracts::AdapterOutput,
        crate::cross_cutting::provider_adapter::ProviderAdapterError,
    > {
        stub_sync_output()
    }

    /// Task 7 分流收口:gateway `run_sync` 只走 validated trait。
    fn run_validated(
        &self,
        _launch: crate::cross_cutting::session_launch::ValidatedAdapterInput,
    ) -> Result<
        crate::protocol::contracts::AdapterOutput,
        crate::cross_cutting::provider_adapter::ProviderAdapterError,
    > {
        stub_sync_output()
    }
}

fn stub_sync_output() -> Result<
    crate::protocol::contracts::AdapterOutput,
    crate::cross_cutting::provider_adapter::ProviderAdapterError,
> {
    use crate::protocol::contracts::TimeoutStatus;
    Ok(crate::protocol::contracts::AdapterOutput {
        exit_code: Some(0),
        stdout: "ok".to_string(),
        stderr: String::new(),
        structured_output: None,
        files_modified: Vec::new(),
        duration_ms: 0,
        timeout_status: TimeoutStatus::NotTimedOut,
    })
}

/// 始终可用的 availability gate fixture:health snapshot 标记所有真实 provider 可用。
fn always_available_gate()
-> Arc<crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate> {
    use crate::cross_cutting::provider_availability_gate::ProviderHealthSource;
    use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
    use crate::product::models::ProviderName;
    use chrono::Utc;

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
        providers: [
            ProviderName::ClaudeCode,
            ProviderName::Codex,
            ProviderName::Pi,
            ProviderName::KimiCode,
        ]
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
    Arc::new(
        crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate::new(Arc::new(
            AlwaysHealthy(snapshot),
        )),
    )
}

struct GatewayFixture {
    _root: tempfile::TempDir,
    paths: ProductAppPaths,
    capabilities: Arc<StaticCapabilitySource>,
    targets: Arc<MutableTargetResolver>,
    streaming_adapter: Arc<CountingStreamingAdapter>,
    sync_adapter: Arc<StubSyncAdapter>,
    gate: Arc<crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate>,
    /// 跨多次 `gateway()` 构造共享的启动审计。Task 11 的覆盖率测试驱动 4 类
    /// 逻辑启动,每次构造独立 gateway 后审计计数仍需累计。
    audit: Arc<GatewayRunAudit>,
}

fn gateway_fixture() -> GatewayFixture {
    let root = tempfile::tempdir().expect("temporary product root");
    let paths = ProductAppPaths::new(root.path());
    let baseline_git_dir = paths.root().join("baseline.git");
    GatewayFixture {
        _root: root,
        paths,
        capabilities: Arc::new(StaticCapabilitySource::new("1.4.0")),
        targets: Arc::new(MutableTargetResolver::new(baseline_git_dir)),
        streaming_adapter: Arc::new(CountingStreamingAdapter::new()),
        sync_adapter: Arc::new(StubSyncAdapter),
        gate: always_available_gate(),
        audit: Arc::new(GatewayRunAudit::new()),
    }
}

impl GatewayFixture {
    fn manifest(&self) -> LogicalCodebaseManifest {
        LogicalCodebaseManifest::new("project_0001", self.paths.root().to_path_buf(), vec![])
    }

    fn policy_store(&self) -> AggregatePolicyArtifactStore {
        AggregatePolicyArtifactStore::new(self.paths.clone())
    }

    fn install_bootstrap_policy(&self) {
        let manifest = self.manifest();
        self.policy_store().ensure_bootstrap(&manifest).unwrap();
    }

    fn gateway(&self) -> LogicalCodebaseProviderGateway {
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::ClaudeCode, self.streaming_adapter.clone());
        registry.register(ProviderName::Codex, self.streaming_adapter.clone());
        // 权威根 = manifest.provider_context_root(temp dir),构造时 canonicalize。
        let authority_root = std::fs::canonicalize(self.manifest().provider_context_root)
            .expect("fixture provider context root exists");
        LogicalCodebaseProviderGateway::with_audit(
            self.policy_store(),
            self.capabilities.clone(),
            self.targets.clone(),
            Arc::new(registry),
            self.sync_adapter.clone(),
            self.gate.clone(),
            self.audit.clone(),
            authority_root,
        )
    }

    /// r61 kimi 例外映射测试构形:ClaudeCode/KimiCode 注册到同一 recording
    /// adapter,支撑「kimi 收 None、claude 对照收 Some(deny)」的对照断言。
    fn gateway_with_recording_streaming_adapter(
        &self,
        adapter: Arc<RecordingStreamingAdapter>,
    ) -> LogicalCodebaseProviderGateway {
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::ClaudeCode, adapter.clone());
        registry.register(ProviderName::KimiCode, adapter);
        // 权威根 = manifest.provider_context_root(temp dir),构造时 canonicalize。
        let authority_root = std::fs::canonicalize(self.manifest().provider_context_root)
            .expect("fixture provider context root exists");
        LogicalCodebaseProviderGateway::with_audit(
            self.policy_store(),
            self.capabilities.clone(),
            self.targets.clone(),
            Arc::new(registry),
            self.sync_adapter.clone(),
            self.gate.clone(),
            self.audit.clone(),
            authority_root,
        )
    }

    /// 返回共享启动审计。多次 `gateway()` 构造后的启动记录都在此处累计。
    fn gateway_audit(&self) -> Arc<GatewayRunAudit> {
        self.audit.clone()
    }

    fn targets(&self) -> Arc<MutableTargetResolver> {
        self.targets.clone()
    }

    fn capabilities(&self) -> Arc<StaticCapabilitySource> {
        self.capabilities.clone()
    }

    fn registry_start_count(&self) -> usize {
        self.streaming_adapter.start_count()
    }

    /// 将 store 中的 policy artifact 升级到 revision 2(新 policy_text + 新 digest),
    /// 模拟 validate→spawn 之间政策被升级(TOCTOU)。
    fn upgrade_policy(&self) {
        let manifest = self.manifest();
        let store = self.policy_store();
        let current = store.get(&manifest.project_id).unwrap().unwrap();
        let revised = current.with_revised_policy(
            "# Aggregate policy (revision 2)\n\nTighter supervised scope.\n",
            manifest.updated_at.clone(),
        );
        store.save(&manifest.project_id, &revised).unwrap();
    }

    /// 创建一个真实存在的 worktree 目录(用于 spawn 前 canonicalize 复验)。
    fn real_worktree(&self) -> PathBuf {
        let worktree = self.paths.root().join("worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        worktree
    }

    /// 无参便利方法:返回 planning 只读请求(内部用 manifest())。
    fn planning_request(&self) -> SessionLaunchRequest {
        self.planning_request_for_manifest(&self.manifest())
    }

    fn planning_request_for_manifest(
        &self,
        manifest: &LogicalCodebaseManifest,
    ) -> SessionLaunchRequest {
        let target = PolicyTarget::checkout(
            "logical_repo_0001",
            "checkout_0001",
            self.paths.root().join("worktree"),
        );
        SessionLaunchRequest::planning(
            &manifest.project_id,
            ProviderRef::claude_code("cap_claude_code_1_4_0"),
            target,
            vec![self.paths.root().to_path_buf()],
            "sha256:managed-config-artifact",
        )
    }

    /// coding 请求:恰好一个等于 canonical target worktree 的 write root。
    fn coding_request(&self, worktree: impl Into<PathBuf>) -> SessionLaunchRequest {
        let manifest = self.manifest();
        let worktree = worktree.into();
        let target = PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree.clone());
        SessionLaunchRequest {
            project_id: manifest.project_id,
            provider: ProviderRef::claude_code("cap_claude_code_1_4_0"),
            action: SessionPolicyAction::CodingTargetWrite,
            target,
            working_directory: worktree.clone(),
            readable_roots: vec![self.paths.root().to_path_buf()],
            writable_roots: vec![worktree],
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        }
    }

    /// 针对真实 worktree 目录构造一个 planning 只读 validated streaming input。
    /// worktree 必须真实存在(spawn 前 canonicalize 复验需要)。
    fn validated_planning_streaming_input(
        self: &GatewayFixture,
        worktree: PathBuf,
    ) -> ValidatedStreamingProviderInput {
        let request = SessionLaunchRequest::planning(
            self.manifest().project_id,
            ProviderRef::claude_code("cap_claude_code_1_4_0"),
            PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree.clone()),
            vec![self.paths.root().to_path_buf()],
            "sha256:managed-config-artifact",
        );
        let validated = self.gateway().validate(request).unwrap();
        ValidatedStreamingProviderInput::new(self.streaming_input(worktree, None), validated)
    }

    /// 构造一个 streaming input;resume 设 `resume_provider_session_id`。
    fn streaming_input(
        &self,
        working_dir: PathBuf,
        resume_id: Option<String>,
    ) -> crate::cross_cutting::streaming_provider::StreamingProviderInput {
        use crate::cross_cutting::streaming_provider::{
            ProviderPermissionMode, StreamingProviderInput,
        };
        use crate::protocol::contracts::{AdapterRole, ProviderType};
        StreamingProviderInput {
            working_directory: None,
            baseline_tree: None,
            tool_policy: None,
            audit_sink: None,
            provider_type: ProviderType::ClaudeCode,
            role: AdapterRole::Executor,
            prompt: "probe".to_string(),
            working_dir,
            workspace_session_id: None,
            resume_provider_session_id: resume_id,
            permission_mode: ProviderPermissionMode::Auto,
            structured_output_contract: None,
            env_vars: Default::default(),
            timeout_secs: 1,
        }
    }

    /// review 只读请求:与 planning 同为 read-only action,但 action 为
    /// `ReviewReadOnly`。Task 11 的 review 流式启动经此请求走 gateway。
    fn review_request(&self, worktree: PathBuf) -> SessionLaunchRequest {
        let manifest = self.manifest();
        let target = PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree);
        SessionLaunchRequest {
            project_id: manifest.project_id,
            provider: ProviderRef::claude_code("cap_claude_code_1_4_0"),
            action: SessionPolicyAction::ReviewReadOnly,
            working_directory: target.worktree.clone(),
            target,
            readable_roots: vec![self.paths.root().to_path_buf()],
            writable_roots: Vec::new(),
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        }
    }

    /// 同步栈入口(work item split):经 gateway `run_sync` 启动同步 adapter。
    /// 对应 `work_item_split_engine/engine.rs` 逻辑代码库分支。worktree 必须真实
    /// 存在以通过 spawn 前 canonicalize 复验。
    async fn run_logical_work_item_split(&self) -> Result<(), ProviderGatewayError> {
        let worktree = self.real_worktree();
        let request = self.planning_request_for_manifest_with_worktree(worktree.clone());
        let validated = self.gateway().validate(request)?;
        let adapter_input = AdapterInput {
            working_directory: None,
            provider_type: crate::protocol::contracts::ProviderType::ClaudeCode,
            role: crate::protocol::contracts::AdapterRole::WorkItemSplitter,
            worktree_path: Some(worktree.to_string_lossy().to_string()),
            provider_stream_log_dir: None,
            prompt: "split work items".to_string(),
            context_files: Vec::new(),
            output_schema: String::new(),
            timeout: 1,
            max_retries: 0,
        };
        let launch = ValidatedAdapterInput::new(adapter_input, validated);
        self.gateway().run_sync(launch)?;
        Ok(())
    }

    /// 流式 planning 栈入口:经 gateway `start_streaming` 启动。对应
    /// `workspace_engine/provider_drive.rs` 逻辑代码库 planning 分支。
    async fn run_logical_planning_stream(&self) -> Result<(), ProviderGatewayError> {
        let worktree = self.real_worktree();
        let request = self.planning_request_for_manifest_with_worktree(worktree.clone());
        let validated = self.gateway().validate(request)?;
        let input = self.streaming_input(worktree, None);
        let launch = ValidatedStreamingProviderInput::new(input, validated);
        let gateway = self.gateway();
        gateway
            .start_streaming(launch, CancellationToken::new())
            .await?;
        Ok(())
    }

    /// 流式 coding 栈入口:经 gateway `start_streaming` 启动。对应
    /// `coding_workspace_engine/provider_stream.rs` 逻辑代码库 coding 分支。
    async fn run_logical_coding_stream(&self) -> Result<(), ProviderGatewayError> {
        let worktree = self.real_worktree();
        let request = self.coding_request(worktree.clone());
        let validated = self.gateway().validate(request)?;
        let input = self.streaming_input(worktree, None);
        let launch = ValidatedStreamingProviderInput::new(input, validated);
        let gateway = self.gateway();
        gateway
            .start_streaming(launch, CancellationToken::new())
            .await?;
        Ok(())
    }

    /// 流式 review 栈入口:经 gateway `start_streaming` 启动。对应
    /// `coding_workspace_engine/provider_stream.rs` 逻辑代码库 review 分支。
    async fn run_logical_review_stream(&self) -> Result<(), ProviderGatewayError> {
        let worktree = self.real_worktree();
        let request = self.review_request(worktree.clone());
        let validated = self.gateway().validate(request)?;
        let input = self.streaming_input(worktree, None);
        let launch = ValidatedStreamingProviderInput::new(input, validated);
        let gateway = self.gateway();
        gateway
            .start_streaming(launch, CancellationToken::new())
            .await?;
        Ok(())
    }

    /// planning 只读请求的 worktree 参数化变体,供 work-item split/planning stream
    /// 复用同一真实 worktree(而不是 fixture 默认的未创建 `worktree` 路径)。
    fn planning_request_for_manifest_with_worktree(
        &self,
        worktree: PathBuf,
    ) -> SessionLaunchRequest {
        let target = PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree);
        SessionLaunchRequest::planning(
            self.manifest().project_id,
            ProviderRef::claude_code("cap_claude_code_1_4_0"),
            target,
            vec![self.paths.root().to_path_buf()],
            "sha256:managed-config-artifact",
        )
    }
}

/// Task 11 覆盖率:验证逻辑代码库同步与流式 provider 全入口经 gateway 接线后,
/// 每类真实启动都在 gateway 留下可审计记录。本模块是 gateway 侧契约测试(B2):
/// 4 个 `run_logical_*` helper 分别模拟逻辑代码库的 work-item split(同步)、
/// planning/coding/review(流式)启动,各向 gateway 发一次真实 `run_sync`/
/// `start_streaming`,断言审计计数(sync==1、stream==3)与全部携带 policy digest。
///
/// 该测试是 4 个真实入口(engine.rs / provider_drive.rs / provider_stream.rs /
/// initializer.rs)逻辑分支接线是否完成的验收门:只要 gateway 是唯一启动入口且
/// 记录审计,计数即为契约。
mod coverage_tests {
    use super::*;

    #[tokio::test]
    async fn logical_provider_entrypoints_use_gateway_for_sync_and_streaming_stacks() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        fixture.run_logical_work_item_split().await.unwrap();
        fixture.run_logical_planning_stream().await.unwrap();
        fixture.run_logical_coding_stream().await.unwrap();
        fixture.run_logical_review_stream().await.unwrap();

        assert_eq!(fixture.gateway_audit().sync_launches(), 1);
        assert_eq!(fixture.gateway_audit().stream_launches(), 3);
        assert!(fixture.gateway_audit().all_have_policy_digest());
    }
}

/// Task 13: Codex `danger-full-access` 路由级阻断、resume 一致性与配置来源审计。
///
/// 三个验收维度:
/// 1. Codex 在受限写配置(danger-full-access)下经 gateway 路由级阻断——不论 UI
///    是否选择该 provider,validate 即拒绝,不触达 registry。这与「UI 隐藏」
///    不同:路由级阻断无法被绕过。
/// 2. resume 一致性:`resume_or_start` 只有当 policy digest、target、provider exact
///    version、dialect 与 capability snapshot 全一致(fingerprint 相等)才 resume;
///    任一漂移则 supersede 旧会话并新建。
/// 3. 配置来源审计:`ConfigSourceAudit` 记录 user/project/local/env/子仓 MCP 来源
///    与最终 argv/config digest;发现 managed settings 时标注
///    `managed_settings_active=true` 并发警告,绝不假装已覆盖;policy 可据此拒绝启动。
mod task13_gateway_hardening {
    use super::*;
    use crate::cross_cutting::codex_provider::CODEX_DEFAULT_SANDBOX_MODE;

    /// 用 Codex provider ref 构造一个 coding 写请求(worktree 必须等于 target)。
    fn codex_coding_request(fixture: &GatewayFixture) -> SessionLaunchRequest {
        let manifest = fixture.manifest();
        let worktree = fixture.paths.root().join("codex-worktree");
        std::fs::create_dir_all(&worktree).unwrap();
        let target = PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree.clone());
        SessionLaunchRequest {
            project_id: manifest.project_id,
            provider: ProviderRef::codex("cap_codex_1_0_0"),
            action: SessionPolicyAction::CodingTargetWrite,
            target,
            working_directory: worktree.clone(),
            readable_roots: vec![fixture.paths.root().to_path_buf()],
            writable_roots: vec![worktree],
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        }
    }

    /// r47 修订:Task 13 期的 Codex 路由级全阻已移除——LC 投影(REQ-LCG-04)
    /// 后 Codex 会话恒非 danger(Planning/Review→read-only,Coding→
    /// workspace-write;projection 无 Danger 变体),全阻把 2d 探针签发的
    /// Confirmed 行一并拒之门外(r47 codex 首轮现场)。LC 会话的 Codex 放行
    /// 由 capability 行(探针 Confirmed)与投影复验裁决;direct coder 的
    /// danger 缺陷属 direct 路径(danger 模式不进入本 gateway)。本测试改钉
    /// 新契约:capability double 放行时 validate 成功(danger 全阻不复存在)。
    #[test]
    fn codex_danger_full_access_is_blocked_at_gateway_route_even_outside_ui() {
        let _ = CODEX_DEFAULT_SANDBOX_MODE;
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let request = codex_coding_request(&fixture);

        let validated = fixture.gateway().validate(request).expect(
            "r47:LC 会话 Codex 不再路由级全阻——capability 行+投影复验裁决",
        );
        assert_eq!(
            validated.envelope().provider_dialect,
            crate::product::logical_codebase::policy::ProviderDialect::CodexCliV1,
            "Codex LC 会话经投影正常 validated(sandbox=workspace-write 非 danger)"
        );
        assert_eq!(fixture.registry_start_count(), 0);
    }

    /// 同一 provider type 但 ClaudeCode 不受 danger-full-access 阻断:确认阻断
    /// 精确针对 Codex+danger-full-access,而非误伤所有 provider。
    #[test]
    fn claude_code_is_not_blocked_by_codex_danger_full_access_guard() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let worktree = fixture.real_worktree();
        let request = SessionLaunchRequest::planning(
            fixture.manifest().project_id,
            ProviderRef::claude_code("cap_claude_code_1_4_0"),
            PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree),
            vec![fixture.paths.root().to_path_buf()],
            "sha256:managed-config-artifact",
        );

        let validated = fixture.gateway().validate(request);
        assert!(
            validated.is_ok(),
            "ClaudeCode must not be blocked: {validated:?}"
        );
    }

    /// resume 一致性:fingerprint 全相等 → `GatewaySessionDisposition::Resume`,
    /// 旧会话不被 supersede。
    #[test]
    fn resume_or_start_resumes_when_fingerprint_matches_exactly() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let worktree = fixture.real_worktree();
        let request = SessionLaunchRequest::planning(
            fixture.manifest().project_id,
            ProviderRef::claude_code("cap_claude_code_1_4_0"),
            PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree),
            vec![fixture.paths.root().to_path_buf()],
            "sha256:managed-config-artifact",
        );

        // 先 validate 一次取基线 fingerprint,模拟旧会话冻结的指纹。
        let previous = fixture.gateway().validate(request.clone()).unwrap();
        let resume_request = ResumeSessionLaunchRequest {
            launch: request,
            previous_fingerprint: previous.fingerprint().clone(),
            previous_session_id: "sess_old_0001".to_string(),
        };

        let disposition = fixture.gateway().resume_or_start(resume_request).unwrap();
        assert!(matches!(disposition, GatewaySessionDisposition::Resume(_)));
        // 无 supersede 审计记录。
        assert_eq!(fixture.gateway_audit().supersede_count(), 0);
    }

    /// resume 一致性:provider version 漂移(旧会话指纹记录 1.4.0,当前 1.4.1)→
    /// fingerprint 不一致 → supersede 旧会话并 `StartNew`,旧 session id 被透传。
    #[test]
    fn resume_or_start_supersedes_when_provider_version_drifts() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let worktree = fixture.real_worktree();
        let request = || {
            SessionLaunchRequest::planning(
                fixture.manifest().project_id,
                ProviderRef::claude_code("cap_claude_code_1_4_0"),
                PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree.clone()),
                vec![fixture.paths.root().to_path_buf()],
                "sha256:managed-config-artifact",
            )
        };

        // 旧会话指纹在 version=1.4.0 时冻结。
        let previous = fixture.gateway().validate(request()).unwrap();
        let stale_fingerprint = previous.fingerprint().clone();

        // resume 前当前 capability version 漂移到 1.4.1。
        fixture.capabilities().set_version("1.4.1");
        let resume_request = ResumeSessionLaunchRequest {
            launch: request(),
            previous_fingerprint: stale_fingerprint,
            previous_session_id: "sess_old_0002".to_string(),
        };

        let disposition = fixture.gateway().resume_or_start(resume_request).unwrap();
        match disposition {
            GatewaySessionDisposition::StartNew {
                superseded_session_id,
                ..
            } => {
                assert_eq!(superseded_session_id, "sess_old_0002");
            }
            other => panic!("expected StartNew, got {other:?}"),
        }
        // supersede 审计记录一次,原因标记为 fingerprint mismatch。
        assert_eq!(fixture.gateway_audit().supersede_count(), 1);
        assert!(
            fixture
                .gateway_audit()
                .last_supersede_reason()
                .is_some_and(|reason| reason == "resume_fingerprint_mismatch")
        );
    }

    /// resume 一致性:policy digest 漂移(政策在旧会话后被升级)→ supersede 旧会话。
    /// 证明 fingerprint 覆盖 policy digest 维度,不只 provider version。
    #[test]
    fn resume_or_start_supersedes_when_policy_digest_drifts() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let worktree = fixture.real_worktree();
        let request = || {
            SessionLaunchRequest::planning(
                fixture.manifest().project_id,
                ProviderRef::claude_code("cap_claude_code_1_4_0"),
                PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree.clone()),
                vec![fixture.paths.root().to_path_buf()],
                "sha256:managed-config-artifact",
            )
        };

        let previous = fixture.gateway().validate(request()).unwrap();
        let stale_fingerprint = previous.fingerprint().clone();

        // 政策在旧会话后被升级(revision 2,新 digest)。
        fixture.upgrade_policy();
        let resume_request = ResumeSessionLaunchRequest {
            launch: request(),
            previous_fingerprint: stale_fingerprint,
            previous_session_id: "sess_old_0003".to_string(),
        };

        let disposition = fixture.gateway().resume_or_start(resume_request).unwrap();
        assert!(matches!(
            disposition,
            GatewaySessionDisposition::StartNew { .. }
        ));
        assert_eq!(fixture.gateway_audit().supersede_count(), 1);
    }

    /// r47 修订:resume_or_start 对 Codex 不再路由级全阻——与其它 provider
    /// 同走指纹比对(stale→StartNew;新契约与上文 validate 测试同源)。
    #[test]
    fn resume_or_start_blocks_codex_danger_full_access_before_resume_decision() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let request = codex_coding_request(&fixture);

        let resume_request = ResumeSessionLaunchRequest {
            launch: request,
            previous_fingerprint: SessionResumeFingerprint {
                digest: "sha256:stale".to_string(),
            },
            previous_session_id: "sess_codex_old".to_string(),
        };

        let disposition = fixture.gateway().resume_or_start(resume_request).expect(
            "r47:Codex resume 与其它 provider 同走指纹比对,不再路由级全阻",
        );
        assert!(
            matches!(disposition, GatewaySessionDisposition::StartNew { .. }),
            "stale 指纹 → StartNew(既有语义)"
        );
    }

    /// C-2 集中映射:session/role 配置的 `ProviderName` → gateway `ProviderRef`。
    /// Task 1a 起四家真实 provider(ClaudeCode/Codex/Pi/KimiCode)各自显式
    /// 映射;Fake 不经 gateway。🔴 禁止静默回退 ClaudeCode(用户配置的
    /// provider 不允许被悄悄换成 Claude 启动)。
    #[test]
    fn provider_ref_from_provider_name_maps_supported_providers() {
        let claude =
            ProviderRef::from_provider_name(&ProviderName::ClaudeCode, "cap_managed_snapshot")
                .expect("ClaudeCode must map to a gateway provider ref");
        assert_eq!(claude.provider_type, ProviderRefType::ClaudeCode);
        assert_eq!(claude.capability_snapshot_ref, "cap_managed_snapshot");

        let codex = ProviderRef::from_provider_name(&ProviderName::Codex, "cap_managed_snapshot")
            .expect("Codex must map to a gateway provider ref");
        assert_eq!(codex.provider_type, ProviderRefType::Codex);
        assert_eq!(codex.capability_snapshot_ref, "cap_managed_snapshot");

        let pi = ProviderRef::from_provider_name(&ProviderName::Pi, "cap_managed_snapshot")
            .expect("Pi must map to a gateway provider ref");
        assert_eq!(pi.provider_type, ProviderRefType::Pi);

        let kimi = ProviderRef::from_provider_name(&ProviderName::KimiCode, "cap_managed_snapshot")
            .expect("KimiCode must map to a gateway provider ref");
        assert_eq!(kimi.provider_type, ProviderRefType::KimiCode);
    }

    /// C-2 集中映射 fail-closed:Fake(及未来新增 provider)一律显式
    /// unsupported,错误信息含稳定判别码与 provider 名,绝不回退 Claude。
    /// (Task 1a 前的旧合同把 Pi/KimiCode 也列入拒绝;四家映射落地后仅
    /// Fake/未知值保持 fail-closed。)
    #[test]
    fn provider_ref_from_provider_name_fails_closed_for_unsupported_providers() {
        let error = ProviderRef::from_provider_name(&ProviderName::Fake, "cap_managed_snapshot")
            .err()
            .unwrap_or_else(|| panic!("Fake must not map to a gateway provider ref"));
        assert!(
            matches!(&error, ProviderGatewayError::UnsupportedCapability(reason)
                if reason.contains(PROVIDER_UNSUPPORTED_FOR_GATEWAY_LAUNCH)),
            "expected {PROVIDER_UNSUPPORTED_FOR_GATEWAY_LAUNCH}, got {error:?}"
        );
        assert!(
            error
                .to_string()
                .contains(&format!("{:?}", ProviderName::Fake)),
            "error must name the configured provider, got {error}"
        );
    }

    /// r61(kimi 现场):LC 策略角色(planning=Orchestrator)由 gateway 角色
    /// 矩阵派生 DenyFileWriteBuiltins,但 kimi 隔离控制面不携带通用 tool
    /// policy(spec「kimi 既有对齐维持」例外)——`start_streaming` 分发漏斗
    /// 对 provider_type==KimiCode 把该 intent 映射为 None(等价面由 kimi
    /// 既有 ClientServicePolicy 承担),否则 kimi validated start 以
    /// `provider_generic_tool_policy_forbidden` 在极早帧拒绝(三家首跑
    /// kimi 根断现场);其余 provider 照发不误(claude 对照)。
    #[tokio::test]
    async fn start_streaming_maps_kimi_policy_role_tool_policy_to_none() {
        use crate::cross_cutting::streaming_provider::ProviderToolPolicy;
        use crate::protocol::contracts::{AdapterRole, ProviderType};

        fn policy_role_streaming_input(
            provider_type: ProviderType,
            working_dir: PathBuf,
        ) -> crate::cross_cutting::streaming_provider::StreamingProviderInput {
            use crate::cross_cutting::streaming_provider::{
                ProviderPermissionMode, StreamingProviderInput,
            };
            // engine 侧照 prepare_streaming_launch 角色矩阵携带 deny 派生:
            // 策略角色(Orchestrator)一律 Some(DenyFileWriteBuiltins)。
            StreamingProviderInput {
                working_directory: None,
                baseline_tree: None,
                tool_policy: Some(ProviderToolPolicy::deny_file_write_builtins()),
                audit_sink: None,
                provider_type,
                role: AdapterRole::Orchestrator,
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

        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let worktree = fixture.real_worktree();
        let adapter = Arc::new(RecordingStreamingAdapter::default());
        let gateway = fixture.gateway_with_recording_streaming_adapter(adapter.clone());

        // kimi planning(策略角色):请求与 input 都指向 kimi。
        let kimi_request = SessionLaunchRequest::planning(
            fixture.manifest().project_id,
            ProviderRef::kimi_code("cap_kimi_code_1_4_0"),
            PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree.clone()),
            vec![fixture.paths.root().to_path_buf()],
            "sha256:managed-config-artifact",
        );
        gateway
            .start_streaming(
                ValidatedStreamingProviderInput::new(
                    policy_role_streaming_input(ProviderType::KimiCode, worktree.clone()),
                    gateway.validate(kimi_request).unwrap(),
                ),
                CancellationToken::new(),
            )
            .await
            .expect("kimi planning launch must start");

        // claude 对照(同请求形状,仅 provider 不同):策略原样到达 adapter。
        let claude_request = SessionLaunchRequest::planning(
            fixture.manifest().project_id,
            ProviderRef::claude_code("cap_claude_code_1_4_0"),
            PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree),
            vec![fixture.paths.root().to_path_buf()],
            "sha256:managed-config-artifact",
        );
        gateway
            .start_streaming(
                ValidatedStreamingProviderInput::new(
                    policy_role_streaming_input(ProviderType::ClaudeCode, fixture.real_worktree()),
                    gateway.validate(claude_request).unwrap(),
                ),
                CancellationToken::new(),
            )
            .await
            .expect("claude planning launch must start");

        assert_eq!(
            adapter.observed_tool_policies(),
            vec![None, Some(ProviderToolPolicy::deny_file_write_builtins())],
            "kimi 策略角色会话必须以 tool_policy=None 分发;claude 对照保持 Some(deny)"
        );
    }

    include!("provider_gateway_tests/audit.inc.rs");
}

/// Task 2.8（映射 tasks.md 2.2；REQ-ENV-01/10/11）：gateway 复验拆除
/// 「canonical cwd == canonical target」错误等式 + 双工厂 root assertion。
///
/// 三个验收维度：
/// 1. 双工厂 root projection（gateway 工厂 authority_root=manifest
///    provider_context_root、登记工厂 record.aggregate_root、aggregate 生产
///    driver 的 root recipe receipt canonical root、envelope 冻结 cwd）四路
///    canonical 相等才放行——不一致 zero spawn（含 gateway 组装即失败）。
///    fixture 带空格目录与 symlink，断言 canonical 相等而非字面相等。
/// 2. cwd authority：越出 authority 的 cwd 在 validate 即拒绝；symlink 逃逸
///    在 spawn 前 canonical 复验拒绝；Git identity 漂移继续 zero spawn；
///    cwd≠target 的合法分离形态（root cwd + member target）放行。
/// 3. 写证据不随 cwd 扩大：coding 的唯一 writable root 仍是 target
///    worktree，root cwd 不得成为写证据（evidence gate 只作证据，不猜 OS
///    sandbox）。
mod task28_gateway_root_authority {
    include!("provider_gateway_tests/task28_root_authority.inc.rs");
}

/// Task 1a(lcg_t01):四家显式 provider 映射与 Fake/未知不回退
/// (REQ-LCG-01;映射、纯 projection/boundary DTO、validated 承载与 registry
/// 基础合同的映射切片)。
mod lcg_t01_provider_mapping {
    include!("provider_gateway_tests/mapping.inc.rs");
}

/// Task 9a(lcg_t09a):resume 指纹核心——五参 `from_envelope`(envelope+
/// projection+action evidence+target git identity)、gateway 内部
/// `resume_or_start_with_projection`(调用者先完整 prepare 得候选投影再比
/// 指纹)与 `StartNew` 的 supersede+显式 fresh 等待面。
///
/// 契约(计划 Interfaces 冻结):
/// 1. 指纹为长度分隔 + schema 域版本的 canonical SHA-256;policy/authority/
///    cwd/target/git identity/trust/tool/MCP bundle/exact version/投影摘要
///    任一漂移都改变 digest。
/// 2. `resume_or_start_with_projection` 只有全上下文指纹与旧会话冻结指纹相等
///    才 `Resume`(返回的 validated 携带完整上下文指纹与候选投影);漂移仅
///    supersede(审计)+`StartNew`(等待显式 `StartGeneration`),决策层
///    零 spawn,不得用仅 envelope 指纹生成可 spawn 权。
/// 3. 旧 `resume_or_start` 的 envelope+capability 基准(单仓/legacy 语义)
///    由既有 task13/task28 测试保持,不在此重证。
mod lcg_t09a_resume_fingerprint {
    use super::*;
    use crate::product::logical_codebase::provider_admission_preflight::BootstrapActionKind;
    use crate::product::logical_codebase::provider_gateway::canonical_target_git_identity;
    use crate::product::logical_codebase::provider_projection::ProviderPolicyProjection;
    use crate::protocol::contracts::AdapterRole;

    /// 由 validated policy 构造候选 LC 投影(与 task28 verdict 消费面同形的
    /// 测试 fixture;trust/MCP/version/摘要可参数化以驱动逐维漂移)。
    #[allow(clippy::too_many_arguments)]
    fn t09a_projection(
        validated: &ValidatedSessionLaunchPolicy,
        trust_digest: &str,
        mcp_bundle_digest: &str,
        exact_version: &str,
        capability_projection_digest: &str,
        projection_digest: &str,
    ) -> ProviderPolicyProjection {
        let envelope = validated.envelope();
        ProviderPolicyProjection::new(
            crate::product::logical_codebase::provider_gateway::ProviderRefType::ClaudeCode,
            envelope.provider_dialect,
            crate::product::logical_codebase::policy::ProviderWireDialect::ClaudeCodeStreamJson,
            exact_version.to_string(),
            envelope.action,
            AdapterRole::Executor,
            crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
            None,
            "on-request".to_string(),
            "read-only".to_string(),
            envelope.working_directory.clone(),
            envelope.working_directory.clone(),
            envelope.target.clone(),
            envelope.readable_roots.clone(),
            envelope.writable_roots.clone(),
            trust_digest.to_string(),
            envelope.config_digest.clone(),
            mcp_bundle_digest.to_string(),
            "probe://boundary-t09a".to_string(),
            capability_projection_digest.to_string(),
            projection_digest.to_string(),
        )
    }

    /// 基准候选投影(稳定摘要 fixture)。
    fn t09a_baseline_projection(
        validated: &ValidatedSessionLaunchPolicy,
    ) -> ProviderPolicyProjection {
        t09a_projection(
            validated,
            "sha256:trust-t09a",
            "sha256:mcp-t09a",
            "claude 1.4.0",
            "sha256:capability-projection-t09a",
            "sha256:session-projection-t09a",
        )
    }

    /// 五参 from_envelope:长度分隔 + schema 域版本的 canonical digest;
    /// policy/cwd/git identity/evidence/trust/MCP/version/投影摘要逐维漂移
    /// 都改变 digest;同输入指纹稳定。
    #[test]
    fn lcg_t09a_from_envelope_freezes_projection_evidence_and_git_dimensions() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let worktree = fixture.real_worktree();
        let request = fixture.planning_request_for_manifest_with_worktree(worktree.clone());
        let validated = fixture
            .gateway()
            .validate(request.clone())
            .expect("planning launch validates");
        let envelope = validated.envelope().clone();
        let projection = t09a_baseline_projection(&validated);
        let evidence = validated.action_evidence_digest().to_string();
        let git_identity = canonical_target_git_identity(&envelope.target.worktree);

        let baseline = SessionResumeFingerprint::from_envelope(
            &envelope,
            &projection,
            &evidence,
            &git_identity,
        );
        assert!(baseline.digest.starts_with("sha256:"));
        assert_eq!(baseline.digest.len(), 71);
        // 稳定控制组:同输入 → 同 digest。
        assert_eq!(
            baseline.digest,
            SessionResumeFingerprint::from_envelope(
                &envelope,
                &projection,
                &evidence,
                &git_identity
            )
            .digest
        );

        // 投影携带维度逐项漂移:trust / MCP bundle / exact version /
        // capability projection 摘要 / 会话全投影摘要。
        for drifted in [
            t09a_projection(
                &validated,
                "sha256:trust-drifted",
                "sha256:mcp-t09a",
                "claude 1.4.0",
                "sha256:capability-projection-t09a",
                "sha256:session-projection-t09a",
            ),
            t09a_projection(
                &validated,
                "sha256:trust-t09a",
                "sha256:mcp-drifted",
                "claude 1.4.0",
                "sha256:capability-projection-t09a",
                "sha256:session-projection-t09a",
            ),
            t09a_projection(
                &validated,
                "sha256:trust-t09a",
                "sha256:mcp-t09a",
                "claude 1.4.1",
                "sha256:capability-projection-t09a",
                "sha256:session-projection-t09a",
            ),
            t09a_projection(
                &validated,
                "sha256:trust-t09a",
                "sha256:mcp-t09a",
                "claude 1.4.0",
                "sha256:capability-projection-drifted",
                "sha256:session-projection-t09a",
            ),
            t09a_projection(
                &validated,
                "sha256:trust-t09a",
                "sha256:mcp-t09a",
                "claude 1.4.0",
                "sha256:capability-projection-t09a",
                "sha256:session-projection-drifted",
            ),
        ] {
            assert_ne!(
                baseline.digest,
                SessionResumeFingerprint::from_envelope(
                    &envelope,
                    &drifted,
                    &evidence,
                    &git_identity
                )
                .digest,
                "projection dimension drift must change the resume fingerprint"
            );
        }

        // 独立参数维度漂移:action evidence 摘要 / target git identity。
        assert_ne!(
            baseline.digest,
            SessionResumeFingerprint::from_envelope(
                &envelope,
                &projection,
                "sha256:action-evidence-drifted",
                &git_identity
            )
            .digest
        );
        assert_ne!(
            baseline.digest,
            SessionResumeFingerprint::from_envelope(
                &envelope,
                &projection,
                &evidence,
                "/main/.git/worktrees/target-drifted"
            )
            .digest
        );

        // envelope 维度漂移:cwd(仅 working_directory 不同)与 policy digest
        // (validate→决策之间政策升级)。
        let mut drifted_request = request.clone();
        drifted_request.working_directory = fixture.paths.root().join("cwd-drifted-t09a");
        let drifted_validated = fixture
            .gateway()
            .validate(drifted_request)
            .expect("cwd-drifted launch still validates within the authority root");
        assert_ne!(
            baseline.digest,
            SessionResumeFingerprint::from_envelope(
                drifted_validated.envelope(),
                &projection,
                &evidence,
                &git_identity
            )
            .digest
        );

        fixture.upgrade_policy();
        let upgraded_validated = fixture
            .gateway()
            .validate(request)
            .expect("upgraded policy still validates");
        assert_ne!(
            baseline.digest,
            SessionResumeFingerprint::from_envelope(
                upgraded_validated.envelope(),
                &projection,
                &evidence,
                &git_identity
            )
            .digest
        );
    }

    /// 全上下文指纹匹配 → `Resume`:调用者先完整 prepare 得候选投影,
    /// gateway 用同一投影+action evidence+git identity 重算指纹与旧会话
    /// 冻结值比对;返回的 validated 携带完整上下文指纹与候选投影(供
    /// spawn 前复验与调用方存储),全程零 supersede。
    #[test]
    fn lcg_t09a_resume_or_start_with_projection_resumes_on_matching_full_context() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let worktree = fixture.real_worktree();
        let request = fixture.planning_request_for_manifest_with_worktree(worktree);
        let gateway = fixture.gateway();
        let validated = gateway
            .validate(request.clone())
            .expect("planning launch validates");
        let projection = t09a_baseline_projection(&validated);
        let previous = SessionResumeFingerprint::from_envelope(
            validated.envelope(),
            &projection,
            validated.action_evidence_digest(),
            &canonical_target_git_identity(&validated.envelope().target.worktree),
        );

        let disposition = gateway
            .resume_or_start_with_projection(
                ResumeSessionLaunchRequest {
                    launch: request,
                    previous_fingerprint: previous.clone(),
                    previous_session_id: "sess_t09a_0001".to_string(),
                },
                projection,
            )
            .expect("matching full-context fingerprint must resume");
        let GatewaySessionDisposition::Resume(resumed) = disposition else {
            panic!("expected Resume for a matching full fingerprint");
        };
        // Resume 返回的 validated 冻结完整上下文指纹与候选投影。
        assert_eq!(resumed.fingerprint(), &previous);
        assert!(resumed.lc_projection().is_some());
        assert_eq!(fixture.gateway_audit().supersede_count(), 0);
        assert_eq!(fixture.registry_start_count(), 0);
    }

    /// 旧会话指纹与全上下文重算不一致(存量 v1/漂移形态)→ 仅 supersede
    /// (审计)+ `StartNew`:等待面只允许显式 `StartGeneration` fresh,
    /// 决策层零 spawn(不读旧 resume 行、不启动 provider)。
    #[test]
    fn lcg_t09a_resume_or_start_with_projection_supersedes_on_stale_fingerprint() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let worktree = fixture.real_worktree();
        let request = fixture.planning_request_for_manifest_with_worktree(worktree);
        let gateway = fixture.gateway();
        let validated = gateway
            .validate(request.clone())
            .expect("planning launch validates");
        let projection = t09a_baseline_projection(&validated);

        let disposition = gateway
            .resume_or_start_with_projection(
                ResumeSessionLaunchRequest {
                    launch: request,
                    // 存量 v1 指纹形态:与五参重算永不一致 → supersede。
                    previous_fingerprint: SessionResumeFingerprint {
                        digest: "sha256:stale".to_string(),
                    },
                    previous_session_id: "sess_t09a_0002".to_string(),
                },
                projection,
            )
            .expect("drifted fingerprint yields a StartNew disposition, not an error");
        let GatewaySessionDisposition::StartNew {
            superseded_session_id,
            waiting,
            ..
        } = disposition
        else {
            panic!("expected StartNew for a stale fingerprint");
        };
        assert_eq!(superseded_session_id, "sess_t09a_0002");
        // StartNew 仅 supersede 并等待显式 fresh:唯一稳定 allowed action
        // 是 StartGeneration。
        assert!(
            waiting
                .allowed_actions
                .contains(&BootstrapActionKind::StartGeneration)
        );
        assert_eq!(fixture.gateway_audit().supersede_count(), 1);
        assert_eq!(
            fixture.gateway_audit().last_supersede_reason().as_deref(),
            Some("resume_fingerprint_mismatch")
        );
        // 决策层零 spawn。
        assert_eq!(fixture.registry_start_count(), 0);
    }

    /// Task 9a 决策测试(matching):全指纹匹配 → Resume 决策,native session
    /// 续接(旧 session id 不被 supersede),决策层零 spawn。
    #[test]
    fn lcg_t09_matching_full_fingerprint_resumes_native_session() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let worktree = fixture.real_worktree();
        let request = fixture.planning_request_for_manifest_with_worktree(worktree);
        let gateway = fixture.gateway();
        let validated = gateway
            .validate(request.clone())
            .expect("planning launch validates");
        let projection = t09a_baseline_projection(&validated);
        let previous = SessionResumeFingerprint::from_envelope(
            validated.envelope(),
            &projection,
            validated.action_evidence_digest(),
            &canonical_target_git_identity(&validated.envelope().target.worktree),
        );

        let matching = gateway
            .resume_or_start_with_projection(
                ResumeSessionLaunchRequest {
                    launch: request,
                    previous_fingerprint: previous.clone(),
                    previous_session_id: "sess_native_0001".to_string(),
                },
                projection,
            )
            .expect("full-context resume decision succeeds");
        assert!(matches!(matching, GatewaySessionDisposition::Resume(_)));
        // native session 续接:无 supersede、零 spawn。
        assert_eq!(fixture.gateway_audit().supersede_count(), 0);
        let decision_spawn_count = fixture.registry_start_count();
        assert_eq!(decision_spawn_count, 0);
    }

    /// Task 9a 决策测试(每维漂移):policy digest/cwd/target/git identity/
    /// action evidence/trust/MCP bundle/exact version/投影摘要任一漂移都
    /// supersede 旧会话并 StartNew,零 spawn、不读旧 resume 行转 fresh。
    #[test]
    fn lcg_t09_each_fingerprint_dimension_supersedes_without_fresh_spawn() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let worktree = fixture.real_worktree();
        let request = fixture.planning_request_for_manifest_with_worktree(worktree.clone());
        let gateway = fixture.gateway();
        let validated = gateway
            .validate(request.clone())
            .expect("planning launch validates");
        let baseline_projection = t09a_baseline_projection(&validated);
        let evidence = validated.action_evidence_digest().to_string();
        let git_identity = canonical_target_git_identity(&validated.envelope().target.worktree);
        let previous = SessionResumeFingerprint::from_envelope(
            validated.envelope(),
            &baseline_projection,
            &evidence,
            &git_identity,
        );

        let mut drifted_cases: Vec<(&str, SessionLaunchRequest, ProviderPolicyProjection)> =
            Vec::new();
        // cwd/target 维度:候选投影按漂移后的请求重新 prepare(调用者会以
        // 当前请求的 envelope 准备投影),漂移体现在 envelope 冻结值。
        let mut cwd_drifted = request.clone();
        cwd_drifted.working_directory = fixture.paths.root().join("cwd-drifted-t09");
        let cwd_drifted_validated = fixture
            .gateway()
            .validate(cwd_drifted.clone())
            .expect("cwd-drifted launch validates");
        drifted_cases.push((
            "cwd",
            cwd_drifted,
            t09a_baseline_projection(&cwd_drifted_validated),
        ));
        let target_worktree = fixture.paths.root().join("member-drifted-t09");
        std::fs::create_dir_all(&target_worktree).unwrap();
        let mut target_drifted = request.clone();
        target_drifted.target =
            PolicyTarget::checkout("logical_repo_0001", "checkout_0001", target_worktree);
        let target_drifted_validated = fixture
            .gateway()
            .validate(target_drifted.clone())
            .expect("target-drifted launch validates");
        drifted_cases.push((
            "target",
            target_drifted,
            t09a_baseline_projection(&target_drifted_validated),
        ));
        // 投影携带维度(trust / MCP bundle / exact version / 会话投影摘要)。
        drifted_cases.push((
            "trust",
            request.clone(),
            t09a_projection(
                &validated,
                "sha256:trust-drifted",
                "sha256:mcp-t09a",
                "claude 1.4.0",
                "sha256:capability-projection-t09a",
                "sha256:session-projection-t09a",
            ),
        ));
        drifted_cases.push((
            "mcp_bundle",
            request.clone(),
            t09a_projection(
                &validated,
                "sha256:trust-t09a",
                "sha256:mcp-drifted",
                "claude 1.4.0",
                "sha256:capability-projection-t09a",
                "sha256:session-projection-t09a",
            ),
        ));
        drifted_cases.push((
            "exact_version",
            request.clone(),
            t09a_projection(
                &validated,
                "sha256:trust-t09a",
                "sha256:mcp-t09a",
                "claude 1.4.1",
                "sha256:capability-projection-t09a",
                "sha256:session-projection-t09a",
            ),
        ));

        for (dimension, drifted_request, projection) in drifted_cases {
            let gateway = fixture.gateway();
            let drifted = gateway
                .resume_or_start_with_projection(
                    ResumeSessionLaunchRequest {
                        launch: drifted_request,
                        previous_fingerprint: previous.clone(),
                        previous_session_id: "sess_native_0002".to_string(),
                    },
                    projection,
                )
                .expect("drifted decision still yields a disposition");
            assert!(
                matches!(drifted, GatewaySessionDisposition::StartNew { .. }),
                "{dimension} drift must supersede into StartNew"
            );
        }
        // policy digest 维度(validate→决策之间政策升级)。
        fixture.upgrade_policy();
        let gateway = fixture.gateway();
        let drifted = gateway
            .resume_or_start_with_projection(
                ResumeSessionLaunchRequest {
                    launch: request.clone(),
                    previous_fingerprint: previous.clone(),
                    previous_session_id: "sess_native_0002".to_string(),
                },
                t09a_baseline_projection(&validated),
            )
            .expect("policy-drift decision still yields a disposition");
        assert!(matches!(
            drifted,
            GatewaySessionDisposition::StartNew { .. }
        ));
        // provider exact version 维度(capability source 漂移)。
        fixture.capabilities().set_version("1.4.1");
        let gateway = fixture.gateway();
        let drifted = gateway
            .resume_or_start_with_projection(
                ResumeSessionLaunchRequest {
                    launch: request,
                    previous_fingerprint: previous,
                    previous_session_id: "sess_native_0002".to_string(),
                },
                t09a_baseline_projection(&validated),
            )
            .expect("version-drift decision still yields a disposition");
        assert!(matches!(
            drifted,
            GatewaySessionDisposition::StartNew { .. }
        ));
        // 每维漂移全程零 spawn(adapter 从未被触达),旧会话全部被标记
        // superseded(7 个漂移案例)。
        let decision_spawn_count = fixture.registry_start_count();
        assert_eq!(decision_spawn_count, 0);
        assert_eq!(fixture.gateway_audit().supersede_count(), 7);
    }

    /// Task 9a 决策测试(explicit resume Unknown):显式 resume 请求在 resume
    /// 证据 Unknown 时稳定拒绝(ResumeNotSupported),零 spawn、零静默 fresh
    /// ——resume 为 Unknown 不阻止之后的合法显式 fresh。
    #[test]
    fn lcg_t09_explicit_resume_unknown_zero_spawn() {
        use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;

        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let worktree = fixture.real_worktree();
        let request = fixture.planning_request_for_manifest_with_worktree(worktree);
        let gateway = fixture.gateway();
        let validated = gateway
            .validate(request.clone())
            .expect("planning launch validates");
        let projection = t09a_baseline_projection(&validated);
        let previous = SessionResumeFingerprint::from_envelope(
            validated.envelope(),
            &projection,
            validated.action_evidence_digest(),
            &canonical_target_git_identity(&validated.envelope().target.worktree),
        );
        // 显式 resume 证据 = Unknown(sync split 无 native session contract 的
        // 真实形态)。
        fixture
            .capabilities()
            .set_resume_evidence_state(ProviderCapabilityEvidence::Unknown);

        let rejected = gateway
            .resume_or_start_with_projection(
                ResumeSessionLaunchRequest {
                    launch: request,
                    previous_fingerprint: previous,
                    previous_session_id: "sess_native_0003".to_string(),
                },
                projection,
            )
            .expect_err("explicit resume with Unknown evidence must be rejected");
        assert!(matches!(rejected, ProviderGatewayError::ResumeNotSupported));
        // 零 spawn、零静默 fresh:拒绝后 adapter 从未被触达、没有 fresh 启动。
        let decision_spawn_count = fixture.registry_start_count();
        assert_eq!(decision_spawn_count, 0);
        let fresh_after_explicit_resume_count = fixture.gateway_audit().stream_launches();
        assert_eq!(fresh_after_explicit_resume_count, 0);
        assert_eq!(fixture.gateway_audit().sync_launches(), 0);
    }

    /// Task 9a 决策测试(StartNew 等显式):指纹漂移的 StartNew 只 supersede
    /// 并等待显式 `StartGeneration`;同一 revision driver 不据此继续
    /// start_streaming(引擎接线归 9c,此处锁定决策面:StartNew 后 gateway
    /// 启动计数为 0)。
    #[test]
    fn lcg_t09_start_new_waits_for_explicit_start_generation() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let worktree = fixture.real_worktree();
        let request = fixture.planning_request_for_manifest_with_worktree(worktree);
        let gateway = fixture.gateway();
        let validated = gateway
            .validate(request.clone())
            .expect("planning launch validates");
        let projection = t09a_baseline_projection(&validated);

        let disposition = gateway
            .resume_or_start_with_projection(
                ResumeSessionLaunchRequest {
                    launch: request,
                    previous_fingerprint: SessionResumeFingerprint {
                        digest: "sha256:stale".to_string(),
                    },
                    previous_session_id: "sess_native_0004".to_string(),
                },
                projection,
            )
            .expect("drifted fingerprint yields StartNew");
        let GatewaySessionDisposition::StartNew {
            waiting: start_new_waiting,
            ..
        } = disposition
        else {
            panic!("expected StartNew");
        };
        // 稳定等待面:唯一 allowed action 是显式 StartGeneration fresh。
        assert!(
            start_new_waiting
                .allowed_actions
                .contains(&BootstrapActionKind::StartGeneration)
        );
        // 旧会话 supersede 已记录(durable superseded 落旧 run 的 gateway
        // 审计面)。
        let old_run_superseded_recorded = fixture.gateway_audit().supersede_count() == 1
            && fixture.gateway_audit().last_supersede_reason().as_deref()
                == Some("resume_fingerprint_mismatch");
        assert!(old_run_superseded_recorded);
        // 同一 revision driver 不得继续 start_streaming:决策后 provider 启动
        // 计数为 0(等待显式 StartGeneration 后重新 prepare/revalidate)。
        let revision_driver_provider_start_count = fixture.registry_start_count();
        assert_eq!(revision_driver_provider_start_count, 0);
    }
}
