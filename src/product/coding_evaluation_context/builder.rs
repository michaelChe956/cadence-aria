use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingAgentRole, CodingAttemptScope, CodingEntryType, CodingExecutionAttempt,
    CodingExecutionStage, CodingExecutionUnit, CodingProviderRole, CodingUnitRun,
};
use crate::product::coding_work_item_context::load_coding_work_item_context;
use crate::product::issue_store::IssueStore;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    IssueWorkItemPlan, LifecycleWorkItemRecord, WorkItemDraftRecord, WorkItemPlanCompileStatus,
    WorkspaceType,
};
use crate::product::work_item_plan_store::WorkItemPlanStore;
use crate::product::work_item_runtime_reader::{ResolvedWorkItemRuntime, WorkItemRuntimeReader};

use super::methods::required_methods_by_role;
use super::repo::repo_context;
use super::sanitize::{push_warning_once, sanitize_context_text};
use super::specs::{
    contexts_for_design_specs, contexts_for_story_specs, latest_artifact_version_for_session,
    latest_session_for, work_item_context,
};
use super::{
    CoderEvidencePack, CodingGroupContextPack, EvaluationContextPack, EvaluationContextRole,
    EvaluationWorkItemContext, OpenSpecContext, SuperpowersContext,
};

const MAX_CODER_EVIDENCE_EXCERPT_CHARS: usize = 6_000;

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
    let mut context_warnings = Vec::new();
    let repo_diff_base = evaluation_repo_diff_base(attempt, &provider_role, &mut context_warnings);
    let coder_evidence = coder_evidence_pack(
        &coding_store,
        attempt,
        &provider_role,
        &mut context_warnings,
    )?;
    if let Some((unit, run, runtime)) = schema_v2_active_unit_runtime(&lifecycle_paths, attempt)? {
        return build_schema_v2_evaluation_context_pack(
            &lifecycle,
            &sessions,
            attempt,
            provider_role,
            coder_evidence,
            quality_bypass_audits,
            unit,
            run,
            runtime,
            repo_diff_base,
        );
    }
    let work_items = lifecycle.list_work_items(&attempt.project_id, &attempt.issue_id)?;
    let current_work_item_id = attempt
        .current_work_item_id
        .as_deref()
        .unwrap_or(&attempt.work_item_id);
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
            coder_evidence,
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
            repo_context: repo_context(attempt, None, repo_diff_base, &mut context_warnings),
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
    let compiled_work_item_context = load_coding_work_item_context(&lifecycle_paths, attempt)?;
    let work_item_context = work_item_context(
        &work_item,
        work_item_version.as_ref(),
        compiled_work_item_context.markdown.as_deref(),
        work_item_session,
        &mut context_warnings,
    );
    let openspec_enabled = sessions.iter().any(|session| session.openspec_enabled);
    let superpowers_enabled = sessions.iter().any(|session| session.superpowers_enabled);

    Ok(EvaluationContextPack {
        issue_id: attempt.issue_id.clone(),
        attempt_id: attempt.id.clone(),
        provider_role,
        coder_evidence,
        story_specs,
        design_specs,
        work_item: work_item_context,
        group_context,
        repo_context: repo_context(
            attempt,
            Some(&work_item),
            repo_diff_base,
            &mut context_warnings,
        ),
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

#[allow(clippy::too_many_arguments)]
fn build_schema_v2_evaluation_context_pack(
    lifecycle: &LifecycleStore,
    sessions: &[crate::product::models::WorkspaceSessionRecord],
    attempt: &CodingExecutionAttempt,
    provider_role: EvaluationContextRole,
    coder_evidence: Option<CoderEvidencePack>,
    quality_bypass_audits: Vec<crate::product::coding_models::QualityGateBypassAudit>,
    _unit: CodingExecutionUnit,
    run: Option<CodingUnitRun>,
    runtime: ResolvedWorkItemRuntime,
    repo_diff_base: Option<&str>,
) -> Result<EvaluationContextPack, ProductStoreError> {
    let mut context_warnings = Vec::new();
    let issue = IssueStore::new(lifecycle.app_paths().clone())
        .get(&attempt.project_id, &attempt.issue_id)?;
    let repository_id = issue.repo_id.ok_or_else(|| ProductStoreError::NotFound {
        kind: "repository",
        id: format!("issue:{}:repo_id", attempt.issue_id),
    })?;
    let stories = lifecycle.list_story_specs(&attempt.project_id, &attempt.issue_id)?;
    let designs = lifecycle.list_design_specs(&attempt.project_id, &attempt.issue_id)?;
    let story_specs = contexts_for_story_specs(
        lifecycle,
        &attempt.project_id,
        &attempt.issue_id,
        &runtime.lineage.story_spec_refs,
        &stories,
        sessions,
        &mut context_warnings,
    )?;
    let design_specs = contexts_for_design_specs(
        lifecycle,
        &attempt.project_id,
        &attempt.issue_id,
        &runtime.lineage.design_spec_refs,
        &designs,
        sessions,
        &mut context_warnings,
    )?;
    let work_item_session = latest_session_for(
        sessions,
        &runtime.binding.logical_work_item_id,
        &WorkspaceType::WorkItem,
    );
    let canonical_contract =
        serde_json::to_string_pretty(&runtime.work_item_revision.canonical_contract)
            .map_err(|error| ProductStoreError::Json(error.to_string()))?;
    let (raw_markdown_or_sections, truncated) = sanitize_context_text(&canonical_contract);
    if truncated {
        push_warning_once(&mut context_warnings, "context_truncated");
    }
    let mut repo_context = repo_context(attempt, None, repo_diff_base, &mut context_warnings);
    repo_context.repository_id = Some(repository_id.clone());
    let openspec_enabled = sessions.iter().any(|session| session.openspec_enabled);
    let superpowers_enabled = sessions.iter().any(|session| session.superpowers_enabled);

    Ok(EvaluationContextPack {
        issue_id: attempt.issue_id.clone(),
        attempt_id: attempt.id.clone(),
        provider_role,
        coder_evidence,
        story_specs,
        design_specs,
        work_item: EvaluationWorkItemContext {
            artifact_id: runtime.binding.logical_work_item_id.clone(),
            version_id: None,
            version: None,
            title: runtime.projection_bundle.human_projection.title.clone(),
            repository_id,
            story_spec_ids: runtime.lineage.story_spec_refs.clone(),
            design_spec_ids: runtime.lineage.design_spec_refs.clone(),
            raw_markdown_or_sections,
            workspace_session_id: work_item_session.map(|session| session.id.clone()),
        },
        group_context: Some(CodingGroupContextPack {
            plan_id: runtime.binding.plan_id,
            current_work_item_id: runtime.binding.logical_work_item_id,
            sibling_work_item_ids: runtime
                .plan_projection_bundle
                .coder_group_context
                .ordered_logical_work_item_ids,
            dependency_handoff_refs: run
                .map(|run| run.resolved_handoff_revision_ids)
                .unwrap_or_default(),
            source_outline_id: None,
            source_draft_id: Some(runtime.work_item_revision.source_draft_revision_id),
        }),
        repo_context,
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

pub(super) fn schema_v2_active_unit_runtime(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<
    Option<(
        CodingExecutionUnit,
        Option<CodingUnitRun>,
        ResolvedWorkItemRuntime,
    )>,
    ProductStoreError,
> {
    if attempt.scope != CodingAttemptScope::WorkItemGroup {
        return Ok(None);
    }
    let Some(plan_id) = attempt.work_item_group_id.as_deref() else {
        return Ok(None);
    };
    let revision_store =
        crate::product::work_item_revision_store::WorkItemRevisionStore::new(paths.clone());
    match revision_store.get_plan_lineage(&attempt.project_id, &attempt.issue_id, plan_id) {
        Ok(_) => {}
        Err(ProductStoreError::NotFound { kind, .. }) if kind == "work_item_plan_lineage" => {
            return Ok(None);
        }
        Err(error) => return Err(error),
    }

    let coding_store = CodingAttemptStore::new(paths.clone());
    let unit = coding_store
        .get_active_coding_unit(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        .ok_or_else(|| ProductStoreError::IdentityMismatch {
            kind: "runtime_binding_missing",
            id: attempt.id.clone(),
        })?;
    let run = coding_store
        .list_coding_unit_runs(attempt, &unit.id)?
        .into_iter()
        .max_by_key(|run| run.execution_no);
    let runtime = WorkItemRuntimeReader::new(paths.clone()).normative_context_for_unit(
        attempt,
        &unit,
        run.as_ref(),
    )?;
    Ok(Some((unit, run, runtime)))
}

fn evaluation_repo_diff_base<'a>(
    attempt: &'a CodingExecutionAttempt,
    provider_role: &EvaluationContextRole,
    context_warnings: &mut Vec<String>,
) -> Option<&'a str> {
    if *provider_role == EvaluationContextRole::CodeReviewer
        && attempt.scope == CodingAttemptScope::WorkItemGroup
    {
        let current_work_item_id = attempt
            .current_work_item_id
            .as_deref()
            .unwrap_or(&attempt.work_item_id);
        if current_work_item_id == attempt.work_item_id {
            return Some(&attempt.base_branch);
        }
        if attempt.head_commit.is_none() {
            context_warnings.push("code_review_diff_base_missing".to_string());
        }
        return attempt.head_commit.as_deref();
    }
    Some(&attempt.base_branch)
}

fn coder_evidence_pack(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    provider_role: &EvaluationContextRole,
    context_warnings: &mut Vec<String>,
) -> Result<Option<CoderEvidencePack>, ProductStoreError> {
    if !matches!(
        provider_role,
        EvaluationContextRole::CodeReviewer | EvaluationContextRole::InternalReviewer
    ) {
        return Ok(None);
    }

    let latest_run = coding_store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        .into_iter()
        .rev()
        .find(|run| {
            run.role == CodingProviderRole::Coder && run.stage == CodingExecutionStage::Coding
        });
    let handoff = coding_store.get_visible_work_item_handoff(attempt)?;
    let mut evidence_warnings = Vec::new();

    if latest_run.is_none() {
        evidence_warnings.push("coder_role_run_missing".to_string());
    }
    if handoff.is_none() && matches!(provider_role, EvaluationContextRole::InternalReviewer) {
        evidence_warnings.push("work_item_handoff_missing".to_string());
    }

    let completion_report_excerpt = match latest_run.as_ref() {
        Some(run) => coder_completion_report_excerpt(coding_store, attempt, run, context_warnings)?,
        None => None,
    };
    if latest_run
        .as_ref()
        .is_some_and(|run| run.raw_provider_output_refs.is_empty())
    {
        evidence_warnings.push("coder_raw_provider_output_refs_missing".to_string());
    }
    if completion_report_excerpt.is_none() {
        evidence_warnings.push("coder_completion_report_missing".to_string());
    }

    Ok(Some(CoderEvidencePack {
        latest_role_run_id: latest_run.as_ref().map(|run| run.id.clone()),
        run_no: latest_run.as_ref().map(|run| run.run_no),
        status: latest_run.as_ref().map(|run| run.status.clone()),
        raw_provider_output_refs: latest_run
            .as_ref()
            .map(|run| run.raw_provider_output_refs.clone())
            .unwrap_or_default(),
        artifact_refs: latest_run
            .as_ref()
            .map(|run| run.artifact_refs.clone())
            .unwrap_or_default(),
        completion_report_excerpt,
        handoff_tests_run: handoff
            .as_ref()
            .map(|handoff| handoff.tests_run.clone())
            .unwrap_or_default(),
        handoff_test_result_summary: handoff
            .as_ref()
            .map(|handoff| handoff.test_result_summary.trim().to_string())
            .filter(|summary| !summary.is_empty()),
        evidence_warnings,
    }))
}

fn coder_completion_report_excerpt(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    run: &crate::product::coding_models::CodingRoleRun,
    context_warnings: &mut Vec<String>,
) -> Result<Option<String>, ProductStoreError> {
    let entries =
        coding_store.list_chat_entries(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
    let entry = entries
        .iter()
        .rev()
        .find(|entry| {
            entry.role == CodingAgentRole::Author
                && matches!(&entry.entry_type, CodingEntryType::AssistantMessage)
                && entry
                    .metadata
                    .as_ref()
                    .and_then(|metadata| metadata.get("role_run_id"))
                    .and_then(|value| value.as_str())
                    == Some(run.id.as_str())
        })
        .or_else(|| {
            run.node_id.as_ref().and_then(|node_id| {
                entries.iter().rev().find(|entry| {
                    entry.role == CodingAgentRole::Author
                        && matches!(&entry.entry_type, CodingEntryType::AssistantMessage)
                        && entry.node_id.as_deref() == Some(node_id.as_str())
                })
            })
        })
        .or_else(|| {
            entries.iter().rev().find(|entry| {
                entry.role == CodingAgentRole::Author
                    && matches!(&entry.entry_type, CodingEntryType::AssistantMessage)
            })
        });
    let Some(content) = entry.and_then(|entry| entry.content.as_deref()) else {
        return Ok(None);
    };
    let (sanitized, sanitized_truncated) = sanitize_context_text(content);
    let mut excerpt: String = sanitized
        .chars()
        .take(MAX_CODER_EVIDENCE_EXCERPT_CHARS)
        .collect();
    if sanitized_truncated || sanitized.chars().count() > MAX_CODER_EVIDENCE_EXCERPT_CHARS {
        context_warnings.push("coder_evidence_truncated".to_string());
        excerpt.push_str("\n[...coder evidence truncated...]");
    }
    Ok(Some(excerpt))
}

pub(super) fn build_group_context(
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
    let current_work_item = work_items
        .iter()
        .find(|record| record.id == current_work_item_id);
    let explicit_source = current_work_item.and_then(|item| {
        item.source_outline_id
            .clone()
            .zip(item.source_draft_id.clone())
    });
    let (source_outline_id, source_draft_id) = if let Some((outline_id, draft_id)) = explicit_source
    {
        warnings.push("group_draft_context_loaded_from_work_item".to_string());
        (Some(outline_id), Some(draft_id))
    } else {
        resolve_group_draft_context(lifecycle_paths, &plan, current_work_item_id, warnings)?
    };

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
