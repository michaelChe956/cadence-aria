use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::interactive::policy::{
    ConfirmationDecision, NodeWriteClass, PolicyPreset, ProviderNodeMeta,
};
use crate::task_run::types::TaskRunError;

#[derive(Debug, Clone, PartialEq)]
pub struct PendingProviderStep {
    pub node_id: String,
    pub provider_type: String,
    pub runtime_role: String,
    pub adapter_role: String,
    pub prompt: String,
    pub input_summary: Value,
    pub output_schema: String,
    pub write_class: NodeWriteClass,
    pub allowed_write_scope: Vec<String>,
    pub forbidden_actions: Vec<String>,
    pub verification_commands: Vec<String>,
    pub checkpoint_id: Option<String>,
}

impl PendingProviderStep {
    pub fn meta(&self) -> ProviderNodeMeta {
        ProviderNodeMeta::new(&self.node_id, &self.provider_type, self.write_class)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepRunnerResult {
    PausedForApproval(String),
    CompletedStep {
        node_id: String,
        provider_run_id: String,
        prompt: String,
    },
    NoMoreSteps,
}

pub trait StepRunner {
    fn next_provider_step(&mut self) -> Result<Option<PendingProviderStep>, TaskRunError>;

    fn run_provider_step(
        &mut self,
        step: PendingProviderStep,
        prompt: String,
    ) -> Result<StepRunnerResult, TaskRunError>;
}

pub struct InteractiveController<R: StepRunner> {
    workspace_root: PathBuf,
    task_id: String,
    policy: PolicyPreset,
    runner: R,
    pending_step: Option<PendingProviderStep>,
}

impl<R: StepRunner> InteractiveController<R> {
    pub fn new(workspace_root: PathBuf, task_id: String, policy: PolicyPreset, runner: R) -> Self {
        Self {
            workspace_root,
            task_id,
            policy,
            runner,
            pending_step: None,
        }
    }

    pub fn advance(&mut self) -> Result<StepRunnerResult, TaskRunError> {
        if let Some(step) = self.pending_step.as_ref() {
            return Ok(StepRunnerResult::PausedForApproval(step.node_id.clone()));
        }

        let Some(step) = self.runner.next_provider_step()? else {
            return Ok(StepRunnerResult::NoMoreSteps);
        };

        match self.policy.decision_for(&step.meta()) {
            ConfirmationDecision::PauseForConfirmation => {
                self.pending_step = Some(step.clone());
                Ok(StepRunnerResult::PausedForApproval(step.node_id))
            }
            ConfirmationDecision::RunAutomatically => self
                .runner
                .run_provider_step(step.clone(), step.prompt.clone()),
        }
    }

    pub fn confirm_pending(&mut self, prompt: String) -> Result<StepRunnerResult, TaskRunError> {
        let Some(step) = self.pending_step.take() else {
            return Err(TaskRunError::new(
                "interactive_no_pending_step",
                "no pending provider step to confirm",
            ));
        };

        self.runner.run_provider_step(step, prompt)
    }

    pub fn pending_step(&self) -> Option<&PendingProviderStep> {
        self.pending_step.as_ref()
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn task_id(&self) -> &str {
        &self.task_id
    }
}
