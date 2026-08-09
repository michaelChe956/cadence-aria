//! Task 12:逻辑代码库真实 provider 不得回落到 legacy `run_streaming` bridge,
//! 也不允许在无 `LogicalCodebaseProviderGateway` 时裸 `start` 真实 provider。
//!
//! 本模块的 fixture 构造一个携带 `target_snapshot`(逻辑代码库 target)的 attempt,
//! coder provider 设为真实 provider(`Codex`),且 adapter 的 `start` 返回
//! "not implemented"(即触发 legacy fallback 条件)。引擎不注入 gateway,因此
//! `validated_input` 为 `None`——这正是 Task 12 需要关闭的场景。

use super::*;

use crate::product::coding_models::AttemptTargetSnapshot;
use crate::product::logical_codebase::RepositoryCheckoutId;

/// Task 12 fixture:一个真实 provider(`ProviderName::Codex`)的 adapter,其 `start`
/// 返回 "not implemented"(即触发 legacy fallback 条件),并统计 `run_streaming`
/// 调用次数。配合一个携带 `target_snapshot`(逻辑代码库 target)的 attempt,
/// 验证逻辑代码库真实 provider 不会回落到 legacy `run_streaming` bridge,
/// 也不允许在无 gateway 时启动。
struct LogicalCodingLegacyFallbackFixture {
    provider: UnimplementedStartCountingAdapter,
    store: CodingAttemptStore,
    attempt: CodingExecutionAttempt,
    _root: tempfile::TempDir,
}

impl LogicalCodingLegacyFallbackFixture {
    fn adapter(&self) -> &UnimplementedStartCountingAdapter {
        &self.provider
    }

    fn build_engine(&self) -> CodingWorkspaceEngine {
        let (tx, _rx) = mpsc::channel(32);
        CodingWorkspaceEngine::new(self.store.clone(), GitWorkspaceService::new(), tx)
    }

    async fn run_coder(&self) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let engine = self.build_engine();
        let (_command_tx, mut command_rx) = mpsc::channel(1);
        engine
            .execute_coding_with_commands(
                &self.attempt,
                &self.provider,
                &CodingExecutionContext::default(),
                &mut command_rx,
            )
            .await
    }
}

struct UnimplementedStartCountingAdapter {
    #[allow(dead_code)]
    starts: AtomicUsize,
    legacy_run_streaming_calls: AtomicUsize,
}

impl UnimplementedStartCountingAdapter {
    fn new() -> Self {
        Self {
            starts: AtomicUsize::new(0),
            legacy_run_streaming_calls: AtomicUsize::new(0),
        }
    }

    fn legacy_run_streaming_calls(&self) -> usize {
        self.legacy_run_streaming_calls.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for UnimplementedStartCountingAdapter {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "streaming provider start is not implemented",
            0,
        ))
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        self.legacy_run_streaming_calls
            .fetch_add(1, Ordering::SeqCst);
        // 返回一个立即关闭的 receiver,表示不应被调用。
        let (_tx, rx) = mpsc::channel(1);
        Ok(rx)
    }
}

/// 构造一个携带逻辑代码库 target(`target_snapshot.is_some()`)的 running attempt,
/// coder provider 为 `Codex`(真实 provider)。不注入 `logical_provider_gateway`,
/// 因此 `validated_input` 为 `None`——这是 Task 12 需要关闭的场景。
fn logical_coding_fixture_with_unimplemented_stream_start() -> LogicalCodingLegacyFallbackFixture {
    let (root, store, attempt) = running_attempt_with_worktree();
    // 设为真实 provider(Codex),使逻辑路径不能以 Fake 例外放行。
    let mut provider_config = store
        .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role provider config");
    provider_config.coder = ProviderName::Codex;
    store
        .update_role_provider_config_snapshot(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            provider_config,
        )
        .expect("update role provider config");
    // 覆写 target_snapshot 为逻辑代码库 target。
    let now = "2026-08-09T00:00:00Z".to_string();
    let mut attempt_with_target = attempt.clone();
    attempt_with_target.target_snapshot = Some(AttemptTargetSnapshot {
        logical_repository_id: crate::product::logical_codebase::LogicalRepositoryId(
            uuid::Uuid::nil(),
        ),
        checkout_id: RepositoryCheckoutId(uuid::Uuid::nil()),
        physical_repository_id: "repository_0001".to_string(),
        canonical_path: attempt_with_target
            .worktree_path
            .clone()
            .unwrap_or_else(|| PathBuf::from("/tmp/worktree")),
        git_dir_identity: "git-dir-identity".to_string(),
        revision: None,
        policy_digest: String::new(),
        membership_revision: 1,
        captured_at: now,
        capture_source: "test".to_string(),
    });
    let attempt_path = store
        .paths()
        .issue_lifecycle_root(
            &attempt_with_target.project_id,
            &attempt_with_target.issue_id,
        )
        .join("coding-attempts")
        .join(format!("{}.json", attempt_with_target.id));
    crate::product::json_store::write_json(&attempt_path, &attempt_with_target)
        .expect("write attempt with target snapshot");
    LogicalCodingLegacyFallbackFixture {
        provider: UnimplementedStartCountingAdapter::new(),
        store,
        attempt: attempt_with_target,
        _root: root,
    }
}

#[tokio::test]
async fn logical_coding_never_falls_back_to_legacy_stream_when_start_is_unimplemented() {
    // Task 12:逻辑代码库真实 provider 在无 gateway(无 validated_input)时必须
    // fail-closed(`logical_provider_gateway_required`),不得回落到 legacy
    // `run_streaming` bridge,也不允许裸 `start` 启动真实 provider。这是「禁止裸
    // AdapterInput/StreamingProviderInput 直接启动真实 provider」的端到端断言。
    let fixture = logical_coding_fixture_with_unimplemented_stream_start();
    let error = fixture.run_coder().await.unwrap_err();

    assert!(
        error
            .to_string()
            .contains("logical_provider_gateway_required"),
        "expected logical_provider_gateway_required error, got: {error}"
    );
    assert_eq!(
        fixture.adapter().legacy_run_streaming_calls(),
        0,
        "legacy run_streaming bridge must not be invoked for logical targets"
    );
}
