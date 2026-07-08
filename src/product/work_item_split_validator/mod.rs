mod draft;
mod outline;
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

use utils::{compute_reachability, error, is_command_unsafe, is_cwd_inside_repository, warning};
