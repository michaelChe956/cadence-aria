//! T19 红测:review 窗口生命周期(技术失败旋转/材料准入/违规人工门)。
//!
//! 覆盖 brief Step 1:
//! - 材料计数发生在 baseline 持久后(provider start 时窗口 Running 且
//!   manifest 可加载);
//! - 技术失败仅在前窗无违规且恢复完整后以原「首次+最多两次自动重试」
//!   预算建新窗(旋转不加次数);
//! - 违规需人工门:重试不发生、新窗口不加次数、锁保持;
//! - Invalidated/Pending 报告即使内容 approve 也不推进。

use super::*;
use crate::product::coding_models::ReviewReportAcceptance;
use crate::product::reviewer_window_store::{
    ReviewerWindowState, ReviewerWindowStore as T19WindowStore,
};
use std::sync::Mutex;

const APPROVE_OUTPUT: &str = r#"{"verdict":"approve","summary":"t19 ok","findings":[]}"#;

fn t19_window_store(root: &Path) -> T19WindowStore {
    T19WindowStore::new(root.join(".aria").join("reviewer-windows"))
}

fn reviewer_runs(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> Vec<CodingRoleRun> {
    let mut runs: Vec<CodingRoleRun> = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role runs")
        .into_iter()
        .filter(|run| run.role == CodingProviderRole::CodeReviewer)
        .collect();
    runs.sort_by_key(|run| run.run_no);
    runs
}

/// provider start 时探测:是否存在 Running 窗口且其 baseline manifest
/// 可加载(材料计数发生在 baseline 持久后)。
struct BaselineProbeProvider {
    starts: AtomicUsize,
    flags: Mutex<Vec<bool>>,
    window_root: PathBuf,
    snapshot_root: PathBuf,
    output: String,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for BaselineProbeProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let start_no = self.starts.fetch_add(1, Ordering::SeqCst) + 1;
        let window_store = T19WindowStore::new(&self.window_root);
        let snapshot_store =
            crate::product::reviewer_snapshot::ReviewerSnapshotStore::new(&self.snapshot_root);
        let ok = std::fs::read_dir(self.window_root.join("windows"))
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok())
                    .filter_map(|entry| {
                        entry
                            .file_name()
                            .to_str()
                            .and_then(|name| name.strip_suffix(".json"))
                            .map(str::to_string)
                    })
                    .any(|window_id| {
                        window_store
                            .load_window(&window_id)
                            .map(|record| record.state == ReviewerWindowState::Running)
                            .unwrap_or(false)
                            && snapshot_store.load_manifest(&window_id).is_ok()
                    })
            })
            .unwrap_or(false);
        self.flags.lock().expect("flags").push(ok);
        let (event_tx, event_rx) = mpsc::channel(2);
        let (command_tx, _command_rx) = mpsc::channel(2);
        let output = self.output.clone();
        let contract = input.structured_output_contract.clone();
        tokio::spawn(async move {
            let event = ProviderEvent::Completed(
                crate::cross_cutting::streaming_provider::ProviderCompletion::from_output(
                    output,
                    contract.as_ref(),
                    Some(format!("probe-session-{start_no}")),
                ),
            );
            let _ = event_tx.send(event).await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[tokio::test]
async fn reviewer_material_collected_only_after_baseline_persisted() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let worktree = attempt.worktree_path.clone().expect("worktree");
    init_test_git_repo(&worktree);
    fs::write(worktree.join("reviewed.rs"), "pub fn reviewed() {}\n").expect("review diff");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = BaselineProbeProvider {
        starts: AtomicUsize::new(0),
        flags: Mutex::new(Vec::new()),
        window_root: root.path().join(".aria").join("reviewer-windows"),
        snapshot_root: root.path().join(".aria").join("reviewer-snapshots"),
        output: APPROVE_OUTPUT.to_string(),
    };
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let report = engine
        .execute_code_review_with_commands(&attempt, &provider, &mut command_rx)
        .await
        .expect("review completes");

    assert_eq!(provider.starts.load(Ordering::SeqCst), 1);
    let flags = provider.flags.lock().expect("flags").clone();
    assert_eq!(flags.len(), 1, "exactly one provider start");
    assert!(
        flags.iter().all(|ok| *ok),
        "每次材料采集(以 provider start 为界)前 baseline 必须已持久且窗口 Running"
    );
    assert_eq!(report.verdict, ReviewVerdict::Approve);
    assert_eq!(report.acceptance, Some(ReviewReportAcceptance::Accepted));
}

#[tokio::test]
async fn reviewer_transport_failure_rotates_clean_window_within_budget() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let worktree = attempt.worktree_path.clone().expect("worktree");
    init_test_git_repo(&worktree);
    fs::write(worktree.join("reviewed.rs"), "pub fn reviewed() {}\n").expect("review diff");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = TransportFailuresThenSuccessProvider::new(1, APPROVE_OUTPUT);
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let report = engine
        .execute_code_review_with_commands(&attempt, &provider, &mut command_rx)
        .await
        .expect("retry within budget succeeds");

    // 原预算:首次 + 1 次自动重试(窗口旋转不加次数)。
    assert_eq!(provider.starts.load(Ordering::SeqCst), 2);
    assert_eq!(report.verdict, ReviewVerdict::Approve);

    let runs = reviewer_runs(&store, &attempt);
    assert_eq!(runs.len(), 2, "initial + automatic retry run");
    let first_window_id =
        crate::product::reviewer_window_store::role_run_window_id(&attempt.id, &runs[0].id);
    let retry_window_id =
        crate::product::reviewer_window_store::role_run_window_id(&attempt.id, &runs[1].id);
    assert_eq!(
        report.review_window_id.as_deref(),
        Some(retry_window_id.as_str())
    );
    assert_eq!(report.acceptance, Some(ReviewReportAcceptance::Accepted));

    let window_store = t19_window_store(root.path());
    // 前窗:干净技术回收(ClosedInvalidated,无违规 journal)。
    let first = window_store
        .load_window(&first_window_id)
        .expect("first window");
    assert_eq!(first.state, ReviewerWindowState::ClosedInvalidated);
    assert!(
        window_store
            .load_violations(&first_window_id)
            .expect("first window journal")
            .is_empty()
    );
    // 终窗:ClosedAccepted。
    let retry = window_store
        .load_window(&retry_window_id)
        .expect("retry window");
    assert_eq!(retry.state, ReviewerWindowState::ClosedAccepted);
    // 锁全部释放:树上无租约。
    assert!(
        window_store.list_leases().expect("leases").is_empty(),
        "技术旋转后锁必须全部释放"
    );
}

