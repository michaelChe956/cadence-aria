use std::collections::{BTreeMap, BTreeSet};

use sha2::{Digest, Sha256};

use crate::product::models::{
    PlanAmendmentConfirmation, PlanAmendmentManifest, PlanAmendmentPublicationJournal,
    PlanAmendmentPublicationPhase, PlanAmendmentPublicationSnapshot,
    PlanAmendmentWorkItemArtifacts, PlanRepairRequestStatus, PlanRepairReviewAttestation,
};
use crate::web::workspace_ws_types::{
    WorkItemPlanReviewAction, WorkItemPlanReviewScope, WorkItemPlanReviewVerdict,
};

use super::{PlanRepairEngine, PreparedPlanAmendment};
use crate::product::plan_repair::PlanRepairError;

impl PlanRepairEngine {
    pub fn publish_amendment(
        &self,
        prepared: PreparedPlanAmendment,
        mut confirmation: PlanAmendmentConfirmation,
    ) -> Result<PlanAmendmentManifest, PlanRepairError> {
        let plan = self
            .store
            .get_plan_lineage(&self.plan.project_id, &self.plan.issue_id, &self.plan.id)
            .map_err(PlanRepairError::Store)?;
        validate_publication_identity(&plan, &prepared, &confirmation)?;
        let request = self
            .store
            .get_repair_request(&plan, &prepared.manifest.repair_request_id)
            .map_err(PlanRepairError::Store)?;
        if request.amendment_id.as_deref() != Some(prepared.manifest.id.as_str())
            || request.base_plan_revision_id != prepared.base_plan_revision_id
            || !matches!(
                request.status,
                PlanRepairRequestStatus::InProgress
                    | PlanRepairRequestStatus::AwaitingConfirmation
                    | PlanRepairRequestStatus::Published
                    | PlanRepairRequestStatus::Applied
            )
        {
            return Err(invalid_publication(
                "repair request is not ready for amendment publication",
            ));
        }

        confirmation.accepted_impact_scope = sorted_unique(&confirmation.accepted_impact_scope);
        confirmation.risk_acceptance_reason = confirmation
            .risk_acceptance_reason
            .map(|reason| reason.trim().to_string())
            .filter(|reason| !reason.is_empty());
        let known_units = prepared
            .next_plan_revision
            .work_item_bindings
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        if confirmation
            .accepted_impact_scope
            .iter()
            .any(|unit| !known_units.contains(unit))
        {
            return Err(invalid_publication(
                "accepted impact scope contains an unknown logical unit",
            ));
        }
        let minimum = prepared
            .manifest
            .revalidation_required_units
            .iter()
            .chain(prepared.manifest.stale_units.iter())
            .cloned()
            .collect::<BTreeSet<_>>();
        let accepted = confirmation
            .accepted_impact_scope
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let shrink = !minimum.is_subset(&accepted);
        if shrink && confirmation.risk_acceptance_reason.is_none() {
            return Err(PlanRepairError::RiskAcceptanceRequired);
        }
        let attestation_id = confirmation
            .review_attestation_id
            .as_deref()
            .ok_or(PlanRepairError::ConfirmationRequired)?;
        let attestation = self
            .store
            .get_plan_repair_review_attestation(&plan, attestation_id)
            .map_err(PlanRepairError::Store)?;
        validate_review_attestation(
            &request,
            &prepared,
            &confirmation,
            &attestation,
            &minimum,
            &accepted,
            shrink,
        )?;
        let final_manifest =
            final_plan_amendment_manifest(&prepared.manifest, &known_units, &accepted);
        if plan.active_revision_id.as_deref() == Some(prepared.next_plan_revision.id.as_str()) {
            let existing = self
                .store
                .find_plan_amendment_publication_journal(&plan, &prepared.manifest.id)
                .map_err(PlanRepairError::Store)?
                .ok_or_else(|| {
                    invalid_publication("published revision has no publication journal")
                })?;
            if existing.confirmation.as_ref() != Some(&confirmation)
                || existing
                    .snapshot
                    .as_ref()
                    .map(|snapshot| &snapshot.manifest)
                    != Some(&final_manifest)
            {
                return Err(PlanRepairError::AmendmentConflict {
                    expected: existing.artifact_fingerprint,
                    actual: "different publication replay".to_string(),
                });
            }
            let published = self
                .store
                .publish_or_resume_plan_amendment(&existing)
                .map_err(PlanRepairError::Store)?;
            if matches!(
                request.status,
                PlanRepairRequestStatus::InProgress | PlanRepairRequestStatus::AwaitingConfirmation
            ) {
                mark_request_published_after_plan(&self.store, &plan, &request.id, &existing.id)?;
            }
            return Ok(published
                .snapshot
                .ok_or_else(|| invalid_publication("published journal snapshot is missing"))?
                .manifest);
        }
        let snapshot = publication_snapshot(&self.store, &plan, &prepared, final_manifest.clone())?;
        let artifact_fingerprint = artifact_fingerprint(&snapshot, &confirmation)?;
        let journal = PlanAmendmentPublicationJournal {
            id: prepared.publication_ids.journal_id.clone(),
            project_id: plan.project_id.clone(),
            issue_id: plan.issue_id.clone(),
            plan_id: plan.id.clone(),
            amendment_id: final_manifest.id.clone(),
            request_id: final_manifest.repair_request_id.clone(),
            base_plan_revision_id: prepared.base_plan_revision_id.clone(),
            new_plan_revision_id: prepared.next_plan_revision.id.clone(),
            confirmation: Some(confirmation),
            artifact_fingerprint,
            snapshot: Some(snapshot),
            phase: PlanAmendmentPublicationPhase::Preparing,
            error: None,
            recovery: None,
            created_at: prepared.manifest.created_at.clone(),
            updated_at: prepared.manifest.created_at.clone(),
        };
        let published = self
            .store
            .publish_or_resume_plan_amendment(&journal)
            .map_err(PlanRepairError::Store)?;
        let published_manifest = published
            .snapshot
            .ok_or_else(|| invalid_publication("published journal snapshot is missing"))?
            .manifest;
        if matches!(
            request.status,
            PlanRepairRequestStatus::InProgress | PlanRepairRequestStatus::AwaitingConfirmation
        ) {
            mark_request_published_after_plan(&self.store, &plan, &request.id, &journal.id)?;
        }
        Ok(published_manifest)
    }
}

