// issue 级跨仓最终评审 engine 测试（REQ-GFR-CR-04「跨仓评审材料与 provider
// 选择」，设计 B §四 B2–B5）。
//
// - provider 取值顺序：快照 `.internal_reviewer` → 缺失回退同一快照的
//   `.code_reviewer` → 两者皆缺 `status=Skipped`（不启动 provider）。
// - 解析层 `default_source_stage = IssueCrossRepoReview`：provider 输出省略
//   `source_stage` 时每条 finding 落为 `issue_cross_repo_review`。
// - advisory：评审是建议性旁路，不开评审窗、不产出 acceptance。

use super::*;
use crate::cross_cutting::streaming_provider::{ProviderCompletion, ProviderSession};
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_models::{
    AttemptTargetSnapshot, CodingAttemptStatus, CodingRoleProviderConfigSnapshot,
    CodingStageGateStatus, IssueCrossRepoReview, IssueCrossRepoReviewStatus,
};
use crate::product::logical_codebase::{LogicalRepositoryId, RepositoryCheckoutId};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";
/// 夹具 issue 的多仓逻辑代码库 id（P2-3：issue 形态判据是
/// `logical_codebase_id`，不是 attempt 快照仓数）。
const CROSS_REPO_LC_ID: &str = "lc_cross_repo_fixture";

/// 计数假 provider：每次 `start` 计一次启动并记录 prompt，随后立即以固定
/// 输出完成会话（无窗口、无命令交互）。
struct CountingFakeProvider {
    output: String,
    starts: Arc<AtomicUsize>,
    prompts: Arc<Mutex<Vec<String>>>,
    inputs: Arc<Mutex<Vec<StreamingProviderInput>>>,
}

