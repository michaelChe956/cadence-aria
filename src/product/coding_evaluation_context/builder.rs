use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::WorkspaceType;

use super::methods::required_methods_by_role;
use super::repo::repo_context;
use super::specs::{
    contexts_for_design_specs, contexts_for_story_specs, latest_artifact_version_for_session,
    latest_session_for, work_item_context,
};
use super::{
    EvaluationContextPack, EvaluationContextRole, EvaluationWorkItemContext, OpenSpecContext,
    SuperpowersContext,
};

pub fn build_evaluation_context_pack(
    paths: ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    provider_role: EvaluationContextRole,
) -> Result<EvaluationContextPack, ProductStoreError> {
    let coding_store = CodingAttemptStore::new(paths.clone());
    let quality_bypass_audits = coding_store.list_quality_bypass_audits(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    let lifecycle = LifecycleStore::new(paths);
    let sessions = lifecycle.list_workspace_sessions(&attempt.project_id, &attempt.issue_id)?;
    let work_item = lifecycle
        .list_work_items(&attempt.project_id, &attempt.issue_id)?
        .into_iter()
        .find(|record| record.id == attempt.work_item_id);

    let mut context_warnings = Vec::new();
    let Some(work_item) = work_item else {
        context_warnings.push("missing_work_item".to_string());
        return Ok(EvaluationContextPack {
            issue_id: attempt.issue_id.clone(),
            attempt_id: attempt.id.clone(),
            provider_role,
            story_specs: Vec::new(),
            design_specs: Vec::new(),
            work_item: EvaluationWorkItemContext {
                artifact_id: attempt.work_item_id.clone(),
                version_id: None,
                version: None,
                title: String::new(),
                repository_id: String::new(),
                story_spec_ids: Vec::new(),
                design_spec_ids: Vec::new(),
                raw_markdown_or_sections: String::new(),
                workspace_session_id: None,
            },
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
