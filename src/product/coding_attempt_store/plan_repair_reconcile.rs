use chrono::Utc;

use crate::product::coding_models::{
    CodingAgentRole, CodingAttemptStatus, CodingExecutionAttempt, CodingTimelineNode,
    CodingTimelineNodeStatus, CodingUnitRun, CodingUnitRunStatus,
};
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    PlanRepairRequestStatus, PlanRepairSessionSnapshotDto, PlanRepairSessionStage,
    WorkspaceSessionRelation,
};
use crate::product::work_item_revision_store::WorkItemRevisionStore;

#[derive(Debug, Clone)]
pub(crate) struct PlanRepairPauseReconciliation {
    pub(crate) attempt: CodingExecutionAttempt,
    pub(crate) snapshot: Option<PlanRepairSessionSnapshotDto>,
    pub(crate) timeline_node: Option<CodingTimelineNode>,
    pub(crate) timeline_created: bool,
}

impl super::CodingAttemptStore {
    pub(crate) fn reconcile_linked_plan_repair_pause(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<PlanRepairPauseReconciliation, ProductStoreError> {
        let current = self.validate_attempt_lineage(attempt)?;
        let Some(snapshot) = self.linked_active_plan_repair_snapshot(&current)? else {
            return Ok(PlanRepairPauseReconciliation {
                attempt: current,
                snapshot: None,
                timeline_node: None,
                timeline_created: false,
            });
        };

        let paused = match current.status {
            CodingAttemptStatus::Running => self.update_attempt_status(
                &current.project_id,
                &current.issue_id,
                &current.id,
                CodingAttemptStatus::AwaitingPlanAmendment,
            )?,
            CodingAttemptStatus::AwaitingPlanAmendment
            | CodingAttemptStatus::ApplyingPlanAmendment
            | CodingAttemptStatus::AmendmentApplyFailed => current,
            _ => {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_plan_repair_attempt_status",
                    id: attempt.id.clone(),
                });
            }
        };
        let trigger_run =
            self.unique_plan_repair_trigger_run(&paused, &snapshot.request.trigger_unit_run_id)?;
        match paused.status {
            CodingAttemptStatus::AwaitingPlanAmendment => {
                self.block_unit_run_for_plan_repair(
                    &paused,
                    &trigger_run.unit_id,
                    &trigger_run.id,
                    true,
                )?;
            }
            CodingAttemptStatus::ApplyingPlanAmendment
            | CodingAttemptStatus::AmendmentApplyFailed => {
                if trigger_run.status != CodingUnitRunStatus::BlockedByPlanDefect {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "coding_unit_run_plan_repair_transition",
                        id: trigger_run.id,
                    });
                }
            }
            _ => unreachable!("pause status was validated above"),
        }
        let (timeline_node, timeline_created) =
            self.ensure_plan_repair_timeline_node(&paused, &snapshot.request.id)?;

        Ok(PlanRepairPauseReconciliation {
            attempt: paused,
            snapshot: Some(snapshot),
            timeline_node: Some(timeline_node),
            timeline_created,
        })
    }

    pub(crate) fn linked_active_plan_repair_snapshot(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<Option<PlanRepairSessionSnapshotDto>, ProductStoreError> {
        let Some(plan_id) = attempt.work_item_group_id.as_deref() else {
            return Ok(None);
        };
        let revision_store = WorkItemRevisionStore::new(self.paths());
        let plan =
            revision_store.get_plan_lineage(&attempt.project_id, &attempt.issue_id, plan_id)?;
        let active_amendment_id = match plan.active_amendment_id.as_deref() {
            Some(amendment_id) => amendment_id,
            None if amendment_status(&attempt.status) => {
                return Err(ProductStoreError::NotFound {
                    kind: "coding_linked_plan_repair",
                    id: attempt.id.clone(),
                });
            }
            None => return Ok(None),
        };
        let lifecycle = LifecycleStore::new(self.paths());
        let mut links = lifecycle
            .list_session_links(&attempt.project_id, &attempt.issue_id)?
            .into_iter()
            .filter(|link| {
                link.relation == WorkspaceSessionRelation::PlanRepair
                    && link.trigger.attempt_id == attempt.id
                    && link.trigger.amendment_id == active_amendment_id
            });
        let link = match links.next() {
            Some(link) => link,
            None if amendment_status(&attempt.status) => {
                return Err(ProductStoreError::NotFound {
                    kind: "coding_linked_plan_repair",
                    id: attempt.id.clone(),
                });
            }
            None => return Ok(None),
        };
        if links.next().is_some() {
            return Err(ProductStoreError::Ambiguous {
                kind: "coding_linked_plan_repair",
                id: attempt.id.clone(),
            });
        }
        let request = revision_store.get_repair_request(&plan, &link.trigger.repair_request_id)?;
        if request.id != link.trigger.repair_request_id
            || request.plan_id != plan.id
            || request.base_plan_revision_id != link.trigger.base_plan_revision_id
            || request.trigger_attempt_id != attempt.id
            || request.trigger_unit_run_id != link.trigger.unit_run_id
            || request.trigger_review_id != link.trigger.review_id
            || request.trigger_finding_id != link.trigger.finding_id
            || request.amendment_id.as_deref() != Some(active_amendment_id)
            || request.fingerprint != link.trigger.fingerprint
            || !matches!(
                request.status,
                PlanRepairRequestStatus::InProgress
                    | PlanRepairRequestStatus::AwaitingConfirmation
                    | PlanRepairRequestStatus::Published
            )
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_linked_plan_repair_request",
                id: request.id,
            });
        }
        let snapshot = lifecycle
            .load_plan_repair_session_state(
                &attempt.project_id,
                &attempt.issue_id,
                &link.child_session_id,
            )?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "coding_linked_plan_repair_snapshot",
                id: link.child_session_id.clone(),
            })?;
        if snapshot.request != request
            || snapshot.link != link
            || matches!(
                snapshot.stage,
                PlanRepairSessionStage::Completed | PlanRepairSessionStage::Failed
            )
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_linked_plan_repair_snapshot",
                id: attempt.id.clone(),
            });
        }
        Ok(Some(snapshot))
    }

    fn unique_plan_repair_trigger_run(
        &self,
        attempt: &CodingExecutionAttempt,
        unit_run_id: &str,
    ) -> Result<CodingUnitRun, ProductStoreError> {
        let mut matches = Vec::new();
        for unit in self.list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)? {
            matches.extend(
                self.list_coding_unit_runs(attempt, &unit.id)?
                    .into_iter()
                    .filter(|run| run.id == unit_run_id),
            );
        }
        match matches.len() {
            1 => Ok(matches.remove(0)),
            0 => Err(ProductStoreError::NotFound {
                kind: "coding_unit_run_plan_repair_authority",
                id: unit_run_id.to_string(),
            }),
            _ => Err(ProductStoreError::Ambiguous {
                kind: "coding_unit_run_plan_repair_authority",
                id: unit_run_id.to_string(),
            }),
        }
    }

    fn ensure_plan_repair_timeline_node(
        &self,
        attempt: &CodingExecutionAttempt,
        request_id: &str,
    ) -> Result<(CodingTimelineNode, bool), ProductStoreError> {
        let existing =
            self.get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let mut matching = existing.iter().filter(|node| {
            node.title == "Plan Repair"
                && node.artifact_refs.iter().any(|value| value == request_id)
        });
        if let Some(node) = matching.next() {
            if matching.next().is_some() {
                return Err(ProductStoreError::Ambiguous {
                    kind: "coding_plan_repair_timeline_node",
                    id: request_id.to_string(),
                });
            }
            return Ok((node.clone(), false));
        }
        let node = CodingTimelineNode {
            id: format!("coding_node_{:04}", existing.len() + 1),
            attempt_id: attempt.id.clone(),
            stage: attempt.stage.clone(),
            title: "Plan Repair".to_string(),
            status: CodingTimelineNodeStatus::Blocked,
            agent_role: Some(CodingAgentRole::System),
            summary: Some("Coding 已暂停，等待计划修订".to_string()),
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            artifact_refs: vec![request_id.to_string()],
        };
        self.save_timeline_node(attempt, node.clone())?;
        Ok((node, true))
    }
}

fn amendment_status(status: &CodingAttemptStatus) -> bool {
    matches!(
        status,
        CodingAttemptStatus::AwaitingPlanAmendment
            | CodingAttemptStatus::ApplyingPlanAmendment
            | CodingAttemptStatus::AmendmentApplyFailed
    )
}
