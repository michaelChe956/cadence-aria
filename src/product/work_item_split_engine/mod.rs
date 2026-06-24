use std::sync::Arc;

use crate::cross_cutting::provider_adapter::ProviderAdapter;

pub(crate) mod context;
pub(crate) mod engine;
pub(crate) mod parse;
pub(crate) mod prompts;
pub(crate) mod revision;
pub(crate) mod schema;
pub(crate) mod types;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub(crate) use context::*;
#[allow(unused_imports)]
pub(crate) use schema::*;
#[allow(unused_imports)]
pub(crate) use types::*;

// 对外 API 保持与拆分前完全一致。
pub use context::{
    design_context_capabilities_for_request, design_context_gaps,
    extract_design_context_capabilities,
};
pub use parse::{
    build_work_item_draft_invocation, parse_work_item_draft_output,
    parse_work_item_plan_outline_output,
};
pub use types::{
    OutlineAuthorOutput, RedoSpec, WorkItemDraftInvocation, WorkItemPlanContextBlocker,
    WorkItemSplitInvocation, WorkItemSplitProviderOutput, repatch_dependencies,
};

/// 主入口：WorkItem 拆分引擎。
#[derive(Clone)]
pub struct WorkItemSplitEngine {
    provider_adapter: Arc<dyn ProviderAdapter + Send + Sync>,
}

impl WorkItemSplitEngine {
    pub fn new(provider_adapter: Arc<dyn ProviderAdapter + Send + Sync>) -> Self {
        Self { provider_adapter }
    }
}
