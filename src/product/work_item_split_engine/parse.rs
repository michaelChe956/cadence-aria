use std::collections::HashSet;

use serde_json::json;

use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    IssueRecord, IssueWorkItemDependencyEdge, IssueWorkItemPlan, IssueWorkItemPlanOptions,
    IssueWorkItemPlanStatus, LifecycleWorkItemRecord, ProviderName, RepositoryProfile,
    RepositoryRecord, VerificationCommand, VerificationCommandSource, VerificationManualCheck,
    VerificationPlan, WorkItemDraftCandidate, WorkItemDraftRecord, WorkItemExecutionPlanStatus,
    WorkItemGenerationMode, WorkItemPlanOutline, WorkItemPlanStatus, WorkItemStatus,
};
use crate::web::error::{ApiError, ApiResult};
use crate::web::types::GenerateWorkItemsRequest;

use super::WorkItemSplitEngine;
use super::prompts::{WORK_ITEM_DRAFT_PROMPT_MAX_BYTES, build_work_item_draft_prompt};
use super::types::{
    ProviderOutput, ProviderWorkItemDraftInput, WorkItemDraftInvocation,
    WorkItemSplitProviderOutput, parse_confidence, parse_fallback_policy, parse_safety,
    parse_verification_scope, parse_work_item_kind, product_store_api_error,
};

