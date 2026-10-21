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

    /// T25:零输出且 provider 未启动(spawn 前失败,如 LC resume 审计拒绝)
    /// 不落空 raw 引用——`recoverable_failed_reviewer_run` 以
    /// "raw_provider_output_refs 非空"表达 provider 真有产出证据,空文件
    /// 引用会毒化恢复判据,把可恢复的 spawn 前失败变成 retry_review 死门。
    /// spawn 后失败(有 ProviderStart 事件)仍保留证据(含空输出)。事件
    /// 日志读取失败时保守视为已启动(维持既有证据保留行为,不改变失败
    /// 分类;持久化自身失败仍走原错误路径)。
    pub(super) fn persist_invocation_output_if_evidenced(
        &self,
        attempt: &CodingExecutionAttempt,
        role_run: &CodingRoleRun,
        output_for_persistence: &str,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let provider_started = self
            .role_run_provider_started(attempt, role_run)
            .unwrap_or(true);
        if !output_for_persistence.trim().is_empty() || provider_started {
            self.persist_invocation_partial_output(attempt, role_run, output_for_persistence)?;
        }
        Ok(())
    }

    /// T25:role run 是否已有 ProviderStart 事件(modern start 成功后
    /// 才落)——区分"spawn 前失败"与"spawn 后失败"的恢复语义:
    /// 前者可经失败评审预留路径恢复,后者保留全部证据不可恢复。
    fn role_run_provider_started(
        &self,
        attempt: &CodingExecutionAttempt,
        role_run: &CodingRoleRun,
    ) -> Result<bool, ProductStoreError> {
        Ok(self
            .store
            .list_role_run_events(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &role_run.id,
            )?
            .iter()
            .any(|event| event.event_type == CodingRoleRunEventType::ProviderStart))
    }

    fn persist_invocation_partial_output(
        &self,
        attempt: &CodingExecutionAttempt,
        role_run: &CodingRoleRun,
        partial_output: &str,
    ) -> Result<(), CodingWorkspaceEngineError> {
        let raw_provider_output_ref = self.store.save_provider_raw_output(
            attempt,
            role_run.stage.clone(),
            "provider_stream_attempt",
            partial_output,
        )?;
        self.store.update_role_run_refs(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
            vec![raw_provider_output_ref],
            Vec::new(),
        )?;
        Ok(())
    }
}