#[tokio::test]
async fn reviewer_violation_blocks_retry_and_keeps_manual_gate_lock() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let worktree = attempt.worktree_path.clone().expect("worktree");
    init_test_git_repo(&worktree);
    fs::write(worktree.join("reviewed.rs"), "pub fn reviewed() {}\n").expect("review diff");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    // 首轮 provider:违规直写 Git 域后 transport 失败;若错误地发生重试,
    // 第二轮输出 approve(以 starts 计数证伪)。
    let provider = ViolationThenTransportFailProvider {
        starts: AtomicUsize::new(0),
        git_config: worktree.join(".git/config"),
        output_after_violation: APPROVE_OUTPUT.to_string(),
    };
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = engine
        .execute_code_review_with_commands(&attempt, &provider, &mut command_rx)
        .await
        .expect_err("violation must abort retry rotation");

    assert!(
        error.to_string().contains("reviewer_window"),
        "error should surface window lifecycle: {error}"
    );
    // 违规→人工门:不重试(新窗口不加次数)。
    assert_eq!(
        provider.starts.load(Ordering::SeqCst),
        1,
        "violation must not trigger automatic retry"
    );
    let runs = reviewer_runs(&store, &attempt);
    assert!(
        runs.len() <= 2,
        "violation must not exceed the original budget shape: {runs:?}"
    );
    assert!(
        runs.iter()
            .all(|run| run.status != CodingRoleRunStatus::Running),
        "no reviewer run may stay running after violation: {runs:?}"
    );

    let window_store = t19_window_store(root.path());
    let window_id =
        crate::product::reviewer_window_store::role_run_window_id(&attempt.id, &runs[0].id);
    let record = window_store
        .load_window(&window_id)
        .expect("violation window record");
    assert_eq!(record.state, ReviewerWindowState::ManualGate);
    assert!(
        !window_store
            .load_violations(&window_id)
            .expect("violation journal")
            .is_empty(),
        "violation facts must be journalled before manual gate"
    );
    // 锁保持到人工裁决(租约仍在)。
    assert_eq!(
        window_store
            .list_leases()
            .expect("leases")
            .iter()
            .filter(|lease| lease.window_id == window_id)
            .count(),
        1,
        "manual gate keeps the worktree lease"
    );
    // Git 观察域(config)按 T16/T18 契约是观察面不是恢复面:违规标记
    // 保留为取证事实(恢复成功也不洗白)。
    let config_after = fs::read_to_string(worktree.join(".git/config")).expect("config");
    assert!(
        config_after.contains("[t19violation]"),
        "git-domain violation evidence must survive: {config_after}"
    );
    // worktree 恢复面:reviewer 无文件改动,Coder 产出不丢。
    assert!(
        worktree.join("reviewed.rs").exists(),
        "coder diff preserved"
    );
    // 报告未落(未驱动 approve)。
    assert!(
        store
            .list_code_review_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("reports")
            .is_empty(),
        "violated round must not produce a consumable report"
    );
}

