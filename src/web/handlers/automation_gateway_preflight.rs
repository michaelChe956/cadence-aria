//! P2 GAP-F（Task 0.2）：enrollment 静态 gateway reviewer 组合预检。
//!
//! 单一静态能力谓词：`GET /automation-target` 投影与最终 PUT Enable 共用，
//! 均在确认唯一 logical target 之后调用。只判**确定性**静态不支持——
//! Pi/KimiCode 无 gateway dialect（`ProviderRef::from_provider_name` 集中
//! fail-closed）、Codex 当前固定 `danger-full-access` sandbox 的路由禁令
//! （与 `LogicalCodebaseProviderGateway::enforce_route_policy` 同源）；动态
//! 账号/版本/网络波动不误判为白名单，不探活、不实例化真实 provider、不改
//! 用户选项。测试运行 `test_provider_enabled` 只豁免 Fake reviewer。

use crate::product::logical_codebase::provider_gateway::{
    CODEX_DANGER_FULL_ACCESS_SANDBOX_MODE, CODEX_DANGER_FULL_ACCESS_UNSUPPORTED, ProviderRef,
};
use crate::product::models::ProviderName;
use crate::web::error::{ApiError, ApiResult};

/// GET 投影与 PUT Enable 共用的稳定错误码（HTTP 422）。
pub(crate) const AUTOMATION_GATEWAY_REVIEWER_UNSUPPORTED: &str =
    "automation_gateway_reviewer_unsupported";

pub(crate) fn invalid_gateway_reviewer(reason: impl Into<String>) -> ApiError {
    ApiError::validation(AUTOMATION_GATEWAY_REVIEWER_UNSUPPORTED, reason)
}

/// 静态 gateway reviewer 预检：`gateway_required` 由调用方在确认 logical
/// routing 后传入；`test_provider_enabled` 只允许 Fake 测试路径绕行，
/// 不允许 Pi/KimiCode 借测试模式通过。
pub(crate) fn validate_gateway_reviewer_for_enrollment(
    reviewer: &ProviderName,
    gateway_required: bool,
    test_provider_enabled: bool,
) -> ApiResult<()> {
    if !gateway_required || (test_provider_enabled && matches!(reviewer, ProviderName::Fake)) {
        return Ok(());
    }
    // Codex 路由级硬门与 provider_gateway::enforce_route_policy 同源：当前
    // 固定 sandbox 即 danger-full-access 时静态拒绝，不依赖运行时 policy。
    if matches!(reviewer, ProviderName::Codex)
        && CODEX_DANGER_FULL_ACCESS_SANDBOX_MODE
            == crate::cross_cutting::codex_provider::CODEX_DEFAULT_SANDBOX_MODE
    {
        return Err(invalid_gateway_reviewer(format!(
            "codex reviewer is statically blocked at the gateway route: \
             {CODEX_DANGER_FULL_ACCESS_UNSUPPORTED}"
        )));
    }
    ProviderRef::from_provider_name(reviewer, "static-preflight")
        .map(|_| ())
        .map_err(|error| invalid_gateway_reviewer(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kimi_code_and_pi_are_statically_rejected() {
        for provider in [ProviderName::KimiCode, ProviderName::Pi] {
            let error = validate_gateway_reviewer_for_enrollment(&provider, true, false).unwrap_err();
            assert_eq!(error.code, "automation_gateway_reviewer_unsupported");
            assert!(
                error.message.contains("provider_unsupported_for_gateway_launch"),
                "should carry the gateway mapping verdict, got: {}",
                error.message
            );
        }
    }

    #[test]
    fn codex_is_rejected_under_current_default_sandbox() {
        let error =
            validate_gateway_reviewer_for_enrollment(&ProviderName::Codex, true, false).unwrap_err();
        assert_eq!(error.code, "automation_gateway_reviewer_unsupported");
        assert!(
            error
                .message
                .contains(CODEX_DANGER_FULL_ACCESS_UNSUPPORTED)
        );
    }

    #[test]
    fn claude_code_passes_and_fake_requires_test_run() {
        assert!(
            validate_gateway_reviewer_for_enrollment(&ProviderName::ClaudeCode, true, false)
                .is_ok()
        );
        // 真实运行 Fake 无 gateway dialect；仅测试运行豁免。
        assert!(
            validate_gateway_reviewer_for_enrollment(&ProviderName::Fake, true, false).is_err()
        );
        assert!(
            validate_gateway_reviewer_for_enrollment(&ProviderName::Fake, true, true).is_ok()
        );
        // 非 gateway 路径不预检。
        assert!(
            validate_gateway_reviewer_for_enrollment(&ProviderName::KimiCode, false, false).is_ok()
        );
        // 测试模式不放行 Pi/KimiCode。
        assert!(
            validate_gateway_reviewer_for_enrollment(&ProviderName::KimiCode, true, true).is_err()
        );
    }
}
