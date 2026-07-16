use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::models::{
    PlanProjectionBundle, PlanValidationReportArtifact, WorkItemPlanLineage,
    WorkItemProjectionBundle,
};

use super::{WorkItemRevisionStore, identity_mismatch, read_required_json, write_immutable};

impl WorkItemRevisionStore {
    pub fn put_plan_validation_report(
        &self,
        plan: &WorkItemPlanLineage,
        value: &PlanValidationReportArtifact,
    ) -> Result<(), ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(&value.id)?;
        validate_relative_id(&value.plan_id)?;
        if value.plan_id != plan.id {
            return Err(identity_mismatch("plan_validation_report", &value.id));
        }
        write_immutable(
            &self.plan_validation_report_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                &value.id,
            ),
            "plan_validation_report",
            &value.id,
            value,
        )
    }

    pub fn get_plan_validation_report(
        &self,
        plan: &WorkItemPlanLineage,
        report_id: &str,
    ) -> Result<PlanValidationReportArtifact, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(report_id)?;
        let value: PlanValidationReportArtifact = read_required_json(
            &self.plan_validation_report_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                report_id,
            ),
            "plan_validation_report",
            report_id,
        )?;
        if value.id != report_id || value.plan_id != plan.id {
            return Err(identity_mismatch("plan_validation_report", report_id));
        }
        Ok(value)
    }

    pub fn put_work_item_projection_bundle(
        &self,
        plan: &WorkItemPlanLineage,
        value: &WorkItemProjectionBundle,
    ) -> Result<(), ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(&value.id)?;
        validate_relative_id(&value.work_item_revision_id)?;
        write_immutable(
            &self.work_item_projection_bundle_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                &value.id,
            ),
            "work_item_projection_bundle",
            &value.id,
            value,
        )
    }

    pub fn get_work_item_projection_bundle(
        &self,
        plan: &WorkItemPlanLineage,
        bundle_id: &str,
    ) -> Result<WorkItemProjectionBundle, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(bundle_id)?;
        let value: WorkItemProjectionBundle = read_required_json(
            &self.work_item_projection_bundle_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                bundle_id,
            ),
            "work_item_projection_bundle",
            bundle_id,
        )?;
        if value.id != bundle_id {
            return Err(identity_mismatch("work_item_projection_bundle", bundle_id));
        }
        Ok(value)
    }

    pub fn put_plan_projection_bundle(
        &self,
        plan: &WorkItemPlanLineage,
        value: &PlanProjectionBundle,
    ) -> Result<(), ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(&value.id)?;
        validate_relative_id(&value.plan_revision_id)?;
        write_immutable(
            &self.plan_projection_bundle_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                &value.id,
            ),
            "plan_projection_bundle",
            &value.id,
            value,
        )
    }

    pub fn get_plan_projection_bundle(
        &self,
        plan: &WorkItemPlanLineage,
        bundle_id: &str,
    ) -> Result<PlanProjectionBundle, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(bundle_id)?;
        let value: PlanProjectionBundle = read_required_json(
            &self.plan_projection_bundle_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                bundle_id,
            ),
            "plan_projection_bundle",
            bundle_id,
        )?;
        if value.id != bundle_id {
            return Err(identity_mismatch("plan_projection_bundle", bundle_id));
        }
        Ok(value)
    }
}