impl WorkItemSplitEngine {
    pub fn complete_generate_from_structured_output(
        request: &GenerateWorkItemsRequest,
        lifecycle: &LifecycleStore,
        issue: &IssueRecord,
        repository: &RepositoryRecord,
        author_provider: &ProviderName,
        prompt: &str,
        structured_output: serde_json::Value,
    ) -> ApiResult<WorkItemSplitProviderOutput> {
        let run_ref = lifecycle
            .save_work_item_split_provider_run(
                &issue.project_id,
                &issue.id,
                author_provider,
                prompt,
                &structured_output,
            )
            .map_err(product_store_api_error)?;

        parse_provider_output(
            lifecycle,
            request,
            issue,
            repository,
            run_ref,
            &structured_output,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn complete_revision_from_structured_output(
        request: &GenerateWorkItemsRequest,
        lifecycle: &LifecycleStore,
        issue: &IssueRecord,
        repository: &RepositoryRecord,
        author_provider: &ProviderName,
        prompt: &str,
        structured_output: serde_json::Value,
        retained: &[LifecycleWorkItemRecord],
        redo_specs: &[super::types::RedoSpec],
    ) -> ApiResult<WorkItemSplitProviderOutput> {
        let run_ref = lifecycle
            .save_work_item_split_provider_run(
                &issue.project_id,
                &issue.id,
                author_provider,
                prompt,
                &structured_output,
            )
            .map_err(product_store_api_error)?;

        if retained.is_empty() && redo_specs.is_empty() {
            return parse_provider_output(
                lifecycle,
                request,
                issue,
                repository,
                run_ref,
                &structured_output,
            );
        }

        super::revision::materialize_revision_output(
            lifecycle,
            request,
            issue,
            repository,
            run_ref,
            &structured_output,
            retained,
            redo_specs,
        )
    }
}

/// 对单候选 markdown outline 执行本地、无 Provider 的机械解析，并返回真实候选项数。
///
/// 此 helper 故意只委托 compiler parser：语法不合法时原样返回 compiler diagnostics，
/// 调用方不得猜测 count、退回客户端选择或回退 legacy flow。Task 5.2 的内部 mode
/// selector 将在 outline invocation 成功后复用此结果。
// Task 5.2 的 selector 接线完成前，此纯 helper 仅由本模块测试使用。
#[allow(dead_code)]
pub(crate) fn count_work_item_plan_candidates(
    source: &str,
) -> Result<usize, Vec<crate::product::work_item_plan_compiler::types::CompilerDiagnostic>> {
    crate::product::work_item_plan_compiler::parse_work_item_plan(source).map(|ast| ast.items.len())
}

pub fn parse_work_item_plan_outline_output(
    value: serde_json::Value,
) -> ApiResult<super::types::OutlineAuthorOutput> {
    if let Some(field) = forbidden_outline_field(&value) {
        return Err(ApiError::validation_with_details(
            "outline_forbidden_field",
            format!("WorkItemPlan Outline output must not contain `{field}`"),
            json!({ "field": field }),
        ));
    }

    let mut output: super::types::ProviderOutlineAuthorOutput = serde_json::from_value(value)
        .map_err(|error| {
            ApiError::runtime(
                "outline_parse_error",
                format!("failed to parse WorkItemPlan Outline output: {error}"),
                json!({}),
            )
        })?;

    if output.outline.is_none() && output.context_blockers.is_empty() {
        return Err(ApiError::validation(
            "outline_empty_output",
            "WorkItemPlan Outline output must include outline or context_blockers",
        ));
    }

    if output.outline.is_some() && !output.context_blockers.is_empty() {
        return Err(ApiError::validation(
            "outline_mixed_context_blockers",
            "WorkItemPlan Outline output must not include context_blockers when outline is present",
        ));
    }

    if let Some(outline) = output.outline.as_mut() {
        outline.normalize_dependency_graph_from_depends_on();
    }

    Ok(super::types::OutlineAuthorOutput {
        outline: output.outline,
        context_blockers: output.context_blockers,
    })
}

pub(crate) fn build_work_item_draft_invocation(
    outline: &WorkItemPlanOutline,
    current_outline_id: &str,
    generation_mode: WorkItemGenerationMode,
    accepted_drafts: &[WorkItemDraftRecord],
    feedback: Option<&str>,
    context: &crate::product::cadence_skills::routing_reference::RoutingReferenceContext,
) -> ApiResult<WorkItemDraftInvocation> {
    let current_outline = outline
        .work_item_outlines
        .iter()
        .find(|item| item.outline_id == current_outline_id)
        .ok_or_else(|| {
            ApiError::validation_with_details(
                "work_item_draft_outline_missing",
                format!("current outline `{current_outline_id}` not found"),
                json!({ "outline_id": current_outline_id }),
            )
        })?;
    let mut catalog_findings = Vec::new();
    crate::product::work_item_split_validator::outline::validate_trusted_verification_command_catalog(
        current_outline,
        &mut catalog_findings,
    );
    if let Some(finding) = catalog_findings.into_iter().next() {
        return Err(ApiError::validation_with_details(
            finding.code,
            finding.message,
            json!({
                "outline_id": current_outline_id,
                "work_item_ids": finding.work_item_ids,
            }),
        ));
    }
    let dependency_ids: HashSet<&str> = current_outline
        .depends_on
        .iter()
        .map(String::as_str)
        .collect();
    let direct_dependencies: Vec<&WorkItemDraftRecord> = accepted_drafts
        .iter()
        .filter(|draft| dependency_ids.contains(draft.outline_id.as_str()))
        .collect();
    let other_previous: Vec<&WorkItemDraftRecord> = accepted_drafts
        .iter()
        .filter(|draft| !dependency_ids.contains(draft.outline_id.as_str()))
        .collect();
    let nonce = super::types::structured_output_nonce();
    let prompt = build_work_item_draft_prompt(
        outline,
        current_outline,
        generation_mode,
        &direct_dependencies,
        &other_previous,
        feedback,
        &nonce,
        context,
    );
    if prompt.len() >= WORK_ITEM_DRAFT_PROMPT_MAX_BYTES {
        return Err(ApiError::validation_with_details(
            "work_item_draft_prompt_too_large",
            format!(
                "work item draft prompt exceeds the {}-byte provider-context hard backstop",
                WORK_ITEM_DRAFT_PROMPT_MAX_BYTES
            ),
            json!({
                "prompt_bytes": prompt.len(),
                "max_prompt_bytes": WORK_ITEM_DRAFT_PROMPT_MAX_BYTES,
                "outline_id": current_outline_id,
            }),
        ));
    }

    Ok(WorkItemDraftInvocation {
        prompt,
        sentinel_nonce: nonce,
    })
}

/// Providers sometimes emit verification checks only under
/// `verification_plan.checks`, treating `canonical_contract.verification_checks`
/// as the same field (observed with Pi). Aria requires both and
/// `validate_draft_verification_plan` enforces that they are equal, so copy the
/// plan checks into the contract when only the contract copy is missing. A
/// missing or empty plan is left untouched so genuinely check-less drafts keep
/// failing.
fn backfill_contract_verification_checks(mut value: serde_json::Value) -> serde_json::Value {
    for draft_path in [Some("draft"), None] {
        let draft = match draft_path {
            Some(key) => value.get_mut(key),
            None => Some(&mut value),
        };
        let Some(draft) = draft.filter(|draft| draft.is_object()) else {
            continue;
        };
        let plan_checks = draft
            .get("verification_plan")
            .and_then(|plan| plan.get("checks"))
            .and_then(|checks| checks.as_array())
            .filter(|checks| !checks.is_empty())
            .cloned();
        let Some(plan_checks) = plan_checks else {
            continue;
        };
        let Some(contract) = draft
            .get_mut("canonical_contract")
            .filter(|contract| contract.is_object())
        else {
            continue;
        };
        if contract.get("verification_checks").is_none() {
            contract["verification_checks"] = serde_json::Value::Array(plan_checks);
        }
    }
    value
}

pub fn parse_work_item_draft_output(value: serde_json::Value) -> ApiResult<WorkItemDraftCandidate> {
    if value.get("drafts").is_some() || value.get("work_items").is_some() {
        return Err(ApiError::validation(
            "work_item_draft_multiple_items",
            "single item draft output must contain exactly one draft",
        ));
    }
    if let Some(field) = forbidden_work_item_draft_field(&value) {
        return Err(ApiError::validation_with_details(
            "work_item_draft_forbidden_field",
            format!("WorkItemDraftCandidate output must not contain `{field}`"),
            json!({ "field": field }),
        ));
    }

    let value = backfill_contract_verification_checks(value);

    let output: ProviderWorkItemDraftInput = serde_json::from_value(value).map_err(|error| {
        ApiError::runtime(
            "work_item_draft_parse_error",
            format!("failed to parse WorkItemDraftCandidate output: {error}"),
            json!({}),
        )
    })?;
    let candidate: WorkItemDraftCandidate = output.into_candidate().into();
    let contract_logical_work_item_id = &candidate
        .canonical_contract_candidate
        .identity
        .logical_work_item_id;
    if candidate.logical_work_item_id != *contract_logical_work_item_id {
        return Err(ApiError::validation_with_details(
            "work_item_draft_identity_mismatch",
            format!(
                "draft logical_work_item_id {} does not match canonical contract identity {}",
                candidate.logical_work_item_id, contract_logical_work_item_id
            ),
            json!({
                "logical_work_item_id": candidate.logical_work_item_id,
                "canonical_logical_work_item_id": contract_logical_work_item_id,
            }),
        ));
    }
    Ok(candidate)
}

fn forbidden_work_item_draft_field(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if is_forbidden_work_item_draft_key(key) {
                    return Some(key.clone());
                }
                if let Some(field) = forbidden_work_item_draft_field(value) {
                    return Some(field);
                }
            }
            None
        }
        serde_json::Value::Array(values) => values.iter().find_map(forbidden_work_item_draft_field),
        _ => None,
    }
}

