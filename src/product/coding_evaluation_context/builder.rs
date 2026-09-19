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
use crate::product::logical_codebase::{RepositoryRouting, RepositoryRoutingErrorCode};
use crate::product::models::{
    IssueWorkItemPlan, LifecycleWorkItemRecord, WorkItemDraftRecord, WorkItemPlanCompileStatus,
    WorkspaceType,
};
use crate::product::project_store::ProjectStore;
use crate::product::repository_store::RepositoryStore;
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
    let _issue = IssueStore::new(lifecycle.app_paths().clone())
        .get(&attempt.project_id, &attempt.issue_id)?;
    let repository_id =
        schema_v2_evaluation_context_repository_id(&lifecycle.app_paths(), attempt)?;
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

fn schema_v2_evaluation_context_repository_id(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<String, ProductStoreError> {
    let project = ProjectStore::new(paths.clone()).get(&attempt.project_id)?;
    let store = RepositoryStore::for_project(paths.clone(), &project);
    match RepositoryRouting::load_for_issue(paths, &attempt.project_id, &attempt.issue_id)? {
        RepositoryRouting::Legacy { .. } => {
            let issue =
                IssueStore::new(paths.clone()).get(&attempt.project_id, &attempt.issue_id)?;
            let physical_repository_id =
                issue.repo_id.ok_or_else(|| ProductStoreError::NotFound {
                    kind: "repository",
                    id: format!("issue:{}:repo_id", attempt.issue_id),
                })?;
            match store.resolve_legacy_physical_repository_if_dual(
                &attempt.project_id,
                &physical_repository_id,
            ) {
                Ok((_, _, repository)) => Ok(repository.id),
                Err(_) => Ok(physical_repository_id),
            }
        }
        RepositoryRouting::Logical {
            manifest,
            selection,
        } => {
            validate_logical_evaluation_selection(
                paths,
                &attempt.project_id,
                &manifest,
                &selection,
            )?;
            // D1 路由权威转移（REQ-MTG-01/REQ-COD-04 分流化）：per-attempt 冻结
            // 快照优先——有快照不再经 selection focus 收敛（多 focus 不阻断
            // target-attempt 评估）；快照 target 仍受有效 selection 成员约束 +
            // 权威身份逐字段校验（fail-closed 保留，与恢复面
            // resolve_coding_attempt_repository 同形态先例）——漂移快照不得静默路由。
            if let Some(snapshot) = attempt.target_snapshot.as_ref() {
                let selected_ids: std::collections::BTreeSet<
                    crate::product::logical_codebase::LogicalRepositoryId,
                > = match selection.selection_policy {
                    crate::product::logical_codebase::SelectionPolicy::AllMembers => {
                        manifest.member_ids.iter().copied().collect()
                    }
                    crate::product::logical_codebase::SelectionPolicy::Explicit => {
                        selection.resolve_effective_members().into_iter().collect()
                    }
                };
                if !selected_ids.contains(&snapshot.logical_repository_id) {
                    return Err(routing_error(
                        RepositoryRoutingErrorCode::TargetUnknown,
                        "target snapshot repository is not in the effective selection",
                    ));
                }
                let lc_id = crate::product::logical_codebase::resolve_issue_logical_codebase_id(
                    paths,
                    &attempt.project_id,
                    &attempt.issue_id,
                )?;
                crate::product::logical_codebase::snapshot_validator::validate_snapshot_fields(
                    paths,
                    attempt,
                    lc_id.as_deref(),
                )
                .map_err(|code| {
                    routing_error(
                        code,
                        "target snapshot does not match logical codebase authority",
                    )
                })?;
                return store
                    .resolve_logical_repository_strict(
                        &attempt.project_id,
                        snapshot.logical_repository_id,
                    )
                    .map(|(_, _, repository)| repository.id);
            }
            // selection focus 面（保留）：无快照 attempt 的现行 focus 收敛。
            let logical_repository_id = match selection.focus_repository_ids.as_slice() {
                [] => {
                    return Err(routing_error(
                        RepositoryRoutingErrorCode::TargetMissing,
                        "issue codebase selection has no focus repository",
                    ));
                }
                [logical_repository_id] => *logical_repository_id,
                _ => {
                    return Err(routing_error(
                        RepositoryRoutingErrorCode::TargetAmbiguous,
                        "issue codebase selection has multiple focus repositories",
                    ));
                }
            };
            store
                .resolve_logical_repository_strict(&attempt.project_id, logical_repository_id)
                .map(|(_, _, repository)| repository.id)
        }
        RepositoryRouting::FailClosed { code, reason } => Err(routing_error(code, reason)),
    }
}

fn validate_logical_evaluation_selection(
    paths: &ProductAppPaths,
    project_id: &str,
    manifest: &crate::product::logical_codebase::LogicalCodebaseManifest,
    selection: &crate::product::logical_codebase::IssueCodebaseSelection,
) -> Result<(), ProductStoreError> {
    if selection.invalidation.is_some() {
        return Err(routing_error(
            RepositoryRoutingErrorCode::SelectionInvalidated,
            "issue codebase selection has been invalidated",
        ));
    }
    let logical = match selection.logical_codebase_id.as_deref() {
        Some(lc_id) => {
            crate::product::logical_codebase::LogicalCodebaseStore::for_lc(paths.clone(), lc_id)
        }
        None => crate::product::logical_codebase::LogicalCodebaseStore::new(paths.clone()),
    };
    let active_members: std::collections::BTreeSet<_> = logical
        .list_members(project_id)?
        .into_iter()
        .filter(|member| member.status == crate::product::logical_codebase::MemberStatus::Active)
        .map(|member| member.logical_repository_id)
        .collect();
    if manifest
        .member_ids
        .iter()
        .any(|id| !active_members.contains(id))
    {
        return Err(routing_error(
            RepositoryRoutingErrorCode::MemberRemoved,
            "logical codebase manifest references a missing or inactive member",
        ));
    }
    selection.validate_focus_subset().map_err(|error| {
        routing_error(
            RepositoryRoutingErrorCode::Inconsistent,
            format!("invalid issue codebase selection: {error}"),
        )
    })
}

fn routing_error(code: RepositoryRoutingErrorCode, reason: impl Into<String>) -> ProductStoreError {
    let stable_code = code.stable_code();
    ProductStoreError::InvalidRecord {
        kind: "repository_routing",
        reason: format!("{stable_code}: {}", reason.into()),
    }
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
    WorkItemRuntimeReader::new(paths.clone()).resolve_active_coding_unit_runtime(attempt)
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
    let mut evidence_warnings = Vec::new();

    if latest_run.is_none() {
        evidence_warnings.push("coder_role_run_missing".to_string());
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
    let dependency_handoff_refs = Vec::new();
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
    use crate::product::logical_codebase::{
        InvalidationRecord, IssueCodebaseSelection, IssueCodebaseSelectionStore,
        LogicalCodebaseManifest, LogicalCodebaseStore, LogicalRepositoryId,
    };
    use crate::product::models::RepositoryRecord;
    use uuid::Uuid;

    #[test]
    fn evaluation_context_logical_state_fail_closed_not_issue_repo() {
        // 有 manifest、selection 已失效 → fail-closed，不得回退 issue.repo_id。
        let root = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        IssueStore::new(paths.clone())
            .create(CreateProductIssueInput {
                project_id: "project_0001".to_string(),
                repo_id: Some("repository_0001".to_string()),
                logical_codebase_id: None,
                title: "评估上下文 fail-closed".to_string(),
                description: None,
                change_id: None,
            })
            .unwrap();
        let logical_repository_id =
            LogicalRepositoryId(Uuid::parse_str("00000000-0000-0000-0000-000000000000").unwrap());
        LogicalCodebaseStore::new(paths.clone())
            .save_manifest(
                "project_0001",
                &LogicalCodebaseManifest::new(
                    "project_0001",
                    root.path().join("aggregate-root"),
                    vec![logical_repository_id],
                ),
            )
            .unwrap();
        let mut selection = IssueCodebaseSelection::explicit(
            "project_0001",
            "issue_0001",
            vec![logical_repository_id],
            Vec::new(),
            vec![logical_repository_id],
            None,
        );
        selection.invalidation = Some(InvalidationRecord {
            reason: "member_removed".to_string(),
            invalidated_at: "2026-08-11T00:00:00Z".to_string(),
        });
        IssueCodebaseSelectionStore::new(paths.clone())
            .save(&selection)
            .unwrap();
        write_physical_repository_fixture(&paths, root.path());
        let attempt = schema_v2_attempt_fixture(None);

        let result = schema_v2_evaluation_context_repository_id(&paths, &attempt);

        assert!(result.is_err());
    }

    #[test]
    fn schema_v2_repository_id_without_snapshot_multi_focus_stays_ambiguous() {
        // selection focus 面（保留，REQ-COD-04 RENAMED 边界）：无快照 attempt +
        // selection 多 focus → TargetAmbiguous 稳定码——focus 面不随分流退役。
        let fixture = schema_v2_routing_fixture();
        let [api, web] = fixture.targets.as_slice() else {
            panic!("fixture must register two targets");
        };
        IssueCodebaseSelectionStore::new(fixture.paths.clone())
            .save(&IssueCodebaseSelection::explicit(
                "project_0001",
                "issue_0001",
                vec![*api, *web],
                Vec::new(),
                vec![*api, *web],
                None,
            ))
            .unwrap();
        let attempt = schema_v2_attempt_fixture(None);

        let error =
            schema_v2_evaluation_context_repository_id(&fixture.paths, &attempt).unwrap_err();

        let ProductStoreError::InvalidRecord { reason, .. } = &error else {
            panic!("expected repository_routing InvalidRecord, got {error:?}");
        };
        assert!(
            reason.starts_with("repository_routing_ambiguous"),
            "focus face must keep TargetAmbiguous, got {reason}"
        );
    }

    #[test]
    fn schema_v2_repository_id_routes_by_frozen_snapshot_over_multi_focus() {
        // D1 路由权威转移（REQ-MTG-01/REQ-COD-04 分流化）：评估上下文面——
        // attempt 冻结快照优先，多 focus 不再阻断 target-attempt 评估。
        // 快照取 build_attempt_target_snapshot 真实权威产物（伪造快照会因
        // 身份不符失败，见 rejects_drifted_snapshot——校验生效证明）。
        let fixture = schema_v2_routing_fixture();
        let [api, web] = fixture.targets.as_slice() else {
            panic!("fixture must register two targets");
        };
        IssueCodebaseSelectionStore::new(fixture.paths.clone())
            .save(&IssueCodebaseSelection::explicit(
                "project_0001",
                "issue_0001",
                vec![*api, *web],
                Vec::new(),
                vec![*api, *web],
                None,
            ))
            .unwrap();
        let snapshot =
            crate::product::coding_attempt_store::target_snapshot::build_attempt_target_snapshot(
                &fixture.paths,
                "project_0001",
                *api,
                None,
            )
            .expect("build authoritative target snapshot");
        let attempt = schema_v2_attempt_fixture(Some(snapshot));

        let repository_id =
            schema_v2_evaluation_context_repository_id(&fixture.paths, &attempt).unwrap();

        assert_eq!(
            repository_id, fixture.api_physical_id,
            "snapshot target must win over multi-focus selection"
        );
    }

    #[test]
    fn schema_v2_repository_id_rejects_drifted_snapshot() {
        // fix round 1（P2）：快照身份校验生效证明——漂移快照（git_dir_identity
        // 伪造）不得静默路由，fail-closed 于权威不一致。
        let fixture = schema_v2_routing_fixture();
        let [api, web] = fixture.targets.as_slice() else {
            panic!("fixture must register two targets");
        };
        IssueCodebaseSelectionStore::new(fixture.paths.clone())
            .save(&IssueCodebaseSelection::explicit(
                "project_0001",
                "issue_0001",
                vec![*api, *web],
                Vec::new(),
                vec![*api, *web],
                None,
            ))
            .unwrap();
        let mut snapshot =
            crate::product::coding_attempt_store::target_snapshot::build_attempt_target_snapshot(
                &fixture.paths,
                "project_0001",
                *api,
                None,
            )
            .expect("build authoritative target snapshot");
        snapshot.git_dir_identity = "sha256:drifted".to_string();
        let attempt = schema_v2_attempt_fixture(Some(snapshot));

        let error =
            schema_v2_evaluation_context_repository_id(&fixture.paths, &attempt).unwrap_err();

        let ProductStoreError::InvalidRecord { reason, .. } = &error else {
            panic!("expected repository_routing InvalidRecord, got {error:?}");
        };
        assert!(
            reason.starts_with("repository_routing_inconsistent"),
            "drifted snapshot must fail closed, got {reason}"
        );
    }

    struct SchemaV2RoutingFixture {
        _root: tempfile::TempDir,
        paths: ProductAppPaths,
        targets: Vec<LogicalRepositoryId>,
        api_physical_id: String,
    }

    fn schema_v2_routing_fixture() -> SchemaV2RoutingFixture {
        let root = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        crate::product::project_store::ProjectStore::new(paths.clone())
            .create(crate::product::project_store::CreateProjectInput {
                name: "schema-v2 routing".to_string(),
                description: None,
            })
            .unwrap();
        IssueStore::new(paths.clone())
            .create(CreateProductIssueInput {
                project_id: "project_0001".to_string(),
                repo_id: None,
                logical_codebase_id: None,
                title: "schema-v2 routing".to_string(),
                description: None,
                change_id: None,
            })
            .unwrap();
        let mut targets = Vec::new();
        let mut api_physical_id = String::new();
        for name in ["api", "web"] {
            let canonical_path = root.path().join(name);
            std::fs::create_dir_all(&canonical_path).unwrap();
            let git = |args: &[&str]| {
                let status = std::process::Command::new("git")
                    .args(args)
                    .current_dir(&canonical_path)
                    .status()
                    .unwrap();
                assert!(status.success(), "git {args:?} failed");
            };
            git(&["init", "--quiet"]);
            git(&["config", "user.email", "builder@example.test"]);
            git(&["config", "user.name", "Builder"]);
            std::fs::write(canonical_path.join("README.md"), format!("# {name}\n")).unwrap();
            git(&["add", "README.md"]);
            git(&["commit", "--quiet", "-m", "initial commit"]);
            let repository =
                crate::product::repository_store::RepositoryStore::with_logical_codebase_feature(
                    paths.clone(),
                    crate::product::logical_codebase::LogicalCodebaseFeature::enabled(),
                )
                .create(crate::product::repository_store::CreateRepositoryInput {
                    project_id: "project_0001".to_string(),
                    name: name.to_string(),
                    path: canonical_path,
                    default_policy_preset: None,
                    default_provider_mode: None,
                    idempotency_key: format!("schema-v2-routing-{name}"),
                })
                .unwrap();
            if name == "api" {
                api_physical_id = repository.id.clone();
            }
            targets.push(
                repository
                    .logical_repository_id
                    .expect("logical repository ID"),
            );
        }
        let manifest = LogicalCodebaseStore::new(paths.clone())
            .load_manifest("project_0001")
            .unwrap()
            .expect("manifest");
        crate::product::logical_codebase::AggregatePolicyArtifactStore::new(paths.clone())
            .ensure_bootstrap(&manifest)
            .unwrap();
        SchemaV2RoutingFixture {
            _root: root,
            paths,
            targets,
            api_physical_id,
        }
    }

    fn schema_v2_attempt_fixture(
        target_snapshot: Option<crate::product::coding_models::AttemptTargetSnapshot>,
    ) -> CodingExecutionAttempt {
        use crate::product::coding_models::{
            CodingAdmissionKind, CodingAttemptScope, CodingAttemptStatus, CodingExecutionStage,
        };
        use crate::product::models::ProviderName;
        use crate::web::workspace_ws_types::ProviderConfigSnapshot;

        CodingExecutionAttempt {
            id: "coding_attempt_0001".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            attempt_no: 1,
            scope: CodingAttemptScope::WorkItemGroup,
            status: CodingAttemptStatus::Running,
            version: 0,
            manual_recovery_reason: None,
            admission_ticket_consumed_at: None,
            admission_kind: CodingAdmissionKind::LegacyGroup,
            stage: CodingExecutionStage::WorktreePrepare,
            base_branch: "main".to_string(),
            branch_name: "aria/attempt".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: None,
                review_rounds: 0,
                permission_modes: Default::default(),
            },
            rework_count: 0,
            max_auto_rework: 0,
            work_item_group_id: Some("work_item_plan_0001".to_string()),
            current_work_item_id: Some("work_item_0001".to_string()),
            active_unit_id: None,
            head_commit: None,
            pushed_remote: None,
            review_request_id: None,
            provider_conversations: Vec::new(),
            created_at: "2026-09-19T00:00:00Z".to_string(),
            updated_at: "2026-09-19T00:00:00Z".to_string(),
            target_snapshot,
            completed_at: None,
        }
    }

    fn write_physical_repository_fixture(paths: &ProductAppPaths, root: &Path) {
        crate::product::json_store::write_json(
            &paths.project_root("project_0001").join("repos.json"),
            &[RepositoryRecord {
                id: "repository_0001".to_string(),
                project_id: "project_0001".to_string(),
                name: "物理仓库".to_string(),
                path: root.join("repository_0001"),
                repo_hash: "sha256:repository".to_string(),
                runtime_root: root.join("repository_0001/.aria/runtime"),
                default_policy_preset: "manual-write".to_string(),
                default_provider_mode: "fake".to_string(),
                created_at: "2026-08-11T00:00:00Z".to_string(),
                logical_repository_id: None,
                primary_checkout_id: None,
                identity_schema_version: 1,
                updated_at: "2026-08-11T00:00:00Z".to_string(),
            }],
        )
        .unwrap();
    }
}
