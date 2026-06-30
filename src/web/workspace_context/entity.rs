use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    DesignSpecRecord, IssueRecord, IssueWorkItemPlan, LifecycleWorkItemRecord, RepositoryRecord,
    SpecVersionRecord, StorySpecRecord, WorkspaceSessionRecord, WorkspaceType,
};
use crate::product::repository_store::RepositoryStore;

pub(super) struct WorkspaceEntityContext {
    pub(super) title: String,
    pub(super) repository_id: String,
    pub(super) linked_context: Vec<String>,
}

pub(super) fn workspace_entity_context(
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
    issue: &IssueRecord,
) -> Result<WorkspaceEntityContext, ProductStoreError> {
    match session.workspace_type {
        WorkspaceType::Story => {
            let story = find_story_spec(lifecycle, session, &session.entity_id)?;
            Ok(WorkspaceEntityContext {
                title: story.title,
                repository_id: story.repository_id,
                linked_context: Vec::new(),
            })
        }
        WorkspaceType::Design => {
            let design = find_design_spec(lifecycle, session, &session.entity_id)?;
            let stories = linked_story_context(lifecycle, session, &design.story_spec_ids)?;
            Ok(WorkspaceEntityContext {
                title: design.title,
                repository_id: issue_repo_id(issue)?,
                linked_context: stories,
            })
        }
        WorkspaceType::WorkItem => {
            let work_item = find_work_item(lifecycle, session, &session.entity_id)?;
            let mut linked_context =
                linked_story_context(lifecycle, session, &work_item.story_spec_ids)?;
            linked_context.extend(linked_design_context(
                lifecycle,
                session,
                &work_item.design_spec_ids,
            )?);
            Ok(WorkspaceEntityContext {
                title: work_item.title,
                repository_id: work_item.repository_id,
                linked_context,
            })
        }
        WorkspaceType::WorkItemPlan => {
            let plan = find_issue_work_item_plan(lifecycle, session, &session.entity_id)?;
            let mut linked_context =
                linked_story_context(lifecycle, session, &plan.source_story_spec_ids)?;
            linked_context.extend(linked_design_context(
                lifecycle,
                session,
                &plan.source_design_spec_ids,
            )?);
            Ok(WorkspaceEntityContext {
                title: format!("Issue Work Item Plan ({})", plan.id),
                repository_id: issue_repo_id(issue)?,
                linked_context,
            })
        }
    }
}

pub(super) fn work_item_context_summary(
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
) -> Result<String, ProductStoreError> {
    if session.workspace_type == WorkspaceType::WorkItemPlan {
        let plan = find_issue_work_item_plan(lifecycle, session, &session.entity_id)?;
        return Ok(format!(
            "plan_id: {}\nstatus: {:?}\nwork_item_count: {}\nverification_plan_count: {}\ndependency_edge_count: {}",
            plan.id,
            plan.status,
            plan.work_item_ids.len(),
            plan.verification_plan_ids.len(),
            plan.dependency_graph.len()
        ));
    }
    if session.workspace_type != WorkspaceType::WorkItem {
        return Ok(String::new());
    }
    let work_item = find_work_item(lifecycle, session, &session.entity_id)?;
    let verification_plan_summary = if let Some(ref plan_ref) = work_item.verification_plan_ref {
        match lifecycle.get_verification_plan(&session.project_id, &session.issue_id, plan_ref) {
            Ok(plan) => {
                let checks: Vec<String> = plan
                    .commands
                    .iter()
                    .map(|command| format!("- {}: {}", command.label, command.command))
                    .collect();
                if checks.is_empty() {
                    "(no commands)".to_string()
                } else {
                    checks.join("\n")
                }
            }
            Err(_) => "(verification plan not found)".to_string(),
        }
    } else {
        "(no verification plan)".to_string()
    };
    let source_context = if work_item.source_work_item_plan_id.is_some()
        || work_item.source_outline_id.is_some()
        || work_item.source_draft_id.is_some()
        || work_item.planned_implementation_context.is_some()
        || work_item.planned_handoff_summary.is_some()
    {
        format!(
            "\n[work_item_plan_source]\nsource_work_item_plan_id: {}\nsource_outline_id: {}\nsource_draft_id: {}\nplanned_implementation_context:\n{}\nplanned_handoff_summary:\n{}",
            work_item
                .source_work_item_plan_id
                .as_deref()
                .unwrap_or("(none)"),
            work_item.source_outline_id.as_deref().unwrap_or("(none)"),
            work_item.source_draft_id.as_deref().unwrap_or("(none)"),
            work_item
                .planned_implementation_context
                .as_deref()
                .unwrap_or("(none)"),
            work_item
                .planned_handoff_summary
                .as_deref()
                .unwrap_or("(none)")
        )
    } else {
        String::new()
    };

    Ok(format!(
        "kind: {:?}\ndepends_on: [{}]\nexclusive_write_scopes: [{}]\nforbidden_write_scopes: [{}]\ncontext_budget: target_context_k={}, max_summary_chars={}, max_code_context_chars={}\nverification_commands:\n{}{source_context}",
        work_item.kind,
        work_item.depends_on.join(", "),
        work_item.exclusive_write_scopes.join(", "),
        work_item.forbidden_write_scopes.join(", "),
        work_item.context_budget.target_context_k,
        work_item.context_budget.max_summary_chars,
        work_item.context_budget.max_code_context_chars,
        verification_plan_summary
    ))
}

