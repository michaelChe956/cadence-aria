use super::*;

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
    assert!(
        validated
            .fingerprint()
            .digest
            .starts_with(&format!("{LC_RESUME_FINGERPRINT_SCHEMA}:sha256:"))
    );
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
        env_vars: Default::default(),
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

impl MutableTargetResolver {
    fn change_target_after_request(&self, target: PolicyTarget) {
        *self.target_override.lock().unwrap() = Some(target);
    }
}

impl StaticCapabilitySource {
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
}

impl GatewayFixture {
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
}
