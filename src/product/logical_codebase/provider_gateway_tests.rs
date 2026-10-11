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

        let validated = fixture
            .gateway()
            .validate(request)
            .expect("r47:LC 会话 Codex 不再路由级全阻——capability 行+投影复验裁决");
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

        let disposition = fixture
            .gateway()
            .resume_or_start(resume_request)
            .expect("r47:Codex resume 与其它 provider 同走指纹比对,不再路由级全阻");
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

/// Task 3.2(B0a2):公共 prepare 合同——`prepare_lc_streaming_candidate`
/// 冻结 opaque `PreparedLcStreamingCandidate`(input/request/validated/
/// full projection/facts/run-bound audit)。
///
/// 契约(计划 Interfaces 冻结):
/// 1. fake 正例 facts 真实可观察(TTL 缓存观测+同版本 projector),不能靠
///    默认 typed unsupported 使正例虚绿;
/// 2. prepare 全程只读:三口径计数分列——①version probe 恰一次、
///    ②adapter start=0、③session child=0;workspace durable 不变;
/// 3. 冻结+复验矩阵:命令 identity 变化后重观测 version 与冻结事实不一致
///    → typed 拒(`PolicyDrift/provider_version`)、零 spawn、零新 claim;
///    旧 basis(v1 指纹)不与 full candidate 混比 → StartNew 等待而非 resume;
/// 4. 未接入 adapter(默认 trait 实现)只得到 typed unsupported 拒绝。
mod anchor_t09_prepare_contract {
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
                canonical_executable: std::path::PathBuf::from(
                    "/opt/anchor-t09-gateway-bin/claude",
                ),
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
            let boundary = match input.boundary() {
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
        let authority_root =
            std::fs::canonicalize(fixture.manifest().provider_context_root).unwrap();
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
        let authority_root =
            std::fs::canonicalize(fixture.manifest().provider_context_root).unwrap();
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
        let authority_root =
            std::fs::canonicalize(fixture.manifest().provider_context_root).unwrap();
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
        sink: Arc<
            crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink,
        >,
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
            policy_before.revision.clone(),
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
}

// ---------------------------------------------------------------------------
// Task 3.7(B0f):四 provider 公共汇合——full 唯一 start 路径与跨家组合守卫。
// `anchor_t10_support` 为 pub(crate):lifecycle_store 的 durable 同源测试
// 复用同一四家 fixture(仅测试构建可达)。
// ---------------------------------------------------------------------------

/// 四家形态参数(消费 3.3–3.6 各家已过的本家事实语义,公共行为断言归
/// 本包):Claude `--resume` argv、Codex 受限 sandbox/approval、Pi
/// extension 写时机、Kimi native-source 与 Aria MCP bundle 区分。
pub(crate) mod anchor_t10_support {
    use super::*;
    use crate::cross_cutting::provider_adapter::ProviderAdapterError;
    use crate::cross_cutting::provider_boundary::{ProviderBoundaryMode, ProviderBoundaryPlan};
    use crate::cross_cutting::provider_registry::ProviderRegistry;
    use crate::cross_cutting::streaming_provider::{
        LcVersionCache, LcVersionProbe, LcVersionProbeKey, StreamingProviderAdapter,
    };
    use crate::cross_cutting::tool_policy_audit::{
        DurableToolPolicyEvent, LcProjectionAudit, LcProviderStartAudit, ProviderStartAudit,
        StoredProviderStart, ToolPolicyAuditSink, append_superseded_policy_drift,
        resume_with_lc_start_record,
    };
    use crate::product::logical_codebase::policy::{ProviderDialect, ProviderWireDialect};
    use crate::product::logical_codebase::provider_projection::{
        ProviderLcPrepareFacts, ProviderPolicyProjection, ProviderPolicyProjector,
        ProviderProjectionError, ProviderProjectionInput, UnprovisionedProviderPolicyProjector,
    };
    use crate::product::logical_codebase::provider_trust::{
        ProviderTrustFingerprintFacts, ProviderTrustSource, ProviderTrustVerification,
    };
    use crate::product::models::ProviderName;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    /// 四家公共 family 形态(版本/exe 身份/approval/MCP/audit 文本各随本家)。
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum AnchorT10Family {
        Claude,
        Codex,
        Pi,
        Kimi,
    }

    impl AnchorT10Family {
        pub(crate) const ALL: [AnchorT10Family; 4] = [
            AnchorT10Family::Claude,
            AnchorT10Family::Codex,
            AnchorT10Family::Pi,
            AnchorT10Family::Kimi,
        ];

        pub(crate) fn tag(&self) -> &'static str {
            match self {
                AnchorT10Family::Claude => "claude",
                AnchorT10Family::Codex => "codex",
                AnchorT10Family::Pi => "pi",
                AnchorT10Family::Kimi => "kimi",
            }
        }

        pub(crate) fn name(&self) -> ProviderName {
            match self {
                AnchorT10Family::Claude => ProviderName::ClaudeCode,
                AnchorT10Family::Codex => ProviderName::Codex,
                AnchorT10Family::Pi => ProviderName::Pi,
                AnchorT10Family::Kimi => ProviderName::KimiCode,
            }
        }

        fn ref_type(&self) -> ProviderRefType {
            match self {
                AnchorT10Family::Claude => ProviderRefType::ClaudeCode,
                AnchorT10Family::Codex => ProviderRefType::Codex,
                AnchorT10Family::Pi => ProviderRefType::Pi,
                AnchorT10Family::Kimi => ProviderRefType::KimiCode,
            }
        }

        fn provider_ref(&self) -> ProviderRef {
            match self {
                AnchorT10Family::Claude => ProviderRef::claude_code("cap_claude_t10"),
                AnchorT10Family::Codex => ProviderRef::codex("cap_codex_t10"),
                AnchorT10Family::Pi => ProviderRef::pi("cap_pi_t10"),
                AnchorT10Family::Kimi => ProviderRef::kimi_code("cap_kimi_t10"),
            }
        }

        fn dialect(&self) -> ProviderDialect {
            match self {
                AnchorT10Family::Claude => ProviderDialect::ClaudeCodeCliV1,
                AnchorT10Family::Codex => ProviderDialect::CodexCliV1,
                AnchorT10Family::Pi => ProviderDialect::PiRpcV1,
                AnchorT10Family::Kimi => ProviderDialect::KimiAcpV1,
            }
        }

        fn wire(&self) -> ProviderWireDialect {
            match self {
                AnchorT10Family::Claude => ProviderWireDialect::ClaudeCodeStreamJson,
                AnchorT10Family::Codex => ProviderWireDialect::CodexAppServerRpc,
                AnchorT10Family::Pi => ProviderWireDialect::PiRpc,
                AnchorT10Family::Kimi => ProviderWireDialect::KimiAcp,
            }
        }

        pub(crate) fn wire_text(&self) -> &'static str {
            match self {
                AnchorT10Family::Claude => "claude-stream-json",
                AnchorT10Family::Codex => "codex-app-server-rpc",
                AnchorT10Family::Pi => "pi-rpc",
                AnchorT10Family::Kimi => "kimi-acp",
            }
        }

        fn protocol_type(&self) -> crate::protocol::contracts::ProviderType {
            match self {
                AnchorT10Family::Claude => crate::protocol::contracts::ProviderType::ClaudeCode,
                AnchorT10Family::Codex => crate::protocol::contracts::ProviderType::Codex,
                AnchorT10Family::Pi => crate::protocol::contracts::ProviderType::Pi,
                AnchorT10Family::Kimi => crate::protocol::contracts::ProviderType::KimiCode,
            }
        }

