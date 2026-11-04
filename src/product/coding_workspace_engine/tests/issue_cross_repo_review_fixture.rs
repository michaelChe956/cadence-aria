// issue 级跨仓最终评审 engine 测试的共享夹具（`issue_cross_repo_review.rs` /
// `issue_cross_repo_review_ack.rs` 两文件共用）。
//
// 覆盖：身份常量、计数/挂起假 provider、多仓 issue 与触发形态 fixture、
// provider 触发边界、评审记录轮询。留痕/时序专属夹具（gated provider、
// group readiness）随用例留在 `issue_cross_repo_review_ack.rs`。

use super::*;
use crate::cross_cutting::streaming_provider::ProviderCompletion;
use crate::product::coding_models::{
    AttemptTargetSnapshot, CodingRoleProviderConfigSnapshot, IssueCrossRepoReview,
};
use crate::product::logical_codebase::{LogicalRepositoryId, RepositoryCheckoutId};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

pub(super) const PROJECT_ID: &str = "project_0001";
pub(super) const ISSUE_ID: &str = "issue_0001";
/// 夹具 issue 的多仓逻辑代码库 id（P2-3：issue 形态判据是
/// `logical_codebase_id`，不是 attempt 快照仓数）。
pub(super) const CROSS_REPO_LC_ID: &str = "lc_cross_repo_fixture";

/// 计数假 provider：每次 `start` 计一次启动并记录 prompt，随后立即以固定
/// 输出完成会话（无窗口、无命令交互）。
pub(super) struct CountingFakeProvider {
    output: String,
    starts: Arc<AtomicUsize>,
    prompts: Arc<Mutex<Vec<String>>>,
    inputs: Arc<Mutex<Vec<StreamingProviderInput>>>,
}

impl CountingFakeProvider {
    pub(super) fn new(output: &str) -> Self {
        Self {
            output: output.to_string(),
            starts: Arc::new(AtomicUsize::new(0)),
            prompts: Arc::new(Mutex::new(Vec::new())),
            inputs: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub(super) fn starts(&self) -> usize {
        self.starts.load(Ordering::SeqCst)
    }

    pub(super) fn first_prompt(&self) -> String {
        self.prompts.lock().unwrap()[0].clone()
    }

    /// 第一个收到的完整 input（P1-1：交给 adapter 前的策略/时限断言）。
    pub(super) fn first_input(&self) -> StreamingProviderInput {
        self.inputs.lock().unwrap()[0].clone()
    }

    /// 计数句柄（provider 被移入 `Arc<dyn StreamingProviderAdapter>` 后仍可观测）。
    pub(super) fn counters(&self) -> (Arc<AtomicUsize>, Arc<Mutex<Vec<String>>>) {
        (self.starts.clone(), self.prompts.clone())
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for CountingFakeProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        self.prompts.lock().unwrap().push(input.prompt.clone());
        self.inputs.lock().unwrap().push(input.clone());
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(4);
        let output = self.output.clone();
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed(ProviderCompletion::from_output(
                    output, None, None,
                )))
                .await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }
}

fn target_snapshot(index: usize) -> AttemptTargetSnapshot {
    AttemptTargetSnapshot {
        logical_repository_id: LogicalRepositoryId(Uuid::new_v4()),
        checkout_id: RepositoryCheckoutId(Uuid::new_v4()),
        physical_repository_id: format!("repository_{index:04}"),
        canonical_path: PathBuf::from(format!("/tmp/repository_{index:04}")),
        git_dir_identity: "sha256:test".to_string(),
        revision: Some("abcdef".to_string()),
        policy_digest: "policy_digest".to_string(),
        membership_revision: 1,
        captured_at: "2026-10-23T00:00:00Z".to_string(),
        capture_source: "test".to_string(),
    }
}

/// 建一个含 `repo_count` 个已完成 group attempt 的多仓 issue fixture：每仓
/// 一个真实 git worktree（base 之后各有一个增量提交，diff 非空）。
pub(super) fn cross_repo_fixture(
    repo_count: usize,
) -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingWorkspaceEngine,
    Vec<CodingExecutionAttempt>,
) {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    // P2-3：issue 形态判据是 issue 的 `logical_codebase_id`——显式执行入口的
    // 夹具必须建多仓 issue，否则引擎按「单仓不评审」兜底，评审不会执行。
    IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: PROJECT_ID.to_string(),
            repo_id: Some("repository_0000".to_string()),
            logical_codebase_id: CROSS_REPO_LC_ID.to_string(),
            title: "issue".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .expect("issue");
    let store = CodingAttemptStore::new(paths);
    let mut attempts = Vec::new();
    for index in 0..repo_count {
        let worktree = root.path().join(format!("worktree_{index}"));
        fs::create_dir_all(&worktree).expect("worktree dir");
        super::init_test_git_repo(&worktree);
        let base_head = super::git_stdout(&worktree, &["rev-parse", "HEAD"])
            .trim()
            .to_string();
        fs::write(
            worktree.join(format!("repo_{index}.txt")),
            format!("change {index}\n"),
        )
        .expect("change file");
        super::run_test_git(&worktree, &["add", "."]);
        super::run_test_git(&worktree, &["commit", "-m", &format!("change {index}")]);
        let attempt = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: format!("work_item_{index:04}"),
                base_branch: base_head,
                branch_name: format!("aria/issues/issue_0001/repo_{index}"),
                worktree_path: Some(worktree),
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::Fake,
                    reviewer: None,
                    review_rounds: 0,
                    permission_modes: Default::default(),
                },
                target_snapshot: Some(target_snapshot(index)),
                max_auto_rework: 1,
            })
            .expect("create attempt");
        // 合法迁移链：Created --(测试播种 admission)--> Running --> Completed。
        store
            .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("seed running attempt");
        let completed = store
            .update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::Completed,
            )
            .expect("complete attempt");
        attempts.push(completed);
    }
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    (root, store, engine, attempts)
}

