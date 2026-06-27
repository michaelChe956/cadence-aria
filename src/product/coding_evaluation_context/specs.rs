use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    DesignSpecRecord, LifecycleWorkItemRecord, SpecVersionRecord, StorySpecRecord,
    WorkspaceSessionRecord, WorkspaceType,
};
use crate::web::workspace_ws_types::ArtifactVersion;

use super::sanitize::{push_warning_once, sanitize_context_text};
use super::{EvaluationSpecContext, EvaluationWorkItemContext};

pub(super) fn contexts_for_story_specs(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    ids: &[String],
    stories: &[StorySpecRecord],
    sessions: &[WorkspaceSessionRecord],
    warnings: &mut Vec<String>,
) -> Result<Vec<EvaluationSpecContext>, ProductStoreError> {
    let mut contexts = Vec::new();
    for id in ids {
        let Some(story) = stories.iter().find(|story| &story.id == id) else {
            warnings.push(format!("missing_story_spec:{id}"));
            continue;
        };
        let version = latest_version(lifecycle, project_id, issue_id, id)?;
        let session = latest_session_for(sessions, id, &WorkspaceType::Story);
        contexts.push(spec_context(
            &story.id,
            &story.title,
            version.as_ref(),
            session,
            warnings,
        ));
    }
    Ok(contexts)
}

pub(super) fn contexts_for_design_specs(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    ids: &[String],
    designs: &[DesignSpecRecord],
    sessions: &[WorkspaceSessionRecord],
    warnings: &mut Vec<String>,
) -> Result<Vec<EvaluationSpecContext>, ProductStoreError> {
    let mut contexts = Vec::new();
    for id in ids {
        let Some(design) = designs.iter().find(|design| &design.id == id) else {
            warnings.push(format!("missing_design_spec:{id}"));
            continue;
        };
        let version = latest_version(lifecycle, project_id, issue_id, id)?;
        let session = latest_session_for(sessions, id, &WorkspaceType::Design);
        contexts.push(spec_context(
            &design.id,
            &design.title,
            version.as_ref(),
            session,
            warnings,
        ));
    }
    Ok(contexts)
}

fn latest_version(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    entity_id: &str,
) -> Result<Option<SpecVersionRecord>, ProductStoreError> {
    Ok(lifecycle
        .list_versions(project_id, issue_id, entity_id)?
        .into_iter()
        .max_by_key(|version| version.version))
}

pub(super) fn latest_artifact_version_for_session(
    lifecycle: &LifecycleStore,
    session: Option<&WorkspaceSessionRecord>,
) -> Result<Option<ArtifactVersion>, ProductStoreError> {
    let Some(session) = session else {
        return Ok(None);
    };
    Ok(lifecycle
        .list_artifact_versions(&session.id)?
        .into_iter()
        .filter(|version| version.is_current)
        .max_by_key(|version| version.version))
}

pub(super) fn latest_session_for<'a>(
    sessions: &'a [WorkspaceSessionRecord],
    entity_id: &str,
    workspace_type: &WorkspaceType,
) -> Option<&'a WorkspaceSessionRecord> {
    sessions
        .iter()
        .filter(|session| {
            session.entity_id == entity_id && &session.workspace_type == workspace_type
        })
        .max_by(|left, right| {
            left.updated_at
                .cmp(&right.updated_at)
                .then_with(|| left.created_at.cmp(&right.created_at))
        })
}

fn spec_context(
    artifact_id: &str,
    title: &str,
    version: Option<&SpecVersionRecord>,
    session: Option<&WorkspaceSessionRecord>,
    warnings: &mut Vec<String>,
) -> EvaluationSpecContext {
    let (raw_markdown_or_sections, truncated) = sanitize_context_text(
        &version
            .map(|version| version.markdown.clone())
            .unwrap_or_default(),
    );
    if truncated {
        push_warning_once(warnings, "context_truncated");
    }
    EvaluationSpecContext {
        artifact_id: artifact_id.to_string(),
        version_id: version.map(|version| version.id.clone()),
        version: version.map(|version| version.version),
        title: title.to_string(),
        raw_markdown_or_sections,
        workspace_session_id: session.map(|session| session.id.clone()),
    }
}

pub(super) fn work_item_context(
    work_item: &LifecycleWorkItemRecord,
    version: Option<&ArtifactVersion>,
    session: Option<&WorkspaceSessionRecord>,
    warnings: &mut Vec<String>,
) -> EvaluationWorkItemContext {
    let (raw_markdown_or_sections, truncated) = sanitize_context_text(
        version
            .map(|version| version.markdown())
            .unwrap_or_default(),
    );
    if truncated {
        push_warning_once(warnings, "context_truncated");
    }
    EvaluationWorkItemContext {
        artifact_id: work_item.id.clone(),
        version_id: version.map(|version| format!("artifact_version_{:04}", version.version)),
        version: version.map(|version| version.version),
        title: work_item.title.clone(),
        repository_id: work_item.repository_id.clone(),
        story_spec_ids: work_item.story_spec_ids.clone(),
        design_spec_ids: work_item.design_spec_ids.clone(),
        raw_markdown_or_sections,
        workspace_session_id: session.map(|session| session.id.clone()),
    }
}