fn is_forbidden_work_item_draft_key(key: &str) -> bool {
    matches!(
        key,
        "implementation_context"
            | "work_item_id"
            | "draft_id"
            | "status"
            | "generated_from_node_id"
            | "accepted_at"
            | "batch_id"
            | "superseded_by_draft_id"
            | "supersede_reason"
            | "copied_from_draft_id"
            | "review_node_id"
            | "review_verdict_ref"
    )
}

fn forbidden_outline_field(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if is_forbidden_outline_key(key) {
                    return Some(key.clone());
                }
                if let Some(field) = forbidden_outline_field(value) {
                    return Some(field);
                }
            }
            None
        }
        serde_json::Value::Array(values) => values.iter().find_map(forbidden_outline_field),
        _ => None,
    }
}

fn is_forbidden_outline_key(key: &str) -> bool {
    matches!(
        key,
        "work_items"
            | "work_item_id"
            | "work_item_ids"
            | "verification_plan"
            | "verification_plans"
            | "dependency_graph"
            | "repository_profile"
            | "parallel_groups"
    )
}

pub(crate) fn parse_provider_output(
    lifecycle: &LifecycleStore,
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
    repository: &RepositoryRecord,
    provider_run_ref: String,
    structured: &serde_json::Value,
) -> ApiResult<WorkItemSplitProviderOutput> {
    let parsed: ProviderOutput = serde_json::from_value(structured.clone()).map_err(|error| {
        ApiError::runtime(
            "work_item_split_provider_output_invalid",
            format!("failed to parse provider output: {error}"),
            json!({}),
        )
    })?;

    if parsed.work_items.is_empty() {
        return Err(ApiError::runtime(
            "work_item_split_provider_output_invalid",
            "provider returned no work items",
            json!({}),
        ));
    }

    if parsed.work_items.len() != parsed.verification_plans.len() {
        return Err(ApiError::runtime(
            "work_item_split_provider_output_invalid",
            "verification_plans count must match work_items count",
            json!({}),
        ));
    }

    let count = lifecycle
        .count_work_items(&issue.project_id, &issue.id)
        .map_err(product_store_api_error)?;
    let work_item_ids: Vec<String> = (0..parsed.work_items.len())
        .map(|index| crate::product::id::next_sequential_id("work_item", count + index))
        .collect();

    let profile_id = crate::product::id::next_sequential_id(
        "repository_profile",
        lifecycle
            .list_repository_profiles(&issue.project_id, &issue.id)
            .map_err(product_store_api_error)?
            .len(),
    );

    let mut verification_plan_ids = Vec::with_capacity(parsed.verification_plans.len());
    let existing_verification_plans = lifecycle
        .list_verification_plans(&issue.project_id, &issue.id)
        .map_err(product_store_api_error)?;
    for index in 0..parsed.verification_plans.len() {
        verification_plan_ids.push(crate::product::id::next_sequential_id(
            "verification_plan",
            existing_verification_plans.len() + index,
        ));
    }

    let mut work_items = Vec::with_capacity(parsed.work_items.len());
    for (index, item) in parsed.work_items.iter().enumerate() {
        let id = work_item_ids[index].clone();
        let depends_on: Vec<String> = item
            .depends_on
            .iter()
            .filter_map(|dep_index| work_item_ids.get(*dep_index).cloned())
            .collect();
        work_items.push(LifecycleWorkItemRecord {
            id,
            project_id: issue.project_id.clone(),
            issue_id: issue.id.clone(),
            repository_id: repository.id.clone(),
            target_repository_id: None,
            story_spec_ids: request.story_spec_ids.clone(),
            design_spec_ids: request.design_spec_ids.clone(),
            title: item.title.clone(),
            plan_status: WorkItemPlanStatus::Draft,
            execution_status: WorkItemStatus::Pending,
            worktree_path: None,
            work_item_set_id: None,
            source_work_item_plan_id: None,
            source_outline_id: None,
            source_draft_id: None,
            planned_implementation_context: None,
            kind: parse_work_item_kind(&item.kind),
            sequence_hint: item.sequence_hint,
            depends_on,
            exclusive_write_scopes: item.exclusive_write_scopes.clone(),
            forbidden_write_scopes: item.forbidden_write_scopes.clone(),
            context_budget: item.context_budget.clone().unwrap_or_default(),
            verification_plan_ref: Some(verification_plan_ids[index].clone()),
            require_execution_plan_confirm: item.require_execution_plan_confirm,
            execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
            completion_commit: None,
            completion_diff_summary_ref: None,
            created_at: String::new(),
            updated_at: String::new(),
        });
    }

    let mut dependency_graph: Vec<IssueWorkItemDependencyEdge> = Vec::new();
    for item in &work_items {
        for dep in &item.depends_on {
            dependency_graph.push(IssueWorkItemDependencyEdge {
                from_work_item_id: dep.clone(),
                to_work_item_id: item.id.clone(),
            });
        }
    }

    let repository_profile = RepositoryProfile {
        id: profile_id.clone(),
        project_id: issue.project_id.clone(),
        issue_id: issue.id.clone(),
        repository_id: repository.id.clone(),
        logical_repository_id: None,
        membership_revision: 0,
        provider_run_ref: Some(provider_run_ref.clone()),
        languages: parsed.repository_profile.languages,
        frameworks: parsed.repository_profile.frameworks,
        package_managers: parsed.repository_profile.package_managers,
        test_frameworks: parsed.repository_profile.test_frameworks,
        build_systems: parsed.repository_profile.build_systems,
        verification_capabilities: parsed.repository_profile.verification_capabilities,
        detected_layers: parsed.repository_profile.detected_layers,
        split_recommendation: parsed.repository_profile.split_recommendation,
        confidence: parse_confidence(&parsed.repository_profile.confidence),
        uncertainties: parsed.repository_profile.uncertainties,
        created_at: String::new(),
        updated_at: String::new(),
    };

    let verification_plans: Vec<VerificationPlan> = parsed
        .verification_plans
        .iter()
        .enumerate()
        .map(|(index, plan)| VerificationPlan {
            id: verification_plan_ids[index].clone(),
            project_id: issue.project_id.clone(),
            issue_id: issue.id.clone(),
            work_item_id: work_item_ids[index].clone(),
            repository_profile_ref: Some(profile_id.clone()),
            provider_run_ref: Some(provider_run_ref.clone()),
            scope: parse_verification_scope(&plan.scope),
            commands: plan
                .commands
                .iter()
                .enumerate()
                .map(|(cmd_index, cmd)| VerificationCommand {
                    id: cmd
                        .id
                        .clone()
                        .unwrap_or_else(|| format!("cmd_{:03}", cmd_index + 1)),
                    label: cmd.label.clone(),
                    command: cmd.command.clone(),
                    cwd: cmd.cwd.clone(),
                    purpose: cmd.purpose.clone(),
                    required: cmd.required,
                    timeout_seconds: cmd.timeout_seconds,
                    source: VerificationCommandSource::Provider,
                    safety: parse_safety(&cmd.safety),
                })
                .collect(),
            manual_checks: plan
                .manual_checks
                .iter()
                .enumerate()
                .map(|(check_index, check)| VerificationManualCheck {
                    id: check
                        .id
                        .clone()
                        .unwrap_or_else(|| format!("manual_{:03}", check_index + 1)),
                    label: check.label.clone(),
                    instructions: check.instructions.clone(),
                    required: check.required,
                })
                .collect(),
            required_gates: plan.required_gates.clone(),
            risk_notes: plan.risk_notes.clone(),
            confidence: parse_confidence(&plan.confidence),
            fallback_policy: parse_fallback_policy(&plan.fallback_policy),
            created_at: String::new(),
            updated_at: String::new(),
        })
        .collect();

    let existing_plans = lifecycle
        .list_issue_work_item_plans(&issue.project_id, &issue.id)
        .map_err(product_store_api_error)?;
    let plan_id =
        crate::product::id::next_sequential_id("issue_work_item_plan", existing_plans.len());

    let plan = IssueWorkItemPlan {
        id: plan_id,
        project_id: issue.project_id.clone(),
        issue_id: issue.id.clone(),
        source_story_spec_ids: request.story_spec_ids.clone(),
        source_design_spec_ids: request.design_spec_ids.clone(),
        options: IssueWorkItemPlanOptions {
            include_integration_tests: request.include_integration_tests.unwrap_or(false),
            include_e2e_tests: request.include_e2e_tests.unwrap_or(false),
            force_frontend_backend_split: request.force_frontend_backend_split.unwrap_or(false),
            require_execution_plan_confirm: request.require_execution_plan_confirm.unwrap_or(false),
        },
        status: IssueWorkItemPlanStatus::Draft,
        work_item_ids: work_item_ids.clone(),
        repository_profile_ref: Some(profile_id),
        verification_plan_ids: verification_plan_ids.clone(),
        dependency_graph,
        created_from_provider_run: Some(provider_run_ref),
        validator_findings: Vec::new(),
        review_summary: None,
        created_at: String::new(),
        updated_at: String::new(),
    };

    Ok(WorkItemSplitProviderOutput {
        repository_profile,
        plan,
        work_items,
        verification_plans,
    })
}

