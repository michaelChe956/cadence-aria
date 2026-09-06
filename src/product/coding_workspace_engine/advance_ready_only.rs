use super::*;

use crate::product::advance_store::{AdvanceInput, AdvanceRecord, AdvanceStatus, AdvanceStore};
use crate::product::coding_attempt_store::CreateGroupCodingAttemptInput;
use crate::product::coding_models::{CodingAdmissionKind, CodingAttemptStatus};
use crate::product::git_workspace_service::GitWorkspaceService;
use crate::product::models::ProviderName;
use crate::web::workspace_ws_types::ProviderConfigSnapshot;
use tempfile::tempdir;
use tokio::sync::mpsc;

fn group_attempt_fixture(root: &std::path::Path) -> (CodingAttemptStore, CodingExecutionAttempt) {
    let worktree = root.join("shared-worktree");
    std::fs::create_dir_all(&worktree).expect("worktree dir");
    super::init_test_git_repo(&worktree);
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.join(".aria")));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "HEAD".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: Default::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("group attempt");
    super::seed_group_attempt_fixture(&store, &attempt, true, false);
    (store, attempt)
}

fn ready_record(attempt_id: &str, status: AdvanceStatus) -> AdvanceRecord {
    AdvanceRecord {
        id: "advance_ready_only".to_string(),
        command_id: "command_ready_only".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        plan_id: "work_item_plan_0001".to_string(),
        plan_revision_id: "plan_revision_0001".to_string(),
        attempt_id: Some(attempt_id.to_string()),
        status,
        workspace_entry: Some("/tmp/ready-only-worktree".to_string()),
        error: None,
        created_at: "2026-08-31T00:00:00Z".to_string(),
        updated_at: "2026-08-31T00:00:01Z".to_string(),
    }
}

fn mark_sc_advance(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> CodingExecutionAttempt {
    let mut attempt = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("persisted group attempt");
    attempt.admission_kind = CodingAdmissionKind::ScAdvance;
    store
        .write_coding_attempt_for_test(&attempt)
        .expect("persist SC admission kind");
    attempt
}

#[tokio::test]
async fn advance_ready_response_does_not_start_coding_provider() {
    let root = tempdir().expect("root");
    let (store, original) = group_attempt_fixture(root.path());
    let attempt = mark_sc_advance(&store, &original);
    AdvanceStore::new(store.paths())
        .put_record(&ready_record(&attempt.id, AdvanceStatus::Ready))
        .expect("ready advance record");

    let (event_tx, mut event_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);
    let started = engine
        .start_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect("ready SC attempt starts");

    assert_eq!(started.status, CodingAttemptStatus::Running);
    assert_eq!(started.stage, CodingExecutionStage::Coding);
    assert!(matches!(
        event_rx.recv().await,
        Some(CodingWsOutMessage::CodingStageChange {
            stage: CodingExecutionStage::Coding
        })
    ));
    assert!(event_rx.try_recv().is_err());
}

#[tokio::test]
async fn sc_coding_start_without_advance_is_rejected() {
    let root = tempdir().expect("root");
    let (store, original) = group_attempt_fixture(root.path());
    let attempt = mark_sc_advance(&store, &original);
    let (event_tx, mut event_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);

    let error = engine
        .start_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect_err("SC attempt without durable advance must be rejected");
    assert!(
        error
            .to_string()
            .contains("must be Ready before StartCoding")
    );
    assert_eq!(
        store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("unchanged attempt")
            .status,
        CodingAttemptStatus::Created
    );
    assert!(event_rx.try_recv().is_err());
}

#[tokio::test]
async fn legacy_group_start_remains_unchanged() {
    let root = tempdir().expect("root");
    let (store, attempt) = group_attempt_fixture(root.path());
    assert_eq!(attempt.admission_kind, CodingAdmissionKind::LegacyGroup);
    let (event_tx, mut event_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);

    let started = engine
        .start_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect("legacy group starts without advance");
    assert_eq!(started.status, CodingAttemptStatus::Running);
    assert_eq!(started.stage, CodingExecutionStage::Coding);
    assert!(matches!(
        event_rx.recv().await,
        Some(CodingWsOutMessage::CodingStageChange {
            stage: CodingExecutionStage::Coding
        })
    ));
    assert!(
        AdvanceStore::new(store.paths())
            .get_advance_for_plan(
                &attempt.project_id,
                &attempt.issue_id,
                "work_item_plan_0001"
            )
            .expect("advance lookup")
            .is_none()
    );
}

#[tokio::test]
async fn sc_advance_unmaterialized_worktree_falls_back_to_worktree_prepare() {
    let root = tempdir().expect("root");
    let repo = root.path().join("repo");
    std::fs::create_dir_all(&repo).expect("repo dir");
    super::init_test_git_repo(&repo);
    let base_branch = super::git_stdout(&repo, &["branch", "--show-current"])
        .trim()
        .to_string();
    // 复现 sc_advance 延迟物化:worktree_path 已置位但目录不存在。
    let worktree_path = repo
        .join(".worktrees")
        .join("aria-issues")
        .join("issue_0001");
    assert!(
        !worktree_path.exists(),
        "fixture must keep worktree unmaterialized"
    );
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let attempt = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch,
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree_path.clone()),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: Default::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("group attempt");
    super::seed_group_attempt_fixture(&store, &attempt, true, false);
    let attempt = mark_sc_advance(&store, &attempt);
    AdvanceStore::new(store.paths())
        .put_record(&ready_record(&attempt.id, AdvanceStatus::Ready))
        .expect("ready advance record");

    let (event_tx, _event_rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);

    let started = engine
        .start_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .await
        .expect("unmaterialized SC advance must fall back to WorktreePrepare");
    assert_eq!(started.status, CodingAttemptStatus::Running);
    assert_eq!(started.stage, CodingExecutionStage::WorktreePrepare);
    assert!(
        !worktree_path.exists(),
        "start_attempt only stages WorktreePrepare; materialization belongs to execute_worktree_prepare"
    );

    let prepared = engine
        .execute_worktree_prepare(&started, &repo)
        .await
        .expect("materialize SC advance worktree");
    assert!(worktree_path.exists(), "worktree must be materialized");
    assert_eq!(
        prepared.worktree_path.as_deref(),
        Some(worktree_path.as_path())
    );

    let coding = engine
        .start_attempt(&prepared.project_id, &prepared.issue_id, &prepared.id)
        .await
        .expect("materialized SC advance starts coding");
    assert_eq!(coding.stage, CodingExecutionStage::Coding);
    assert!(
        coding.head_commit.is_some(),
        "head commit backfilled after materialization"
    );
}

