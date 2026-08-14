use super::entity::{repository_for, work_item_context_summary, workspace_entity_context};
use super::prompts::{
    completion_or_failure_for, constraint_summary_for, node_id_for, output_schema_for,
    runtime_contract_for, system_prompt_for, workflow_discipline_for, workspace_runtime_role,
    workspace_type_label,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::cadence_skills::routing_reference::{
    LogicalPolicyReference, RoutingReferenceContext,
};
use crate::product::issue_store::IssueStore;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::logical_codebase::{
    AggregatePolicyArtifactStore, LogicalCodebaseStore, PlanningContextResolver, RepositoryRouting,
    RepositoryRoutingErrorCode,
};
use crate::product::models::{WorkspaceMessageRecord, WorkspaceSessionRecord, WorkspaceType};
use crate::product::work_item_runtime_reader::WorkItemRuntimeReader;
use crate::product::workspace_engine::{
    aggregate_design_scope_prompt, aggregate_story_scope_prompt,
    aggregate_work_item_target_scope_prompt,
};
use chrono::Utc;

pub fn ensure_workspace_context_message(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
    session: WorkspaceSessionRecord,
) -> Result<WorkspaceSessionRecord, ProductStoreError> {
    let has_generation_brief = session.messages.iter().any(is_generation_brief_message);
    let has_runtime_context = session.messages.iter().any(is_runtime_context_message);
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

    if has_runtime_context {
        let content = build_workspace_context_message(app_paths, lifecycle, &session)?;
        if !has_legacy_brief
            && session
                .messages
                .iter()
                .any(|message| is_runtime_context_message(message) && message.content == content)
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
            .find(|message| is_runtime_context_message(message))
        {
            message.content = content;
            message.created_at = Utc::now().to_rfc3339();
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
    if session.workspace_type == WorkspaceType::WorkItem {
        WorkItemRuntimeReader::new(app_paths.clone()).resolve_workspace(session)?;
    }
    let issue = IssueStore::new(app_paths.clone()).get(&session.project_id, &session.issue_id)?;
    // 方案 X 阶段1：按 RepositoryRouting 分流。Logical（聚合代码库）Story/Design 无单一
    // 物理仓库，以聚合根 cwd 为仓库上下文并注入聚合视野 prompt；Legacy（单仓）保持原
    // 物理仓库解析，向后兼容；FailClosed 提前拦截报稳定错误码 repository_routing_*。
    // C1 修复：routing 判定在 workspace_entity_context 之前，Logical Story/Design 传
    // logical_aggregate 标志（Design 分支不再依赖 issue.repo_id，多仓 issue 可达）。
    // Task 7 扩展：WorkItemPlan 同样走聚合视野（无单一物理仓库，entity 用空串占位，
    // prompt 注入 target 集合），解决“WorkItemPlan+Logical 仍依赖 issue.repo_id”遗留。
    let routing =
        RepositoryRouting::load_for_issue(app_paths, &session.project_id, &session.issue_id)?;
    let aggregate_planning = match &routing {
        RepositoryRouting::FailClosed { code, reason } => {
            return Err(routing_error_for_builder(*code, reason.clone()));
        }
        RepositoryRouting::Logical { .. }
            if matches!(
                session.workspace_type,
                WorkspaceType::Story | WorkspaceType::Design | WorkspaceType::WorkItemPlan
            ) =>
        {
            // targets 空 → AI 自决 involved；snapshot 为权威 effective_member_ids。
            Some(PlanningContextResolver::new(app_paths.clone()).build(
                &session.project_id,
                &session.issue_id,
                &[],
            )?)
        }
        _ => None,
    };
    let entity = workspace_entity_context(
        app_paths,
        lifecycle,
        session,
        &issue,
        aggregate_planning.is_some(),
    )?;
    let (repository_name, repository_id_label, repository_path) =
        if let Some(resolved) = aggregate_planning.as_ref() {
            (
                "聚合代码库".to_string(),
                "<logical>".to_string(),
                resolved.cwd.display().to_string(),
            )
        } else {
            let repository = repository_for(app_paths, &session.project_id, &entity.repository_id)?;
            (
                repository.name,
                repository.id,
                repository.path.display().to_string(),
            )
        };
    let aggregate_prompt =
        aggregate_planning
            .as_ref()
            .map(|resolved| match session.workspace_type {
                WorkspaceType::Story => aggregate_story_scope_prompt(
                    &resolved.inventory_injection.rendered,
                    &resolved.snapshot.effective_member_ids,
                ),
                WorkspaceType::WorkItemPlan => aggregate_work_item_target_scope_prompt(
                    &resolved.inventory_injection.rendered,
                    &resolved.snapshot.effective_member_ids,
                ),
                _ => aggregate_design_scope_prompt(
                    &resolved.inventory_injection.rendered,
                    &resolved.snapshot.effective_member_ids,
                ),
            });
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

    let work_item_context = work_item_context_summary(app_paths, lifecycle, session)?;
    let work_item_context_block = if work_item_context.is_empty() {
        String::new()
    } else {
        format!("\n\n[work_item_context]\n{work_item_context}")
    };
    let runtime_contract = runtime_contract_for(session);
    let routing_context = routing_reference_context_for_project(app_paths, &session.project_id);

    let mut message = format!(
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
         [runtime_contract]\n\
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
        repository_name,
        repository_id_label,
        repository_path,
        linked_context,
        work_item_context_block,
        constraint_summary_for(session),
        runtime_contract,
        workflow_discipline_for(session, &routing_context),
        output_schema_for(&session.workspace_type),
        completion_or_failure_for(session),
    );
    if let Some(aggregate_prompt) = aggregate_prompt {
        message.push_str(&aggregate_prompt);
    }
    Ok(message)
}

/// workspace 上下文 prompt 注入用的路由引用上下文(store-backed 受控 artifact 引用)。
///
/// 逻辑代码库(有 manifest + 已 bootstrap 的 `AggregatePolicyArtifact`)时派生
/// `Logical`:`policy_id`/`policy_revision`/`policy_digest` 来自 persisted artifact,
/// `authority_root` 取 `LogicalCodebaseManifest.provider_context_root`。两者与
/// gateway envelope 同源,无漂移。无 manifest/artifact 或读取失败一律 `Legacy`
/// (与改造前 `_legacy()` 字节一致)。
pub(super) fn routing_reference_context_for_project(
    app_paths: &ProductAppPaths,
    project_id: &str,
) -> RoutingReferenceContext {
    let Ok(Some(manifest)) = LogicalCodebaseStore::new(app_paths.clone()).load_manifest(project_id)
    else {
        return RoutingReferenceContext::Legacy;
    };
    let Ok(Some(artifact)) = AggregatePolicyArtifactStore::new(app_paths.clone()).get(project_id)
    else {
        return RoutingReferenceContext::Legacy;
    };
    RoutingReferenceContext::Logical(LogicalPolicyReference {
        policy_id: artifact.policy_id,
        policy_revision: artifact.revision,
        policy_digest: artifact.digest,
        authority_root: manifest.provider_context_root.to_string_lossy().to_string(),
    })
}

/// FailClosed 稳定错误码映射（B3）：ProductStoreError 经 product_store_api_error 的
/// routing_error_code_from_reason 映射回 repository_routing_* HTTP 稳定错误码。
fn routing_error_for_builder(
    code: RepositoryRoutingErrorCode,
    reason: impl Into<String>,
) -> ProductStoreError {
    ProductStoreError::InvalidRecord {
        kind: "repository_routing",
        reason: format!("{}: {}", code.stable_code(), reason.into()),
    }
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

fn is_runtime_context_message(message: &WorkspaceMessageRecord) -> bool {
    message.role == "system" && message.content.starts_with("Workspace 生成任务已准备")
}
