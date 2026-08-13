//! gateway 错误码归一与稳定码映射（T11 §3 稳定码表收口）。
//!
//! 从 `support.rs` 拆出以保持 large_file_guard 1200 行红线（T11 fix round 2）。
//! 生产活跃路径把 `ProviderGatewayError` 扁平化为 `ProviderAdapterError{ProviderUnavailable,
//! details=error.to_string()}` 或 `CodingWorkspaceEngineError::ProviderStream(error.to_string())`,
//! 稳定码前缀只留在 details/message 字符串里;本模块把 `provider_gateway_*` 前缀归一为
//! 稳定码,与 `web/error.rs` 的 HttpStatus 段构成「variant/前缀 → 稳定码 → HTTP 状态」两级映射。

use serde_json::json;

use crate::product::coding_workspace_engine::CodingWorkspaceEngineError;
use crate::product::logical_codebase::ProviderGatewayError;
use crate::web::error::ApiError;

/// `ProviderGatewayError` → 稳定码前缀表（T11 §3 收口）。
///
/// gateway 的 `thiserror` 前缀即稳定码；此处把 variant 集中归一为稳定码字符串，
/// 供 web 层把「政策门/复验拒绝」与「adapter 运行失败」区分开，避免调用点各自解析
/// `to_string()` 前缀导致错误码漂移。与 `web/error.rs` 的 HttpStatus 段构成
/// 「variant → 稳定码 → HTTP 状态」两级集中映射。
///
/// 注：`ManagedSettingsActive` 自 Task 11 起不再产生（改为标注后放行），此处仍保留
/// 其前缀映射以保持穷尽性；`Adapter` 是真实 provider 运行失败，归为
/// `provider_gateway_adapter`（默认 500，非政策门）。
pub(crate) fn provider_gateway_error_code(error: &ProviderGatewayError) -> &'static str {
    match error {
        ProviderGatewayError::PolicyMissing(_) => "provider_gateway_policy_missing",
        ProviderGatewayError::Policy(_) => "provider_gateway_policy",
        ProviderGatewayError::Target(_) => "provider_gateway_target",
        ProviderGatewayError::TargetMismatch { .. } => "provider_gateway_target_mismatch",
        ProviderGatewayError::MissingCwd => "provider_gateway_missing_cwd",
        ProviderGatewayError::UnsupportedCapability(_) => "provider_gateway_capability",
        ProviderGatewayError::ProviderUnavailable(_) => "provider_gateway_unavailable",
        ProviderGatewayError::RegistryLookup(_) => "provider_gateway_registry_lookup",
        ProviderGatewayError::PolicyDrift { .. } => "provider_gateway_policy_drift",
        ProviderGatewayError::ResumeNotSupported => "provider_gateway_resume_not_supported",
        ProviderGatewayError::ManagedSettingsActive => "provider_gateway_managed_settings_active",
        ProviderGatewayError::Adapter(_) => "provider_gateway_adapter",
    }
}

/// 把承载在 `details` 字符串里的 gateway 错误前缀归一为稳定码（T11 fix round 收口）。
///
/// 稳定码前缀只留在扁平化后的 `details` 里。本 helper 从 `details` 开头识别
/// `provider_gateway_*` 前缀(与 `provider_gateway_error_code` 输出的稳定码集一致),
/// 返回归一化稳定码;否则 `None`。
///
/// `codex_danger_full_access_unsupported` 是 `UnsupportedCapability` 的 reason 载荷,防御性地
/// 归一为 `provider_gateway_capability`(两者在 `web/error.rs` 同映射 403)。
pub(crate) fn gateway_stable_code_from_details(details: &str) -> Option<&'static str> {
    if details.starts_with("codex_danger_full_access_unsupported") {
        return Some("provider_gateway_capability");
    }
    // 更具体的前缀在前:避免 `provider_gateway_policy` 吞掉 `provider_gateway_policy_missing`
    // / `provider_gateway_policy_drift`,避免 `provider_gateway_target` 吞掉
    // `provider_gateway_target_mismatch`。
    const GATEWAY_PREFIXES: &[&str] = &[
        "provider_gateway_policy_missing",
        "provider_gateway_policy_drift",
        "provider_gateway_target_mismatch",
        "provider_gateway_resume_not_supported",
        "provider_gateway_managed_settings_active",
        "provider_gateway_registry_lookup",
        "provider_gateway_missing_cwd",
        "provider_gateway_capability",
        "provider_gateway_unavailable",
        "provider_gateway_policy",
        "provider_gateway_target",
        "provider_gateway_adapter",
    ];
    GATEWAY_PREFIXES
        .iter()
        .copied()
        .find(|prefix| details.starts_with(prefix))
}

/// 把 `CodingWorkspaceEngineError` 中承载 gateway 错误的 variant(`ProviderStream` /
/// `ProviderAdapter`)归一为稳定码 `ApiError`;未命中或非 gateway variant 返回 `None`。
pub(crate) fn coding_gateway_api_error(error: &CodingWorkspaceEngineError) -> Option<ApiError> {
    match error {
        CodingWorkspaceEngineError::ProviderStream(message) => {
            gateway_stable_code_from_details(message)
                .map(|code| ApiError::runtime(code, message.clone(), json!({})))
        }
        CodingWorkspaceEngineError::ProviderAdapter(adapter_error) => {
            gateway_stable_code_from_details(&adapter_error.details)
                .map(|code| ApiError::runtime(code, adapter_error.details.clone(), json!({})))
        }
        _ => None,
    }
}
