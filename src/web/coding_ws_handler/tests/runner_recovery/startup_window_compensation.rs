//! 批 2 Task 21:取消/崩溃启动补偿与人工恢复续接矩阵。
//!
//! Step 1 红测锚点:`startup_scan_compensates_open_window_before_resuming_runner`
//! ——启动扫描必须先把未关闭窗口补偿到终态(停旧登记资源→按原 manifest 续
//! 恢复→人工门/释放),再放行 claimed-run 分诊;不得沿「仅日志后继续在同树
//! 启动 runner」的旧序。注入式崩溃前缀模拟(丢句柄重建 store);真进程重启
use std::path::{Path, PathBuf};

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
use crate::product::coding_models::{
    CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage,
    CodingProviderPermissionMode, CodingRolePermissionModes, CodingRoleProviderConfigSnapshot,
};
use crate::product::coding_workspace_engine::CodingWorkspaceEngine;
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::product::models::ProviderName;
use crate::product::reviewer_window_store::{ReviewerWindowState, ReviewerWindowStore};
use crate::web::runtime::WebRuntime;
use crate::web::state::{CodingAttemptRunKey, WebAppState};
use crate::web::workspace_ws_types::ProviderConfigSnapshot;

fn t21_dir() -> tempfile::TempDir {
    let root = Path::new("/tmp/aria-bug45/t21");
    std::fs::create_dir_all(root).expect("create /tmp/aria-bug45/t21");
    tempfile::tempdir_in(root).expect("t21 fixture dir")
}

fn git_ok(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {:?} in {} failed: {}",
        args,
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn git_stdout(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .expect("spawn git");
    assert!(out.status.success(), "git {args:?} failed");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// 启动补偿矩阵 fixture:git worktree + 权威 project/issue/attempt 记录
/// (Fake providers,Running + Coding stage)+ Coder 原始 dirty(untracked,
/// 开窗前已存在,必须跨恢复保留)。
pub(super) struct WindowStartupFixture {
    pub(super) root: PathBuf,
    pub(super) _keep: Option<tempfile::TempDir>,
    pub(super) store: CodingAttemptStore,
    pub(super) attempt: CodingExecutionAttempt,
    pub(super) worktree: PathBuf,
    pub(super) window_id: String,
}

impl WindowStartupFixture {
    /// label 用于同测多树(局部坏窗用例)区分 attempt/worktree。
    pub(super) fn new(label: &str) -> Self {
        let root = t21_dir();
        let mut fixture = Self::new_in(root.path(), label);
        fixture._keep = Some(root);
        fixture
    }

    /// 共享根构造(局部坏窗用例:多树同 `.aria`,project/issue 只种一次)。
    pub(super) fn new_in(root: &Path, label: &str) -> Self {
        let worktree = root.join(label).join("worktree");
        std::fs::create_dir_all(&worktree).expect("worktree dir");
        std::fs::write(worktree.join("tracked.txt"), "base\n").expect("tracked base");
        git_ok(&worktree, &["init", "-q", "-b", "main"]);
        git_ok(&worktree, &["config", "user.email", "t21@aria.invalid"]);
        git_ok(&worktree, &["config", "user.name", "t21-fixture"]);
        git_ok(&worktree, &["add", "-A"]);
        git_ok(&worktree, &["commit", "-qm", "base"]);
        // Coder 原始 dirty:开窗前存在 → 必须进 baseline 并跨恢复保留。
        std::fs::write(worktree.join("coder-dirty.txt"), "coder wip\n").expect("coder dirty");

        let paths = ProductAppPaths::new(root.join(".aria"));
        // 同根多树(局部坏窗用例)共用 project;issue 每树一个(单 issue
        // 仅允许一个 active attempt,双树须分 issue)。
        let projects = crate::product::project_store::ProjectStore::new(paths.clone())
            .list()
            .expect("list projects");
        let project_id = if let Some(existing) = projects.first() {
            existing.id.clone()
        } else {
            crate::product::project_store::ProjectStore::new(paths.clone())
                .create(crate::product::project_store::CreateProjectInput {
                    name: "t21 startup compensation project".to_string(),
                    description: None,
                })
                .expect("seed project")
                .id
        };
        let issue_id = crate::product::issue_store::IssueStore::new(paths.clone())
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: project_id.clone(),
                repo_id: None,
                logical_codebase_id: None,
                title: format!("t21 issue {label}"),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .expect("seed issue")
            .id;
        let store = CodingAttemptStore::new(paths.clone());
        let attempt = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: project_id.clone(),
                issue_id: issue_id.clone(),
                work_item_id: "work_item_0001".to_string(),
                base_branch: "main".to_string(),
                branch_name: format!("aria/work-items/work_item_0001/{label}-1"),
                worktree_path: Some(worktree.clone()),
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::Fake,
                    reviewer: Some(ProviderName::Fake),
                    review_rounds: 1,
                    permission_modes: Default::default(),
                },
                target_snapshot: None,
                max_auto_rework: 2,
            })
            .expect("create attempt");
        store
            .update_role_provider_config_snapshot(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingRoleProviderConfigSnapshot {
                    coder: ProviderName::Fake,
                    code_reviewer: Some(ProviderName::Fake),
                    internal_reviewer: Some(ProviderName::Fake),
                    review_rounds: 1,
                    permission_modes: CodingRolePermissionModes {
                        coder: CodingProviderPermissionMode::Auto,
                        code_reviewer: CodingProviderPermissionMode::Auto,
                        internal_reviewer: CodingProviderPermissionMode::Auto,
                    },
                },
            )
            .expect("role config");
        let attempt = store
            .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("seed running");
        let attempt = store
            .update_attempt_stage(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingExecutionStage::Coding,
            )
            .expect("coding stage");
        Self {
            root: root.to_path_buf(),
            _keep: None,
            store,
            attempt,
            worktree,
            window_id: format!("t21-window-{label}"),
        }
    }

    pub(super) fn paths(&self) -> ProductAppPaths {
        ProductAppPaths::new(self.root.join(".aria"))
    }

    pub(super) fn window_store(&self) -> ReviewerWindowStore {
        ReviewerWindowStore::new(self.root.join(".aria").join("reviewer-windows"))
    }

    /// 「进程重启」替身:全新 engine(无任何内存态),同 `.aria` 根。
    pub(super) fn engine(&self) -> CodingWorkspaceEngine {
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(64);
        CodingWorkspaceEngine::new(
            CodingAttemptStore::new(self.paths()),
            GitWorkspaceService::new(),
            event_tx,
        )
    }

    /// 真实开窗(engine 服务入口;空登记主 scope 即合法静止证明)。
    pub(super) async fn open_window(&self, window_id: &str, run_id: &str) {
        self.engine()
            .begin_reviewer_window_for_run(
                window_id,
                &self.attempt,
                run_id,
                &ProviderName::Fake,
                &self.worktree,
            )
            .await
            .expect("open reviewer window");
    }

    pub(super) fn restart_state(&self) -> WebAppState {
        let root = self.root.clone();
        WebAppState::new(root.clone(), WebRuntime::new_fake(root))
    }

    pub(super) fn attempt_key(&self) -> CodingAttemptRunKey {
        CodingAttemptRunKey::from_attempt(&self.attempt)
    }

    pub(super) fn reload_attempt(&self) -> CodingExecutionAttempt {
        self.store
            .get_attempt(
                &self.attempt.project_id,
                &self.attempt.issue_id,
                &self.attempt.id,
            )
            .expect("reload attempt")
    }

    pub(super) fn window_record(
        &self,
        window_id: &str,
    ) -> crate::product::reviewer_window_store::ReviewerWindowRecord {
        self.window_store()
            .load_window(window_id)
            .expect("window record")
    }

    pub(super) fn role_runs_empty(&self) -> bool {
        self.store
            .list_role_runs(
                &self.attempt.project_id,
                &self.attempt.issue_id,
                &self.attempt.id,
            )
            .expect("role runs")
            .is_empty()
    }

    pub(super) fn head(&self) -> String {
        git_stdout(&self.worktree, &["rev-parse", "HEAD"])
    }

    /// 平台写通道探测:冻结期间拒绝;释放后可进可出。
    pub(super) fn channel_allows_platform_write(&self) -> bool {
        let engine = self.engine();
        let attempt_id = self.attempt.id.clone();
        let worktree = self.worktree.clone();
        match engine.guard_platform_write_channel(
            &worktree,
            crate::product::reviewer_window_store::WriteChannelKind::Stage,
            &attempt_id,
        ) {
            Ok(()) => {
                engine.release_platform_write_channel(
                    &worktree,
                    crate::product::reviewer_window_store::WriteChannelKind::Stage,
                    &attempt_id,
                );
                true
            }
            Err(_) => false,
        }
    }
}