struct ViolationThenTransportFailProvider {
    starts: AtomicUsize,
    git_config: PathBuf,
    output_after_violation: String,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ViolationThenTransportFailProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let start_no = self.starts.fetch_add(1, Ordering::SeqCst) + 1;
        let (event_tx, event_rx) = mpsc::channel(2);
        let (command_tx, _command_rx) = mpsc::channel(2);
        if start_no == 1 {
            // 违规:reviewer 窗口冻结期间直写 Git 域。
            let mut config = fs::read_to_string(&self.git_config).expect("read git config");
            config.push_str("\n[t19violation]\n\tmarker = true\n");
            fs::write(&self.git_config, config).expect("direct git-domain write");
            tokio::spawn(async move {
                let _ = event_tx
                    .send(ProviderEvent::Failed {
                        message: "connection reset by peer".to_string(),
                    })
                    .await;
            });
        } else {
            let output = self.output_after_violation.clone();
            let contract = input.structured_output_contract.clone();
            tokio::spawn(async move {
                let event = ProviderEvent::Completed(
                    crate::cross_cutting::streaming_provider::ProviderCompletion::from_output(
                        output,
                        contract.as_ref(),
                        Some("violation-retry-session".to_string()),
                    ),
                );
                let _ = event_tx.send(event).await;
            });
        }
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }
}

#[tokio::test]
async fn invalidated_report_content_approve_never_drives_progression() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let worktree = attempt.worktree_path.clone().expect("worktree");
    init_test_git_repo(&worktree);
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    // provider:违规直写 Git 域,随后输出 approve 内容——报告即使内容
    // approve,采信也必须是 Invalidated 并落人工门。
    let provider = ViolatingApproveProvider {
        git_config: worktree.join(".git/config"),
        output: APPROVE_OUTPUT.to_string(),
    };
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let report = engine
        .execute_code_review_with_commands(&attempt, &provider, &mut command_rx)
        .await
        .expect("review completes with violation");

    assert_eq!(report.verdict, ReviewVerdict::Approve, "模型内容保持原样");
    assert_eq!(
        report.acceptance,
        Some(ReviewReportAcceptance::Invalidated),
        "违规整轮作废:内容 approve 也不采信"
    );
    // 采信门:落 code_review_report_not_accepted 门(复用既有门链)。
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("open gates");
    assert!(
        gates
            .iter()
            .any(|gate| gate.reason_code.as_deref() == Some("code_review_report_not_accepted")),
        "not-accepted gate must land: {gates:?}"
    );
    // Git 观察域(config)是观察面不是恢复面:违规标记保留为取证事实。
    let config_after = fs::read_to_string(worktree.join(".git/config")).expect("config");
    assert!(
        config_after.contains("[t19violation]"),
        "violation evidence must survive: {config_after}"
    );
    let window_store = t19_window_store(root.path());
    let record = window_store
        .load_window(report.review_window_id.as_deref().expect("window bound"))
        .expect("window record");
    assert_eq!(record.state, ReviewerWindowState::ManualGate);
    assert_eq!(
        record.report_acceptance,
        ReportAcceptanceProjection::Invalidated
    );
}

