use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Deserializer};
use serde_json::json;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::{
    DEFAULT_PROVIDER_TIMEOUT_SECS, ProviderAdapter, ProviderAdapterError,
};
use crate::cross_cutting::streaming_provider::{
    ChoiceRequestData, PermissionRequestData, ProviderCommand, ProviderEvent,
    ProviderExecutionEvent, ProviderExecutionEventKind, ProviderExecutionEventStatus,
    ProviderPermissionMode, ProviderStatus, ProviderToolCall, ProviderToolResult, RiskLevel,
    StreamChunk, StreamingProviderAdapter, StreamingProviderInput,
};
use crate::cross_cutting::worktree::{scope_allows_path, validate_write_path};
use crate::product::coding_attempt_store::{
    CodingAttemptStore, CreateBlockedGateInput, CreateChoiceGateInput,
    CreateQualityBypassAuditInput,
};
use crate::product::coding_evaluation_context::{
    EvaluationContextRole, build_evaluation_context_pack,
};
use crate::product::coding_models::{
    AnalystDecisionNextStage, AnalystDecisionRecord, AnalystDecisionVerdict,
    AnalystHumanGateRecommendation, AnalystReworkInstructions, AnalystVerdict, CodeReviewReport,
    CodingAgentRole, CodingAttemptStatus, CodingChatEntry, CodingChoiceOption, CodingContextNote,
    CodingEntryType, CodingExecutionAttempt, CodingExecutionStage, CodingGateAction,
    CodingGateActionType, CodingGateRequired, CodingProviderPermissionMode, CodingProviderRole,
    CodingReworkInstruction, CodingRoleRun, CodingRoleRunEventType, CodingRoleRunStatus,
    CodingRoleRunTrigger, CodingTimelineNode, CodingTimelineNodeStatus, InternalPrReview,
    PushStatus, ReviewFinding, ReviewRequest, ReviewRequestKind, ReviewVerdict, TestCommand,
    TestCommandStatus, TestPlan, TestPlanRiskLevel, TestingOverallStatus, TestingReport,
    TestingStepResult, TestingUnplannedEvidence, WorkItemHandoff,
};
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::product::git_workspace_service::{GitWorkspaceError, GitWorkspaceService};
use crate::product::id::next_sequential_id;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    LifecycleWorkItemRecord, ProviderConversationRef, ProviderConversationRole, ProviderName,
    WorkItemStatus, WorkspaceType,
};
use crate::product::test_executor::{TestCommandSpec, TestExecutorError, run_all_tests};
use crate::product::tester_agent_loop::{
    TesterAgentOptions, build_plan_based_testing_report, build_tester_execute_repair_prompt,
    build_tester_plan_prompt, build_tester_plan_repair_prompt, build_testing_report,
    execute_tester_tool_call, format_test_plan_chat_summary, format_testing_report_chat_summary,
    parse_test_plan_payload,
};
use crate::protocol::contracts::ProviderType;
use crate::protocol::contracts::{AdapterInput, AdapterRole};
use crate::web::coding_ws_handler::CodingWsOutMessage;
use crate::web::workspace_ws_types::{
    ChoiceOption, WsExecutionEvent, WsExecutionEventKind, WsExecutionEventStatus,
    WsPermissionRiskLevel,
};

mod analyst_parser;
mod code_review;
mod gates;
mod handoffs;
mod internal_pr_review;
mod lifecycle;
mod prompts;
mod provider_stream;
mod reports;
mod review_parser;
mod rework;
mod testing;
mod testing_parser;
mod testing_provider;
mod timeline;
mod tool_format;
mod types;
mod ws_event_mapper;

pub use testing_parser::{
    testing_report_has_execution_evidence, testing_report_should_enter_analyst,
};
pub use types::{
    CodingExecutionContext, CodingWorkspaceEngine, CodingWorkspaceEngineError,
    CompletionGateReport, TESTING_RESULT_REVIEW_REASON_CODE,
};

#[allow(unused_imports)]
pub(crate) use analyst_parser::*;
#[allow(unused_imports)]
pub(crate) use prompts::*;
#[allow(unused_imports)]
pub(crate) use reports::*;
#[allow(unused_imports)]
pub(crate) use review_parser::*;
#[allow(unused_imports)]
pub(crate) use testing_parser::*;
#[allow(unused_imports)]
pub(crate) use tool_format::*;
#[allow(unused_imports)]
pub(crate) use types::*;
#[allow(unused_imports)]
pub(crate) use ws_event_mapper::*;

#[cfg(test)]
mod tests;
