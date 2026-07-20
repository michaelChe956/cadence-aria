use super::*;

impl CodingWorkspaceEngine {
    pub(crate) fn record_role_run_event(
        &self,
        attempt: &CodingExecutionAttempt,
        role_run: Option<&CodingRoleRun>,
        event_type: CodingRoleRunEventType,
        payload: serde_json::Value,
    ) {
        let Some(role_run) = role_run else {
            return;
        };
        if let Err(error) = self
            .store
            .append_role_run_event(attempt, role_run, event_type, payload)
        {
            tracing::warn!(
                role_run_id = role_run.id.as_str(),
                event_type = ?event_type,
                error = %error,
                "failed to persist coding role run event"
            );
        }
    }

    pub(super) fn record_provider_start_required(
        &self,
        attempt: &CodingExecutionAttempt,
        role_run: Option<&CodingRoleRun>,
        payload: serde_json::Value,
    ) -> Result<(), ProductStoreError> {
        let Some(role_run) = role_run else {
            return Ok(());
        };
        self.store
            .append_role_run_event(
                attempt,
                role_run,
                CodingRoleRunEventType::ProviderStart,
                payload,
            )
            .map(|_| ())
    }

    pub(crate) fn persist_provider_cancellation(
        &self,
        attempt: &CodingExecutionAttempt,
        role_run: Option<&CodingRoleRun>,
        phase: &str,
    ) -> Result<(), CodingWorkspaceEngineError> {
        self.record_role_run_event(
            attempt,
            role_run,
            CodingRoleRunEventType::Aborted,
            json!({
                "reason": "abort_attempt",
                "phase": phase,
            }),
        );
        if let Some(role_run) = role_run {
            let current = self.store.get_role_run(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &role_run.id,
            )?;
            if current.status == CodingRoleRunStatus::Running {
                self.store.update_role_run_status(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    &role_run.id,
                    CodingRoleRunStatus::Aborted,
                    Some("abort_attempt".to_string()),
                )?;
            }
        }
        Ok(())
    }

    pub(crate) fn unresolved_provider_choice_error(
        &self,
        attempt: &CodingExecutionAttempt,
        role_run: Option<&CodingRoleRun>,
        phase: &str,
        open_choice_ids: &[String],
    ) -> CodingWorkspaceEngineError {
        self.record_role_run_event(
            attempt,
            role_run,
            CodingRoleRunEventType::ProviderFailed,
            json!({
                "phase": phase,
                "code": "provider_choice_unresolved",
                "message": "provider continued before required user choice was resolved",
                "choice_ids": open_choice_ids
            }),
        );
        CodingWorkspaceEngineError::ProviderStream("provider_choice_unresolved".to_string())
    }
}