fn mark_request_published_after_plan(
    store: &crate::product::work_item_revision_store::WorkItemRevisionStore,
    plan: &crate::product::models::WorkItemPlanLineage,
    request_id: &str,
    journal_id: &str,
) -> Result<(), PlanRepairError> {
    match store.update_repair_request_status(plan, request_id, PlanRepairRequestStatus::Published) {
        Ok(_) => Ok(()),
        Err(error) => {
            let _ =
                store.mark_plan_amendment_publication_failed(plan, journal_id, error.to_string());
            Err(PlanRepairError::Store(error))
        }
    }
}

fn validate_publication_identity(
    plan: &crate::product::models::WorkItemPlanLineage,
    prepared: &PreparedPlanAmendment,
    confirmation: &PlanAmendmentConfirmation,
) -> Result<(), PlanRepairError> {
    if confirmation.amendment_id != prepared.manifest.id
        || confirmation.base_plan_revision_id != prepared.base_plan_revision_id
        || prepared.manifest.previous_plan_revision_id != prepared.base_plan_revision_id
        || prepared.manifest.new_plan_revision_id != prepared.next_plan_revision.id
        || plan.active_amendment_id.as_deref() != Some(prepared.manifest.id.as_str())
    {
        return Err(PlanRepairError::AmendmentConflict {
            expected: format!(
                "{}@{}",
                prepared.base_plan_revision_id, prepared.manifest.id
            ),
            actual: format!(
                "{}@{}",
                plan.active_revision_id.clone().unwrap_or_default(),
                plan.active_amendment_id.clone().unwrap_or_default()
            ),
        });
    }
    match plan.active_revision_id.as_deref() {
        Some(active)
            if active == prepared.base_plan_revision_id
                || active == prepared.next_plan_revision.id =>
        {
            Ok(())
        }
        _ => Err(PlanRepairError::AmendmentConflict {
            expected: prepared.base_plan_revision_id.clone(),
            actual: plan.active_revision_id.clone().unwrap_or_default(),
        }),
    }
}

fn validate_review_attestation(
    request: &crate::product::models::PlanRepairRequest,
    prepared: &PreparedPlanAmendment,
    confirmation: &PlanAmendmentConfirmation,
    attestation: &PlanRepairReviewAttestation,
    minimum: &BTreeSet<String>,
    accepted: &BTreeSet<String>,
    shrink: bool,
) -> Result<(), PlanRepairError> {
    let candidate_package_fingerprint = crate::product::plan_repair::candidate_package_fingerprint(
        request,
        &prepared.manifest,
        &prepared.plan_projection_bundle,
        &prepared.work_item_projection_bundles,
        &prepared.validation_report,
        &prepared.impact_report,
    )?;
    let review = &attestation.review;
    if attestation.request_id != prepared.manifest.repair_request_id
        || attestation.amendment_id != prepared.manifest.id
        || attestation.plan_id != prepared.next_plan_revision.plan_id
        || attestation.base_plan_revision_id != prepared.base_plan_revision_id
        || attestation.reviewed_plan_revision_id != prepared.next_plan_revision.id
        || attestation.plan_projection_bundle_id != prepared.plan_projection_bundle.id
        || attestation.candidate_package_fingerprint != candidate_package_fingerprint
        || review.generation_round_id != attestation.generation_round_id
        || review.verdict != WorkItemPlanReviewVerdict::Pass
        || review.review_scope != WorkItemPlanReviewScope::Outline
        || review.review_action != WorkItemPlanReviewAction::Continue
        || !review.gates.is_empty()
        || review.draft_id.is_some()
        || review.batch_id.is_some()
    {
        return Err(invalid_publication(
            "review attestation provenance mismatch",
        ));
    }
    let attested_scope = attestation
        .accepted_impact_scope
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if shrink {
        if &attested_scope != accepted
            || attestation.risk_acceptance_reason.as_deref()
                != confirmation.risk_acceptance_reason.as_deref()
        {
            return Err(invalid_publication(
                "shrunken scope requires a new review attestation bound to scope and risk",
            ));
        }
    } else if &attested_scope != minimum || attestation.risk_acceptance_reason.is_some() {
        return Err(invalid_publication(
            "normal or expanded scope must use the system-minimum review attestation",
        ));
    }
    Ok(())
}

