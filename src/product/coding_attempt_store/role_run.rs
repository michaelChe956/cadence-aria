use chrono::Utc;

use crate::product::coding_models::{
    CodingExecutionAttempt, CodingExecutionStage, CodingProviderRole, CodingRoleRun,
    CodingRoleRunRetryMetadata, CodingRoleRunStatus, CodingRoleRunTrigger,
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
        validate_relative_id(&attempt.project_id)?;
        validate_relative_id(&attempt.issue_id)?;
        validate_relative_id(&attempt.id)?;
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
            id: id.clone(),
            attempt_id: attempt.id.clone(),
            stage,
            role,
            run_no,
            status: CodingRoleRunStatus::Running,
            trigger,
            retry_metadata: Some(CodingRoleRunRetryMetadata {
                cycle_id: id.clone(),
                attempt_no: 1,
                prior_run_id: None,
            }),
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

    pub fn create_retry_role_run(
        &self,
        attempt: &CodingExecutionAttempt,
        stage: CodingExecutionStage,
        role: CodingProviderRole,
        trigger: CodingRoleRunTrigger,
        node_id: Option<String>,
        retry: CodingRoleRunRetryMetadata,
    ) -> Result<CodingRoleRun, ProductStoreError> {
        validate_relative_id(&attempt.project_id)?;
        validate_relative_id(&attempt.issue_id)?;
        validate_relative_id(&attempt.id)?;
        validate_retry_metadata(&retry)?;
        if let Some(node_id) = &node_id {
            validate_relative_id(node_id)?;
        }

        let prior_run_id = retry
            .prior_run_id
            .as_deref()
            .ok_or_else(|| invalid_retry_metadata("missing_prior_run_id"))?;
        let prior = self.get_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            prior_run_id,
        )?;
        let Some(prior_retry) = prior.retry_metadata.as_ref() else {
            return Err(invalid_retry_metadata("prior_run_missing_retry_metadata"));
        };
        if prior.status != CodingRoleRunStatus::Failed
            || prior.stage != stage
            || prior.role != role
            || prior_retry.cycle_id != retry.cycle_id
            || prior_retry.attempt_no != retry.attempt_no - 1
        {
            return Err(invalid_retry_metadata(prior.id));
        }

        let existing = self.list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        if existing.iter().any(|run| {
            run.retry_metadata.as_ref().is_some_and(|existing_retry| {
                existing_retry.cycle_id == retry.cycle_id
                    && existing_retry.attempt_no == retry.attempt_no
            })
        }) {
            return Err(invalid_retry_metadata(format!(
                "{}:{}",
                retry.cycle_id, retry.attempt_no
            )));
        }
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
            retry_metadata: Some(retry),
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

fn validate_retry_metadata(retry: &CodingRoleRunRetryMetadata) -> Result<(), ProductStoreError> {
    validate_relative_id(&retry.cycle_id)?;
    let Some(prior_run_id) = retry.prior_run_id.as_deref() else {
        return Err(invalid_retry_metadata("missing_prior_run_id"));
    };
    validate_relative_id(prior_run_id)?;
    if !(2..=3).contains(&retry.attempt_no) {
        return Err(invalid_retry_metadata(retry.attempt_no.to_string()));
    }
    Ok(())
}

fn invalid_retry_metadata(id: impl Into<String>) -> ProductStoreError {
    ProductStoreError::Conflict {
        kind: "coding_role_run_retry_metadata",
        id: id.into(),
    }
}
