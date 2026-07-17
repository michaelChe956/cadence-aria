use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::models::{HandoffRevision, WorkItemPlanLineage};

use super::{WorkItemRevisionStore, identity_mismatch, read_required_json, write_immutable};

impl WorkItemRevisionStore {
    pub fn put_handoff_revision(
        &self,
        plan: &WorkItemPlanLineage,
        value: &HandoffRevision,
    ) -> Result<(), ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(&value.id)?;
        validate_relative_id(&value.logical_work_item_id)?;
        self.get_logical_work_item(plan, &value.logical_work_item_id)?;
        write_immutable(
            &self.handoff_revision_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                &value.logical_work_item_id,
                &value.id,
            ),
            "handoff_revision",
            &value.id,
            value,
        )
    }

    pub fn get_handoff_revision(
        &self,
        plan: &WorkItemPlanLineage,
        logical_work_item_id: &str,
        handoff_revision_id: &str,
    ) -> Result<HandoffRevision, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(logical_work_item_id)?;
        validate_relative_id(handoff_revision_id)?;
        let value: HandoffRevision = read_required_json(
            &self.handoff_revision_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                logical_work_item_id,
                handoff_revision_id,
            ),
            "handoff_revision",
            handoff_revision_id,
        )?;
        if value.id != handoff_revision_id || value.logical_work_item_id != logical_work_item_id {
            return Err(identity_mismatch("handoff_revision", handoff_revision_id));
        }
        Ok(value)
    }
}
