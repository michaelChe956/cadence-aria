use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{CodingAttemptScope, CodingExecutionAttempt};
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    IssueWorkItemPlan, LifecycleWorkItemRecord, WorkItemDraftRecord, WorkItemPlanCompileStatus,
    WorkspaceType,
};
use crate::product::work_item_plan_store::WorkItemPlanStore;

use super::methods::required_methods_by_role;
use super::repo::repo_context;
use super::specs::{
    contexts_for_design_specs, contexts_for_story_specs, latest_artifact_version_for_session,
    latest_session_for, work_item_context,
};
use super::{
    CodingGroupContextPack, EvaluationContextPack, EvaluationContextRole,
    EvaluationWorkItemContext, OpenSpecContext, SuperpowersContext,
};

pub fn build_evaluation_context_pack(
    paths: ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    provider_role: EvaluationContextRole,
) -> Result<EvaluationContextPack, ProductStoreError> {
    let lifecycle_paths = paths.clone();
    let coding_store = CodingAttemptStore::new(paths.clone());
    let quality_bypass_audits = coding_store.list_quality_bypass_audits(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    let lifecycle = LifecycleStore::new(lifecycle_paths.clone());
    let sessions = lifecycle.list_workspace_sessions(&attempt.project_id, &attempt.issue_id)?;
    let work_items = lifecycle.list_work_items(&attempt.project_id, &attempt.issue_id)?;
    let current_work_item_id = attempt
        .current_work_item_id
        .as_deref()
        .unwrap_or(&attempt.work_item_id);
    let mut context_warnings = Vec::new();
    let group_context = build_group_context(
        lifecycle_paths.clone(),
        &lifecycle,
        attempt,
        current_work_item_id,
        &work_items,
        &mut context_warnings,
    )?;
    let work_item = work_items
        .iter()
        .find(|record| record.id == current_work_item_id)
        .cloned();
    let Some(work_item) = work_item else {
        context_warnings.push("missing_work_item".to_string());
        return Ok(EvaluationContextPack {
            issue_id: attempt.issue_id.clone(),
            attempt_id: attempt.id.clone(),
            provider_role,
            story_specs: Vec::new(),
            design_specs: Vec::new(),
            work_item: EvaluationWorkItemContext {
                artifact_id: current_work_item_id.to_string(),
                version_id: None,
                version: None,
                title: String::new(),
                repository_id: String::new(),
                story_spec_ids: Vec::new(),
                design_spec_ids: Vec::new(),
                raw_markdown_or_sections: String::new(),
                workspace_session_id: None,
            },
            group_context,
            repo_context: repo_context(attempt, None, &mut context_warnings),
            openspec_context: OpenSpecContext {
                enabled: false,
                active_change_id: None,
                relevant_requirements: Vec::new(),
                traceability_notes: Vec::new(),
            },
            superpowers_context: SuperpowersContext {
                enabled: false,
                required_methods_by_role: required_methods_by_role(),
            },
            quality_bypass_audits,
            context_warnings,
        });
    };

    let stories = lifecycle.list_story_specs(&attempt.project_id, &attempt.issue_id)?;
    let designs = lifecycle.list_design_specs(&attempt.project_id, &attempt.issue_id)?;
    let story_specs = contexts_for_story_specs(
        &lifecycle,
        &attempt.project_id,
        &attempt.issue_id,
        &work_item.story_spec_ids,
        &stories,
        &sessions,
        &mut context_warnings,
    )?;
    let design_specs = contexts_for_design_specs(
        &lifecycle,
        &attempt.project_id,
        &attempt.issue_id,
        &work_item.design_spec_ids,
        &designs,
        &sessions,
        &mut context_warnings,
    )?;
    let work_item_session = latest_session_for(&sessions, &work_item.id, &WorkspaceType::WorkItem);
    let work_item_version = latest_artifact_version_for_session(&lifecycle, work_item_session)?;
    let work_item_context = work_item_context(
        &work_item,
        work_item_version.as_ref(),
        work_item_session,
        &mut context_warnings,
    );
    let openspec_enabled = sessions.iter().any(|session| session.openspec_enabled);
    let superpowers_enabled = sessions.iter().any(|session| session.superpowers_enabled);

    Ok(EvaluationContextPack {
        issue_id: attempt.issue_id.clone(),
        attempt_id: attempt.id.clone(),
        provider_role,
        story_specs,
        design_specs,
        work_item: work_item_context,
        group_context,
        repo_context: repo_context(attempt, Some(&work_item), &mut context_warnings),
        openspec_context: OpenSpecContext {
            enabled: openspec_enabled,
            active_change_id: None,
            relevant_requirements: Vec::new(),
            traceability_notes: Vec::new(),
        },
        superpowers_context: SuperpowersContext {
            enabled: superpowers_enabled,
            required_methods_by_role: required_methods_by_role(),
        },
        quality_bypass_audits,
        context_warnings,
    })
}

fn build_group_context(
    lifecycle_paths: ProductAppPaths,
    lifecycle: &LifecycleStore,
    attempt: &CodingExecutionAttempt,
    current_work_item_id: &str,
    work_items: &[LifecycleWorkItemRecord],
    warnings: &mut Vec<String>,
) -> Result<Option<CodingGroupContextPack>, ProductStoreError> {
    if attempt.scope != CodingAttemptScope::WorkItemGroup {
        return Ok(None);
    }

    let Some(plan_id) = attempt.work_item_group_id.as_deref() else {
        return Ok(None);
    };
    let plan =
        lifecycle.get_issue_work_item_plan(&attempt.project_id, &attempt.issue_id, plan_id)?;
    if !plan
        .work_item_ids
        .iter()
        .any(|id| id == current_work_item_id)
    {
        warnings.push("group_plan_mapping_mismatch".to_string());
    }
    let dependency_handoff_refs =
        dependency_handoff_refs_for_current(work_items, current_work_item_id).unwrap_or_default();
    let (source_outline_id, source_draft_id) =
        resolve_group_draft_context(lifecycle_paths, &plan, current_work_item_id, warnings)?;

    Ok(Some(CodingGroupContextPack {
        plan_id: plan.id,
        current_work_item_id: current_work_item_id.to_string(),
        sibling_work_item_ids: plan.work_item_ids,
        dependency_handoff_refs,
        source_outline_id,
        source_draft_id,
    }))
}

fn dependency_handoff_refs_for_current(
    work_items: &[LifecycleWorkItemRecord],
    current_work_item_id: &str,
) -> Option<Vec<String>> {
    let current = work_items
        .iter()
        .find(|item| item.id == current_work_item_id)?;
    Some(
        current
            .required_handoff_from
            .iter()
            .filter_map(|dependency_id| {
                work_items
                    .iter()
                    .find(|item| item.id == *dependency_id)
                    .and_then(|item| item.handoff_summary_ref.clone())
            })
            .collect(),
    )
}

fn resolve_group_draft_context(
    paths: ProductAppPaths,
    plan: &IssueWorkItemPlan,
    current_work_item_id: &str,
    warnings: &mut Vec<String>,
) -> Result<(Option<String>, Option<String>), ProductStoreError> {
    let store = WorkItemPlanStore::new(paths);
    let tx = store
        .list_compile_transactions(&plan.project_id, &plan.issue_id, &plan.id)?
        .into_iter()
        .filter(|tx| tx.status == WorkItemPlanCompileStatus::Committed)
        .max_by(|left, right| left.created_at.cmp(&right.created_at));
    let Some(tx) = tx else {
        warnings.push("group_draft_context_unavailable".to_string());
        return Ok((None, None));
    };

    let source_outline_id =
        tx.outline_to_work_item_id
            .iter()
            .find_map(|(outline_id, work_item_id)| {
                (work_item_id == current_work_item_id).then(|| outline_id.clone())
            });
    let Some(source_outline_id) = source_outline_id else {
        warnings.push("group_draft_context_unavailable".to_string());
        return Ok((None, None));
    };

    let draft_records = store.list_draft_records(&plan.project_id, &plan.issue_id, &plan.id)?;
    let source_draft_id = tx.active_draft_ids.iter().find_map(|draft_id| {
        draft_records.iter().find_map(|record| {
            matches_draft_for_outline(
                record,
                &tx.generation_round_id,
                draft_id,
                &source_outline_id,
            )
            .then(|| record.draft_id.clone())
        })
    });
    let Some(source_draft_id) = source_draft_id else {
        warnings.push("group_draft_context_unavailable".to_string());
        return Ok((None, None));
    };

    warnings.push("group_draft_context_loaded".to_string());
    Ok((Some(source_outline_id), Some(source_draft_id)))
}

fn matches_draft_for_outline(
    record: &WorkItemDraftRecord,
    generation_round_id: &str,
    draft_id: &str,
    outline_id: &str,
) -> bool {
    record.generation_round_id == generation_round_id
        && record.draft_id == draft_id
        && record.outline_id == outline_id
}
