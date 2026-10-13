use chrono::Utc;

use crate::product::id::next_sequential_id_in_directory;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::VerificationPlan;

use super::{
    CreateVerificationPlanInput, LifecycleStore, delete_required_file, list_json_records,
    path_is_regular_file,
};

impl LifecycleStore {
    pub fn create_verification_plan(
        &self,
        input: CreateVerificationPlanInput,
    ) -> Result<VerificationPlan, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.work_item_id)?;

        // D1.3a：验证计划归拥有 WorkItem 的 Plan——按 WI 记录真实 refs 归约；
        // WI 档案缺失（尚无归属可证）保持原行为。
        // 直接构造路径：WI 档案缺失（尚无归属可证）保持原行为，不引入 NotFound。
        let work_item_path = self
            .work_items_root(&input.project_id, &input.issue_id)
            .join(format!("{}.json", input.work_item_id));
        let owning_scope = if path_is_regular_file(&work_item_path)? {
            let record: crate::product::models::LifecycleWorkItemRecord = read_json(&work_item_path)?;
            self.entity_scope_for_work_item_record(&record)?
        } else {
            None
        };
        self.with_optional_entity_live_write(owning_scope.as_ref(), move || {
            let root = self.verification_plans_root(&input.project_id, &input.issue_id);
            let id = match input.id {
                Some(id) => {
                    validate_relative_id(&id)?;
                    id
                }
                None => next_sequential_id_in_directory("verification_plan", &root).map_err(
                    |error| ProductStoreError::Io(format!("read {}: {error}", root.display())),
                )?,
            };
            let now = Utc::now().to_rfc3339();
            let plan = VerificationPlan {
                id: id.clone(),
                project_id: input.project_id,
                issue_id: input.issue_id,
                work_item_id: input.work_item_id,
                repository_profile_ref: input.repository_profile_ref,
                provider_run_ref: input.provider_run_ref,
                scope: input.scope,
                commands: input.commands,
                manual_checks: input.manual_checks,
                required_gates: input.required_gates,
                risk_notes: input.risk_notes,
                confidence: input.confidence,
                fallback_policy: input.fallback_policy,
                created_at: now.clone(),
                updated_at: now,
            };
            write_json(&root.join(format!("{id}.json")), &plan)?;
            Ok(plan)
        })
    }

    pub fn get_verification_plan(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<VerificationPlan, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(plan_id)?;
        read_json(
            &self
                .verification_plans_root(project_id, issue_id)
                .join(format!("{plan_id}.json")),
        )
    }

    pub fn list_verification_plans(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<VerificationPlan>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        list_json_records(&self.verification_plans_root(project_id, issue_id))
    }

    pub fn delete_verification_plan(
        &self,
        project_id: &str,
        issue_id: &str,
        verification_plan_id: &str,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(verification_plan_id)?;

        let path = self
            .verification_plans_root(project_id, issue_id)
            .join(format!("{verification_plan_id}.json"));
        delete_required_file(&path, "verification_plan", verification_plan_id)
    }
}