impl CountingFakeProvider {
    fn new(output: &str) -> Self {
        Self {
            output: output.to_string(),
            starts: Arc::new(AtomicUsize::new(0)),
            prompts: Arc::new(Mutex::new(Vec::new())),
            inputs: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn starts(&self) -> usize {
        self.starts.load(Ordering::SeqCst)
    }

    fn first_prompt(&self) -> String {
        self.prompts.lock().unwrap()[0].clone()
    }

    /// 第一个收到的完整 input（P1-1：交给 adapter 前的策略/时限断言）。
    fn first_input(&self) -> StreamingProviderInput {
        self.inputs.lock().unwrap()[0].clone()
    }

    /// 计数句柄（provider 被移入 `Arc<dyn StreamingProviderAdapter>` 后仍可观测）。
    fn counters(&self) -> (Arc<AtomicUsize>, Arc<Mutex<Vec<String>>>) {
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
fn cross_repo_fixture(
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
            logical_codebase_id: Some(CROSS_REPO_LC_ID.to_string()),
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
fn seed_role_provider_config(
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

#[tokio::test]
async fn cross_repo_review_falls_back_to_code_reviewer_provider() {
    let (_root, store, engine, attempts) = cross_repo_fixture(2);
    // 存量旧 JSON 形态：缺 `internal_reviewer`，`code_reviewer` 在场。
    seed_role_provider_config(&store, &attempts, None, Some(ProviderName::ClaudeCode));
    let provider = CountingFakeProvider::new(
        "{\"verdict\":\"approve\",\"summary\":\"跨仓一致性检查通过\",\"findings\":[]}",
    );

    let review = engine
        .maybe_run_issue_cross_repo_final_review(PROJECT_ID, ISSUE_ID, &provider)
        .await
        .expect("cross repo review executes")
        .expect("review record");

    assert_eq!(review.status, IssueCrossRepoReviewStatus::Completed);
    assert_ne!(review.status, IssueCrossRepoReviewStatus::Skipped);
    assert_eq!(review.verdict, ReviewVerdict::Approve);
    assert_eq!(
        provider.starts(),
        1,
        "code_reviewer fallback must start provider exactly once"
    );
    assert_eq!(review.repository_sections.len(), 2);
    assert!(
        provider
            .first_prompt()
            .contains("Coding Workspace IssueCrossRepoFinalReview"),
        "prompt must carry the cross repo review title"
    );
    let persisted = store
        .load_issue_cross_repo_review(PROJECT_ID, ISSUE_ID)
        .expect("load persisted review")
        .expect("review persisted");
    assert_eq!(persisted.status, IssueCrossRepoReviewStatus::Completed);
    assert_eq!(persisted.id, review.id);
}

#[tokio::test]
async fn cross_repo_review_skipped_when_both_providers_missing() {
    let (_root, store, engine, attempts) = cross_repo_fixture(2);
    seed_role_provider_config(&store, &attempts, None, None);
    let provider = CountingFakeProvider::new("must not run");

    let review = engine
        .maybe_run_issue_cross_repo_final_review(PROJECT_ID, ISSUE_ID, &provider)
        .await
        .expect("cross repo review call succeeds")
        .expect("review record");

    assert_eq!(review.status, IssueCrossRepoReviewStatus::Skipped);
    assert_eq!(
        provider.starts(),
        0,
        "no provider must start when both reviewers missing"
    );
    let persisted = store
        .load_issue_cross_repo_review(PROJECT_ID, ISSUE_ID)
        .expect("load persisted review")
        .expect("skipped review persisted");
    assert_eq!(persisted.status, IssueCrossRepoReviewStatus::Skipped);
}

#[tokio::test]
async fn cross_repo_review_marks_every_finding_source_stage() {
    let (_root, _store, engine, attempts) = cross_repo_fixture(2);
    seed_role_provider_config(&_store, &attempts, Some(ProviderName::ClaudeCode), None);
    let provider = CountingFakeProvider::new(
        "{\"verdict\":\"request_changes\",\"summary\":\"跨仓契约存在不一致\",\"findings\":[\
         {\"severity\":\"warning\",\"file_path\":\"services/api/gateway.rs\",\"line\":10,\
         \"message\":\"调用面与上游契约不一致\",\"required_action\":\"建议对齐契约（仅建议）\"},\
         {\"severity\":\"info\",\"file_path\":\"crates/core/lib.rs\",\"line\":1,\
         \"message\":\"上下游交付闭环缺少说明\"}]}",
    );

    let review = engine
        .maybe_run_issue_cross_repo_final_review(PROJECT_ID, ISSUE_ID, &provider)
        .await
        .expect("cross repo review executes")
        .expect("review record");

    assert_eq!(review.status, IssueCrossRepoReviewStatus::Completed);
    assert_eq!(review.verdict, ReviewVerdict::RequestChanges);
    assert_eq!(review.findings.len(), 2);
    for finding in &review.findings {
        assert_eq!(
            finding.source_stage,
            CodingExecutionStage::IssueCrossRepoReview,
            "findings without source_stage must default to issue_cross_repo_review"
        );
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
fn cross_repo_trigger_fixture(
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
fn cross_repo_trigger_fixture_with_states(
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
fn cross_repo_trigger_fixture_single_repo(
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
fn single_repo_trigger_fixture(
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
            logical_codebase_id: issue_lc_id.map(str::to_string),
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
fn fake_review_provider(
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
async fn wait_for_review_record(store: &CodingAttemptStore) -> IssueCrossRepoReview {
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
fn hanging_review_provider() -> (
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

#[tokio::test]
async fn cross_repo_review_triggers_exactly_once_after_all_work_items_completed() {
    let (_root, store, engine, attempts) = cross_repo_trigger_fixture(3);
    let (resolver, starts, prompts) = fake_review_provider(
        "{\"verdict\":\"approve\",\"summary\":\"跨仓一致性检查通过\",\"findings\":[]}",
    );

    // 收口钩子（WorkItem scope 形态）尾部触发 issue 级跨仓最终评审。
    engine
        .maybe_complete_issue_delivery(&attempts[2], &resolver)
        .await
        .expect("issue delivery completion");

    let review = wait_for_review_record(&store).await;
    assert_eq!(starts.load(Ordering::SeqCst), 1, "跨仓评审必须恰好执行一次");
    assert_eq!(review.status, IssueCrossRepoReviewStatus::Completed);
    assert_eq!(
        review.repository_sections.len(),
        3,
        "材料必须覆盖全部 3 个仓"
    );
    let logical_ids = review
        .repository_sections
        .iter()
        .map(|section| section.logical_repository_id.clone())
        .collect::<Vec<String>>();
    let mut ascending = logical_ids.clone();
    ascending.sort();
    assert_eq!(
        logical_ids, ascending,
        "跨仓材料分段必须按 logical_repository_id 升序"
    );
    assert_ne!(
        logical_ids[0], logical_ids[2],
        "分段顺序不得退化为 attempt 创建顺序"
    );
    let prompt = prompts.lock().unwrap()[0].clone();
    assert_eq!(
        prompt.matches("Repository #").count(),
        3,
        "材料必须含 3 段逐仓 diff"
    );
    for index in 0..3 {
        assert!(
            prompt.contains(&format!("repo_{index}.txt")),
            "材料缺少第 {index} 仓的 diff"
        );
    }
}

#[tokio::test]
async fn cross_repo_review_is_not_triggered_twice() {
    let (_root, store, engine, attempts) = cross_repo_trigger_fixture(3);
    let (resolver, starts, prompts) = fake_review_provider(
        "{\"verdict\":\"approve\",\"summary\":\"跨仓一致性检查通过\",\"findings\":[]}",
    );

    engine
        .maybe_complete_issue_delivery(&attempts[2], &resolver)
        .await
        .expect("first issue delivery completion");
    let first = wait_for_review_record(&store).await;

    // 重复进入收口钩子 → 不得重复评审、不得新增工件。
    engine
        .maybe_complete_issue_delivery(&attempts[2], &resolver)
        .await
        .expect("second issue delivery completion");
    let second = store
        .load_issue_cross_repo_review(PROJECT_ID, ISSUE_ID)
        .expect("load persisted review")
        .expect("cross repo review record");

    assert_eq!(
        starts.load(Ordering::SeqCst),
        1,
        "重复进入收口钩子不得重复评审"
    );
    assert_eq!(prompts.lock().unwrap().len(), 1);
    assert_eq!(first.id, second.id, "评审记录按 issue 唯一");
    assert_eq!(
        first.created_at, second.created_at,
        "评审记录不得被覆盖重写"
    );
    assert!(
        store
            .list_issue_cross_repo_acknowledgements(PROJECT_ID, ISSUE_ID)
            .expect("acknowledgements")
            .is_empty(),
        "重复触发不得新增留痕"
    );
    let review_root = store
        .paths()
        .issue_root(PROJECT_ID, ISSUE_ID)
        .join("coding-cross-repo-reviews");
    let review_files = fs::read_dir(&review_root)
        .expect("cross repo review root")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().and_then(|value| value.to_str()) == Some("json"))
        .count();
    assert_eq!(review_files, 1, "评审本体按 issue 唯一，不得新增第二份");
}

#[tokio::test]
async fn cross_repo_review_material_covers_attempt_awaiting_final_confirm() {
    // group 形态的收口钩子落在 attempt「已到人工最终确认、尚未 Completed」的时刻：
    // 该仓开发已定稿，必须进入跨仓材料——否则该仓缺段（甚至整份材料为空 → 评审
    // 永不执行，多仓形态形同虚设）。
    let (_root, store, engine, attempts) =
        cross_repo_trigger_fixture_with_states(2, &[false, true]);
    let (resolver, starts, prompts) = fake_review_provider(
        "{\"verdict\":\"approve\",\"summary\":\"跨仓一致性检查通过\",\"findings\":[]}",
    );

    engine
        .maybe_complete_issue_delivery(&attempts[1], &resolver)
        .await
        .expect("issue delivery completion");

    let review = wait_for_review_record(&store).await;
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    assert_eq!(review.status, IssueCrossRepoReviewStatus::Completed);
    assert_eq!(
        review.repository_sections.len(),
        2,
        "停在人工最终确认的仓也必须进入跨仓材料"
    );
    let prompt = prompts.lock().unwrap()[0].clone();
    assert!(prompt.contains("repo_0.txt"));
    assert!(prompt.contains("repo_1.txt"));
}

// ---------------------------------------------------------------------------
// Task 18：advisory 呈现 + 已知风险留痕（REQ-GFR-CR-05「advisory 红线」/
// REQ-GFR-CR-06「已知风险确认留痕」，设计 B §五 C / §六 D）。
// ---------------------------------------------------------------------------

/// request_changes 假 provider 输出：1 条 error + 1 条 warning finding。
const REQUEST_CHANGES_OUTPUT: &str = "{\"verdict\":\"request_changes\",\"summary\":\"跨仓契约不一致\",\"findings\":[{\"severity\":\"error\",\"message\":\"网关与前端对公告分页参数命名不一致\"},{\"severity\":\"warning\",\"message\":\"busi 删除接口缺少级联清理\"}]}";

/// 把已停在人工最终确认的 attempt 补齐 `head_commit`（完成门需要）。
fn with_head_commit(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> CodingExecutionAttempt {
    let worktree = attempt
        .worktree_path
        .clone()
        .expect("fixture attempt has worktree");
    let head = super::git_stdout(&worktree, &["rev-parse", "HEAD"])
        .trim()
        .to_string();
    store
        .update_attempt_head_commit(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            Some(head),
        )
        .expect("seed head commit")
}

#[tokio::test]
async fn cross_repo_request_changes_still_allows_final_confirm() {
    let (_root, store, engine, attempts) =
        cross_repo_trigger_fixture_with_states(2, &[false, true]);
    let (resolver, _starts, _prompts) = fake_review_provider(REQUEST_CHANGES_OUTPUT);

    // 评审记录在人工确认之前落盘（T17 接线），结论为要求修改。
    engine
        .maybe_complete_issue_delivery(&attempts[0], &resolver)
        .await
        .expect("issue delivery completion");
    let review = wait_for_review_record(&store).await;
    assert_eq!(review.verdict, ReviewVerdict::RequestChanges);

    // REQ-GFR-CR-05：要求修改仍可最终确认，attempt 进入完成态。
    let parked = with_head_commit(&store, &attempts[1]);
    let confirmed = engine
        .handle_final_confirm(PROJECT_ID, ISSUE_ID, &parked.id)
        .await
        .expect("request_changes 不得阻断最终确认");
    assert_eq!(confirmed.status, CodingAttemptStatus::Completed);

    // issue 不被阻断。
    let issue = IssueStore::new(store.paths())
        .get(PROJECT_ID, ISSUE_ID)
        .expect("issue record");
    assert_ne!(issue.status, IssueStatus::Blocked);

    // 待处理门保持为空（advisory 不建门：choice / blocked / stage 全空）。
    for attempt_id in [&attempts[0].id, &confirmed.id] {
        assert!(
            store
                .list_open_choice_gates(PROJECT_ID, ISSUE_ID, attempt_id)
                .expect("choice gates")
                .is_empty()
        );
        assert!(
            store
                .list_open_blocked_gate_facts(PROJECT_ID, ISSUE_ID, attempt_id)
                .expect("blocked gates")
                .is_empty()
        );
        assert!(
            store
                .list_open_stage_gates(PROJECT_ID, ISSUE_ID, attempt_id)
                .expect("stage gates")
                .iter()
                .all(|gate| gate.status != CodingStageGateStatus::Open)
        );
    }

    // 确认在案：REQ-GFR-CR-06 留痕恰好一条（字段级断言见下一用例）。
    let acks = store
        .list_issue_cross_repo_acknowledgements(PROJECT_ID, ISSUE_ID)
        .expect("acknowledgements");
    assert_eq!(acks.len(), 1, "要求修改下确认必须留下已知风险留痕");
    assert_eq!(acks[0].cross_repo_review_id, review.id);
    assert_eq!(acks[0].attempt_id, confirmed.id);
}

#[tokio::test]
async fn cross_repo_acknowledgement_written_once_after_final_confirm() {
    let (_root, store, engine, attempts) =
        cross_repo_trigger_fixture_with_states(2, &[false, true]);
    let (resolver, _starts, _prompts) = fake_review_provider(REQUEST_CHANGES_OUTPUT);
    engine
        .maybe_complete_issue_delivery(&attempts[0], &resolver)
        .await
        .expect("issue delivery completion");
    let review = wait_for_review_record(&store).await;
    assert_eq!(review.verdict, ReviewVerdict::RequestChanges);
    assert_eq!(review.findings.len(), 2);

    // 确认前无留痕。
    assert!(
        store
            .list_issue_cross_repo_acknowledgements(PROJECT_ID, ISSUE_ID)
            .expect("acknowledgements")
            .is_empty()
    );

    let parked = with_head_commit(&store, &attempts[1]);
    engine
        .handle_final_confirm(PROJECT_ID, ISSUE_ID, &parked.id)
        .await
        .expect("final confirm");

    // 确认后恰好一条留痕：时间 + 评审 id + 结论 + findings 摘要。
    let acks = store
        .list_issue_cross_repo_acknowledgements(PROJECT_ID, ISSUE_ID)
        .expect("acknowledgements");
    assert_eq!(acks.len(), 1, "确认后必须恰好留痕一条");
    let ack = &acks[0];
    assert_eq!(ack.cross_repo_review_id, review.id);
    assert_eq!(ack.attempt_id, parked.id);
    assert_eq!(ack.verdict, ReviewVerdict::RequestChanges);
    assert!(
        chrono::DateTime::parse_from_rfc3339(&ack.acknowledged_at).is_ok(),
        "acknowledged_at 必须是 RFC3339 时间"
    );
    assert!(ack.findings_summary.contains("error=1"));
    assert!(ack.findings_summary.contains("warning=1"));
    assert!(
        ack.findings_summary
            .contains("网关与前端对公告分页参数命名不一致")
    );
    assert!(ack.findings_summary.contains("busi 删除接口缺少级联清理"));

    // 原始 JSON 只含允许字段：不得出现确认人 / 通道（owner 裁定）。
    let ack_dir = store
        .paths()
        .issue_root(PROJECT_ID, ISSUE_ID)
        .join("coding-cross-repo-reviews")
        .join("acknowledgements");
    let mut ack_files: Vec<PathBuf> = fs::read_dir(&ack_dir)
        .expect("acknowledgements dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect();
    ack_files.sort();
    assert_eq!(ack_files.len(), 1);
    let raw: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&ack_files[0]).expect("read ack"))
            .expect("ack json");
    let allowed = [
        "id",
        "project_id",
        "issue_id",
        "plan_id",
        "attempt_id",
        "cross_repo_review_id",
        "verdict",
        "findings_summary",
        "acknowledged_at",
    ];
    for key in raw.as_object().expect("ack object").keys() {
        assert!(
            allowed.contains(&key.as_str()),
            "留痕不得含确认人/通道等字段：{key}"
        );
    }
    assert!(raw.get("acknowledged_by").is_none());
    assert!(raw.get("acknowledged_channel").is_none());

    // 幂等（REQ-GFR-CR-06）：同一 (cross_repo_review_id, attempt_id) 只写一条。
    let mut duplicate = ack.clone();
    duplicate.id = "cross_repo_review_ack_9999".to_string();
    let written = store
        .append_issue_cross_repo_acknowledgement(&duplicate)
        .expect("duplicate append");
    assert!(!written, "同键留痕必须幂等拒绝");
    // 重复确认被状态机拒绝，也不得新增留痕。
    assert!(
        engine
            .handle_final_confirm(PROJECT_ID, ISSUE_ID, &parked.id)
            .await
            .is_err()
    );
    assert_eq!(
        store
            .list_issue_cross_repo_acknowledgements(PROJECT_ID, ISSUE_ID)
            .expect("acknowledgements")
            .len(),
        1,
        "重复确认不得产生第二条留痕"
    );
}

// ---------------------------------------------------------------------------
// P1-1 / P1-2 / P2-3 修复回归：Reviewer 窗口合同、触发不阻塞、issue 形态判据。
// ---------------------------------------------------------------------------

/// P1-1：交给 adapter 前的 input 必须已携带非空 `tool_policy`，且能通过真实
/// adapter spawn 前的同一守卫（`validate_tool_policy_for_role`）——fake
/// provider 不跑该守卫，因此此处显式调用守卫断言。修复前
/// `tool_policy = None`，守卫对 `Reviewer + None` 直接 `PolicyRequired` 拒启
/// （生产恒 `Failed("provider 启动失败…")`，测试不可见）。
#[tokio::test]
async fn cross_repo_review_input_carries_window_unbound_reviewer_contract() {
    let (_root, store, engine, attempts) = cross_repo_fixture(2);
    seed_role_provider_config(&store, &attempts, Some(ProviderName::ClaudeCode), None);
    let provider = CountingFakeProvider::new(
        "{\"verdict\":\"approve\",\"summary\":\"跨仓一致性检查通过\",\"findings\":[]}",
    );

    let review = engine
        .maybe_run_issue_cross_repo_final_review(PROJECT_ID, ISSUE_ID, &provider)
        .await
        .expect("cross repo review executes")
        .expect("review record");

    let input = provider.first_input();
    assert_eq!(
        input.role,
        AdapterRole::Reviewer,
        "跨仓评审以 Reviewer wire 角色运行"
    );
    let policy = input
        .tool_policy
        .clone()
        .expect("input.tool_policy 必须在交给 adapter 前设置（P1-1）");
    crate::cross_cutting::streaming_provider::validate_tool_policy_for_role(
        &AdapterRole::Reviewer,
        Some(&policy),
    )
    .expect("真实 adapter 的 spawn 前守卫必须放行本策略（P1-1）");
    // 合同身份与评审窗口解绑：等于本 issue 级评审记录的 id（durable、按 issue
    // 唯一），不复用任何评审窗 id。
    assert_eq!(
        policy,
        ProviderToolPolicy::reviewer_verify(review.id.clone()),
        "ReviewerVerify 合同引用必须是本评审记录 id"
    );
    assert!(!review.id.trim().is_empty());
    // P1-2：advisory 评审自带短时限（≤120s），不与 3h 的整 run 总上限同界。
    assert!(
        input.timeout_secs <= 120,
        "advisory 评审时限必须 ≤120s，实际 {}s",
        input.timeout_secs
    );
}

/// P1-2：收口钩子触发跨仓评审必须与 provider 耗时无关地秒级返回——挂起
/// provider（楔死）下调用方 2s 内返回；评审在后台按 advisory 短时限收敛为
/// `Failed` 落盘。修复前触发同步 await provider 终态（总超时 3h），调用方
/// 被挂住直到超时。
#[tokio::test]
async fn cross_repo_review_trigger_returns_without_waiting_for_provider() {
    let (_root, store, engine, attempts) = cross_repo_trigger_fixture(2);
    let (resolver, starts) = hanging_review_provider();

    let began = std::time::Instant::now();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        engine.maybe_complete_issue_delivery(&attempts[1], &resolver),
    )
    .await
    .expect("触发必须与 provider 无关地秒级返回（REQ-GFR-CR-05 不阻断）")
    .expect("issue delivery completion");
    assert!(
        began.elapsed() < std::time::Duration::from_secs(2),
        "收口钩子不得等待 provider 终态，实际耗时 {:?}",
        began.elapsed()
    );

    // 登记后评审确实在后台执行（provider 被启动），而非静默不跑。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while starts.load(Ordering::SeqCst) == 0 {
        assert!(
            std::time::Instant::now() < deadline,
            "后台评审任务必须启动 provider"
        );
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }

    // 楔死 provider 按 advisory 短时限收敛为 `Failed` 落盘（测试态时限 1s）。
    let review = wait_for_review_record(&store).await;
    assert_eq!(review.status, IssueCrossRepoReviewStatus::Failed);
    assert!(
        review
            .failure_reason
            .as_deref()
            .unwrap_or_default()
            .contains("超时"),
        "挂起 provider 必须按 advisory 时限收敛为 Failed：{:?}",
        review.failure_reason
    );
}

/// P2-3：多仓 issue（`logical_codebase_id=Some`）的全部 work item 落在同一仓
/// 时仍必须评审——形态按 issue 判，不按 attempt 快照的去重仓数。修复前
/// `distinct_logical_repos < 2` 把这类 issue 全部静默跳过（无记录、无提醒）。
#[tokio::test]
async fn cross_repo_issue_with_all_work_items_in_one_repo_still_reviewed() {
    let (_root, store, engine, attempts) = cross_repo_trigger_fixture_single_repo(2);
    let (resolver, starts, _prompts) = fake_review_provider(
        "{\"verdict\":\"approve\",\"summary\":\"跨仓一致性检查通过\",\"findings\":[]}",
    );

    engine
        .maybe_complete_issue_delivery(&attempts[1], &resolver)
        .await
        .expect("issue delivery completion");

    let review = wait_for_review_record(&store).await;
    assert_eq!(
        starts.load(Ordering::SeqCst),
        1,
        "全部 work item 同仓的多仓 issue 仍必须评审（P2-3）"
    );
    assert_eq!(review.status, IssueCrossRepoReviewStatus::Completed);
    assert_eq!(
        review.repository_sections.len(),
        2,
        "每 attempt 一段（同仓也分段）"
    );
    let distinct_repos: std::collections::BTreeSet<&str> = review
        .repository_sections
        .iter()
        .map(|section| section.logical_repository_id.as_str())
        .collect();
    assert_eq!(distinct_repos.len(), 1, "两段同属一个逻辑仓");
}

/// P2-3 负例：单仓 issue（`logical_codebase_id=None`）即使 attempt 快照横跨
/// 多个逻辑仓也不评审——快照仓数不再是触发判据，REQ-GFR-CR-02「单仓不产生
/// 跨仓评审」的引擎侧兜底改挂在 issue 形态上。
#[tokio::test]
async fn single_repo_issue_never_cross_repo_reviewed_even_with_multi_repo_snapshots() {
    let (_root, store, engine, attempts) = single_repo_trigger_fixture(2);
    let (resolver, starts, _prompts) = fake_review_provider(
        "{\"verdict\":\"approve\",\"summary\":\"跨仓一致性检查通过\",\"findings\":[]}",
    );

    engine
        .maybe_complete_issue_delivery(&attempts[1], &resolver)
        .await
        .expect("issue delivery completion");

    // 修复前：attempt 快照跨 2 仓 → 评审执行、记录立即落盘。修复后：单仓
    // issue 不评审——轮询确认记录在宽限期内始终缺失。
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(1500);
    loop {
        assert!(
            store
                .load_issue_cross_repo_review(PROJECT_ID, ISSUE_ID)
                .expect("load review")
                .is_none(),
            "单仓 issue 不得产生跨仓评审记录（P2-3）"
        );
        if std::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(
        starts.load(Ordering::SeqCst),
        0,
        "单仓 issue 不得启动评审 provider"
    );
}