struct ViolatingApproveProvider {
    git_config: PathBuf,
    output: String,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ViolatingApproveProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let mut config = fs::read_to_string(&self.git_config).expect("read git config");
        config.push_str("\n[t19violation]\n\tmarker = approve-round\n");
        fs::write(&self.git_config, config).expect("direct git-domain write");
        let (event_tx, event_rx) = mpsc::channel(2);
        let (command_tx, _command_rx) = mpsc::channel(2);
        let output = self.output.clone();
        let contract = input.structured_output_contract.clone();
        tokio::spawn(async move {
            let event = ProviderEvent::Completed(
                crate::cross_cutting::streaming_provider::ProviderCompletion::from_output(
                    output,
                    contract.as_ref(),
                    Some("violating-approve-session".to_string()),
                ),
            );
            let _ = event_tx.send(event).await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }
}

use crate::product::reviewer_window_store::ReportAcceptance as ReportAcceptanceProjection;

/// brief 参数化补齐:注入历史 CodeReviewer 会话的 attempt 经
/// `execute_code_review_with_commands` 续跑——契约(仅 Coder 可 resume,
/// `should_resume_provider_conversation`)下 reviewer 恒 fresh spawn
/// (每轮评审窗口 id 进入 LC 全投影,跨轮 resume 必然漂移),窗口
/// 生命周期与 initial 同构。
#[tokio::test]
async fn reviewer_history_session_never_resumes_but_shares_window_lifecycle() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let worktree = attempt.worktree_path.clone().expect("worktree");
    init_test_git_repo(&worktree);
    fs::write(worktree.join("reviewed.rs"), "pub fn reviewed() {}\n").expect("review diff");
    // 注入历史 reviewer 会话 → 契约下不进 resume 通道,首轮即 fresh。
    let mut resumed = attempt.clone();
    resumed.provider_conversations = vec![crate::product::models::ProviderConversationRef {
        role: crate::product::models::ProviderConversationRole::CodeReviewer,
        provider: ProviderName::ClaudeCode,
        provider_session_id: "session-resume-1".to_string(),
        updated_at: "2026-07-01T00:00:00Z".to_string(),
        last_node_id: None,
    }];
    store
        .write_coding_attempt_for_test(&resumed)
        .expect("persist resumed attempt");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = ResumeCapturingProvider {
        inner: TransportFailuresThenSuccessProvider::new(0, APPROVE_OUTPUT),
    };
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let report = engine
        .execute_code_review_with_commands(&resumed, &provider, &mut command_rx)
        .await
        .expect("resume review completes");

    // 历史 reviewer 会话不进 resume 通道(仅 Coder 可 resume 契约)。
    let inputs = provider.inner.recorded_inputs();
    assert_eq!(inputs.len(), 1);
    assert_eq!(inputs[0].resume_provider_session_id, None);
    // 窗口生命周期与 initial 同构:报告绑定窗口且被采信。
    assert_eq!(report.verdict, ReviewVerdict::Approve);
    assert_eq!(report.acceptance, Some(ReviewReportAcceptance::Accepted));
    let window_store = t19_window_store(root.path());
    let record = window_store
        .load_window(report.review_window_id.as_deref().expect("window bound"))
        .expect("window record");
    assert_eq!(
        record.state,
        crate::product::reviewer_window_store::ReviewerWindowState::ClosedAccepted
    );
}

struct ResumeCapturingProvider {
    inner: TransportFailuresThenSuccessProvider,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ResumeCapturingProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.inner.start(input, cancel).await
    }
}

/// N2(修复轮 2):F2 回归——成功后落盘链失败必须收口窗口,树不锁死。
/// 注入:provider raw 输出的 stage 目录被普通文件占位 →
/// `save_provider_raw_output` 的 create_dir_all 失败(确定性 Io)。
#[tokio::test]
async fn code_review_post_run_persist_failure_releases_window_no_deadlock() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let worktree = attempt.worktree_path.clone().expect("worktree");
    init_test_git_repo(&worktree);
    fs::write(worktree.join("reviewed.rs"), "pub fn reviewed() {}\n").expect("review diff");
    let raw_root =
        store.provider_raw_output_root(&attempt.project_id, &attempt.issue_id, &attempt.id);
    fs::create_dir_all(&raw_root).expect("raw root");
    fs::write(raw_root.join("code_review"), "collision").expect("stage dir collision");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = TransportFailuresThenSuccessProvider::new(0, APPROVE_OUTPUT);
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = engine
        .execute_code_review_with_commands(&attempt, &provider, &mut command_rx)
        .await
        .expect_err("persist failure must surface");
    assert!(
        error.to_string().contains("code_review"),
        "unexpected error: {error}"
    );

    // F2:窗口收口——离开 Running,干净回收释放锁,树不永久锁死。
    let runs = reviewer_runs(&store, &attempt);
    let window_id =
        crate::product::reviewer_window_store::role_run_window_id(&attempt.id, &runs[0].id);
    let window_store = t19_window_store(root.path());
    let record = window_store
        .load_window(&window_id)
        .expect("window record after persist failure");
    assert_ne!(
        record.state,
        crate::product::reviewer_window_store::ReviewerWindowState::Running
    );
    assert_eq!(
        record.state,
        crate::product::reviewer_window_store::ReviewerWindowState::ClosedInvalidated,
        "clean recycle must release the tree"
    );
    assert!(
        window_store.list_leases().expect("leases").is_empty(),
        "no lease may survive the persist-failure close"
    );
    // 后续开窗可继续(死锁反证)。
    let reopened = engine
        .begin_reviewer_window_for_run(
            "t19-reopen-after-persist-failure",
            &attempt,
            &runs[0].id,
            &ProviderName::ClaudeCode,
            &worktree,
        )
        .await
        .expect("tree must be reopenable after close");
    engine
        .recycle_reviewer_window(&reopened, "t19 teardown")
        .await
        .expect("teardown recycle");
}

