//! P2 GAP-F（Task 0.2）→ C5 Task 3（REQ-WIGA-C5-PREFLIGHT）：Enable 前
//! 完整角色链静态预检。
//!
//! 逐角色循环：plan_author/coder←author_provider，plan/code reviewer←
//! reviewer_provider，internal reviewer 按同一 reviewer 配置三值派生先例
//!（`From<&ProviderConfigSnapshot>`：internal_reviewer＝reviewer）取
//! reviewer_provider。谓词本体与原 reviewer 单角色预检同源（Pi/KimiCode
//! 无 gateway dialect 经 `ProviderRef::from_provider_name` 集中 fail-closed、
//! Codex 固定 `danger-full-access` sandbox 的路由禁令与
//! `LogicalCodebaseProviderGateway::enforce_route_policy` 同源），不产生第二
//! 套支持矩阵。只判**确定性**静态不支持：不探活、不实例化 provider、不改
//! 用户选项。GET 投影、PUT Enable、rebind 三调用点共用同一判定；单仓载体
//! 整体跳过 gateway 谓词不误拒（A10）；422 payload 一次列全全部违规角色
//!（不逐次试错）。测试运行 `test_provider_enabled` 只按角色豁免 Fake。

use crate::product::logical_codebase::provider_gateway::{
    CODEX_DANGER_FULL_ACCESS_SANDBOX_MODE, CODEX_DANGER_FULL_ACCESS_UNSUPPORTED, ProviderRef,
};
use crate::product::models::ProviderName;
use crate::web::error::{ApiError, ApiResult};
use serde_json::json;

use super::support::AutomationCarrierResolution;

/// GET 投影／PUT Enable／rebind 共用的稳定错误码（HTTP 422）。
pub(crate) const AUTOMATION_ROLE_CHAIN_UNSUPPORTED: &str = "automation_role_chain_unsupported";

/// 逐角色违规投影：一次 422 列全（契约 `AutomationRoleChainViolation`）。
#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct AutomationRoleChainViolation {
    pub role: String,
    pub provider: String,
    pub reason_code: String,
}

/// 单角色静态谓词本体（与 `provider_gateway::enforce_route_policy` 同源）：
/// 返回稳定 reason_code（`codex_danger_full_access_unsupported` 路由禁令 /
/// `provider_unsupported_for_gateway_launch` dialect 缺失）与人类可读 message。
fn static_gateway_verdict(provider: &ProviderName) -> Result<(), (String, String)> {
    // Codex 路由级硬门：当前固定 sandbox 即 danger-full-access 时静态拒绝，
    // 不依赖运行时 policy。
    if matches!(provider, ProviderName::Codex)
        && CODEX_DANGER_FULL_ACCESS_SANDBOX_MODE
            == crate::cross_cutting::codex_provider::CODEX_DEFAULT_SANDBOX_MODE
    {
        return Err((
            CODEX_DANGER_FULL_ACCESS_UNSUPPORTED.to_string(),
            format!(
                "codex is statically blocked at the gateway route: \
                 {CODEX_DANGER_FULL_ACCESS_UNSUPPORTED}"
            ),
        ));
    }
    ProviderRef::from_provider_name(provider, "static-preflight")
        .map(|_| ())
        .map_err(|error| {
            (
                "provider_unsupported_for_gateway_launch".to_string(),
                error.to_string(),
            )
        })
}

/// 共享核心：角色派生＋逐角色谓词循环。`single_repository_carrier` 为真时
/// 整体跳过 gateway 谓词（单仓不误拒，Review Focus 5）；`gateway_required`
/// 为假不预检；`test_provider_enabled` 只按角色豁免 Fake（Pi/KimiCode 不豁免）。
fn validate_role_chain(
    author_provider: &ProviderName,
    reviewer_provider: &ProviderName,
    single_repository_carrier: bool,
    gateway_required: bool,
    test_provider_enabled: bool,
) -> ApiResult<()> {
    if !gateway_required || single_repository_carrier {
        return Ok(());
    }
    let provider_name = |provider: &ProviderName| {
        serde_json::to_value(provider)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_else(|| format!("{provider:?}"))
    };
    let mut violations: Vec<AutomationRoleChainViolation> = Vec::new();
    for (role, provider) in [
        ("plan_author", author_provider),
        ("coder", author_provider),
        ("plan_reviewer", reviewer_provider),
        ("code_reviewer", reviewer_provider),
        // internal reviewer 无独立 ProviderName 变体：按同一 reviewer 配置
        // 三值派生先例取 reviewer_provider（在场即参与；缺失即 None 不参与
        // ——enrollment options 的 reviewer 必填，故此处恒在场）。
        ("internal_reviewer", reviewer_provider),
    ] {
        if test_provider_enabled && matches!(provider, ProviderName::Fake) {
            continue;
        }
        if let Err((reason_code, message)) = static_gateway_verdict(provider) {
            violations.push(AutomationRoleChainViolation {
                role: role.to_string(),
                provider: provider_name(provider),
                reason_code,
            });
        }
    }
    if violations.is_empty() {
        return Ok(());
    }
    let details = json!({
        "violations": violations,
        "hint": "reconfigure the automation providers or re-select the automation target",
    });
    Err(ApiError::validation_with_details(
        AUTOMATION_ROLE_CHAIN_UNSUPPORTED,
        "automation role chain has statically unsupported providers",
        details,
    ))
}

