use std::path::Path;
use std::process::Command;
use std::time::Duration;

use crate::cross_cutting::provider_registry::ProviderRegistry;
use crate::product::coding_models::{
    CodingAmendmentApplicationPhase, CodingAttemptStatus, CodingExecutionAttempt,
    CodingExecutionStage,
};
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    AmendmentResumeMode, AmendmentResumeTarget, ContractDeltaKind, PlanAmendmentManifest,
    PlanRepairRequestStatus, PlanRepairSessionStage, PlanRevisionReason,
    WorkItemRevisionReplacement,
};
use crate::web::coding_ws_handler::execute_start_coding_flow;
use crate::web::runtime::WebRuntime;
use crate::web::state::WebAppState;

use super::*;

#[derive(Debug, Clone, Copy)]
enum RunnerRecoveryState {
    Awaiting,
    Applying,
    Failed,
}

#[tokio::test]
async fn coding_ws_plan_repair_runner_recovers_amendment_before_provider_entry() {
    for state in [
        RunnerRecoveryState::Awaiting,
        RunnerRecoveryState::Applying,
        RunnerRecoveryState::Failed,
    ] {
        assert_runner_recovers_amendment_before_provider(state).await;
    }
}

async fn assert_runner_recovers_amendment_before_provider(state: RunnerRecoveryState) {
    let fixture = plan_repair_fixture();
    init_runner_test_git_repo(fixture.attempt.worktree_path.as_deref().unwrap());
    let (attempt, manifest) =
        prepare_runner_amendment(&fixture, state, AmendmentResumeMode::Reexecute).await;
    let provider = CountingProvider::default();
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::Codex, Arc::new(provider.clone()));
    let mut web_state = WebAppState::new(
        fixture._tmp.path().to_path_buf(),
        WebRuntime::new_fake(fixture._tmp.path().to_path_buf()),
    );
    web_state.provider_registry = Arc::new(registry);
    web_state.test_provider_enabled = true;
    let (event_tx, mut socket_event_rx) = mpsc::channel(64);
    let (observed_event_tx, mut event_rx) = mpsc::channel(64);
    tokio::spawn(async move {
        while let Some(event) = socket_event_rx.recv().await {
            crate::web::coding_ws_handler::delivery_ack::confirm_plan_amendment_socket_write(
                &event,
            );
            if observed_event_tx.send(event).await.is_err() {
                break;
            }
        }
    });
    let (_command_tx, command_rx) = mpsc::channel(8);
    let runner_state = web_state.clone();
    let runner_store = fixture.store.clone();
    let runner_attempt = attempt.clone();
    let runner_event_tx = event_tx.clone();
    let task = tokio::spawn(async move {
        let engine = CodingWorkspaceEngine::with_provider(
            runner_store.clone(),
            GitWorkspaceService::new(),
            runner_state.provider_adapter.clone(),
            runner_event_tx.clone(),
        );
        execute_start_coding_flow(
            &runner_state,
            &runner_store,
            &engine,
            &runner_event_tx,
            command_rx,
            &runner_attempt,
        )
        .await
    });
    drop(event_tx);

    let mut amendment_updates = 0;
    let mut reached_stage_gate = false;
    while !reached_stage_gate {
        let event = match tokio::time::timeout(Duration::from_secs(2), event_rx.recv()).await {
            Ok(Some(event)) => event,
            Ok(None) => panic!(
                "runner ended before amendment recovery for {state:?}: {:?}",
                task.await
            ),
            Err(_) => panic!("runner event timeout for {state:?}"),
        };
        match event {
            CodingWsOutMessage::PlanAmendmentUpdated { amendment, .. } => {
                assert_eq!(amendment.id, manifest.id, "{state:?}");
                amendment_updates += 1;
            }
            CodingWsOutMessage::CodingGateRequired { .. } => reached_stage_gate = true,
            _ => {}
        }
    }

    assert_eq!(amendment_updates, 1, "{state:?}");
    assert_eq!(provider.starts(), 0, "provider started for {state:?}");
    let recovered = fixture
        .store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    assert_eq!(recovered.status, CodingAttemptStatus::Running, "{state:?}");
    let journal = fixture
        .store
        .get_amendment_application_journal(&recovered, &manifest.id)
        .unwrap();
    assert_eq!(
        journal.phase,
        CodingAmendmentApplicationPhase::Completed,
        "{state:?}"
    );
    assert_eq!(journal.error, None, "{state:?}");
    let plan = fixture
        .revision_store
        .get_plan_lineage(&recovered.project_id, &recovered.issue_id, &fixture.plan.id)
        .unwrap();
    assert_eq!(plan.active_amendment_id, None, "{state:?}");

    task.abort();
    let cancelled = tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .unwrap_or_else(|_| panic!("runner abort timeout for {state:?}"))
        .unwrap_err();
    assert!(cancelled.is_cancelled(), "{state:?}: {cancelled}");
}

