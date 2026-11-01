use super::*;

/// P2 Task 10 campaign fixture：`EnrolledGateFixture`（真实 enrollment
/// 绑定 + 物理 main checkout 就位，等 advance 用）+ 同 project 未授权
/// 手工 issue 对照。停在人工 plan 批准前：advance→首启→Fake 运行→
/// FinalConfirm 等待全链由编排器 `reconcile` 驱动（测试只当人手）。
pub(crate) struct P2CampaignFixture {
    pub(crate) gate: EnrolledGateFixture,
    manual_issue_id: String,
}

impl P2CampaignFixture {
    pub(crate) async fn new() -> Self {
        let gate = EnrolledGateFixture::new().await;
        init_real_main_checkout(&gate.inner.paths.root().join("checkout-enroll-a"));
        normalize_checkout_revision_to_unobserved(&gate.inner.paths);
        Self::with_gate(gate).await
    }

    /// C5 Task 8（A02）：单仓 campaign fixture——issue.repo_id 指向真实
    /// git 物理仓（main checkout 由 `new_single_repository` 就位），
    /// enrollment target 为 SingleRepository；其余与 LC campaign 同构。
    pub(crate) async fn new_single_repository() -> Self {
        let gate = EnrolledGateFixture::new_single_repository().await;
        Self::with_gate(gate).await
    }

    async fn with_gate(gate: EnrolledGateFixture) -> Self {
        // 未授权手工对照 issue（同 project；编排器对其零动作）。
        let manual = crate::product::issue_store::IssueStore::new(gate.inner.paths.clone())
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: PROJECT_ID.to_string(),
                repo_id: Some(
                    crate::web::handlers::automation_enrollment_test_support::REPOSITORY_ID
                        .to_string(),
                ),
                logical_codebase_id: None,
                title: "manual campaign issue".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .expect("manual campaign issue");
        Self {
            gate,
            manual_issue_id: manual.id,
        }
    }

    /// 人手干净批准绑定 plan（单次 P0 REST Approve；测试里显式可见）。
    pub(crate) async fn confirm_plan_by_human(&mut self) {
        self.gate.confirm_plan_by_human().await;
    }

    fn worker(&self) -> AutopilotOrchestrator {
        AutopilotOrchestrator::new(self.gate.state.clone(), Default::default())
    }

