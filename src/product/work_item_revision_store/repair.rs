use chrono::Utc;

use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{
    PlanAmendmentManifest, PlanAmendmentPublicationJournal, PlanRepairRequest,
    PlanRepairRequestStatus, WorkItemPlanLineage,
};

use super::{
    WorkItemRevisionStore, identity_mismatch, json_file_paths, read_required_json, write_immutable,
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
        let mut request = self.get_repair_request(plan, request_id)?;
        if request.status == status {
            return Ok(request);
        }
        request.status = status;
        request.updated_at = Utc::now().to_rfc3339();
        write_json(
            &self.repair_request_path(&plan.project_id, &plan.issue_id, &plan.id, request_id),
            &request,
        )?;
        Ok(request)
    }

    pub fn merge_repair_request_evidence(
        &self,
        plan: &WorkItemPlanLineage,
        request_id: &str,
        evidence: Vec<serde_json::Value>,
    ) -> Result<PlanRepairRequest, ProductStoreError> {
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
            write_json(
                &self.repair_request_path(&plan.project_id, &plan.issue_id, &plan.id, request_id),
                &request,
            )?;
        }
        Ok(request)
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

    fn get_repair_request(
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

fn is_open_status(status: &PlanRepairRequestStatus) -> bool {
    matches!(
        status,
        PlanRepairRequestStatus::Open
            | PlanRepairRequestStatus::InProgress
            | PlanRepairRequestStatus::AwaitingConfirmation
    )
}
