//! Pi 侧 RootRecipe 相位 `BootstrapExecutorMarker` canonical 化测试
//! (REQ-BOOT-15;owner 2026-11-05 裁定方案 B):root recipe 相位对 provider
//! 无关——Pi 的 LC 投影与 Claude 同语义(marker 在 RootRecipe 相位放行并
//! 使用共享 digest,Normal 相位 canonical 层独立 fail-closed,与 T7 guard
//! 相位门构成双层防线)。

use std::path::PathBuf;

use crate::cross_cutting::pi_provider::{PiPolicyProjector, projection};
use crate::cross_cutting::streaming_provider::{
    ProviderPermissionMode, ProviderToolPolicy, ToolPolicyIntent,
};
use crate::product::logical_codebase::aggregate_initialization::AggregateInitializationStepKind;
use crate::product::logical_codebase::policy::{
    PolicyTarget, ProviderDialect, SessionPolicyAction, SessionPolicyEnvelope,
};
use crate::product::logical_codebase::provider_admission_preflight::{
    BootstrapExecutorMarker, BootstrapPhaseCredential,
};
use crate::product::logical_codebase::provider_gateway::ProviderRef;
use crate::product::logical_codebase::provider_projection::{
    ProviderPolicyProjector, ProviderProjectionInput,
};
use crate::protocol::contracts::AdapterRole;

/// 构造完整四要素的 BootstrapExecutorMarker 策略(canonical 层测试材料;
/// 生产面凭据只能经 `from_running_operation` 派生,此处用 for_test seam,
/// 与 claude 侧 Task 10 fix 测试同构)。
fn t10_bootstrap_marker_policy(operation_id: &str, receipt_context: &str) -> ProviderToolPolicy {
    let canonical_root = PathBuf::from("/lc/bootstrap-root");
    let credential = BootstrapPhaseCredential::for_test(
        "project_0001",
        "logical_codebase_0001",
        operation_id,
        AggregateInitializationStepKind::PreCheck,
        "sha256:test-input-digest",
        canonical_root.clone(),
    );
    let marker = BootstrapExecutorMarker::new(
        credential,
        SessionPolicyAction::CodingTargetWrite,
        canonical_root,
        receipt_context,
    )
    .expect("complete bootstrap executor marker");
    ProviderToolPolicy {
        intent: ToolPolicyIntent::BootstrapExecutorMarker(marker),
    }
}

/// marker 会话的投影输入(Coding 写面;root recipe 自举执行器承载形态)。
fn t10_marker_projection_input(policy: &ProviderToolPolicy) -> ProviderProjectionInput {
    let target = PathBuf::from("/lc/bootstrap-root/member-worktree");
    let envelope = SessionPolicyEnvelope {
        policy_id: "policy-lc-pi-marker-0001".to_string(),
        policy_revision: 1,
        policy_digest: "sha256:policy-lc-pi-marker-0001".to_string(),
        action: SessionPolicyAction::CodingTargetWrite,
        target: PolicyTarget::checkout("logical_repo_0001", "checkout_0001", target.clone()),
        working_directory: PathBuf::from("/lc/bootstrap-root"),
        readable_roots: vec![PathBuf::from("/lc/bootstrap-root")],
        writable_roots: vec![target],
        provider_dialect: ProviderDialect::PiRpcV1,
        config_artifact_ref: "sha256:cfg-a".to_string(),
        config_digest: "sha256:cfg-digest-a".to_string(),
        created_at: "2026-11-05T00:00:00Z".to_string(),
        authority_root: PathBuf::from("/lc/bootstrap-root"),
    };
    ProviderProjectionInput::new(
        envelope,
        ProviderRef::pi("cap_pi_lc_marker_fixture"),
        SessionPolicyAction::CodingTargetWrite,
        AdapterRole::Executor,
        ProviderPermissionMode::Auto,
        Some(policy.clone()),
        projection::PI_LC_APPROVAL_POLICY.to_string(),
        String::new(),
        "sha256:cfg-a".to_string(),
        "sha256:trust-1".to_string(),
        None,
    )
}

