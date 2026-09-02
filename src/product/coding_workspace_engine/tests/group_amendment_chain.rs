use super::*;
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::coding_models::PlanAmendmentContext;
use crate::product::coding_models::{
    CodingExecutionStage, CodingExecutionUnitStatus, CodingUnitRun, CodingUnitRunStatus,
    PlanAmendmentContextStatus,
};
use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, WorkItemPlanSessionOptions};
use crate::product::models::{
    AmendmentResumeMode, AmendmentResumeTarget, ContractDeltaKind, HumanGateTurnStatus,
    PlanAmendmentManifest, PlanDefectClass, PlanDefectEvidence, PlanRepairRequest,
    PlanRepairRequestStatus, PlanRepairSessionStage, RepairTarget, RepairTargetKind,
    SingleCandidatePhase, WorkItemRevisionReplacement, WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::work_item_plan_policy::{HumanGateSnapshot, HumanReason, RunPolicy};
use crate::product::workspace_engine::{
    EngineEvent, HumanGateCommandOutcome, HumanGateFeedbackInput, WorkspaceEngine, WorkspaceSession,
};
use std::sync::Arc;
use tempfile::TempDir;

const TRIGGER_FINDING_ID: &str = "code_review_report_0001_finding_0001";
const TRIGGER_RUN_ID: &str = "coding_unit_run_blocked";

struct AmendmentChainFixture {
    _root: TempDir,
    store: CodingAttemptStore,
    lifecycle: LifecycleStore,
    attempt: CodingExecutionAttempt,
    trigger_unit_id: String,
    plan: WorkItemPlanLineage,
    manifest: PlanAmendmentManifest,
    plan_session_id: String,
    child_session_id: String,
    engine: CodingWorkspaceEngine,
    _event_rx: mpsc::Receiver<CodingWsOutMessage>,
}

fn seed_completed_unit_run(
    store: &CodingAttemptStore,
    plan: &WorkItemPlanLineage,
    attempt: &CodingExecutionAttempt,
    unit: &crate::product::coding_models::CodingExecutionUnit,
) {
    seed_unit_run_with_status(
        store,
        plan,
        attempt,
        unit,
        "coding_unit_run_completed",
        CodingUnitRunStatus::Completed,
    );
}

fn seed_trigger_unit_run(
    store: &CodingAttemptStore,
    plan: &WorkItemPlanLineage,
    attempt: &CodingExecutionAttempt,
    unit: &crate::product::coding_models::CodingExecutionUnit,
) {
    seed_unit_run_with_status(
        store,
        plan,
        attempt,
        unit,
        TRIGGER_RUN_ID,
        CodingUnitRunStatus::Running,
    );
}

fn seed_unit_run_with_status(
    store: &CodingAttemptStore,
    plan: &WorkItemPlanLineage,
    attempt: &CodingExecutionAttempt,
    unit: &crate::product::coding_models::CodingExecutionUnit,
    run_id: &str,
    status: CodingUnitRunStatus,
) {
    let revisions = WorkItemRevisionStore::new(store.paths());
    let revision = revisions
        .get_work_item_revision(
            plan,
            &unit.logical_work_item_id,
            &unit.work_item_revision_id,
        )
        .unwrap();
    let bundle = revisions
        .get_work_item_projection_bundle(plan, &revision.work_item_projection_bundle_id)
        .unwrap();
    store
        .create_coding_unit_run(
            attempt,
            &CodingUnitRun {
                id: run_id.to_string(),
                unit_id: unit.id.clone(),
                execution_no: 1,
                work_item_revision_id: revision.id,
                resolved_handoff_revision_ids: Vec::new(),
                canonical_contract_hash: bundle.canonical_contract_hash,
                projection_bundle_id: bundle.id,
                projection_compiler_version: bundle.compiler_version,
                coder_provider_renderer_version: "coder-v1".to_string(),
                reviewer_provider_renderer_version: "reviewer-v1".to_string(),
                internal_reviewer_provider_renderer_version: None,
                coder_projection_hash: bundle.coder_projection_hash,
                reviewer_projection_hash: bundle.reviewer_projection_hash,
                coder_execution_context_hash: None,
                reviewer_execution_context_hash: None,
                internal_reviewer_execution_context_hash: None,
                status,
                unit_rework_count: 0,
                verification_retry_count: 0,
                operational_retry_count: 0,
                plan_repair_count: 0,
                start_commit: None,
                completion_commit: None,
                created_at: "2026-08-31T00:00:00Z".to_string(),
                updated_at: "2026-08-31T00:00:00Z".to_string(),
            },
        )
        .unwrap();
}

async fn amendment_chain_fixture(resume_mode: AmendmentResumeMode) -> AmendmentChainFixture {
    let root = TempDir::new().unwrap();
    let worktree = root.path().join("worktree");
    std::fs::create_dir_all(&worktree).unwrap();
    init_test_git_repo(&worktree);
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let store = CodingAttemptStore::new(paths.clone());
    let initial = store
        .create_group_attempt(CreateGroupCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "work_item_plan_0001".to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: Some(worktree),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .unwrap();
    seed_group_attempt_fixture(&store, &initial, true, false);
    let revision_store = WorkItemRevisionStore::new(paths.clone());
    let plan = revision_store
        .get_plan_lineage("project_0001", "issue_0001", "work_item_plan_0001")
        .unwrap();

    let lifecycle = LifecycleStore::new(paths.clone());
    // 原 SC plan session：已通过首次人工门（Confirmed/Completed），门快照与
    // manual_repairs_remaining 保留在 session record 上（D11 单一预算源）。
    let mut parent = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: plan.id.clone(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: true,
            openspec_enabled: true,
            work_item_plan_options: Some(WorkItemPlanSessionOptions {
                flow_kind:
                    crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate,
                run_policy: RunPolicy::Interactive,
                rollout_snapshot: true,
            }),
        })
        .unwrap();
    parent.status = WorkspaceSessionStatus::Confirmed;
    parent.single_candidate_phase = Some(SingleCandidatePhase::Completed);
    parent.human_gate_snapshot = Some(HumanGateSnapshot {
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 2,
        trigger: HumanReason::NativeHumanRequired,
        resumable: true,
    });
    crate::product::json_store::write_json(
        &paths
            .issue_lifecycle_root("project_0001", "issue_0001")
            .join("workspace-sessions")
            .join(format!("{}.json", parent.id)),
        &parent,
    )
    .unwrap();

    let mut attempt = store
        .seed_running_attempt_for_test(&initial.project_id, &initial.issue_id, &initial.id)
        .unwrap();
    let units = store
        .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .unwrap();
    seed_completed_unit_run(&store, &plan, &attempt, &units[0]);
    store
        .update_coding_unit_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &units[0].id,
            CodingExecutionUnitStatus::Completed,
            Some("completed before amendment".to_string()),
        )
        .unwrap();
    seed_trigger_unit_run(&store, &plan, &attempt, &units[1]);
    store
        .update_coding_unit_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &units[1].id,
            CodingExecutionUnitStatus::Running,
            None,
        )
        .unwrap();
    attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .unwrap();

    let request = PlanRepairRequest {
        id: "plan_repair_request_0001".to_string(),
        plan_id: plan.id.clone(),
        base_plan_revision_id: "plan_revision_0001".to_string(),
        trigger_attempt_id: attempt.id.clone(),
        trigger_unit_run_id: TRIGGER_RUN_ID.to_string(),
        trigger_review_id: Some("code_review_report_0001".to_string()),
        trigger_finding_id: TRIGGER_FINDING_ID.to_string(),
        amendment_id: None,
        defect_class: PlanDefectClass::UpstreamContractInvalid,
        reason_code: "upstream_contract_invalid".to_string(),
        repair_target: RepairTarget {
            kind: RepairTargetKind::UpstreamWorkItem,
            logical_work_item_ids: vec!["work_item_0001".to_string()],
            work_item_revision_ids: vec!["work_item_revision_0001".to_string()],
        },
        contract_refs: vec!["contract_work_item_0001".to_string()],
        capability_refs: vec!["capability_work_item_0001".to_string()],
        evidence: vec![PlanDefectEvidence {
            kind: "review".to_string(),
            source_ref: "code_review_report_0001".to_string(),
            message: "upstream contract needs repair".to_string(),
        }],
        fingerprint: "plan_repair_fingerprint_0001".to_string(),
        status: PlanRepairRequestStatus::Open,
        created_at: "2026-08-31T00:00:00Z".to_string(),
        updated_at: "2026-08-31T00:00:00Z".to_string(),
    };
    let (workspace_tx, _workspace_rx) = mpsc::channel::<EngineEvent>(8);
    let mut workspace_engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(root.path().join("checkpoints"))),
        lifecycle.clone(),
        workspace_tx,
        WorkspaceSession::from_record(parent.clone()),
    );
    let child = workspace_engine.start_plan_repair(request).await.unwrap();
    let reconciliation = store.reconcile_linked_plan_repair_pause(&attempt).unwrap();
    attempt = reconciliation.attempt;
    let plan_session_id = parent.id.clone();
    let trigger_unit_id = units[1].id.clone();

    let manifest = publish_amendment_manifest(
        &store,
        &revision_store,
        &lifecycle,
        &plan,
        &paths,
        &child.id,
        resume_mode,
    );

    let (event_tx, mut socket_event_rx) = mpsc::channel(16);
    let (observed_event_tx, event_rx) = mpsc::channel(16);
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
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx);

    AmendmentChainFixture {
        _root: root,
        store,
        lifecycle,
        attempt,
        trigger_unit_id,
        plan,
        manifest,
        plan_session_id,
        child_session_id: child.id,
        engine,
        _event_rx: event_rx,
    }
}

