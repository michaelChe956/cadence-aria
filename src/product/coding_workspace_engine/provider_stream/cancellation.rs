//! D①（诊断打点，不改行为）：provider_stream 取消位点审计打点。

use super::*;

/// provider_stream 各 `cancel.cancel()` 位点统一打点名取消者（attempt_key +
/// role_run_id + 触发原因 + 阶段）。勘察结论：握手期外部取消会被 biased-select
/// 的 result 分支 masking 成「handshake cancelled」错误文案，durable 事件与返回
/// 值均无法指认取消者；服务端日志是确证 H1（同 attempt_key 重试清理 abort 误
/// 杀）/H2（显式 abort）的唯一可靠信号面。触发原因枚举文案：
/// `engine_cancellation`（engine/runner token 取消）、`registry_abort_attempt`
/// （coding_run_registry 打点）、`runner_abort_attempt_command`（会话期
/// AbortAttempt 命令）、`provider_stream_timeout`/`choice_timeout`（超时）、
/// `provider_start_persistence_failure`（provider_start 审计落盘失败清理）。
pub(super) fn warn_cancellation_site(
    attempt: &CodingExecutionAttempt,
    role_run: Option<&CodingRoleRun>,
    trigger: &str,
    phase: &str,
) {
    // aria 二进制未安装 tracing subscriber（D① 落地后实测打点进黑洞）；生产构建
    // 用 aria-choice-diag 同款 eprintln 直写保证可见，测试构建保留 tracing 以
    // 维持既有捕获断言。
    #[cfg(not(test))]
    eprintln!(
        "[aria-cancellation] provider_stream cancelling child token trigger={} phase={} project_id={} issue_id={} attempt_id={} role_run_id={}",
        trigger,
        phase,
        attempt.project_id,
        attempt.issue_id,
        attempt.id,
        role_run.map(|run| run.id.as_str()).unwrap_or("none")
    );
    #[cfg(test)]
    tracing::warn!(
        trigger = trigger,
        phase = phase,
        project_id = %attempt.project_id,
        issue_id = %attempt.issue_id,
        attempt_id = %attempt.id,
        role_run_id = role_run.map(|run| run.id.as_str()).unwrap_or("none"),
        "cancellation site: provider_stream cancelling child token"
    );
}
