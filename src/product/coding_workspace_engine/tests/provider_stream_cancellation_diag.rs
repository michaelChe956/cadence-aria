// D①（诊断打点）测试：provider_stream 取消位点的 tracing 打点名取消者。

use super::provider_start_persistence::provider_start_persistence_fixture;
use super::*;
use std::sync::Arc;
use std::time::Duration;

/// D①（红→绿）：engine 级取消（parent token）在 provider_start 窗口触发且
/// provider start 未先行返回时，必须打点名取消者（attempt_key + role_run_id +
/// 触发原因 + 阶段）。注：若 provider start 观察到 child token 后自行返回错
/// 误，biased-select 的 result 分支会抢先（即勘察所述 masking）——本测试锁定
/// 分支体真正执行时的打点。
struct HangingStartProvider {
    entered: Arc<tokio::sync::Notify>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for HangingStartProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.entered.notify_one();
        // 不观察 cancel：保证 engine 级取消分支体真正执行。
        std::future::pending::<()>().await;
        unreachable!()
    }
}

#[tokio::test]
async fn coding_provider_stream_engine_cancellation_logs_cancellation_site() {
    use crate::cross_cutting::tracing_capture::SharedTracingCapture;

    let fixture = provider_start_persistence_fixture().await;
    let cancellation = CancellationToken::new();
    let entered = Arc::new(tokio::sync::Notify::new());
    let provider = Arc::new(HangingStartProvider {
        entered: entered.clone(),
    });
    // REQ-ROUTE-01：快照 attempt 的 provider run 只经 gateway 启动；取消沿用
    // 同一 engine 取消位点（provider_start 窗口）。
    let engine = fixture
        .engine_with(provider.clone())
        .with_cancellation(cancellation.clone());
    let capture = SharedTracingCapture::global();

    let execute = engine.execute_code_review(&fixture.attempt, provider.as_ref());
    let cancel_parent = async {
        tokio::time::timeout(Duration::from_millis(250), entered.notified())
            .await
            .expect("provider start did not receive control");
        cancellation.cancel();
    };
    let (result, ()) = tokio::join!(execute, cancel_parent);

    assert!(result.is_err(), "cancelled provider run must stop");
    let logs = capture.captured();
    assert!(logs.contains("engine_cancellation"), "logs: {logs}");
    assert!(logs.contains("provider_start"), "logs: {logs}");
    assert!(logs.contains(&fixture.attempt.project_id), "logs: {logs}");
    assert!(logs.contains(&fixture.attempt.issue_id), "logs: {logs}");
    assert!(logs.contains(&fixture.attempt.id), "logs: {logs}");
    assert!(logs.contains("role_run_id="), "logs: {logs}");
}
