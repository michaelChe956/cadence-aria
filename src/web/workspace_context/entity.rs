use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    DesignSpecRecord, IssueRecord, IssueWorkItemPlan, RepositoryRecord, SpecVersionRecord,
    StorySpecRecord, WorkspaceSessionRecord, WorkspaceType,
};
use crate::product::project_store::ProjectStore;
use crate::product::repository_store::RepositoryStore;
use crate::product::work_item_runtime_reader::WorkItemRuntimeReader;

pub(super) struct WorkspaceEntityContext {
    pub(super) title: String,
    pub(super) repository_id: String,
    pub(super) linked_context: Vec<String>,
}

pub(super) fn workspace_entity_context(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
    issue: &IssueRecord,
    logical_aggregate: bool,
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
                // 聚合代码库（logical_aggregate）Design 无单一物理仓库，用空串占位；
                // 单仓（Legacy）仍以 issue.repo_id 解析（向后兼容）。
                repository_id: if logical_aggregate {
                    String::new()
                } else {
                    issue_repo_id(issue)?
                },
                linked_context: stories,
            })
        }
        WorkspaceType::WorkItem => {
            let runtime =
                WorkItemRuntimeReader::new(app_paths.clone()).resolve_workspace(session)?;
            let mut linked_context =
                linked_story_context(lifecycle, session, &runtime.lineage.story_spec_refs)?;
            linked_context.extend(linked_design_context(
                lifecycle,
                session,
                &runtime.lineage.design_spec_refs,
            )?);
            Ok(WorkspaceEntityContext {
                title: runtime.projection_bundle.human_projection.title,
                repository_id: issue_repo_id(issue)?,
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
                // 聚合代码库（logical_aggregate）WorkItemPlan 无单一物理仓库，用空串占位；
                // 单仓（Legacy）仍以 issue.repo_id 解析（向后兼容）。
                repository_id: if logical_aggregate {
                    String::new()
                } else {
                    issue_repo_id(issue)?
                },
                linked_context,
            })
        }
    }
}

pub(super) fn work_item_context_summary(
    app_paths: &ProductAppPaths,
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
    let runtime = WorkItemRuntimeReader::new(app_paths.clone()).resolve_workspace(session)?;
    let human = &runtime.projection_bundle.human_projection;
    let inputs = human
        .inputs
        .iter()
        .map(|contract| contract.contract_id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let outputs = human
        .outputs
        .iter()
        .map(|contract| contract.contract_id.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let verification_checks = runtime
        .verification_plan_revision
        .verification_checks
        .iter()
        .map(|check| {
            let instruction = check
                .command
                .as_deref()
                .or(check.manual_instruction.as_deref())
                .unwrap_or("无执行说明");
            format!("- {}: {instruction}", check.check_id)
        })
        .collect::<Vec<_>>()
        .join("\n");
    let human_presentation = runtime.human_presentation.as_ref().map_or_else(
        || "无".to_string(),
        |presentation| {
            format!(
                "human_presentation_id: {}\nhuman_summary: {}\nwhy_split: {}\ndependency_explanation: [{}]\nrisk_explanation: [{}]\nsource_refs: [{}]",
                presentation.id,
                presentation.human_summary,
                presentation.why_split.as_deref().unwrap_or("无"),
                presentation.dependency_explanation.join(", "),
                presentation.risk_explanation.join(", "),
                presentation.source_refs.join(", "),
            )
        },
    );

    Ok(format!(
        "plan_id: {}\nplan_revision_id: {}\nwork_item_revision_id: {}\nprojection_bundle_id: {}\nverification_plan_revision_id: {}\nhuman_projection_hash: {}\ntitle: {}\ngoal: {}\nnon_goals: [{}]\ninput_contracts: [{}]\noutput_contracts: [{}]\ndepends_on: [{}]\nexclusive_write_scopes: [{}]\nforbidden_write_scopes: [{}]\ncompletion_summary: [{}]\nsource_refs: [{}]\nnormative: {}\nused_by_provider: {}\n\n[verification_checks]\n{}\n\n[human_presentation]\n{}",
        runtime.binding.plan_id,
        runtime.binding.plan_revision_id,
        runtime.binding.work_item_revision_id,
        runtime.binding.projection_bundle_id,
        runtime.binding.verification_plan_revision_id,
        runtime.binding.human_projection_hash,
        human.title,
        human.goal,
        human.non_goals.join(", "),
        inputs,
        outputs,
        human.dependencies.join(", "),
        human.scope_summary.owned_scopes.join(", "),
        human.scope_summary.forbidden_scopes.join(", "),
        human.completion_summary.join(", "),
        human.source_refs.join(", "),
        human.normative,
        human.used_by_provider,
        verification_checks,
        human_presentation,
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
    let project = ProjectStore::new(app_paths.clone()).get(project_id)?;
    RepositoryStore::for_project(app_paths.clone(), &project)
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
