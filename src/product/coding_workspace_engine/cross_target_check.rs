//! post-hoc 越界检测子系统（REQ-COD-01 分层 c 的 Scenario 2 检测部分）。
//!
//! 多仓 issue 的每个 provider role run（Coder/Reviewer/rework/auto-retry）启动前，
//! 采集该 attempt 涉及的**所有成员主 checkout** 的 `git rev-parse HEAD` 与
//! `git status --porcelain` 快照并持久化；run 结束后（Task 15 统一门）重采比对，
//! 任一成员主 checkout 的 HEAD 或工作区发生变化即视为越界。
//!
//! 诚实边界：只监控成员主 checkout 的 HEAD/status。symlink 跳出仓外、`/tmp` 等
//! 绝对路径写入、provider 运行中的实时越界不属于本模块检测范围（交 C-2 的
//! provider 写配置约束 + capability 版本钉定 + 越界矩阵 fixture）。
//!
//! 单仓红线：Legacy attempt（`target_snapshot` 为 `None`）不采 baseline、不做
//! 检测，行为与既有路径一致；post-hoc 检测主要服务多仓（含单仓逻辑代码库的
//! 主 checkout 本身）。

use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::{CodingAttemptStore, StableCode};
use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::json_store::{read_json, write_json};
use crate::product::logical_codebase::{
    CheckoutAvailability, CheckoutKind, LogicalCodebaseStore, LogicalRepositoryId,
};

/// 单个成员主 checkout 的不可变快照（HEAD + 工作区 porcelain 状态）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MemberCheckoutSnapshot {
    pub(crate) logical_repository_id: LogicalRepositoryId,
    pub(crate) canonical_path: PathBuf,
    pub(crate) head_revision: String,
    pub(crate) porcelain_status: String,
}

/// 一个 provider role run 的越界检测基线，持久化到
/// `coding-attempts/{attempt_id}/cross-target-baselines/{run_id}.json`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CrossTargetBaseline {
    pub(crate) run_id: String,
    pub(crate) captured_at: String,
    pub(crate) member_checkouts: Vec<MemberCheckoutSnapshot>,
}

/// provider role run 启动前采集并持久化所有成员主 checkout 的越界基线。
///
/// Legacy attempt（`target_snapshot` 为 `None`）不采 baseline，返回空基线且不落盘，
/// 保持单仓路径现状。
pub(crate) fn capture_cross_target_baseline(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    run_id: &str,
) -> Result<CrossTargetBaseline, StableCode> {
    let now = Utc::now().to_rfc3339();
    if attempt.target_snapshot.is_none() {
        // 单仓红线：Legacy attempt 不采多仓 baseline，行为不变。
        return Ok(CrossTargetBaseline {
            run_id: run_id.to_string(),
            captured_at: now,
            member_checkouts: Vec::new(),
        });
    }

    // v1.3：按 attempt 所属 issue 的 lc_id 寻址（R9 编码/交付链切换点）；
    // 单仓/无 LC 回退 legacy project 级路径。
    let lc_id = crate::product::logical_codebase::resolve_issue_logical_codebase_id(
        paths,
        &attempt.project_id,
        &attempt.issue_id,
    )
    .map_err(|_| StableCode::CrossTargetStoreFailure)?;
    let authority = match lc_id.as_deref() {
        Some(lc_id) => LogicalCodebaseStore::for_lc(paths.clone(), lc_id),
        None => LogicalCodebaseStore::new(paths.clone()),
    };
    let manifest = authority
        .load_manifest(&attempt.project_id)
        .map_err(|_| StableCode::CrossTargetStoreFailure)?
        .ok_or(StableCode::CrossTargetStoreFailure)?;
    let checkouts = authority
        .list_checkouts(&attempt.project_id)
        .map_err(|_| StableCode::CrossTargetStoreFailure)?;

    let mut member_checkouts = Vec::with_capacity(manifest.member_ids.len());
    for member_id in &manifest.member_ids {
        let main_checkouts: Vec<_> = checkouts
            .iter()
            .filter(|checkout| {
                checkout.logical_repository_id == *member_id && checkout.kind == CheckoutKind::Main
            })
            .collect();
        let [checkout] = main_checkouts.as_slice() else {
            return Err(StableCode::CrossTargetStoreFailure);
        };
        if checkout.availability != CheckoutAvailability::Available {
            return Err(StableCode::CrossTargetStoreFailure);
        }
        member_checkouts.push(MemberCheckoutSnapshot {
            logical_repository_id: *member_id,
            canonical_path: checkout.canonical_path.clone(),
            head_revision: git_head(&checkout.canonical_path)?,
            porcelain_status: git_status(&checkout.canonical_path)?,
        });
    }

    let baseline = CrossTargetBaseline {
        run_id: run_id.to_string(),
        captured_at: now,
        member_checkouts,
    };
    let baseline_path = CodingAttemptStore::new(paths.clone())
        .attempt_cross_target_baselines_path(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            run_id,
        )
        .map_err(|_| StableCode::CrossTargetStoreFailure)?;
    write_json(&baseline_path, &baseline).map_err(|_| StableCode::CrossTargetStoreFailure)?;

    Ok(baseline)
}

