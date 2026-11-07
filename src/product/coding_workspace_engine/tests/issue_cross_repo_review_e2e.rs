//! T19 coding 线端到端夹具验收（REQ-GFR-CR-03/04/05 的 e2e 面 + Review
//! Focus 4 负例）。
//!
//! - `three_repo_issue_runs_cross_repo_review_once`：3 个 work item / 3 个仓的
//!   attempt 经**真实收口入口** `complete_attempt_after_review_request` 逐个完成
//!   → issue 级跨仓最终评审恰好一次：材料 3 段逐仓 diff、按
//!   `logical_repository_id` 升序（刻意与 attempt 创建顺序相反）、评审不改任何
//!   attempt 的 stage（REQ-GFR-CR-03）。
//! - `cross_repo_request_changes_does_not_trigger_coder_fix`（RF4）：跨仓评审结论
//!   request_changes 时不得触发任何 `RunCoderFix` / 转人工分诊路径——无新
//!   attempt、无待处理门、attempt 状态与 stage 不被评审改动；advisory 提醒以
//!   `kind=cross_repo_review` 投影（REQ-GFR-CR-05）。
//!
//! 夹具仓根（真实 git worktree）全部落 `/tmp`（`tempfile::tempdir`）。

use super::*;
use crate::cross_cutting::streaming_provider::{
    ProviderCompletion, ProviderEvent, StreamingProviderAdapter, StreamingProviderInput,
};
use crate::product::coding_models::{
    AttemptTargetSnapshot, CodingAttemptStatus, CodingRoleProviderConfigSnapshot,
    CodingStageGateStatus, IssueCrossRepoReviewStatus,
};
use crate::product::logical_codebase::{LogicalRepositoryId, RepositoryCheckoutId};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";
const REPO_COUNT: usize = 3;

/// 计数 verdict provider：记录启动次数与 prompt，按固定输出完成会话。
struct CountingVerdictProvider {
    output: String,
    starts: Arc<AtomicUsize>,
    prompts: Arc<Mutex<Vec<String>>>,
}

impl CountingVerdictProvider {
    fn new(output: &str) -> Self {
        Self {
            output: output.to_string(),
            starts: Arc::new(AtomicUsize::new(0)),
            prompts: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn counters(&self) -> (Arc<AtomicUsize>, Arc<Mutex<Vec<String>>>) {
        (self.starts.clone(), self.prompts.clone())
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for CountingVerdictProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: tokio_util::sync::CancellationToken,
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

/// 逻辑仓 id 固定 UUID：`index` 越大 id 越小——刻意让「按
/// logical_repository_id 升序」与「按 attempt 创建顺序」相反，排序断言真实生效。
fn descending_logical_id(index: usize) -> Uuid {
    Uuid::parse_str(&format!(
        "00000000-0000-0000-0000-{:012}",
        REPO_COUNT - index
    ))
    .expect("logical repository id")
}

fn e2e_target_snapshot(index: usize) -> AttemptTargetSnapshot {
    AttemptTargetSnapshot {
        logical_repository_id: LogicalRepositoryId(descending_logical_id(index)),
        checkout_id: RepositoryCheckoutId(Uuid::new_v4()),
        physical_repository_id: format!("repository_{index:04}"),
        canonical_path: PathBuf::from(format!("/tmp/t19-e2e-repository-{index}")),
        git_dir_identity: "sha256:t19-e2e".to_string(),
        revision: Some("abcdef".to_string()),
        policy_digest: "policy_digest".to_string(),
        membership_revision: 1,
        captured_at: "2026-10-23T00:00:00Z".to_string(),
        capture_source: "test".to_string(),
    }
}

/// 三仓 issue 夹具：多仓 issue（`logical_codebase_id` 在场）+ 3 个 work item +
/// 3 个 Running attempt（真实 git worktree，base 后各有增量提交、已推 review
/// request），停在真实收口入口可推进的形态。
fn three_repo_delivery_fixture() -> (
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
            repo_id: None,
            logical_codebase_id: "logical_codebase_t19_e2e".to_string(),
            title: "三仓公告管理升级".to_string(),
            description: None,
            change_id: None,
            base_branch: None,
        })
        .expect("issue");
    let store = CodingAttemptStore::new(paths.clone());
    let mut attempts = Vec::new();
    for index in 0..REPO_COUNT {
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
        let head = super::git_stdout(&worktree, &["rev-parse", "HEAD"])
            .trim()
            .to_string();

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
                target_snapshot: Some(e2e_target_snapshot(index)),
                max_auto_rework: 1,
            })
            .expect("create attempt");
        store
            .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("seed running attempt");
        let staged = store
            .update_attempt_stage(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingExecutionStage::ReviewRequest,
            )
            .expect("review request stage");
        let staged = store
            .update_attempt_head_commit(
                &staged.project_id,
                &staged.issue_id,
                &staged.id,
                Some(head),
            )
            .expect("head commit");
        store
            .save_review_request(
                &staged,
                &ReviewRequest {
                    id: format!("review_request_{index:04}"),
                    attempt_id: staged.id.clone(),
                    kind: ReviewRequestKind::GitBranchOnly,
                    remote_kind: RemoteKind::GenericGit,
                    remote: "origin".to_string(),
                    base_branch: staged.base_branch.clone(),
                    branch_name: staged.branch_name.clone(),
                    commit_sha: staged.head_commit.clone().unwrap_or_default(),
                    push_status: PushStatus::Pushed,
                    external_url: None,
                    manual_instructions: Vec::new(),
                    created_at: "2026-10-23T00:00:00Z".to_string(),
                    updated_at: "2026-10-23T00:00:00Z".to_string(),
                    push_error: None,
                    owner_kind: ReviewRequestOwnerKind::Attempt,
                    pointer_publication_id: None,
                    revoked: false,
                },
            )
            .expect("review request");
        // 跨仓评审 provider 在场（internal_reviewer）。
        store
            .update_role_provider_config_snapshot(
                &staged.project_id,
                &staged.issue_id,
                &staged.id,
                CodingRoleProviderConfigSnapshot {
                    coder: ProviderName::Fake,
                    code_reviewer: None,
                    internal_reviewer: Some(ProviderName::ClaudeCode),
                    review_rounds: 0,
                    permission_modes: Default::default(),
                },
            )
            .expect("role provider config");
        attempts.push(staged);
    }
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    (root, store, engine, attempts)
}

