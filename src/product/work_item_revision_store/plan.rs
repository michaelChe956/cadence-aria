use chrono::Utc;

use crate::product::json_store::{ProductStoreError, validate_relative_id, write_json};
use crate::product::models::{WorkItemPlanLineage, WorkItemPlanRevision};

use super::{
    PlanScope, WorkItemRevisionStore, identity_mismatch, read_required_json, write_immutable,
};

impl WorkItemRevisionStore {
    pub fn put_plan_lineage(&self, value: &WorkItemPlanLineage) -> Result<(), ProductStoreError> {
        validate_plan_lineage(value)?;
        let scope = PlanScope {
            plan_id: value.id.clone(),
            project_id: value.project_id.clone(),
            issue_id: value.issue_id.clone(),
        };
        write_immutable(
            &self.plan_scope_path(&value.id, &value.project_id, &value.issue_id),
            "work_item_plan_scope",
            &value.id,
            &scope,
        )?;
        write_immutable(
            &self.plan_lineage_path(&value.project_id, &value.issue_id, &value.id),
            "work_item_plan_lineage",
            &value.id,
            value,
        )
    }

    pub fn get_plan_lineage(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<WorkItemPlanLineage, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(plan_id)?;
        let value: WorkItemPlanLineage = read_required_json(
            &self.plan_lineage_path(project_id, issue_id, plan_id),
            "work_item_plan_lineage",
            plan_id,
        )?;
        if value.project_id != project_id || value.issue_id != issue_id || value.id != plan_id {
            return Err(identity_mismatch("work_item_plan_lineage", plan_id));
        }
        Ok(value)
    }

    pub fn put_plan_revision(
        &self,
        lineage: &WorkItemPlanLineage,
        value: &WorkItemPlanRevision,
    ) -> Result<(), ProductStoreError> {
        self.ensure_plan_scope(lineage)?;
        validate_relative_id(&value.id)?;
        validate_relative_id(&value.plan_id)?;
        if value.plan_id != lineage.id {
            return Err(identity_mismatch("work_item_plan_revision", &value.id));
        }
        write_immutable(
            &self.plan_revision_path(
                &lineage.project_id,
                &lineage.issue_id,
                &lineage.id,
                &value.id,
            ),
            "work_item_plan_revision",
            &value.id,
            value,
        )
    }

    pub fn get_plan_revision(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        revision_id: &str,
    ) -> Result<WorkItemPlanRevision, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(plan_id)?;
        validate_relative_id(revision_id)?;
        self.get_plan_lineage(project_id, issue_id, plan_id)?;
        let value: WorkItemPlanRevision = read_required_json(
            &self.plan_revision_path(project_id, issue_id, plan_id, revision_id),
            "work_item_plan_revision",
            revision_id,
        )?;
        if value.id != revision_id || value.plan_id != plan_id {
            return Err(identity_mismatch("work_item_plan_revision", revision_id));
        }
        Ok(value)
    }

    pub fn set_active_plan_revision(
        &self,
        lineage: &WorkItemPlanLineage,
        revision_id: &str,
    ) -> Result<WorkItemPlanLineage, ProductStoreError> {
        let mut stored = self.ensure_plan_scope(lineage)?;
        self.get_plan_revision(
            &lineage.project_id,
            &lineage.issue_id,
            &lineage.id,
            revision_id,
        )?;
        stored.active_revision_id = Some(revision_id.to_string());
        stored.updated_at = Utc::now().to_rfc3339();
        write_json(
            &self.plan_lineage_path(&stored.project_id, &stored.issue_id, &stored.id),
            &stored,
        )?;
        Ok(stored)
    }

    pub fn compare_and_set_active_plan_revision(
        &self,
        lineage: &WorkItemPlanLineage,
        expected_revision_id: &str,
        next_revision_id: &str,
    ) -> Result<WorkItemPlanLineage, ProductStoreError> {
        validate_relative_id(expected_revision_id)?;
        let stored = self.ensure_plan_scope(lineage)?;
        if stored.active_revision_id.as_deref() != Some(expected_revision_id) {
            return Err(identity_mismatch(
                "active_work_item_plan_revision",
                &lineage.id,
            ));
        }
        self.set_active_plan_revision(&stored, next_revision_id)
    }

    pub fn acquire_active_amendment(
        &self,
        lineage: &WorkItemPlanLineage,
        amendment_id: &str,
    ) -> Result<WorkItemPlanLineage, ProductStoreError> {
        validate_relative_id(amendment_id)?;
        let mut stored = self.ensure_plan_scope(lineage)?;
        match stored.active_amendment_id.as_deref() {
            Some(active) if active == amendment_id => return Ok(stored),
            Some(_) => {
                return Err(identity_mismatch("active_plan_amendment", &lineage.id));
            }
            None => {}
        }
        stored.active_amendment_id = Some(amendment_id.to_string());
        stored.updated_at = Utc::now().to_rfc3339();
        write_json(
            &self.plan_lineage_path(&stored.project_id, &stored.issue_id, &stored.id),
            &stored,
        )?;
        Ok(stored)
    }

    pub fn release_active_amendment(
        &self,
        lineage: &WorkItemPlanLineage,
        amendment_id: &str,
    ) -> Result<WorkItemPlanLineage, ProductStoreError> {
        validate_relative_id(amendment_id)?;
        let mut stored = self.ensure_plan_scope(lineage)?;
        if stored.active_amendment_id.as_deref() != Some(amendment_id) {
            return Err(identity_mismatch("active_plan_amendment", &lineage.id));
        }
        stored.active_amendment_id = None;
        stored.updated_at = Utc::now().to_rfc3339();
        write_json(
            &self.plan_lineage_path(&stored.project_id, &stored.issue_id, &stored.id),
            &stored,
        )?;
        Ok(stored)
    }
}

fn validate_plan_lineage(value: &WorkItemPlanLineage) -> Result<(), ProductStoreError> {
    validate_relative_id(&value.id)?;
    validate_relative_id(&value.project_id)?;
    validate_relative_id(&value.issue_id)?;
    for reference in &value.story_spec_refs {
        validate_relative_id(reference)?;
    }
    for reference in &value.design_spec_refs {
        validate_relative_id(reference)?;
    }
    if let Some(value) = value.active_revision_id.as_deref() {
        validate_relative_id(value)?;
    }
    if let Some(value) = value.active_amendment_id.as_deref() {
        validate_relative_id(value)?;
    }
    Ok(())
}