    /// 无页面 reconcile 循环：Confirmed→advance→Ready→单发首启→Fake
    /// runner 后台跑到 durable `WaitingForHuman ∧ FinalConfirm`。中途
    /// Failed/Aborted/AwaitingManualRecovery 即失败（真实业务断点）。
    pub(crate) async fn reconcile_until_coding_waiting_for_human(&self) {
        let worker = self.worker();
        tokio::time::timeout(std::time::Duration::from_secs(150), async {
                loop {
                    worker
                        .reconcile(&self.gate.state, PROJECT_ID, ISSUE_ID)
                        .await
                        .expect("campaign reconcile");
                    if let Some(attempt) = self.gate.coding_attempts().first() {
                        use crate::product::coding_models::CodingAttemptStatus;
                        match attempt.status {
                            CodingAttemptStatus::WaitingForHuman
                                if attempt.stage
                                    == crate::product::coding_models::CodingExecutionStage::FinalConfirm =>
                            {
                                break;
                            }
                            CodingAttemptStatus::Failed
                            | CodingAttemptStatus::Aborted
                            | CodingAttemptStatus::AwaitingManualRecovery => panic!(
                                "campaign stopped unexpectedly at {:?}/{:?} reason={:?}",
                                attempt.status, attempt.stage, attempt.manual_recovery_reason
                            ),
                            _ => {}
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
            })
            .await
            .expect("fake campaign reaches human FinalConfirm via reconcile only");
    }

    /// attempt 的 durable 首启 claim 数（不可复位单发事实；恰 1 = 只首启过一次）。
    pub(crate) fn runner_start_claims(&self, attempt_id: &str) -> usize {
        self.gate
            .coding_attempts()
            .iter()
            .filter(|attempt| attempt.id == attempt_id && attempt.start_claim.is_some())
            .count()
    }

    /// 未授权手工 issue 的 coding 首启 claim 总数（对照恒 0）。
    pub(crate) fn manual_issue_runner_start_claims(&self) -> usize {
        crate::product::coding_attempt_store::CodingAttemptStore::new(self.gate.inner.paths.clone())
            .list_attempts_for_issue(PROJECT_ID, &self.manual_issue_id)
            .expect("manual issue attempts")
            .iter()
            .filter(|attempt| attempt.start_claim.is_some())
            .count()
    }

    /// 未授权手工 issue 名下的自动前缀 plan 数（对照恒 0——补偿扫描
    /// 不为 manual/off issue 铺设计）。
    pub(crate) fn manual_issue_auto_plans(&self) -> usize {
        crate::product::lifecycle_store::LifecycleStore::new(self.gate.inner.paths.clone())
            .list_issue_work_item_plans(PROJECT_ID, &self.manual_issue_id)
            .expect("manual issue plans")
            .into_iter()
            .filter(|plan| plan.id.starts_with("issue_work_item_plan_auto_"))
            .count()
    }

    pub(crate) fn attempt(&self) -> crate::product::coding_models::CodingExecutionAttempt {
        self.gate.attempt()
    }
}

/// P2 Task 10 共享 fixture：campaign 起点＝人工批准后的 Confirmed
/// enrollment（真实 main checkout + 干净单次 approve，无 failpoint）。
pub(crate) async fn p2_enrolled_campaign_fixture() -> P2CampaignFixture {
    P2CampaignFixture::new().await
}

/// C5 Task 8（A02）共享 fixture：单仓 Confirmed enrollment 的无页面
/// campaign 起点（真实 main checkout + 干净单次 approve，无 failpoint）。
pub(crate) async fn c5_single_repository_campaign_fixture() -> P2CampaignFixture {
    P2CampaignFixture::new_single_repository().await
}

/// 在 physical checkout 上初始化真实 main 分支 git 仓库（空提交即可满足
/// `resolve_advance_base_branch` 的 main→master 默认链验证）。
pub(super) fn init_real_main_checkout(repo: &std::path::Path) {
    fn git(repo: &std::path::Path, args: &[&str]) {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(args)
            .output()
            .expect("spawn git");
        assert!(
            output.status.success(),
            "git {args:?} in {repo:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    std::fs::create_dir_all(repo).expect("create checkout dir");
    git(repo, &["init"]);
    git(repo, &["symbolic-ref", "HEAD", "refs/heads/main"]);
    git(repo, &["config", "user.email", "aria@example.com"]);
    git(repo, &["config", "user.name", "Aria Test"]);
    git(repo, &["commit", "--allow-empty", "-m", "advance base"]);
    // ReviewRequest 阶段对 aria/issues/{issue} 分支做 `git ls-remote origin`
    // 并 push：配一个本地 bare origin 满足远端存在性（不触网）。
    let origin = repo
        .parent()
        .map(|parent| parent.join("origin-enroll-a.git"))
        .expect("checkout parent");
    let _ = std::fs::remove_dir_all(&origin);
    git(
        repo,
        &["init", "--bare", origin.to_str().expect("origin path")],
    );
    git(
        repo,
        &[
            "remote",
            "add",
            "origin",
            origin.to_str().expect("origin path"),
        ],
    );
    git(repo, &["push", "origin", "main"]);
}

/// 对齐生产不变量（snapshot_validator 文档）：repository registration 与
/// identity 迁移从不持久化 `RepositoryCheckoutRecord.revision`（恒 None），
/// admission 只在 checkout 侧已观测（Some）时逐字比对。seed 写入的合成
/// `Some("abcdef")` 会与 `init_real_main_checkout` 真实 HEAD 冻结出的快照
/// revision 恒不一致 → admission 误判 `target_snapshot_identity_drifted`。
/// 真仓就位后把权威 checkout 记录 revision 归一为未观测（None）。
pub(super) fn normalize_checkout_revision_to_unobserved(
    paths: &crate::product::app_paths::ProductAppPaths,
) {
    let lc_store = crate::product::logical_codebase::LogicalCodebaseStore::new(paths.clone());
    for checkout in lc_store.list_checkouts(PROJECT_ID).expect("list checkouts") {
        let mut patched = checkout.clone();
        patched.revision = None;
        lc_store
            .save_checkout(PROJECT_ID, &patched)
            .expect("normalize checkout revision");
    }
}
