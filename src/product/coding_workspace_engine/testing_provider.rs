use super::*;

mod execution;
mod execution_types;
mod plan;
mod report;

use execution_types::{ProviderTestingExecutionInput, ProviderTestingExecutionOutcome};
use plan::{ProviderTestingPlanInput, ProviderTestingPlanOutcome, ProviderTestingPlanPhase};
use report::ProviderTestingReportInput;

impl CodingWorkspaceEngine {
    pub async fn execute_testing_with_provider_commands(
        &self,
        attempt: &CodingExecutionAttempt,
        provider: &dyn StreamingProviderAdapter,
        _context: &CodingExecutionContext,
        _specs: &[TestCommandSpec],
        options: TesterAgentOptions,
        command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    ) -> Result<TestingReport, CodingWorkspaceEngineError> {
        self.execute_testing_with_distinct_provider_commands(
            attempt,
            ProviderTestingAdapters {
                plan: provider,
                execute: provider,
            },
            _context,
            _specs,
            options,
            command_rx,
        )
        .await
    }

    pub async fn execute_testing_with_distinct_provider_commands(
        &self,
        attempt: &CodingExecutionAttempt,
        providers: ProviderTestingAdapters<'_>,
        _context: &CodingExecutionContext,
        _specs: &[TestCommandSpec],
        options: TesterAgentOptions,
        command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    ) -> Result<TestingReport, CodingWorkspaceEngineError> {
        let attempt = self.store.ensure_provider_run_allowed(attempt)?;
        let Some(worktree_path) = attempt.worktree_path.as_ref() else {
            return Err(CodingWorkspaceEngineError::MissingWorktree(
                attempt.id.clone(),
            ));
        };
        let worktree_path = worktree_path.clone();
        let attempt = self.store.update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Testing,
        )?;
        let node = self.create_testing_timeline_node(&attempt)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node: node.clone() })
            .await;
        let role_run = match self.store.latest_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Testing,
            CodingProviderRole::Tester,
        )? {
            Some(run) if run.status == CodingRoleRunStatus::Running && run.node_id.is_none() => {
                self.store.attach_role_run_node(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    &run.id,
                    node.id.clone(),
                )?
            }
            _ => self.store.create_role_run(
                &attempt,
                CodingExecutionStage::Testing,
                CodingProviderRole::Tester,
                CodingRoleRunTrigger::Initial,
                Some(node.id.clone()),
            )?,
        };

        if !providers.plan.supports_provider_driven_testing()
            || !providers.execute.supports_provider_driven_testing()
        {
            return self
                .block_provider_driven_testing(
                    &attempt,
                    &node,
                    BlockedTestingGateContext {
                        reason_code: "provider_driven_testing_not_supported".to_string(),
                        description: "Tester provider does not support provider-driven testing"
                            .to_string(),
                        raw_provider_output_ref: None,
                        role_run: Some(&role_run),
                    },
                )
                .await;
        }

        let plan_phase = self
            .run_provider_testing_plan_phase(ProviderTestingPlanInput {
                attempt: attempt.clone(),
                node: node.clone(),
                role_run: role_run.clone(),
                provider: providers.plan,
                worktree_path: worktree_path.clone(),
                options: &options,
                command_rx,
            })
            .await?;
        let ProviderTestingPlanPhase {
            tester_provider: _plan_tester_provider,
            plan,
            chat_entry_sequence,
        } = match plan_phase {
            ProviderTestingPlanOutcome::EarlyReport(report) => return Ok(report),
            ProviderTestingPlanOutcome::Completed(phase) => phase,
        };
        let execute_tester_provider = self
            .store
            .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .tester_execute_provider()
            .clone();

        let execution = self
            .run_provider_testing_execution_phase(ProviderTestingExecutionInput {
                attempt: attempt.clone(),
                node: node.clone(),
                role_run: role_run.clone(),
                provider: providers.execute,
                worktree_path: worktree_path.clone(),
                tester_provider: execute_tester_provider.clone(),
                plan: plan.clone(),
                chat_entry_sequence,
                options: &options,
                command_rx,
            })
            .await?;
        let phase = match execution {
            ProviderTestingExecutionOutcome::EarlyReport(report) => return Ok(*report),
            ProviderTestingExecutionOutcome::Completed(phase) => phase,
        };

        self.finalize_provider_testing_report_phase(ProviderTestingReportInput {
            attempt,
            node,
            role_run,
            provider: providers.execute,
            worktree_path,
            tester_provider: execute_tester_provider,
            plan,
            options: &options,
            command_rx,
            phase,
        })
        .await
    }
}