#[test]
fn advance_returns_existing_status_for_each_attempt_state() {
    let root = tempdir().expect("root");
    let store = AdvanceStore::new(ProductAppPaths::new(root.path().join(".aria")));
    for (index, status) in [
        AdvanceStatus::Initializing,
        AdvanceStatus::Ready,
        AdvanceStatus::Running,
        AdvanceStatus::AwaitingPlanAmendment,
        AdvanceStatus::Completed,
        AdvanceStatus::Failed,
        AdvanceStatus::Aborted,
    ]
    .into_iter()
    .enumerate()
    {
        let mut record = ready_record("attempt_status_matrix", status);
        record.id = format!("advance_status_{index}");
        record.command_id = format!("command_status_{index}");
        record.plan_id = format!("plan_status_{index}");
        store.put_record(&record).expect("status record");
        let replay = store
            .persist_advance_record_if_absent(
                &AdvanceInput {
                    command_id: record.command_id.clone(),
                    project_id: record.project_id.clone(),
                    issue_id: record.issue_id.clone(),
                    plan_id: record.plan_id.clone(),
                },
                &record.plan_revision_id,
            )
            .expect("existing command lookup");
        assert_eq!(replay, record);
    }
}

#[test]
fn advance_failed_or_aborted_attempt_is_not_rebuilt() {
    let root = tempdir().expect("root");
    let store = AdvanceStore::new(ProductAppPaths::new(root.path().join(".aria")));
    for (index, status) in [AdvanceStatus::Failed, AdvanceStatus::Aborted]
        .into_iter()
        .enumerate()
    {
        let mut record = ready_record("attempt_terminal", status);
        record.id = format!("advance_terminal_{index}");
        record.command_id = format!("command_terminal_{index}");
        record.plan_id = format!("plan_terminal_{index}");
        store.put_record(&record).expect("terminal record");
        let replay = store
            .persist_advance_record_if_absent(
                &AdvanceInput {
                    command_id: record.command_id.clone(),
                    project_id: record.project_id.clone(),
                    issue_id: record.issue_id.clone(),
                    plan_id: record.plan_id.clone(),
                },
                "changed_revision",
            )
            .expect("terminal replay");
        assert_eq!(replay, record);
    }
}