/// N2(修复轮 2):F3 回归——internal review 流后落盘失败同样收口窗口。
#[tokio::test]
async fn internal_pr_review_persist_failure_releases_window_no_deadlock() {
    let (root, store, attempt) = running_attempt_with_worktree();
    let worktree = attempt.worktree_path.clone().expect("worktree");
    init_test_git_repo(&worktree);
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            crate::product::coding_models::CodingExecutionStage::InternalPrReview,
        )
        .expect("stage to internal review");
    store
        .save_review_request(
            &attempt,
            &crate::product::coding_models::ReviewRequest {
                id: "review_request_0001".to_string(),
                attempt_id: attempt.id.clone(),
                kind: crate::product::coding_models::ReviewRequestKind::GitBranchOnly,
                remote_kind: crate::product::coding_models::RemoteKind::GenericGit,
                remote: "origin".to_string(),
                base_branch: attempt.base_branch.clone(),
                branch_name: attempt.branch_name.clone(),
                commit_sha: "unknown".to_string(),
                push_status: crate::product::coding_models::PushStatus::Pushed,
                external_url: None,
                manual_instructions: Vec::new(),
                created_at: "2026-07-01T00:00:00Z".to_string(),
                updated_at: "2026-07-01T00:00:00Z".to_string(),
                push_error: None,
                owner_kind: crate::product::coding_models::ReviewRequestOwnerKind::Attempt,
                pointer_publication_id: None,
                revoked: false,
            },
        )
        .expect("seed review request");
    let raw_root =
        store.provider_raw_output_root(&attempt.project_id, &attempt.issue_id, &attempt.id);
    fs::create_dir_all(&raw_root).expect("raw root");
    fs::write(raw_root.join("internal_pr_review"), "collision").expect("stage dir collision");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = TransportFailuresThenSuccessProvider::new(0, APPROVE_OUTPUT);
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let error = engine
        .execute_internal_pr_review_with_commands(&attempt, &provider, &mut command_rx)
        .await
        .expect_err("persist failure must surface");
    assert!(
        error.to_string().contains("internal_pr_review"),
        "unexpected error: {error}"
    );

    let run = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role runs")
        .into_iter()
        .find(|run| run.role == CodingProviderRole::InternalReviewer)
        .expect("internal reviewer run");
    let window_store = t19_window_store(root.path());
    let record = window_store
        .load_window(&crate::product::reviewer_window_store::role_run_window_id(
            &attempt.id,
            &run.id,
        ))
        .expect("window record after persist failure");
    assert_eq!(
        record.state,
        crate::product::reviewer_window_store::ReviewerWindowState::ClosedInvalidated,
        "F3 close must recycle the window cleanly"
    );
    assert!(
        window_store.list_leases().expect("leases").is_empty(),
        "no lease may survive the internal-review close"
    );
    let reopened = engine
        .begin_reviewer_window_for_run(
            "t19-reopen-after-internal-persist-failure",
            &attempt,
            &run.id,
            &ProviderName::ClaudeCode,
            &worktree,
        )
        .await
        .expect("tree must be reopenable after close");
    engine
        .recycle_reviewer_window(&reopened, "t19 teardown")
        .await
        .expect("teardown recycle");
}
