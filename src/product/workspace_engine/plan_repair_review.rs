use chrono::Utc;

use crate::product::models::{
    PlanRepairAwaitingConfirmationPackage, PlanRepairImpactScopeReview, PlanRepairPackageIdentity,
    PlanRepairRequestStatus, PlanRepairReviewAttestation, PlanRepairSessionStage,
};
use crate::product::plan_repair::{PlanRepairEngine, PlanRepairError, PreparedPlanAmendment};

use super::plan_repair_validation::{
    awaiting_confirmation_package_from_snapshot, persisted_candidate_package_fingerprint,
    validate_persisted_awaiting_confirmation_package,
};
use super::*;

impl WorkspaceEngine {
    pub async fn enter_plan_repair_review(
        &mut self,
        prepared: PreparedPlanAmendment,
    ) -> Result<(), PlanRepairError> {
        let mut snapshot = self.require_plan_repair_snapshot()?.clone();
        if snapshot.stage != PlanRepairSessionStage::AuthoringRevision
            && snapshot.stage != PlanRepairSessionStage::ValidatingContract
            && snapshot.stage != PlanRepairSessionStage::GeneratingProjections
            && snapshot.stage != PlanRepairSessionStage::PlanReview
        {
            return Err(PlanRepairError::InvalidRepairTarget(format!(
                "plan repair cannot enter Plan Review from {:?}",
                snapshot.stage
            )));
        }
        if prepared.manifest.repair_request_id != snapshot.request.id
            || prepared.base_plan_revision_id != snapshot.request.base_plan_revision_id
            || snapshot.request.amendment_id.as_deref() != Some(prepared.manifest.id.as_str())
        {
            return Err(invalid_plan_repair_review(
                "prepared amendment identity does not match the repair session",
            ));
        }
        let lifecycle = self.persistent_lifecycle()?.clone();
        let store = WorkItemRevisionStore::new(lifecycle.app_paths());
        let plan = store
            .get_plan_lineage(
                &self.session.project_id,
                &self.session.issue_id,
                &snapshot.request.plan_id,
            )
            .map_err(PlanRepairError::Store)?;
        PlanRepairEngine::new(store, plan).persist_candidate(&prepared)?;
        snapshot.stage = PlanRepairSessionStage::PlanReview;
        snapshot.projection = Some(prepared.plan_projection_bundle.clone());
        snapshot.amendment = Some(prepared.manifest.clone());
        snapshot.validation = Some(prepared.validation_report.clone());
        snapshot.impact = Some(prepared.impact_report.clone());
        snapshot.plan_review = None;
        snapshot.package_identity = None;
        snapshot.impact_scope_review = None;
        snapshot.error = None;
        lifecycle
            .save_plan_repair_session_state(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.session_id,
                &snapshot,
            )
            .map_err(PlanRepairError::Store)?;
        self.plan_repair_snapshot = Some(snapshot);
        self.update_artifact(ArtifactPayload::WorkItemPlanProjection {
            projection: Box::new(prepared.plan_projection_bundle),
        })
        .await;
        self.begin_work_item_plan_outline_review_run().await;
        if let Some(snapshot) = self.plan_repair_snapshot.as_mut() {
            snapshot.timeline_nodes = self.timeline_nodes.clone();
            lifecycle
                .save_plan_repair_session_state(
                    &self.session.project_id,
                    &self.session.issue_id,
                    &self.session.session_id,
                    snapshot,
                )
                .map_err(PlanRepairError::Store)?;
        }
        Ok(())
    }

