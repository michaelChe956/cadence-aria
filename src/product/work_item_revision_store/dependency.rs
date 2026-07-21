use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::models::{DependencyGraphRevision, WorkItemPlanLineage};

use super::{WorkItemRevisionStore, identity_mismatch, read_required_json, write_immutable};

impl WorkItemRevisionStore {
    pub fn put_dependency_graph_revision(
        &self,
        plan: &WorkItemPlanLineage,
        value: &DependencyGraphRevision,
    ) -> Result<(), ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(&value.id)?;
        validate_relative_id(&value.plan_id)?;
        if value.plan_id != plan.id {
            return Err(identity_mismatch("dependency_graph_revision", &value.id));
        }
        write_immutable(
            &self.dependency_graph_revision_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                &value.id,
            ),
            "dependency_graph_revision",
            &value.id,
            value,
        )
    }

    pub fn get_dependency_graph_revision(
        &self,
        plan: &WorkItemPlanLineage,
        revision_id: &str,
    ) -> Result<DependencyGraphRevision, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(revision_id)?;
        let value: DependencyGraphRevision = read_required_json(
            &self.dependency_graph_revision_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                revision_id,
            ),
            "dependency_graph_revision",
            revision_id,
        )?;
        if value.id != revision_id || value.plan_id != plan.id {
            return Err(identity_mismatch("dependency_graph_revision", revision_id));
        }
        Ok(value)
    }
}
