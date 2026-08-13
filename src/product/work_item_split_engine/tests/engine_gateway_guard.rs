// `engine.rs::invoke_provider` 的 sync 死路径 fail-closed 防线测试。
//
// 防线契约：`logical_repository_id.is_some()`（逻辑代码库仓库）必须经
// `LogicalCodebaseProviderGateway` 启动，禁止直连真实 provider（对应
// REQ-ENV-01/02「无政策不得启动」）。该同步路径当前为死代码，但若未来被
// 复活用于逻辑代码库仓库，会绕过 gateway——因此必须在 adapter.run 之前
// fail-closed。Legacy 单仓（`logical_repository_id: None`）行为零变化，
// 仍走直接 adapter 路径。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::cross_cutting::provider_adapter::{ProviderAdapter, ProviderAdapterError};
use crate::product::app_paths::ProductAppPaths;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::models::ProviderName;
use crate::product::work_item_split_engine::WorkItemSplitEngine;
use crate::protocol::contracts::{AdapterInput, AdapterOutput};

/// 记录 `run` 是否被调用的 adapter。用于证明 fail-closed 防线在
/// `spawn_blocking(adapter.run)` 之前返回，adapter 从未被调用。
struct RecordingAdapter {
    invoked: Arc<AtomicBool>,
}

impl RecordingAdapter {
    fn new(invoked: Arc<AtomicBool>) -> Self {
        Self { invoked }
    }
}

impl ProviderAdapter for RecordingAdapter {
    fn run(&self, _input: &AdapterInput) -> Result<AdapterOutput, ProviderAdapterError> {
        self.invoked.store(true, Ordering::SeqCst);
        Err(ProviderAdapterError::execution_failed(
            Some(1),
            "adapter.run must not be reached behind the gateway guard",
            "adapter.run must not be reached behind the gateway guard",
            0,
        ))
    }
}

fn logical_repository() -> RepositoryRecord {
    let (_, _, mut repository) = split_prompt_fixture();
    repository.logical_repository_id = Some(LogicalRepositoryId(uuid::Uuid::nil()));
    repository
}

fn engine(invoked: Arc<AtomicBool>) -> WorkItemSplitEngine {
    WorkItemSplitEngine::new(Arc::new(RecordingAdapter::new(invoked)))
}

fn lifecycle() -> LifecycleStore {
    LifecycleStore::new(ProductAppPaths::new("/tmp/aria-work-item-split-engine-t8"))
}

#[tokio::test]
async fn logical_repository_generate_fails_closed_with_gateway_required() {
    let invoked = Arc::new(AtomicBool::new(false));
    let (request, issue, _) = split_prompt_fixture();
    let repository = logical_repository();
    let lifecycle = lifecycle();

    let error = engine(invoked.clone())
        .generate(
            &request,
            &lifecycle,
            &issue,
            &repository,
            ProviderName::ClaudeCode,
        )
        .await
        .expect_err("logical codebase work item split must fail closed");

    assert_eq!(error.code, "logical_provider_gateway_required");
    assert!(
        !invoked.load(Ordering::SeqCst),
        "adapter.run must not be invoked for a logical codebase repository"
    );
}

#[tokio::test]
async fn logical_repository_generate_revision_fails_closed_with_gateway_required() {
    let invoked = Arc::new(AtomicBool::new(false));
    let (request, issue, _) = split_prompt_fixture();
    let repository = logical_repository();
    let lifecycle = lifecycle();

    let error = engine(invoked.clone())
        .generate_revision(
            &request,
            &lifecycle,
            &issue,
            &repository,
            ProviderName::Codex,
            &[],
            &[],
        )
        .await
        .expect_err("logical codebase revision must fail closed");

    assert_eq!(error.code, "logical_provider_gateway_required");
    assert!(
        !invoked.load(Ordering::SeqCst),
        "adapter.run must not be invoked for a logical codebase repository"
    );
}

#[tokio::test]
async fn legacy_repository_generate_still_invokes_adapter_directly() {
    let invoked = Arc::new(AtomicBool::new(false));
    let (request, issue, repository) = split_prompt_fixture();
    let lifecycle = lifecycle();

    let error = engine(invoked.clone())
        .generate(
            &request,
            &lifecycle,
            &issue,
            &repository,
            ProviderName::ClaudeCode,
        )
        .await
        .expect_err("recording adapter returns a controlled error");

    // 防线不触发：错误来自 adapter（经 map_provider_adapter_error 映射），
    // 且 adapter.run 确实被调用——Legacy 路径行为零变化。
    assert_eq!(error.code, "work_item_split_provider_error");
    assert!(
        invoked.load(Ordering::SeqCst),
        "legacy single-repo path must still call adapter.run directly"
    );
}