fn publish_amendment_manifest(
    store: &CodingAttemptStore,
    revision_store: &WorkItemRevisionStore,
    lifecycle: &LifecycleStore,
    plan: &WorkItemPlanLineage,
    paths: &ProductAppPaths,
    child_session_id: &str,
    resume_mode: AmendmentResumeMode,
) -> PlanAmendmentManifest {
    let attempt = store
        .get_attempt_for_work_item_group("project_0001", "issue_0001", &plan.id)
        .unwrap()
        .expect("group attempt");
    let old_revision = revision_store
        .get_work_item_revision(plan, "work_item_0001", "work_item_revision_0001")
        .unwrap();
    let old_bundle = revision_store
        .get_work_item_projection_bundle(plan, &old_revision.work_item_projection_bundle_id)
        .unwrap();
    let mut revised = old_revision.clone();
    revised.id = "work_item_revision_0101".to_string();
    revised.source_draft_revision_id = "draft_revision_0101".to_string();
    revised.work_item_projection_bundle_id = "projection_bundle_0101".to_string();
    revised.created_at = "2026-08-31T00:00:01Z".to_string();
    revision_store
        .put_work_item_revision(plan, &revised)
        .unwrap();
    WorkItemPlanStore::new(paths.clone())
        .put_draft_record(&WorkItemDraftRecord {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: plan.id.clone(),
            draft_id: revised.source_draft_revision_id.clone(),
            outline_id: "outline_0101".to_string(),
            generation_round_id: "round_0001".to_string(),
            batch_id: None,
            attempt_index: 1,
            outline_version_ref: "outline_version_0001".to_string(),
            generation_mode: WorkItemGenerationMode::Serial,
            generation_diagnostics: None,
            candidate: WorkItemDraftCandidate {
                target_repository_id: None,
                outline_id: "outline_0101".to_string(),
                logical_work_item_id: revised.logical_work_item_id.clone(),
                canonical_contract_candidate: revised.canonical_contract.clone(),
                verification_plan: WorkItemDraftVerificationPlan {
                    checks: revised.canonical_contract.verification_checks.clone(),
                },
            },
            status: WorkItemDraftStatus::Accepted,
            active: true,
            superseded_by_draft_id: None,
            supersede_reason: None,
            copied_from_draft_id: None,
            review_node_id: None,
            review_verdict_ref: None,
            generated_from_node_id: "timeline_node_0001".to_string(),
            accepted_at: Some("2026-08-31T00:00:01Z".to_string()),
            superseded_at: None,
            created_at: "2026-08-31T00:00:01Z".to_string(),
            updated_at: "2026-08-31T00:00:01Z".to_string(),
        })
        .unwrap();
    let mut revised_bundle = old_bundle;
    revised_bundle.id = revised.work_item_projection_bundle_id.clone();
    revised_bundle.work_item_revision_id = revised.id.clone();
    revised_bundle.coder_projection.work_item_revision_id = revised.id.clone();
    revised_bundle.reviewer_projection.work_item_revision_id = revised.id.clone();
    let revised_hashes = projection_hashes(
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
    revised_bundle.created_at = "2026-08-31T00:00:01Z".to_string();
    revision_store
        .put_work_item_projection_bundle(plan, &revised_bundle)
        .unwrap();
    let logical = revision_store
        .get_logical_work_item(plan, "work_item_0001")
        .unwrap();
    revision_store
        .set_active_work_item_revision(plan, &logical, Some("work_item_revision_0001"), &revised.id)
        .unwrap();
    let previous_plan = revision_store
        .get_plan_revision(
            &plan.project_id,
            &plan.issue_id,
            &plan.id,
            "plan_revision_0001",
        )
        .unwrap();
    let mut next_bindings = previous_plan.work_item_bindings.clone();
    next_bindings.insert("work_item_0001".to_string(), revised.id.clone());
    let next_plan = WorkItemPlanRevision {
        id: "plan_revision_0002".to_string(),
        plan_id: plan.id.clone(),
        revision_no: 2,
        supersedes: Some(previous_plan.id.clone()),
        reason: PlanRevisionReason::RepairUpstreamContract,
        work_item_bindings: next_bindings,
        dependency_graph_revision_id: previous_plan.dependency_graph_revision_id,
        validation_report_ref: "validation_report_0002".to_string(),
        plan_projection_bundle_id: "plan_projection_bundle_0002".to_string(),
        publication_provenance_ref: None,
        created_at: "2026-08-31T00:00:01Z".to_string(),
    };
    let previous_plan_projection = revision_store
        .get_plan_projection_bundle(plan, &previous_plan.plan_projection_bundle_id)
        .unwrap();
    let mut next_plan_projection = previous_plan_projection;
    next_plan_projection.id = next_plan.plan_projection_bundle_id.clone();
    next_plan_projection.plan_revision_id = next_plan.id.clone();
    next_plan_projection.dependency_graph_revision_id =
        next_plan.dependency_graph_revision_id.clone();
    next_plan_projection.work_item_projection_bundle_refs = next_plan_projection
        .work_item_projection_bundle_refs
        .iter()
        .map(|bundle_id| {
            if bundle_id == &old_revision.work_item_projection_bundle_id {
                revised.work_item_projection_bundle_id.clone()
            } else {
                bundle_id.clone()
            }
        })
        .collect();
    next_plan_projection.created_at = "2026-08-31T00:00:01Z".to_string();
    revision_store
        .put_plan_projection_bundle(plan, &next_plan_projection)
        .unwrap();
    revision_store.put_plan_revision(plan, &next_plan).unwrap();
    let published_request = revision_store
        .get_repair_request(plan, "plan_repair_request_0001")
        .unwrap();
    let amendment_id = published_request.amendment_id.clone().unwrap();
    revision_store
        .publish_active_plan_amendment_revision(
            plan,
            &amendment_id,
            "plan_revision_0001",
            "plan_revision_0002",
            "2026-08-31T00:00:02Z",
        )
        .unwrap();
    let manifest = PlanAmendmentManifest {
        id: amendment_id,
        repair_request_id: published_request.id.clone(),
        previous_plan_revision_id: "plan_revision_0001".to_string(),
        new_plan_revision_id: "plan_revision_0002".to_string(),
        revised_work_items: std::collections::BTreeMap::from([(
            "work_item_0001".to_string(),
            WorkItemRevisionReplacement {
                previous_revision_id: "work_item_revision_0001".to_string(),
                next_revision_id: revised.id,
                delta_kind: ContractDeltaKind::ImplementationGuidance,
            },
        )]),
        superseded_revisions: vec!["work_item_revision_0001".to_string()],
        dependency_graph_changes: Vec::new(),
        contract_deltas: Vec::new(),
        unaffected_units: vec!["work_item_0003".to_string()],
        revalidation_required_units: vec!["work_item_0002".to_string()],
        stale_units: Vec::new(),
        replacement_units: std::collections::BTreeMap::new(),
        resume_target: AmendmentResumeTarget {
            logical_work_item_id: "work_item_0001".to_string(),
            mode: resume_mode,
        },
        created_at: "2026-08-31T00:00:02Z".to_string(),
    };
    revision_store
        .put_amendment_manifest(plan, &manifest)
        .unwrap();
    let published_request = revision_store
        .update_repair_request_status(
            plan,
            &manifest.repair_request_id,
            PlanRepairRequestStatus::Published,
        )
        .unwrap();
    let mut snapshot = lifecycle
        .load_plan_repair_session_state(&attempt.project_id, &attempt.issue_id, child_session_id)
        .unwrap()
        .unwrap();
    snapshot.request = published_request;
    snapshot.stage = PlanRepairSessionStage::Published;
    snapshot.amendment = Some(manifest.clone());
    lifecycle
        .save_plan_repair_session_state(
            &attempt.project_id,
            &attempt.issue_id,
            child_session_id,
            &snapshot,
        )
        .unwrap();
    lifecycle
        .update_workspace_session_status(child_session_id, WorkspaceSessionStatus::WaitingForHuman)
        .unwrap();
    manifest
}

fn plan_session_engine(fixture: &AmendmentChainFixture) -> WorkspaceEngine {
    let record = fixture
        .lifecycle
        .get_workspace_session(&fixture.plan_session_id)
        .unwrap();
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut session = WorkspaceSession::from_record(record);
    session.artifact = Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
        markdown: "# Work Item Plan\n\n## Work Item WI-001: candidate\n".to_string(),
        diff: None,
    });
    WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(
            fixture._root.path().join("checkpoints"),
        )),
        fixture.lifecycle.clone(),
        event_tx,
        session,
    )
}

