use super::*;

mod execution;
mod plan;
mod report;

use execution::{ProviderTestingExecutionInput, ProviderTestingExecutionOutcome};
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

        if !provider.supports_provider_driven_testing() {
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
                provider,
                worktree_path: worktree_path.clone(),
                options: &options,
                command_rx,
            })
            .await?;
        let ProviderTestingPlanPhase {
            tester_provider,
            evaluation_context_json,
            plan,
            chat_entry_sequence,
        } = match plan_phase {
            ProviderTestingPlanOutcome::EarlyReport(report) => return Ok(report),
            ProviderTestingPlanOutcome::Completed(phase) => phase,
        };

        let execution = self
            .run_provider_testing_execution_phase(ProviderTestingExecutionInput {
                attempt: attempt.clone(),
                node: node.clone(),
                role_run: role_run.clone(),
                provider,
                worktree_path: worktree_path.clone(),
                tester_provider: tester_provider.clone(),
                plan: plan.clone(),
                evaluation_context_json,
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
            provider,
            worktree_path,
            tester_provider,
            plan,
            options: &options,
            command_rx,
            phase,
        })
        .await
    }
}
