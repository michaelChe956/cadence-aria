use super::entity::{repository_for, work_item_context_summary, workspace_entity_context};
use super::prompts::{
    completion_or_failure_for, constraint_summary_for, node_id_for, output_schema_for,
    system_prompt_for, workflow_discipline_for, workspace_runtime_role, workspace_type_label,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::issue_store::IssueStore;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{WorkspaceMessageRecord, WorkspaceSessionRecord};
use chrono::Utc;

pub fn ensure_workspace_context_message(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
    session: WorkspaceSessionRecord,
) -> Result<WorkspaceSessionRecord, ProductStoreError> {
    let has_generation_brief = session.messages.iter().any(is_generation_brief_message);
    let has_legacy_brief = session.messages.iter().any(is_legacy_context_message);

    if has_generation_brief {
        let content = build_workspace_context_message(app_paths, lifecycle, &session)?;
        if !has_legacy_brief
            && session
                .messages
                .iter()
                .any(|message| is_generation_brief_message(message) && message.content == content)
        {
            return Ok(session);
        }

        let mut messages: Vec<WorkspaceMessageRecord> = session
            .messages
            .clone()
            .into_iter()
            .filter(|message| !is_legacy_context_message(message))
            .collect();
        if let Some(message) = messages
            .iter_mut()
            .find(|message| is_generation_brief_message(message))
        {
            message.content = content;
            message.created_at = Utc::now().to_rfc3339();
        } else {
            messages.insert(
                0,
                WorkspaceMessageRecord {
                    role: "system".to_string(),
                    content,
                    created_at: Utc::now().to_rfc3339(),
                },
            );
        }
        return lifecycle.replace_workspace_messages(&session.id, messages);
    }

    let content = build_workspace_context_message(app_paths, lifecycle, &session)?;
    let mut messages: Vec<WorkspaceMessageRecord> = session
        .messages
        .into_iter()
        .filter(|message| !is_legacy_context_message(message))
        .collect();
    messages.insert(
        0,
        WorkspaceMessageRecord {
            role: "system".to_string(),
            content,
            created_at: Utc::now().to_rfc3339(),
        },
    );
    lifecycle.replace_workspace_messages(&session.id, messages)
}

fn build_workspace_context_message(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
) -> Result<String, ProductStoreError> {
    let issue = IssueStore::new(app_paths.clone()).get(&session.project_id, &session.issue_id)?;
    let entity = workspace_entity_context(lifecycle, session, &issue)?;
    let repository = repository_for(app_paths, &session.project_id, &entity.repository_id)?;
    let issue_description = issue
        .description
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("无");
    let linked_context = if entity.linked_context.is_empty() {
        "无".to_string()
    } else {
        entity.linked_context.join("\n")
    };

    let work_item_context = work_item_context_summary(lifecycle, session)?;
    let work_item_context_block = if work_item_context.is_empty() {
        String::new()
    } else {
        format!("\n\n[work_item_context]\n{work_item_context}")
    };

    Ok(format!(
        "Workspace 生成任务已准备\n\n\
         [system]\n\
         {}\n\n\
         [node_contract]\n\
         node_id={}\n\
         runtime_role=workspace_{}\n\
         adapter_role=orchestrator\n\
         advisory_only=false\n\n\
         [canonical_inputs]\n\
         Workspace 类型: {}\n\
         目标产物: {} ({})\n\
         Issue: {} ({})\n\
         Issue 描述: {}\n\
         Repository: {} ({})\n\
         Repository 路径: {}\n\
         关联上下文:\n{}{}\n\n\
         [constraint_summary]\n\
         {}\n\n\
         [workflow_discipline]\n\
         {}\n\n\
         [output_schema]\n\
         {}\n\n\
         [completion_or_failure]\n\
         {}",
        system_prompt_for(&session.workspace_type),
        node_id_for(&session.workspace_type),
        workspace_runtime_role(&session.workspace_type),
        workspace_type_label(&session.workspace_type),
        entity.title,
        session.entity_id,
        issue.title,
        issue.id,
        issue_description,
        repository.name,
        repository.id,
        repository.path.display(),
        linked_context,
        work_item_context_block,
        constraint_summary_for(session),
        workflow_discipline_for(session),
        output_schema_for(&session.workspace_type),
        completion_or_failure_for(session),
    ))
}

fn is_workspace_generation_brief(content: &str) -> bool {
    content.contains("候选 spec 生成器")
        || content.contains("候选 design 生成器")
        || content.contains("候选 work item 生成器")
        || content.contains("候选 work item plan 生成器")
}

fn is_generation_brief_message(message: &WorkspaceMessageRecord) -> bool {
    message.role == "system" && is_workspace_generation_brief(&message.content)
}

fn is_legacy_context_message(message: &WorkspaceMessageRecord) -> bool {
    message.role == "system" && message.content.starts_with("Workspace 上下文已准备")
}
