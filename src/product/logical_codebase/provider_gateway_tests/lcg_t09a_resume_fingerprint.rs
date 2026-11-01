//! Task 9a(lcg_t09a):resume 指纹核心——五参 `from_envelope`(envelope+
//! projection+action evidence+target git identity)、gateway 内部
//! `resume_or_start_with_projection`(调用者先完整 prepare 得候选投影再比
//! 指纹)与 `StartNew` 的 supersede+显式 fresh 等待面。
//!
//! 契约(计划 Interfaces 冻结):
//! 1. 指纹为长度分隔 + schema 域版本的 canonical SHA-256;policy/authority/
//!    cwd/target/git identity/trust/tool/MCP bundle/exact version/投影摘要
//!    任一漂移都改变 digest。
//! 2. `resume_or_start_with_projection` 只有全上下文指纹与旧会话冻结指纹相等
//!    才 `Resume`(返回的 validated 携带完整上下文指纹与候选投影);漂移仅
//!    supersede(审计)+`StartNew`(等待显式 `StartGeneration`),决策层
//!    零 spawn,不得用仅 envelope 指纹生成可 spawn 权。
//! 3. 旧 `resume_or_start` 的 envelope+capability 基准(单仓/legacy 语义)
//!    由既有 task13/task28 测试保持,不在此重证。
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
fn t09a_baseline_projection(validated: &ValidatedSessionLaunchPolicy) -> ProviderPolicyProjection {
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

    let baseline =
        SessionResumeFingerprint::from_envelope(&envelope, &projection, &evidence, &git_identity);
    assert!(
        baseline
            .digest
            .starts_with(&format!("{LC_RESUME_FINGERPRINT_SCHEMA}:sha256:"))
    );
    assert_eq!(
        baseline.digest.len(),
        LC_RESUME_FINGERPRINT_SCHEMA.len() + 1 + 71
    );
    // 稳定控制组:同输入 → 同 digest。
    assert_eq!(
        baseline.digest,
        SessionResumeFingerprint::from_envelope(&envelope, &projection, &evidence, &git_identity)
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
            SessionResumeFingerprint::from_envelope(&envelope, &drifted, &evidence, &git_identity)
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

    let mut drifted_cases: Vec<(&str, SessionLaunchRequest, ProviderPolicyProjection)> = Vec::new();
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
