use chrono::Utc;

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(test)]
use std::sync::{Mutex, OnceLock};

use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{
    PlanAmendmentManifest, PlanDefectEvidence, PlanRepairRequest, PlanRepairRequestStatus,
    PlanRepairReviewAttestation, WorkItemPlanLineage,
};

use super::{
    WorkItemRevisionStore, identity_mismatch, json_file_paths, read_required_json,
    with_exclusive_lock, write_immutable,
};

#[cfg(test)]
struct RepairRequestStatusFailpointEntry {
    registration_id: u64,
    status: PlanRepairRequestStatus,
}

#[cfg(test)]
pub(crate) struct RepairRequestStatusFailpointGuard {
    request_path: PathBuf,
    registration_id: u64,
}

#[cfg(test)]
static REPAIR_REQUEST_STATUS_FAILPOINTS: OnceLock<
    Mutex<HashMap<PathBuf, RepairRequestStatusFailpointEntry>>,
> = OnceLock::new();
#[cfg(test)]
static NEXT_REPAIR_REQUEST_STATUS_FAILPOINT_ID: AtomicU64 = AtomicU64::new(1);

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
        #[cfg(test)]
        maybe_fail_repair_request_status(&path, &status)?;
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
        if value.plan_id != plan.id
            || value.review.generation_round_id != value.generation_round_id
            || value.candidate_package_fingerprint.trim().is_empty()
            || !is_sorted_unique(&value.accepted_impact_scope)
            || value
                .risk_acceptance_reason
                .as_deref()
                .is_some_and(|reason| reason.trim().is_empty())
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
            || value.candidate_package_fingerprint.trim().is_empty()
            || !is_sorted_unique(&value.accepted_impact_scope)
            || value
                .risk_acceptance_reason
                .as_deref()
                .is_some_and(|reason| reason.trim().is_empty())
        {
            return Err(identity_mismatch(
                "plan_repair_review_attestation",
                attestation_id,
            ));
        }
        Ok(value)
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

#[cfg(test)]
pub(crate) fn register_repair_request_status_failpoint(
    store: &WorkItemRevisionStore,
    plan: &WorkItemPlanLineage,
    request_id: &str,
    status: PlanRepairRequestStatus,
) -> RepairRequestStatusFailpointGuard {
    let request_path =
        store.repair_request_path(&plan.project_id, &plan.issue_id, &plan.id, request_id);
    let registration_id = NEXT_REPAIR_REQUEST_STATUS_FAILPOINT_ID.fetch_add(1, Ordering::Relaxed);
    let previous = repair_request_status_failpoints()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(
            request_path.clone(),
            RepairRequestStatusFailpointEntry {
                registration_id,
                status,
            },
        );
    assert!(
        previous.is_none(),
        "repair request status failpoint already registered"
    );
    RepairRequestStatusFailpointGuard {
        request_path,
        registration_id,
    }
}

#[cfg(test)]
fn repair_request_status_failpoints()
-> &'static Mutex<HashMap<PathBuf, RepairRequestStatusFailpointEntry>> {
    REPAIR_REQUEST_STATUS_FAILPOINTS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
fn maybe_fail_repair_request_status(
    request_path: &Path,
    status: &PlanRepairRequestStatus,
) -> Result<(), ProductStoreError> {
    let failpoints = repair_request_status_failpoints()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if failpoints
        .get(request_path)
        .is_some_and(|entry| &entry.status == status)
    {
        return Err(ProductStoreError::Io(format!(
            "repair_request_status_failpoint:{status:?}"
        )));
    }
    Ok(())
}

#[cfg(test)]
impl Drop for RepairRequestStatusFailpointGuard {
    fn drop(&mut self) {
        let mut failpoints = repair_request_status_failpoints()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if failpoints
            .get(&self.request_path)
            .is_some_and(|entry| entry.registration_id == self.registration_id)
        {
            failpoints.remove(&self.request_path);
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

fn is_sorted_unique(values: &[String]) -> bool {
    values.windows(2).all(|window| window[0] < window[1])
}
