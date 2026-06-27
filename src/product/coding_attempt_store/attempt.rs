use chrono::Utc;

use crate::product::coding_attempt_store::CreateCodingAttemptInput;
use crate::product::coding_models::{
    CodingAttemptScope, CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage,
    CodingRoleProviderConfigSnapshot,
};
use crate::product::coding_models::{WorkItemExecutionPlan, WorkItemHandoff};
use crate::product::id::next_sequential_id;
use crate::product::json_store::{
    ProductStoreError, read_json, validate_relative_artifact_ref, validate_relative_id, write_json,
};
use crate::product::models::{ProviderConversationRef, WorkItemExecutionPlanStatus};
use crate::web::workspace_ws_types::ProviderConfigSnapshot;

impl super::CodingAttemptStore {
    pub fn create_attempt(
        &self,
        input: CreateCodingAttemptInput,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.work_item_id)?;

        if let Some(active) =
            self.get_active_attempt(&input.project_id, &input.issue_id, &input.work_item_id)?
        {
            return Err(ProductStoreError::Io(format!(
                "active_coding_attempt_exists: {}",
                active.id
            )));
        }

        let root = self.coding_attempts_root(&input.project_id, &input.issue_id);
        let id = next_sequential_id("coding_attempt", super::count_json_files(&root)?);
        let attempt_no = self
            .list_attempts_for_work_item(&input.project_id, &input.issue_id, &input.work_item_id)?
            .iter()
            .map(|attempt| attempt.attempt_no)
            .max()
            .unwrap_or(0)
            + 1;
        let now = Utc::now().to_rfc3339();
        let current_work_item_id = input.work_item_id.clone();
        let attempt = CodingExecutionAttempt {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            work_item_id: input.work_item_id,
            attempt_no,
            scope: CodingAttemptScope::WorkItem,
            status: CodingAttemptStatus::Created,
            stage: CodingExecutionStage::PrepareContext,
            base_branch: input.base_branch,
            branch_name: input.branch_name,
            worktree_path: input.worktree_path,
            provider_config_snapshot: input.provider_config_snapshot,
            rework_count: 0,
            max_auto_rework: input.max_auto_rework,
            work_item_group_id: None,
            current_work_item_id: Some(current_work_item_id),
            active_unit_id: None,
            head_commit: None,
            pushed_remote: None,
            review_request_id: None,
            provider_conversations: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
            completed_at: None,
        };