fn trigger_context(fixture: &AmendmentChainFixture) -> PlanAmendmentContext {
    fixture
        .store
        .find_plan_amendment_context_by_finding(&fixture.attempt, TRIGGER_FINDING_ID)
        .unwrap()
        .expect("plan amendment context must exist after reconcile")
}

#[tokio::test]
async fn group_amendment_plan_defect_stays_on_same_attempt() {
    let fixture = amendment_chain_fixture(AmendmentResumeMode::Reexecute).await;

    let persisted = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    assert_eq!(
        persisted.id, fixture.attempt.id,
        "defect stays on the same attempt"
    );
    assert_eq!(persisted.status, CodingAttemptStatus::AwaitingPlanAmendment);
    let trigger_run = fixture
        .store
        .list_coding_unit_runs(&persisted, &fixture.trigger_unit_id)
        .unwrap()
        .into_iter()
        .find(|run| run.id == TRIGGER_RUN_ID)
        .expect("trigger run");
    assert_eq!(trigger_run.status, CodingUnitRunStatus::BlockedByPlanDefect);

    let context = trigger_context(&fixture);
    assert_eq!(context.plan_session_id, fixture.plan_session_id);
    assert_eq!(context.group_attempt_id, persisted.id);
    assert_eq!(context.trigger_unit_id, fixture.trigger_unit_id);
    assert_eq!(context.trigger_finding_id, TRIGGER_FINDING_ID);
    assert_eq!(context.previous_plan_revision_id, "plan_revision_0001");
    assert_eq!(context.new_plan_revision_id, None);
    assert_eq!(
        context.resume_target,
        AmendmentResumeTarget {
            logical_work_item_id: "work_item_0002".to_string(),
            mode: AmendmentResumeMode::AwaitHandoff,
        }
    );
    assert_eq!(context.status, PlanAmendmentContextStatus::Open);

    // 重复 defect/finding 事件：reconcile 幂等，返回原 context，不重复开门。
    let reconciliation = fixture
        .store
        .reconcile_linked_plan_repair_pause(&fixture.attempt)
        .unwrap();
    assert_eq!(reconciliation.attempt.id, persisted.id);
    assert_eq!(
        reconciliation.attempt.status,
        CodingAttemptStatus::AwaitingPlanAmendment
    );
    let contexts = fixture
        .store
        .list_plan_amendment_contexts(&persisted)
        .unwrap();
    assert_eq!(contexts.len(), 1, "no second context may be opened");
    assert_eq!(contexts[0], context);
}