    pub async fn request_plan_repair_impact_scope_review(
        &mut self,
        accepted_impact_scope: Vec<String>,
        risk_acceptance_reason: String,
    ) -> Result<(), PlanRepairError> {
        self.recover_pending_plan_repair_transition()?;
        let mut snapshot = self.require_plan_repair_snapshot()?.clone();
        if snapshot.stage != PlanRepairSessionStage::AwaitingConfirmation {
            return Err(invalid_plan_repair_review(
                "impact scope re-review requires AwaitingConfirmation",
            ));
        }
        let lifecycle = self.persistent_lifecycle()?.clone();
        let store = WorkItemRevisionStore::new(lifecycle.app_paths());
        let plan = store
            .get_plan_lineage(
                &self.session.project_id,
                &self.session.issue_id,
                &snapshot.request.plan_id,
            )
            .map_err(PlanRepairError::Store)?;
        let stored_request = store
            .get_repair_request(&plan, &snapshot.request.id)
            .map_err(PlanRepairError::Store)?;
        if stored_request != snapshot.request
            || stored_request.status != PlanRepairRequestStatus::AwaitingConfirmation
        {
            return Err(invalid_plan_repair_review(
                "impact scope re-review requires the authoritative awaiting request",
            ));
        }
        let package = awaiting_confirmation_package_from_snapshot(&snapshot)?;
        validate_persisted_awaiting_confirmation_package(&store, &snapshot, &plan, &package)?;
        let minimum = system_minimum_impact_scope(&package.amendment);
        let accepted = sorted_unique(accepted_impact_scope);
        let minimum_set = minimum
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        let accepted_set = accepted
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        if minimum_set.is_subset(&accepted_set) {
            return Err(invalid_plan_repair_review(
                "impact scope re-review is only valid for a proposed shrink",
            ));
        }
        let risk = risk_acceptance_reason.trim().to_string();
        if risk.is_empty() {
            return Err(PlanRepairError::RiskAcceptanceRequired);
        }
        let candidate_revision = store
            .get_plan_revision(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                &package.amendment.new_plan_revision_id,
            )
            .map_err(PlanRepairError::Store)?;
        if accepted
            .iter()
            .any(|unit| !candidate_revision.work_item_bindings.contains_key(unit))
        {
            return Err(invalid_plan_repair_review(
                "proposed accepted impact scope contains an unknown unit",
            ));
        }
        let fingerprint = persisted_candidate_package_fingerprint(
            &store,
            &plan,
            &snapshot.request,
            &package.amendment,
            &package.projection,
            &package.validation,
            &package.impact,
        )?;
        if package.package_identity.candidate_package_fingerprint != fingerprint {
            return Err(invalid_plan_repair_review(
                "awaiting package fingerprint changed before impact scope re-review",
            ));
        }
        let review_no = self
            .timeline_nodes
            .iter()
            .filter(|node| node.node_type == TimelineNodeType::WorkItemPlanOutlineReview)
            .count()
            .max(1);
        snapshot.stage = PlanRepairSessionStage::PlanReview;
        snapshot.impact_scope_review = Some(PlanRepairImpactScopeReview {
            system_minimum_impact_scope: minimum,
            proposed_accepted_impact_scope: accepted,
            risk_acceptance_reason: risk,
            candidate_package_fingerprint: fingerprint,
            review_generation_round_id: format!("plan_repair_impact_scope_review_{review_no:04}"),
        });
        snapshot.plan_review = None;
        snapshot.package_identity = None;
        snapshot.error = None;
        lifecycle
            .save_plan_repair_session_state(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.session_id,
                &snapshot,
            )
            .map_err(PlanRepairError::Store)?;
        self.plan_repair_snapshot = Some(snapshot);
        self.update_artifact(ArtifactPayload::WorkItemPlanProjection {
            projection: Box::new(package.projection),
        })
        .await;
        self.begin_work_item_plan_outline_review_run().await;
        if let Some(snapshot) = self.plan_repair_snapshot.as_mut() {
            snapshot.timeline_nodes = self.timeline_nodes.clone();
            lifecycle
                .save_plan_repair_session_state(
                    &self.session.project_id,
                    &self.session.issue_id,
                    &self.session.session_id,
                    snapshot,
                )
                .map_err(PlanRepairError::Store)?;
        }
        Ok(())
    }

    pub(crate) async fn route_plan_repair_candidate_review(&mut self, verdict: ReviewVerdict) {
        let Some(review) = verdict.work_item_plan_review.clone() else {
            self.enter_review_decision(
                self.active_review_round().unwrap_or(1),
                "Plan Repair review 缺少结构化 Plan Review 结果".to_string(),
            )
            .await;
            return;
        };
        if verdict.verdict != ReviewVerdictType::Pass
            || verdict.review_gate != ReviewGate::UserConfirmAllowed
            || review.verdict != WorkItemPlanReviewVerdict::Pass
            || review.review_scope != WorkItemPlanReviewScope::Outline
            || review.review_action != WorkItemPlanReviewAction::Continue
            || !review.gates.is_empty()
            || review.draft_id.is_some()
            || review.batch_id.is_some()
        {
            let summary = verdict.summary.clone();
            if verdict.review_gate == ReviewGate::RequiresRevision
                || review.verdict == WorkItemPlanReviewVerdict::Revise
                || review.verdict == WorkItemPlanReviewVerdict::PlanReopenRequired
            {
                self.enter_review_decision(self.active_review_round().unwrap_or(1), summary)
                    .await;
            } else {
                self.enter_human_confirm(Some(summary)).await;
            }
            return;
        }

        if let Err(error) = self.persist_plan_repair_review_and_await(review).await {
            let message = format!("{error:?}");
            if let Some(snapshot) = self.plan_repair_snapshot.as_mut() {
                snapshot.error = Some(message.clone());
            }
            let _ = self.event_tx.send(EngineEvent::Error { message }).await;
            self.enter_human_confirm(Some("Plan Repair review provenance 持久化失败".to_string()))
                .await;
        }
    }

