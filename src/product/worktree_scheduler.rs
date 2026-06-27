use std::collections::{HashMap, HashSet};

use crate::product::models::{LifecycleWorkItemRecord, WorkItemStatus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadyDecision {
    Ready,
    WaitingForDependency,
    WaitingForScope,
    // TODO(P1): 迁移到 LifecycleWorkItemRecord 后当前无分支构造该变体；
    // 保留是为了 API 兼容，待 P3 接入 agent 可执行性判断时复用或清理。
    NotAgentExecutable,
    NotPending,
}

pub fn ready_work_items(
    items: &[LifecycleWorkItemRecord],
    completed: &[String],
    active_scopes: &[String],
) -> HashMap<String, ReadyDecision> {
    let completed = completed.iter().cloned().collect::<HashSet<_>>();
    items
        .iter()
        .map(|item| {
            let decision = if item.execution_status != WorkItemStatus::Pending {
                ReadyDecision::NotPending
            } else if item.depends_on.iter().any(|dep| !completed.contains(dep)) {
                ReadyDecision::WaitingForDependency
            } else if item.exclusive_write_scopes.iter().any(|scope| {
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