/// 模拟「reviewer 在窗口内写了工作树文件」的非违规残留(纯文件面,
/// 不动 Git 域):恢复后必须清除(manifest 外多余项)。
pub(super) fn pollute_worktree_scratch(fx: &WindowStartupFixture) {
    std::fs::write(
        fx.worktree.join("reviewer-scratch.txt"),
        "reviewer residue\n",
    )
    .expect("reviewer scratch");
}

/// 模拟「reviewer 在窗口内动了 Git 域」的违规形态(自提交):恢复回写
/// 基线并落违规 journal → 人工门。
pub(super) fn pollute_reviewer_commit(fx: &WindowStartupFixture) {
    std::fs::write(fx.worktree.join("reviewer-change.txt"), "reviewer edit\n")
        .expect("reviewer change");
    git_ok(&fx.worktree, &["add", "-A"]);
    git_ok(&fx.worktree, &["commit", "-qm", "reviewer self commit"]);
}

/// 主线红测(brief Step 2):启动扫描先补偿未关闭窗口(停旧登记资源→
/// 按原 manifest 续恢复→人工门/释放),再放行 claimed-run 分诊。旧序
/// (仅日志后直接 resume 同树 runner,窗口保持 Running)必须在此断言上 FAIL。
#[tokio::test]
async fn startup_scan_compensates_open_window_before_resuming_runner() {
    let fx = WindowStartupFixture::new("order");
    fx.open_window(&fx.window_id, "run-t21-order").await;
    let baseline_head = fx.head();
    pollute_worktree_scratch(&fx);

    // 「进程崩溃重启」:窗口/句柄全丢,仅 durable 记录在;重启后无 attach
    // 启动扫描先行。
    let state = fx.restart_state();
    let resumed = crate::web::autopilot_orchestrator::reconcile_claimed_coding_runs_once(&state)
        .await
        .expect("startup reconcile ok");

    let record = fx.window_record(&fx.window_id);
    assert!(
        record.state.is_terminal(),
        "启动分诊前必须先补偿窗口到终态,当前状态 {}(旧序:runner 先于窗恢复)",
        record.state.as_str()
    );
    // 补偿证据:reviewer 残留清除、Coder 原 dirty 保留、HEAD 回基线。
    assert!(
        !fx.worktree.join("reviewer-scratch.txt").exists(),
        "窗口补偿必须按原 manifest 恢复(reviewer 残留清除)"
    );
    assert!(
        fx.worktree.join("coder-dirty.txt").exists(),
        "Coder 原始 dirty 必须跨恢复保留"
    );
    assert_eq!(fx.head(), baseline_head, "恢复后 HEAD 回基线");
    // 补偿期间零 provider 会话;树释放后分诊才放行 resume。
    assert!(
        fx.role_runs_empty(),
        "窗口未收口前不得有 provider 会话/材料采集"
    );
    assert_eq!(resumed, 1, "树释放后启动分诊恢复 runner");
    assert_eq!(
        state.coding_runs.runner_count(&fx.attempt_key()),
        1,
        "补偿完成后同树 runner 才放行"
    );
    assert!(
        fx.channel_allows_platform_write(),
        "终态窗口必须解冻通道/释放租约"
    );
    assert_eq!(fx.reload_attempt().status, CodingAttemptStatus::Running);
}