fn counting_resolver(
    output: &str,
) -> (
    IssueCrossRepoReviewProviderResolver<'static>,
    Arc<AtomicUsize>,
    Arc<Mutex<Vec<String>>>,
) {
    let provider = CountingVerdictProvider::new(output);
    let (starts, prompts) = provider.counters();
    let adapter: Arc<dyn StreamingProviderAdapter> = Arc::new(provider);
    let resolver =
        IssueCrossRepoReviewProviderResolver::new(move |_: &ProviderName| Some(adapter.clone()));
    (resolver, starts, prompts)
}

/// P1-2：收口尾部的跨仓评审触发为「登记 + tokio::spawn 异步执行」——轮询
/// 等记录落盘（deadline 15s / 25ms 间隔）再做断言。
async fn await_cross_repo_review(
    store: &CodingAttemptStore,
) -> crate::product::coding_models::IssueCrossRepoReview {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        if let Some(review) = store
            .load_issue_cross_repo_review(PROJECT_ID, ISSUE_ID)
            .expect("load review")
        {
            return review;
        }
        if std::time::Instant::now() > deadline {
            panic!("跨仓评审记录 15s 内未落盘（异步触发未完成）");
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
}

/// REQ-GFR-CR-03/04（F2-B1）：3 个 attempt 经真实收口入口逐个完成 → 恰好一次
/// issue 级跨仓评审；材料 3 段逐仓 diff 按 `logical_repository_id` 升序；评审
/// 不改任何 attempt 的 stage。
#[tokio::test]
async fn three_repo_issue_runs_cross_repo_review_once() {
    let (_root, store, engine, attempts) = three_repo_delivery_fixture();
    let (resolver, starts, prompts) = counting_resolver(
        "{\"verdict\":\"approve\",\"summary\":\"三仓一致性检查通过\",\"findings\":[]}",
    );

    // 3 个仓依次经生产收口入口（runner.rs 同款）完成。
    for attempt in &attempts {
        let completed = engine
            .complete_attempt_after_review_request(attempt, &resolver)
            .await
            .expect("attempt completes via real entry");
        assert_eq!(completed.status, CodingAttemptStatus::Completed);
    }

    // P1-2：收口尾部触发为「登记 + tokio::spawn 异步执行」——轮询等记录落盘
    // （记录在 provider 跑完后才落盘，落盘即可断言启动计数）。
    let review = await_cross_repo_review(&store).await;
    assert_eq!(
        starts.load(Ordering::SeqCst),
        1,
        "跨仓评审必须恰好执行一次（前两个收口不触发，第三个触发）"
    );
    assert_eq!(review.status, IssueCrossRepoReviewStatus::Completed);
    assert_eq!(
        review.repository_sections.len(),
        REPO_COUNT,
        "材料必须覆盖全部 3 个仓"
    );

    // 分段按 logical_repository_id 升序，且与 attempt 创建顺序相反（排序真实生效）。
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

    // 材料 3 段逐仓 diff（每仓各含自己的增量文件）。
    let prompt = prompts.lock().unwrap()[0].clone();
    assert_eq!(
        prompt.matches("Repository #").count(),
        REPO_COUNT,
        "材料必须含 3 段逐仓 diff"
    );
    for index in 0..REPO_COUNT {
        assert!(
            prompt.contains(&format!("repo_{index}.txt")),
            "材料缺少第 {index} 仓的 diff"
        );
    }

    // REQ-GFR-CR-03：评审不得改任何 attempt 的 stage。
    for attempt in &attempts {
        let persisted = store
            .get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id)
            .expect("persisted attempt");
        assert_eq!(
            persisted.stage,
            CodingExecutionStage::ReviewRequest,
            "跨仓评审不得改 attempt 的 stage"
        );
        assert_eq!(persisted.status, CodingAttemptStatus::Completed);
    }

    // 评审本体按 issue 唯一（review.json 恰好一份）。
    let review_root = store
        .paths()
        .issue_root(PROJECT_ID, ISSUE_ID)
        .join("coding-cross-repo-reviews");
    let review_files = fs::read_dir(&review_root)
        .expect("cross repo review root")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().and_then(|v| v.to_str()) == Some("json"))
        .count();
    assert_eq!(review_files, 1, "review.json 必须按 issue 唯一");

    // 全部完成 → issue 收口为 Completed（AllPushed 落盘）。
    let issue = IssueStore::new(store.paths())
        .get(PROJECT_ID, ISSUE_ID)
        .expect("issue");
    assert_eq!(issue.status, IssueStatus::Completed);
}