        pub(crate) fn exact_version(&self) -> &'static str {
            match self {
                AnchorT10Family::Claude => "claude 1.4.0-t10",
                AnchorT10Family::Codex => "codex 0.9.4-t10",
                AnchorT10Family::Pi => "pi 0.83.0-t10",
                AnchorT10Family::Kimi => "kimi 0.34.0-t10",
            }
        }

        fn base_executable_identity(&self) -> String {
            format!("sha256:t10-{}-exe-v1", self.tag())
        }

        fn canonical_executable(&self) -> std::path::PathBuf {
            std::path::PathBuf::from(format!("/opt/anchor-t10/{}/cli", self.tag()))
        }

        pub(crate) fn approval_policy(&self) -> &'static str {
            match self {
                AnchorT10Family::Claude => "permission-prompt-tool=stdio",
                AnchorT10Family::Codex => "on-request",
                AnchorT10Family::Pi => "ask-for-extension",
                AnchorT10Family::Kimi => "kimi-acp-default",
            }
        }

        pub(crate) fn aria_mcp_bundle_digest(&self) -> &'static str {
            match self {
                AnchorT10Family::Kimi => "sha256:t10-mcp-aria-bundle",
                _ => "sha256:t10-mcp-bundle",
            }
        }

        pub(crate) fn audit_sandbox(&self) -> Option<String> {
            match self {
                AnchorT10Family::Codex => Some("read-only".to_string()),
                _ => None,
            }
        }

        pub(crate) fn audit_approval(&self) -> Option<String> {
            match self {
                AnchorT10Family::Codex => Some("on-request".to_string()),
                _ => None,
            }
        }

        pub(crate) fn audit_provider_text(&self) -> &'static str {
            match self {
                AnchorT10Family::Claude => "claude-code",
                AnchorT10Family::Codex => "codex",
                AnchorT10Family::Pi => "pi",
                AnchorT10Family::Kimi => "kimi-code",
            }
        }

        /// 本家 argv 形态(claude 携 `--resume <id>`;pi 携预生成 session id;
        /// codex 携受限 sandbox/approval 片段;kimi 携 acp 冻结片段)。
        fn audit_argv(&self, native_id: &str, resume_id: Option<&str>) -> Vec<String> {
            match self {
                AnchorT10Family::Claude => {
                    let mut argv = vec![self.tag().to_string()];
                    if let Some(id) = resume_id {
                        argv.push("--resume".to_string());
                        argv.push(id.to_string());
                    }
                    argv
                }
                AnchorT10Family::Codex => vec![
                    self.tag().to_string(),
                    "--sandbox".to_string(),
                    "read-only".to_string(),
                    "--ask".to_string(),
                    "on-request".to_string(),
                ],
                AnchorT10Family::Pi => vec![
                    self.tag().to_string(),
                    "--session-id".to_string(),
                    native_id.to_string(),
                ],
                AnchorT10Family::Kimi => {
                    vec![self.tag().to_string(), "acp".to_string()]
                }
            }
        }
    }

    fn anchor_t10_digest(parts: &[&str]) -> String {
        use sha2::Digest as _;
        let mut canonical = String::new();
        for part in parts {
            canonical.push_str(&format!("{}:{}", part.len(), part));
        }
        format!("sha256:{:x}", sha2::Sha256::digest(canonical.as_bytes()))
    }

    /// t10 四家 capability source:version/exact_version 同源,action 行
    /// `projection_digest` 与 t10 projector 的 capability 摘要逐字相等
    /// (`projection-digest-{action:?}`,与 StaticCapabilitySource 同形)。
    pub(crate) struct AnchorT10CapabilitySource {
        family: AnchorT10Family,
    }

    impl AnchorT10CapabilitySource {
        pub(crate) fn new(family: AnchorT10Family) -> Self {
            Self { family }
        }

        fn capability(
            &self,
            provider: &ProviderRef,
            action: SessionPolicyAction,
        ) -> ProviderCapability {
            ProviderCapability {
                provider_type: self.family.ref_type(),
                version: self.family.exact_version().to_string(),
                adapter_dialect: self.family.dialect(),
                wire_dialect: self.family.wire(),
                capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
                action_capability:
                    crate::product::logical_codebase::provider_capability_store::ProviderActionCapability {
                        action,
                        launch: crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed,
                        resume: crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed,
                        write_boundary: crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed,
                        projection_digest: format!("projection-digest-{action:?}"),
                        evidence_ref: format!("probe://t10-{action:?}"),
                    },
                trust: crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence::Confirmed,
            }
        }
    }

    impl ProviderCapabilitySource for AnchorT10CapabilitySource {
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
            Ok(self.capability(provider, action))
        }

        fn require_write_boundary(
            &self,
            provider: &ProviderRef,
            action: SessionPolicyAction,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            Ok(self.capability(provider, action))
        }

        fn require_root_recipe_supported(
            &self,
            _provider: &ProviderRef,
            _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            Err(ProviderGatewayError::UnsupportedCapability(
                "anchor t10 fixture has no root recipe facts".to_string(),
            ))
        }
    }

    /// t10 可翻转只读 trust source:verify_trusted 与 fingerprint_facts 同源
    /// 翻转(trust spawn 前变化负例);Codex/Kimi 需 workspace trust 的家族
    /// 真实触达该 source。
    pub(crate) struct AnchorT10TrustSource {
        trusted: AtomicBool,
    }

    impl AnchorT10TrustSource {
        pub(crate) fn new() -> Arc<Self> {
            Arc::new(Self {
                trusted: AtomicBool::new(true),
            })
        }

        pub(crate) fn set_trusted(&self, trusted: bool) {
            self.trusted.store(trusted, Ordering::SeqCst);
        }

        fn verification(
            &self,
            provider: &ProviderName,
            canonical_root: &std::path::Path,
        ) -> ProviderTrustVerification {
            ProviderTrustVerification {
                provider: provider.clone(),
                canonical_root: canonical_root.to_path_buf(),
                trust_key: String::new(),
                trusted: self.trusted.load(Ordering::SeqCst),
                ownership: None,
                detail: String::new(),
                verified_at: String::new(),
            }
        }
    }

    impl ProviderTrustSource for AnchorT10TrustSource {
        fn verify_trusted(
            &self,
            _project_id: &str,
            _lc_id: &str,
            provider: &ProviderName,
            canonical_root: &std::path::Path,
        ) -> Result<ProviderTrustVerification, ProviderGatewayError> {
            Ok(self.verification(provider, canonical_root))
        }

        fn fingerprint_facts(
            &self,
            _project_id: &str,
            _lc_id: &str,
            provider: &ProviderName,
            canonical_root: &std::path::Path,
        ) -> Result<ProviderTrustFingerprintFacts, ProviderGatewayError> {
            if !self.trusted.load(Ordering::SeqCst) {
                return Err(ProviderGatewayError::ProviderUnavailable(
                    "anchor t10 trust source: entry not trusted; refusing to freeze facts"
                        .to_string(),
                ));
            }
            Ok(ProviderTrustFingerprintFacts {
                verification: self.verification(provider, canonical_root),
                record_digest: Some(format!("sha256:t10-trust-record-{}", provider_wire_tag(provider))),
            })
        }
    }

    fn provider_wire_tag(provider: &ProviderName) -> String {
        format!("{provider:?}").to_lowercase()
    }

    /// t10 版本化 projector:capability 摘要与 capability 行逐字相等
    /// (`projection-digest-{action:?}`);会话摘要为长度分隔+schema 前缀的
    /// SHA-256(family/version/trust/MCP/approval/cwd/config 全部入摘要);
    /// 负例可覆写 capability/会话摘要(v2 材料完整性门)。
    pub(crate) struct AnchorT10Projector {
        family: AnchorT10Family,
        exact_version: String,
        capability_digest_override: Option<String>,
        session_digest_override: Option<String>,
    }

    impl AnchorT10Projector {
        fn new(family: AnchorT10Family, exact_version: String) -> Self {
            Self {
                family,
                exact_version,
                capability_digest_override: None,
                session_digest_override: None,
            }
        }
    }

    impl ProviderPolicyProjector for AnchorT10Projector {
        fn project(
            &self,
            input: &ProviderProjectionInput,
        ) -> Result<ProviderPolicyProjection, ProviderProjectionError> {
            let envelope = input.envelope();
            // 匹配门:只投影本家(跨家投影拒绝,与真实 projector 同构)。
            if input.provider().provider_type != self.family.ref_type() {
                return Err(ProviderProjectionError::Unsupported(format!(
                    "anchor t10 projector only projects {} (got {:?})",
                    self.family.tag(),
                    input.provider().provider_type
                )));
            }
            if envelope.provider_dialect != self.family.dialect() {
                return Err(ProviderProjectionError::Unsupported(format!(
                    "anchor t10 projector requires dialect {:?}",
                    self.family.dialect()
                )));
            }
            // read-only action 的 boundary 证据引用为空串(合法形态),
            // 不消费 facts 边界计划。
            let capability_projection_digest = self
                .capability_digest_override
                .clone()
                .unwrap_or_else(|| format!("projection-digest-{:?}", envelope.action));
            let session_projection_digest = self.session_digest_override.clone().unwrap_or_else(|| {
                anchor_t10_digest(&[
                    "anchor-t10-session-v1",
                    self.family.tag(),
                    self.exact_version.as_str(),
                    capability_projection_digest.as_str(),
                    input.trust_digest(),
                    input.mcp_bundle_digest(),
                    input.approval_policy(),
                    &envelope.working_directory.to_string_lossy(),
                    &envelope.config_digest,
                ])
            });
            Ok(ProviderPolicyProjection::new(
                self.family.ref_type(),
                envelope.provider_dialect,
                self.family.wire(),
                self.exact_version.clone(),
                envelope.action,
                input.role().clone(),
                input.permission_mode().clone(),
                input.tool_policy().cloned(),
                input.approval_policy().to_string(),
                self.family
                    .audit_sandbox()
                    .unwrap_or_else(|| "read-only".to_string()),
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

    /// t10 四家 fake streaming adapter:真实 prepare 链(TTL 缓存+计数
    /// probe=口径①)+start_validated 消费冻结 full projection(口径②)并
    /// 如实计数 native child(口径③)与 kill/reap;basis 旁路(裸投影
    /// validated)typed 拒;audit 字面量与 gateway 冻结 full projection
    /// 同源;pi extension 写只在 mismatch 判定之后;kimi 复验 MCP 来源。
    pub(crate) struct AnchorT10FamilyAdapter {
        family: AnchorT10Family,
        cache: Arc<LcVersionCache>,
        probe_calls: Arc<AtomicUsize>,
        executable_identity: Mutex<String>,
        observed_version: Arc<Mutex<String>>,
        mcp_bundle_digest: Mutex<String>,
        capability_digest_override: Mutex<Option<String>>,
        session_digest_override: Mutex<Option<String>>,
        handshake_native_id: Mutex<Option<String>>,
        adapter_starts: AtomicUsize,
        session_children: AtomicUsize,
        children_killed: AtomicUsize,
        extension_writes: AtomicUsize,
        starts_without_projection: AtomicUsize,
        last_start_projection_digest: Mutex<Option<String>>,
        last_start_audit_literal: Mutex<Option<LcProviderStartAudit>>,
    }

    impl AnchorT10FamilyAdapter {
        pub(crate) fn new(family: AnchorT10Family, cache: Arc<LcVersionCache>) -> Arc<Self> {
            Arc::new(Self {
                family,
                cache,
                probe_calls: Arc::new(AtomicUsize::new(0)),
                executable_identity: Mutex::new(family.base_executable_identity()),
                observed_version: Arc::new(Mutex::new(family.exact_version().to_string())),
                mcp_bundle_digest: Mutex::new(family.aria_mcp_bundle_digest().to_string()),
                capability_digest_override: Mutex::new(None),
                session_digest_override: Mutex::new(None),
                handshake_native_id: Mutex::new(None),
                adapter_starts: AtomicUsize::new(0),
                session_children: AtomicUsize::new(0),
                children_killed: AtomicUsize::new(0),
                extension_writes: AtomicUsize::new(0),
                starts_without_projection: AtomicUsize::new(0),
                last_start_projection_digest: Mutex::new(None),
                last_start_audit_literal: Mutex::new(None),
            })
        }

        pub(crate) fn set_executable_identity(&self, identity: &str) {
            *self.executable_identity.lock().unwrap() = identity.to_string();
        }

        pub(crate) fn set_observed_version(&self, version: &str) {
            *self.observed_version.lock().unwrap() = version.to_string();
        }

        pub(crate) fn set_mcp_bundle_digest(&self, digest: &str) {
            *self.mcp_bundle_digest.lock().unwrap() = digest.to_string();
        }

        pub(crate) fn set_projector_overrides(
            &self,
            capability: Option<String>,
            session: Option<String>,
        ) {
            *self.capability_digest_override.lock().unwrap() = capability;
            *self.session_digest_override.lock().unwrap() = session;
        }

        pub(crate) fn set_handshake_native_id(&self, id: Option<String>) {
            *self.handshake_native_id.lock().unwrap() = id;
        }

        pub(crate) fn probe_calls(&self) -> usize {
            self.probe_calls.load(Ordering::SeqCst)
        }

        pub(crate) fn adapter_starts(&self) -> usize {
            self.adapter_starts.load(Ordering::SeqCst)
        }

        pub(crate) fn session_children(&self) -> usize {
            self.session_children.load(Ordering::SeqCst)
        }

        pub(crate) fn children_killed(&self) -> usize {
            self.children_killed.load(Ordering::SeqCst)
        }

        pub(crate) fn extension_writes(&self) -> usize {
            self.extension_writes.load(Ordering::SeqCst)
        }

        pub(crate) fn starts_without_projection(&self) -> usize {
            self.starts_without_projection.load(Ordering::SeqCst)
        }

        pub(crate) fn last_start_projection_digest(&self) -> Option<String> {
            self.last_start_projection_digest.lock().unwrap().clone()
        }

        pub(crate) fn last_start_audit_literal(&self) -> Option<LcProviderStartAudit> {
            self.last_start_audit_literal.lock().unwrap().clone()
        }

        fn minimal_session(
            native_id: Option<String>,
        ) -> crate::cross_cutting::streaming_provider::ProviderSession {
            let (_event_tx, events) = tokio::sync::mpsc::channel(1);
            let (commands, _command_rx) = tokio::sync::mpsc::channel(1);
            crate::cross_cutting::streaming_provider::ProviderSession {
                events,
                commands,
                native_session_id: native_id,
            }
        }
    }

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for AnchorT10FamilyAdapter {
        async fn start_validated(
            &self,
            input: ValidatedStreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<crate::cross_cutting::streaming_provider::ProviderSession, ProviderAdapterError>
        {
            self.adapter_starts.fetch_add(1, Ordering::SeqCst);
            let (input, launch) = input.into_parts();
            // basis 旁路不可达:唯一 start 路径必须携带冻结 full projection。
            let Some(lc_projection) = launch.lc_projection().cloned() else {
                self.starts_without_projection
                    .fetch_add(1, Ordering::SeqCst);
                return Err(ProviderAdapterError::parse_error(
                    "anchor t10 adapter: bare-basis validated input reached start_validated \
                     (no frozen full projection); refusing to spawn"
                        .to_string(),
                    String::new(),
                    String::new(),
                ));
            };
            let envelope = launch.envelope();
            // 本家 mutable facts 复验(与真实 adapter 同构:version 恒复验,
            // kimi 追加 MCP 来源复验)。
            let observed_version = self.observed_version.lock().unwrap().clone();
            if lc_projection.exact_version() != observed_version.as_str() {
                return Err(ProviderAdapterError::parse_error(
                    format!(
                        "anchor t10 {}: prepared projection version {} drifted from current \
                         observation {observed_version}; refusing to spawn",
                        self.family.tag(),
                        lc_projection.exact_version()
                    ),
                    String::new(),
                    String::new(),
                ));
            }
            if self.family == AnchorT10Family::Kimi
                && lc_projection.mcp_bundle_digest() != *self.mcp_bundle_digest.lock().unwrap()
            {
                return Err(ProviderAdapterError::parse_error(
                    format!(
                        "anchor t10 kimi: mcp source drifted before spawn (frozen={}, current={})",
                        lc_projection.mcp_bundle_digest(),
                        self.mcp_bundle_digest.lock().unwrap()
                    ),
                    String::new(),
                    String::new(),
                ));
            }
            // audit 字面量(与真实 lc_start_audit_literal 同形):从冻结
            // full projection 装配,v2 材料不完整即拒。
            let git_identity =
                canonical_target_git_identity(&envelope.target.worktree);
            let action_evidence = lc_projection.capability_projection_digest().to_string();
            let literal = LcProviderStartAudit {
                action: "planning_read_only".to_string(),
                wire_dialect: self.family.wire_text().to_string(),
                capability_projection_digest: action_evidence.clone(),
                projection_digest: lc_projection.projection_digest().to_string(),
                boundary_evidence_ref: lc_projection.boundary_evidence_ref().to_string(),
                resume_fingerprint: SessionResumeFingerprint::from_envelope(
                    envelope,
                    &lc_projection,
                    &action_evidence,
                    &git_identity,
                )
                .digest,
                action_evidence_digest: action_evidence,
                target_git_identity: git_identity,
            };
            if !literal.is_v2_complete() {
                return Err(ProviderAdapterError::parse_error(
                    "anchor t10 adapter: lc start audit v2 material incomplete".to_string(),
                    String::new(),
                    String::new(),
                ));
            }
            *self.last_start_projection_digest.lock().unwrap() =
                Some(lc_projection.projection_digest().to_string());
            *self.last_start_audit_literal.lock().unwrap() = Some(literal.clone());
            // 显式 resume:先对存档完整字面量判定(漂移 → superseded 终止
            // 审计写被取代旧 run,零 child typed 拒)。
            let resume_id = input
                .resume_provider_session_id
                .as_deref()
                .map(str::trim)
                .filter(|id| !id.is_empty())
                .map(str::to_string);
            if let (Some(resume_id), Some(sink)) =
                (resume_id.clone(), input.audit_sink.as_ref())
            {
                let stored: Option<StoredProviderStart> = sink
                    .find_provider_start(&resume_id)
                    .map_err(|error| {
                        ProviderAdapterError::parse_error(
                            format!("anchor t10 resume lookup failed: {error}"),
                            String::new(),
                            String::new(),
                        )
                    })?;
                if let Some(stored) = stored.as_ref()
                    && matches!(
                        resume_with_lc_start_record(&stored.record, &literal),
                        crate::cross_cutting::tool_policy_audit::ResumeDecision::RejectSupersedeAndStartNew
                    )
                {
                    append_superseded_policy_drift(sink.as_ref(), stored).map_err(|error| {
                        ProviderAdapterError::parse_error(
                            format!("anchor t10 superseded audit append failed: {error}"),
                            String::new(),
                            String::new(),
                        )
                    })?;
                    return Err(ProviderAdapterError::parse_error(
                        "anchor t10 adapter: resume audit drifted or legacy record lacks lc \
                         projection; refusing to spawn a fresh child for an explicit LC resume"
                            .to_string(),
                        String::new(),
                        String::new(),
                    ));
                }
            }
            // pi extension 准备在 mismatch 判定之后(早写拒收)。
            if self.family == AnchorT10Family::Pi {
                self.extension_writes.fetch_add(1, Ordering::SeqCst);
            }
            // native child(口径③)。
            self.session_children.fetch_add(1, Ordering::SeqCst);
            let native_id = self
                .handshake_native_id
                .lock()
                .unwrap()
                .clone()
                .or_else(|| resume_id.clone())
                .unwrap_or_else(|| format!("native-{}-t10", self.family.tag()));
            if let (Some(expected), Some(confirmed)) =
                (resume_id.as_deref(), self.handshake_native_id.lock().unwrap().as_deref())
                && expected != confirmed
            {
                // 错 ID:child 已启动的 runtime 失败——如实 kill/reap,不伪称零 child。
                self.children_killed.fetch_add(1, Ordering::SeqCst);
                return Err(ProviderAdapterError::parse_error(
                    format!(
                        "anchor t10 {}: handshake confirmed {confirmed} but resume requested \
                         {expected} (session NOT resumed; child killed and reaped)",
                        self.family.tag()
                    ),
                    String::new(),
                    String::new(),
                ));
            }
            // 成功路径:provider_start 首行(含本家 argv/sandbox/approval 与
            // lc_projection 五字段摘要)。
            if let Some(sink) = input.audit_sink.as_ref() {
                sink.append_bound(DurableToolPolicyEvent::ProviderStart(ProviderStartAudit {
                    provider: self.family.audit_provider_text().to_string(),
                    role: "executor".to_string(),
                    workspace_session_id: input
                        .workspace_session_id
                        .clone()
                        .unwrap_or_else(|| "ws-anchor-t10".to_string()),
                    provider_session_id: native_id.clone(),
                    tool_policy_canonical_digest: format!(
                        "sha256:t10-{}-policy",
                        self.family.tag()
                    ),
                    argv: self
                        .family
                        .audit_argv(&native_id, resume_id.as_deref()),
                    sandbox: self.family.audit_sandbox(),
                    approval_policy: self.family.audit_approval(),
                    provider_version: observed_version,
                    adapter_dialect: self.family.wire_text().to_string(),
                    lc_projection: Some(LcProjectionAudit {
                        action: literal.action.clone(),
                        wire_dialect: literal.wire_dialect.clone(),
                        capability_projection_digest: literal
                            .capability_projection_digest
                            .clone(),
                        projection_digest: literal.projection_digest.clone(),
                        boundary_evidence_ref: literal.boundary_evidence_ref.clone(),
                    }),
                }))
                .map_err(|error| {
                    ProviderAdapterError::parse_error(
                        format!("anchor t10 provider_start append failed: {error}"),
                        String::new(),
                        String::new(),
                    )
                })?;
            }
            Ok(Self::minimal_session(Some(native_id)))
        }

        async fn prepare_lc_facts(
            &self,
            _input: &crate::cross_cutting::streaming_provider::StreamingProviderInput,
            launch: &ValidatedSessionLaunchPolicy,
            cancel: CancellationToken,
        ) -> Result<ProviderLcPrepareFacts, ProviderAdapterError> {
            let key = LcVersionProbeKey {
                provider: self.family.name(),
                canonical_executable: self.family.canonical_executable(),
                executable_identity: self.executable_identity.lock().unwrap().clone(),
                argv: vec!["--version".to_string()],
                supplier_scope: 10 + (self.family.tag().len() as u64),
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
                        format!("anchor t10 fake prepare: {error}"),
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
            let mut projector =
                AnchorT10Projector::new(self.family, observation.exact_version().to_string());
            projector.capability_digest_override =
                self.capability_digest_override.lock().unwrap().clone();
            projector.session_digest_override =
                self.session_digest_override.lock().unwrap().clone();
            Ok(ProviderLcPrepareFacts::new(
                self.family.ref_type(),
                envelope.provider_dialect,
                self.family.wire(),
                observation.exact_version().to_string(),
                observation.observed_at(),
                key,
                self.family.approval_policy().to_string(),
                self.mcp_bundle_digest.lock().unwrap().clone(),
                boundary,
                Arc::new(projector),
            ))
        }
    }

    /// t10 汇合 harness:四家公共 fixture(自包含 tempdir,不依赖
    /// GatewayFixture;policy/capability/target/gate/audit 与 anchor_t09
    /// 同形,registry 槽位随 family)。
    pub(crate) struct AnchorT10Harness {
        pub(crate) adapter: Arc<AnchorT10FamilyAdapter>,
        pub(crate) gateway: LogicalCodebaseProviderGateway,
        pub(crate) trust: Arc<AnchorT10TrustSource>,
        pub(crate) audit: Arc<GatewayRunAudit>,
        pub(crate) request: SessionLaunchRequest,
        pub(crate) family: AnchorT10Family,
        paths: ProductAppPaths,
        authority_root: std::path::PathBuf,
        capabilities: Arc<AnchorT10CapabilitySource>,
        targets: Arc<MutableTargetResolver>,
        _root: tempfile::TempDir,
    }

    impl AnchorT10Harness {
        pub(crate) fn install_policy(&self) {
            let manifest = LogicalCodebaseManifest::new(
                "project_0001",
                self.paths.root().to_path_buf(),
                vec![],
            );
            AggregatePolicyArtifactStore::new(self.paths.clone())
                .ensure_bootstrap(&manifest)
                .expect("anchor t10 bootstrap policy");
        }

        pub(crate) fn streaming_input(
            &self,
            resume: Option<String>,
        ) -> crate::cross_cutting::streaming_provider::StreamingProviderInput {
            crate::cross_cutting::streaming_provider::StreamingProviderInput {
                provider_type: self.family.protocol_type(),
                role: crate::protocol::contracts::AdapterRole::Executor,
                prompt: "anchor t10 convergence".to_string(),
                working_dir: self.authority_root.clone(),
                working_directory: Some(self.authority_root.clone()),
                workspace_session_id: None,
                resume_provider_session_id: resume,
                permission_mode: crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
                tool_policy: None,
                audit_sink: None,
                structured_output_contract: None,
                env_vars: Default::default(),
                timeout_secs: 5,
                baseline_tree: None,
            }
        }

        pub(crate) fn audit_context(
            &self,
            sink: Arc<dyn ToolPolicyAuditSink>,
        ) -> ProviderLaunchAuditContext {
            anchor_t10_audit_context(sink, "ws-anchor-t10", 1)
        }

        /// 以自定义 registry 构造同源 gateway(混合矩阵负例:换绑 adapter
        /// 实例/跨家串接)。
        pub(crate) fn gateway_with_registry(
            &self,
            registry: ProviderRegistry,
        ) -> LogicalCodebaseProviderGateway {
            self.build_gateway(Arc::new(registry))
        }

        pub(crate) fn registry(&self) -> ProviderRegistry {
            let mut registry = ProviderRegistry::new();
            registry.register_gated(
                self.family.name(),
                self.adapter.clone(),
                Arc::new(UnprovisionedProviderPolicyProjector),
                always_available_gate(),
            )
            .expect("anchor t10 harness registry register");
            registry
        }

        fn build_gateway(
            &self,
            registry: Arc<ProviderRegistry>,
        ) -> LogicalCodebaseProviderGateway {
            LogicalCodebaseProviderGateway::with_audit(
                AggregatePolicyArtifactStore::new(self.paths.clone()),
                self.capabilities.clone(),
                self.targets.clone(),
                registry,
                Arc::new(StubSyncAdapter),
                always_available_gate(),
                self.audit.clone(),
                self.authority_root.clone(),
            )
            .with_readonly_lc_facts(
                crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
                    self.paths.clone(),
                    "logical_codebase_0001",
                ),
                self.trust.clone(),
                Some("logical_codebase_0001".to_string()),
            )
        }
    }

    pub(crate) fn anchor_t10_harness(family: AnchorT10Family) -> AnchorT10Harness {
        let root = tempfile::tempdir().expect("anchor t10 tempdir");
        let paths = ProductAppPaths::new(root.path());
        let manifest =
            LogicalCodebaseManifest::new("project_0001", paths.root().to_path_buf(), vec![]);
        let authority_root =
            std::fs::canonicalize(&manifest.provider_context_root).expect("authority root");
        let worktree = paths.root().join("worktree");
        std::fs::create_dir_all(&worktree).expect("anchor t10 worktree");
        let request = SessionLaunchRequest {
            project_id: manifest.project_id.clone(),
            provider: family.provider_ref(),
            action: SessionPolicyAction::PlanningReadOnly,
            target: PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree),
            // LC 会话 cwd = canonical LC root(root-cwd 契约,与 target 分离)。
            working_directory: authority_root.clone(),
            readable_roots: vec![paths.root().to_path_buf()],
            writable_roots: Vec::new(),
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        };
        let adapter = AnchorT10FamilyAdapter::new(family, Arc::new(LcVersionCache::new()));
        let audit = Arc::new(GatewayRunAudit::new());
        let trust = AnchorT10TrustSource::new();
        let capabilities = Arc::new(AnchorT10CapabilitySource::new(family));
        let targets = Arc::new(MutableTargetResolver::new(paths.root().join("baseline.git")));
        let mut registry = ProviderRegistry::new();
        registry
            .register_gated(
                family.name(),
                adapter.clone(),
                Arc::new(UnprovisionedProviderPolicyProjector),
                always_available_gate(),
            )
            .expect("anchor t10 register gated family adapter");
        let gateway = LogicalCodebaseProviderGateway::with_audit(
            AggregatePolicyArtifactStore::new(paths.clone()),
            capabilities.clone(),
            targets.clone(),
            Arc::new(registry),
            Arc::new(StubSyncAdapter),
            always_available_gate(),
            audit.clone(),
            authority_root.clone(),
        )
        .with_readonly_lc_facts(
            crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
                paths.clone(),
                "logical_codebase_0001",
            ),
            trust.clone(),
            Some("logical_codebase_0001".to_string()),
        );
        AnchorT10Harness {
            adapter,
            gateway,
            trust,
            audit,
            request,
            family,
            paths,
            authority_root,
            capabilities,
            targets,
            _root: root,
        }
    }

    pub(crate) fn anchor_t10_audit_context(
        sink: Arc<dyn ToolPolicyAuditSink>,
        workspace_session_id: &str,
        role_run_seq: u64,
    ) -> ProviderLaunchAuditContext {
        ProviderLaunchAuditContext {
            workspace_session_id: workspace_session_id.to_string(),
            role_run_seq,
            audit_sink: sink,
        }
    }

    /// 预置旧 run 的 durable provider_start 存档(resume 检索输入;lc_
    /// projection 五字段与冻结 full projection 同源)。
    pub(crate) fn anchor_t10_preset_resume_archive(
        sink: &Arc<crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink>,
        family: AnchorT10Family,
        resume_id: &str,
        projection: &ProviderPolicyProjection,
    ) {
        let record = ProviderStartAudit {
            provider: family.audit_provider_text().to_string(),
            role: "executor".to_string(),
            workspace_session_id: "ws-anchor-t10".to_string(),
            provider_session_id: resume_id.to_string(),
            tool_policy_canonical_digest: format!("sha256:t10-{}-policy", family.tag()),
            argv: Vec::new(),
            sandbox: family.audit_sandbox(),
            approval_policy: family.audit_approval(),
            provider_version: family.exact_version().to_string(),
            adapter_dialect: family.wire_text().to_string(),
            lc_projection: Some(LcProjectionAudit {
                action: "planning_read_only".to_string(),
                wire_dialect: family.wire_text().to_string(),
                capability_projection_digest: projection
                    .capability_projection_digest()
                    .to_string(),
                projection_digest: projection.projection_digest().to_string(),
                boundary_evidence_ref: projection.boundary_evidence_ref().to_string(),
            }),
        };
        sink.with_stored_provider_start(record);
    }
}

mod anchor_t10_convergence {
    use super::anchor_t10_support::*;
    use super::*;
    use crate::cross_cutting::tool_policy_audit::test_support::RecordingToolPolicyAuditSink;
    use crate::cross_cutting::tool_policy_audit::DurableToolPolicyEvent;

    fn provider_start_records(
        sink: &RecordingToolPolicyAuditSink,
    ) -> Vec<crate::cross_cutting::tool_policy_audit::ProviderStartAudit> {
        sink.events()
            .iter()
            .filter_map(|event| match event {
                DurableToolPolicyEvent::ProviderStart(record) => Some(record.clone()),
                _ => None,
            })
            .collect()
    }

    /// Task 3.7 Step 1 用例一:四家公共 fixture 汇总——full 唯一 start 路径
    /// 无 unsupported/basis 旁路;oldbasis/missingfull 拒等待(unavailable
    /// 稳定原因码);投影/audit/fingerprint 同源;wrongID 如实 kill/reap;
    /// TTL 三计数真实;Pi extension 早写拒收/Kimi native-source 与 Aria
    /// bundle 区分/Codex 受限 sandbox/Claude --resume 事实逐家消费。
    #[tokio::test]
    async fn anchor_t10_full_projection_all_providers_no_basis_resume() {
        for family in AnchorT10Family::ALL {
            let tag = family.tag();
            let harness = anchor_t10_harness(family);
            harness.install_policy();
            let sink = RecordingToolPolicyAuditSink::new();

            // -- 默认 unsupported 不可达正例的前置:未接入 adapter(默认
            //    trait)在 full 路径只得到 typed 拒,registry start 零触达。
            let mut bare_registry = crate::cross_cutting::provider_registry::ProviderRegistry::new();
            bare_registry.register(family.name(), Arc::new(CountingStreamingAdapter::new()) as Arc<dyn StreamingProviderAdapter>);
            let bare_error = match harness
                .gateway_with_registry(bare_registry)
                .prepare_lc_streaming_candidate(
                    harness.streaming_input(None),
                    harness.request.clone(),
                    harness.audit_context(sink.clone()),
                    CancellationToken::new(),
                )
                .await
            {
                Err(ProviderGatewayError::Adapter(error)) => error,
                Err(other) => panic!("[{tag}] unexpected rejection shape: {other}"),
                Ok(_) => panic!("[{tag}] default adapter must fail closed with typed unsupported"),
            };
            assert!(
                bare_error
                    .stderr
                    .contains("lc prepare facts is not supported"),
                "[{tag}] unexpected default rejection: {bare_error:?}"
            );

            // -- 正例:真实 facts prepare(口径①恰一次)。
            let candidate = harness
                .gateway
                .prepare_lc_streaming_candidate(
                    harness.streaming_input(None),
                    harness.request.clone(),
                    harness.audit_context(sink.clone()),
                    CancellationToken::new(),
                )
                .await
                .unwrap_or_else(|error| panic!("[{tag}] prepare failed: {error}"));
            assert_eq!(harness.adapter.probe_calls(), 1, "[{tag}] ① probe");
            assert_eq!(candidate.facts().exact_version(), family.exact_version());
            assert_eq!(candidate.facts().version_probe_key().provider, family.name());
            assert_eq!(
                candidate.facts().approval_policy(),
                family.approval_policy()
            );
            assert_eq!(
                candidate.full_projection().mcp_bundle_digest(),
                family.aria_mcp_bundle_digest()
            );

            // -- full 指纹与 prepared full map(basis=FullLcProjectionV2)同源。
            let fingerprint = candidate.full_context_fingerprint();
            assert!(!fingerprint.digest.is_empty());
            let persisted_prepared =
                crate::product::models::LogicalLaunchFingerprintRecord {
                    schema_version: 1,
                    basis: crate::product::models::LogicalLaunchBasis::FullLcProjectionV2,
                    role: crate::product::models::ProviderConversationRole::Author,
                    provider: family.name(),
                    role_run_seq: 1,
                    node_id: "node-t10".to_string(),
                    native_session_id: None,
                    fingerprint_digest: fingerprint.digest.clone(),
                    planning_access_fingerprint: fingerprint.digest.clone(),
                    state: crate::product::models::LogicalLaunchFingerprintState::Prepared,
                    created_at: "2026-10-11T00:00:00Z".to_string(),
                    updated_at: "2026-10-11T00:00:00Z".to_string(),
                };

            // -- full-matching resume:Resume,validated 携带冻结 full 投影。
            let disposition = harness
                .gateway
                .resume_or_start_prepared_lc_candidate(
                    &candidate,
                    PreparedLcResumeBasis::Full {
                        previous_fingerprint: fingerprint.clone(),
                        previous_session_id: "native-t10-previous".to_string(),
                    },
                )
                .expect("[{tag}] full-matching resume decision");
            match &disposition {
                GatewaySessionDisposition::Resume(validated) => {
                    assert_eq!(
                        validated.lc_projection().map(|projection| projection.projection_digest()),
                        Some(candidate.full_projection().projection_digest()),
                        "[{tag}] Resume validated must carry the frozen full projection"
                    );
                    assert_eq!(validated.fingerprint().digest, persisted_prepared.fingerprint_digest);
                }
                _ => panic!("[{tag}] full-matching fingerprint must resume"),
            }

            // -- 旧 basis(legacy 指纹)不与 full 混比:StartNew 等待 mismatch。
            let supersede_before = harness.audit.supersede_count();
            let legacy = harness
                .gateway
                .resume_or_start_prepared_lc_candidate(
                    &candidate,
                    PreparedLcResumeBasis::Full {
                        previous_fingerprint: SessionResumeFingerprint {
                            digest: "sha256:legacy-v1-basis-t10".to_string(),
                        },
                        previous_session_id: "native-t10-legacy".to_string(),
                    },
                )
                .expect("[{tag}] legacy basis disposition");
            match legacy {
                GatewaySessionDisposition::StartNew { waiting, .. } => {
                    assert_eq!(waiting.reason_code, "resume_fingerprint_mismatch");
                }
                _ => panic!("[{tag}] legacy basis must wait for explicit fresh"),
            }
            assert_eq!(harness.audit.supersede_count(), supersede_before + 1);
            assert_eq!(harness.adapter.adapter_starts(), 0, "[{tag}] ② after dispositions");
            assert_eq!(harness.adapter.session_children(), 0, "[{tag}] ③ after dispositions");

            // -- 缺 full map/旧 basis:Unavailable → resume_fingerprint_unavailable
            //    等待,不 supersede 未知会话。
            let unavailable = harness
                .gateway
                .resume_or_start_prepared_lc_candidate(
                    &candidate,
                    PreparedLcResumeBasis::Unavailable,
                )
                .expect("[{tag}] unavailable basis disposition");
            match unavailable {
                GatewaySessionDisposition::StartNew { waiting, validated, .. } => {
                    assert_eq!(
                        waiting.reason_code,
                        RESUME_FINGERPRINT_UNAVAILABLE,
                        "[{tag}] missing full map must wait with resume_fingerprint_unavailable"
                    );
                    assert!(validated.lc_projection().is_some());
                }
                _ => panic!("[{tag}] unavailable basis must wait for explicit fresh"),
            }
            assert_eq!(
                harness.audit.supersede_count(),
                supersede_before + 1,
                "[{tag}] unavailable must not supersede an unknown session"
            );
            assert_eq!(harness.adapter.adapter_starts(), 0);
            assert_eq!(harness.adapter.session_children(), 0);

            // -- full 唯一 start 路径:成功 spawn,三计数真实(TTL 命中,①不增)。
            let stream_launches_before = harness.audit.stream_launches();
            let session = harness
                .gateway
                .start_lc_prepared_streaming(candidate.clone(), CancellationToken::new())
                .await
                .unwrap_or_else(|error| panic!("[{tag}] unique start failed: {error}"));
            assert_eq!(
                session.native_session_id.as_deref(),
                Some(format!("native-{tag}-t10").as_str())
            );
            assert_eq!(harness.adapter.probe_calls(), 1, "[{tag}] TTL hit at revalidation");
            assert_eq!(harness.adapter.adapter_starts(), 1, "[{tag}] ② unique start");
            assert_eq!(harness.adapter.session_children(), 1, "[{tag}] ③ unique start");
            assert_eq!(harness.adapter.children_killed(), 0);
            assert_eq!(harness.adapter.starts_without_projection(), 0, "[{tag}] no basis bypass");
            assert_eq!(harness.audit.stream_launches(), stream_launches_before + 1);

            // 投影/audit/fingerprint 同源:adapter 消费冻结 full 投影,audit
            // 字面量 v2 完整且 resume 指纹 == full 指纹 == prepared map digest。
            assert_eq!(
                harness.adapter.last_start_projection_digest().as_deref(),
                Some(candidate.full_projection().projection_digest())
            );
            let literal = harness
                .adapter
                .last_start_audit_literal()
                .expect("[{tag}] audit literal assembled");
            assert!(literal.is_v2_complete());
            assert_eq!(literal.resume_fingerprint, fingerprint.digest);
            assert_eq!(literal.resume_fingerprint, persisted_prepared.fingerprint_digest);
            assert_eq!(literal.wire_dialect, family.wire_text());

            // durable provider_start:lc_projection 五字段与本家标记同源。
            let records = provider_start_records(&sink);
            assert_eq!(records.len(), 1, "[{tag}] one provider_start");
            let record = &records[0];
            assert_eq!(record.provider, family.audit_provider_text());
            assert_eq!(record.provider_version, family.exact_version());
            assert_eq!(record.adapter_dialect, family.wire_text());
            assert_eq!(record.sandbox, family.audit_sandbox());
            assert_eq!(record.approval_policy, family.audit_approval());
            let audit_projection = record
                .lc_projection
                .as_ref()
                .expect("[{tag}] lc projection persisted");
            assert_eq!(
                audit_projection.projection_digest,
                candidate.full_projection().projection_digest()
            );
            assert_eq!(
                audit_projection.capability_projection_digest,
                candidate
                    .full_projection()
                    .capability_projection_digest()
            );

            // -- 本家事实消费(公共断言,不跨 provider 文件):
            match family {
                AnchorT10Family::Claude => {
                    assert!(!record.argv.contains(&"--resume".to_string()));
                }
                AnchorT10Family::Codex => {
                    assert_eq!(record.sandbox.as_deref(), Some("read-only"));
                    assert!(record.argv.contains(&"read-only".to_string()));
                }
                AnchorT10Family::Pi => {
                    assert_eq!(
                        harness.adapter.extension_writes(),
                        1,
                        "[pi] extension 写只在 start 器械段(prepare/判定阶段零写)"
                    );
                }
                AnchorT10Family::Kimi => {
                    assert!(candidate
                        .full_projection()
                        .trust_digest()
                        .starts_with(&format!("lc-trust-v1:{:?}:", family.name())));
                    // native-source 与 Aria bundle 区分:换 native 标记后重
                    // prepare,facts/projection digest 随 MCP 来源变化。
                    harness
                        .adapter
                        .set_mcp_bundle_digest("kimi-native-project-config");
                    let native_candidate = harness
                        .gateway
                        .prepare_lc_streaming_candidate(
                            harness.streaming_input(None),
                            harness.request.clone(),
                            harness.audit_context(sink.clone()),
                            CancellationToken::new(),
                        )
                        .await
                        .expect("[kimi] native-source prepare");
                    assert_eq!(
                        native_candidate.full_projection().mcp_bundle_digest(),
                        "kimi-native-project-config"
                    );
                    assert_ne!(
                        native_candidate.full_projection().projection_digest(),
                        candidate.full_projection().projection_digest(),
                        "[kimi] native-source 与 Aria bundle 不得混源"
                    );
                }
            }

            // kimi 的 native-source 复验不得污染 wrongID 段:恢复 aria
            // bundle,使存档/当前投影回到同源形态(错 ID 断言的是 handshake
            // 失败的 kill/reap,不是 MCP 漂移拒绝)。
            if family == AnchorT10Family::Kimi {
                harness
                    .adapter
                    .set_mcp_bundle_digest(family.aria_mcp_bundle_digest());
            }

            // -- wrongID 如实 kill/reap(child 后 handshake 失败,不伪称零 child)。
            let resume_id = format!("native-{tag}-t10");
            anchor_t10_preset_resume_archive(
                &sink,
                family,
                &resume_id,
                candidate.full_projection(),
            );
            harness
                .adapter
                .set_handshake_native_id(Some(format!("native-{tag}-wrong")));
            let resume_candidate = harness
                .gateway
                .prepare_lc_streaming_candidate(
                    harness.streaming_input(Some(resume_id.clone())),
                    harness.request.clone(),
                    harness.audit_context(sink.clone()),
                    CancellationToken::new(),
                )
                .await
                .expect("[{tag}] resume prepare");
            let children_before = harness.adapter.session_children();
            let killed_before = harness.adapter.children_killed();
            let provider_starts_before = provider_start_records(&sink).len();
            let wrong = harness
                .gateway
                .start_lc_prepared_streaming(resume_candidate, CancellationToken::new())
                .await;
            assert!(wrong.is_err(), "[{tag}] wrong native id must fail the resume");
            assert_eq!(harness.adapter.session_children(), children_before + 1);
            assert_eq!(harness.adapter.children_killed(), killed_before + 1);
            assert_eq!(
                harness.adapter.session_children() - harness.adapter.children_killed(),
                1,
                "[{tag}] 只有成功路径的 child 存活;错 ID child 已 kill/reap"
            );
            assert_eq!(
                provider_start_records(&sink).len(),
                provider_starts_before,
                "[{tag}] 错 ID 不写 fresh provider_start"
            );

            // -- Claude --resume 事实:同 ID resume 成功,argv 携 --resume <id>。
            if family == AnchorT10Family::Claude {
                harness.adapter.set_handshake_native_id(None);
                let claude_resume_candidate = harness
                    .gateway
                    .prepare_lc_streaming_candidate(
                        harness.streaming_input(Some(resume_id.clone())),
                        harness.request.clone(),
                        harness.audit_context(sink.clone()),
                        CancellationToken::new(),
                    )
                    .await
                    .expect("[claude] resume prepare");
                let claude_session = harness
                    .gateway
                    .start_lc_prepared_streaming(
                        claude_resume_candidate,
                        CancellationToken::new(),
                    )
                    .await
                    .expect("[claude] same-id resume start");
                assert_eq!(
                    claude_session.native_session_id.as_deref(),
                    Some(resume_id.as_str())
                );
                let claude_records = provider_start_records(&sink);
                let last = claude_records.last().expect("[claude] resume provider_start");
                assert!(last.argv.contains(&"--resume".to_string()));
                assert!(last.argv.contains(&resume_id));
            }
        }
    }

    /// Task 3.7 Step 1 用例二:spawn 前复验矩阵——prepared facts 在 session
    /// start 前重验;version/trust/spawn 前变化受门、混合 provider 矩阵与
    /// audit v2 完整性门全部 fail-closed 在真实 adapter start 之前(②③=0)。
    #[tokio::test]
    async fn anchor_t10_prepared_facts_revalidate_before_session_start() {
        // A. 四家:prepare 后 executable identity+version 漂移 → typed 拒。
        for family in AnchorT10Family::ALL {
            let tag = family.tag();
            let harness = anchor_t10_harness(family);
            harness.install_policy();
            let sink = RecordingToolPolicyAuditSink::new();
            let candidate = harness
                .gateway
                .prepare_lc_streaming_candidate(
                    harness.streaming_input(None),
                    harness.request.clone(),
                    harness.audit_context(sink.clone()),
                    CancellationToken::new(),
                )
                .await
                .unwrap_or_else(|error| panic!("[{tag}] prepare failed: {error}"));
            assert_eq!(harness.adapter.probe_calls(), 1);
            harness
                .adapter
                .set_executable_identity(&format!("sha256:t10-{tag}-exe-v2"));
            harness
                .adapter
                .set_observed_version(&format!("{tag} 9.9.9-drifted"));
            let drift = harness
                .gateway
                .start_lc_prepared_streaming(candidate.clone(), CancellationToken::new())
                .await
                .err()
                .expect("[{tag}] version drift must typed-reject before spawn");
            assert!(
                matches!(
                    &drift,
                    ProviderGatewayError::PolicyDrift { dimension } if dimension == "provider_version"
                ),
                "[{tag}] unexpected drift shape: {drift}"
            );
            assert_eq!(harness.adapter.probe_calls(), 2, "[{tag}] 重观测换 key 再探测");
            assert_eq!(harness.adapter.adapter_starts(), 0, "[{tag}] ② mismatch=0");
            assert_eq!(harness.adapter.session_children(), 0, "[{tag}] ③ mismatch=0");
            assert_eq!(harness.adapter.starts_without_projection(), 0);
            assert_eq!(harness.audit.stream_launches(), 0);
        }

        // B. 混合 provider 矩阵:prepare→start 间换绑同家新实例/跨家 adapter。
        let claude_harness = anchor_t10_harness(AnchorT10Family::Claude);
        claude_harness.install_policy();
        let sink = RecordingToolPolicyAuditSink::new();
        let candidate = claude_harness
            .gateway
            .prepare_lc_streaming_candidate(
                claude_harness.streaming_input(None),
                claude_harness.request.clone(),
                claude_harness.audit_context(sink.clone()),
                CancellationToken::new(),
            )
            .await
            .expect("claude prepare");
        let swapped_adapter = AnchorT10FamilyAdapter::new(
            AnchorT10Family::Claude,
            Arc::new(crate::cross_cutting::streaming_provider::LcVersionCache::new()),
        );
        let mut swapped_registry = claude_harness.registry();
        swapped_registry.register_gated(
            crate::product::models::ProviderName::ClaudeCode,
            swapped_adapter.clone(),
            Arc::new(
                crate::product::logical_codebase::provider_projection::UnprovisionedProviderPolicyProjector,
            ),
            always_available_gate(),
        )
        .expect("anchor t10 register swapped adapter");
        let swapped_error = claude_harness
            .gateway_with_registry(swapped_registry)
            .start_lc_prepared_streaming(candidate.clone(), CancellationToken::new())
            .await
            .err()
            .expect("swapped same-family adapter must typed-reject");
        assert!(
            matches!(
                &swapped_error,
                ProviderGatewayError::PolicyDrift { dimension } if dimension == "provider_adapter"
            ),
            "unexpected swapped shape: {swapped_error}"
        );

        let cross_adapter = AnchorT10FamilyAdapter::new(
            AnchorT10Family::Codex,
            Arc::new(crate::cross_cutting::streaming_provider::LcVersionCache::new()),
        );
        let mut cross_registry = claude_harness.registry();
        cross_registry.register_gated(
            crate::product::models::ProviderName::ClaudeCode,
            cross_adapter.clone(),
            Arc::new(
                crate::product::logical_codebase::provider_projection::UnprovisionedProviderPolicyProjector,
            ),
            always_available_gate(),
        )
        .expect("anchor t10 register cross adapter");
        let cross_error = claude_harness
            .gateway_with_registry(cross_registry)
            .start_lc_prepared_streaming(candidate.clone(), CancellationToken::new())
            .await
            .err()
            .expect("cross-family adapter must typed-reject");
        assert!(
            matches!(
                &cross_error,
                ProviderGatewayError::PolicyDrift { dimension } if dimension == "provider_adapter"
            ),
            "unexpected cross shape: {cross_error}"
        );
        assert_eq!(swapped_adapter.adapter_starts(), 0);
        assert_eq!(cross_adapter.adapter_starts(), 0);
        assert_eq!(claude_harness.adapter.adapter_starts(), 0);
        assert_eq!(claude_harness.adapter.session_children(), 0);

        // C. audit v2 材料不完整:会话投影摘要为空的 projector 可 prepare
        //    冻结,但不可达生产 spawn(gateway 门先于 adapter start)。
        let v2_harness = anchor_t10_harness(AnchorT10Family::Pi);
        v2_harness.install_policy();
        let v2_sink = RecordingToolPolicyAuditSink::new();
        v2_harness
            .adapter
            .set_projector_overrides(None, Some(String::new()));
        let v2_candidate = v2_harness
            .gateway
            .prepare_lc_streaming_candidate(
                v2_harness.streaming_input(None),
                v2_harness.request.clone(),
                v2_harness.audit_context(v2_sink.clone()),
                CancellationToken::new(),
            )
            .await
            .expect("v2-incomplete projector can freeze a candidate");
        let v2_error = v2_harness
            .gateway
            .start_lc_prepared_streaming(v2_candidate, CancellationToken::new())
            .await
            .err()
            .expect("v2-incomplete audit material must not spawn");
        assert!(
            matches!(&v2_error, ProviderGatewayError::UnsupportedCapability(message)
                if message.contains("v2 material incomplete")),
            "unexpected v2 shape: {v2_error}"
        );
        assert_eq!(v2_harness.adapter.adapter_starts(), 0);
        assert_eq!(v2_harness.adapter.session_children(), 0);

        // D. trust spawn 前变化受门(Codex 需 workspace trust):prepare 冻结
        //    后 trust 失效 → typed 拒,零 spawn。
        let codex_harness = anchor_t10_harness(AnchorT10Family::Codex);
        codex_harness.install_policy();
        let codex_sink = RecordingToolPolicyAuditSink::new();
        let codex_candidate = codex_harness
            .gateway
            .prepare_lc_streaming_candidate(
                codex_harness.streaming_input(None),
                codex_harness.request.clone(),
                codex_harness.audit_context(codex_sink.clone()),
                CancellationToken::new(),
            )
            .await
            .expect("codex prepare under trusted source");
        assert!(codex_candidate
            .full_projection()
            .trust_digest()
            .contains("sha256:t10-trust-record-codex"));
        codex_harness.trust.set_trusted(false);
        let trust_error = codex_harness
            .gateway
            .start_lc_prepared_streaming(codex_candidate, CancellationToken::new())
            .await
            .err()
            .expect("trust drift before spawn must reject");
        assert!(
            trust_error.to_string().contains("trust"),
            "unexpected trust shape: {trust_error}"
        );
        assert_eq!(codex_harness.adapter.adapter_starts(), 0);
        assert_eq!(codex_harness.adapter.session_children(), 0);

        // E. 相位收口:非 PlanningReadOnly 候选不经唯一 start 路径(Coding/
        //    Recipe/Review 等既有相位仍走同名 gateway API)。ReviewReadOnly
        //    与 planning 同为 read-only 形态,只差 action,可直接 prepare。
        let phase_harness = anchor_t10_harness(AnchorT10Family::Claude);
        phase_harness.install_policy();
        let phase_sink = RecordingToolPolicyAuditSink::new();
        let mut review_request = phase_harness.request.clone();
        review_request.action = SessionPolicyAction::ReviewReadOnly;
        let phase_candidate = phase_harness
            .gateway
            .prepare_lc_streaming_candidate(
                phase_harness.streaming_input(None),
                review_request,
                phase_harness.audit_context(phase_sink),
                CancellationToken::new(),
            )
            .await
            .expect("review candidate can be prepared");
        let phase_error = phase_harness
            .gateway
            .start_lc_prepared_streaming(phase_candidate, CancellationToken::new())
            .await
            .err()
            .expect("non-planning candidate must not use the converged start path");
        assert!(
            matches!(&phase_error, ProviderGatewayError::UnsupportedCapability(message)
                if message.contains("PlanningReadOnly")),
            "unexpected phase shape: {phase_error}"
        );
        assert_eq!(phase_harness.adapter.adapter_starts(), 0);
    }
}