use std::os::unix::process::ExitStatusExt;

// ---------------------------------------------------------------------------
// 崩溃前缀矩阵(注入式;真进程重启演练归 T24)
// ---------------------------------------------------------------------------

use crate::cross_cutting::process_manager::process_start_time_ticks;
use crate::cross_cutting::provider_resources::{
    OwnedResourceKind, OwnedResourceSpec, ProviderRunResourceScope, ResourceScopeKey,
    StartedResourceIdentity, new_boot_identity,
};
use crate::product::coding_models::{CodingProviderRole, CodingRoleRunTrigger};
use crate::product::reviewer_snapshot::{ReviewWorktreeLockIdentity, ReviewerSnapshotStore};
use crate::product::reviewer_window::ManualGateDecision;
use crate::product::reviewer_window_store::{
    REVIEWER_WINDOW_SCHEMA, ReportAcceptance, ReviewerWindowRecord as WindowRecordForPrefix,
    WindowLeaseRecord, tree_key_of,
};

fn snapshot_store(fx: &WindowStartupFixture) -> ReviewerSnapshotStore {
    ReviewerSnapshotStore::new(fx.paths().root().join("reviewer-snapshots"))
}

fn lease_held(fx: &WindowStartupFixture) -> bool {
    fx.window_store()
        .list_leases()
        .expect("list leases")
        .iter()
        .any(|lease| lease.window_id == fx.window_id)
}

async fn run_startup_compensation(
    fx: &WindowStartupFixture,
) -> Vec<crate::product::coding_workspace_engine::StartupWindowCompensation> {
    fx.engine()
        .compensate_open_reviewer_windows_at_startup()
        .await
}

/// 崩溃前缀「租约已持久/manifest 未全」:手工构造 Preparing 记录 + 租约
/// (owner=已死进程,启动接管视角)。
fn prepare_leased_window(fx: &WindowStartupFixture, run_id: &str) {
    let store = fx.window_store();
    let canonical = fx.worktree.canonicalize().expect("canonical worktree");
    let (_, gitdir, common_dir) =
        crate::product::reviewer_window::resolve_worktree_git_identity(&canonical)
            .expect("resolve identity");
    let identity = ReviewWorktreeLockIdentity {
        canonical_worktree: canonical.clone(),
        gitdir,
        common_dir,
        lease_id: format!("wlease-{}", fx.window_id),
    };
    let record = WindowRecordForPrefix::new(
        &fx.window_id,
        identity.clone(),
        &fx.attempt.id,
        run_id,
        "fake",
        Vec::new(),
    );
    store
        .create_or_resume_window(&record)
        .expect("create preparing record");
    let lease = WindowLeaseRecord {
        schema: REVIEWER_WINDOW_SCHEMA.to_string(),
        lease_id: identity.lease_id.clone(),
        tree_key: tree_key_of(&canonical),
        canonical_worktree: identity.canonical_worktree.display().to_string(),
        gitdir: identity.gitdir.display().to_string(),
        common_dir: identity.common_dir.display().to_string(),
        window_id: fx.window_id.clone(),
        attempt_id: fx.attempt.id.clone(),
        run_id: run_id.to_string(),
        provider: "fake".to_string(),
        acquired_at: chrono::Utc::now().to_rfc3339(),
        owner_pid: 4_242_424,
        owner_start_ticks: 999,
    };
    store.acquire_lease(&lease).expect("acquire lease");
}

/// 在生产资源根登记带 `aria.worktree` 归属标签的 writer scope。
/// `child=None`:未确认预留(未知 writer 崩溃窗口);`Some`:真实存活
/// 进程(组长)已确认。
async fn register_tree_writer_scope(
    fx: &WindowStartupFixture,
    label: &str,
    child: Option<&std::process::Child>,
) {
    let store_root = fx.paths().root().join("provider-resources");
    let private = fx.root.join("scope-private").join(label);
    std::fs::create_dir_all(&private).expect("private root");
    let key = ResourceScopeKey {
        scope_id: format!("t21-writer-{label}"),
        run_id: format!("run-{label}"),
        boot_identity: new_boot_identity(),
    };
    let scope = ProviderRunResourceScope::open_for_recovery(
        &key,
        Some(&fx.window_id),
        &private,
        &store_root,
        None,
    )
    .expect("open scope")
    .with_worktree_key(&tree_key_of(
        &fx.worktree.canonicalize().expect("canonical worktree"),
    ));
    let spec = OwnedResourceSpec::new("writer-1", OwnedResourceKind::Process, "t21 tree writer");
    let reservation = scope.reserve_owned_resource(&spec).expect("reserve writer");
    if let Some(child) = child {
        let ticks = process_start_time_ticks(child.id()).expect("child starttime");
        scope
            .confirm_started_resource(
                &reservation,
                StartedResourceIdentity::process(child.id(), ticks),
            )
            .expect("confirm writer");
    }
}

