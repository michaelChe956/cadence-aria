use crate::product::app_paths::ProductAppPaths;
use crate::product::issue_store::IssueStore;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    DesignSpecRecord, IssueRecord, LifecycleWorkItemRecord, RepositoryRecord, StorySpecRecord,
    WorkspaceMessageRecord, WorkspaceSessionRecord, WorkspaceType,
};
use crate::product::repository_store::RepositoryStore;
use chrono::Utc;

pub fn ensure_workspace_context_message(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
    session: WorkspaceSessionRecord,
) -> Result<WorkspaceSessionRecord, ProductStoreError> {
    let has_generation_brief = session.messages.iter().any(is_generation_brief_message);
    let has_legacy_brief = session.messages.iter().any(is_legacy_context_message);

    if has_generation_brief {
        if has_legacy_brief {
            let messages = session
                .messages
                .into_iter()
                .filter(|message| !is_legacy_context_message(message))
                .collect();
            return lifecycle.replace_workspace_messages(&session.id, messages);
        }
        return Ok(session);
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
         关联上下文:\n{}\n\n\
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
}

fn is_generation_brief_message(message: &WorkspaceMessageRecord) -> bool {
    message.role == "system" && is_workspace_generation_brief(&message.content)
}

fn is_legacy_context_message(message: &WorkspaceMessageRecord) -> bool {
    message.role == "system" && message.content.starts_with("Workspace 上下文已准备")
}

struct WorkspaceEntityContext {
    title: String,
    repository_id: String,
    linked_context: Vec<String>,
}

fn workspace_entity_context(
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
                title: format!(
                    "{} ({})",
                    design.title,
                    design_kind_label(&design.design_kind)
                ),
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
    }
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

fn linked_story_context(
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
    story_spec_ids: &[String],
) -> Result<Vec<String>, ProductStoreError> {
    story_spec_ids
        .iter()
        .map(|id| {
            find_story_spec(lifecycle, session, id)
                .map(|story| format!("- Story Spec: {} ({})", story.title, story.id))
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
            find_design_spec(lifecycle, session, id)
                .map(|design| format!("- Design Spec: {} ({})", design.title, design.id))
        })
        .collect()
}

fn repository_for(
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

fn workspace_type_label(workspace_type: &WorkspaceType) -> &'static str {
    match workspace_type {
        WorkspaceType::Story => "Story Spec",
        WorkspaceType::Design => "Design Spec",
        WorkspaceType::WorkItem => "Work Item",
    }
}

fn node_id_for(workspace_type: &WorkspaceType) -> &'static str {
    match workspace_type {
        WorkspaceType::Story => "N05",
        WorkspaceType::Design => "N07",
        WorkspaceType::WorkItem => "WORK_ITEM",
    }
}

fn workspace_runtime_role(workspace_type: &WorkspaceType) -> &'static str {
    match workspace_type {
        WorkspaceType::Story => "story_spec",
        WorkspaceType::Design => "design_spec",
        WorkspaceType::WorkItem => "work_item",
    }
}

fn system_prompt_for(workspace_type: &WorkspaceType) -> &'static str {
    match workspace_type {
        WorkspaceType::Story => {
            "你是 Aria 的候选 spec 生成器。你负责基于 Issue、Repository 代码上下文和项目规则生成用户可读 Markdown Story Spec 候选；daemon 负责校验、落盘、编译 SpecProjection。"
        }
        WorkspaceType::Design => {
            "你是 Aria 的候选 design 生成器。你负责基于已确认 Story Spec、Repository 代码上下文和项目规则生成候选设计文档；daemon 负责 canonical 校验、落盘与 DesignProjection 编译。"
        }
        WorkspaceType::WorkItem => {
            "你是 Aria 的候选 work item 生成器。你负责基于已确认 Story Spec、Design Spec、Repository 代码上下文和项目规则生成候选工作项与计划输入；daemon 负责校验、落盘与后续执行调度。"
        }
    }
}