// ---------------------------------------------------------------------------
// Task 1b 段①:WorkItemSplitProviderRunHandle 三 lifecycle 方法与 complete
// 消费面(计划冻结接口「WS Plan/split run identity」)。
//
// 本文件承载 `impl LifecycleStore` 扩展:1b 的文件门不含 `lifecycle_store/*`,
// split run 的 durable 身份由 split engine 侧的 store 扩展承担(`begin` 经
// `next_tool_policy_role_run_seq` 分配 run-bound seq,与 tool-policy 审计分区
// 同一分配器);1c-coordinator 之后按 §0 owner 门接续装配。
// ---------------------------------------------------------------------------

/// split provider run 的运行身份句柄(计划冻结字段)。`begin` 分配并持久化,
/// caller 依次 begin handle → bind sink → start → parse → complete/fail;retry
/// 每次新 handle。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItemSplitProviderRunHandle {
    pub run_ref: String,
    pub workspace_session_id: String,
    pub role_run_seq: u64,
}

/// complete/fail 后的 read-back 快照(供 caller 与测试断言 run 收口状态)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplitProviderRunSnapshot {
    pub provider_run_ref: String,
    pub status: String,
}

/// split run 身份分区名(handle 定位的 durable 存储)。
const WORK_ITEM_SPLIT_PROVIDER_RUN_PARTITION: &str = "work-item-split-runs";

