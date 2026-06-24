use chrono::Utc;

use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::VerificationPlan;

use super::{
    CreateVerificationPlanInput, LifecycleStore, count_json_files, delete_required_file,
    list_json_records,
};

impl LifecycleStore {
    pub fn create_verification_plan(
        &self,
        input: CreateVerificationPlanInput,
    ) -> Result<VerificationPlan, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.work_item_id)?;

        let root = self.verification_plans_root(&input.project_id, &input.issue_id);
        let id = match input.id {
            Some(ref id) => {
                validate_relative_id(id)?;
                id.clone()
            }
            None => next_sequential_id("verification_plan", count_json_files(&root)?),
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