/// 覆写全部 attempt 的角色 provider 快照（模拟存量 JSON 缺 `internal_reviewer`
/// 但 `code_reviewer` 在场 / 两者皆缺的形态）。
pub(super) fn seed_role_provider_config(
    store: &CodingAttemptStore,
    attempts: &[CodingExecutionAttempt],
    internal_reviewer: Option<ProviderName>,
    code_reviewer: Option<ProviderName>,
) {
    for attempt in attempts {
        store
            .update_role_provider_config_snapshot(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingRoleProviderConfigSnapshot {
                    coder: ProviderName::Fake,
                    code_reviewer: code_reviewer.clone(),
                    internal_reviewer: internal_reviewer.clone(),
                    review_rounds: 0,
                    permission_modes: Default::default(),
                },
            )
            .expect("seed role provider config");
    }
}

// ---------------------------------------------------------------------------
// Task 17：触发接线（REQ-GFR-CR-03「issue 级跨仓最终评审恰好一次」，
// 设计 B §七 E）。
// ---------------------------------------------------------------------------

/// 逻辑仓 id：固定 UUID 串（可控升序），`index` 越大 id **越小**——刻意让
/// 「按 logical_repository_id 升序」与「按 attempt 创建顺序」相反，使排序断言
/// 真实生效。
fn logical_repository_id(index: usize, repo_count: usize) -> Uuid {
    Uuid::parse_str(&format!(
        "00000000-0000-0000-0000-{:012}",
        repo_count - index
    ))
    .expect("logical repository id")
}

fn target_snapshot_with_logical_id(index: usize, repo_count: usize) -> AttemptTargetSnapshot {
    AttemptTargetSnapshot {
        logical_repository_id: LogicalRepositoryId(logical_repository_id(index, repo_count)),
        checkout_id: RepositoryCheckoutId(Uuid::new_v4()),
        physical_repository_id: format!("repository_{index:04}"),
        canonical_path: PathBuf::from(format!("/tmp/repository_{index:04}")),
        git_dir_identity: "sha256:test".to_string(),
        revision: Some("abcdef".to_string()),
        policy_digest: "policy_digest".to_string(),
        membership_revision: 1,
        captured_at: "2026-10-23T00:00:00Z".to_string(),
        capture_source: "test".to_string(),
    }
}

