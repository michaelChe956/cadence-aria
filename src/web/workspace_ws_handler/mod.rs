pub(crate) use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

pub(crate) use axum::extract::ws::{CloseFrame, Message, WebSocket, close_code};
pub(crate) use axum::extract::{Path, State, WebSocketUpgrade};
pub(crate) use axum::http::StatusCode;
pub(crate) use axum::response::IntoResponse;
pub(crate) use futures_util::{SinkExt, StreamExt};
pub(crate) use tokio::sync::{Mutex, mpsc};
pub(crate) use tokio_util::sync::CancellationToken;

pub(crate) use crate::cross_cutting::provider_adapter::parse_last_structured_output;
pub(crate) use crate::cross_cutting::provider_registry::ProviderRegistry;
pub(crate) use crate::cross_cutting::streaming_provider::{
    ChoiceOptionData, ChoiceRequestSource, ProviderCommand, ProviderExecutionEvent,
    ProviderExecutionEventKind, ProviderExecutionEventStatus, ProviderStatus, RiskLevel,
    StreamingProviderAdapter,
};
pub(crate) use crate::product::app_paths::ProductAppPaths;
pub(crate) use crate::product::checkpoint_store::CheckpointStore;
pub(crate) use crate::product::issue_store::IssueStore;
pub(crate) use crate::product::lifecycle_store::LifecycleStore;
pub(crate) use crate::product::models::{
    OutlineContextBlockerResolution, OutlineContextIndex, ProviderName, WorkItemSplitFinding,
    WorkspaceSessionRecord, WorkspaceType,
};
pub(crate) use crate::product::work_item_plan_store::WorkItemPlanStore;
pub(crate) use crate::product::work_item_split_engine::{
    WorkItemSplitEngine, design_context_capabilities_for_request, design_context_gaps,
    parse_work_item_draft_output, parse_work_item_plan_outline_output,
};
pub(crate) use crate::product::workspace_engine::{
    AuthorDecisionOutcome, EngineEvent, PendingAuthorChoiceError, ReviewDecisionOutcome,
    WorkItemBatchDecisionOutcome, WorkItemDraftDecisionOutcome, WorkItemPlanAuthorOutcome,
    WorkItemPlanCompileRecoveryOutcome, WorkspaceEngine, WorkspaceSession, WorkspaceStage,
    build_work_item_plan_revision_input,
};
pub(crate) use crate::product::workspace_repository::workspace_repository_for_session;
pub(crate) use crate::web::state::{WebAppState, WorkspaceActiveRun, WorkspaceRunRegistry};
pub(crate) use crate::web::test_controls::WorkspaceSocketControl;
pub(crate) use crate::web::types::GenerateWorkItemsRequest;
pub(crate) use crate::web::workspace_context::ensure_workspace_context_message;
pub(crate) use crate::web::workspace_ws_types::{
    ChoiceOption, HumanConfirmDecision, RevisionPath, TimelineNodeRetryError,
    WorkItemGenerationModeDto, WsExecutionEvent, WsExecutionEventKind, WsExecutionEventStatus,
    WsInMessage, WsOutMessage, WsPermissionRiskLevel, WsProviderStatus,
};

mod decisions;
mod mapping;
mod protocol;
mod run;
mod socket;

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests;

pub use socket::workspace_ws;

pub(crate) use decisions::*;
pub(crate) use mapping::*;
pub(crate) use protocol::*;
pub(crate) use run::*;
pub(crate) use socket::{OutboundControl, send_json_outbound};

#[cfg(test)]
pub(crate) use socket::spawn_idle_timeout_task;