#[tokio::test]
async fn group_amendment_feedback_reuses_original_plan_session_budget() {
    let fixture = amendment_chain_fixture(AmendmentResumeMode::Reexecute).await;
    let mut engine = plan_session_engine(&fixture);
    let attempt_before = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    let sessions_before = fixture
        .lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .unwrap()
        .into_iter()
        .filter(|session| session.entity_id == fixture.plan.id)
        .count();

    let opened = engine
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: "cmd_amendment_feedback_1".to_string(),
            feedback: "只修正 WI-001 的 Outputs，其余逐字保留".to_string(),
        })
        .await
        .expect("amendment feedback must reopen the original plan session gate");

    let (turn, remaining_budget) = match opened {
        HumanGateCommandOutcome::TurnOpened {
            turn,
            remaining_budget,
            ..
        } => (turn, remaining_budget),
        other => panic!("expected amendment turn opened, got {other:?}"),
    };
    assert_eq!(turn.session_id, fixture.plan_session_id);
    assert_eq!(remaining_budget, 1);

    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.plan_session_id)
        .unwrap();
    assert_eq!(durable.status, WorkspaceSessionStatus::WaitingForHuman);
    let snapshot = durable.human_gate_snapshot.as_ref().expect("gate snapshot");
    assert_eq!(
        snapshot.manual_repairs_remaining, 1,
        "budget decremented once on the original session"
    );
    assert_eq!(durable.provider_start_ledger.len(), 1);
    let turns = fixture
        .lifecycle
        .list_human_gate_turns(&fixture.plan_session_id)
        .unwrap();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].turn_id, turn.turn_id);

    // group attempt 侧不产生新预算账：attempt 状态不变、无新门、无第三个 session。
    let attempt_after = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    assert_eq!(attempt_after, attempt_before);
    assert!(
        fixture
            .store
            .list_open_blocked_gates(
                &attempt_after.project_id,
                &attempt_after.issue_id,
                &attempt_after.id
            )
            .unwrap()
            .is_empty()
    );
    let sessions_after = fixture
        .lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .unwrap()
        .into_iter()
        .filter(|session| session.entity_id == fixture.plan.id)
        .count();
    assert_eq!(
        sessions_after, sessions_before,
        "no second human gate instance"
    );

    // 同 command_id 重放：同 turn，预算与 ledger 不再变化。
    let replayed = engine
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: "cmd_amendment_feedback_1".to_string(),
            feedback: "只修正 WI-001 的 Outputs，其余逐字保留".to_string(),
        })
        .await
        .expect("replay");
    match replayed {
        HumanGateCommandOutcome::Replayed { turn: replay_turn } => {
            assert_eq!(replay_turn.turn_id, turn.turn_id);
        }
        other => panic!("expected replay, got {other:?}"),
    }
    let durable_after_replay = fixture
        .lifecycle
        .get_workspace_session(&fixture.plan_session_id)
        .unwrap();
    assert_eq!(durable_after_replay, durable);
}