/// 「全部开发完成」的多仓 issue 夹具：issue + `repo_count` 个 work item +
/// `repo_count` 个已完成 attempt（每仓一个真实 git worktree，diff 非空）。
pub(super) fn cross_repo_trigger_fixture(
    repo_count: usize,
) -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingWorkspaceEngine,
    Vec<CodingExecutionAttempt>,
) {
    cross_repo_trigger_fixture_inner(repo_count, repo_count, Some(CROSS_REPO_LC_ID), &[])
}

/// 同上，但可指定每仓 attempt 的收尾形态：`true` = 停在「人工最终确认前」
/// （`WaitingForHuman` + `stage == FinalConfirm`，group 形态收口钩子的真实落点），
/// `false` / 缺省 = `Completed`。
pub(super) fn cross_repo_trigger_fixture_with_states(
    repo_count: usize,
    awaiting_final_confirm: &[bool],
) -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingWorkspaceEngine,
    Vec<CodingExecutionAttempt>,
) {
    cross_repo_trigger_fixture_inner(
        repo_count,
        repo_count,
        Some(CROSS_REPO_LC_ID),
        awaiting_final_confirm,
    )
}

/// 多仓 issue 但**全部 work item 同仓**（P2-3 用例形态）：`repo_count` 个
/// attempt 的材料只覆盖 `1` 个去重逻辑仓。
pub(super) fn cross_repo_trigger_fixture_single_repo(
    repo_count: usize,
) -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingWorkspaceEngine,
    Vec<CodingExecutionAttempt>,
) {
    cross_repo_trigger_fixture_inner(repo_count, 1, Some(CROSS_REPO_LC_ID), &[])
}

/// **单仓 issue**（`logical_codebase_id=None`，P2-3 负例形态）：attempt 快照
/// 仍横跨 `repo_count` 个逻辑仓——证明快照仓数不再是触发判据。
pub(super) fn single_repo_trigger_fixture(
    repo_count: usize,
) -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingWorkspaceEngine,
    Vec<CodingExecutionAttempt>,
) {
    cross_repo_trigger_fixture_inner(repo_count, repo_count, None, &[])
}

/// 全量夹具：issue（形态由 `issue_lc_id` 决定）+ `repo_count` 个 work item +
/// `repo_count` 个已完成 attempt（材料覆盖 `distinct_logical_repos` 个去重
/// 逻辑仓；`awaiting_final_confirm` 同 `cross_repo_trigger_fixture_with_states`）。
fn cross_repo_trigger_fixture_inner(
    repo_count: usize,
    distinct_logical_repos: usize,
    issue_lc_id: Option<&str>,
    awaiting_final_confirm: &[bool],
) -> (
    tempfile::TempDir,
    CodingAttemptStore,
    CodingWorkspaceEngine,
    Vec<CodingExecutionAttempt>,
) {
    let root = tempdir().expect("tempdir");
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    IssueStore::new(paths.clone())
        .create(CreateProductIssueInput {
            project_id: PROJECT_ID.to_string(),
            repo_id: Some("repository_0001".to_string()),
            logical_codebase_id: issue_lc_id.map_or_else(
                || crate::product::logical_codebase::legacy_logical_codebase_id(PROJECT_ID),
                str::to_string,
            ),
            title: "issue".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .expect("issue");
    let store = CodingAttemptStore::new(paths.clone());
    let mut attempts = Vec::new();
    for index in 0..repo_count {
        let work_item_id = format!("work_item_{index:04}");
        LifecycleStore::new(paths.clone())
            .create_work_item(CreateWorkItemInput {
                id: Some(work_item_id.clone()),
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: format!("repository_{index:04}"),
                title: format!("work item {work_item_id}"),
                plan_status: WorkItemPlanStatus::Confirmed,
                ..Default::default()
            })
            .expect("work item");
        let worktree = root.path().join(format!("worktree_{index}"));
        fs::create_dir_all(&worktree).expect("worktree dir");
        super::init_test_git_repo(&worktree);
        let base_head = super::git_stdout(&worktree, &["rev-parse", "HEAD"])
            .trim()
            .to_string();
        fs::write(
            worktree.join(format!("repo_{index}.txt")),
            format!("change {index}\n"),
        )
        .expect("change file");
        super::run_test_git(&worktree, &["add", "."]);
        super::run_test_git(&worktree, &["commit", "-m", &format!("change {index}")]);
        let attempt = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id,
                base_branch: base_head,
                branch_name: format!("aria/issues/issue_0001/repo_{index}"),
                worktree_path: Some(worktree),
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::Fake,
                    reviewer: None,
                    review_rounds: 0,
                    permission_modes: Default::default(),
                },
                target_snapshot: Some(target_snapshot_with_logical_id(
                    index % distinct_logical_repos.max(1),
                    distinct_logical_repos.max(1),
                )),
                max_auto_rework: 1,
            })
            .expect("create attempt");
        store
            .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("seed running attempt");
        let parked = awaiting_final_confirm.get(index).copied().unwrap_or(false);
        let settled = if parked {
            store
                .update_attempt_status(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    CodingAttemptStatus::WaitingForHuman,
                )
                .expect("park attempt at final confirm");
            store
                .update_attempt_stage(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    CodingExecutionStage::FinalConfirm,
                )
                .expect("stage final confirm")
        } else {
            store
                .update_attempt_status(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    CodingAttemptStatus::Completed,
                )
                .expect("complete attempt")
        };
        attempts.push(settled);
    }
    // 评审 provider 在场（`.internal_reviewer`）。
    seed_role_provider_config(&store, &attempts, Some(ProviderName::ClaudeCode), None);
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    (root, store, engine, attempts)
}

