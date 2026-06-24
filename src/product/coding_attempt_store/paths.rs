use std::path::PathBuf;

use crate::product::coding_attempt_store::utils::coding_stage_dir_name;
use crate::product::coding_models::{CodingExecutionStage, CodingRoleRun};
use crate::product::json_store::{ProductStoreError, validate_relative_id};

impl super::CodingAttemptStore {
    pub(crate) fn attempt_path(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> PathBuf {
        self.coding_attempts_root(project_id, issue_id)
            .join(format!("{attempt_id}.json"))
    }

    pub(crate) fn attempt_dir(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> PathBuf {
        self.coding_attempts_root(project_id, issue_id)
            .join(attempt_id)
    }

    pub(crate) fn work_item_execution_plan_path(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("work-item-execution-plan.json")
    }

    pub(crate) fn work_item_handoff_path(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("work-item-handoff.json")
    }

    pub(crate) fn role_provider_config_path(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("role-provider-config.json")
    }

    pub(crate) fn rework_instructions_root(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("rework-instructions")
    }

    pub(crate) fn analyst_decisions_root(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("analyst-decisions")
    }

    pub(crate) fn test_plans_root(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("test-plans")
    }

    pub(crate) fn role_runs_root(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("role-runs")
    }

    pub(crate) fn role_run_events_root(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("role-run-events")
    }

    pub(crate) fn role_run_event_log_path(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_run_id: &str,
    ) -> PathBuf {
        self.role_run_events_root(project_id, issue_id, attempt_id)
            .join(format!("{role_run_id}.jsonl"))
    }

    pub(crate) fn role_run_event_artifact_root(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_run_id: &str,
    ) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("artifacts")
            .join("role-run-events")
            .join(role_run_id)
    }

    pub(crate) fn role_run_path(
        &self,
        project_id: &str,
        issue_id: &str,
        run: &CodingRoleRun,
    ) -> PathBuf {
        self.role_runs_root(project_id, issue_id, &run.attempt_id)
            .join(format!("{}.json", run.id))
    }

    pub(crate) fn provider_raw_output_root(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("provider-raw")
    }

    pub(crate) fn blocked_gates_root(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("blocked-gates")
    }

    pub(crate) fn choice_gates_root(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("choice-gates")
    }

    pub(crate) fn quality_bypass_audits_root(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("quality-bypass-audits")
    }

    pub(crate) fn coding_attempts_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("coding-attempts")
    }

    pub fn attempt_artifact_root(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> PathBuf {
        self.attempt_dir(project_id, issue_id, attempt_id)
            .join("artifacts")
    }

    pub fn attempt_test_output_root(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> PathBuf {
        self.attempt_artifact_root(project_id, issue_id, attempt_id)
            .join("test-output")
    }

    pub fn attempt_test_output_path(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        artifact_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        use std::path::Path;

        let file_name = Path::new(artifact_id)
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| ProductStoreError::PathEscape(artifact_id.to_string()))?;
        validate_relative_id(file_name)?;
        Ok(self
            .attempt_test_output_root(project_id, issue_id, attempt_id)
            .join(file_name))
    }

    pub fn save_provider_raw_output(
        &self,
        attempt_id: &str,
        stage: CodingExecutionStage,
        purpose: &str,
        output: &str,
    ) -> Result<String, ProductStoreError> {
        use std::fs;

        validate_relative_id(purpose)?;
        let attempt = self.find_attempt_by_id(attempt_id)?;
        let stage_dir_name = coding_stage_dir_name(&stage);
        let raw_root = self
            .provider_raw_output_root(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .join(stage_dir_name);
        fs::create_dir_all(&raw_root).map_err(|error| {
            ProductStoreError::Io(format!("create {}: {error}", raw_root.display()))
        })?;

        let sequence = super::next_text_file_sequence(&raw_root, purpose)?;
        let file_name = format!("{purpose}_{sequence:04}.txt");
        let path = raw_root.join(&file_name);
        fs::write(&path, output)
            .map_err(|error| ProductStoreError::Io(format!("write {}: {error}", path.display())))?;

        Ok(format!(
            "provider-raw/{}/{}",
            coding_stage_dir_name(&stage),
            file_name
        ))
    }

    pub fn save_analyst_evidence(
        &self,
        attempt_id: &str,
        evidence: &str,
    ) -> Result<String, ProductStoreError> {
        use std::fs;

        let attempt = self.find_attempt_by_id(attempt_id)?;
        let evidence_root = self
            .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .join("artifacts")
            .join("rework");
        fs::create_dir_all(&evidence_root).map_err(|error| {
            ProductStoreError::Io(format!("create {}: {error}", evidence_root.display()))
        })?;

        let sequence = super::next_text_file_sequence(&evidence_root, "analyst_evidence")?;
        let file_name = format!("analyst_evidence_{sequence:04}.txt");
        let path = evidence_root.join(&file_name);
        fs::write(&path, evidence)
            .map_err(|error| ProductStoreError::Io(format!("write {}: {error}", path.display())))?;

        Ok(format!("artifacts/rework/{}", file_name))
    }
}