#[tokio::test]
async fn group_amendment_approve_updates_binding_and_resumes_target() {
    for (mode, expected_status, expected_stage, expected_unit_status) in [
        (
            AmendmentResumeMode::Reexecute,
            CodingAttemptStatus::Running,
            CodingExecutionStage::Coding,
            CodingExecutionUnitStatus::Running,
        ),
        (
            AmendmentResumeMode::Revalidate,
            CodingAttemptStatus::Running,
            CodingExecutionStage::CodeReview,
            CodingExecutionUnitStatus::NeedsRevalidation,
        ),
        (
            AmendmentResumeMode::AwaitHandoff,
            CodingAttemptStatus::AwaitingPlanAmendment,
            CodingExecutionStage::Coding,
            CodingExecutionUnitStatus::AwaitingAmendment,
        ),
    ] {
        let fixture = amendment_chain_fixture(mode.clone()).await;
        let context = trigger_context(&fixture);

        let resumed = fixture
            .engine
            .resume_group_after_plan_amendment(&fixture.attempt, &context, &fixture.manifest)
            .await
            .unwrap_or_else(|error| panic!("resume {mode:?}: {error}"));

        assert_eq!(
            resumed.id, fixture.attempt.id,
            "resume keeps the same attempt ({mode:?})"
        );
        assert_eq!(resumed.status, expected_status, "{mode:?}");
        assert_eq!(resumed.stage, expected_stage, "{mode:?}");
        let binding = fixture.store.get_plan_binding(&resumed).unwrap();
        assert_eq!(
            binding.bound_plan_revision_id, "plan_revision_0002",
            "{mode:?}"
        );
        assert_eq!(
            binding.applied_amendment_ids,
            vec![fixture.manifest.id.clone()],
            "{mode:?}"
        );

        let updated_context = trigger_context(&fixture);
        assert_eq!(updated_context.id, context.id);
        assert_eq!(
            updated_context.status,
            PlanAmendmentContextStatus::Applied,
            "{mode:?}"
        );
        assert_eq!(
            updated_context.new_plan_revision_id.as_deref(),
            Some("plan_revision_0002"),
            "{mode:?}"
        );
        assert_eq!(
            updated_context.resume_target, fixture.manifest.resume_target,
            "{mode:?}"
        );
        assert_eq!(
            updated_context.previous_plan_revision_id, context.previous_plan_revision_id,
            "{mode:?}"
        );

        let resume_unit = fixture
            .store
            .list_coding_units(&resumed.project_id, &resumed.issue_id, &resumed.id)
            .unwrap()
            .into_iter()
            .find(|unit| unit.logical_work_item_id == "work_item_0001")
            .expect("resume target unit");
        assert_eq!(resume_unit.status, expected_unit_status, "{mode:?}");

        // 原 plan session 的重开门在 amendment 应用后关回 Confirmed。
        let durable = fixture
            .lifecycle
            .get_workspace_session(&fixture.plan_session_id)
            .unwrap();
        assert_eq!(
            durable.status,
            WorkspaceSessionStatus::Confirmed,
            "{mode:?}"
        );
    }
}

