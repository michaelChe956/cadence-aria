use std::fs;
use std::path::PathBuf;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    PlanAmendmentPublicationPhase, PlanRepairAwaitingConfirmationPackage, PlanRepairRequestStatus,
    PlanRepairSessionSnapshotDto, PlanRepairSessionStage, WorkItemPlanLineage,
    WorkspaceSessionStatus,
};
use crate::product::plan_repair::PlanRepairError;
use crate::product::work_item_revision_store::{
    ActiveAmendmentReleaseOutcome, WorkItemRevisionStore,
};
use crate::web::workspace_ws_types::{
    ProviderConfigSnapshot, TimelineNode, TimelineNodeStatus, TimelineNodeType,
    WorkspaceStage as WsWorkspaceStage,
};

use super::{PlanRepairCrashPoint, WorkspaceEngine, WorkspaceStage};

const PLAN_REPAIR_TRANSITION_JOURNAL_FILE: &str = "plan_repair_transition_journal.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PlanRepairTransitionOperation {
    AwaitingConfirmation,
    Confirm,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PlanRepairTransitionPhase {
    Prepared,
    TimelinePersisted,
    SnapshotPersisted,
    SessionPersisted,
    RequestPersisted,
    LockReleased,
    Completed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum PlanRepairTargetWorkspaceStage {
    Running,
    HumanConfirm,
    Completed,
}

impl PlanRepairTargetWorkspaceStage {
    fn workspace_stage(self) -> WorkspaceStage {
        match self {
            Self::Running => WorkspaceStage::Running,
            Self::HumanConfirm => WorkspaceStage::HumanConfirm,
            Self::Completed => WorkspaceStage::Completed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PlanRepairTransitionJournal {
    operation: PlanRepairTransitionOperation,
    phase: PlanRepairTransitionPhase,
    project_id: String,
    issue_id: String,
    plan_id: String,
    session_id: String,
    request_id: String,
    amendment_id: String,
    fingerprint: String,
    base_plan_revision_id: String,
    target_timeline_nodes: Vec<TimelineNode>,
    target_active_node_id: Option<String>,
    target_snapshot: PlanRepairSessionSnapshotDto,
    target_workspace_stage: PlanRepairTargetWorkspaceStage,
    target_session_status: WorkspaceSessionStatus,
    target_request_status: PlanRepairRequestStatus,
    release_lock: bool,
    created_at: String,
    updated_at: String,
}

struct PlanRepairTransitionTarget {
    snapshot: PlanRepairSessionSnapshotDto,
    timeline_nodes: Vec<TimelineNode>,
    active_node_id: Option<String>,
    workspace_stage: PlanRepairTargetWorkspaceStage,
    session_status: WorkspaceSessionStatus,
    request_status: PlanRepairRequestStatus,
    release_lock: bool,
}

pub(crate) fn awaiting_confirmation_transition(
    engine: &WorkspaceEngine,
    mut snapshot: PlanRepairSessionSnapshotDto,
    package: PlanRepairAwaitingConfirmationPackage,
) -> PlanRepairTransitionJournal {
    let now = Utc::now().to_rfc3339();
    let (timeline_nodes, active_node_id) = awaiting_confirmation_timeline(engine, &now);
    snapshot.request.status = PlanRepairRequestStatus::AwaitingConfirmation;
    snapshot.request.updated_at = now.clone();
    snapshot.stage = PlanRepairSessionStage::AwaitingConfirmation;
    snapshot.projection = Some(package.projection);
    snapshot.amendment = Some(package.amendment);
    snapshot.validation = Some(package.validation);
    snapshot.impact = Some(package.impact);
    snapshot.plan_review = Some(package.plan_review);
    snapshot.package_identity = Some(package.package_identity);
    snapshot.timeline_nodes = timeline_nodes.clone();
    snapshot.error = None;
    transition_journal(
        engine,
        PlanRepairTransitionOperation::AwaitingConfirmation,
        PlanRepairTransitionTarget {
            snapshot,
            timeline_nodes,
            active_node_id,
            workspace_stage: PlanRepairTargetWorkspaceStage::HumanConfirm,
            session_status: WorkspaceSessionStatus::WaitingForHuman,
            request_status: PlanRepairRequestStatus::AwaitingConfirmation,
            release_lock: false,
        },
        now,
    )
}

pub(crate) fn confirmation_transition(
    engine: &WorkspaceEngine,
    mut snapshot: PlanRepairSessionSnapshotDto,
) -> PlanRepairTransitionJournal {
    let now = Utc::now().to_rfc3339();
    let mut timeline_nodes = engine.timeline_nodes.clone();
    if let Some(node) = timeline_nodes
        .iter_mut()
        .find(|node| node.node_type == TimelineNodeType::PlanAmendmentConfirmation)
    {
        complete_timeline_node(node, "用户已确认 Plan Amendment", &now);
    }
    snapshot.timeline_nodes = timeline_nodes.clone();
    transition_journal(
        engine,
        PlanRepairTransitionOperation::Confirm,
        PlanRepairTransitionTarget {
            snapshot,
            timeline_nodes,
            active_node_id: None,
            workspace_stage: PlanRepairTargetWorkspaceStage::HumanConfirm,
            session_status: WorkspaceSessionStatus::WaitingForHuman,
            request_status: PlanRepairRequestStatus::AwaitingConfirmation,
            release_lock: false,
        },
        now,
    )
}

pub(crate) fn cancellation_transition(
    engine: &WorkspaceEngine,
    mut snapshot: PlanRepairSessionSnapshotDto,
    cancel_summary: String,
) -> PlanRepairTransitionJournal {
    let now = Utc::now().to_rfc3339();
    let mut timeline_nodes = engine.timeline_nodes.clone();
    if let Some(node_id) = engine.active_node_id.as_deref()
        && let Some(node) = timeline_nodes
            .iter_mut()
            .find(|node| node.node_id == node_id)
    {
        skip_timeline_node(node, "Plan Amendment 已取消", &now);
    }
    append_timeline_node(
        engine,
        &mut timeline_nodes,
        TimelineNodeType::PlanAmendmentCancelled,
        WsWorkspaceStage::Completed,
        "Plan Amendment 已取消",
        Some(cancel_summary.clone()),
        TimelineNodeStatus::Completed,
        true,
        &now,
    );
    snapshot.request.status = PlanRepairRequestStatus::Cancelled;
    snapshot.request.updated_at = now.clone();
    snapshot.stage = PlanRepairSessionStage::Failed;
    snapshot.timeline_nodes = timeline_nodes.clone();
    snapshot.error = Some(cancel_summary);
    transition_journal(
        engine,
        PlanRepairTransitionOperation::Cancel,
        PlanRepairTransitionTarget {
            snapshot,
            timeline_nodes,
            active_node_id: None,
            workspace_stage: PlanRepairTargetWorkspaceStage::Completed,
            session_status: WorkspaceSessionStatus::Terminated,
            request_status: PlanRepairRequestStatus::Cancelled,
            release_lock: true,
        },
        now,
    )
}

impl WorkspaceEngine {
    pub(crate) fn recover_pending_plan_repair_transition(
        &mut self,
    ) -> Result<bool, PlanRepairError> {
        let lifecycle = self.plan_repair_lifecycle()?;
        let recovered = recover_plan_repair_transition(
            &lifecycle,
            &self.session.project_id,
            &self.session.issue_id,
            &self.session.session_id,
        )?;
        if let Some(journal) = recovered {
            self.apply_plan_repair_transition_state(&journal);
            return Ok(true);
        }
        Ok(false)
    }

    pub(crate) fn commit_plan_repair_transition(
        &mut self,
        mut journal: PlanRepairTransitionJournal,
    ) -> Result<(), PlanRepairError> {
        let lifecycle = self.plan_repair_lifecycle()?;
        save_transition_journal(&lifecycle, &journal)?;
        apply_transition_journal(&lifecycle, &mut journal, self.plan_repair_crash_after)?;
        self.apply_plan_repair_transition_state(&journal);
        Ok(())
    }

    fn plan_repair_lifecycle(&self) -> Result<LifecycleStore, PlanRepairError> {
        self.lifecycle_store.clone().ok_or_else(|| {
            PlanRepairError::Store(ProductStoreError::Io(
                "plan repair requires a persistent workspace engine".to_string(),
            ))
        })
    }

    fn apply_plan_repair_transition_state(&mut self, journal: &PlanRepairTransitionJournal) {
        self.timeline_nodes = journal.target_timeline_nodes.clone();
        self.active_node_id = journal.target_active_node_id.clone();
        self.session.stage = journal.target_workspace_stage.workspace_stage();
        self.plan_repair_snapshot = Some(journal.target_snapshot.clone());
    }
}

pub(crate) fn recover_plan_repair_transition(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    session_id: &str,
) -> Result<Option<PlanRepairTransitionJournal>, PlanRepairError> {
    let Some(mut journal) = load_transition_journal(lifecycle, project_id, issue_id, session_id)?
    else {
        return Ok(None);
    };
    validate_transition_journal(&journal, project_id, issue_id, session_id)?;
    apply_transition_journal(lifecycle, &mut journal, None)?;
    Ok(Some(journal))
}

fn transition_journal(
    engine: &WorkspaceEngine,
    operation: PlanRepairTransitionOperation,
    target: PlanRepairTransitionTarget,
    now: String,
) -> PlanRepairTransitionJournal {
    let request = &target.snapshot.request;
    let amendment_id = request.amendment_id.clone().unwrap_or_default();
    PlanRepairTransitionJournal {
        operation,
        phase: PlanRepairTransitionPhase::Prepared,
        project_id: engine.session.project_id.clone(),
        issue_id: engine.session.issue_id.clone(),
        plan_id: request.plan_id.clone(),
        session_id: engine.session.session_id.clone(),
        request_id: request.id.clone(),
        amendment_id,
        fingerprint: request.fingerprint.clone(),
        base_plan_revision_id: request.base_plan_revision_id.clone(),
        target_timeline_nodes: target.timeline_nodes,
        target_active_node_id: target.active_node_id,
        target_snapshot: target.snapshot,
        target_workspace_stage: target.workspace_stage,
        target_session_status: target.session_status,
        target_request_status: target.request_status,
        release_lock: target.release_lock,
        created_at: now.clone(),
        updated_at: now,
    }
}

fn apply_transition_journal(
    lifecycle: &LifecycleStore,
    journal: &mut PlanRepairTransitionJournal,
    crash_after: Option<PlanRepairCrashPoint>,
) -> Result<(), PlanRepairError> {
    validate_transition_journal(
        journal,
        &journal.project_id,
        &journal.issue_id,
        &journal.session_id,
    )?;
    let revision_store = WorkItemRevisionStore::new(lifecycle.app_paths());
    ensure_cancel_not_published(&revision_store, journal)?;

    if journal.phase < PlanRepairTransitionPhase::TimelinePersisted {
        lifecycle
            .save_timeline_nodes(&journal.session_id, &journal.target_timeline_nodes)
            .map_err(PlanRepairError::Store)?;
        advance_transition(
            lifecycle,
            journal,
            PlanRepairTransitionPhase::TimelinePersisted,
        )?;
        maybe_simulate_crash(crash_after, PlanRepairCrashPoint::TimelinePersisted)?;
    }
    if journal.phase < PlanRepairTransitionPhase::SnapshotPersisted {
        lifecycle
            .save_plan_repair_session_state(
                &journal.project_id,
                &journal.issue_id,
                &journal.session_id,
                &journal.target_snapshot,
            )
            .map_err(PlanRepairError::Store)?;
        advance_transition(
            lifecycle,
            journal,
            PlanRepairTransitionPhase::SnapshotPersisted,
        )?;
        maybe_simulate_crash(crash_after, PlanRepairCrashPoint::SnapshotPersisted)?;
    }
    if journal.phase < PlanRepairTransitionPhase::SessionPersisted {
        lifecycle
            .update_workspace_session_status(
                &journal.session_id,
                journal.target_session_status.clone(),
            )
            .map_err(PlanRepairError::Store)?;
        advance_transition(
            lifecycle,
            journal,
            PlanRepairTransitionPhase::SessionPersisted,
        )?;
        maybe_simulate_crash(crash_after, PlanRepairCrashPoint::SessionPersisted)?;
    }
    if journal.phase < PlanRepairTransitionPhase::RequestPersisted {
        ensure_cancel_not_published(&revision_store, journal)?;
        let plan = revision_store
            .get_plan_lineage(&journal.project_id, &journal.issue_id, &journal.plan_id)
            .map_err(PlanRepairError::Store)?;
        let request = revision_store
            .update_repair_request_status(
                &plan,
                &journal.request_id,
                journal.target_request_status.clone(),
            )
            .map_err(PlanRepairError::Store)?;
        journal.target_snapshot.request = request;
        save_transition_journal(lifecycle, journal)?;
        lifecycle
            .save_plan_repair_session_state(
                &journal.project_id,
                &journal.issue_id,
                &journal.session_id,
                &journal.target_snapshot,
            )
            .map_err(PlanRepairError::Store)?;
        advance_transition(
            lifecycle,
            journal,
            PlanRepairTransitionPhase::RequestPersisted,
        )?;
        maybe_simulate_crash(crash_after, PlanRepairCrashPoint::RequestPersisted)?;
    }
    if journal.release_lock && journal.phase < PlanRepairTransitionPhase::LockReleased {
        ensure_cancel_not_published(&revision_store, journal)?;
        let stored_plan = revision_store
            .get_plan_lineage(&journal.project_id, &journal.issue_id, &journal.plan_id)
            .map_err(PlanRepairError::Store)?;
        let next_plan_revision_id = journal
            .target_snapshot
            .amendment
            .as_ref()
            .map(|amendment| amendment.new_plan_revision_id.as_str())
            .ok_or_else(|| {
                PlanRepairError::InvalidRepairTarget(
                    "cancel transition requires an amendment manifest".to_string(),
                )
            })?;
        match stored_plan.active_amendment_id.as_deref() {
            Some(active) if active == journal.amendment_id => {
                match revision_store
                    .compare_and_release_active_amendment(
                        &stored_plan,
                        &journal.amendment_id,
                        &journal.base_plan_revision_id,
                        next_plan_revision_id,
                    )
                    .map_err(PlanRepairError::Store)?
                {
                    ActiveAmendmentReleaseOutcome::Released(_) => {}
                    ActiveAmendmentReleaseOutcome::PlanPublished(_) => {
                        return fail_cancel_after_publication(
                            &revision_store,
                            &stored_plan,
                            journal,
                        );
                    }
                }
            }
            None if stored_plan.active_revision_id.as_deref()
                == Some(journal.base_plan_revision_id.as_str()) => {}
            None => {
                return fail_cancel_after_publication(&revision_store, &stored_plan, journal);
            }
            Some(_) => {
                return Err(PlanRepairError::Store(
                    ProductStoreError::IdentityMismatch {
                        kind: "active_plan_amendment",
                        id: journal.plan_id.clone(),
                    },
                ));
            }
        }
        advance_transition(lifecycle, journal, PlanRepairTransitionPhase::LockReleased)?;
        maybe_simulate_crash(crash_after, PlanRepairCrashPoint::LockReleased)?;
    }
    journal.phase = PlanRepairTransitionPhase::Completed;
    journal.updated_at = Utc::now().to_rfc3339();
    save_transition_journal(lifecycle, journal)?;
    delete_transition_journal(lifecycle, journal)?;
    Ok(())
}

fn ensure_cancel_not_published(
    revision_store: &WorkItemRevisionStore,
    journal: &PlanRepairTransitionJournal,
) -> Result<(), PlanRepairError> {
    if journal.operation != PlanRepairTransitionOperation::Cancel
        || journal.phase >= PlanRepairTransitionPhase::LockReleased
    {
        return Ok(());
    }
    let plan = revision_store
        .get_plan_lineage(&journal.project_id, &journal.issue_id, &journal.plan_id)
        .map_err(PlanRepairError::Store)?;
    let next_plan_revision_id = journal
        .target_snapshot
        .amendment
        .as_ref()
        .map(|amendment| amendment.new_plan_revision_id.as_str())
        .ok_or_else(|| {
            PlanRepairError::InvalidRepairTarget(
                "cancel transition requires an amendment manifest".to_string(),
            )
        })?;
    if plan.active_revision_id.as_deref() == Some(next_plan_revision_id) {
        return fail_cancel_after_publication(revision_store, &plan, journal);
    }
    match revision_store.find_plan_amendment_publication_journal(&plan, &journal.amendment_id) {
        Ok(Some(publication))
            if publication.phase == PlanAmendmentPublicationPhase::PlanPublished =>
        {
            fail_cancel_after_publication(revision_store, &plan, journal)
        }
        Ok(_) => Ok(()),
        Err(error) => Err(PlanRepairError::Store(error)),
    }
}

fn fail_cancel_after_publication(
    revision_store: &WorkItemRevisionStore,
    plan: &WorkItemPlanLineage,
    journal: &PlanRepairTransitionJournal,
) -> Result<(), PlanRepairError> {
    if journal.phase >= PlanRepairTransitionPhase::RequestPersisted {
        let request = revision_store
            .get_repair_request(plan, &journal.request_id)
            .map_err(PlanRepairError::Store)?;
        if request.status == PlanRepairRequestStatus::Cancelled {
            revision_store
                .update_repair_request_status(
                    plan,
                    &journal.request_id,
                    PlanRepairRequestStatus::AwaitingConfirmation,
                )
                .map_err(PlanRepairError::Store)?;
        }
    }
    Err(cancel_plan_published_conflict())
}

fn cancel_plan_published_conflict() -> PlanRepairError {
    PlanRepairError::AmendmentConflict {
        expected: "before_plan_published".to_string(),
        actual: "plan_published".to_string(),
    }
}

fn advance_transition(
    lifecycle: &LifecycleStore,
    journal: &mut PlanRepairTransitionJournal,
    phase: PlanRepairTransitionPhase,
) -> Result<(), PlanRepairError> {
    journal.phase = phase;
    journal.updated_at = Utc::now().to_rfc3339();
    save_transition_journal(lifecycle, journal)
}

fn validate_transition_journal(
    journal: &PlanRepairTransitionJournal,
    project_id: &str,
    issue_id: &str,
    session_id: &str,
) -> Result<(), PlanRepairError> {
    let request = &journal.target_snapshot.request;
    let link = &journal.target_snapshot.link;
    if journal.project_id != project_id
        || journal.issue_id != issue_id
        || journal.session_id != session_id
        || journal.plan_id != request.plan_id
        || journal.request_id != request.id
        || journal.fingerprint != request.fingerprint
        || journal.base_plan_revision_id != request.base_plan_revision_id
        || request.amendment_id.as_deref() != Some(journal.amendment_id.as_str())
        || link.child_session_id != session_id
        || link.trigger.repair_request_id != journal.request_id
        || link.trigger.amendment_id != journal.amendment_id
        || link.trigger.fingerprint != journal.fingerprint
        || link.trigger.base_plan_revision_id != journal.base_plan_revision_id
        || journal.target_timeline_nodes.is_empty()
    {
        return Err(PlanRepairError::Store(
            ProductStoreError::IdentityMismatch {
                kind: "plan_repair_transition_journal",
                id: session_id.to_string(),
            },
        ));
    }
    Ok(())
}

fn journal_path(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    session_id: &str,
) -> Result<PathBuf, PlanRepairError> {
    validate_relative_id(project_id).map_err(PlanRepairError::Store)?;
    validate_relative_id(issue_id).map_err(PlanRepairError::Store)?;
    validate_relative_id(session_id).map_err(PlanRepairError::Store)?;
    Ok(lifecycle
        .workspace_timeline_root_for_issue_session(project_id, issue_id, session_id)
        .map_err(PlanRepairError::Store)?
        .join(PLAN_REPAIR_TRANSITION_JOURNAL_FILE))
}

fn save_transition_journal(
    lifecycle: &LifecycleStore,
    journal: &PlanRepairTransitionJournal,
) -> Result<(), PlanRepairError> {
    let path = journal_path(
        lifecycle,
        &journal.project_id,
        &journal.issue_id,
        &journal.session_id,
    )?;
    write_json(&path, journal).map_err(PlanRepairError::Store)
}

fn load_transition_journal(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    session_id: &str,
) -> Result<Option<PlanRepairTransitionJournal>, PlanRepairError> {
    let path = journal_path(lifecycle, project_id, issue_id, session_id)?;
    if !path.exists() {
        return Ok(None);
    }
    read_json(&path).map(Some).map_err(PlanRepairError::Store)
}

fn delete_transition_journal(
    lifecycle: &LifecycleStore,
    journal: &PlanRepairTransitionJournal,
) -> Result<(), PlanRepairError> {
    let path = journal_path(
        lifecycle,
        &journal.project_id,
        &journal.issue_id,
        &journal.session_id,
    )?;
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(PlanRepairError::Store(ProductStoreError::Io(
            error.to_string(),
        ))),
    }
}

fn maybe_simulate_crash(
    configured: Option<PlanRepairCrashPoint>,
    current: PlanRepairCrashPoint,
) -> Result<(), PlanRepairError> {
    if configured == Some(current) {
        return Err(PlanRepairError::Store(ProductStoreError::Io(format!(
            "simulated plan repair crash after {current:?}"
        ))));
    }
    Ok(())
}

fn awaiting_confirmation_timeline(
    engine: &WorkspaceEngine,
    now: &str,
) -> (Vec<TimelineNode>, Option<String>) {
    let mut timeline_nodes = engine.timeline_nodes.clone();
    if let Some(node_id) = engine.active_node_id.as_deref()
        && let Some(node) = timeline_nodes
            .iter_mut()
            .find(|node| node.node_id == node_id)
    {
        complete_timeline_node(node, "Work Item 修订已生成", now);
    }
    for (node_type, title, summary) in [
        (
            TimelineNodeType::PlanRepairContractValidation,
            "Contract Validation",
            "Contract 校验通过",
        ),
        (
            TimelineNodeType::PlanRepairProjectionGeneration,
            "Projection Generation",
            "三投影已生成",
        ),
        (
            TimelineNodeType::PlanRepairPlanReview,
            "Plan Review",
            "Plan Review 已通过",
        ),
    ] {
        append_timeline_node(
            engine,
            &mut timeline_nodes,
            node_type,
            WsWorkspaceStage::Running,
            title,
            Some(summary.to_string()),
            TimelineNodeStatus::Completed,
            true,
            now,
        );
    }
    let active_node_id = append_timeline_node(
        engine,
        &mut timeline_nodes,
        TimelineNodeType::PlanAmendmentConfirmation,
        WsWorkspaceStage::HumanConfirm,
        "确认 Plan Amendment",
        Some("等待一次最终确认".to_string()),
        TimelineNodeStatus::Active,
        false,
        now,
    );
    (timeline_nodes, Some(active_node_id))
}

#[allow(clippy::too_many_arguments)]
fn append_timeline_node(
    engine: &WorkspaceEngine,
    timeline_nodes: &mut Vec<TimelineNode>,
    node_type: TimelineNodeType,
    stage: WsWorkspaceStage,
    title: &str,
    summary: Option<String>,
    status: TimelineNodeStatus,
    completed: bool,
    now: &str,
) -> String {
    let node_id = format!("timeline_node_{:03}", timeline_nodes.len() + 1);
    timeline_nodes.push(TimelineNode {
        node_id: node_id.clone(),
        node_type,
        agent: None,
        stage,
        round: None,
        status,
        title: title.to_string(),
        summary,
        started_at: now.to_string(),
        completed_at: completed.then(|| now.to_string()),
        duration_ms: completed.then_some(0),
        artifact_ref: engine
            .session
            .artifact
            .as_ref()
            .map(|_| "artifact_current".to_string()),
        provider_config_snapshot: ProviderConfigSnapshot {
            author: engine.session.author_provider.clone(),
            reviewer: engine.session.reviewer_provider.clone(),
            review_rounds: engine.session.review_rounds,
        },
        retry: None,
    });
    node_id
}

fn complete_timeline_node(node: &mut TimelineNode, summary: &str, now: &str) {
    node.status = TimelineNodeStatus::Completed;
    node.summary = Some(summary.to_string());
    node.completed_at = Some(now.to_string());
}

fn skip_timeline_node(node: &mut TimelineNode, summary: &str, now: &str) {
    node.status = TimelineNodeStatus::Skipped;
    node.summary = Some(summary.to_string());
    node.completed_at = Some(now.to_string());
}