#[tokio::test]
async fn coding_ws_plan_repair_await_handoff_stays_blocked_after_stage_gate_continue() {
    let fixture = plan_repair_fixture();
    init_runner_test_git_repo(fixture.attempt.worktree_path.as_deref().unwrap());
    let (attempt, manifest) = prepare_runner_amendment(
        &fixture,
        RunnerRecoveryState::Awaiting,
        AmendmentResumeMode::AwaitHandoff,
    )
    .await;
    let provider = CountingProvider::default();
    let mut registry = ProviderRegistry::new();
    registry.register(ProviderName::Codex, Arc::new(provider.clone()));
    let mut web_state = WebAppState::new(
        fixture._tmp.path().to_path_buf(),
        WebRuntime::new_fake(fixture._tmp.path().to_path_buf()),
    );
    web_state.provider_registry = Arc::new(registry);
    web_state.test_provider_enabled = true;
    let (event_tx, mut socket_event_rx) = mpsc::channel(64);
    let (observed_event_tx, mut event_rx) = mpsc::channel(64);
    tokio::spawn(async move {
        while let Some(event) = socket_event_rx.recv().await {
            crate::web::coding_ws_handler::delivery_ack::confirm_plan_amendment_socket_write(
                &event,
            );
            if observed_event_tx.send(event).await.is_err() {
                break;
            }
        }
    });
    let (command_tx, command_rx) = mpsc::channel(8);
    command_tx
        .send(CodingRunnerCommand::StageGateConfirm {
            stage: CodingExecutionStage::Coding,
        })
        .await
        .unwrap();
    let engine = CodingWorkspaceEngine::with_provider(
        fixture.store.clone(),
        GitWorkspaceService::new(),
        web_state.provider_adapter.clone(),
        event_tx.clone(),
    );

    let error = execute_start_coding_flow(
        &web_state,
        &fixture.store,
        &engine,
        &event_tx,
        command_rx,
        &attempt,
    )
    .await
    .expect_err("AwaitHandoff must remain provider-blocked");
    drop(engine);
    drop(command_tx);
    drop(event_tx);

    assert!(
        error
            .to_string()
            .contains("plan_amendment_blocks_provider_run")
    );
    assert_eq!(provider.starts(), 0);
    let mut amendment_updates = 0;
    while let Some(event) = event_rx.recv().await {
        if let CodingWsOutMessage::PlanAmendmentUpdated { amendment, .. } = event {
            assert_eq!(amendment.id, manifest.id);
            amendment_updates += 1;
        }
    }
    assert_eq!(amendment_updates, 1);
    assert_eq!(
        fixture
            .store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .unwrap()
            .status,
        CodingAttemptStatus::AwaitingPlanAmendment
    );

    let (replay_event_tx, mut replay_event_rx) = mpsc::channel(16);
    tokio::spawn(async move {
        while let Some(event) = replay_event_rx.recv().await {
            crate::web::coding_ws_handler::delivery_ack::confirm_plan_amendment_socket_write(
                &event,
            );
        }
    });
    let (_replay_command_tx, replay_command_rx) = mpsc::channel(8);
    let replay_engine = CodingWorkspaceEngine::with_provider(
        fixture.store.clone(),
        GitWorkspaceService::new(),
        web_state.provider_adapter.clone(),
        replay_event_tx.clone(),
    );
    let replay_attempt = fixture
        .store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    let replay_error = execute_start_coding_flow(
        &web_state,
        &fixture.store,
        &replay_engine,
        &replay_event_tx,
        replay_command_rx,
        &replay_attempt,
    )
    .await
    .expect_err("AwaitHandoff replay must remain provider-blocked");

    assert!(
        replay_error
            .to_string()
            .contains("plan_amendment_blocks_provider_run"),
        "{replay_error}"
    );
    assert_eq!(provider.starts(), 0);
}

