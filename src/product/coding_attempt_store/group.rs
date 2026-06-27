use chrono::Utc;

use crate::product::coding_models::{
    CodingAttemptScope, CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionStage,
    CodingExecutionUnit, CodingExecutionUnitStatus, CodingRoleProviderConfigSnapshot,
};
use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};

use super::{CreateCodingExecutionUnitInput, CreateGroupCodingAttemptInput};

impl super::CodingAttemptStore {
    pub fn create_group_attempt(
        &self,
        input: CreateGroupCodingAttemptInput,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.plan_id)?;
        validate_relative_id(&input.current_work_item_id)?;

        let existing_attempts: Vec<CodingExecutionAttempt> = super::list_json_records(
            &self.coding_attempts_root(&input.project_id, &input.issue_id),
        )?;
        if let Some(active) = existing_attempts
            .into_iter()
            .find(|attempt| attempt.status.is_active())
        {
            return Err(ProductStoreError::Io(format!(
                "active_coding_attempt_exists: {}",
                active.id
            )));
        }

        let root = self.coding_attempts_root(&input.project_id, &input.issue_id);
        let id = next_sequential_id("coding_attempt", super::count_json_files(&root)?);
        let attempt_no = self
            .list_attempts_for_work_item(
                &input.project_id,
                &input.issue_id,
                &input.current_work_item_id,
            )?
            .iter()
            .map(|attempt| attempt.attempt_no)
            .max()
            .unwrap_or(0)
            + 1;
        let now = Utc::now().to_rfc3339();
        let attempt = CodingExecutionAttempt {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            work_item_id: input.current_work_item_id.clone(),
            attempt_no,
            scope: CodingAttemptScope::WorkItemGroup,
            status: CodingAttemptStatus::Created,
            stage: CodingExecutionStage::PrepareContext,
            base_branch: input.base_branch,
            branch_name: input.branch_name,
            worktree_path: input.worktree_path,
            provider_config_snapshot: input.provider_config_snapshot,
            rework_count: 0,
            max_auto_rework: input.max_auto_rework,
            work_item_group_id: Some(input.plan_id),
            current_work_item_id: Some(input.current_work_item_id),
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

    pub fn create_coding_unit(
        &self,
        input: CreateCodingExecutionUnitInput,
    ) -> Result<CodingExecutionUnit, ProductStoreError> {
        validate_relative_id(&input.attempt_id)?;
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.plan_id)?;
        validate_relative_id(&input.work_item_id)?;

        let attempt = self.get_attempt(&input.project_id, &input.issue_id, &input.attempt_id)?;
        if attempt.scope != CodingAttemptScope::WorkItemGroup {
            return Err(ProductStoreError::Io(format!(
                "coding_attempt_scope_invalid: {}",
                attempt.id
            )));
        }
        if attempt.work_item_group_id.as_deref() != Some(input.plan_id.as_str()) {
            return Err(ProductStoreError::Io(format!(
                "coding_attempt_plan_mismatch: {}",
                attempt.id
            )));
        }
        if input.status.is_active()
            && self
                .list_coding_units(&input.project_id, &input.issue_id, &input.attempt_id)?
                .into_iter()
                .any(|unit| unit.status.is_active())
        {
            return Err(ProductStoreError::Io(format!(
                "active_coding_unit_exists: {}",
                input.attempt_id
            )));
        }

        let root = self.coding_units_root(&input.project_id, &input.issue_id, &input.attempt_id);
        let id = next_sequential_id("coding_unit", super::count_json_files(&root)?);
        let now = Utc::now().to_rfc3339();
        let started_at = if matches!(input.status, CodingExecutionUnitStatus::Running) {
            Some(now.clone())
        } else {
            None
        };
        let unit = CodingExecutionUnit {
            id: id.clone(),
            attempt_id: input.attempt_id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            plan_id: input.plan_id,
            work_item_id: input.work_item_id,
            order_index: input.order_index,
            status: input.status,
            started_at,
            completed_at: None,
            handoff_ref: None,
            completion_commit: None,
            summary: None,
            created_at: now.clone(),
            updated_at: now,
        };
        write_json(
            &self.coding_unit_path(&unit.project_id, &unit.issue_id, &unit.attempt_id, &unit.id),
            &unit,
        )?;

        if unit.status.is_active() {
            let mut attempt = attempt;
            attempt.active_unit_id = Some(unit.id.clone());
            attempt.current_work_item_id = Some(unit.work_item_id.clone());
            attempt.updated_at = Utc::now().to_rfc3339();
            self.save_coding_attempt(&attempt)?;
        }

        Ok(unit)
    }

    pub fn list_coding_units(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<CodingExecutionUnit>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        let mut units: Vec<CodingExecutionUnit> =
            super::list_json_records(&self.coding_units_root(project_id, issue_id, attempt_id))?;
        units.sort_by(|left, right| {
            left.order_index
                .cmp(&right.order_index)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(units)
    }

    pub fn get_active_coding_unit(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Option<CodingExecutionUnit>, ProductStoreError> {
        let mut active_units = self
            .list_coding_units(project_id, issue_id, attempt_id)?
            .into_iter()
            .filter(|unit| unit.status.is_active());
        let first = active_units.next();
        if let Some(extra) = active_units.next() {
            return Err(ProductStoreError::Io(format!(
                "active_coding_unit_ambiguous: {}",
                extra.id
            )));
        }
        Ok(first)
    }

    pub fn update_coding_unit_status(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        unit_id: &str,
        status: CodingExecutionUnitStatus,
        summary: Option<String>,
    ) -> Result<CodingExecutionUnit, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        validate_relative_id(unit_id)?;
        let path = self.coding_unit_path(project_id, issue_id, attempt_id, unit_id);
        let mut unit: CodingExecutionUnit = read_json(&path)?;
        if status.is_active()
            && self
                .list_coding_units(project_id, issue_id, attempt_id)?
                .into_iter()
                .any(|existing| existing.id != unit_id && existing.status.is_active())
        {
            return Err(ProductStoreError::Io(format!(
                "active_coding_unit_exists: {}",
                attempt_id
            )));
        }
        let now = Utc::now().to_rfc3339();
        if matches!(status, CodingExecutionUnitStatus::Running) && unit.started_at.is_none() {
            unit.started_at = Some(now.clone());
        }
        if matches!(
            status,
            CodingExecutionUnitStatus::Completed
                | CodingExecutionUnitStatus::Failed
                | CodingExecutionUnitStatus::Skipped
        ) {
            unit.completed_at = Some(now.clone());
        }
        unit.status = status;
        unit.summary = summary;
        unit.updated_at = now;
        write_json(&path, &unit)?;

        let mut attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
        match self.get_active_coding_unit(project_id, issue_id, attempt_id)? {
            Some(active) => {
                attempt.active_unit_id = Some(active.id.clone());
                attempt.current_work_item_id = Some(active.work_item_id.clone());
            }
            None => {
                attempt.active_unit_id = None;
                attempt.current_work_item_id = None;
            }
        }
        attempt.updated_at = Utc::now().to_rfc3339();
        self.save_coding_attempt(&attempt)?;

        Ok(unit)
    }
}