#[tokio::test]
async fn group_amendment_incompatible_revision_fails_closed() {
    let fixture = amendment_chain_fixture(AmendmentResumeMode::Reexecute).await;
    let context = trigger_context(&fixture);
    let binding_before = fixture.store.get_plan_binding(&fixture.attempt).unwrap();
    let mut forged = fixture.manifest.clone();
    forged.repair_request_id = "plan_repair_request_forged".to_string();

    let error = fixture
        .engine
        .resume_group_after_plan_amendment(&fixture.attempt, &context, &forged)
        .await
        .expect_err("forged amendment identity must fail closed");
    assert!(
        error.to_string().contains("identity_mismatch"),
        "unexpected error: {error}"
    );

    let persisted = fixture
        .store
        .get_attempt(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .unwrap();
    assert_eq!(persisted.id, fixture.attempt.id);
    assert_eq!(persisted.status, CodingAttemptStatus::AwaitingPlanAmendment);
    assert_eq!(
        fixture.store.get_plan_binding(&persisted).unwrap(),
        binding_before
    );

    let failed = trigger_context(&fixture);
    assert_eq!(failed.id, context.id);
    assert_eq!(failed.status, PlanAmendmentContextStatus::FailedClosed);
    assert_eq!(failed.new_plan_revision_id, None);
    assert_eq!(
        failed.previous_plan_revision_id,
        context.previous_plan_revision_id
    );
    let diagnostic = fixture
        .store
        .get_plan_amendment_context_diagnostic(&persisted, &failed.id)
        .unwrap()
        .expect("durable fail-closed diagnostic");
    assert!(diagnostic.reason.contains("identity_mismatch"));

    // FailedClosed 之后重试同一不兼容 revision：仍拒绝，要求人工处置。
    let retry = fixture
        .engine
        .resume_group_after_plan_amendment(&persisted, &failed, &forged)
        .await
        .expect_err("failed-closed context must keep blocking");
    assert!(retry.to_string().contains("failed_closed"));
}

#[tokio::test]
async fn group_amendment_reconnect_recovers_original_gate_and_context() {
    let fixture = amendment_chain_fixture(AmendmentResumeMode::Reexecute).await;
    let context = trigger_context(&fixture);

    // 在原 plan session 上开一个 in-flight amendment 反馈 turn 后断线。
    let mut engine = plan_session_engine(&fixture);
    let opened = engine
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: "cmd_amendment_reconnect".to_string(),
            feedback: "只修正 WI-001 的 Outputs，其余逐字保留".to_string(),
        })
        .await
        .expect("amendment turn");
    let turn_id = match opened {
        HumanGateCommandOutcome::TurnOpened { turn, .. } => turn.turn_id,
        other => panic!("expected amendment turn opened, got {other:?}"),
    };
    drop(engine);

    // 断线恢复：coding 侧 recover 返回同一 attempt 与 history/child session 引用。
    let (recovered, child_session_id) = fixture
        .engine
        .recover_plan_amendment_with_history_session(&fixture.attempt)
        .await
        .expect("recover amendment after reconnect");
    assert_eq!(recovered.id, fixture.attempt.id);
    assert_eq!(child_session_id, fixture.child_session_id);
    assert_eq!(recovered.status, CodingAttemptStatus::Running);

    // 原 plan session 负责恢复 amendment 上下文与 in-flight turn。
    let recovered_context = fixture
        .store
        .find_plan_amendment_context_by_finding(&recovered, TRIGGER_FINDING_ID)
        .unwrap()
        .expect("context survives reconnect");
    assert_eq!(recovered_context.id, context.id);
    assert_eq!(recovered_context.plan_session_id, fixture.plan_session_id);
    assert_eq!(
        recovered_context.status,
        PlanAmendmentContextStatus::Applied
    );
    assert_eq!(
        recovered_context.new_plan_revision_id.as_deref(),
        Some("plan_revision_0002")
    );

    let durable = fixture
        .lifecycle
        .get_workspace_session(&fixture.plan_session_id)
        .unwrap();
    assert_eq!(durable.status, WorkspaceSessionStatus::WaitingForHuman);
    let turns = fixture
        .lifecycle
        .list_human_gate_turns(&fixture.plan_session_id)
        .unwrap();
    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].turn_id, turn_id);
    assert_eq!(turns[0].status, HumanGateTurnStatus::Reserved);
    let snapshot = durable.human_gate_snapshot.as_ref().expect("gate snapshot");
    assert_eq!(snapshot.manual_repairs_remaining, 1);

    // 重连后的新引擎：恢复 in-flight turn，重发同命令得到同一 turn。
    let mut reconnected = plan_session_engine(&fixture);
    let actions = reconnected.recover_human_gate_turns(false).unwrap();
    assert_eq!(actions.len(), 1);
    assert_eq!(actions[0].0, turn_id);
    let replayed = reconnected
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: "cmd_amendment_reconnect".to_string(),
            feedback: "只修正 WI-001 的 Outputs，其余逐字保留".to_string(),
        })
        .await
        .expect("replay after reconnect");
    match replayed {
        HumanGateCommandOutcome::Replayed { turn } => assert_eq!(turn.turn_id, turn_id),
        other => panic!("expected replay after reconnect, got {other:?}"),
    }
}