fn io_error(error: impl std::fmt::Display) -> crate::product::json_store::ProductStoreError {
    crate::product::json_store::ProductStoreError::Io(error.to_string())
}

impl LifecycleStore {
    /// split run 分区根:按 handle 冻结的 `(workspace_session_id,
    /// role_run_seq)` 定位——complete/fail 仅凭 handle 即可收口,不需要
    /// 调用方重复传 project/issue(begin 已把它们写入 run.json 溯源)。
    fn work_item_split_provider_run_dir(
        &self,
        handle: &WorkItemSplitProviderRunHandle,
    ) -> Result<std::path::PathBuf, crate::product::json_store::ProductStoreError> {
        crate::product::json_store::validate_relative_id(&handle.workspace_session_id)
            .map_err(io_error)?;
        Ok(self
            .app_paths()
            .root()
            .join(WORK_ITEM_SPLIT_PROVIDER_RUN_PARTITION)
            .join(&handle.workspace_session_id)
            .join(handle.role_run_seq.to_string()))
    }

    /// 开始一次 split provider run:分配 run-bound `role_run_seq`(与
    /// tool-policy 审计分区同一分配器),写入 status=running 的 run 记录并
    /// 返回句柄。后续 run-bound audit sink 以 `(workspace_session_id,
    /// role_run_seq)` 绑定,provider_start 恰为审计文件首行。
    pub fn begin_work_item_split_provider_run(
        &self,
        project_id: &str,
        issue_id: &str,
        provider: &ProviderName,
        workspace_session_id: &str,
    ) -> Result<WorkItemSplitProviderRunHandle, crate::product::json_store::ProductStoreError> {
        crate::product::json_store::validate_relative_id(project_id).map_err(io_error)?;
        crate::product::json_store::validate_relative_id(issue_id).map_err(io_error)?;
        crate::product::json_store::validate_relative_id(workspace_session_id).map_err(io_error)?;

        let role_run_seq = self
            .next_tool_policy_role_run_seq(workspace_session_id)
            .map_err(|error| {
                crate::product::json_store::ProductStoreError::Io(error.to_string())
            })?;
        let handle = WorkItemSplitProviderRunHandle {
            run_ref: format!("ws-{workspace_session_id}-split-run-{role_run_seq}"),
            workspace_session_id: workspace_session_id.to_string(),
            role_run_seq,
        };
        let dir = self.work_item_split_provider_run_dir(&handle)?;
        std::fs::create_dir_all(&dir).map_err(io_error)?;
        crate::product::json_store::write_json(
            &dir.join("run.json"),
            &json!({
                "provider_run_id": handle.run_ref,
                "project_id": project_id,
                "issue_id": issue_id,
                "provider_type": provider,
                "status": "running",
                "workspace_session_id": workspace_session_id,
                "role_run_seq": role_run_seq,
                "created_at": chrono::Utc::now().to_rfc3339(),
            }),
        )?;
        Ok(handle)
    }