/// 跨仓评审的 provider 边界（调用边界注入 adapter）：无论引擎选中哪个 provider
/// 名称，都返回同一个假 provider。
pub(super) fn fake_review_provider(
    output: &str,
) -> (
    IssueCrossRepoReviewProviderResolver<'static>,
    Arc<AtomicUsize>,
    Arc<Mutex<Vec<String>>>,
) {
    let provider = CountingFakeProvider::new(output);
    let (starts, prompts) = provider.counters();
    let adapter: Arc<dyn StreamingProviderAdapter> = Arc::new(provider);
    let resolver =
        IssueCrossRepoReviewProviderResolver::new(move |_: &ProviderName| Some(adapter.clone()));
    (resolver, starts, prompts)
}

/// 轮询等待评审记录落盘（P1-2：触发层只登记并异步执行，终态经 store 落盘）。
pub(super) async fn wait_for_review_record(store: &CodingAttemptStore) -> IssueCrossRepoReview {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        if let Some(review) = store
            .load_issue_cross_repo_review(PROJECT_ID, ISSUE_ID)
            .expect("load review")
        {
            return review;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "跨仓评审记录必须最终落盘（15s 内）"
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

/// 挂起假 provider（P1-2）：`start` 计数后会话永不产出事件——事件 sender 被
/// 一个长眠 task 持有，模拟真实 provider 楔死。
struct HangingFakeProvider {
    starts: Arc<AtomicUsize>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for HangingFakeProvider {
    async fn start(
        &self,
        _input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        let (event_tx, event_rx) = mpsc::channel(4);
        let (command_tx, _command_rx) = mpsc::channel(4);
        tokio::spawn(async move {
            // 永不产出任何事件：持有 sender 长眠（远超 advisory 时限）。
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            drop(event_tx);
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }
}

/// 挂起 provider 的触发边界（P1-2：调用方秒级返回断言用）。
pub(super) fn hanging_review_provider() -> (
    IssueCrossRepoReviewProviderResolver<'static>,
    Arc<AtomicUsize>,
) {
    let provider = HangingFakeProvider {
        starts: Arc::new(AtomicUsize::new(0)),
    };
    let starts = provider.starts.clone();
    let adapter: Arc<dyn StreamingProviderAdapter> = Arc::new(provider);
    let resolver =
        IssueCrossRepoReviewProviderResolver::new(move |_: &ProviderName| Some(adapter.clone()));
    (resolver, starts)
}