        write_json(
            &self.attempt_path(&attempt.project_id, &attempt.issue_id, &id),
            &attempt,
        )?;
        write_json(
            &self.role_provider_config_path(&attempt.project_id, &attempt.issue_id, &id),
            &CodingRoleProviderConfigSnapshot::from(&attempt.provider_config_snapshot),
        )?;
        Ok(attempt)
    }

    pub fn save_coding_attempt(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&attempt.project_id)?;
        validate_relative_id(&attempt.issue_id)?;
        validate_relative_id(&attempt.id)?;
        write_json(
            &self.attempt_path(&attempt.project_id, &attempt.issue_id, &attempt.id),
            attempt,
        )?;
        write_json(
            &self.role_provider_config_path(&attempt.project_id, &attempt.issue_id, &attempt.id),
            &CodingRoleProviderConfigSnapshot::from(&attempt.provider_config_snapshot),
        )?;
        Ok(())
    }

    pub fn get_attempt(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        let path = self.attempt_path(project_id, issue_id, attempt_id);
        if !super::path_is_regular_file(&path)? {
            return Err(ProductStoreError::NotFound {
                kind: "coding_attempt",
                id: attempt_id.to_string(),
            });
        }
        read_json(&path)
    }

    pub fn save_work_item_execution_plan(
        &self,
        plan: &WorkItemExecutionPlan,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&plan.project_id)?;
        validate_relative_id(&plan.issue_id)?;
        validate_relative_id(&plan.attempt_id)?;
        write_json(
            &self.work_item_execution_plan_path(&plan.project_id, &plan.issue_id, &plan.attempt_id),
            plan,
        )
    }

    pub fn get_work_item_execution_plan(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Option<WorkItemExecutionPlan>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        let path = self.work_item_execution_plan_path(project_id, issue_id, attempt_id);
        if !super::path_is_regular_file(&path)? {
            return Ok(None);
        }
        read_json(&path).map(Some)
    }

    pub fn update_work_item_execution_plan_status(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        status: WorkItemExecutionPlanStatus,
    ) -> Result<WorkItemExecutionPlan, ProductStoreError> {
        let mut plan = self
            .get_work_item_execution_plan(project_id, issue_id, attempt_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "work_item_execution_plan",
                id: attempt_id.to_string(),
            })?;
        plan.status = status;
        plan.updated_at = Utc::now().to_rfc3339();
        self.save_work_item_execution_plan(&plan)?;
        Ok(plan)
    }

    pub fn save_work_item_handoff(
        &self,
        handoff: &WorkItemHandoff,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&handoff.project_id)?;
        validate_relative_id(&handoff.issue_id)?;
        validate_relative_id(&handoff.attempt_id)?;
        write_json(
            &self.work_item_handoff_path(
                &handoff.project_id,
                &handoff.issue_id,
                &handoff.attempt_id,
            ),
            handoff,
        )
    }

    pub fn save_coding_unit_handoff(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        unit_id: &str,
        handoff: &WorkItemHandoff,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        validate_relative_id(unit_id)?;
        write_json(
            &self.coding_unit_handoff_path(project_id, issue_id, attempt_id, unit_id),
            handoff,
        )
    }

    pub fn get_work_item_handoff(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Option<WorkItemHandoff>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        let path = self.work_item_handoff_path(project_id, issue_id, attempt_id);
        if !super::path_is_regular_file(&path)? {
            return Ok(None);
        }
        read_json(&path).map(Some)
    }

    pub fn get_coding_unit_handoff(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        unit_id: &str,
    ) -> Result<Option<WorkItemHandoff>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        validate_relative_id(unit_id)?;
        let path = self.coding_unit_handoff_path(project_id, issue_id, attempt_id, unit_id);
        if !super::path_is_regular_file(&path)? {
            return Ok(None);
        }
        read_json(&path).map(Some)
    }

    pub fn get_visible_work_item_handoff(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<Option<WorkItemHandoff>, ProductStoreError> {
        if attempt.scope != CodingAttemptScope::WorkItemGroup {
            return self.get_work_item_handoff(&attempt.project_id, &attempt.issue_id, &attempt.id);
        }
        let Some(active_unit) =
            self.get_active_coding_unit(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        else {
            return Ok(None);
        };
        self.get_coding_unit_handoff(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &active_unit.id,
        )
    }

    pub fn get_attempt_by_id(
        &self,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        self.find_attempt_by_id(attempt_id)
    }

    pub fn list_attempts_for_work_item(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<Vec<CodingExecutionAttempt>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;
        let mut attempts: Vec<CodingExecutionAttempt> =
            super::list_json_records(&self.coding_attempts_root(project_id, issue_id))?
                .into_iter()
                .filter(|attempt: &CodingExecutionAttempt| attempt.work_item_id == work_item_id)
                .collect();
        attempts.sort_by(|left, right| {
            left.attempt_no
                .cmp(&right.attempt_no)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(attempts)
    }

    pub fn get_active_attempt(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<Option<CodingExecutionAttempt>, ProductStoreError> {
        let active = self
            .list_attempts_for_work_item(project_id, issue_id, work_item_id)?
            .into_iter()
            .find(|attempt| attempt.status.is_active());
        Ok(active)
    }

    pub fn delete_attempt(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        let attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        super::remove_file_if_exists(&self.attempt_path(project_id, issue_id, attempt_id))?;
        super::remove_dir_all_if_exists(&self.attempt_dir(project_id, issue_id, attempt_id))?;
        Ok(attempt)
    }

    pub fn delete_attempts_for_work_item(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<Vec<CodingExecutionAttempt>, ProductStoreError> {
        let attempts = self.list_attempts_for_work_item(project_id, issue_id, work_item_id)?;
        for attempt in &attempts {
            self.delete_attempt(project_id, issue_id, &attempt.id)?;
        }
        Ok(attempts)
    }

    pub fn update_attempt_status(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        status: CodingAttemptStatus,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let path = self.attempt_path(project_id, issue_id, attempt_id);
        let mut attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        if !valid_status_transition(&attempt.status, &status) {
            return Err(ProductStoreError::Io(format!(
                "invalid_coding_attempt_status_transition: {:?} -> {:?}",
                attempt.status, status
            )));
        }
        let now = Utc::now().to_rfc3339();
        if matches!(
            status,
            CodingAttemptStatus::Completed
                | CodingAttemptStatus::Failed
                | CodingAttemptStatus::Aborted
        ) {
            attempt.completed_at = Some(now.clone());
        }
        attempt.status = status;
        attempt.updated_at = now;
        write_json(&path, &attempt)?;
        Ok(attempt)
    }

    pub fn update_attempt_stage(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        stage: CodingExecutionStage,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let path = self.attempt_path(project_id, issue_id, attempt_id);
        let mut attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        if !valid_stage_transition(&attempt.stage, &stage) {
            return Err(ProductStoreError::Io(format!(
                "invalid_coding_attempt_stage_transition: {:?} -> {:?}",
                attempt.stage, stage
            )));
        }
        attempt.stage = stage;
        attempt.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &attempt)?;
        Ok(attempt)
    }

    pub fn update_attempt_worktree_path(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        worktree_path: std::path::PathBuf,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let path = self.attempt_path(project_id, issue_id, attempt_id);
        let mut attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        attempt.worktree_path = Some(worktree_path);
        attempt.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &attempt)?;
        Ok(attempt)
    }

    pub fn update_attempt_head_commit(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        head_commit: Option<String>,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let path = self.attempt_path(project_id, issue_id, attempt_id);
        let mut attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        attempt.head_commit = head_commit;
        attempt.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &attempt)?;
        Ok(attempt)
    }

    pub fn update_attempt_review_request_state(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        head_commit: String,
        pushed_remote: String,
        review_request_id: String,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let path = self.attempt_path(project_id, issue_id, attempt_id);
        let mut attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        attempt.head_commit = Some(head_commit);
        attempt.pushed_remote = Some(pushed_remote);
        attempt.review_request_id = Some(review_request_id);
        attempt.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &attempt)?;
        Ok(attempt)
    }

    pub fn update_attempt_provider_config_snapshot(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        provider_config_snapshot: ProviderConfigSnapshot,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let path = self.attempt_path(project_id, issue_id, attempt_id);
        let mut attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        attempt.provider_config_snapshot = provider_config_snapshot;
        attempt.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &attempt)?;
        Ok(attempt)
    }

    pub fn get_role_provider_config_snapshot(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingRoleProviderConfigSnapshot, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        let path = self.role_provider_config_path(project_id, issue_id, attempt_id);
        if super::path_is_regular_file(&path)? {
            return read_json(&path);
        }
        let attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        Ok(CodingRoleProviderConfigSnapshot::from(
            &attempt.provider_config_snapshot,
        ))
    }

    pub fn update_role_provider_config_snapshot(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_provider_config_snapshot: CodingRoleProviderConfigSnapshot,
    ) -> Result<CodingRoleProviderConfigSnapshot, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        let attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        write_json(
            &self.role_provider_config_path(&attempt.project_id, &attempt.issue_id, &attempt.id),
            &role_provider_config_snapshot,
        )?;
        Ok(role_provider_config_snapshot)
    }

    pub fn increment_attempt_rework_count(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        let path = self.attempt_path(project_id, issue_id, attempt_id);
        let mut attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        attempt.rework_count += 1;
        attempt.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &attempt)?;
        Ok(attempt)
    }

    pub fn replace_attempt_provider_conversations(
        &self,
        attempt_id: &str,
        provider_conversations: Vec<ProviderConversationRef>,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        validate_relative_id(attempt_id)?;
        let mut attempt = self.find_attempt_by_id(attempt_id)?;
        let path = self.attempt_path(&attempt.project_id, &attempt.issue_id, &attempt.id);
        attempt.provider_conversations = provider_conversations;
        attempt.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &attempt)?;
        Ok(attempt)
    }

    pub fn read_attempt_artifact_text(
        &self,
        attempt_id: &str,
        artifact_ref: &str,
    ) -> Result<String, ProductStoreError> {
        use std::fs;

        validate_relative_artifact_ref(artifact_ref)?;
        let attempt = self.find_attempt_by_id(attempt_id)?;
        let path = self
            .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .join(artifact_ref);
        fs::read_to_string(&path)
            .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))
    }
}