    /// 收口成功的 split provider run:run 记录改写为 status=completed 并落
    /// structured output,不再走 `save_work_item_split_provider_run` 的旧路径。
    pub fn complete_work_item_split_provider_run(
        &self,
        handle: &WorkItemSplitProviderRunHandle,
        prompt: &str,
        structured_output: &serde_json::Value,
    ) -> Result<(), crate::product::json_store::ProductStoreError> {
        let dir = self.work_item_split_provider_run_dir(handle)?;
        let run_path = dir.join("run.json");
        if !run_path.exists() {
            return Err(crate::product::json_store::ProductStoreError::Io(format!(
                "split provider run {} was never begun",
                handle.run_ref
            )));
        }
        crate::product::json_store::write_json(
            &run_path,
            &json!({
                "provider_run_id": handle.run_ref,
                "status": "completed",
                "workspace_session_id": handle.workspace_session_id,
                "role_run_seq": handle.role_run_seq,
                "prompt_chars": prompt.chars().count(),
                "structured_output_ref": format!("{}_structured_output", handle.run_ref),
                "completed_at": chrono::Utc::now().to_rfc3339(),
            }),
        )?;
        crate::product::json_store::write_json(
            &dir.join("structured_output.json"),
            structured_output,
        )?;
        Ok(())
    }

