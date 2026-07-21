use super::*;

pub(super) struct TesterToolExecutionInput<'a> {
    pub(super) attempt: &'a CodingExecutionAttempt,
    pub(super) role_run: &'a CodingRoleRun,
    pub(super) call: &'a ProviderToolCall,
    pub(super) worktree_path: &'a Path,
    pub(super) artifact_output_root: &'a Path,
    pub(super) context_loader: &'a TestContextLoader,
    pub(super) provider_commands: &'a mpsc::Sender<ProviderCommand>,
    pub(super) cancellation: &'a CancellationToken,
}

impl CodingWorkspaceEngine {
    pub(super) async fn execute_and_send_tester_tool_call(
        &self,
        input: TesterToolExecutionInput<'_>,
    ) -> Result<crate::product::tester_agent_loop::TesterToolOutcome, CodingWorkspaceEngineError>
    {
        let TesterToolExecutionInput {
            attempt,
            role_run,
            call,
            worktree_path,
            artifact_output_root,
            context_loader,
            provider_commands,
            cancellation,
        } = input;
        let outcome = execute_tester_tool_call_with_context(
            call,
            worktree_path,
            artifact_output_root,
            Some(context_loader),
            cancellation.clone(),
        )
        .await;
        let outcome = match outcome {
            Err(crate::product::tester_agent_loop::TesterAgentError::TestExecutor(
                TestExecutorError::Cancelled,
            )) => {
                self.persist_provider_cancellation(
                    attempt,
                    Some(role_run),
                    "execute_test_plan_tool_call",
                )?;
                return Err(CodingWorkspaceEngineError::Aborted);
            }
            Err(error) => return Err(error.into()),
            Ok(outcome) => outcome,
        };
        if cancellation.is_cancelled() {
            self.persist_provider_cancellation(
                attempt,
                Some(role_run),
                "execute_test_plan_tool_result",
            )?;
            return Err(CodingWorkspaceEngineError::Aborted);
        }

        let sent = send_provider_command_with_cancellation(
            provider_commands,
            ProviderCommand::ToolResult(outcome.result.clone()),
            cancellation,
        )
        .await;
        if !sent && cancellation.is_cancelled() {
            self.persist_provider_cancellation(
                attempt,
                Some(role_run),
                "execute_test_plan_tool_result",
            )?;
            return Err(CodingWorkspaceEngineError::Aborted);
        }
        Ok(outcome)
    }
}
