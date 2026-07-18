use chrono::Utc;

use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{
    PlanAmendmentManifest, PlanAmendmentPublicationJournal, PlanAmendmentPublicationPhase,
    PlanDefectEvidence, PlanRepairRequest, PlanRepairRequestStatus, PlanRepairReviewAttestation,
    WorkItemPlanLineage,
};

use super::{
    WorkItemRevisionStore, identity_mismatch, json_file_paths, read_required_json,
    with_exclusive_lock, write_immutable,
};

impl WorkItemRevisionStore {
    pub fn put_repair_request(
        &self,
        plan: &WorkItemPlanLineage,
        value: &PlanRepairRequest,
    ) -> Result<(), ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(&value.id)?;
        validate_relative_id(&value.plan_id)?;
        if value.plan_id != plan.id {
            return Err(identity_mismatch("plan_repair_request", &value.id));
        }
        write_immutable(
            &self.repair_request_path(&plan.project_id, &plan.issue_id, &plan.id, &value.id),
            "plan_repair_request",
            &value.id,
            value,
        )
    }

    pub fn update_repair_request_status(
        &self,
        plan: &WorkItemPlanLineage,
        request_id: &str,
        status: PlanRepairRequestStatus,
    ) -> Result<PlanRepairRequest, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(request_id)?;
        let path = self.repair_request_path(&plan.project_id, &plan.issue_id, &plan.id, request_id);
        with_exclusive_lock(&path, || {
            let mut request = self.get_repair_request(plan, request_id)?;
            if request.status == status {
                return Ok(request);
            }
            request.status = status;
            request.updated_at = Utc::now().to_rfc3339();
            write_json(&path, &request)?;
            Ok(request)
        })
    }

    pub fn transition_orphan_repair_request_to_in_progress(
        &self,
        plan: &WorkItemPlanLineage,
        request_id: &str,
    ) -> Result<PlanRepairRequest, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(request_id)?;
        let path = self.repair_request_path(&plan.project_id, &plan.issue_id, &plan.id, request_id);
        with_exclusive_lock(&path, || {
            let mut request = self.get_repair_request(plan, request_id)?;
            match request.status {
                PlanRepairRequestStatus::Open => {
                    request.status = PlanRepairRequestStatus::InProgress;
                    request.updated_at = Utc::now().to_rfc3339();
                    write_json(&path, &request)?;
                    Ok(request)
                }
                PlanRepairRequestStatus::InProgress => Ok(request),
                _ => Err(identity_mismatch(
                    "plan_repair_request_orphan_transition",
                    request_id,
                )),
            }
        })
    }

    pub fn transition_repair_request_to_awaiting_confirmation(
        &self,
        plan: &WorkItemPlanLineage,
        request_id: &str,
    ) -> Result<PlanRepairRequest, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(request_id)?;
        let path = self.repair_request_path(&plan.project_id, &plan.issue_id, &plan.id, request_id);
        with_exclusive_lock(&path, || {
            let mut request = self.get_repair_request(plan, request_id)?;
            match request.status {
                PlanRepairRequestStatus::InProgress => {
                    request.status = PlanRepairRequestStatus::AwaitingConfirmation;
                    request.updated_at = Utc::now().to_rfc3339();
                    write_json(&path, &request)?;
                    Ok(request)
                }
                PlanRepairRequestStatus::AwaitingConfirmation => Ok(request),
                _ => Err(identity_mismatch(
                    "plan_repair_request_awaiting_transition",
                    request_id,
                )),
            }
        })
    }

    pub fn ensure_repair_request_can_enter_awaiting_confirmation(
        &self,
        plan: &WorkItemPlanLineage,
        request_id: &str,
    ) -> Result<PlanRepairRequest, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(request_id)?;
        let path = self.repair_request_path(&plan.project_id, &plan.issue_id, &plan.id, request_id);
        with_exclusive_lock(&path, || {
            let request = self.get_repair_request(plan, request_id)?;
            if matches!(
                request.status,
                PlanRepairRequestStatus::InProgress | PlanRepairRequestStatus::AwaitingConfirmation
            ) {
                Ok(request)
            } else {
                Err(identity_mismatch(
                    "plan_repair_request_awaiting_guard",
                    request_id,
                ))
            }
        })
    }

    pub fn confirm_repair_request_awaiting_confirmation(
        &self,
        plan: &WorkItemPlanLineage,
        request_id: &str,
    ) -> Result<PlanRepairRequest, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(request_id)?;
        let path = self.repair_request_path(&plan.project_id, &plan.issue_id, &plan.id, request_id);
        with_exclusive_lock(&path, || {
            let request = self.get_repair_request(plan, request_id)?;
            if request.status == PlanRepairRequestStatus::AwaitingConfirmation {
                Ok(request)
            } else {
                Err(identity_mismatch(
                    "plan_repair_request_confirm_transition",
                    request_id,
                ))
            }
        })
    }

    pub fn assign_repair_request_amendment(
        &self,
        plan: &WorkItemPlanLineage,
        request_id: &str,
        amendment_id: &str,
    ) -> Result<PlanRepairRequest, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(request_id)?;
        validate_relative_id(amendment_id)?;
        let path = self.repair_request_path(&plan.project_id, &plan.issue_id, &plan.id, request_id);
        with_exclusive_lock(&path, || {
            let mut request = self.get_repair_request(plan, request_id)?;
            match request.amendment_id.as_deref() {
                Some(existing) if existing == amendment_id => return Ok(request),
                Some(_) => {
                    return Err(identity_mismatch(
                        "plan_repair_request_amendment",
                        request_id,
                    ));
                }
                None => {}
            }
            request.amendment_id = Some(amendment_id.to_string());
            request.updated_at = Utc::now().to_rfc3339();
            write_json(&path, &request)?;
            Ok(request)
        })
    }

    pub fn merge_repair_request_evidence(
        &self,
        plan: &WorkItemPlanLineage,
        request_id: &str,
        evidence: Vec<PlanDefectEvidence>,
    ) -> Result<PlanRepairRequest, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(request_id)?;
        let path = self.repair_request_path(&plan.project_id, &plan.issue_id, &plan.id, request_id);
        with_exclusive_lock(&path, || {
            let mut request = self.get_repair_request(plan, request_id)?;
            let mut changed = false;
            for value in evidence {
                if !request.evidence.contains(&value) {
                    request.evidence.push(value);
                    changed = true;
                }
            }
            if changed {
                request.updated_at = Utc::now().to_rfc3339();
                write_json(&path, &request)?;
            }
            Ok(request)
        })
    }

    pub fn list_open_repair_requests(
        &self,
        plan: &WorkItemPlanLineage,
    ) -> Result<Vec<PlanRepairRequest>, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        let mut requests = Vec::new();
        for path in
            json_file_paths(&self.repair_requests_root(&plan.project_id, &plan.issue_id, &plan.id))?
        {
            let request: PlanRepairRequest = read_json(&path)?;
            let file_id = path
                .file_stem()
                .and_then(|value| value.to_str())
                .ok_or_else(|| {
                    ProductStoreError::Io(format!(
                        "invalid repair request path: {}",
                        path.display()
                    ))
                })?;
            if request.id != file_id || request.plan_id != plan.id {
                return Err(identity_mismatch("plan_repair_request", file_id));
            }
            if is_open_status(&request.status) {
                requests.push(request);
            }
        }
        requests.sort_by(|left, right| {
            (left.created_at.as_str(), left.id.as_str())
                .cmp(&(right.created_at.as_str(), right.id.as_str()))
        });
        Ok(requests)
    }

    pub fn put_amendment_manifest(
        &self,
        plan: &WorkItemPlanLineage,
        value: &PlanAmendmentManifest,
    ) -> Result<(), ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(&value.id)?;
        validate_relative_id(&value.repair_request_id)?;
        write_immutable(
            &self.amendment_manifest_path(&plan.project_id, &plan.issue_id, &plan.id, &value.id),
            "plan_amendment_manifest",
            &value.id,
            value,
        )
    }

    pub fn get_amendment_manifest(
        &self,
        plan: &WorkItemPlanLineage,
        amendment_id: &str,
    ) -> Result<PlanAmendmentManifest, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(amendment_id)?;
        let value: PlanAmendmentManifest = read_required_json(
            &self.amendment_manifest_path(&plan.project_id, &plan.issue_id, &plan.id, amendment_id),
            "plan_amendment_manifest",
            amendment_id,
        )?;
        if value.id != amendment_id {
            return Err(identity_mismatch("plan_amendment_manifest", amendment_id));
        }
        Ok(value)
    }

    pub fn put_plan_repair_review_attestation(
        &self,
        plan: &WorkItemPlanLineage,
        value: &PlanRepairReviewAttestation,
    ) -> Result<(), ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        for id in [
            &value.id,
            &value.request_id,
            &value.amendment_id,
            &value.plan_id,
            &value.base_plan_revision_id,
            &value.reviewed_plan_revision_id,
            &value.plan_projection_bundle_id,
            &value.generation_round_id,
        ] {
            validate_relative_id(id)?;
        }
        if value.plan_id != plan.id || value.review.generation_round_id != value.generation_round_id
        {
            return Err(identity_mismatch(
                "plan_repair_review_attestation",
                &value.id,
            ));
        }
        write_immutable(
            &self.plan_repair_review_attestation_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                &value.id,
            ),
            "plan_repair_review_attestation",
            &value.id,
            value,
        )
    }

    pub fn get_plan_repair_review_attestation(
        &self,
        plan: &WorkItemPlanLineage,
        attestation_id: &str,
    ) -> Result<PlanRepairReviewAttestation, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(attestation_id)?;
        let value: PlanRepairReviewAttestation = read_required_json(
            &self.plan_repair_review_attestation_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                attestation_id,
            ),
            "plan_repair_review_attestation",
            attestation_id,
        )?;
        if value.id != attestation_id
            || value.plan_id != plan.id
            || value.review.generation_round_id != value.generation_round_id
        {
            return Err(identity_mismatch(
                "plan_repair_review_attestation",
                attestation_id,
            ));
        }
        Ok(value)
    }

    pub fn put_plan_amendment_publication_journal(
        &self,
        plan: &WorkItemPlanLineage,
        value: &PlanAmendmentPublicationJournal,
    ) -> Result<(), ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(&value.id)?;
        validate_relative_id(&value.plan_id)?;
        validate_relative_id(&value.amendment_id)?;
        if value.plan_id != plan.id {
            return Err(identity_mismatch(
                "plan_amendment_publication_journal",
                &value.id,
            ));
        }
        write_immutable(
            &self.amendment_publication_journal_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                &value.id,
            ),
            "plan_amendment_publication_journal",
            &value.id,
            value,
        )
    }

    pub fn advance_plan_amendment_publication(
        &self,
        plan: &WorkItemPlanLineage,
        amendment_id: &str,
        next: PlanAmendmentPublicationPhase,
    ) -> Result<PlanAmendmentPublicationJournal, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(amendment_id)?;
        let path = self.amendment_publication_journal_path(
            &plan.project_id,
            &plan.issue_id,
            &plan.id,
            amendment_id,
        );
        with_exclusive_lock(&path, || {
            let mut journal = self.get_plan_amendment_publication_journal(plan, amendment_id)?;
            if journal.phase == next {
                return Ok(journal);
            }
            if journal.phase == PlanAmendmentPublicationPhase::PlanPublished
                || next.order() <= journal.phase.order()
            {
                return Err(ProductStoreError::Io(format!(
                    "amendment_phase_regression: {:?} -> {:?}",
                    journal.phase, next
                )));
            }
            journal.phase = next;
            journal.error = None;
            journal.updated_at = Utc::now().to_rfc3339();
            write_json(&path, &journal)?;
            Ok(journal)
        })
    }

    pub fn mark_plan_amendment_publication_failed(
        &self,
        plan: &WorkItemPlanLineage,
        amendment_id: &str,
        error: String,
    ) -> Result<PlanAmendmentPublicationJournal, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(amendment_id)?;
        let path = self.amendment_publication_journal_path(
            &plan.project_id,
            &plan.issue_id,
            &plan.id,
            amendment_id,
        );
        with_exclusive_lock(&path, || {
            let mut journal = self.get_plan_amendment_publication_journal(plan, amendment_id)?;
            journal.error = Some(error);
            journal.updated_at = Utc::now().to_rfc3339();
            write_json(&path, &journal)?;
            Ok(journal)
        })
    }

    pub fn get_plan_amendment_publication_journal(
        &self,
        plan: &WorkItemPlanLineage,
        amendment_id: &str,
    ) -> Result<PlanAmendmentPublicationJournal, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(amendment_id)?;
        let value: PlanAmendmentPublicationJournal = read_required_json(
            &self.amendment_publication_journal_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                amendment_id,
            ),
            "plan_amendment_publication_journal",
            amendment_id,
        )?;
        if value.id != amendment_id || value.plan_id != plan.id {
            return Err(identity_mismatch(
                "plan_amendment_publication_journal",
                amendment_id,
            ));
        }
        Ok(value)
    }

    pub fn find_plan_amendment_publication_journal(
        &self,
        plan: &WorkItemPlanLineage,
        amendment_id: &str,
    ) -> Result<Option<PlanAmendmentPublicationJournal>, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(amendment_id)?;
        let mut matched = None;
        for path in json_file_paths(&self.amendment_publication_journals_root(
            &plan.project_id,
            &plan.issue_id,
            &plan.id,
        ))? {
            let value: PlanAmendmentPublicationJournal = read_json(&path)?;
            let file_id = path
                .file_stem()
                .and_then(|value| value.to_str())
                .ok_or_else(|| {
                    ProductStoreError::Io(format!(
                        "invalid amendment publication journal path: {}",
                        path.display()
                    ))
                })?;
            if value.id != file_id || value.plan_id != plan.id {
                return Err(identity_mismatch(
                    "plan_amendment_publication_journal",
                    file_id,
                ));
            }
            if value.amendment_id != amendment_id {
                continue;
            }
            if matched.is_some() {
                return Err(ProductStoreError::Ambiguous {
                    kind: "plan_amendment_publication_journal",
                    id: amendment_id.to_string(),
                });
            }
            matched = Some(value);
        }
        Ok(matched)
    }

    pub fn get_repair_request(
        &self,
        plan: &WorkItemPlanLineage,
        request_id: &str,
    ) -> Result<PlanRepairRequest, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(request_id)?;
        let value: PlanRepairRequest = read_required_json(
            &self.repair_request_path(&plan.project_id, &plan.issue_id, &plan.id, request_id),
            "plan_repair_request",
            request_id,
        )?;
        if value.id != request_id || value.plan_id != plan.id {
            return Err(identity_mismatch("plan_repair_request", request_id));
        }
        Ok(value)
    }
}

impl PlanAmendmentPublicationPhase {
    fn order(&self) -> u8 {
        match self {
            Self::Prepared => 0,
            Self::PlanPublished => 1,
        }
    }
}

fn is_open_status(status: &PlanRepairRequestStatus) -> bool {
    matches!(
        status,
        PlanRepairRequestStatus::Open
            | PlanRepairRequestStatus::InProgress
            | PlanRepairRequestStatus::AwaitingConfirmation
    )
}