    /// 收口失败的 split provider run:run 记录改写为 status=failed 与 reason。
    pub fn fail_work_item_split_provider_run(
        &self,
        handle: &WorkItemSplitProviderRunHandle,
        reason: &str,
    ) -> Result<(), crate::product::json_store::ProductStoreError> {
        let dir = self.work_item_split_provider_run_dir(handle)?;
        let run_path = dir.join("run.json");
        if !run_path.exists() {
            return Err(crate::product::json_store::ProductStoreError::Io(format!(
                "split provider run {} was never begun",
                handle.run_ref
            )));
        }
        crate::product::json_store::write_json(
            &run_path,
            &json!({
                "provider_run_id": handle.run_ref,
                "status": "failed",
                "workspace_session_id": handle.workspace_session_id,
                "role_run_seq": handle.role_run_seq,
                "reason": reason,
                "failed_at": chrono::Utc::now().to_rfc3339(),
            }),
        )?;
        Ok(())
    }

    /// read-back:按 handle 读取 run 记录的收口状态(complete/fail 后断言用)。
    pub fn read_work_item_split_provider_run_status(
        &self,
        handle: &WorkItemSplitProviderRunHandle,
    ) -> Result<SplitProviderRunSnapshot, crate::product::json_store::ProductStoreError> {
        let run_path = self
            .work_item_split_provider_run_dir(handle)?
            .join("run.json");
        let value: serde_json::Value = crate::product::json_store::read_json(&run_path)?;
        Ok(SplitProviderRunSnapshot {
            provider_run_ref: value
                .get("provider_run_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            status: value
                .get("status")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
        })
    }
}

/// parse.rs 的 complete 函数(计划冻结口径):消费已有 handle 收口 run 记录,
/// 不再重新 `save_work_item_split_provider_run`。返回 read-back 快照,使
/// `parsed.provider_run_ref == handle.run_ref` 成为可断言事实。
pub fn complete_split_provider_run(
    lifecycle: &LifecycleStore,
    handle: &WorkItemSplitProviderRunHandle,
    prompt: &str,
    structured_output: &serde_json::Value,
) -> ApiResult<SplitProviderRunSnapshot> {
    lifecycle
        .complete_work_item_split_provider_run(handle, prompt, structured_output)
        .map_err(product_store_api_error)?;
    lifecycle
        .read_work_item_split_provider_run_status(handle)
        .map_err(product_store_api_error)
}
