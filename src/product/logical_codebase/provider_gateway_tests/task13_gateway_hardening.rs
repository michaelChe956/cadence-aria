//! Task 13(r47/T12 修订):Codex LC 会话放行、resume 一致性与配置来源审计。
//!
//! 三个验收维度:
//! 1. (T12)Codex LC 全相位 danger-full-access 是合法基线——旧路由级阻断与
//!    受限档(read-only/workspace-write)已退役,validate 由 capability 行与
//!    投影复验裁决,不再因 danger 形态拒绝。
//! 2. resume 一致性:`resume_or_start` 只有当 policy digest、target、provider exact
//!    version、dialect 与 capability snapshot 全一致(fingerprint 相等)才 resume;
//!    任一漂移则 supersede 旧会话并新建。
//! 3. 配置来源审计:`ConfigSourceAudit` 记录 user/project/local/env/子仓 MCP 来源
//!    与最终 argv/config digest;发现 managed settings 时标注
//!    `managed_settings_active=true` 并发警告,绝不假装已覆盖;policy 可据此拒绝启动。
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

/// r47 修订(T12 收口):Codex 路由级全阻已彻底退役——danger-full-access
/// 是 LC 全相位(REQ-LCG-04 改版)的当前执行基线,LC 会话的 Codex 放行
/// 由 capability 行(探针 Confirmed)与投影复验裁决。本测试钉住新契约:
/// capability double 放行时 validate 成功(无路由级 danger 全阻)。
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
        "Codex LC 会话经投影正常 validated(全相位 danger 基线,无路由阻断)"
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
    let claude = ProviderRef::from_provider_name(&ProviderName::ClaudeCode, "cap_managed_snapshot")
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

include!("audit.inc.rs");
