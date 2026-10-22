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
    IssueCrossRepoReviewStatus,
};
use crate::product::logical_codebase::{LogicalRepositoryId, RepositoryCheckoutId};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";

/// 计数假 provider：每次 `start` 计一次启动并记录 prompt，随后立即以固定
/// 输出完成会话（无窗口、无命令交互）。
struct CountingFakeProvider {
    output: String,
    starts: Arc<AtomicUsize>,
    prompts: Arc<Mutex<Vec<String>>>,
}

impl CountingFakeProvider {
    fn new(output: &str) -> Self {
        Self {
            output: output.to_string(),
            starts: Arc::new(AtomicUsize::new(0)),
            prompts: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn starts(&self) -> usize {
        self.starts.load(Ordering::SeqCst)
    }

    fn first_prompt(&self) -> String {
        self.prompts.lock().unwrap()[0].clone()
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
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
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
