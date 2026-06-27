use std::collections::VecDeque;

use crate::interactive::controller::{PendingProviderStep, StepRunner, StepRunnerResult};
use crate::interactive::policy::NodeWriteClass;
use crate::protocol::contracts::{AdapterInput, AdapterRole, ProviderType};
use crate::task_run::types::TaskRunError;
use serde_json::json;

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

pub fn provider_step_from_adapter_input(
    node_id: &str,
    input: &AdapterInput,
) -> Result<PendingProviderStep, TaskRunError> {
    Ok(PendingProviderStep {
        node_id: node_id.to_string(),
        provider_type: provider_type_text(&input.provider_type).to_string(),
        runtime_role: adapter_role_text(&input.role).to_string(),
        adapter_role: adapter_role_text(&input.role).to_string(),
        prompt: input.prompt.clone(),
        input_summary: json!({
            "worktree_path": input.worktree_path,
            "context_files": input.context_files,
            "timeout": input.timeout,
            "max_retries": input.max_retries,
        }),
        output_schema: input.output_schema.clone(),
        write_class: write_class_for_node(node_id),
        allowed_write_scope: allowed_write_scope_for_node(node_id),
        forbidden_actions: vec![
            "不要修改 .claude/rules".to_string(),
            "不要修改 cadence/project-rules".to_string(),
        ],
        verification_commands: verification_commands_for_node(node_id),
        checkpoint_id: None,
    })
}

fn adapter_role_text(role: &AdapterRole) -> &'static str {
    match role {
        AdapterRole::Orchestrator => "orchestrator",
        AdapterRole::Reviewer => "reviewer",
        AdapterRole::Executor => "executor",
        AdapterRole::WorkItemSplitter => "work_item_splitter",
        AdapterRole::Handoff => "handoff",
    }
}

fn provider_type_text(provider_type: &ProviderType) -> &'static str {
    match provider_type {
        ProviderType::ClaudeCode => "claude_code",
        ProviderType::Codex => "codex",
        ProviderType::Fake => "fake",
    }
}

fn write_class_for_node(node_id: &str) -> NodeWriteClass {
    match node_id {
        "N16" | "N19" => NodeWriteClass::WritesWorkspace,
        "N04" | "N05" | "N07" | "N09" | "N10" | "N11" | "N12" | "N25" | "N26" | "N27" => {
            NodeWriteClass::WritesRuntime
        }
        _ => NodeWriteClass::ReadOnly,
    }
}

fn allowed_write_scope_for_node(node_id: &str) -> Vec<String> {
    match node_id {
        "N16" | "N19" => vec!["src/".to_string(), "tests/".to_string()],
        "N04" | "N05" | "N07" | "N09" | "N10" | "N11" | "N12" | "N25" | "N26" | "N27" => {
            vec![".aria/runtime/".to_string(), "openspec/".to_string()]
        }
        _ => Vec::new(),
    }
}

fn verification_commands_for_node(node_id: &str) -> Vec<String> {
    match node_id {
        "N16" | "N17" | "N18" | "N19" => vec!["cargo test --locked -j 1".to_string()],
        "N25" | "N27" => vec!["cargo test --locked -j 1".to_string()],
        _ => vec!["cargo check --locked".to_string()],
    }
}