/// 登记容器(kind=Container)writer:生产无引擎观察通道 → 静止不可证。
async fn register_tree_container_scope(fx: &WindowStartupFixture, label: &str) {
    let store_root = fx.paths().root().join("provider-resources");
    let private = fx.root.join("scope-private").join(label);
    std::fs::create_dir_all(&private).expect("private root");
    let key = ResourceScopeKey {
        scope_id: format!("t21-ctr-{label}"),
        run_id: format!("run-{label}"),
        boot_identity: new_boot_identity(),
    };
    let scope = ProviderRunResourceScope::open_for_recovery(
        &key,
        Some(&fx.window_id),
        &private,
        &store_root,
        None,
    )
    .expect("open scope")
    .with_worktree_key(&tree_key_of(
        &fx.worktree.canonicalize().expect("canonical worktree"),
    ));
    let spec = OwnedResourceSpec::new("ctr-1", OwnedResourceKind::Container, "t21 container");
    let reservation = scope
        .reserve_owned_resource(&spec)
        .expect("reserve container");
    scope
        .confirm_started_resource(
            &reservation,
            StartedResourceIdentity::container("t21-ctr-1"),
        )
        .expect("confirm container");
}

fn spawn_group_child() -> std::process::Child {
    use std::os::unix::process::CommandExt;
    std::process::Command::new("sleep")
        .arg("120")
        .process_group(0)
        .spawn()
        .expect("spawn group writer")
}

/// 推进到 ViolationRecorded 前缀(违规 journal 已落盘,恢复未开始)。
async fn drive_to_violation_recorded(fx: &WindowStartupFixture) {
    fx.open_window(&fx.window_id, "run-t21-viol").await;
    pollute_reviewer_commit(fx);
    let store = fx.window_store();
    let record = fx.window_record(&fx.window_id);
    let quiescing = crate::product::reviewer_window::begin_quiesce(
        &store,
        &fx.window_id,
        record.generation,
        "t21 crash prefix",
    )
    .expect("quiesce");
    crate::product::reviewer_window::record_violation(
        &store,
        &snapshot_store(fx),
        &fx.window_id,
        quiescing.generation,
        crate::product::reviewer_window::ViolationRecord {
            kind: "git_domain_mutation",
            path: ".git[HEAD]",
            before_digest: None,
            after_digest: None,
            detail: "t21 seeded violation",
            report_id: None,
        },
    )
    .expect("record violation");
    assert_eq!(
        fx.window_record(&fx.window_id).state,
        ReviewerWindowState::ViolationRecorded
    );
}

/// 推进到 ManualGate(违规恢复完整)。
async fn drive_to_manual_gate(fx: &WindowStartupFixture) {
    drive_to_violation_recorded(fx).await;
    let outcomes = run_startup_compensation(fx).await;
    assert_eq!(outcomes.len(), 1);
    assert!(!outcomes[0].released, "violation window must keep lock");
    assert_eq!(
        fx.window_record(&fx.window_id).state,
        ReviewerWindowState::ManualGate
    );
}

/// 崩溃前缀「租约持久/manifest 未全」:Preparing + 租约 → 启动补偿弃置
/// 释放(无 baseline ⇒ 树无污染可恢复;不重采);重复启动零补偿。
#[tokio::test]
async fn crash_prefix_preparing_with_lease_abandons_without_recapture() {
    let fx = WindowStartupFixture::new("prep");
    prepare_leased_window(&fx, "run-t21-prep");

    let outcomes = run_startup_compensation(&fx).await;
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].released);
    let record = fx.window_record(&fx.window_id);
    assert_eq!(record.state, ReviewerWindowState::ClosedInvalidated);
    assert!(!lease_held(&fx), "preparing 崩溃前缀必须释放租约");
    assert!(
        !fx.paths()
            .root()
            .join("reviewer-snapshots")
            .join("windows")
            .join(&fx.window_id)
            .exists(),
        "未完成开窗不得重采 baseline"
    );
    assert!(fx.role_runs_empty(), "补偿期间零 provider 会话");
    assert!(fx.channel_allows_platform_write(), "通道解冻");

    // 重复启动:终态窗口零补偿、零状态推进。
    let second = run_startup_compensation(&fx).await;
    assert!(second.is_empty(), "终态窗口不再补偿");
    assert_eq!(
        fx.window_record(&fx.window_id).generation,
        record.generation
    );
}

