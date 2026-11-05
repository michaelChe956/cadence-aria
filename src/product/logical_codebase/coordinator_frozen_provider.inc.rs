use crate::product::logical_codebase::provider_selection::provider_from_wire;

/// 片4（REQ-BOOT-07）「重试不换人」的结构性保证：执行期与重试期的 provider
/// 唯一来源是 durable `operation.input.provider`，选择器永不在 execute/retry
/// 路径重跑。未知串与 `Fake` 一律 fail-closed（绝不回落默认 provider），失败
/// 以 `ProviderTurnDiagnosed` 落账（记录携带被拒的 provider 名与结构化
/// cause），operation 判 Failed、所有 step 保持未启动语义（step: None）。
impl AggregateInitializationCoordinator {
    fn frozen_provider(
        &self,
        project_id: &str,
        operation_id: &str,
        step: AggregateInitializationStepKind,
    ) -> Result<crate::product::models::ProviderName, AggregateInitializationError> {
        let operation = self.load_operation(project_id, operation_id)?;
        match provider_from_wire(&operation.input.provider) {
            Some(provider) => Ok(provider),
            None => {
                let error = AggregateInitializationError::ProviderTurnDiagnosed {
                    step,
                    reason: format!(
                        "aggregate recipe provider {} is not launchable; no fallback provider",
                        operation.input.provider
                    ),
                    retryable: false,
                    provider: Some(operation.input.provider.clone()),
                    diagnostic: TurnDiagnostic {
                        cause: Some("provider_unsupported_for_aggregate_recipe".to_string()),
                        output: None,
                    },
                };
                Err(self.persist_execution_failure(project_id, operation_id, None, error))
            }
        }
    }
}

/// 7.7（启动期 `ProviderAdapterError` → cause / diagnostic）：gateway
/// provider turn 的诊断输出：**优先用真实输出**；输出为空（provider 根本没
/// 产出输出 —— 例如启动前就被拒、协议解析失败）时改用**真实原因**。
///
/// 否则失败面只显示「未捕获到 provider 输出。」这句无信息量的兜底串，把真因
/// （adapter details / turn reason）完全遮住 ⇒ 用户与 agent 都无法判断真实
/// 问题（本仓 2026-11-05 真机事故：真实原因 `pi lc tool policy is not
/// canonicalizable: ...` 只落在 operation 的 `action` 字段里，界面看不到）。
pub(crate) fn provider_turn_diagnostic_output(output: &str, reason: &str) -> String {
    use crate::cross_cutting::diagnostic_redaction::sanitize_turn_output;
    if output.trim().is_empty() {
        return sanitize_turn_output(reason);
    }
    sanitize_turn_output(output)
}

/// `start_streaming` 的启动期失败。底层是 adapter 错误时，`cause` 取其
/// code 子码（`ProviderErrorCode::as_str()`，如 `provider_unauthorized`），
/// stdout/stderr 合并后走块2 唯一脱敏入口（路径降级 + 凭据 `[REDACTED]` +
/// 4096 字节/40 行截断）——不再把两者压进 reason 串丢弃。stdout/stderr **皆空**
/// 时（进程压根没拉起）改用 `reason` 承载真因，不落无信息量的兜底串
/// （见 `provider_turn_diagnostic_output`）。非 adapter 的
/// gateway 错误 reason 形态不变、不带子码（`cause`/`output` 双 `None`，
/// error_record 映射与既有 `ProviderTurn` 等价）。
pub(crate) fn start_streaming_failure(
    step: AggregateInitializationStepKind,
    error: crate::product::logical_codebase::ProviderGatewayError,
) -> AggregateInitializationError {
    let reason = format!("gateway start_streaming failed: {error}");
    let (cause, output) = match &error {
        crate::product::logical_codebase::ProviderGatewayError::Adapter(adapter) => (
            Some(adapter.code.as_str().to_string()),
            Some(provider_turn_diagnostic_output(
                &format!("{}\n{}", adapter.stdout, adapter.stderr),
                &reason,
            )),
        ),
        _ => (None, None),
    };
    AggregateInitializationError::ProviderTurnDiagnosed {
        step,
        reason,
        retryable: true,
        provider: None,
        diagnostic: TurnDiagnostic { cause, output },
    }
}