    async fn persist_plan_repair_review_and_await(
        &mut self,
        review: WorkItemPlanReviewComplete,
    ) -> Result<(), PlanRepairError> {
        let snapshot = self.require_plan_repair_snapshot()?.clone();
        let amendment = snapshot
            .amendment
            .clone()
            .ok_or_else(|| invalid_plan_repair_review("candidate amendment is missing"))?;
        let projection = snapshot
            .projection
            .clone()
            .ok_or_else(|| invalid_plan_repair_review("candidate projection is missing"))?;
        let validation = snapshot
            .validation
            .clone()
            .ok_or_else(|| invalid_plan_repair_review("candidate validation is missing"))?;
        let impact = snapshot
            .impact
            .clone()
            .ok_or_else(|| invalid_plan_repair_review("candidate impact is missing"))?;
        let lifecycle = self.persistent_lifecycle()?;
        let store = WorkItemRevisionStore::new(lifecycle.app_paths());
        let plan = store
            .get_plan_lineage(
                &self.session.project_id,
                &self.session.issue_id,
                &snapshot.request.plan_id,
            )
            .map_err(PlanRepairError::Store)?;
        let stored_request = store
            .get_repair_request(&plan, &snapshot.request.id)
            .map_err(PlanRepairError::Store)?;
        if stored_request != snapshot.request {
            return Err(invalid_plan_repair_review(
                "repair request changed while Plan Review was running",
            ));
        }
        let candidate_package_fingerprint = persisted_candidate_package_fingerprint(
            &store,
            &plan,
            &snapshot.request,
            &amendment,
            &projection,
            &validation,
            &impact,
        )?;
        let (accepted_impact_scope, risk_acceptance_reason) = match &snapshot.impact_scope_review {
            Some(proposal) => {
                if proposal.candidate_package_fingerprint != candidate_package_fingerprint
                    || proposal.review_generation_round_id != review.generation_round_id
                {
                    return Err(invalid_plan_repair_review(
                        "impact scope re-review provenance changed before completion",
                    ));
                }
                (
                    proposal.proposed_accepted_impact_scope.clone(),
                    Some(proposal.risk_acceptance_reason.clone()),
                )
            }
            None => (system_minimum_impact_scope(&amendment), None),
        };
        let attestation_id = format!(
            "plan_repair_review_attestation_{}_{}",
            amendment.id, review.generation_round_id
        );
        let created_at = self
            .timeline_nodes
            .iter()
            .find(|node| Some(&node.node_id) == self.active_node_id.as_ref())
            .and_then(|node| node.completed_at.clone())
            .unwrap_or_else(|| Utc::now().to_rfc3339());
        let attestation = PlanRepairReviewAttestation {
            id: attestation_id.clone(),
            request_id: snapshot.request.id.clone(),
            amendment_id: amendment.id.clone(),
            plan_id: snapshot.request.plan_id.clone(),
            base_plan_revision_id: snapshot.request.base_plan_revision_id.clone(),
            reviewed_plan_revision_id: amendment.new_plan_revision_id.clone(),
            plan_projection_bundle_id: projection.id.clone(),
            generation_round_id: review.generation_round_id.clone(),
            accepted_impact_scope,
            risk_acceptance_reason,
            candidate_package_fingerprint: candidate_package_fingerprint.clone(),
            review: review.clone(),
            created_at,
        };
        store
            .put_plan_repair_review_attestation(&plan, &attestation)
            .map_err(PlanRepairError::Store)?;
        self.enter_plan_repair_awaiting_confirmation(PlanRepairAwaitingConfirmationPackage {
            package_identity: PlanRepairPackageIdentity {
                request_id: snapshot.request.id,
                amendment_id: amendment.id.clone(),
                plan_id: snapshot.request.plan_id,
                base_plan_revision_id: snapshot.request.base_plan_revision_id,
                next_plan_revision_id: amendment.new_plan_revision_id.clone(),
                projection_bundle_id: projection.id.clone(),
                validation_report_id: validation.id.clone(),
                review_attestation_id: attestation_id,
                reviewed_plan_revision_id: amendment.new_plan_revision_id.clone(),
                review_generation_round_id: review.generation_round_id.clone(),
                candidate_package_fingerprint,
            },
            projection,
            amendment,
            validation,
            impact,
            plan_review: review,
        })
        .await
    }
}

fn system_minimum_impact_scope(
    amendment: &crate::product::models::PlanAmendmentManifest,
) -> Vec<String> {
    amendment
        .revalidation_required_units
        .iter()
        .chain(amendment.stale_units.iter())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn sorted_unique(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn invalid_plan_repair_review(message: &str) -> PlanRepairError {
    PlanRepairError::InvalidRepairTarget(format!("invalid Plan Repair review: {message}"))
}
