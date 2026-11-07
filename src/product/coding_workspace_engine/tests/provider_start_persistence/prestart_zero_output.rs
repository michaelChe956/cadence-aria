//! T25 spawn 前零输出失败不落空 raw 引用的契约测试。
//! 从 provider_start_persistence.rs 拆出以满足 large_file_guard 的登记上限
//! (纯移动,行为不变)。
use super::*;

#[tokio::test]
async fn modern_start_io_invocation_keeps_typed_error_without_empty_raw_ref() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let role_run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::Initial,
            Some("coding_invocation_start_io".to_string()),
        )
        .expect("role run");
    let (event_tx, _event_rx) = mpsc::channel(8);
    let provider = std::sync::Arc::new(StartIoInvocationProvider::default());
    // REQ-ROUTE-01：快照 attempt 的 provider run 只经 gateway 启动。
    let engine = super::super::provider_gateway_validated_input::engine_with_fixture_gateway(
        &store,
        event_tx,
        "project_0001",
        provider.clone(),
    );
    let (legacy_input, input) = provider_invocation_inputs(&attempt);
    let (input, validated_input) =
        super::prepare_coder_validated_launch(&engine, &store, &attempt, input);
    let provider_name = ProviderName::Codex;
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let outcome = engine
        .run_provider_stream_invocation(CodingProviderStreamRun {
            attempt: &attempt,
            node_id: "coding_invocation_start_io",
            role_run: Some(&role_run),
            provider: provider.as_ref(),
            legacy_input: &legacy_input,
            input,
            provider_name: &provider_name,
            provider_role: CodingProviderRole::Coder,
            command_rx: &mut command_rx,
            allow_legacy_stream_fallback: false,
            timeout: None,
            timeout_reason_code: None,
            suppress_failure_side_effects: false,
            validated_input: Some(validated_input),
        })
        .await;

    assert!(matches!(
        &outcome,
        ProviderInvocationOutcome::RetryableTransport {
            failure: RetryableProviderFailure::StartIo,
            reason_code,
            message,
            partial_output,
        } if reason_code == "provider_start_io"
            && message.contains("ProviderExecutionFailed")
            && message.contains("Text file busy")
            && partial_output.is_empty()
    ));
    assert_eq!(provider.starts.load(Ordering::SeqCst), 1);
    // spawn 前零输出失败不落空 raw 引用:recoverable_failed_reviewer_run
    // 以"raw refs 非空"表达 provider 真有产出证据,空文件引用会毒化
    // 恢复判据(把可恢复的 spawn 前失败变成死门)。
    let persisted_run = store
        .get_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
        )
        .expect("persisted role run");
    assert!(
        persisted_run.raw_provider_output_refs.is_empty(),
        "pre-start zero-output failure must not persist an empty raw output ref"
    );
    assert!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates")
            .is_empty()
    );
}
