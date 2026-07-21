use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::models::{PlanRepairCandidatePackageArtifact, WorkItemPlanLineage};

use super::{WorkItemRevisionStore, identity_mismatch, read_required_json, write_immutable};

impl WorkItemRevisionStore {
    pub fn put_plan_repair_candidate_package(
        &self,
        plan: &WorkItemPlanLineage,
        value: &PlanRepairCandidatePackageArtifact,
    ) -> Result<(), ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_candidate_package(plan, value)?;
        write_immutable(
            &self.plan_repair_candidate_package_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                &value.id,
            ),
            "plan_repair_candidate_package",
            &value.id,
            value,
        )
    }

    pub fn get_plan_repair_candidate_package(
        &self,
        plan: &WorkItemPlanLineage,
        package_id: &str,
    ) -> Result<PlanRepairCandidatePackageArtifact, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(package_id)?;
        let value: PlanRepairCandidatePackageArtifact = read_required_json(
            &self.plan_repair_candidate_package_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                package_id,
            ),
            "plan_repair_candidate_package",
            package_id,
        )?;
        validate_candidate_package(plan, &value)?;
        if value.id != package_id {
            return Err(identity_mismatch(
                "plan_repair_candidate_package",
                package_id,
            ));
        }
        Ok(value)
    }
}

fn validate_candidate_package(
    plan: &WorkItemPlanLineage,
    value: &PlanRepairCandidatePackageArtifact,
) -> Result<(), ProductStoreError> {
    for id in [
        &value.id,
        &value.project_id,
        &value.issue_id,
        &value.plan_id,
        &value.request_id,
        &value.amendment_id,
        &value.base_plan_revision_id,
        &value.new_plan_revision_id,
    ] {
        validate_relative_id(id)?;
    }
    if value.project_id != plan.project_id
        || value.issue_id != plan.issue_id
        || value.plan_id != plan.id
        || value.request.id != value.request_id
        || value.request.plan_id != value.plan_id
        || value.request.amendment_id.as_deref() != Some(value.amendment_id.as_str())
        || value.request.base_plan_revision_id != value.base_plan_revision_id
        || value.minimum_manifest.id != value.amendment_id
        || value.minimum_manifest.repair_request_id != value.request_id
        || value.minimum_manifest.previous_plan_revision_id != value.base_plan_revision_id
        || value.minimum_manifest.new_plan_revision_id != value.new_plan_revision_id
        || value.plan_projection_bundle.plan_revision_id != value.new_plan_revision_id
        || value.validation_report.plan_revision_id != value.new_plan_revision_id
        || value.validation_report.plan_projection_bundle_id != value.plan_projection_bundle.id
        || (value.request.repair_target.kind == crate::product::models::RepairTargetKind::Subgraph)
            != value.subgraph_replan.is_some()
        || value.candidate_package_fingerprint.trim().is_empty()
    {
        return Err(identity_mismatch(
            "plan_repair_candidate_package",
            &value.id,
        ));
    }
    Ok(())
}