/// 崩溃前缀「恢复游标中途」:T16 failpoint 注入中断 → 启动补偿按**原
/// manifest** 续接完成(指纹不变,不重冻/不重采)并释放。
#[tokio::test]
async fn crash_prefix_mid_cursor_resumes_from_original_manifest() {
    let fx = WindowStartupFixture::new("cursor");
    fx.open_window(&fx.window_id, "run-t21-cursor").await;
    let window_store = fx.window_store();
    let snapshot = snapshot_store(&fx);
    let record = fx.window_record(&fx.window_id);
    let quiescing = crate::product::reviewer_window::begin_quiesce(
        &window_store,
        &fx.window_id,
        record.generation,
        "t21 crash prefix",
    )
    .expect("quiesce");
    snapshot.set_restore_failpoint_after_cursor_writes(1);
    let interrupted = crate::product::reviewer_window::restore_window(
        &window_store,
        &snapshot,
        &fx.window_id,
        quiescing.generation,
    )
    .await;
    assert!(interrupted.is_err(), "failpoint must interrupt mid-cursor");
    assert_eq!(
        fx.window_record(&fx.window_id).state,
        ReviewerWindowState::Restoring,
        "游标中途崩溃留在 Restoring"
    );
    let fingerprint_before = crate::product::reviewer_window::manifest_fingerprint(
        &snapshot.load_manifest(&fx.window_id).expect("manifest"),
    );

    // 「崩溃重启」:全新 engine/store(failpoint 属实例级,新实例失效)。
    let outcomes = run_startup_compensation(&fx).await;
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].released);
    assert_eq!(
        fx.window_record(&fx.window_id).state,
        ReviewerWindowState::ClosedInvalidated
    );
    assert!(!lease_held(&fx));
    let fingerprint_after = crate::product::reviewer_window::manifest_fingerprint(
        &snapshot_store(&fx)
            .load_manifest(&fx.window_id)
            .expect("manifest after"),
    );
    assert_eq!(
        fingerprint_before, fingerprint_after,
        "续恢复必须按原 manifest(内容指纹不变,不重采)"
    );
    assert!(fx.role_runs_empty(), "补偿期间零 provider 会话");
}

/// 崩溃前缀「ViolationRecorded」:启动补偿先持久违规(journal-first,
/// T18 链内建)→ 按原 manifest 恢复 → ManualGate 保持锁并投影 attempt
/// 人工恢复面;Coder 原 dirty 保留;重复启动零重复补偿/零新 session。
#[tokio::test]
async fn violation_window_reaches_manual_gate_keeps_lock_and_projects_attempt() {
    let fx = WindowStartupFixture::new("viol");
    let baseline_head = fx.head();
    drive_to_violation_recorded(&fx).await;

    let outcomes = run_startup_compensation(&fx).await;
    assert_eq!(outcomes.len(), 1);
    assert!(!outcomes[0].released);
    assert_eq!(
        fx.window_record(&fx.window_id).state,
        ReviewerWindowState::ManualGate
    );
    assert!(lease_held(&fx), "违规窗人工门保持锁");
    assert!(
        !fx.channel_allows_platform_write(),
        "人工门期间平台写/材料全拒"
    );
    assert!(fx.role_runs_empty(), "补偿期间零 provider 会话");
    let attempt = fx.reload_attempt();
    assert_eq!(attempt.status, CodingAttemptStatus::AwaitingManualRecovery);
    assert_eq!(
        attempt.manual_recovery_reason.as_deref(),
        Some("reviewer_window_manual_gate")
    );
    // 树回基线(恢复完整);证据在 journal;Coder dirty 保留。
    assert_eq!(fx.head(), baseline_head);
    assert!(!fx.worktree.join("reviewer-change.txt").exists());
    assert!(fx.worktree.join("coder-dirty.txt").exists());
    let journal = fx
        .window_store()
        .load_violations(&fx.window_id)
        .expect("journal");
    assert!(
        journal
            .iter()
            .any(|entry| entry.kind == "git_domain_mutation"),
        "journal 含违规事实"
    );

    // 重复启动:零状态推进、零重复 journal、零新 session。
    let generation = fx.window_record(&fx.window_id).generation;
    let journal_len = journal.len();
    let second = run_startup_compensation(&fx).await;
    assert_eq!(second.len(), 1);
    assert!(!second[0].released);
    assert_eq!(
        fx.window_record(&fx.window_id).generation,
        generation,
        "ManualGate 二次启动零状态推进"
    );
    assert_eq!(
        fx.window_store()
            .load_violations(&fx.window_id)
            .expect("journal")
            .len(),
        journal_len,
        "无重复补偿(journal 不增)"
    );
    assert!(fx.role_runs_empty(), "重复启动无新 session");
}

/// journal 不一致(序号断裂):启动补偿经恢复链篡改检测 → ManualGate
/// 保持锁(不释放、不洗白)。
#[tokio::test]
async fn journal_inconsistency_keeps_lock_via_manual_gate() {
    let fx = WindowStartupFixture::new("tamper");
    drive_to_violation_recorded(&fx).await;
    let journal_path = fx
        .paths()
        .root()
        .join("reviewer-windows")
        .join("violations")
        .join(format!("{}.json", fx.window_id));
    let mut journal: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&journal_path).expect("journal file"))
            .expect("journal json");
    journal["entries"][0]["seq"] = serde_json::json!(9);
    std::fs::write(
        &journal_path,
        serde_json::to_string(&journal).expect("write json"),
    )
    .expect("tamper journal");

    let outcomes = run_startup_compensation(&fx).await;
    assert_eq!(outcomes.len(), 1);
    assert!(!outcomes[0].released, "journal 不一致必须维持锁");
    assert_eq!(
        fx.window_record(&fx.window_id).state,
        ReviewerWindowState::ManualGate
    );
    assert!(lease_held(&fx));
    assert!(fx.role_runs_empty());
}

