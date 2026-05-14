use std::collections::VecDeque;
use std::path::PathBuf;

use serde_json::json;

use crate::interactive::controller::{PendingProviderStep, StepRunner, StepRunnerResult};
use crate::interactive::policy::NodeWriteClass;
use crate::task_run::types::TaskRunError;
use crate::web::types::CreateTaskRequest;

pub struct InteractiveTaskRunner {
    steps: VecDeque<InteractiveStep>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InteractiveStep {
    Internal {
        node_id: String,
        action: String,
        artifact_refs: Vec<String>,
    },
    Provider(Box<PendingProviderStep>),
}

impl InteractiveTaskRunner {
    pub fn new_fake(
        _workspace_root: PathBuf,
        _request: CreateTaskRequest,
    ) -> Result<Self, TaskRunError> {
        Ok(Self {
            steps: vec![
                InteractiveStep::Internal {
                    node_id: "N00".to_string(),
                    action: "bootstrap runtime".to_string(),
                    artifact_refs: vec!["internal_n00".to_string()],
                },
                InteractiveStep::Provider(Box::new(step(
                    "N04",
                    "claude_code",
                    NodeWriteClass::WritesRuntime,
                ))),
                InteractiveStep::Provider(Box::new(step(
                    "N10",
                    "claude_code",
                    NodeWriteClass::WritesRuntime,
                ))),
                InteractiveStep::Provider(Box::new(step(
                    "N16",
                    "codex",
                    NodeWriteClass::WritesWorkspace,
                ))),
                InteractiveStep::Provider(Box::new(step("N17", "codex", NodeWriteClass::ReadOnly))),
                InteractiveStep::Provider(Box::new(step(
                    "N25",
                    "claude_code",
                    NodeWriteClass::WritesRuntime,
                ))),
            ]
            .into(),
        })
    }

    pub fn next_step(&mut self) -> Result<Option<InteractiveStep>, TaskRunError> {
        Ok(self.steps.front().cloned())
    }

    pub fn run_internal_step(&mut self, node_id: &str) -> Result<StepRunnerResult, TaskRunError> {
        match self.steps.pop_front() {
            Some(InteractiveStep::Internal {
                node_id: expected, ..
            }) if expected == node_id => Ok(StepRunnerResult::CompletedStep {
                node_id: expected,
                provider_run_id: "internal".to_string(),
                prompt: String::new(),
            }),
            Some(_) => Err(TaskRunError::new(
                "interactive_runner_step_mismatch",
                "expected internal step",
            )),
            None => Err(TaskRunError::new(
                "interactive_runner_empty",
                "no step available",
            )),
        }
    }
}

impl StepRunner for InteractiveTaskRunner {
    fn next_provider_step(&mut self) -> Result<Option<PendingProviderStep>, TaskRunError> {
        Ok(self.steps.iter().find_map(|step| match step {
            InteractiveStep::Provider(step) => Some(step.as_ref().clone()),
            InteractiveStep::Internal { .. } => None,
        }))
    }

    fn run_provider_step(
        &mut self,
        step: PendingProviderStep,
        prompt: String,
    ) -> Result<StepRunnerResult, TaskRunError> {
        let index = self
            .steps
            .iter()
            .position(|candidate| matches!(candidate, InteractiveStep::Provider(_)))
            .ok_or_else(|| {
                TaskRunError::new("interactive_runner_empty", "no pending provider step")
            })?;
        let expected = match self.steps.remove(index) {
            Some(InteractiveStep::Provider(expected)) => *expected,
            _ => {
                return Err(TaskRunError::new(
                    "interactive_runner_empty",
                    "no pending provider step",
                ));
            }
        };
        if expected.node_id != step.node_id {
            return Err(TaskRunError::new(
                "interactive_runner_step_mismatch",
                format!("expected {} got {}", expected.node_id, step.node_id),
            ));
        }
        Ok(StepRunnerResult::CompletedStep {
            node_id: step.node_id,
            provider_run_id: "interactive_fake_provider_run".to_string(),
            prompt,
        })
    }
}

fn step(node_id: &str, provider_type: &str, write_class: NodeWriteClass) -> PendingProviderStep {
    PendingProviderStep {
        node_id: node_id.to_string(),
        provider_type: provider_type.to_string(),
        runtime_role: "executor".to_string(),
        adapter_role: "executor".to_string(),
        prompt: format!("执行 {node_id}"),
        input_summary: json!({"node_id": node_id}),
        output_schema: "schema://aria/artifacts/provider_output/v1".to_string(),
        write_class,
        allowed_write_scope: vec![
            ".aria/runtime/".to_string(),
            "openspec/".to_string(),
            "src/".to_string(),
            "tests/".to_string(),
        ],
        forbidden_actions: vec!["修改 cadence/project-rules".to_string()],
        verification_commands: vec!["cargo test --locked -j 1".to_string()],
        checkpoint_id: None,
    }
}