fn constraint_summary_for(session: &WorkspaceSessionRecord) -> String {
    if session.openspec_enabled {
        match session.workspace_type {
            WorkspaceType::Story => {
                "OpenSpec 已启用。必须覆盖 Issue 所表达的 proposal constraints；Markdown spec 中必须声明稳定 requirement IDs，供 daemon 在 review pass 后写回 OpenSpec 并编译 requirement_constraints。不要把 OpenSpec 当作 runtime truth。"
                    .to_string()
            }
            WorkspaceType::Design => {
                "OpenSpec 已启用。必须覆盖已确认 Story Spec 的 requirement constraints；设计决策、组件/API 与风险必须可追踪，供 daemon 写回 OpenSpec design constraints。不要把 OpenSpec 当作 runtime truth。"
                    .to_string()
            }
            WorkspaceType::WorkItem => {
                "OpenSpec 已启用。必须覆盖已确认 Story/Design 约束，并产生可追踪的 task/routing 候选，供 daemon 写回 OpenSpec tasks constraints。不要把 OpenSpec 当作 runtime truth。"
                    .to_string()
            }
        }
    } else {
        "OpenSpec 未启用；仍需保持产物结构化、可追踪，并明确记录假设与待确认项。".to_string()
    }
}

fn workflow_discipline_for(session: &WorkspaceSessionRecord) -> String {
    if session.superpowers_enabled {
        match session.workspace_type {
            WorkspaceType::Story | WorkspaceType::Design => {
                "必须遵守 using-superpowers 与 brainstorming。必须优先通过交互提问解决需求、范围、验收标准中的未决问题，并等待用户回答后继续；不要把可通过当前用户确认解决的问题直接写入待确认项。只有用户明确要求保留、用户回答后仍需后续确认，或当前 provider 环境无法交互时，才允许在待确认项/open_items 中保留。"
            }
            WorkspaceType::WorkItem => {
                "必须遵守 using-superpowers 与 writing-plans；不得直接输出实现代码，先生成可确认的计划与任务拆分。"
            }
        }
        .to_string()
    } else {
        "Superpowers 未启用；仍需显式说明假设、风险、待确认项与下一步。".to_string()
    }
}

fn output_schema_for(workspace_type: &WorkspaceType) -> &'static str {
    match workspace_type {
        WorkspaceType::Story => {
            "Markdown Story Spec 必须包含以下 heading：\n\
             - ## 范围\n\
             - ## 用户故事\n\
             - ## 功能需求\n\
             - ## 成功标准\n\
             - ## 待确认项\n\
             - ## 非功能需求\n\n\
             每条需求必须显式写稳定 ID，例如 [REQ-001]；每条验收标准必须显式写稳定 ID，例如 [AC-001]。如果通过交互已解决所有疑问，## 待确认项 写“无”；不要为了填充该 heading 编造未决问题。"
        }
        WorkspaceType::Design => {
            "Markdown Design Spec 必须包含设计范围、关键决策、组件/API/数据模型、风险和追踪关系；关键决策使用 [DEC-001]，组件/API 使用 [CMP-001] 或 [API-001]。"
        }
        WorkspaceType::WorkItem => {
            "Markdown Work Item 必须包含目标、范围、任务拆分、依赖、验证命令、风险和追踪关系；任务使用 [TASK-001]，并绑定来源 Story/Design。"
        }
    }
}

fn completion_or_failure_for(session: &WorkspaceSessionRecord) -> &'static str {
    if session.openspec_enabled {
        "不要直接修改 OpenSpec。不要直接生成 projection。daemon 会做结构化落盘、OpenSpec 写回与约束编译。"
    } else {
        "不要直接生成 projection。daemon 会做结构化落盘与校验。"
    }
}

fn design_kind_label(kind: &crate::product::models::DesignKind) -> &'static str {
    match kind {
        crate::product::models::DesignKind::Frontend => "frontend",
        crate::product::models::DesignKind::Backend => "backend",
    }
}
