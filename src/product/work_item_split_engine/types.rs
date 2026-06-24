use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::product::models::{
    IssueWorkItemDependencyEdge, IssueWorkItemPlan, LifecycleWorkItemRecord,
    OutlineContextBlockerResolution, ProviderName, RepositoryProfile, RepositoryProfileConfidence,
    VerificationFallbackPolicy, VerificationPlan, WorkItemContextBudget, WorkItemKind,
    WorkItemPlanOutline,
};
use crate::protocol::contracts::ProviderType;

#[derive(Debug, Clone)]
pub struct WorkItemSplitProviderOutput {
    pub repository_profile: RepositoryProfile,
    pub plan: IssueWorkItemPlan,
    pub work_items: Vec<LifecycleWorkItemRecord>,
    pub verification_plans: Vec<VerificationPlan>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct WorkItemPlanContextBlocker {
    pub code: String,
    pub message: String,
    pub needed_context: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutlineAuthorOutput {
    pub outline: Option<WorkItemPlanOutline>,
    pub context_blockers: Vec<WorkItemPlanContextBlocker>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItemDraftInvocation {
    pub prompt: String,
    pub sentinel_nonce: String,
}

/// 被重做的 WorkItem 规格：旧 id + 用户反馈。
#[derive(Debug, Clone)]
pub struct RedoSpec {
    pub old_id: String,
    pub feedback: String,
}

#[derive(Debug, Clone)]
pub struct WorkItemSplitInvocation {
    pub prompt: String,
    pub provider_type: ProviderType,
    pub worktree_path: String,
    pub author_provider: ProviderName,
    pub sentinel_nonce: String,
}

#[derive(Debug)]
pub(crate) struct ProviderInvocationResult {
    pub(crate) structured_output: serde_json::Value,
    pub(crate) run_ref: String,
}

/// DAG 重连：把 graph 中对旧 id 的引用改为新 id。
///
/// `id_mapping`: old_id → new_id。只重写映射中存在的 id，未映射的边原样保留。
pub fn repatch_dependencies(
    graph: &[IssueWorkItemDependencyEdge],
    id_mapping: &HashMap<String, String>,
) -> Vec<IssueWorkItemDependencyEdge> {
    graph
        .iter()
        .map(|edge| IssueWorkItemDependencyEdge {
            from_work_item_id: id_mapping
                .get(&edge.from_work_item_id)
                .cloned()
                .unwrap_or_else(|| edge.from_work_item_id.clone()),
            to_work_item_id: id_mapping
                .get(&edge.to_work_item_id)
                .cloned()
                .unwrap_or_else(|| edge.to_work_item_id.clone()),
        })
        .collect()
}

pub(crate) fn provider_name_to_type(name: &ProviderName) -> ProviderType {
    match name {
        ProviderName::ClaudeCode => ProviderType::ClaudeCode,
        ProviderName::Codex => ProviderType::Codex,
        ProviderName::Fake => ProviderType::Fake,
    }
}

pub(crate) fn parse_work_item_kind(value: &str) -> WorkItemKind {
    match value {
        "backend" => WorkItemKind::Backend,
        "frontend" => WorkItemKind::Frontend,
        "integration" => WorkItemKind::Integration,
        "e2e" => WorkItemKind::E2e,
        "docs" => WorkItemKind::Docs,
        "infra" => WorkItemKind::Infra,
        _ => WorkItemKind::Other,
    }
}

pub(crate) fn parse_confidence(value: &str) -> RepositoryProfileConfidence {
    match value {
        "high" => RepositoryProfileConfidence::High,
        "low" => RepositoryProfileConfidence::Low,
        _ => RepositoryProfileConfidence::Medium,
    }
}

pub(crate) fn parse_verification_scope(value: &str) -> crate::product::models::VerificationScope {
    use crate::product::models::VerificationScope;
    match value {
        "unit" => VerificationScope::Unit,
        "integration" => VerificationScope::Integration,
        "e2e" => VerificationScope::E2e,
        "build" => VerificationScope::Build,
        "lint" => VerificationScope::Lint,
        "manual" => VerificationScope::Manual,
        _ => VerificationScope::Custom,
    }
}

pub(crate) fn parse_safety(value: &str) -> crate::product::models::VerificationCommandSafety {
    use crate::product::models::VerificationCommandSafety;
    match value {
        "approved" => VerificationCommandSafety::Approved,
        _ => VerificationCommandSafety::NeedsManualReview,
    }
}

pub(crate) fn parse_fallback_policy(value: &str) -> VerificationFallbackPolicy {
    match value {
        "repair_provider_output" => VerificationFallbackPolicy::RepairProviderOutput,
        _ => VerificationFallbackPolicy::ManualGate,
    }
}

pub(crate) fn work_item_kind_text(kind: &WorkItemKind) -> &'static str {
    match kind {
        WorkItemKind::Backend => "backend",
        WorkItemKind::Frontend => "frontend",
        WorkItemKind::Integration => "integration",
        WorkItemKind::E2e => "e2e",
        WorkItemKind::Docs => "docs",
        WorkItemKind::Infra => "infra",
        WorkItemKind::Other => "other",
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct ProviderOutlineAuthorOutput {
    #[serde(default)]
    pub(crate) outline: Option<WorkItemPlanOutline>,
    #[serde(default)]
    pub(crate) context_blockers: Vec<WorkItemPlanContextBlocker>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct ProviderRepositoryProfile {
    pub(crate) confidence: String,
    pub(crate) detected_layers: Vec<String>,
    pub(crate) split_recommendation: String,
    #[serde(default)]
    pub(crate) languages: Vec<String>,
    #[serde(default)]
    pub(crate) frameworks: Vec<String>,
    #[serde(default)]
    pub(crate) package_managers: Vec<String>,
    #[serde(default)]
    pub(crate) test_frameworks: Vec<String>,
    #[serde(default)]
    pub(crate) build_systems: Vec<String>,
    #[serde(default)]
    pub(crate) verification_capabilities: Vec<String>,
    #[serde(default)]
    pub(crate) uncertainties: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct ProviderWorkItem {
    pub(crate) title: String,
    /// Provider 习惯输出 `type` 而非 `kind`,接受别名以兼容真实 claude 输出。
    /// 合法取值见 `parse_work_item_kind`: backend/frontend/integration/e2e/docs/infra/other。
    #[serde(alias = "type")]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) sequence_hint: Option<u32>,
    #[serde(default)]
    pub(crate) depends_on: Vec<usize>,
    #[serde(default)]
    pub(crate) exclusive_write_scopes: Vec<String>,
    #[serde(default)]
    pub(crate) forbidden_write_scopes: Vec<String>,
    #[serde(default)]
    pub(crate) context_budget: Option<WorkItemContextBudget>,
    #[serde(default)]
    pub(crate) required_handoff_from: Vec<String>,
    #[serde(default)]
    pub(crate) require_execution_plan_confirm: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct ProviderVerificationCommand {
    pub(crate) id: Option<String>,
    pub(crate) label: String,
    pub(crate) command: String,
    #[serde(default)]
    pub(crate) cwd: String,
    pub(crate) purpose: String,
    #[serde(default)]
    pub(crate) required: bool,
    #[serde(default = "default_timeout")]
    pub(crate) timeout_seconds: u64,
    #[serde(default)]
    pub(crate) safety: String,
}

fn default_timeout() -> u64 {
    300
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct ProviderVerificationManualCheck {
    pub(crate) id: Option<String>,
    pub(crate) label: String,
    pub(crate) instructions: String,
    #[serde(default)]
    pub(crate) required: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct ProviderVerificationPlan {
    #[serde(default)]
    pub(crate) scope: String,
    #[serde(default)]
    pub(crate) commands: Vec<ProviderVerificationCommand>,
    #[serde(default)]
    pub(crate) manual_checks: Vec<ProviderVerificationManualCheck>,
    #[serde(default)]
    pub(crate) required_gates: Vec<String>,
    #[serde(default)]
    pub(crate) risk_notes: Vec<String>,
    #[serde(default)]
    pub(crate) confidence: String,
    #[serde(default)]
    pub(crate) fallback_policy: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) struct ProviderOutput {
    pub(crate) repository_profile: ProviderRepositoryProfile,
    pub(crate) work_items: Vec<ProviderWorkItem>,
    pub(crate) verification_plans: Vec<ProviderVerificationPlan>,
}

pub(crate) fn structured_output_nonce() -> String {
    uuid::Uuid::new_v4()
        .simple()
        .to_string()
        .chars()
        .take(8)
        .collect()
}

pub(crate) fn prompt_nonce(prompt: &str) -> String {
    prompt
        .split_once("<ARIA_STRUCTURED_OUTPUT nonce=\"")
        .and_then(|(_, tail)| tail.split_once('"'))
        .map(|(nonce, _)| nonce.to_string())
        .unwrap_or_default()
}

pub(crate) fn format_string_list(values: &[String]) -> String {
    if values.is_empty() {
        "(none)".to_string()
    } else {
        values
            .iter()
            .map(|value| format!("- {value}"))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

pub(crate) fn format_context_resolutions(
    resolutions: &[OutlineContextBlockerResolution],
) -> String {
    if resolutions.is_empty() {
        return "(none)".to_string();
    }

    resolutions
        .iter()
        .map(|resolution| {
            let summary = resolution.summary.as_deref().unwrap_or("(none)");
            format!(
                "- blocker_node_id: {blocker}\n  resolution_node_id: {resolution_node}\n  artifact_ref: {artifact_ref}\n  estimated_tokens: {tokens}\n  summary: {summary}",
                blocker = resolution.blocker_node_id,
                resolution_node = resolution.resolution_node_id,
                artifact_ref = resolution.resolution_artifact_ref,
                tokens = resolution.estimated_tokens,
                summary = summary
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn product_store_api_error(
    error: crate::product::json_store::ProductStoreError,
) -> crate::web::error::ApiError {
    crate::web::error::ApiError::runtime("product_store_error", error.to_string(), json!({}))
}