/// Review Focus 4（REQ-GFR-CR-05 红线）：跨仓评审 `request_changes` 不得流进
/// `CodeReviewFlowDecision` / `RunCoderFix`——无自动返工、无待处理门、无新增
/// attempt；advisory 提醒以 `kind=cross_repo_review` 投影。
#[tokio::test]
async fn cross_repo_request_changes_does_not_trigger_coder_fix() {
    let (_root, store, engine, attempts) = three_repo_delivery_fixture();
    let (resolver, _starts, _prompts) = counting_resolver(
        "{\"verdict\":\"request_changes\",\"summary\":\"三仓契约不一致\",\"findings\":[\
         {\"severity\":\"error\",\"message\":\"网关与前端对公告分页参数命名不一致\",\
         \"required_action\":\"建议对齐契约（仅建议）\"}]}",
    );

    for attempt in &attempts {
        engine
            .complete_attempt_after_review_request(attempt, &resolver)
            .await
            .expect("attempt still completes (advisory)");
    }

    let review = await_cross_repo_review(&store).await;
    assert_eq!(review.status, IssueCrossRepoReviewStatus::Completed);
    assert_eq!(review.verdict, ReviewVerdict::RequestChanges);
    assert_eq!(review.findings.len(), 1);

    // 不触发返工：attempt 总数不变（无 RunCoderFix 新 attempt）。
    let persisted_attempts = store
        .list_attempts_for_issue(PROJECT_ID, ISSUE_ID)
        .expect("attempts");
    assert_eq!(
        persisted_attempts.len(),
        REPO_COUNT,
        "request_changes 不得触发 coder fix / 返工 attempt"
    );

    // 不建任何门：choice / blocked / stage 全空（不分流返修或人工分诊）。
    for attempt in &persisted_attempts {
        assert_eq!(attempt.status, CodingAttemptStatus::Completed);
        assert_eq!(attempt.stage, CodingExecutionStage::ReviewRequest);
        assert!(
            store
                .list_open_choice_gates(PROJECT_ID, ISSUE_ID, &attempt.id)
                .expect("choice gates")
                .is_empty(),
            "advisory 不得建 choice 门"
        );
        assert!(
            store
                .list_open_blocked_gate_facts(PROJECT_ID, ISSUE_ID, &attempt.id)
                .expect("blocked gates")
                .is_empty(),
            "advisory 不得建 blocked 门"
        );
        assert!(
            store
                .list_open_stage_gates(PROJECT_ID, ISSUE_ID, &attempt.id)
                .expect("stage gates")
                .iter()
                .all(|gate| gate.status != CodingStageGateStatus::Open),
            "advisory 不得建 stage 门"
        );
    }

    // issue 不被阻断；advisory 提醒以 kind=cross_repo_review 投影。
    let issue = IssueStore::new(store.paths())
        .get(PROJECT_ID, ISSUE_ID)
        .expect("issue");
    assert_ne!(issue.status, IssueStatus::Blocked);
    let attention = crate::web::coding_attention_info::issue_coding_attention_items(
        &store.paths(),
        PROJECT_ID,
        ISSUE_ID,
    )
    .expect("attention items");
    assert!(
        attention.iter().any(|item| item.kind == "cross_repo_review"
            && item.reason_code.as_deref() == Some("cross_repo_review_request_changes")),
        "request_changes 必须投影 cross_repo_review advisory 提醒: {attention:?}"
    );
}
