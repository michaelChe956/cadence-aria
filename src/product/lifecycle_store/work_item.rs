use chrono::Utc;
use std::path::PathBuf;

use crate::product::id::next_sequential_id_in_directory;
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
            None => next_sequential_id_in_directory("work_item", &root).map_err(|error| {
                ProductStoreError::Io(format!("read {}: {error}", root.display()))
            })?,
        };
        let now = Utc::now().to_rfc3339();
        let work_item = LifecycleWorkItemRecord {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            repository_id: input.repository_id,
            target_repository_id: None,
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
            kind: input.kind,
            sequence_hint: input.sequence_hint,
            depends_on: input.depends_on,
            exclusive_write_scopes: input.exclusive_write_scopes,
            forbidden_write_scopes: input.forbidden_write_scopes,
            context_budget: input.context_budget,
            verification_plan_ref: input.verification_plan_ref,
            require_execution_plan_confirm: input.require_execution_plan_confirm,
            execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
            completion_commit: None,
            completion_diff_summary_ref: None,
            created_at: now.clone(),
            updated_at: now,
        };

        write_json(&root.join(format!("{id}.json")), &work_item)?;
        Ok(work_item)
    }

    /// park-1：compile finalize 的 per-target work item 记录写入点。
    ///
    /// 键域 (project, issue, record.id)。已存在时仅覆盖 compile 派生归属
    /// 字段，保留执行态（execution_status/worktree_path/execution_plan_status/
    /// completion_*）与首次 created_at——重复 finalize / 恢复重放 / 再编译
    /// 均不重复建、不清执行进度；新建时执行态按 Pending 初始。
    pub fn upsert_compiled_work_item_record(
        &self,
        record: LifecycleWorkItemRecord,
    ) -> Result<LifecycleWorkItemRecord, ProductStoreError> {
        validate_relative_id(&record.project_id)?;
        validate_relative_id(&record.issue_id)?;
        validate_relative_id(&record.id)?;
        validate_relative_id(&record.repository_id)?;

        let path = self
            .work_items_root(&record.project_id, &record.issue_id)
            .join(format!("{}.json", record.id));
        crate::product::coding_attempt_store::locking::with_exclusive_lock(&path, || {
            let mut persisted = if super::utils::path_is_regular_file(&path)? {
                let mut existing: LifecycleWorkItemRecord = read_json(&path)?;
                existing.repository_id = record.repository_id.clone();
                existing.target_repository_id = record.target_repository_id;
                existing.story_spec_ids = record.story_spec_ids.clone();
                existing.design_spec_ids = record.design_spec_ids.clone();
                existing.title = record.title.clone();
                existing.plan_status = record.plan_status.clone();
                existing.work_item_set_id = record.work_item_set_id.clone();
                existing.source_work_item_plan_id = record.source_work_item_plan_id.clone();
                existing.source_outline_id = record.source_outline_id.clone();
                existing.source_draft_id = record.source_draft_id.clone();
                existing.planned_implementation_context =
                    record.planned_implementation_context.clone();
                existing.kind = record.kind.clone();
                existing.sequence_hint = record.sequence_hint;
                existing.depends_on = record.depends_on.clone();
                existing.exclusive_write_scopes = record.exclusive_write_scopes.clone();
                existing.forbidden_write_scopes = record.forbidden_write_scopes.clone();
                existing.context_budget = record.context_budget.clone();
                existing.verification_plan_ref = record.verification_plan_ref.clone();
                existing.require_execution_plan_confirm = record.require_execution_plan_confirm;
                existing
            } else {
                record
            };
            persisted.updated_at = Utc::now().to_rfc3339();
            write_json(&path, &persisted)?;
            Ok(persisted)
        })
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

    /// 写入 work item 的完成 commit。
    ///
    /// 该函数是 `completion_commit` 在全仓的唯一写入点，其消费者独立于已移除的
    /// 交接摘要（依赖交接引用展示），因此不随交接摘要一并删除。
    pub fn update_work_item_completion_commit(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
        completion_commit: Option<String>,
    ) -> Result<LifecycleWorkItemRecord, ProductStoreError> {
        let path = self.work_item_path(project_id, issue_id, work_item_id)?;
        let mut record: LifecycleWorkItemRecord = read_json(&path)?;
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
