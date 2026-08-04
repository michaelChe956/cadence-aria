use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::time::Duration;

use chrono::Utc;
use serde::{Deserialize, Deserializer};
use serde_json::json;
use thiserror::Error;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::provider_adapter::{DEFAULT_PROVIDER_TIMEOUT_SECS, ProviderAdapterError};
use crate::cross_cutting::streaming_provider::{
    ChoiceRequestData, PermissionRequestData, ProviderCommand, ProviderEvent,
    ProviderExecutionEvent, ProviderExecutionEventKind, ProviderExecutionEventStatus,
    ProviderPermissionMode, ProviderStatus, ProviderToolCall, ProviderToolResult, RiskLevel,
    StreamChunk, StreamingProviderAdapter, StreamingProviderInput,
};
use crate::cross_cutting::worktree::{scope_allows_path, validate_write_path};
use crate::product::coding_attempt_store::{
    CodingAttemptStore, CodingGitOperationJournal, CodingGitOperationKind, CodingGitOperationPhase,
    CompleteReviewGitOperationInput, CreateBlockedGateInput, CreateChoiceGateInput,
    CreateQualityBypassAuditInput, PrepareCodingGitOperationInput,
};
use crate::product::coding_evaluation_context::{
    EvaluationContextRole, build_evaluation_context_pack,
};
use crate::product::coding_models::{
    CodeReviewReport, CodingAgentRole, CodingAttemptStatus, CodingChatEntry, CodingChoiceOption,
    CodingContextNote, CodingEntryType, CodingExecutionAttempt, CodingExecutionStage,
    CodingGateAction, CodingGateActionType, CodingGateRequired, CodingProviderPermissionMode,
    CodingProviderRole, CodingReworkInstruction, CodingRoleRun, CodingRoleRunEventType,
    CodingRoleRunStatus, CodingRoleRunTrigger, CodingTimelineNode, CodingTimelineNodeStatus,
    FindingSeverity, InternalPrReview, PushStatus, ReviewFinding, ReviewRequest, ReviewRequestKind,
    ReviewVerdict,
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
use crate::product::workspace_engine::permission_mode_for_provider_type;
use crate::protocol::contracts::ProviderType;
use crate::protocol::contracts::{AdapterInput, AdapterRole};
use crate::web::coding_ws_handler::CodingWsOutMessage;
use crate::web::workspace_ws_types::{
    ChoiceOption, WsExecutionEvent, WsExecutionEventKind, WsExecutionEventStatus,
    WsPermissionRiskLevel,
};

mod amendment;
mod code_review;
mod coding;
mod failed_review_recovery;
mod gates;
mod git_operation;
mod group;
mod group_completion;
mod group_review_budget;
mod group_review_material;
mod group_review_types;
mod handoffs;
mod internal_pr_review;
mod lifecycle;
#[cfg(test)]
mod mutation_test_pause;
mod plan_defect;
mod plan_defect_routing;
mod plan_repair_start;
mod prompts;
mod provider_failure;
mod provider_stream;
mod reports;
mod review_parser;
mod reviewer_context;
mod rework;
mod runtime_handoff_authority;
mod runtime_impact;
mod timeline;
mod tool_format;
mod types;
mod ws_event_mapper;

#[cfg(test)]
pub(crate) use mutation_test_pause::{
    CodingMutationTestPoint, register_coding_mutation_test_pause,
};
#[cfg(test)]
pub(crate) use plan_repair_start::register_plan_repair_start_snapshot_request_pause;

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

pub use runtime_impact::{
    HandoffDeltaKind, RuntimeHandoffImpactPropagator, RuntimeHandoffImpactResult,
    compare_handoff_revisions,
};
pub use types::{
    CodingExecutionContext, CodingWorkspaceEngine, CodingWorkspaceEngineError, CompletionGateReport,
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

fn balanced_json_object_end(output: &str, start: usize) -> Option<usize> {
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
                    return Some(start + offset + ch.len_utf8());
                }
            }
            _ => {}
        }
    }
    None
}

/// 按出现顺序提取输出中的全部顶层平衡 JSON 对象。
///
/// Provider 输出可能在路由回执、证据表格或示例中夹带与结论无关的 JSON
/// 片段（例如 `{"type": "module"}`），调用方需要遍历候选并按 Schema
/// 校验挑选真正的结构化结论，而不是盲目信任第一个 `{`。
pub(crate) fn extract_json_object_candidates(output: &str) -> Vec<&str> {
    let mut candidates = Vec::new();
    let mut search_from = 0usize;
    while let Some(relative) = output[search_from..].find('{') {
        let start = search_from + relative;
        match balanced_json_object_end(output, start) {
            Some(end) => {
                candidates.push(&output[start..end]);
                search_from = end;
            }
            None => {
                // 当前 `{` 无法平衡（可能是散文中的括号），跳过它继续找后续候选。
                search_from = start + 1;
            }
        }
    }
    candidates
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
pub(crate) use tool_format::*;
#[allow(unused_imports)]
pub(crate) use types::*;
#[allow(unused_imports)]
pub(crate) use ws_event_mapper::*;

#[cfg(test)]
mod tests;
