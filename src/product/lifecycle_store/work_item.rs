use chrono::Utc;
use std::path::PathBuf;

use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{
    LifecycleWorkItemRecord, WorkItemExecutionPlanStatus, WorkItemPlanStatus, WorkItemStatus,
    WorkspaceType,
};

use super::{
    CreateWorkItemInput, LifecycleStore, count_json_files, delete_required_file, list_json_records,
    validate_relative_ids,
};

impl LifecycleStore {
    pub fn create_work_item(
        &self,
        input: CreateWorkItemInput,
    ) -> Result<LifecycleWorkItemRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.repository_id)?;
        validate_relative_ids(&input.story_spec_ids)?;
        validate_relative_ids(&input.design_spec_ids)?;

        let root = self.work_items_root(&input.project_id, &input.issue_id);
        let id = match input.id {
            Some(ref id) => {
                validate_relative_id(id)?;
                id.clone()
            }
            None => next_sequential_id("work_item", count_json_files(&root)?),
        };
        let now = Utc::now().to_rfc3339();
        let work_item = LifecycleWorkItemRecord {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            repository_id: input.repository_id,
            story_spec_ids: input.story_spec_ids,
            design_spec_ids: input.design_spec_ids,
            title: input.title,
            plan_status: input.plan_status,
            execution_status: WorkItemStatus::Pending,
            worktree_path: None,
            work_item_set_id: input.work_item_set_id,
            source_work_item_plan_id: input.source_work_item_plan_id,
            source_outline_id: input.source_outline_id,
            source_draft_id: input.source_draft_id,
            planned_implementation_context: input.planned_implementation_context,
            planned_handoff_summary: input.planned_handoff_summary,
            kind: input.kind,
            sequence_hint: input.sequence_hint,
            depends_on: input.depends_on,
            exclusive_write_scopes: input.exclusive_write_scopes,
            forbidden_write_scopes: input.forbidden_write_scopes,
            context_budget: input.context_budget,
            required_handoff_from: input.required_handoff_from,
            verification_plan_ref: input.verification_plan_ref,
            require_execution_plan_confirm: input.require_execution_plan_confirm,
            execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
            handoff_summary_ref: None,
            completion_commit: None,
            completion_diff_summary_ref: None,
            created_at: now.clone(),
            updated_at: now,
        };

        write_json(&root.join(format!("{id}.json")), &work_item)?;
        Ok(work_item)
    }

    pub fn count_work_items(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<usize, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        count_json_files(&self.work_items_root(project_id, issue_id))
    }

    pub fn list_work_items(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<LifecycleWorkItemRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        list_json_records(&self.work_items_root(project_id, issue_id))
    }

    pub fn delete_work_item(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;

        delete_required_file(
            &self
                .work_items_root(project_id, issue_id)
                .join(format!("{work_item_id}.json")),
            "work_item",
            work_item_id,
        )?;
        self.delete_workspace_sessions_for_entity(
            project_id,
            issue_id,
            work_item_id,
            WorkspaceType::WorkItem,
        )
    }

    pub fn update_work_item_plan_status(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
        plan_status: WorkItemPlanStatus,
    ) -> Result<LifecycleWorkItemRecord, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;

        let path = self
            .work_items_root(project_id, issue_id)
            .join(format!("{work_item_id}.json"));
        if !super::path_is_regular_file(&path)? {
            return Err(ProductStoreError::NotFound {
                kind: "work_item",
                id: work_item_id.to_string(),
            });
        }

        let mut record: LifecycleWorkItemRecord = read_json(&path)?;
        record.plan_status = plan_status;
        record.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &record)?;
        Ok(record)
    }

    pub fn update_work_item_execution_status(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
        execution_status: WorkItemStatus,
    ) -> Result<LifecycleWorkItemRecord, ProductStoreError> {
        let path = self.work_item_path(project_id, issue_id, work_item_id)?;
        let mut record: LifecycleWorkItemRecord = read_json(&path)?;
        record.execution_status = execution_status;
        record.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &record)?;
        Ok(record)
    }

    pub fn update_work_item_execution_plan_status(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
        execution_plan_status: WorkItemExecutionPlanStatus,
    ) -> Result<LifecycleWorkItemRecord, ProductStoreError> {
        let path = self.work_item_path(project_id, issue_id, work_item_id)?;
        let mut record: LifecycleWorkItemRecord = read_json(&path)?;
        record.execution_plan_status = execution_plan_status;
        record.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &record)?;
        Ok(record)
    }

    pub fn update_work_item_handoff_summary(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
        handoff_summary_ref: Option<String>,
        completion_commit: Option<String>,
    ) -> Result<LifecycleWorkItemRecord, ProductStoreError> {
        let path = self.work_item_path(project_id, issue_id, work_item_id)?;
        let mut record: LifecycleWorkItemRecord = read_json(&path)?;
        record.handoff_summary_ref = handoff_summary_ref;
        record.completion_commit = completion_commit;
        record.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &record)?;
        Ok(record)
    }

    pub fn update_work_item_worktree_path(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
        worktree_path: Option<PathBuf>,
    ) -> Result<LifecycleWorkItemRecord, ProductStoreError> {
        let path = self.work_item_path(project_id, issue_id, work_item_id)?;
        let mut record: LifecycleWorkItemRecord = read_json(&path)?;
        record.worktree_path = worktree_path;
        record.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &record)?;
        Ok(record)
    }
}
