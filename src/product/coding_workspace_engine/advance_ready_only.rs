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
