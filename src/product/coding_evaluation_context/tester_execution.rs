use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    DesignSpecRecord, LifecycleWorkItemRecord, StorySpecRecord, WorkspaceSessionRecord,
    WorkspaceType,
};

use super::builder::build_group_context;
use super::repo::repo_context;
use super::specs::latest_session_for;
use super::{
    EvaluationSourceArtifactRef, TesterExecutionContextPack, TesterExecutionSourceArtifacts,
    TesterExecutionWorkItemContext,
};

pub fn build_tester_execution_context_pack(
    paths: ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<TesterExecutionContextPack, ProductStoreError> {
    let lifecycle_paths = paths.clone();
    let lifecycle = LifecycleStore::new(paths);
    let sessions = lifecycle.list_workspace_sessions(&attempt.project_id, &attempt.issue_id)?;
    let work_items = lifecycle.list_work_items(&attempt.project_id, &attempt.issue_id)?;
    let current_work_item_id = attempt
        .current_work_item_id
        .as_deref()
        .unwrap_or(&attempt.work_item_id);
    let mut context_warnings = Vec::new();
    let group_context = build_group_context(
        lifecycle_paths,
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
        return Ok(TesterExecutionContextPack {
            issue_id: attempt.issue_id.clone(),
            attempt_id: attempt.id.clone(),
            work_item: TesterExecutionWorkItemContext {
                artifact_id: current_work_item_id.to_string(),
                title: String::new(),
                repository_id: String::new(),
                story_spec_ids: Vec::new(),
                design_spec_ids: Vec::new(),
                workspace_session_id: None,
            },
            source_artifacts: TesterExecutionSourceArtifacts {
                story_specs: Vec::new(),
                design_specs: Vec::new(),
            },
            group_context,
            repo_context: repo_context(
                attempt,
                None,
                Some(&attempt.base_branch),
                &mut context_warnings,
            ),
            context_warnings,
        });
    };

    let stories = lifecycle.list_story_specs(&attempt.project_id, &attempt.issue_id)?;
    let designs = lifecycle.list_design_specs(&attempt.project_id, &attempt.issue_id)?;
    let source_artifacts = TesterExecutionSourceArtifacts {
        story_specs: story_refs(
            &lifecycle,
            &attempt.project_id,
            &attempt.issue_id,
            &work_item.story_spec_ids,
            &stories,
            &sessions,
            &mut context_warnings,
        )?,
        design_specs: design_refs(
            &lifecycle,
            &attempt.project_id,
            &attempt.issue_id,
            &work_item.design_spec_ids,
            &designs,
            &sessions,
            &mut context_warnings,
        )?,
    };
    let work_item_session = latest_session_for(&sessions, &work_item.id, &WorkspaceType::WorkItem);

    Ok(TesterExecutionContextPack {
        issue_id: attempt.issue_id.clone(),
        attempt_id: attempt.id.clone(),
        work_item: work_item_context_ref(&work_item, work_item_session),
        source_artifacts,
        group_context,
        repo_context: repo_context(
            attempt,
            Some(&work_item),
            Some(&attempt.base_branch),
            &mut context_warnings,
        ),
        context_warnings,
    })
}

fn story_refs(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    ids: &[String],
    stories: &[StorySpecRecord],
    sessions: &[WorkspaceSessionRecord],
    warnings: &mut Vec<String>,
) -> Result<Vec<EvaluationSourceArtifactRef>, ProductStoreError> {
    let mut refs = Vec::new();
    for id in ids {
        let Some(story) = stories.iter().find(|story| &story.id == id) else {
            warnings.push(format!("missing_story_spec:{id}"));
            continue;
        };
        refs.push(source_artifact_ref(
            lifecycle,
            project_id,
            issue_id,
            &story.id,
            &story.title,
            latest_session_for(sessions, id, &WorkspaceType::Story),
        )?);
    }
    Ok(refs)
}

fn design_refs(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    ids: &[String],
    designs: &[DesignSpecRecord],
    sessions: &[WorkspaceSessionRecord],
    warnings: &mut Vec<String>,
) -> Result<Vec<EvaluationSourceArtifactRef>, ProductStoreError> {
    let mut refs = Vec::new();
    for id in ids {
        let Some(design) = designs.iter().find(|design| &design.id == id) else {
            warnings.push(format!("missing_design_spec:{id}"));
            continue;
        };
        refs.push(source_artifact_ref(
            lifecycle,
            project_id,
            issue_id,
            &design.id,
            &design.title,
            latest_session_for(sessions, id, &WorkspaceType::Design),
        )?);
    }
    Ok(refs)
}

fn source_artifact_ref(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    artifact_id: &str,
    title: &str,
    session: Option<&WorkspaceSessionRecord>,
) -> Result<EvaluationSourceArtifactRef, ProductStoreError> {
    let version = lifecycle
        .list_versions(project_id, issue_id, artifact_id)?
        .into_iter()
        .max_by_key(|version| version.version);
    Ok(EvaluationSourceArtifactRef {
        artifact_id: artifact_id.to_string(),
        version_id: version.as_ref().map(|version| version.id.clone()),
        version: version.as_ref().map(|version| version.version),
        title: title.to_string(),
        workspace_session_id: session.map(|session| session.id.clone()),
    })
}

fn work_item_context_ref(
    work_item: &LifecycleWorkItemRecord,
    session: Option<&WorkspaceSessionRecord>,
) -> TesterExecutionWorkItemContext {
    TesterExecutionWorkItemContext {
        artifact_id: work_item.id.clone(),
        title: work_item.title.clone(),
        repository_id: work_item.repository_id.clone(),
        story_spec_ids: work_item.story_spec_ids.clone(),
        design_spec_ids: work_item.design_spec_ids.clone(),
        workspace_session_id: session.map(|session| session.id.clone()),
    }
}
