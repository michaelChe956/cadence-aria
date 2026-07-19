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
    EvaluationContextRole, build_evaluation_context_pack, build_tester_execution_context_pack,
};
use crate::product::coding_models::{
    CodeReviewReport, CodingAgentRole, CodingAttemptStatus, CodingChatEntry, CodingChoiceOption,
    CodingContextNote, CodingEntryType, CodingExecutionAttempt, CodingExecutionStage,
    CodingGateAction, CodingGateActionType, CodingGateRequired, CodingProviderPermissionMode,
    CodingProviderRole, CodingReworkInstruction, CodingRoleRun, CodingRoleRunEventType,
    CodingRoleRunStatus, CodingRoleRunTrigger, CodingTimelineNode, CodingTimelineNodeStatus,
    FindingSeverity, InternalPrReview, PushStatus, ReviewFinding, ReviewRequest, ReviewRequestKind,
    ReviewVerdict, TestCommand, TestCommandStatus, TestPlan, TestPlanRiskLevel,
    TestingOverallStatus, TestingReport, TestingStepResult, TestingUnplannedEvidence,
    WorkItemHandoff,
};
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::product::git_workspace_service::{GitWorkspaceError, GitWorkspaceService};
use crate::product::id::next_sequential_id;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    LifecycleWorkItemRecord, ProviderConversationRef, ProviderConversationRole, ProviderName,
    WorkItemStatus,
};
use crate::product::test_executor::{TestCommandSpec, TestExecutorError, run_all_tests};
use crate::product::tester_agent_loop::{
    ProviderTestExecutionPayload, TestContextLoader, TesterAgentOptions,
    build_plan_based_testing_report, build_tester_execute_repair_prompt, build_tester_plan_prompt,
    build_tester_plan_repair_prompt, build_testing_report, execute_tester_tool_call_with_context,
    format_test_plan_chat_summary, format_testing_report_chat_summary, parse_test_plan_payload,
};
use crate::protocol::contracts::ProviderType;
use crate::protocol::contracts::{AdapterInput, AdapterRole};
use crate::web::coding_ws_handler::CodingWsOutMessage;
use crate::web::workspace_ws_types::{
    ChoiceOption, WsExecutionEvent, WsExecutionEventKind, WsExecutionEventStatus,
    WsPermissionRiskLevel,
};

mod code_review;
mod coding;
mod failed_review_recovery;
mod gates;
mod group;
mod handoffs;
mod internal_pr_review;
mod lifecycle;
mod plan_defect;
mod plan_defect_routing;
mod prompts;
mod provider_failure;
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

pub(crate) struct CoderOutputChatEntryInput<'a> {
    pub(crate) attempt: &'a CodingExecutionAttempt,
    pub(crate) node_id: &'a str,
    pub(crate) provider_name: &'a ProviderName,
    pub(crate) role_run: &'a CodingRoleRun,
    pub(crate) full_output: &'a str,
    pub(crate) raw_provider_output_ref: &'a str,
    pub(crate) source: &'a str,
    pub(crate) plan_defect_route: Option<&'a str>,
}

pub use testing_parser::{
    testing_report_has_execution_evidence, testing_report_needs_blocked_gate,
};
pub use types::{
    CodingExecutionContext, CodingWorkspaceEngine, CodingWorkspaceEngineError,
    CompletionGateReport, ProviderTestingAdapters, TESTING_RESULT_REVIEW_REASON_CODE,
};

pub(crate) fn code_review_report_has_actionable_findings(report: &CodeReviewReport) -> bool {
    review_findings_have_actionable_findings(&report.findings)
}

pub(crate) fn review_findings_have_actionable_findings(findings: &[ReviewFinding]) -> bool {
    findings.iter().any(|finding| {
        matches!(
            finding.severity,
            FindingSeverity::Error | FindingSeverity::Warning
        ) && (!finding.message.trim().is_empty()
            || finding
                .required_action
                .as_deref()
                .is_some_and(|action| !action.trim().is_empty())
            || finding
                .file_path
                .as_deref()
                .is_some_and(|path| !path.trim().is_empty()))
    })
}

pub(crate) fn extract_json_object(output: &str) -> Option<&str> {
    let start = output.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (offset, ch) in output[start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
                continue;
            }
            match ch {
                '\\' => escaped = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(&output[start..start + offset + ch.len_utf8()]);
                }
            }
            _ => {}
        }
    }
    None
}

#[allow(unused_imports)]
pub(crate) use failed_review_recovery::{FailedCodeReviewRecovery, recoverable_failed_code_review};
#[allow(unused_imports)]
pub(crate) use gates::*;
#[allow(unused_imports)]
pub(crate) use group::*;
#[allow(unused_imports)]
pub(crate) use internal_pr_review::internal_review_blocked_gate_reason;
#[allow(unused_imports)]
pub(crate) use plan_defect::*;
#[allow(unused_imports)]
pub(crate) use plan_defect_routing::*;
#[allow(unused_imports)]
pub(crate) use prompts::*;
#[allow(unused_imports)]
pub(crate) use provider_failure::*;
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