/// 人工拒绝补偿(Invalidate)不解锁 attempt:窗口按 T17 语义关闭释放,
/// 但 attempt 不自动复活、旧报告永久 Invalidated、启动分诊零恢复;
/// 人工许可新轮(RequireNewReview)另取租约/新基线,不 resume 旧报告。
#[tokio::test]
async fn manual_gate_invalidate_keeps_attempt_locked_and_new_round_takes_new_lease() {
    let fx = WindowStartupFixture::new("resolve-invalidate");
    drive_to_manual_gate(&fx).await;
    let engine = fx.engine();
    let resolved = engine
        .resolve_reviewer_manual_gate_for_attempt(&fx.attempt.id, ManualGateDecision::Invalidate)
        .expect("resolve invalidate");
    assert_eq!(resolved.state, ReviewerWindowState::ClosedInvalidated);
    assert_eq!(resolved.report_acceptance, ReportAcceptance::Invalidated);
    assert!(!lease_held(&fx), "T17 终态语义:窗口关闭释放树");
    assert!(fx.channel_allows_platform_write());
    assert_eq!(
        engine.recheck_review_acceptance_for_delivery(Some(&fx.window_id)),
        Some(ReportAcceptance::Invalidated),
        "旧报告永久 Invalidated,不可 resume"
    );
    // 人工拒绝补偿不解锁 attempt:不自动复活 runner;分诊零恢复。
    assert_eq!(
        fx.reload_attempt().status,
        CodingAttemptStatus::AwaitingManualRecovery,
        "拒绝补偿后 attempt 停在人工恢复面(无自动 resume)"
    );
    let state = fx.restart_state();
    let resumed = crate::web::autopilot_orchestrator::reconcile_claimed_coding_runs_once(&state)
        .await
        .expect("reconcile");
    assert_eq!(resumed, 0, "人工恢复态不自动复活");
    assert_eq!(state.coding_runs.runner_count(&fx.attempt_key()), 0);
    // journal 保留审计。
    assert!(
        !fx.window_store()
            .load_violations(&fx.window_id)
            .expect("journal")
            .is_empty()
    );

    // 新轮变体:RequireNewReview → 新窗口另取租约/新基线。
    let fx2 = WindowStartupFixture::new("resolve-newround");
    drive_to_manual_gate(&fx2).await;
    let resolved2 = fx2
        .engine()
        .resolve_reviewer_manual_gate_for_attempt(
            &fx2.attempt.id,
            ManualGateDecision::RequireNewReview,
        )
        .expect("resolve require_new_review");
    assert_eq!(resolved2.state, ReviewerWindowState::NewReviewRequired);
    assert!(!lease_held(&fx2));
    let new_window = format!("{}-round2", fx2.window_id);
    fx2.open_window(&new_window, "run-t21-round2").await;
    let tree_key = tree_key_of(&fx2.worktree.canonicalize().expect("canonical"));
    let lease = fx2
        .window_store()
        .load_lease(&tree_key)
        .expect("lease")
        .expect("new lease taken");
    assert_eq!(lease.window_id, new_window, "新轮另取租约(新 lease)");
    assert_ne!(lease.lease_id, format!("wlease-{}", fx2.window_id));
    assert!(
        snapshot_store(&fx2)
            .load_manifest(&new_window)
            .expect("new baseline manifest")
            .window_id
            == new_window,
        "新轮新基线 manifest"
    );
    assert_eq!(
        fx2.engine()
            .recheck_review_acceptance_for_delivery(Some(&fx2.window_id)),
        Some(ReportAcceptance::Invalidated),
        "旧违规报告不可 resume"
    );
}

/// REST 生产入口(reviewer-window:resolve)路由可达并驱动同一 engine 裁决。
#[tokio::test]
async fn reviewer_window_resolve_route_decides_manual_gate() {
    let fx = WindowStartupFixture::new("route");
    drive_to_manual_gate(&fx).await;
    let state = fx.restart_state();
    let app = crate::web::app::build_web_router(state);
    let response = tower::ServiceExt::oneshot(
        app,
        axum::http::Request::builder()
            .method("POST")
            .uri(format!(
                "/api/coding-attempts/{}/reviewer-window:resolve",
                fx.attempt.id
            ))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::json!({"decision": "require_new_review"}).to_string(),
            ))
            .expect("request"),
    )
    .await
    .expect("response");
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    let value: serde_json::Value = serde_json::from_slice(&body).expect("json body");
    assert_eq!(value["state"], "new_review_required");
    assert_eq!(value["report_acceptance"], "invalidated");
    assert!(!lease_held(&fx), "裁决后窗口终态释放");
    // 非法 decision → 校验拒绝。
    let bad = crate::web::app::build_web_router(fx.restart_state());
    let response = tower::ServiceExt::oneshot(
        bad,
        axum::http::Request::builder()
            .method("POST")
            .uri(format!(
                "/api/coding-attempts/{}/reviewer-window:resolve",
                fx.attempt.id
            ))
            .header("content-type", "application/json")
            .body(axum::body::Body::from(
                serde_json::json!({"decision": "whitewash"}).to_string(),
            ))
            .expect("request"),
    )
    .await
    .expect("response");
    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
}

/// 未知 writer(未确认预留,是否已 spawn 不可证):启动补偿停等
/// RecoveryRequired,租约/通道保持,provider/材料计数 0,该树分诊零 runner。
#[tokio::test]
async fn unknown_writer_keeps_lock_zero_provider_activity() {
    let fx = WindowStartupFixture::new("unknown");
    fx.open_window(&fx.window_id, "run-t21-unknown").await;
    register_tree_writer_scope(&fx, "unknown", None).await;

    let outcomes = run_startup_compensation(&fx).await;
    assert_eq!(outcomes.len(), 1);
    assert!(!outcomes[0].released);
    assert_eq!(
        fx.window_record(&fx.window_id).state,
        ReviewerWindowState::RecoveryRequired
    );
    assert!(
        outcomes[0]
            .manual_attention
            .as_deref()
            .unwrap_or_default()
            .contains("unconfirmed"),
        "停等原因可见:{:?}",
        outcomes[0].manual_attention
    );
    assert!(lease_held(&fx), "未知 writer 维持锁");
    assert!(
        !fx.channel_allows_platform_write(),
        "停等期间平台写/材料全拒"
    );
    assert!(fx.role_runs_empty(), "provider/材料计数 0");
    assert_eq!(
        fx.reload_attempt().status,
        CodingAttemptStatus::AwaitingManualRecovery,
        "停等投影人工恢复面"
    );
    let state = fx.restart_state();
    let resumed = crate::web::autopilot_orchestrator::reconcile_claimed_coding_runs_once(&state)
        .await
        .expect("reconcile");
    assert_eq!(resumed, 0, "该树持续 fail-closed");
    assert_eq!(state.coding_runs.runner_count(&fx.attempt_key()), 0);
}

