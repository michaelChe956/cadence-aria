use std::collections::HashMap;

use serde_json::json;

use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    IssueRecord, IssueWorkItemDependencyEdge, IssueWorkItemPlan, IssueWorkItemPlanOptions,
    IssueWorkItemPlanStatus, LifecycleWorkItemRecord, RepositoryProfile,
    RepositoryProfileConfidence, RepositoryRecord, VerificationCommand, VerificationCommandSource,
    VerificationManualCheck, VerificationPlan, VerificationScope, WorkItemExecutionPlanStatus,
    WorkItemPlanStatus, WorkItemStatus,
};
use crate::web::error::{ApiError, ApiResult};
use crate::web::types::GenerateWorkItemsRequest;

use super::types::{
    ProviderOutput, ProviderRepositoryProfile, ProviderVerificationPlan, ProviderWorkItem,
    WorkItemSplitProviderOutput, parse_confidence, parse_fallback_policy, parse_safety,
    parse_verification_scope, parse_work_item_kind, product_store_api_error,
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn materialize_revision_output(
    lifecycle: &LifecycleStore,
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
    repository: &RepositoryRecord,
    provider_run_ref: String,
    structured: &serde_json::Value,
    retained: &[LifecycleWorkItemRecord],
    redo_specs: &[super::types::RedoSpec],
) -> ApiResult<WorkItemSplitProviderOutput> {
    let redo = parse_revision_redo_output(structured)?;
    if redo.work_items.len() != redo_specs.len()
        || redo.verification_plans.len() != redo_specs.len()
    {
        return Err(ApiError::validation(
            "revision_redo_count_mismatch",
            format!(
                "redo_specs={} but provider returned work_items={} verification_plans={}",
                redo_specs.len(),
                redo.work_items.len(),
                redo.verification_plans.len()
            ),
        ));
    }

    let mut id_mapping = HashMap::new();
    let mut merged_work_items = retained.to_vec();

    let profile_id = crate::product::id::next_sequential_id(
        "repository_profile",
        lifecycle
            .list_repository_profiles(&issue.project_id, &issue.id)
            .map_err(product_store_api_error)?
            .len(),
    );

    let parsed_profile = redo.repository_profile;
    let (mut redo_work_items, redo_verification_plans) = materialize_redo_items(
        lifecycle,
        request,
        issue,
        repository,
        &provider_run_ref,
        redo.work_items,
        redo.verification_plans,
        redo_specs,
        &profile_id,
        &mut id_mapping,
    )?;
    merged_work_items.append(&mut redo_work_items);

    for wi in &mut merged_work_items {
        wi.depends_on = wi
            .depends_on
            .iter()
            .map(|dep| id_mapping.get(dep).cloned().unwrap_or_else(|| dep.clone()))
            .collect();
    }

    let old_graph = build_graph_from_work_items(&merged_work_items);
    let dependency_graph = super::types::repatch_dependencies(&old_graph, &id_mapping);

    build_revision_provider_output(
        lifecycle,
        request,
        issue,
        repository,
        provider_run_ref,
        profile_id,
        merged_work_items,
        parsed_profile,
        redo_verification_plans,
        dependency_graph,
    )
}

#[allow(clippy::too_many_arguments)]
fn materialize_redo_items(
    lifecycle: &LifecycleStore,
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
    repository: &RepositoryRecord,
    provider_run_ref: &str,
    redo_items: Vec<ProviderWorkItem>,
    redo_plans: Vec<ProviderVerificationPlan>,
    redo_specs: &[super::types::RedoSpec],
    profile_id: &str,
    id_mapping: &mut HashMap<String, String>,
) -> ApiResult<(Vec<LifecycleWorkItemRecord>, Vec<VerificationPlan>)> {
    let count = lifecycle
        .count_work_items(&issue.project_id, &issue.id)
        .map_err(product_store_api_error)?;
    let work_item_ids: Vec<String> = (0..redo_items.len())
        .map(|index| crate::product::id::next_sequential_id("work_item", count + index))
        .collect();

    let existing_verification_plans = lifecycle
        .list_verification_plans(&issue.project_id, &issue.id)
        .map_err(product_store_api_error)?;
    let verification_plan_ids: Vec<String> = (0..redo_plans.len())
        .map(|index| {
            crate::product::id::next_sequential_id(
                "verification_plan",
                existing_verification_plans.len() + index,
            )
        })
        .collect();

    let mut work_items = Vec::with_capacity(redo_items.len());
    for (index, item) in redo_items.iter().enumerate() {
        let id = work_item_ids[index].clone();
        let depends_on: Vec<String> = item
            .depends_on
            .iter()
            .filter_map(|dep_index| {
                work_item_ids.get(*dep_index).cloned().or_else(|| {
                    tracing::warn!(
                        "revision redo work item {} depends_on index {} is out of range or references a retained item; ignoring",
                        id,
                        dep_index
                    );
                    None
                })
            })
            .collect();
        work_items.push(LifecycleWorkItemRecord {
            id: id.clone(),
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

        id_mapping.insert(redo_specs[index].old_id.clone(), id);
    }

    let verification_plans: Vec<VerificationPlan> = redo_plans
        .iter()
        .enumerate()
        .map(|(index, plan)| VerificationPlan {
            id: verification_plan_ids[index].clone(),
            project_id: issue.project_id.clone(),
            issue_id: issue.id.clone(),
            work_item_id: work_item_ids[index].clone(),
            repository_profile_ref: Some(profile_id.to_string()),
            provider_run_ref: Some(provider_run_ref.to_string()),
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

    Ok((work_items, verification_plans))
}

fn build_graph_from_work_items(
    items: &[LifecycleWorkItemRecord],
) -> Vec<IssueWorkItemDependencyEdge> {
    let mut graph = Vec::new();
    for item in items {
        for dep in &item.depends_on {
            graph.push(IssueWorkItemDependencyEdge {
                from_work_item_id: dep.clone(),
                to_work_item_id: item.id.clone(),
            });
        }
    }
    graph
}

fn parse_revision_redo_output(structured: &serde_json::Value) -> ApiResult<ProviderOutput> {
    serde_json::from_value(structured.clone()).map_err(|error| {
        ApiError::runtime(
            "work_item_split_provider_output_invalid",
            format!("failed to parse revision redo output: {error}"),
            json!({}),
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn build_revision_provider_output(
    lifecycle: &LifecycleStore,
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
    repository: &RepositoryRecord,
    provider_run_ref: String,
    profile_id: String,
    mut work_items: Vec<LifecycleWorkItemRecord>,
    parsed_profile: ProviderRepositoryProfile,
    redo_verification_plans: Vec<VerificationPlan>,
    dependency_graph: Vec<IssueWorkItemDependencyEdge>,
) -> ApiResult<WorkItemSplitProviderOutput> {
    let redo_count = redo_verification_plans.len();
    let retained_count = work_items.len().saturating_sub(redo_count);

    let existing_verification_plans = lifecycle
        .list_verification_plans(&issue.project_id, &issue.id)
        .map_err(product_store_api_error)?;
    let mut verification_plans = Vec::with_capacity(work_items.len());
    let mut verification_plan_ids = Vec::with_capacity(work_items.len());

    // retained 的 verification_plan id 必须跳过 redo_verification_plans 已占用的
    // redo_count 个 id，避免与 redo 项冲突。
    for (index, wi) in work_items.iter_mut().enumerate() {
        let plan_id = if index < retained_count {
            crate::product::id::next_sequential_id(
                "verification_plan",
                existing_verification_plans.len() + redo_count + index,
            )
        } else {
            redo_verification_plans[index - retained_count].id.clone()
        };
        wi.verification_plan_ref = Some(plan_id.clone());
        verification_plan_ids.push(plan_id.clone());

        if index < retained_count {
            verification_plans.push(VerificationPlan {
                id: plan_id,
                project_id: issue.project_id.clone(),
                issue_id: issue.id.clone(),
                work_item_id: wi.id.clone(),
                repository_profile_ref: Some(profile_id.clone()),
                provider_run_ref: Some(provider_run_ref.clone()),
                scope: VerificationScope::Custom,
                commands: Vec::new(),
                manual_checks: Vec::new(),
                required_gates: Vec::new(),
                risk_notes: Vec::new(),
                confidence: RepositoryProfileConfidence::Medium,
                fallback_policy: crate::product::models::VerificationFallbackPolicy::ManualGate,
                created_at: String::new(),
                updated_at: String::new(),
            });
        } else {
            verification_plans.push(redo_verification_plans[index - retained_count].clone());
        }
    }

    let repository_profile = RepositoryProfile {
        id: profile_id.clone(),
        project_id: issue.project_id.clone(),
        issue_id: issue.id.clone(),
        repository_id: repository.id.clone(),
        provider_run_ref: Some(provider_run_ref.clone()),
        languages: parsed_profile.languages,
        frameworks: parsed_profile.frameworks,
        package_managers: parsed_profile.package_managers,
        test_frameworks: parsed_profile.test_frameworks,
        build_systems: parsed_profile.build_systems,
        verification_capabilities: parsed_profile.verification_capabilities,
        detected_layers: parsed_profile.detected_layers,
        split_recommendation: parsed_profile.split_recommendation,
        confidence: parse_confidence(&parsed_profile.confidence),
        uncertainties: parsed_profile.uncertainties,
        created_at: String::new(),
        updated_at: String::new(),
    };

    let work_item_ids: Vec<String> = work_items.iter().map(|wi| wi.id.clone()).collect();

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
        work_item_ids,
        repository_profile_ref: Some(profile_id),
        verification_plan_ids,
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
