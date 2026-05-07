use std::collections::VecDeque;

use crate::interactive::controller::{PendingProviderStep, StepRunner, StepRunnerResult};
use crate::task_run::types::TaskRunError;

#[derive(Debug, Clone, PartialEq)]
pub enum StepScriptItem {
    Provider(PendingProviderStep),
}

pub struct ScriptedStepRunner {
    queue: VecDeque<StepScriptItem>,
    last_peeked: Option<PendingProviderStep>,
}

impl ScriptedStepRunner {
    pub fn new(items: Vec<StepScriptItem>) -> Self {
        Self {
            queue: items.into(),
            last_peeked: None,
        }
    }
}

impl StepRunner for ScriptedStepRunner {
    fn next_provider_step(&mut self) -> Result<Option<PendingProviderStep>, TaskRunError> {
        match self.queue.front() {
            Some(StepScriptItem::Provider(step)) => {
                self.last_peeked = Some(step.clone());
                Ok(Some(step.clone()))
            }
            None => Ok(None),
        }
    }

    fn run_provider_step(
        &mut self,
        step: PendingProviderStep,
        prompt: String,
    ) -> Result<StepRunnerResult, TaskRunError> {
        let Some(StepScriptItem::Provider(expected)) = self.queue.front() else {
            return Err(TaskRunError::new(
                "scripted_step_missing",
                "no scripted provider step is available",
            ));
        };
        if expected.node_id != step.node_id
            || expected.provider_type != step.provider_type
            || expected.output_schema != step.output_schema
        {
            return Err(TaskRunError::new(
                "scripted_step_mismatch",
                format!(
                    "expected provider step {}:{} but got {}:{}",
                    expected.node_id, expected.provider_type, step.node_id, step.provider_type
                ),
            ));
        }

        let _ = self.queue.pop_front();
        self.last_peeked = None;
        Ok(StepRunnerResult::CompletedStep {
            node_id: step.node_id,
            provider_run_id: "scripted_provider_run".to_string(),
            prompt,
        })
    }
}