pub(crate) fn final_plan_amendment_manifest(
    prepared: &PlanAmendmentManifest,
    known_units: &BTreeSet<String>,
    accepted: &BTreeSet<String>,
) -> PlanAmendmentManifest {
    let revised = prepared
        .revised_work_items
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let original_stale = prepared
        .stale_units
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let original_revalidation = prepared
        .revalidation_required_units
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let stale = original_stale
        .intersection(accepted)
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut revalidation = original_revalidation
        .intersection(accepted)
        .cloned()
        .collect::<BTreeSet<_>>();
    revalidation.extend(
        accepted
            .difference(&original_stale)
            .filter(|unit| !revised.contains(*unit))
            .cloned(),
    );
    revalidation.retain(|unit| !stale.contains(unit));
    let replacements = prepared
        .replacement_units
        .keys()
        .chain(prepared.replacement_units.values().flatten())
        .cloned()
        .collect::<BTreeSet<_>>();
    let unaffected = known_units
        .iter()
        .filter(|unit| {
            !revised.contains(*unit)
                && !stale.contains(*unit)
                && !revalidation.contains(*unit)
                && !replacements.contains(*unit)
        })
        .cloned()
        .collect();
    let mut manifest = prepared.clone();
    manifest.stale_units = stale.into_iter().collect();
    manifest.revalidation_required_units = revalidation.into_iter().collect();
    manifest.unaffected_units = unaffected;
    manifest
}

fn publication_snapshot(
    store: &crate::product::work_item_revision_store::WorkItemRevisionStore,
    plan: &crate::product::models::WorkItemPlanLineage,
    prepared: &PreparedPlanAmendment,
    manifest: PlanAmendmentManifest,
) -> Result<PlanAmendmentPublicationSnapshot, PlanRepairError> {
    let drafts = prepared
        .draft_revisions
        .iter()
        .map(|value| (value.logical_work_item_id.as_str(), value))
        .collect::<BTreeMap<_, _>>();
    let verifications = prepared
        .verification_plan_revisions
        .iter()
        .map(|value| (value.logical_work_item_id.as_str(), value))
        .collect::<BTreeMap<_, _>>();
    let bundles = prepared
        .work_item_projection_bundles
        .iter()
        .map(|value| (value.work_item_revision_id.as_str(), value))
        .collect::<BTreeMap<_, _>>();
    let mut work_items = Vec::new();
    for revision in &prepared.revised_work_items {
        let logical_id = revision.logical_work_item_id.as_str();
        work_items.push(PlanAmendmentWorkItemArtifacts {
            logical_work_item: store
                .get_logical_work_item(plan, logical_id)
                .map_err(PlanRepairError::Store)?,
            draft_revision: (*drafts
                .get(logical_id)
                .ok_or_else(|| invalid_publication("candidate draft is missing"))?)
            .clone(),
            work_item_revision: revision.clone(),
            verification_plan_revision: (*verifications
                .get(logical_id)
                .ok_or_else(|| invalid_publication("verification revision is missing"))?)
            .clone(),
            projection_bundle: (*bundles
                .get(revision.id.as_str())
                .ok_or_else(|| invalid_publication("projection bundle is missing"))?)
            .clone(),
        });
    }
    Ok(PlanAmendmentPublicationSnapshot {
        lineage: plan.clone(),
        plan_revision: prepared.next_plan_revision.clone(),
        dependency_graph_revision: prepared.dependency_graph_revision.clone(),
        validation_report: prepared.validation_report.clone(),
        plan_projection_bundle: prepared.plan_projection_bundle.clone(),
        work_items,
        manifest,
    })
}

fn artifact_fingerprint(
    snapshot: &PlanAmendmentPublicationSnapshot,
    confirmation: &PlanAmendmentConfirmation,
) -> Result<String, PlanRepairError> {
    let bytes = serde_json::to_vec(&(snapshot, confirmation)).map_err(|error| {
        invalid_publication(&format!("serialize publication snapshot: {error}"))
    })?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn sorted_unique(values: &[String]) -> Vec<String> {
    values
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn invalid_publication(message: &str) -> PlanRepairError {
    PlanRepairError::InvalidRepairTarget(format!("invalid plan amendment publication: {message}"))
}