fn find_story_spec(
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
    story_spec_id: &str,
) -> Result<StorySpecRecord, ProductStoreError> {
    lifecycle
        .list_story_specs(&session.project_id, &session.issue_id)?
        .into_iter()
        .find(|story| story.id == story_spec_id)
        .ok_or_else(|| ProductStoreError::NotFound {
            kind: "story_spec",
            id: story_spec_id.to_string(),
        })
}

fn find_design_spec(
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
    design_spec_id: &str,
) -> Result<DesignSpecRecord, ProductStoreError> {
    lifecycle
        .list_design_specs(&session.project_id, &session.issue_id)?
        .into_iter()
        .find(|design| design.id == design_spec_id)
        .ok_or_else(|| ProductStoreError::NotFound {
            kind: "design_spec",
            id: design_spec_id.to_string(),
        })
}

fn find_work_item(
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
    work_item_id: &str,
) -> Result<LifecycleWorkItemRecord, ProductStoreError> {
    lifecycle
        .list_work_items(&session.project_id, &session.issue_id)?
        .into_iter()
        .find(|work_item| work_item.id == work_item_id)
        .ok_or_else(|| ProductStoreError::NotFound {
            kind: "work_item",
            id: work_item_id.to_string(),
        })
}

fn find_issue_work_item_plan(
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
    plan_id: &str,
) -> Result<IssueWorkItemPlan, ProductStoreError> {
    lifecycle
        .list_issue_work_item_plans(&session.project_id, &session.issue_id)?
        .into_iter()
        .find(|plan| plan.id == plan_id)
        .ok_or_else(|| ProductStoreError::NotFound {
            kind: "issue_work_item_plan",
            id: plan_id.to_string(),
        })
}

fn linked_story_context(
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
    story_spec_ids: &[String],
) -> Result<Vec<String>, ProductStoreError> {
    story_spec_ids
        .iter()
        .map(|id| {
            let story = find_story_spec(lifecycle, session, id)?;
            let latest = latest_spec_version(lifecycle, session, &story.id)?;
            Ok(format_linked_spec_context(
                "Story Spec",
                &story.title,
                &story.id,
                latest.as_ref(),
            ))
        })
        .collect()
}

fn linked_design_context(
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
    design_spec_ids: &[String],
) -> Result<Vec<String>, ProductStoreError> {
    design_spec_ids
        .iter()
        .map(|id| {
            let design = find_design_spec(lifecycle, session, id)?;
            let latest = latest_spec_version(lifecycle, session, &design.id)?;
            Ok(format_linked_spec_context(
                "Design Spec",
                &design.title,
                &design.id,
                latest.as_ref(),
            ))
        })
        .collect()
}

fn latest_spec_version(
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
    entity_id: &str,
) -> Result<Option<SpecVersionRecord>, ProductStoreError> {
    Ok(lifecycle
        .list_versions(&session.project_id, &session.issue_id, entity_id)?
        .into_iter()
        .max_by_key(|version| version.version))
}

fn format_linked_spec_context(
    kind: &str,
    title: &str,
    id: &str,
    latest: Option<&SpecVersionRecord>,
) -> String {
    let mut context = format!("- {kind}: {title} ({id})");
    if let Some(version) = latest {
        context.push_str(&format!(
            "\n  当前版本: v{}\n  Markdown:\n````markdown\n{}\n````",
            version.version,
            version.markdown.trim()
        ));
    }
    context
}

pub(super) fn repository_for(
    app_paths: &ProductAppPaths,
    project_id: &str,
    repository_id: &str,
) -> Result<RepositoryRecord, ProductStoreError> {
    RepositoryStore::new(app_paths.clone())
        .list(project_id)?
        .into_iter()
        .find(|repository| repository.id == repository_id)
        .ok_or_else(|| ProductStoreError::NotFound {
            kind: "repository",
            id: repository_id.to_string(),
        })
}

fn issue_repo_id(issue: &IssueRecord) -> Result<String, ProductStoreError> {
    issue
        .repo_id
        .clone()
        .ok_or_else(|| ProductStoreError::NotFound {
            kind: "repository",
            id: format!("issue:{}:repo_id", issue.id),
        })
}
