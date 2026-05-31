use std::collections::{HashMap, HashSet};

use crate::product::models::{ExecutionMode, WorkItemRecord, WorkItemStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadyDecision {
    Ready,
    WaitingForDependency,
    WaitingForScope,
    NotAgentExecutable,
    NotPending,
}

pub fn ready_work_items(
    items: &[WorkItemRecord],
    completed: &[String],
    active_scopes: &[String],
) -> HashMap<String, ReadyDecision> {
    let completed = completed.iter().cloned().collect::<HashSet<_>>();
    items
        .iter()
        .map(|item| {
            let decision = if item.status != WorkItemStatus::Pending {
                ReadyDecision::NotPending
            } else if item.execution_mode != ExecutionMode::Agent {
                ReadyDecision::NotAgentExecutable
            } else if item.depends_on.iter().any(|dep| !completed.contains(dep)) {
                ReadyDecision::WaitingForDependency
            } else if item.allowed_write_scope.iter().any(|scope| {
                active_scopes
                    .iter()
                    .any(|active| scopes_may_overlap(scope, active))
            }) {
                ReadyDecision::WaitingForScope
            } else {
                ReadyDecision::Ready
            };
            (item.id.clone(), decision)
        })
        .collect()
}

fn scopes_may_overlap(left: &str, right: &str) -> bool {
    let left_scope = vec![left.to_string()];
    let right_scope = vec![right.to_string()];
    crate::cross_cutting::worktree::scopes_may_overlap(&left_scope, &right_scope, true)
}
