mod draft;
pub(crate) mod outline;
mod plan;
mod types;
mod utils;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, HashSet};

use crate::cross_cutting::worktree::scopes_may_overlap;
use crate::product::models::{
    IssueWorkItemPlan, LifecycleWorkItemRecord, RepositoryProfile, RepositoryProfileConfidence,
    VerificationCommandSafety, VerificationCommandSource, VerificationPlan, WorkItemDraftCandidate,
    WorkItemKind, WorkItemOutline, WorkItemOutlineSessionFit, WorkItemPlanOutline,
    WorkItemSplitFinding, WorkItemSplitFindingSeverity,
};

pub use types::{
    WorkItemDraftLocalValidator, WorkItemPlanOutlineValidator, WorkItemSplitValidationReport,
    WorkItemSplitValidator,
};

/// options×items 三族预检的共享修复路径文案（F-51）。finding 消息、SC author
/// prompt 教学与机械 verdict 适配 MUST 引用同一常量（REQ-WSC-06 口径一致），
/// 禁止在消费侧重述第二份文案。
pub const INTEGRATION_WORK_ITEM_REQUIRED_REPAIR_ACTION: &str =
    "新增一个 kind=integration 的 Work Item（## Work Item WI-XXX 条目且 - kind: integration），或回创建计划选项关闭 include_integration_tests";
pub const E2E_WORK_ITEM_REQUIRED_REPAIR_ACTION: &str =
    "新增一个 kind=e2e 的 Work Item（## Work Item WI-XXX 条目且 - kind: e2e），或回创建计划选项关闭 include_e2e_tests";
pub const FRONTEND_BACKEND_SPLIT_REQUIRED_REPAIR_ACTION: &str =
    "补齐 frontend 与 backend 两类 Work Item（各含 - kind: frontend 与 - kind: backend 条目），或回创建计划选项关闭 force_frontend_backend_split";

use utils::{compute_reachability, error, is_command_unsafe, is_cwd_inside_repository, warning};