async fn prepare_runner_amendment(
    fixture: &PlanRepairFixture,
    state: RunnerRecoveryState,
    resume_mode: AmendmentResumeMode,
) -> (CodingExecutionAttempt, PlanAmendmentManifest) {
    let report = plan_defect_report(plan_defect_finding("runner_amendment_recovery"));
    let awaiting = fixture
        .engine
        .start_plan_repair_from_review(
            &fixture.attempt,
            &report.id,
            "code_review_report_0001_finding_0001",
            &report.findings[0],
            &fixture.projection,
        )
        .await
        .unwrap();
    let active_plan = fixture
        .revision_store
        .get_plan_lineage(&awaiting.project_id, &awaiting.issue_id, &fixture.plan.id)
        .unwrap();
    let request = fixture
        .revision_store
        .list_open_repair_requests(&active_plan)
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let amendment_id = request.amendment_id.clone().unwrap();
    let logical = fixture
        .revision_store
        .get_logical_work_item(&active_plan, "wi_current")
        .unwrap();
    let mut revised = fixture
        .revision_store
        .get_work_item_revision(&active_plan, &logical.id, "work_item_revision_current")
        .unwrap();
    let previous_revision_id = revised.id.clone();
    let previous_bundle = fixture
        .revision_store
        .get_work_item_projection_bundle(&active_plan, &revised.work_item_projection_bundle_id)
        .unwrap();
    let previous_verification = fixture
        .revision_store
        .get_verification_plan_revision(&active_plan, &revised.verification_plan_revision_id)
        .unwrap();
    let previous_bundle_id = previous_bundle.id.clone();
    revised.id = "work_item_revision_current_0002".to_string();
    revised.source_draft_revision_id = "draft_revision_current_0002".to_string();
    revised.work_item_projection_bundle_id = "projection_bundle_current_0002".to_string();
    revised.verification_plan_revision_id = "verification_revision_current_0002".to_string();
    revised.created_at = "2026-07-19T00:00:03Z".to_string();
    let mut revised_verification = previous_verification;
    revised_verification.id = revised.verification_plan_revision_id.clone();
    revised_verification.source_draft_revision_id = revised.source_draft_revision_id.clone();
    revised_verification.created_at = revised.created_at.clone();
    fixture
        .revision_store
        .put_verification_plan_revision(&active_plan, &revised_verification)
        .unwrap();
    fixture
        .revision_store
        .put_work_item_revision(&active_plan, &revised)
        .unwrap();
    let mut revised_bundle = previous_bundle;
    revised_bundle.id = revised.work_item_projection_bundle_id.clone();
    revised_bundle.work_item_revision_id = revised.id.clone();
    revised_bundle.coder_projection.work_item_revision_id = revised.id.clone();
    revised_bundle.reviewer_projection.work_item_revision_id = revised.id.clone();
    let revised_hashes = crate::product::work_item_projection::projection_hashes(
        &crate::product::work_item_projection::CompiledWorkItemProjections {
            human: revised_bundle.human_projection.clone(),
            coder: revised_bundle.coder_projection.clone(),
            reviewer: revised_bundle.reviewer_projection.clone(),
        },
    )
    .unwrap();
    revised_bundle.human_projection_hash = revised_hashes.human;
    revised_bundle.coder_projection_hash = revised_hashes.coder;
    revised_bundle.reviewer_projection_hash = revised_hashes.reviewer;
    revised_bundle.created_at = "2026-07-19T00:00:03Z".to_string();
    fixture
        .revision_store
        .put_work_item_projection_bundle(&active_plan, &revised_bundle)
        .unwrap();
    fixture
        .revision_store
        .set_active_work_item_revision(
            &active_plan,
            &logical,
            Some(&previous_revision_id),
            &revised.id,
        )
        .unwrap();

    let previous_plan = fixture
        .revision_store
        .get_plan_revision(
            &active_plan.project_id,
            &active_plan.issue_id,
            &active_plan.id,
            "plan_revision_0001",
        )
        .unwrap();
    let mut next_plan = previous_plan.clone();
    next_plan.id = "plan_revision_0002".to_string();
    next_plan.revision_no = 2;
    next_plan.supersedes = Some(previous_plan.id.clone());
    next_plan.reason = PlanRevisionReason::RepairCurrentWorkItem;
    next_plan
        .work_item_bindings
        .insert(logical.id.clone(), revised.id.clone());
    next_plan.validation_report_ref = "validation_report_0002".to_string();
    next_plan.plan_projection_bundle_id = "plan_projection_bundle_0002".to_string();
    next_plan.created_at = "2026-07-19T00:00:03Z".to_string();
    let mut next_plan_projection = fixture
        .revision_store
        .get_plan_projection_bundle(&active_plan, &previous_plan.plan_projection_bundle_id)
        .unwrap();
    next_plan_projection.id = next_plan.plan_projection_bundle_id.clone();
    next_plan_projection.plan_revision_id = next_plan.id.clone();
    next_plan_projection.work_item_projection_bundle_refs = next_plan_projection
        .work_item_projection_bundle_refs
        .iter()
        .map(|bundle_id| {
            if bundle_id == &previous_bundle_id {
                revised_bundle.id.clone()
            } else {
                bundle_id.clone()
            }
        })
        .collect();
    next_plan_projection.created_at = next_plan.created_at.clone();
    fixture
        .revision_store
        .put_plan_projection_bundle(&active_plan, &next_plan_projection)
        .unwrap();
    fixture
        .revision_store
        .put_plan_revision(&active_plan, &next_plan)
        .unwrap();
    let published_plan = fixture
        .revision_store
        .publish_active_plan_amendment_revision(
            &active_plan,
            &amendment_id,
            &previous_plan.id,
            &next_plan.id,
            "2026-07-19T00:00:04Z",
        )
        .unwrap();
    let manifest = PlanAmendmentManifest {
        id: amendment_id,
        repair_request_id: request.id.clone(),
        previous_plan_revision_id: previous_plan.id,
        new_plan_revision_id: next_plan.id,
        revised_work_items: BTreeMap::from([(
            logical.id.clone(),
            WorkItemRevisionReplacement {
                previous_revision_id,
                next_revision_id: revised.id,
                delta_kind: ContractDeltaKind::ImplementationGuidance,
            },
        )]),
        superseded_revisions: vec!["work_item_revision_current".to_string()],
        dependency_graph_changes: Vec::new(),
        contract_deltas: Vec::new(),
        unaffected_units: vec!["wi_upstream".to_string()],
        revalidation_required_units: Vec::new(),
        stale_units: Vec::new(),
        replacement_units: BTreeMap::new(),
        resume_target: AmendmentResumeTarget {
            logical_work_item_id: logical.id,
            mode: resume_mode,
        },
        created_at: "2026-07-19T00:00:04Z".to_string(),
    };
    fixture
        .revision_store
        .put_amendment_manifest(&published_plan, &manifest)
        .unwrap();
    let published_request = fixture
        .revision_store
        .update_repair_request_status(
            &published_plan,
            &request.id,
            PlanRepairRequestStatus::Published,
        )
        .unwrap();
    let lifecycle = LifecycleStore::new(fixture.store.paths());
    let link = lifecycle
        .list_session_links(&awaiting.project_id, &awaiting.issue_id)
        .unwrap()
        .into_iter()
        .find(|link| link.trigger.repair_request_id == request.id)
        .unwrap();
    let mut snapshot = lifecycle
        .load_plan_repair_session_state(
            &awaiting.project_id,
            &awaiting.issue_id,
            &link.child_session_id,
        )
        .unwrap()
        .unwrap();
    snapshot.request = published_request;
    snapshot.stage = PlanRepairSessionStage::Published;
    snapshot.amendment = Some(manifest.clone());
    lifecycle
        .save_plan_repair_session_state(
            &awaiting.project_id,
            &awaiting.issue_id,
            &link.child_session_id,
            &snapshot,
        )
        .unwrap();
    fixture
        .store
        .load_or_prepare_amendment_application(&awaiting, &manifest)
        .unwrap();

    if matches!(
        state,
        RunnerRecoveryState::Applying | RunnerRecoveryState::Failed
    ) {
        let applying = fixture
            .store
            .update_attempt_status(
                &awaiting.project_id,
                &awaiting.issue_id,
                &awaiting.id,
                CodingAttemptStatus::ApplyingPlanAmendment,
            )
            .unwrap();
        fixture
            .store
            .update_plan_binding_from_manifest(&applying, &manifest)
            .unwrap();
        fixture
            .store
            .advance_amendment_application_journal(
                &applying,
                &manifest.id,
                CodingAmendmentApplicationPhase::PlanBindingWritten,
                None,
                "2026-07-19T00:00:05Z".to_string(),
            )
            .unwrap();
    }
    if matches!(state, RunnerRecoveryState::Failed) {
        fixture
            .store
            .mark_amendment_application_failed(
                &awaiting,
                &manifest.id,
                "interrupted materialization".to_string(),
            )
            .unwrap();
        fixture
            .store
            .update_attempt_status(
                &awaiting.project_id,
                &awaiting.issue_id,
                &awaiting.id,
                CodingAttemptStatus::AmendmentApplyFailed,
            )
            .unwrap();
        snapshot.stage = PlanRepairSessionStage::AmendmentApplyFailed;
        snapshot.error = Some("interrupted materialization".to_string());
        lifecycle
            .save_plan_repair_session_state(
                &awaiting.project_id,
                &awaiting.issue_id,
                &link.child_session_id,
                &snapshot,
            )
            .unwrap();
    }
    let current = fixture
        .store
        .get_attempt(&awaiting.project_id, &awaiting.issue_id, &awaiting.id)
        .unwrap();
    (current, manifest)
}

fn init_runner_test_git_repo(path: &Path) {
    std::fs::create_dir_all(path).unwrap();
    run_git(path, &["init"]);
    run_git(path, &["config", "user.email", "aria@example.com"]);
    run_git(path, &["config", "user.name", "Aria Test"]);
    std::fs::write(path.join("README.md"), "initial\n").unwrap();
    run_git(path, &["add", "."]);
    run_git(path, &["commit", "-m", "initial"]);
}

fn run_git(path: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}