/// REQ-BOOT-15(root recipe 相位对 provider 无关):RootRecipe 相位的
/// BootstrapExecutorMarker 在 Pi 侧可确定性 canonical 化(与 claude 同语义
/// 的共享 digest 实现),供会话投影与统一 launch audit 落盘;Normal 相位的
/// marker 在 canonical 层独立拒绝(T7 guard 相位门之外的二层防线,普通
/// Coder 不可借 marker);deny/None 的既有 canonical 语义与相位无关,零回归。
#[test]
fn lcg_t10_pi_root_recipe_marker_canonicalization_is_deterministic_and_phase_gated() {
    // 1) RootRecipe 相位:可翻译且确定性(同材料两次一致,digest 形状与
    //    LC 分层摘要同构:sha256: 前缀 + 64 位小写 hex)。
    let marker_policy = t10_bootstrap_marker_policy(
        "aggregate_initialization_0001",
        "root-recipe:pre_check:command-1",
    );
    let canonical = projection::lc_tool_policy_canonical_digest(Some(&marker_policy), true)
        .expect("root recipe phase marker must canonicalize");
    assert!(canonical.starts_with("sha256:"));
    assert_eq!(canonical.len(), 71);
    let again = projection::lc_tool_policy_canonical_digest(Some(&marker_policy), true)
        .expect("canonicalization is deterministic");
    assert_eq!(canonical, again);

    // 2) 漂移敏感:凭据 operation id / receipt context 任一变化 → digest 变化。
    let drifted_operation = t10_bootstrap_marker_policy(
        "aggregate_initialization_0002",
        "root-recipe:pre_check:command-1",
    );
    let drifted_receipt = t10_bootstrap_marker_policy(
        "aggregate_initialization_0001",
        "root-recipe:rule_and_mcp_config:command-2",
    );
    assert_ne!(
        canonical,
        projection::lc_tool_policy_canonical_digest(Some(&drifted_operation), true)
            .expect("drifted operation still canonicalizes")
    );
    assert_ne!(
        canonical,
        projection::lc_tool_policy_canonical_digest(Some(&drifted_receipt), true)
            .expect("drifted receipt still canonicalizes")
    );

    // 3) canonical 层防线:Normal 相位不可传 marker(普通 Coder 不可借),
    //    拒绝文案与 claude 同形(provider 名替换)。
    let rejected = projection::lc_tool_policy_canonical_digest(Some(&marker_policy), false)
        .expect_err("normal phase marker must be rejected at the canonical layer");
    let rejected_text = rejected.to_string();
    assert!(
        rejected_text.contains("pi lc tool policy is not canonicalizable"),
        "unexpected rejection: {rejected_text}"
    );
    assert!(
        rejected_text.contains("root recipe phase"),
        "unexpected rejection: {rejected_text}"
    );

    // 4) deny/None 与相位无关(既有 canonical 语义零回归)。
    let deny = ProviderToolPolicy::deny_file_write_builtins();
    assert_eq!(
        projection::lc_tool_policy_canonical_digest(Some(&deny), true)
            .expect("deny canonicalizes in root recipe phase"),
        projection::lc_tool_policy_canonical_digest(Some(&deny), false)
            .expect("deny canonicalizes in normal phase")
    );
    assert_eq!(
        projection::lc_tool_policy_canonical_digest(None, true).expect("none in root recipe"),
        projection::lc_tool_policy_canonical_digest(None, false).expect("none in normal")
    );

    // 5) projector 层:RootRecipe 相位投影成功且确定性;Normal 相位的
    //    projector 独立 fail-closed(T7 guard 先拦之外的二层防线)。
    let marker_input = t10_marker_projection_input(&marker_policy);
    let root_recipe_projector =
        PiPolicyProjector::new("pi 0.83.0-lc-fixture").with_root_recipe_phase(true);
    let projected = root_recipe_projector
        .project(&marker_input)
        .expect("root recipe projector accepts the bootstrap marker");
    assert!(projected.tool_policy().is_some());
    assert!(projected.projection_digest().starts_with("sha256:"));
    assert_eq!(
        projected.projection_digest(),
        root_recipe_projector
            .project(&marker_input)
            .expect("projection is deterministic")
            .projection_digest()
    );

    let normal_projector = PiPolicyProjector::new("pi 0.83.0-lc-fixture");
    let rejected_projection = normal_projector
        .project(&marker_input)
        .expect_err("normal phase projector must refuse the marker");
    assert!(
        rejected_projection
            .to_string()
            .contains("root recipe phase"),
        "unexpected projection rejection: {rejected_projection:?}"
    );
}