/// C5 Task 3 契约入口：GET 投影与 PUT Enable 的载体判定来自
/// `resolve_automation_carrier`（Task 2 产物），同一 carrier 进同一判定。
pub(crate) fn validate_role_chain_for_enrollment(
    author_provider: &ProviderName,
    reviewer_provider: &ProviderName,
    carrier: &AutomationCarrierResolution,
    gateway_required: bool,
    test_provider_enabled: bool,
) -> ApiResult<()> {
    validate_role_chain(
        author_provider,
        reviewer_provider,
        matches!(
            carrier,
            AutomationCarrierResolution::SingleRepository { .. }
        ),
        gateway_required,
        test_provider_enabled,
    )
}

/// rebind 调用点专用：手上只有 enrollment 声明的 target（无 authority
/// resolution），按声明 target 的载体类别进同一判定核心——不重解析 issue
/// 权威载体（契约「rebind 的 carrier 取 enrollment 现有 target」）。
pub(crate) fn validate_role_chain_for_declared_enrollment_target(
    author_provider: &ProviderName,
    reviewer_provider: &ProviderName,
    declared: &crate::product::logical_codebase::EnrollmentTarget,
    gateway_required: bool,
    test_provider_enabled: bool,
) -> ApiResult<()> {
    validate_role_chain(
        author_provider,
        reviewer_provider,
        matches!(
            declared,
            crate::product::logical_codebase::EnrollmentTarget::SingleRepository { .. }
        ),
        gateway_required,
        test_provider_enabled,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::logical_codebase::EnrollmentTarget;
    use crate::product::logical_codebase::{
        AuthorityAggregateIndexReference, RepositoryAuthorityResolution, RepositoryTargetKind,
        ResolvedTargetIdentity,
    };

    fn lc_carrier() -> AutomationCarrierResolution {
        AutomationCarrierResolution::LogicalCodebase {
            resolution: RepositoryAuthorityResolution {
                authority_root: std::path::PathBuf::new(),
                target: ResolvedTargetIdentity {
                    kind: RepositoryTargetKind::LogicalCodebase,
                    repository_id: None,
                    logical_codebase_id: Some("logical_codebase_0001".to_string()),
                    logical_repository_id: None,
                    checkout_id: None,
                    canonical_path: std::path::PathBuf::new(),
                    source_identity_digest: String::new(),
                },
                manifest: None,
                selection: None,
                policy: None,
                aggregate_index: AuthorityAggregateIndexReference {
                    aggregate_index_id: None,
                    membership_revision: None,
                    status: None,
                },
            },
        }
    }

    fn violations_of(error: ApiError) -> Vec<serde_json::Value> {
        error
            .details
            .get("violations")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default()
    }

    /// C5 Task 3 主链：LC 载体下 author=Pi（plan_author＋coder 违规）且
    /// reviewer=KimiCode（plan/code/internal reviewer 违规）→ 同一次 422
    /// payload 列出全部 5 个角色（一次列全，不逐次试错）。
    #[test]
    fn role_chain_preflight_lists_every_violating_role_at_once() {
        let error = validate_role_chain_for_enrollment(
            &ProviderName::Pi,
            &ProviderName::KimiCode,
            &lc_carrier(),
            true,
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, AUTOMATION_ROLE_CHAIN_UNSUPPORTED);
        let violations = violations_of(error);
        let roles: Vec<&str> = violations
            .iter()
            .map(|violation| violation["role"].as_str().unwrap())
            .collect();
        assert_eq!(
            roles,
            vec![
                "plan_author",
                "coder",
                "plan_reviewer",
                "code_reviewer",
                "internal_reviewer"
            ],
            "every violating role must be listed at once: {violations:?}"
        );
        for violation in &violations {
            assert_eq!(
                violation["reason_code"], "provider_unsupported_for_gateway_launch",
                "{violation:?}"
            );
        }

        // 纯净组合零违规。
        assert!(
            validate_role_chain_for_enrollment(
                &ProviderName::ClaudeCode,
                &ProviderName::ClaudeCode,
                &lc_carrier(),
                true,
                false
            )
            .is_ok()
        );
    }

    /// 路由阻断：任一角色派生为 Codex（固定 danger-full-access 静态拒）→
    /// 该角色以路由禁令 reason_code 列出。
    #[test]
    fn role_chain_preflight_routes_codex_violations() {
        let error = validate_role_chain_for_declared_enrollment_target(
            &ProviderName::Codex,
            &ProviderName::ClaudeCode,
            &EnrollmentTarget::LogicalCodebase {
                logical_codebase_id: "logical_codebase_0001".to_string(),
                logical_repository_id: crate::product::logical_codebase::LogicalRepositoryId(
                    uuid::Uuid::nil(),
                ),
            },
            true,
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, AUTOMATION_ROLE_CHAIN_UNSUPPORTED);
        let violations = violations_of(error);
        let roles: Vec<&str> = violations
            .iter()
            .map(|violation| violation["role"].as_str().unwrap())
            .collect();
        assert_eq!(roles, vec!["plan_author", "coder"]);
        for violation in &violations {
            assert!(
                violation["reason_code"]
                    .as_str()
                    .unwrap()
                    .contains(CODEX_DANGER_FULL_ACCESS_UNSUPPORTED),
                "{violation:?}"
            );
        }
    }

    /// 单仓载体整体跳过 gateway 谓词：LC 不支持但本机可用的组合不误拒
    ///（Review Focus 5／A10）。
    #[test]
    fn role_chain_preflight_skips_gateway_predicates_for_single_repository() {
        let single = AutomationCarrierResolution::SingleRepository {
            target: EnrollmentTarget::SingleRepository {
                repository_id: "repo-1".to_string(),
            },
        };
        for (author, reviewer) in [
            (ProviderName::Pi, ProviderName::KimiCode),
            (ProviderName::KimiCode, ProviderName::Pi),
        ] {
            assert!(
                validate_role_chain_for_enrollment(&author, &reviewer, &single, true, false)
                    .is_ok(),
                "single-repository carrier must skip gateway predicates"
            );
        }
    }

    /// 旧实名 1/3：Pi/KimiCode 无 gateway dialect，静态拒且携带稳定 verdict。
    #[test]
    fn kimi_code_and_pi_are_statically_rejected() {
        let lc = EnrollmentTarget::LogicalCodebase {
            logical_codebase_id: "logical_codebase_0001".to_string(),
            logical_repository_id: crate::product::logical_codebase::LogicalRepositoryId(
                uuid::Uuid::nil(),
            ),
        };
        for provider in [ProviderName::KimiCode, ProviderName::Pi] {
            let error = validate_role_chain_for_declared_enrollment_target(
                &provider,
                &ProviderName::ClaudeCode,
                &lc,
                true,
                false,
            )
            .unwrap_err();
            assert_eq!(error.code, AUTOMATION_ROLE_CHAIN_UNSUPPORTED);
            let violations = error
                .details
                .get("violations")
                .and_then(|value| value.as_array())
                .cloned()
                .unwrap_or_default();
            assert!(
                violations.iter().all(|violation| {
                    violation["reason_code"] == "provider_unsupported_for_gateway_launch"
                }),
                "should carry the gateway mapping verdict, got: {violations:?}"
            );
        }
    }

    /// 旧实名 2/3：Codex 在当前固定 danger-full-access sandbox 下被路由禁令
    /// 静态拒绝。
    #[test]
    fn codex_is_rejected_under_current_default_sandbox() {
        let lc = EnrollmentTarget::LogicalCodebase {
            logical_codebase_id: "logical_codebase_0001".to_string(),
            logical_repository_id: crate::product::logical_codebase::LogicalRepositoryId(
                uuid::Uuid::nil(),
            ),
        };
        let error = validate_role_chain_for_declared_enrollment_target(
            &ProviderName::ClaudeCode,
            &ProviderName::Codex,
            &lc,
            true,
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, AUTOMATION_ROLE_CHAIN_UNSUPPORTED);
        let violations = violations_of(error);
        assert_eq!(
            violations
                .iter()
                .filter(|violation| violation["role"] == "plan_reviewer")
                .count(),
            1
        );
    }

    /// 旧实名 3/3：ClaudeCode 通过；Fake 仅测试运行豁免；非 gateway 路径
    /// 不预检；测试模式不放行 Pi/KimiCode。
    #[test]
    fn claude_code_passes_and_fake_requires_test_run() {
        let lc = EnrollmentTarget::LogicalCodebase {
            logical_codebase_id: "logical_codebase_0001".to_string(),
            logical_repository_id: crate::product::logical_codebase::LogicalRepositoryId(
                uuid::Uuid::nil(),
            ),
        };
        assert!(
            validate_role_chain_for_declared_enrollment_target(
                &ProviderName::ClaudeCode,
                &ProviderName::ClaudeCode,
                &lc,
                true,
                false
            )
            .is_ok()
        );
        // 真实运行 Fake 无 gateway dialect；仅测试运行豁免。
        assert!(
            validate_role_chain_for_declared_enrollment_target(
                &ProviderName::Fake,
                &ProviderName::Fake,
                &lc,
                true,
                false
            )
            .is_err()
        );
        assert!(
            validate_role_chain_for_declared_enrollment_target(
                &ProviderName::Fake,
                &ProviderName::Fake,
                &lc,
                true,
                true
            )
            .is_ok()
        );
        // 非 gateway 路径不预检。
        assert!(
            validate_role_chain_for_declared_enrollment_target(
                &ProviderName::KimiCode,
                &ProviderName::KimiCode,
                &lc,
                false,
                false
            )
            .is_ok()
        );
        // 测试模式不放行 Pi/KimiCode。
        assert!(
            validate_role_chain_for_declared_enrollment_target(
                &ProviderName::Pi,
                &ProviderName::KimiCode,
                &lc,
                true,
                true
            )
            .is_err()
        );
    }
}
