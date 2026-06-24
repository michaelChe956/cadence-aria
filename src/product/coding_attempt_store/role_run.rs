use chrono::Utc;

use crate::product::coding_models::{
    CodingExecutionAttempt, CodingExecutionStage, CodingProviderRole, CodingRoleRun,
    CodingRoleRunStatus, CodingRoleRunTrigger,
};
use crate::product::id::next_sequential_id;
use crate::product::json_store::{
    ProductStoreError, read_json, validate_relative_artifact_ref, validate_relative_id, write_json,
};

impl super::CodingAttemptStore {
    pub fn create_role_run(
        &self,
        attempt: &CodingExecutionAttempt,
        stage: CodingExecutionStage,
        role: CodingProviderRole,
        trigger: CodingRoleRunTrigger,
        node_id: Option<String>,
    ) -> Result<CodingRoleRun, ProductStoreError> {
        if let Some(node_id) = &node_id {
            validate_relative_id(node_id)?;
        }
        let existing = self.list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let id = next_sequential_id("coding_role_run", existing.len());
        let run_no = existing
            .iter()
            .filter(|run| run.stage == stage && run.role == role)
            .map(|run| run.run_no)
            .max()
            .unwrap_or(0)
            + 1;
        let run = CodingRoleRun {
            id,
            attempt_id: attempt.id.clone(),
            stage,
            role,
            run_no,
            status: CodingRoleRunStatus::Running,
            trigger,
            node_id,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            supersedes_run_id: None,
            superseded_by_run_id: None,
            reason_code: None,
            raw_provider_output_refs: Vec::new(),
            artifact_refs: Vec::new(),
        };
        self.save_role_run(&attempt.project_id, &attempt.issue_id, &run)?;
        Ok(run)
    }

    pub fn list_role_runs(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<CodingRoleRun>, ProductStoreError> {
        let mut runs: Vec<CodingRoleRun> =
            super::list_json_records(&self.role_runs_root(project_id, issue_id, attempt_id))?;
        runs.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(runs)
    }

    pub fn latest_role_run(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        stage: CodingExecutionStage,
        role: CodingProviderRole,
    ) -> Result<Option<CodingRoleRun>, ProductStoreError> {
        Ok(self
            .list_role_runs(project_id, issue_id, attempt_id)?
            .into_iter()
            .rev()
            .find(|run| run.stage == stage && run.role == role))
    }

    pub fn update_role_run_status(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_run_id: &str,
        status: CodingRoleRunStatus,
        reason_code: Option<String>,
    ) -> Result<CodingRoleRun, ProductStoreError> {
        let mut run = self.get_role_run(project_id, issue_id, attempt_id, role_run_id)?;
        run.status = status;
        run.reason_code = reason_code;
        run.completed_at = Some(Utc::now().to_rfc3339());
        self.save_role_run(project_id, issue_id, &run)?;
        Ok(run)
    }

    pub fn attach_role_run_node(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_run_id: &str,
        node_id: String,
    ) -> Result<CodingRoleRun, ProductStoreError> {
        validate_relative_id(&node_id)?;
        let mut run = self.get_role_run(project_id, issue_id, attempt_id, role_run_id)?;
        run.node_id = Some(node_id);
        self.save_role_run(project_id, issue_id, &run)?;
        Ok(run)
    }

    pub fn supersede_latest_role_run_and_create(
        &self,
        attempt: &CodingExecutionAttempt,
        stage: CodingExecutionStage,
        role: CodingProviderRole,
        trigger: CodingRoleRunTrigger,
        node_id: Option<String>,
        reason_code: Option<String>,
    ) -> Result<CodingRoleRun, ProductStoreError> {
        let previous = self.latest_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            stage.clone(),
            role.clone(),
        )?;
        let mut next = self.create_role_run(attempt, stage, role, trigger, node_id)?;
        next.supersedes_run_id = previous.as_ref().map(|run| run.id.clone());
        next.reason_code = reason_code;
        self.save_role_run(&attempt.project_id, &attempt.issue_id, &next)?;
        if let Some(mut previous_run) = previous {
            previous_run.status = CodingRoleRunStatus::Superseded;
            previous_run.superseded_by_run_id = Some(next.id.clone());
            previous_run.completed_at = Some(Utc::now().to_rfc3339());
            self.save_role_run(&attempt.project_id, &attempt.issue_id, &previous_run)?;
        }
        Ok(next)
    }

    pub fn update_role_run_refs(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_run_id: &str,
        raw_provider_output_refs: Vec<String>,
        artifact_refs: Vec<String>,
    ) -> Result<CodingRoleRun, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        validate_relative_id(role_run_id)?;
        let mut run = self.get_role_run(project_id, issue_id, attempt_id, role_run_id)?;
        for reference in raw_provider_output_refs {
            validate_relative_artifact_ref(&reference)?;
            if !run
                .raw_provider_output_refs
                .iter()
                .any(|existing| existing == &reference)
            {
                run.raw_provider_output_refs.push(reference);
            }
        }
        for reference in artifact_refs {
            validate_relative_artifact_ref(&reference)?;
            if !run
                .artifact_refs
                .iter()
                .any(|existing| existing == &reference)
            {
                run.artifact_refs.push(reference);
            }
        }
        self.save_role_run(project_id, issue_id, &run)?;
        Ok(run)
    }

    pub(crate) fn save_role_run(
        &self,
        project_id: &str,
        issue_id: &str,
        run: &CodingRoleRun,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&run.id)?;
        write_json(&self.role_run_path(project_id, issue_id, run), run)
    }

    pub(crate) fn get_role_run(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_run_id: &str,
    ) -> Result<CodingRoleRun, ProductStoreError> {
        validate_relative_id(role_run_id)?;
        read_json(
            &self
                .role_runs_root(project_id, issue_id, attempt_id)
                .join(format!("{role_run_id}.json")),
        )
    }
}