/// 登记资源停清:真实存活 writer 进程(组长)被停止(SIGKILL 组信号 +
/// 身份观察确认)且 scope 清理;窗口继续完成补偿释放。
#[tokio::test]
async fn registered_writer_process_stopped_and_scope_cleared() {
    let fx = WindowStartupFixture::new("procstop");
    fx.open_window(&fx.window_id, "run-t21-procstop").await;
    let mut child = spawn_group_child();
    register_tree_writer_scope(&fx, "procstop", Some(&child)).await;

    let outcomes = run_startup_compensation(&fx).await;
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].released, "静止证明达成后照常补偿");
    let status = child
        .try_wait()
        .expect("try_wait")
        .expect("writer process must be stopped");
    assert_eq!(
        status.signal(),
        Some(9),
        "killpg 送达且以真实身份观察确认退出(killpg 不当成功证据)"
    );
    let resource_store = crate::product::provider_resource_store::ProviderResourceStore::new(
        fx.paths().root().join("provider-resources"),
    );
    let scope_ids: Vec<String> = resource_store
        .scope_keys()
        .expect("scope keys")
        .into_iter()
        .map(|key| key.scope_id)
        .collect();
    assert!(
        !scope_ids.iter().any(|id| id == "t21-writer-procstop"),
        "登记资源停清(scope 记录清理):{scope_ids:?}"
    );
    assert!(!lease_held(&fx));
    assert_eq!(
        fx.window_record(&fx.window_id).state,
        ReviewerWindowState::ClosedInvalidated
    );
}

/// 登记容器无引擎观察通道:静止不可证(Drop/stop 声称不当成功证据)→
/// 停等保持锁。
#[tokio::test]
async fn container_without_observer_parks_lock_kept() {
    let fx = WindowStartupFixture::new("ctr");
    fx.open_window(&fx.window_id, "run-t21-ctr").await;
    register_tree_container_scope(&fx, "ctr").await;

    let outcomes = run_startup_compensation(&fx).await;
    assert_eq!(outcomes.len(), 1);
    assert!(!outcomes[0].released, "容器静止不可证必须停等");
    assert_eq!(
        fx.window_record(&fx.window_id).state,
        ReviewerWindowState::RecoveryRequired
    );
    assert!(lease_held(&fx));
    assert!(fx.role_runs_empty());
}

/// 局部坏窗:坏树(未知 writer)停等 fail-closed,无冲突树照常补偿并在
/// 分诊中恢复 runner;窗口收口先于下一材料采集(trace)。
#[tokio::test]
async fn bad_tree_fail_closed_while_conflict_free_tree_resumes() {
    let root = t21_dir();
    let bad = WindowStartupFixture::new_in(root.path(), "treeA");
    let good = WindowStartupFixture::new_in(root.path(), "treeB");
    bad.open_window(&bad.window_id, "run-t21-bad").await;
    register_tree_writer_scope(&bad, "bad", None).await;
    good.open_window(&good.window_id, "run-t21-good").await;
    pollute_worktree_scratch(&good);

    let outcomes = bad
        .engine()
        .compensate_open_reviewer_windows_at_startup()
        .await;
    assert_eq!(outcomes.len(), 2, "局部坏窗不停扫");
    let bad_outcome = outcomes
        .iter()
        .find(|outcome| outcome.window_id == bad.window_id)
        .expect("bad outcome");
    let good_outcome = outcomes
        .iter()
        .find(|outcome| outcome.window_id == good.window_id)
        .expect("good outcome");
    assert!(!bad_outcome.released);
    assert!(good_outcome.released);
    assert_eq!(
        bad.window_record(&bad.window_id).state,
        ReviewerWindowState::RecoveryRequired
    );
    assert_eq!(
        good.window_record(&good.window_id).state,
        ReviewerWindowState::ClosedInvalidated
    );
    assert!(!good.worktree.join("reviewer-scratch.txt").exists());

    // 分诊:好树恢复 runner(材料采集放行),坏树零 runner。
    let state = bad.restart_state();
    let resumed = crate::web::autopilot_orchestrator::reconcile_claimed_coding_runs_once(&state)
        .await
        .expect("reconcile");
    assert_eq!(resumed, 1, "无冲突树照常放行");
    assert_eq!(
        state.coding_runs.runner_count(&good.attempt_key()),
        1,
        "好树 runner 放行(补偿完成后)"
    );
    assert_eq!(state.coding_runs.runner_count(&bad.attempt_key()), 0);
    assert_eq!(
        bad.reload_attempt().status,
        CodingAttemptStatus::AwaitingManualRecovery
    );
    // trace:好树窗口终态迁移时间戳早于分诊后观察时刻(baseline 恢复/
    // 窗口收口先于下一材料采集)。
    let terminal_at = good
        .window_record(&good.window_id)
        .transitions
        .last()
        .expect("terminal transition")
        .at
        .clone();
    let terminal_at =
        chrono::DateTime::parse_from_rfc3339(&terminal_at).expect("rfc3339 transition timestamp");
    assert!(
        terminal_at.with_timezone(&chrono::Utc) < chrono::Utc::now(),
        "窗口收口先于下一材料采集(trace)"
    );
}

