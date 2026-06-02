use crate::product::app_paths::ProductAppPaths;
use crate::product::issue_store::IssueStore;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    DesignSpecRecord, IssueRecord, LifecycleWorkItemRecord, ProviderName, RepositoryRecord,
    SpecVersionRecord, StorySpecRecord, WorkspaceMessageRecord, WorkspaceSessionRecord,
    WorkspaceType,
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
    let base = if session.superpowers_enabled {
        match session.workspace_type {
            WorkspaceType::Story | WorkspaceType::Design => {
                "必须遵守 using-superpowers 与 brainstorming。必须优先通过交互提问解决需求、范围、验收标准中的未决问题，并等待用户回答后继续；如果需要向用户提问，必须使用结构化 AskUserQuestion / requestUserInput 交互能力。不要把 A/B/C 选择题作为最终候选产物正文输出，也不要把文本选择题当作正常交互路径；不要把可通过当前用户确认解决的问题直接写入待确认项。只有用户明确要求保留、用户回答后仍需后续确认，或当前 provider 环境确实无法交互时，才允许在待确认项/open_items 中保留。若仍输出了可解析的文本选择题，daemon 只会作为 text_fallback 异常兜底暂停 reviewer 并转换为用户选择卡片，用户回答后仅追加 compact QA，不会重新灌入完整 prompt。"
            }
            WorkspaceType::WorkItem => {
                "必须遵守 using-superpowers 与 writing-plans；只使用 writing-plans 的计划结构要求来生成候选 Work Item artifact，不要执行该技能默认的落盘和执行交接流程。不得直接输出实现代码，先生成可确认的计划与任务拆分。不要创建 docs/superpowers/plans 文件，不要询问 Subagent-Driven 或 Inline Execution；daemon 会负责候选产物落盘和后续执行调度。"
            }
        }
        .to_string()
    } else {
        "Superpowers 未启用；仍需显式说明假设、风险、待确认项与下一步。".to_string()
    };

    if matches!(
        (&session.workspace_type, &session.author_provider),
        (
            WorkspaceType::Story | WorkspaceType::Design,
            ProviderName::ClaudeCode
        )
    ) {
        format!(
            "{base}\n当前 author provider 是 Claude Code；需要向用户确认时，必须使用结构化 AskUserQuestion，让同一个 Claude Code 进程等待用户回答后继续。禁止输出文本 A/B/C 选择题作为交互替代；若仍输出可解析的文本选择题，daemon 仅作为 text_fallback 异常兜底处理，并在用户回答后只追加 compact QA。"
        )
    } else {
        base
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
             最终候选 Markdown 必须用 ```artifact fenced block 包裹，且 fenced block 内第一行必须是 Story Spec 一级标题，例如 # <名称> Story Spec；过程说明必须放在 fenced block 外。每条需求必须显式写稳定 ID，例如 [REQ-001]；每条验收标准必须显式写稳定 ID，例如 [AC-001]。如果通过交互已解决所有疑问，## 待确认项 写“无”；不要为了填充该 heading 编造未决问题。"
        }
        WorkspaceType::Design => {
            "Markdown Design Spec 必须用 ```artifact fenced block 包裹，且 fenced block 内第一行必须是 Design Spec 一级标题；内容必须包含设计范围、关键决策、组件/API/数据模型、风险和追踪关系；关键决策使用 [DEC-001]，组件/API 使用 [CMP-001] 或 [API-001]。"
        }
        WorkspaceType::WorkItem => {
            "Markdown Work Item 必须用 ```artifact fenced block 包裹，且 fenced block 内第一行必须是 Work Item 一级标题；内容必须包含目标、范围、任务拆分、依赖、验证命令、风险和追踪关系；任务使用 [TASK-001]，并绑定来源 Story/Design。"
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

#[cfg(test)]
mod tests {
    use super::{ensure_workspace_context_message, output_schema_for};
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
    use crate::product::lifecycle_store::{
        AppendSpecVersionInput, CreateDesignSpecInput, CreateStorySpecInput, CreateWorkItemInput,
        CreateWorkspaceSessionInput, LifecycleStore,
    };
    use crate::product::models::{
        DesignKind, LifecycleConfirmationStatus, ProviderName, WorkspaceMessageRecord,
        WorkspaceType,
    };
    use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
    use tempfile::tempdir;

    #[test]
    fn all_workspace_artifact_outputs_require_artifact_fence() {
        for workspace_type in [
            WorkspaceType::Story,
            WorkspaceType::Design,
            WorkspaceType::WorkItem,
        ] {
            let schema = output_schema_for(&workspace_type);
            assert!(
                schema.contains("```artifact fenced block"),
                "{workspace_type:?} output schema must require artifact fenced block"
            );
        }
    }

    #[test]
    fn claude_code_story_context_requires_structured_ask_user_question() {
        let root = tempdir().expect("root");
        let repo = tempdir().expect("repo");
        let app_paths = ProductAppPaths::new(root.path().join(".aria"));
        let repository = RepositoryStore::new(app_paths.clone())
            .create(CreateRepositoryInput {
                project_id: "project_0001".to_string(),
                name: "Repo".to_string(),
                path: repo.path().to_path_buf(),
                default_policy_preset: None,
                default_provider_mode: None,
            })
            .expect("repository");
        IssueStore::new(app_paths.clone())
            .create(CreateProductIssueInput {
                project_id: "project_0001".to_string(),
                repo_id: Some(repository.id.clone()),
                title: "爬楼梯问题".to_string(),
                description: Some("使用 Python 实现 climb_stairs".to_string()),
                change_id: None,
            })
            .expect("issue");

        let lifecycle = LifecycleStore::new(app_paths.clone());
        let story = lifecycle
            .create_story_spec(CreateStorySpecInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: repository.id,
                title: "爬楼梯问题 Story Spec".to_string(),
            })
            .expect("story");
        let session = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: story.id,
                workspace_type: WorkspaceType::Story,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: ProviderName::Codex,
                review_rounds: 1,
                superpowers_enabled: true,
                openspec_enabled: true,
            })
            .expect("session");

        let session = ensure_workspace_context_message(&app_paths, &lifecycle, session)
            .expect("workspace context");
        let context = &session.messages[0].content;

        assert!(context.contains("当前 author provider 是 Claude Code"));
        assert!(context.contains("必须使用结构化 AskUserQuestion"));
        assert!(context.contains("禁止输出文本 A/B/C 选择题"));
        assert!(context.contains("text_fallback 异常兜底"));
        assert!(context.contains("只追加 compact QA"));
    }

    #[test]
    fn design_workspace_context_includes_linked_story_markdown() {
        let root = tempdir().expect("root");
        let repo = tempdir().expect("repo");
        let app_paths = ProductAppPaths::new(root.path().join(".aria"));
        let repository = RepositoryStore::new(app_paths.clone())
            .create(CreateRepositoryInput {
                project_id: "project_0001".to_string(),
                name: "Repo".to_string(),
                path: repo.path().to_path_buf(),
                default_policy_preset: None,
                default_provider_mode: None,
            })
            .expect("repository");
        IssueStore::new(app_paths.clone())
            .create(CreateProductIssueInput {
                project_id: "project_0001".to_string(),
                repo_id: Some(repository.id.clone()),
                title: "爬楼梯问题".to_string(),
                description: Some("使用 Python 实现 climb_stairs".to_string()),
                change_id: None,
            })
            .expect("issue");

        let lifecycle = LifecycleStore::new(app_paths.clone());
        let story = lifecycle
            .create_story_spec(CreateStorySpecInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: repository.id,
                title: "爬楼梯问题 Story Spec".to_string(),
            })
            .expect("story");
        lifecycle
            .append_version(AppendSpecVersionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: story.id.clone(),
                markdown: "# 爬楼梯问题 Story Spec\n\n[REQ-001] 返回爬楼梯方法数。".to_string(),
                provider_run_refs: Vec::new(),
                review_refs: Vec::new(),
                confirmed_by: Some("human".to_string()),
            })
            .expect("story version");
        lifecycle
            .update_spec_confirmation_status(
                "project_0001",
                "issue_0001",
                &story.id,
                LifecycleConfirmationStatus::Confirmed,
            )
            .expect("confirm story");
        let design = lifecycle
            .create_design_spec(CreateDesignSpecInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                story_spec_ids: vec![story.id.clone()],
                design_kind: DesignKind::Backend,
                title: "爬楼梯问题 Design Spec".to_string(),
            })
            .expect("design");
        let session = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: design.id,
                workspace_type: WorkspaceType::Design,
                author_provider: ProviderName::Codex,
                reviewer_provider: ProviderName::ClaudeCode,
                review_rounds: 1,
                superpowers_enabled: true,
                openspec_enabled: true,
            })
            .expect("session");

        let session = ensure_workspace_context_message(&app_paths, &lifecycle, session)
            .expect("workspace context");
        let context = &session.messages[0].content;

        assert!(context.contains("- Story Spec: 爬楼梯问题 Story Spec (story_spec_0001)"));
        assert!(context.contains("当前版本: v1"));
        assert!(context.contains("````markdown"));
        assert!(context.contains("# 爬楼梯问题 Story Spec"));
        assert!(context.contains("[REQ-001] 返回爬楼梯方法数。"));
    }

    #[test]
    fn work_item_workspace_context_includes_linked_design_markdown() {
        let root = tempdir().expect("root");
        let repo = tempdir().expect("repo");
        let app_paths = ProductAppPaths::new(root.path().join(".aria"));
        let repository = RepositoryStore::new(app_paths.clone())
            .create(CreateRepositoryInput {
                project_id: "project_0001".to_string(),
                name: "Repo".to_string(),
                path: repo.path().to_path_buf(),
                default_policy_preset: None,
                default_provider_mode: None,
            })
            .expect("repository");
        IssueStore::new(app_paths.clone())
            .create(CreateProductIssueInput {
                project_id: "project_0001".to_string(),
                repo_id: Some(repository.id.clone()),
                title: "爬楼梯问题".to_string(),
                description: Some("使用 Python 实现 climb_stairs".to_string()),
                change_id: None,
            })
            .expect("issue");

        let lifecycle = LifecycleStore::new(app_paths.clone());
        let story = lifecycle
            .create_story_spec(CreateStorySpecInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: repository.id.clone(),
                title: "爬楼梯问题 Story Spec".to_string(),
            })
            .expect("story");
        let design = lifecycle
            .create_design_spec(CreateDesignSpecInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                story_spec_ids: vec![story.id.clone()],
                design_kind: DesignKind::Backend,
                title: "爬楼梯问题 Design Spec".to_string(),
            })
            .expect("design");
        lifecycle
            .append_version(AppendSpecVersionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: design.id.clone(),
                markdown: "# 爬楼梯问题 Design Spec\n\n[DEC-001] 使用迭代动态规划。".to_string(),
                provider_run_refs: Vec::new(),
                review_refs: Vec::new(),
                confirmed_by: Some("human".to_string()),
            })
            .expect("design version");
        lifecycle
            .update_spec_confirmation_status(
                "project_0001",
                "issue_0001",
                &design.id,
                LifecycleConfirmationStatus::Confirmed,
            )
            .expect("confirm design");
        let work_item = lifecycle
            .create_work_item(CreateWorkItemInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: repository.id,
                story_spec_ids: vec![story.id],
                design_spec_ids: vec![design.id],
                title: "实现爬楼梯问题".to_string(),
            })
            .expect("work item");
        let session = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: work_item.id,
                workspace_type: WorkspaceType::WorkItem,
                author_provider: ProviderName::Codex,
                reviewer_provider: ProviderName::ClaudeCode,
                review_rounds: 1,
                superpowers_enabled: true,
                openspec_enabled: true,
            })
            .expect("session");

        let session = ensure_workspace_context_message(&app_paths, &lifecycle, session)
            .expect("workspace context");
        let context = &session.messages[0].content;

        assert!(context.contains("- Design Spec: 爬楼梯问题 Design Spec (design_spec_0001)"));
        assert!(context.contains("# 爬楼梯问题 Design Spec"));
        assert!(context.contains("[DEC-001] 使用迭代动态规划。"));
        assert!(context.contains("只使用 writing-plans 的计划结构要求"));
        assert!(context.contains("不要创建 docs/superpowers/plans 文件"));
        assert!(context.contains("不要询问 Subagent-Driven 或 Inline Execution"));
    }

    #[test]
    fn existing_generation_brief_is_refreshed_when_linked_context_changes() {
        let root = tempdir().expect("root");
        let repo = tempdir().expect("repo");
        let app_paths = ProductAppPaths::new(root.path().join(".aria"));
        let repository = RepositoryStore::new(app_paths.clone())
            .create(CreateRepositoryInput {
                project_id: "project_0001".to_string(),
                name: "Repo".to_string(),
                path: repo.path().to_path_buf(),
                default_policy_preset: None,
                default_provider_mode: None,
            })
            .expect("repository");
        IssueStore::new(app_paths.clone())
            .create(CreateProductIssueInput {
                project_id: "project_0001".to_string(),
                repo_id: Some(repository.id.clone()),
                title: "爬楼梯问题".to_string(),
                description: Some("使用 Python 实现 climb_stairs".to_string()),
                change_id: None,
            })
            .expect("issue");

        let lifecycle = LifecycleStore::new(app_paths.clone());
        let story = lifecycle
            .create_story_spec(CreateStorySpecInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: repository.id,
                title: "爬楼梯问题 Story Spec".to_string(),
            })
            .expect("story");
        lifecycle
            .append_version(AppendSpecVersionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: story.id.clone(),
                markdown: "# 爬楼梯问题 Story Spec\n\n[REQ-001] 返回爬楼梯方法数。".to_string(),
                provider_run_refs: Vec::new(),
                review_refs: Vec::new(),
                confirmed_by: Some("human".to_string()),
            })
            .expect("story version");
        lifecycle
            .update_spec_confirmation_status(
                "project_0001",
                "issue_0001",
                &story.id,
                LifecycleConfirmationStatus::Confirmed,
            )
            .expect("confirm story");
        let design = lifecycle
            .create_design_spec(CreateDesignSpecInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                story_spec_ids: vec![story.id],
                design_kind: DesignKind::Backend,
                title: "爬楼梯问题 Design Spec".to_string(),
            })
            .expect("design");
        let session = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: design.id,
                workspace_type: WorkspaceType::Design,
                author_provider: ProviderName::Codex,
                reviewer_provider: ProviderName::ClaudeCode,
                review_rounds: 1,
                superpowers_enabled: true,
                openspec_enabled: true,
            })
            .expect("session");
        let stale_messages = vec![
            WorkspaceMessageRecord {
                role: "system".to_string(),
                content: "Workspace 生成任务已准备\n\n[system]\n你是 Aria 的候选 design 生成器。\n\n关联上下文:\n- Story Spec: 爬楼梯问题 Story Spec (story_spec_0001)".to_string(),
                created_at: "2026-05-27T00:00:00Z".to_string(),
            },
            WorkspaceMessageRecord {
                role: "user".to_string(),
                content: "开始生成 Design Spec".to_string(),
                created_at: "2026-05-27T00:00:01Z".to_string(),
            },
        ];
        let session = lifecycle
            .replace_workspace_messages(&session.id, stale_messages)
            .expect("replace stale messages");

        let session = ensure_workspace_context_message(&app_paths, &lifecycle, session)
            .expect("workspace context");

        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[1].content, "开始生成 Design Spec");
        assert!(
            session.messages[0]
                .content
                .contains("# 爬楼梯问题 Story Spec")
        );
        assert!(
            session.messages[0]
                .content
                .contains("[REQ-001] 返回爬楼梯方法数。")
        );
    }
}
