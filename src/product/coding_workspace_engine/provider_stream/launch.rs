//! provider 启动路径选择（Task 11）与 gateway 错误映射。

use super::*;

/// Task 11:按是否携带 validated input + 是否注入 gateway 选择 provider 启动路径。
///
/// - 两者均存在:经 `LogicalCodebaseProviderGateway::start_streaming` 启动,使政策
///   校验、canonical 复验、resume fail-closed 都在 gateway 内完成并留 audit。
/// - 否则(传统/非逻辑 issue):直接 `provider.start`,保留既有行为。
///
/// 返回 boxed future 以便外层 `tokio::select!` 统一内联。gateway 错误映射为
/// `ProviderAdapterError`,使其与直接 adapter 错误在 stream 层等价处理。
pub(super) fn launch_provider_session<'a>(
    provider: &'a dyn StreamingProviderAdapter,
    input: StreamingProviderInput,
    cancel: CancellationToken,
    validated: Option<crate::cross_cutting::session_launch::ValidatedStreamingProviderInput>,
    gateway: Option<
        &std::sync::Arc<crate::product::logical_codebase::LogicalCodebaseProviderGateway>,
    >,
) -> std::pin::Pin<
    Box<
        dyn std::future::Future<
                Output = Result<
                    crate::cross_cutting::streaming_provider::ProviderSession,
                    ProviderAdapterError,
                >,
            > + Send
            + 'a,
    >,
> {
    if let (Some(validated), Some(gateway)) = (validated, gateway) {
        let gateway = gateway.clone();
        Box::pin(async move {
            gateway
                .start_streaming(validated, cancel)
                .await
                .map_err(provider_adapter_error_from_gateway)
        })
    } else {
        // 非 gateway 路径(传统/非逻辑 issue):直接 adapter start。逻辑代码库 feature
        // 在入口处由 `validated_input`/`logical_provider_gateway` 的存在与否分流,
        // 使旧 API 行为不被本工作包扩大。
        Box::pin(async move { provider.start(input, cancel).await })
    }
}

/// 把 gateway 错误映射为 `ProviderAdapterError`,使 provider stream 层把「政策门/
/// 复验拒绝」与「adapter 运行失败」统一为 transport 错误。fail-closed 维度保留在
/// `details` 中供上游诊断。
fn provider_adapter_error_from_gateway(
    error: crate::product::logical_codebase::ProviderGatewayError,
) -> ProviderAdapterError {
    use crate::protocol::contracts::TimeoutStatus;
    use crate::protocol::provider_errors::ProviderErrorCode;
    ProviderAdapterError {
        code: ProviderErrorCode::ProviderUnavailable,
        details: error.to_string(),
        stdout: String::new(),
        stderr: String::new(),
        exit_code: None,
        timeout_status: TimeoutStatus::NotTimedOut,
        duration_ms: 0,
    }
}