fn valid_status_transition(current: &CodingAttemptStatus, next: &CodingAttemptStatus) -> bool {
    if current == next {
        return true;
    }
    match current {
        CodingAttemptStatus::Created => {
            matches!(
                next,
                CodingAttemptStatus::Running | CodingAttemptStatus::Aborted
            )
        }
        CodingAttemptStatus::Running => matches!(
            next,
            CodingAttemptStatus::WaitingForHuman
                | CodingAttemptStatus::Blocked
                | CodingAttemptStatus::Completed
                | CodingAttemptStatus::Failed
                | CodingAttemptStatus::Aborted
        ),
        CodingAttemptStatus::WaitingForHuman => {
            matches!(
                next,
                CodingAttemptStatus::Running
                    | CodingAttemptStatus::Completed
                    | CodingAttemptStatus::Aborted
            )
        }
        CodingAttemptStatus::Blocked => {
            matches!(
                next,
                CodingAttemptStatus::Running | CodingAttemptStatus::Aborted
            )
        }
        CodingAttemptStatus::Completed
        | CodingAttemptStatus::Failed
        | CodingAttemptStatus::Aborted => false,
    }
}

fn valid_stage_transition(current: &CodingExecutionStage, next: &CodingExecutionStage) -> bool {
    if current == next {
        return true;
    }
    if matches!(next, CodingExecutionStage::Rework) {
        return true;
    }
    if matches!(current, CodingExecutionStage::Rework) {
        return matches!(
            next,
            CodingExecutionStage::Coding
                | CodingExecutionStage::Testing
                | CodingExecutionStage::CodeReview
                | CodingExecutionStage::ReviewRequest
                | CodingExecutionStage::InternalPrReview
                | CodingExecutionStage::FinalConfirm
        );
    }
    next.order() >= current.order()
}