/// 半启动恢复（3b）：group 半启动在 worktree 已物化时补 git head
/// （与 `start_attempt` group 短路同语义）。
#[tokio::test]
async fn prepare_resumed_attempt_backfills_group_head_from_materialized_worktree() {
    let root = tempdir().expect("root");
    let (store, attempt) = group_attempt_fixture(root.path());
    let mut semi_started = attempt.clone();
    semi_started.status = CodingAttemptStatus::Running;
    semi_started.stage = CodingExecutionStage::Coding;
    semi_started.head_commit = None;
    store
        .write_coding_attempt_for_test(&semi_started)
        .expect("persist semi-started group attempt");

    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);
    let prepared = engine
        .prepare_resumed_attempt_for_runner(&semi_started)
        .await
        .expect("prepare resumed group attempt");

    let worktree = attempt.worktree_path.as_deref().expect("worktree path");
    let expected_head = super::git_stdout(worktree, &["rev-parse", "HEAD"]);
    assert_eq!(prepared.stage, CodingExecutionStage::Coding);
    assert_eq!(
        prepared.head_commit.as_deref(),
        Some(expected_head.trim()),
        "materialized group worktree must have its git head backfilled before Coding"
    );
    assert!(prepared.worktree_path.is_some());
}

/// 半启动恢复（3b）：worktree 未物化（sc_advance 延迟物化/目录丢失）时回落
/// WorktreePrepare，由 runner 管道的 execute_worktree_prepare 重新物化。
#[tokio::test]
async fn prepare_resumed_attempt_demotes_unmaterialized_worktree() {
    let root = tempdir().expect("root");
    let (store, attempt) = group_attempt_fixture(root.path());
    let missing_worktree = root.path().join("missing-worktree");
    assert!(!missing_worktree.exists());
    let mut semi_started = attempt.clone();
    semi_started.status = CodingAttemptStatus::Running;
    semi_started.stage = CodingExecutionStage::Coding;
    semi_started.head_commit = None;
    semi_started.worktree_path = Some(missing_worktree.clone());
    store
        .write_coding_attempt_for_test(&semi_started)
        .expect("persist unmaterialized semi-started attempt");

    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);
    let prepared = engine
        .prepare_resumed_attempt_for_runner(&semi_started)
        .await
        .expect("demote unmaterialized semi-started attempt");

    assert_eq!(prepared.stage, CodingExecutionStage::WorktreePrepare);
    assert_eq!(prepared.status, CodingAttemptStatus::Running);
    assert!(
        !missing_worktree.exists(),
        "demotion only stages; materialization belongs to execute_worktree_prepare"
    );
}

/// 半启动恢复（3b）：head 已落盘或 stage 仍是 WorktreePrepare 时原样返回，
/// 不改写任何字段。
#[tokio::test]
async fn prepare_resumed_attempt_keeps_prepared_states_untouched() {
    let root = tempdir().expect("root");
    let (store, attempt) = group_attempt_fixture(root.path());

    let (event_tx, _event_rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), event_tx);

    // head 已落盘的 Coding attempt：原样返回。
    let mut with_head = attempt.clone();
    with_head.status = CodingAttemptStatus::Running;
    with_head.stage = CodingExecutionStage::Coding;
    with_head.head_commit = Some("deadbeef".to_string());
    let prepared = engine
        .prepare_resumed_attempt_for_runner(&with_head)
        .await
        .expect("prepared head-commit attempt");
    assert_eq!(prepared.stage, CodingExecutionStage::Coding);
    assert_eq!(prepared.head_commit.as_deref(), Some("deadbeef"));

    // stage 仍是 WorktreePrepare：原样返回，物化交给管道既有分支。
    let mut worktree_prepare = attempt.clone();
    worktree_prepare.status = CodingAttemptStatus::Running;
    worktree_prepare.stage = CodingExecutionStage::WorktreePrepare;
    let prepared = engine
        .prepare_resumed_attempt_for_runner(&worktree_prepare)
        .await
        .expect("worktree-prepare attempt");
    assert_eq!(prepared.stage, CodingExecutionStage::WorktreePrepare);
}
