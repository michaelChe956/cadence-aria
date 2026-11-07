// 片4（REQ-BOOT-07）测试前置 helper 分片：provider health 全可用注入 +
// claude_code Confirmed 种子。本文件通过 `include!` 进入
// aggregate_initialization/tests.inc.rs 的同一 `mod tests`，拆分只为满足
// large_file_guard 的 1200 行上限；helper 体、签名与注释逐字保持不变。

/// 片4（REQ-BOOT-07）：为「期望创建成功」的既有夹具补齐选择器前置——
/// (1) 注入全可用 health（stub runner：四家版本探测全部成功，非降级、
/// 无真实探针）；(2) 把 claude_code 种为 Confirmed（与 provider turn 用例
/// 同一种子 helper）。选择器据此稳定选中 claude_code，与接线前（固定
/// claude_code）行为等价、零回归；选择期失败面由 tests_parts/
/// selection_failure.inc.rs 的专用夹具逐场景构造，不经本 helper。
fn with_provider_health_all_available(
    state: WebAppState,
    root: &std::path::Path,
) -> WebAppState {
    struct AllAvailableRunner;
    #[async_trait::async_trait]
    impl crate::cross_cutting::bounded_command_runner::BoundedCommandRunner
        for AllAvailableRunner
    {
        async fn run(
            &self,
            request: crate::cross_cutting::bounded_command_runner::BoundedCommandRequest,
        ) -> Result<
            crate::cross_cutting::bounded_command_runner::BoundedCommandResult,
            crate::cross_cutting::bounded_command_runner::BoundedCommandError,
        > {
            // 版本取各家最低兼容版本（与 providers.rs 的探测断言同源），
            // 避免「低于最低版本」把可用性改写。
            let version = match request.executable.as_str() {
                "codex" => "2.0",
                "pi" => "0.83.0",
                "kimi" => "0.34.0",
                _ => "1.0",
            };
            Ok(crate::cross_cutting::bounded_command_runner::BoundedCommandResult {
                exit_code: Some(0),
                stdout: version.to_string(),
                stderr: String::new(),
                timed_out: false,
                cancelled: false,
                stdout_truncated: false,
                stderr_truncated: false,
                duration_ms: 1,
            })
        }
    }
    let runner: std::sync::Arc<
        dyn crate::cross_cutting::bounded_command_runner::BoundedCommandRunner,
    > = Arc::new(AllAvailableRunner);
    let health = std::sync::Arc::new(
        crate::cross_cutting::provider_health::ProviderHealthService::with_dependencies(
            crate::cross_cutting::aria_state_paths::AriaStatePaths::from_workspace_root(root),
            runner.clone(),
            std::sync::Arc::new(
                crate::cross_cutting::provider_health::SystemProviderHealthClock,
            ),
            std::time::Duration::from_secs(1),
            4096,
        ),
    );
    let gate = std::sync::Arc::new(
        crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate::new(
            health.clone(),
        ),
    );
    state.with_provider_health(health, gate, runner)
}

/// [`with_provider_health_all_available`] + 把 claude_code 种为 Confirmed：
/// 选择器据此稳定选中 claude_code（与 provider turn 用例同一种子 helper）。
///
/// capability 记录的作用域是**项目级全局**（owner 2026-11-04）⇒ 不再按 LC
/// 分叉播种，故无 lc_id 参数。
fn with_selector_defaulting_to_claude(
    state: WebAppState,
    root: &std::path::Path,
    paths: &ProductAppPaths,
) -> WebAppState {
    crate::web::handlers::automation_enrollment_test_support::seed_confirmed_capability_records(
        paths,
        "project_0001",
        &[crate::product::logical_codebase::provider_gateway::ProviderRefType::ClaudeCode],
    );
    with_provider_health_all_available(state, root)
}