/// provider role run 结束后重采各成员主 checkout 并与基线比对。
///
/// - 任一成员主 checkout 的 HEAD 或工作区变更 → `cross_target_violation_detected`
/// - baseline 文件缺失（崩溃重启/丢失）→ `cross_target_baseline_missing`
/// - Legacy attempt 不检测，直接放行。
pub(crate) fn detect_cross_target_violation(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    run_id: &str,
) -> Result<(), StableCode> {
    if attempt.target_snapshot.is_none() {
        // 单仓红线：Legacy attempt 不检测，行为不变。
        return Ok(());
    }

    let baseline_path = CodingAttemptStore::new(paths.clone())
        .attempt_cross_target_baselines_path(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            run_id,
        )
        .map_err(|_| StableCode::CrossTargetStoreFailure)?;
    if !baseline_path.exists() {
        return Err(StableCode::CrossTargetBaselineMissing);
    }
    let baseline: CrossTargetBaseline =
        read_json(&baseline_path).map_err(|_| StableCode::CrossTargetStoreFailure)?;

    for snapshot in &baseline.member_checkouts {
        let head = git_head(&snapshot.canonical_path)?;
        let status = git_status(&snapshot.canonical_path)?;
        if head != snapshot.head_revision || status != snapshot.porcelain_status {
            return Err(StableCode::CrossTargetViolationDetected);
        }
    }

    Ok(())
}

