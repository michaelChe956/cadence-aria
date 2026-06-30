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
use super::prompts::build_work_item_draft_prompt;
use super::types::{
    ProviderOutput, WorkItemDraftInvocation, WorkItemSplitProviderOutput, parse_confidence,
    parse_fallback_policy, parse_safety, parse_verification_scope, parse_work_item_kind,
    product_store_api_error,
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

    let output: super::types::ProviderOutlineAuthorOutput =
        serde_json::from_value(value).map_err(|error| {
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

    Ok(super::types::OutlineAuthorOutput {
        outline: output.outline,
        context_blockers: output.context_blockers,
    })
}

pub fn build_work_item_draft_invocation(
    outline: &WorkItemPlanOutline,
    current_outline_id: &str,
    generation_mode: WorkItemGenerationMode,
    accepted_drafts: &[WorkItemDraftRecord],
    feedback: Option<&str>,
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
    );

    Ok(WorkItemDraftInvocation {
        prompt,
        sentinel_nonce: nonce,
    })
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

    let draft_value = value.get("draft").cloned().unwrap_or(value);
    serde_json::from_value(draft_value).map_err(|error| {
        ApiError::runtime(
            "work_item_draft_parse_error",
            format!("failed to parse WorkItemDraftCandidate output: {error}"),
            json!({}),
        )
    })
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
        "work_item_id"
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
            planned_handoff_summary: None,
            kind: parse_work_item_kind(&item.kind),
            sequence_hint: item.sequence_hint,
            depends_on,
            exclusive_write_scopes: item.exclusive_write_scopes.clone(),
            forbidden_write_scopes: item.forbidden_write_scopes.clone(),
            context_budget: item.context_budget.clone().unwrap_or_default(),
            required_handoff_from: item.required_handoff_from.clone(),
            verification_plan_ref: Some(verification_plan_ids[index].clone()),
            require_execution_plan_confirm: item.require_execution_plan_confirm,
            execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
            handoff_summary_ref: None,
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