/// attempt 终止 → reviewer 窗口与租约收口 + T14 code_delivery scope 回收;
/// Coder 原 dirty 保留,attempt Aborted。
#[tokio::test]
async fn attempt_termination_closes_windows_scopes_and_keeps_coder_dirty() {
    let fx = WindowStartupFixture::new("term");
    fx.open_window(&fx.window_id, "run-t21-term").await;
    pollute_worktree_scratch(&fx);
    // T14 登记面:真实 role run + code_delivery scope(scratch 目录资源)。
    let role_run = fx
        .store
        .create_role_run(
            &fx.attempt,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::Initial,
            None,
        )
        .expect("role run");
    let scope = fx
        .engine()
        .begin_coder_delivery_scope(&fx.attempt, &role_run, &fx.worktree)
        .await
        .expect("coder delivery scope");
    drop(scope);

    fx.engine()
        .finalize_attempt_for_plan_termination(
            &fx.attempt.project_id,
            &fx.attempt.issue_id,
            &fx.attempt.id,
        )
        .await
        .expect("finalize termination");

    assert_eq!(
        fx.window_record(&fx.window_id).state,
        ReviewerWindowState::ClosedInvalidated,
        "attempt 终止→reviewer-windows 收口"
    );
    assert!(!lease_held(&fx), "租约释放");
    assert!(
        fx.worktree.join("coder-dirty.txt").exists(),
        "Coder dirty 保留"
    );
    assert!(
        !fx.worktree.join("reviewer-scratch.txt").exists(),
        "reviewer 残留按基线恢复"
    );
    assert!(
        fx.store
            .coder_delivery_scope_keys_for_attempt(&fx.attempt.id)
            .expect("coder delivery keys")
            .is_empty(),
        "T14:code_delivery 登记资源回收"
    );
    assert_eq!(fx.reload_attempt().status, CodingAttemptStatus::Aborted);
}

/// attempt 删除语义:停等窗(未知 writer)也无条件收口——通道解冻+租约
/// 释放,不留冻结残锁卡共享 common-dir 的 git mutation。
#[tokio::test]
async fn attempt_delete_close_is_unconditional_after_park() {
    let fx = WindowStartupFixture::new("del");
    fx.open_window(&fx.window_id, "run-t21-del").await;
    register_tree_writer_scope(&fx, "del", None).await;
    let outcomes = run_startup_compensation(&fx).await;
    assert!(!outcomes[0].released, "先经启动补偿停等(未知 writer)");
    assert_eq!(
        fx.window_record(&fx.window_id).state,
        ReviewerWindowState::RecoveryRequired
    );

    let closed = fx
        .engine()
        .close_reviewer_windows_for_attempt(
            &fx.reload_attempt(),
            crate::product::coding_workspace_engine::AttemptWindowCloseMode::Delete,
        )
        .await
        .expect("delete close");
    assert_eq!(closed, 1);
    assert!(!lease_held(&fx), "删除语义=无条件释放(工作树随之删除)");
    assert!(fx.channel_allows_platform_write(), "通道解冻");
    assert_eq!(
        fx.window_record(&fx.window_id).state,
        ReviewerWindowState::ClosedInvalidated
    );
}

/// 人工恢复续接(socket RecoverCoding 前置 engine 面):恢复中(Restoring)
/// redrive 续接完成放行;ManualGate 需显式裁决;无窗 attempt 直接放行。
#[tokio::test]
async fn manual_redrive_completes_recovery_or_requires_decision() {
    let fx = WindowStartupFixture::new("redrive");
    fx.open_window(&fx.window_id, "run-t21-redrive").await;
    let store = fx.window_store();
    let snapshot = snapshot_store(&fx);
    let record = fx.window_record(&fx.window_id);
    let quiescing = crate::product::reviewer_window::begin_quiesce(
        &store,
        &fx.window_id,
        record.generation,
        "t21 crash prefix",
    )
    .expect("quiesce");
    snapshot.set_restore_failpoint_after_cursor_writes(1);
    assert!(
        crate::product::reviewer_window::restore_window(
            &store,
            &snapshot,
            &fx.window_id,
            quiescing.generation
        )
        .await
        .is_err(),
        "游标中途中断"
    );
    fx.engine()
        .redrive_reviewer_window_recovery_for_attempt(&fx.reload_attempt())
        .await
        .expect("redrive completes recovery");
    assert_eq!(
        fx.window_record(&fx.window_id).state,
        ReviewerWindowState::ClosedInvalidated
    );
    assert!(!lease_held(&fx));

    // ManualGate:redrive 不代作废,需显式裁决。
    let fx2 = WindowStartupFixture::new("redrive-gate");
    drive_to_manual_gate(&fx2).await;
    let error = fx2
        .engine()
        .redrive_reviewer_window_recovery_for_attempt(&fx2.reload_attempt())
        .await
        .expect_err("manual gate requires explicit decision");
    assert!(
        error.to_string().contains("manual_gate_requires_decision"),
        "{error}"
    );
    assert_eq!(
        fx2.window_record(&fx2.window_id).state,
        ReviewerWindowState::ManualGate,
        "redrive 不代人工作废"
    );

    // 无窗 attempt:直接放行(恢复续接面零副作用)。
    let fx3 = WindowStartupFixture::new("redrive-none");
    fx3.engine()
        .redrive_reviewer_window_recovery_for_attempt(&fx3.reload_attempt())
        .await
        .expect("no window: proceed");
}