/// 交付前统一门（Task 15）：对 attempt 的全部 provider role run 逐个重采比对。
///
/// - Legacy attempt（`target_snapshot` 为 `None`）→ 单仓红线，直接放行；
/// - 无 role run（未跑 provider run）→ 按无越界放行；
/// - 每个 role run 经 [`detect_cross_target_violation`] 比对：任一越界 →
///   `cross_target_violation_detected`，基线文件缺失（崩溃重启/丢失）→
///   `cross_target_baseline_missing`，均阻断交付。
pub(crate) fn detect_cross_target_violation_for_delivery(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<(), StableCode> {
    if attempt.target_snapshot.is_none() {
        // 单仓红线：Legacy attempt 不做任何越界检测。
        return Ok(());
    }
    let store = CodingAttemptStore::new(paths.clone());
    let role_runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .map_err(|_| StableCode::CrossTargetStoreFailure)?;
    for role_run in &role_runs {
        detect_cross_target_violation(paths, attempt, &role_run.id)?;
    }
    Ok(())
}

fn git_head(path: &Path) -> Result<String, StableCode> {
    git_stdout(path, &["rev-parse", "HEAD"]).map(|revision| revision.trim().to_string())
}

fn git_status(path: &Path) -> Result<String, StableCode> {
    git_stdout(path, &["status", "--porcelain"])
}

fn git_stdout(cwd: &Path, args: &[&str]) -> Result<String, StableCode> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|_| StableCode::CrossTargetStoreFailure)?;
    if !output.status.success() {
        return Err(StableCode::CrossTargetStoreFailure);
    }
    String::from_utf8(output.stdout).map_err(|_| StableCode::CrossTargetStoreFailure)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::coding_models::{
        AttemptTargetSnapshot, CodingAttemptScope, CodingAttemptStatus, CodingExecutionStage,
        CodingProviderRole, CodingRoleRunTrigger,
    };
    use crate::product::coding_workspace_engine::{
        CodingWorkspaceEngine, CodingWorkspaceEngineError,
    };
    use crate::product::git_workspace_service::GitWorkspaceService;
    use crate::product::logical_codebase::{
        CheckoutAvailability, CodebaseMemberRecord, LogicalCodebaseManifest, LogicalCodebaseStore,
        MemberStatus, RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity,
        RepositoryType,
    };
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;
    use std::fs;
    use std::process::Command as StdCommand;
    use uuid::Uuid;

    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_0001";
    const ATTEMPT_ID: &str = "coding_attempt_0001";

    struct Fixture {
        _temp: tempfile::TempDir,
        paths: ProductAppPaths,
        attempt: CodingExecutionAttempt,
        checkout_paths: Vec<PathBuf>,
    }

    fn run_git(cwd: &Path, args: &[&str]) {
        let output = StdCommand::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap_or_else(|error| panic!("git {} failed to start: {error}", args.join(" ")));
        assert!(
            output.status.success(),
            "git {} failed\nstdout:\n{}\nstderr:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_stdout(cwd: &Path, args: &[&str]) -> String {
        let output = StdCommand::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap_or_else(|error| panic!("git {} failed to start: {error}", args.join(" ")));
        assert!(
            output.status.success(),
            "git {} failed\nstdout:\n{}\nstderr:\n{}",
            args.join(" "),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    fn init_repo(path: &Path) {
        fs::create_dir_all(path).expect("create repo dir");
        run_git(path, &["init"]);
        run_git(path, &["config", "user.email", "aria@example.com"]);
        run_git(path, &["config", "user.name", "Aria Test"]);
        fs::write(path.join("README.md"), "initial\n").expect("seed file");
        run_git(path, &["add", "."]);
        run_git(path, &["commit", "-m", "initial"]);
    }

    /// 构造一个双成员多仓 fixture：两个真实 git 仓库作为成员主 checkout，
    /// manifest 登记两个成员，attempt 携带 target_snapshot（逻辑代码库 attempt）。
    fn two_member_fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));

        let member_a_id = LogicalRepositoryId(Uuid::new_v4());
        let member_b_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_a_id = RepositoryCheckoutId(Uuid::new_v4());
        let checkout_b_id = RepositoryCheckoutId(Uuid::new_v4());
        let checkout_a = temp.path().join("repo_a");
        let checkout_b = temp.path().join("repo_b");
        init_repo(&checkout_a);
        init_repo(&checkout_b);

        let authority = LogicalCodebaseStore::new(paths.clone());
        let manifest = LogicalCodebaseManifest::new(
            PROJECT_ID,
            temp.path().join("aggregate-root"),
            vec![member_a_id, member_b_id],
        );
        authority.save_manifest(PROJECT_ID, &manifest).unwrap();

        let now = "2026-08-11T00:00:00Z".to_string();
        for (member_id, checkout_id, checkout_path) in [
            (member_a_id, checkout_a_id, &checkout_a),
            (member_b_id, checkout_b_id, &checkout_b),
        ] {
            let source_identity = RepositorySourceIdentity::from_git_parts(
                checkout_path,
                checkout_path.join(".git"),
                None,
            );
            authority
                .save_member(
                    PROJECT_ID,
                    &CodebaseMemberRecord {
                        logical_repository_id: member_id,
                        physical_repository_id: "repository_0001".to_string(),
                        alias: "repo".to_string(),
                        role: "repository".to_string(),
                        ordinal: 1,
                        source_identity,
                        repo_type: RepositoryType::Unknown,
                        tech_stack: Vec::new(),
                        owner: None,
                        tags: Vec::new(),
                        default_ref: None,
                        checkout_ids: vec![checkout_id],
                        status: MemberStatus::Active,
                        created_at: now.clone(),
                        updated_at: now.clone(),
                    },
                )
                .unwrap();
            authority
                .save_checkout(
                    PROJECT_ID,
                    &RepositoryCheckoutRecord {
                        checkout_id,
                        logical_repository_id: member_id,
                        physical_repository_id: "repository_0001".to_string(),
                        kind: CheckoutKind::Main,
                        canonical_path: checkout_path.clone(),
                        checkout_path_hash: "sha256:checkout".to_string(),
                        git_dir_identity: "sha256:git-dir".to_string(),
                        revision: Some(
                            git_stdout(checkout_path, &["rev-parse", "HEAD"])
                                .trim()
                                .to_string(),
                        ),
                        availability: CheckoutAvailability::Available,
                        observed_at: now.clone(),
                        created_at: now.clone(),
                        updated_at: now.clone(),
                    },
                )
                .unwrap();
        }

        let attempt = logical_attempt(PROJECT_ID, ISSUE_ID, ATTEMPT_ID, &checkout_a);

        Fixture {
            _temp: temp,
            paths,
            attempt,
            checkout_paths: vec![checkout_a, checkout_b],
        }
    }

    fn logical_attempt(
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        target_path: &Path,
    ) -> CodingExecutionAttempt {
        CodingExecutionAttempt {
            id: attempt_id.to_string(),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            work_item_id: "work_item_0001".to_string(),
            attempt_no: 1,
            scope: CodingAttemptScope::WorkItem,
            status: CodingAttemptStatus::Running,
            version: 0,
            manual_recovery_reason: None,
            admission_ticket_consumed_at: None,
            stage: CodingExecutionStage::Coding,
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: crate::product::models::ProviderName::Fake,
                reviewer: None,
                review_rounds: 0,
                permission_modes: Default::default(),
            },
            rework_count: 0,
            max_auto_rework: 0,
            work_item_group_id: None,
            current_work_item_id: Some("work_item_0001".to_string()),
            active_unit_id: None,
            head_commit: None,
            pushed_remote: None,
            review_request_id: None,
            provider_conversations: Vec::new(),
            created_at: "2026-08-11T00:00:00Z".to_string(),
            updated_at: "2026-08-11T00:00:00Z".to_string(),
            target_snapshot: Some(AttemptTargetSnapshot {
                logical_repository_id: LogicalRepositoryId(Uuid::new_v4()),
                checkout_id: RepositoryCheckoutId(Uuid::new_v4()),
                physical_repository_id: "repository_0001".to_string(),
                canonical_path: target_path.to_path_buf(),
                git_dir_identity: "sha256:git-dir".to_string(),
                revision: Some(
                    git_stdout(target_path, &["rev-parse", "HEAD"])
                        .trim()
                        .to_string(),
                ),
                policy_digest: String::new(),
                membership_revision: 1,
                captured_at: "2026-08-11T00:00:00Z".to_string(),
                capture_source: "test".to_string(),
            }),
            completed_at: None,
        }
    }

    fn baseline_path(fixture: &Fixture, run_id: &str) -> PathBuf {
        CodingAttemptStore::new(fixture.paths.clone())
            .attempt_cross_target_baselines_path(PROJECT_ID, ISSUE_ID, ATTEMPT_ID, run_id)
            .unwrap()
    }

    #[test]
    fn capture_persists_baseline_with_every_member_main_checkout() {
        let fixture = two_member_fixture();
        let run_id = "coding_role_run_0001";

        let baseline =
            capture_cross_target_baseline(&fixture.paths, &fixture.attempt, run_id).unwrap();

        assert_eq!(baseline.run_id, run_id);
        assert_eq!(baseline.member_checkouts.len(), 2);
        assert!(
            baseline
                .member_checkouts
                .iter()
                .all(|snapshot| !snapshot.head_revision.is_empty()),
            "every member checkout must carry a real HEAD"
        );
        assert!(
            baseline_path(&fixture, run_id).exists(),
            "baseline must be persisted to the per-role-run path"
        );
    }

    #[test]
    fn detect_passes_when_no_member_checkout_changed() {
        let fixture = two_member_fixture();
        let run_id = "coding_role_run_0001";
        capture_cross_target_baseline(&fixture.paths, &fixture.attempt, run_id).unwrap();

        detect_cross_target_violation(&fixture.paths, &fixture.attempt, run_id)
            .expect("no checkout changed, detection must pass");
    }

    #[test]
    fn detect_rejects_when_another_member_checkout_is_dirty() {
        let fixture = two_member_fixture();
        let run_id = "coding_role_run_0001";
        capture_cross_target_baseline(&fixture.paths, &fixture.attempt, run_id).unwrap();

        // 模拟他仓主 checkout 被写文件（工作区变更）。
        fs::write(
            fixture.checkout_paths[1].join("trespass.txt"),
            "out of worktree write\n",
        )
        .expect("write into other member checkout");

        assert_eq!(
            detect_cross_target_violation(&fixture.paths, &fixture.attempt, run_id),
            Err(StableCode::CrossTargetViolationDetected)
        );
    }

    #[test]
    fn detect_fails_closed_when_baseline_file_missing() {
        let fixture = two_member_fixture();
        let run_id = "coding_role_run_0001";
        capture_cross_target_baseline(&fixture.paths, &fixture.attempt, run_id).unwrap();

        fs::remove_file(baseline_path(&fixture, run_id)).expect("simulate baseline loss");

        assert_eq!(
            detect_cross_target_violation(&fixture.paths, &fixture.attempt, run_id),
            Err(StableCode::CrossTargetBaselineMissing)
        );
    }

    #[test]
    fn role_run_baselines_are_isolated_by_run_id() {
        let fixture = two_member_fixture();
        let first_run = "coding_role_run_0001";
        let second_run = "coding_role_run_0002";

        capture_cross_target_baseline(&fixture.paths, &fixture.attempt, first_run).unwrap();
        capture_cross_target_baseline(&fixture.paths, &fixture.attempt, second_run).unwrap();

        // 只污染 second run 的检测窗口：删掉 first run 基线后，second run 仍可独立检测。
        fs::remove_file(baseline_path(&fixture, first_run)).expect("remove first run baseline");
        assert_eq!(
            detect_cross_target_violation(&fixture.paths, &fixture.attempt, first_run),
            Err(StableCode::CrossTargetBaselineMissing)
        );
        detect_cross_target_violation(&fixture.paths, &fixture.attempt, second_run)
            .expect("second run baseline untouched, detection must pass");
    }

    #[test]
    fn legacy_attempt_skips_capture_and_detection() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let checkout = temp.path().join("legacy_repo");
        init_repo(&checkout);
        let mut attempt = logical_attempt(PROJECT_ID, ISSUE_ID, ATTEMPT_ID, &checkout);
        attempt.target_snapshot = None;

        let baseline = capture_cross_target_baseline(&paths, &attempt, "coding_role_run_0001")
            .expect("legacy capture is a no-op");
        assert!(baseline.member_checkouts.is_empty());
        let baseline_file = CodingAttemptStore::new(paths.clone())
            .attempt_cross_target_baselines_path(
                PROJECT_ID,
                ISSUE_ID,
                ATTEMPT_ID,
                "coding_role_run_0001",
            )
            .unwrap();
        assert!(
            !baseline_file.exists(),
            "legacy attempt must not persist a baseline"
        );
        detect_cross_target_violation(&paths, &attempt, "coding_role_run_0001")
            .expect("legacy detection is a no-op");
    }

    /// 构造一个带 worktree_path 的多仓 fixture，并交付给 `execute_review_request`
    /// 的交付门检测；worktree_path 指向成员主 checkout，仅用于通过入口检查。
    fn fixture_with_worktree() -> Fixture {
        let mut fixture = two_member_fixture();
        fixture.attempt.worktree_path = Some(fixture.checkout_paths[0].clone());
        fixture
    }

    fn delivery_engine(fixture: &Fixture) -> CodingWorkspaceEngine {
        let store = CodingAttemptStore::new(fixture.paths.clone());
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
        CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), event_tx)
    }

    /// 创建一次 provider role run 并为其采集越界基线（role_run.id 即 run_id），
    /// 返回 role_run.id。交付门按 role run 枚举并 detect 对应基线。
    fn capture_baseline_for_new_role_run(fixture: &Fixture, store: &CodingAttemptStore) -> String {
        let role_run = store
            .create_role_run(
                &fixture.attempt,
                CodingExecutionStage::Coding,
                CodingProviderRole::Coder,
                CodingRoleRunTrigger::Initial,
                None,
            )
            .expect("create role run");
        capture_cross_target_baseline(&fixture.paths, &fixture.attempt, &role_run.id).unwrap();
        role_run.id
    }

    #[tokio::test]
    async fn execute_review_request_blocks_when_another_member_checkout_is_dirty() {
        let fixture = fixture_with_worktree();
        let store = CodingAttemptStore::new(fixture.paths.clone());
        capture_baseline_for_new_role_run(&fixture, &store);

        // 模拟他仓主 checkout 被写文件（工作区变更）。
        fs::write(
            fixture.checkout_paths[1].join("trespass.txt"),
            "out of worktree write\n",
        )
        .expect("write into other member checkout");

        let engine = delivery_engine(&fixture);
        let result = engine
            .execute_review_request(&fixture.attempt, "origin", "feat: blocked")
            .await;

        match result {
            Err(CodingWorkspaceEngineError::CrossTargetDeliveryBlocked(code)) => {
                assert_eq!(code, "cross_target_violation_detected");
            }
            other => panic!("expected CrossTargetDeliveryBlocked, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn execute_review_request_blocks_when_baseline_file_missing() {
        let fixture = fixture_with_worktree();
        let store = CodingAttemptStore::new(fixture.paths.clone());
        let run_id = capture_baseline_for_new_role_run(&fixture, &store);
        fs::remove_file(baseline_path(&fixture, &run_id)).expect("simulate baseline loss");

        let engine = delivery_engine(&fixture);
        let result = engine
            .execute_review_request(&fixture.attempt, "origin", "feat: blocked")
            .await;

        match result {
            Err(CodingWorkspaceEngineError::CrossTargetDeliveryBlocked(code)) => {
                assert_eq!(code, "cross_target_baseline_missing");
            }
            other => panic!("expected CrossTargetDeliveryBlocked, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn execute_review_request_checks_every_role_run_baseline() {
        let fixture = fixture_with_worktree();
        let store = CodingAttemptStore::new(fixture.paths.clone());

        // 第一个 provider run 的基线在干净状态采集。
        capture_baseline_for_new_role_run(&fixture, &store);
        // 越界发生在第一个 run 之后：写入他仓主 checkout。
        fs::write(
            fixture.checkout_paths[1].join("trespass.txt"),
            "out of worktree write\n",
        )
        .expect("write into other member checkout");
        // 第二个 provider run 的基线在已越界状态采集（相对自身基线“干净”）。
        capture_baseline_for_new_role_run(&fixture, &store);

        let engine = delivery_engine(&fixture);
        let result = engine
            .execute_review_request(&fixture.attempt, "origin", "feat: blocked")
            .await;

        // 交付门必须按 role run 枚举全部基线：第一个 run 的基线已被越界污染，
        // 即使第二个 run 相对自身基线“干净”，也必须阻断。
        match result {
            Err(CodingWorkspaceEngineError::CrossTargetDeliveryBlocked(code)) => {
                assert_eq!(code, "cross_target_violation_detected");
            }
            other => panic!("expected CrossTargetDeliveryBlocked, got {other:?}"),
        }
    }

    /// R9：非 legacy 新 LC fixture——manifest/member/checkout 全部只落在
    /// `logical-codebases/{lc_id}/` 子树，issue 记录携带 lc 归属。
    fn new_lc_single_member_fixture() -> Fixture {
        let temp = tempfile::tempdir().expect("cross target temp");
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        crate::product::project_store::ProjectStore::new(paths.clone())
            .create(crate::product::project_store::CreateProjectInput {
                name: "cross target new lc".to_string(),
                description: None,
            })
            .unwrap();
        let project_id = PROJECT_ID.to_string();
        let record = LogicalCodebaseStore::new(paths.clone())
            .create(
                &project_id,
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "new-lc".to_string(),
                    aggregate_root: temp.path().join("aggregate-root"),
                },
            )
            .unwrap();
        let lc_id = record.id;

        let member_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        let checkout_path = temp.path().join("repo_a");
        init_repo(&checkout_path);

        let authority = LogicalCodebaseStore::for_lc(paths.clone(), lc_id.clone());
        authority
            .save_manifest(
                &project_id,
                &LogicalCodebaseManifest::new(
                    &project_id,
                    temp.path().join("aggregate-root"),
                    vec![member_id],
                ),
            )
            .unwrap();
        let now = "2026-08-18T00:00:00Z".to_string();
        let source_identity = RepositorySourceIdentity::from_git_parts(
            &checkout_path,
            checkout_path.join(".git"),
            None,
        );
        authority
            .save_member(
                &project_id,
                &CodebaseMemberRecord {
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_0001".to_string(),
                    alias: "repo_a".to_string(),
                    role: "repository".to_string(),
                    ordinal: 0,
                    source_identity: source_identity.clone(),
                    repo_type: RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_id],
                    status: MemberStatus::Active,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                },
            )
            .unwrap();
        authority
            .save_checkout(
                &project_id,
                &RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_0001".to_string(),
                    kind: CheckoutKind::Main,
                    canonical_path: checkout_path.clone(),
                    checkout_path_hash: "sha256:checkout".to_string(),
                    git_dir_identity: source_identity.git_dir_identity().to_string(),
                    revision: None,
                    availability: CheckoutAvailability::Available,
                    observed_at: now.clone(),
                    created_at: now.clone(),
                    updated_at: now,
                },
            )
            .unwrap();
        crate::product::logical_codebase::IdentityRegistryStore::new(paths.clone())
            .upsert_active(
                &project_id,
                crate::product::logical_codebase::IdentityRegistryEntry::active(
                    source_identity,
                    member_id,
                    "repository_0001".to_string(),
                    checkout_id,
                    "cross-target-new-lc-fixture".to_string(),
                ),
            )
            .unwrap();
        // issue 归属 lc（v1.3）。
        let issue = crate::product::issue_store::IssueStore::new(paths.clone())
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: project_id.clone(),
                repo_id: Some("repository_0001".to_string()),
                logical_codebase_id: Some(lc_id),
                title: "new lc issue".to_string(),
                description: None,
                change_id: None,
            })
            .unwrap();

        let mut attempt = logical_attempt(&project_id, &issue.id, ATTEMPT_ID, &checkout_path);
        let snapshot = attempt.target_snapshot.as_mut().unwrap();
        snapshot.logical_repository_id = member_id;
        snapshot.checkout_id = checkout_id;

        Fixture {
            _temp: temp,
            paths,
            attempt,
            checkout_paths: vec![checkout_path],
        }
    }

    #[test]
    fn capture_resolves_member_checkouts_from_new_lc_subtree() {
        let fixture = new_lc_single_member_fixture();
        let run_id = "coding_role_run_0001";

        let baseline = capture_cross_target_baseline(&fixture.paths, &fixture.attempt, run_id)
            .expect("baseline capture must resolve the lc subtree authority");

        assert_eq!(baseline.member_checkouts.len(), 1);
        assert_eq!(
            baseline.member_checkouts[0].canonical_path,
            fixture.checkout_paths[0]
        );
        assert!(baseline_path(&fixture, run_id).exists());
    }

    #[test]
    fn detect_across_new_lc_baseline_passes_until_member_checkout_changes() {
        let fixture = new_lc_single_member_fixture();
        let run_id = "coding_role_run_0001";
        capture_cross_target_baseline(&fixture.paths, &fixture.attempt, run_id).unwrap();
        detect_cross_target_violation(&fixture.paths, &fixture.attempt, run_id)
            .expect("unchanged member checkout must pass");

        fs::write(fixture.checkout_paths[0].join("README.md"), "changed\n").unwrap();
        assert_eq!(
            detect_cross_target_violation(&fixture.paths, &fixture.attempt, run_id).unwrap_err(),
            StableCode::CrossTargetViolationDetected
        );
    }
}
