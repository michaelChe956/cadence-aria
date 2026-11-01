use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    LifecycleWorkItemRecord, VerificationPlan, WorkspaceSessionRecord, WorkspaceSessionStatus,
    WorkspaceType,
};
use crate::product::test_executor::planned_test_commands_from_markdown;
use crate::product::work_item_plan_store::WorkItemPlanStore;
use crate::product::workspace_engine::selected_workspace_artifact_markdown;

/// 规范写入范围段的 markdown 标题（渲染器与 coder prompt 兜底判定共用，改名必须两处同步）。
pub(crate) const WRITE_SCOPE_SECTION_HEADING: &str = "写入范围（硬边界）";

/// 规范写入范围段内的「允许修改范围」子节标题（材料标题与 reviewer/coder 协议
/// 锚点同源，改名必须两处同步）。
pub(crate) const WRITE_SCOPE_EXCLUSIVE_LIST_HEADING: &str =
    "允许修改范围（exclusive_write_scopes）";

/// 规范写入范围段内的「禁止修改范围」子节标题（材料标题与 reviewer/coder 协议
/// 锚点同源，改名必须两处同步）。
pub(crate) const WRITE_SCOPE_FORBIDDEN_LIST_HEADING: &str =
    "禁止修改范围（forbidden_write_scopes）";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CompiledCodingWorkItemContext {
    pub(crate) markdown: Option<String>,
    pub(crate) verification_commands: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct CompiledWorkItemContext {
    markdown: Option<String>,
    verification_commands: Vec<String>,
    needs_workspace_artifact_fallback: bool,
}

pub(crate) fn load_coding_work_item_context(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<CompiledCodingWorkItemContext, ProductStoreError> {
    if is_schema_v2_group_attempt(app_paths, attempt)? {
        return bound_coder_work_item_context(app_paths, attempt);
    }

    let current_work_item_id = attempt
        .current_work_item_id
        .as_deref()
        .unwrap_or(&attempt.work_item_id);
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let compiled =
        compiled_work_item_context(&lifecycle, app_paths, attempt, current_work_item_id)?;
    let workspace_markdown =
        if compiled.markdown.is_none() || compiled.needs_workspace_artifact_fallback {
            workspace_artifact_work_item_markdown(&lifecycle, attempt, current_work_item_id)?
        } else {
            None
        };
    let markdown = merge_work_item_markdown(compiled.markdown, workspace_markdown);
    let verification_commands =
        merge_verification_commands(compiled.verification_commands, markdown.as_deref());

    Ok(CompiledCodingWorkItemContext {
        markdown,
        verification_commands,
    })
}

fn is_schema_v2_group_attempt(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<bool, ProductStoreError> {
    if attempt.scope != crate::product::coding_models::CodingAttemptScope::WorkItemGroup {
        return Ok(false);
    }
    let Some(plan_id) = attempt.work_item_group_id.as_deref() else {
        return Ok(false);
    };
    match crate::product::work_item_revision_store::WorkItemRevisionStore::new(app_paths.clone())
        .get_plan_lineage(&attempt.project_id, &attempt.issue_id, plan_id)
    {
        Ok(_) => Ok(true),
        Err(ProductStoreError::NotFound {
            kind: "work_item_plan_lineage",
            ..
        }) => Ok(false),
        Err(error) => Err(error),
    }
}

fn bound_coder_work_item_context(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<CompiledCodingWorkItemContext, ProductStoreError> {
    let coding_store =
        crate::product::coding_attempt_store::CodingAttemptStore::new(app_paths.clone());
    let Some(unit) =
        coding_store.get_active_coding_unit(&attempt.project_id, &attempt.issue_id, &attempt.id)?
    else {
        return Ok(CompiledCodingWorkItemContext::default());
    };
    let run = coding_store
        .list_coding_unit_runs(attempt, &unit.id)?
        .into_iter()
        .max_by_key(|run| run.execution_no);
    let reader =
        crate::product::work_item_runtime_reader::WorkItemRuntimeReader::new(app_paths.clone());
    let (projection, coder_projection_hash) =
        reader.coder_projection_for_unit(attempt, &unit, run.as_ref())?;
    let runtime = reader.normative_context_for_unit(attempt, &unit, run.as_ref())?;
    let projection_json = serde_json::to_string_pretty(&projection)
        .map_err(|error| ProductStoreError::Json(error.to_string()))?;
    let verification_commands = runtime
        .verification_plan_revision
        .verification_checks
        .iter()
        .filter_map(|check| check.command.as_deref())
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .map(str::to_string)
        .collect();

    let mut markdown = format!(
        "# Bound Coder Context\n\n- Plan revision: {}\n- Work item revision: {}\n- Coder projection hash: {}\n\n## Coder Projection\n\n{}\n",
        runtime.binding.plan_revision_id,
        runtime.binding.work_item_revision_id,
        coder_projection_hash,
        projection_json,
    );
    // 范围段与 legacy 路径同源同名：数据取 coder projection 的 write_policy，
    // 即 runtime.work_item_revision.canonical_contract.write_policy（机器门
    // `gates/schema_v2.rs::validate_changed_files_for_runtime` 的同一份取值）。
    // 缺这段时 group 兜底路径（rendered_context 缺失）会误报「范围未知」。
    markdown.push_str(&write_scope_section_text(
        &projection.write_policy.exclusive_scopes,
        &projection.write_policy.forbidden_scopes,
    ));

    Ok(CompiledCodingWorkItemContext {
        markdown: Some(markdown),
        verification_commands,
    })
}

fn compiled_work_item_context(
    lifecycle: &LifecycleStore,
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    current_work_item_id: &str,
) -> Result<CompiledWorkItemContext, ProductStoreError> {
    let Some(work_item) = lifecycle
        .list_work_items(&attempt.project_id, &attempt.issue_id)?
        .into_iter()
        .find(|item| item.id == current_work_item_id)
    else {
        return Ok(CompiledWorkItemContext::default());
    };

    let verification_plan = match work_item.verification_plan_ref.as_deref() {
        Some(plan_id) => Some(lifecycle.get_verification_plan(
            &attempt.project_id,
            &attempt.issue_id,
            plan_id,
        )?),
        None => None,
    };
    let verification_commands = verification_plan
        .as_ref()
        .map(verification_command_lines)
        .unwrap_or_default();
    let needs_draft_supplement = needs_source_draft_supplement(&work_item);
    let draft_supplement = final_compile_draft_supplement(app_paths, attempt, &work_item)?;
    let needs_workspace_artifact_fallback = needs_draft_supplement && draft_supplement.is_none();

    Ok(CompiledWorkItemContext {
        markdown: Some(compiled_work_item_markdown(
            &work_item,
            verification_plan.as_ref(),
            draft_supplement.as_deref(),
        )),
        verification_commands,
        needs_workspace_artifact_fallback,
    })
}

fn needs_source_draft_supplement(work_item: &LifecycleWorkItemRecord) -> bool {
    work_item
        .planned_implementation_context
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .is_empty()
}

fn final_compile_draft_supplement(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    work_item: &LifecycleWorkItemRecord,
) -> Result<Option<String>, ProductStoreError> {
    if !needs_source_draft_supplement(work_item) {
        return Ok(None);
    }
    let Some(plan_id) = work_item.source_work_item_plan_id.as_deref() else {
        return Ok(None);
    };
    let Some(draft_id) = work_item.source_draft_id.as_deref() else {
        return Ok(None);
    };

    let draft = WorkItemPlanStore::new(app_paths.clone())
        .list_draft_records(&attempt.project_id, &attempt.issue_id, plan_id)?
        .into_iter()
        .find(|record| record.draft_id == draft_id);

    let Some(draft) = draft else {
        return Ok(None);
    };

    let mut markdown = String::new();
    markdown.push_str(&format!("- Draft ID: {}\n", draft.draft_id));
    markdown.push_str(&format!("- Outline ID: {}\n", draft.outline_id));
    let canonical_contract =
        serde_json::to_string_pretty(&draft.candidate.canonical_contract_candidate)
            .map_err(|error| ProductStoreError::Json(error.to_string()))?;
    push_markdown_section(
        &mut markdown,
        "Draft Canonical Contract Candidate JSON",
        Some(&canonical_contract),
    );
    let verification_plan = serde_json::to_string_pretty(&draft.candidate.verification_plan)
        .map_err(|error| ProductStoreError::Json(error.to_string()))?;
    push_markdown_section(
        &mut markdown,
        "Draft Verification Plan JSON",
        Some(&verification_plan),
    );

    Ok((!markdown.trim().is_empty()).then_some(markdown))
}

fn workspace_artifact_work_item_markdown(
    lifecycle: &LifecycleStore,
    attempt: &CodingExecutionAttempt,
    current_work_item_id: &str,
) -> Result<Option<String>, ProductStoreError> {
    let sessions = lifecycle.list_workspace_sessions(&attempt.project_id, &attempt.issue_id)?;
    let work_item_session = sessions
        .iter()
        .rev()
        .find(|session| {
            session.entity_id == current_work_item_id
                && session.workspace_type == WorkspaceType::WorkItem
                && session.status == WorkspaceSessionStatus::Confirmed
        })
        .or_else(|| {
            sessions.iter().rev().find(|session| {
                session.entity_id == current_work_item_id
                    && session.workspace_type == WorkspaceType::WorkItem
            })
        });
    Ok(match work_item_session {
        Some(session) => lifecycle
            .list_artifact_versions(&session.id)?
            .into_iter()
            .last()
            .map(|version| version.to_markdown_string())
            .and_then(|markdown| select_work_item_markdown(Some(markdown), session))
            .or_else(|| select_work_item_markdown(None, session)),
        None => None,
    })
}

fn compiled_work_item_markdown(
    work_item: &LifecycleWorkItemRecord,
    verification_plan: Option<&VerificationPlan>,
    draft_supplement: Option<&str>,
) -> String {
    let mut markdown = String::new();
    markdown.push_str("# Final Compile Work Item\n\n");
    markdown.push_str(&format!("- Work Item ID: {}\n", work_item.id));
    markdown.push_str(&format!("- Title: {}\n", work_item.title));
    markdown.push_str(&format!("- Kind: {}\n", work_item.kind.as_str()));
    push_optional_line(
        &mut markdown,
        "source_work_item_plan_id",
        work_item.source_work_item_plan_id.as_deref(),
    );
    push_optional_line(
        &mut markdown,
        "source_outline_id",
        work_item.source_outline_id.as_deref(),
    );
    push_optional_line(
        &mut markdown,
        "source_draft_id",
        work_item.source_draft_id.as_deref(),
    );
    push_optional_line(
        &mut markdown,
        "verification_plan_ref",
        work_item.verification_plan_ref.as_deref(),
    );

    push_markdown_section(
        &mut markdown,
        "Planned Implementation Context",
        work_item.planned_implementation_context.as_deref(),
    );
    push_string_list(&mut markdown, "Story Spec IDs", &work_item.story_spec_ids);
    push_string_list(&mut markdown, "Design Spec IDs", &work_item.design_spec_ids);
    push_string_list(&mut markdown, "Depends On", &work_item.depends_on);
    // 写入范围是 coder / reviewer / evaluation context 三方共用的单一载体：
    // 只在这里渲染一份规范段（group 投影路径由同一渲染器
    // `write_scope_section_text` 在最终 prompt 追加同一份文案），prompt 侧不再
    // 重复渲染。
    push_write_scope_section(&mut markdown, work_item);

    if let Some(plan) = verification_plan {
        markdown.push_str("\n## Verification Plan\n\n");
        markdown.push_str(&format!("- Verification Plan ID: {}\n", plan.id));
        markdown.push_str(&format!("- Scope: {}\n", plan.scope.as_str()));
        if !plan.commands.is_empty() {
            markdown.push_str("\n### 验证命令\n\n");
            for command in &plan.commands {
                markdown.push_str(&format!(
                    "- `{}`: {} (cwd: {}, required: {})\n",
                    command.label, command.command, command.cwd, command.required
                ));
            }
        }
        push_string_list(&mut markdown, "Required Gates", &plan.required_gates);
        push_string_list(&mut markdown, "Risk Notes", &plan.risk_notes);
    }

    push_markdown_section(&mut markdown, "Source Draft Supplement", draft_supplement);
    markdown
}

fn verification_command_lines(plan: &VerificationPlan) -> Vec<String> {
    plan.commands
        .iter()
        .map(|command| command.command.trim())
        .filter(|command| !command.is_empty())
        .map(str::to_string)
        .collect()
}

fn merge_work_item_markdown(
    compiled_markdown: Option<String>,
    workspace_markdown: Option<String>,
) -> Option<String> {
    match (compiled_markdown, workspace_markdown) {
        (Some(compiled), Some(workspace))
            if !workspace.trim().is_empty() && workspace.trim() != compiled.trim() =>
        {
            Some(format!(
                "{}\n\n---\n\n## Workspace Artifact Snapshot\n\n{}",
                compiled.trim(),
                workspace.trim()
            ))
        }
        (Some(compiled), _) => Some(compiled),
        (None, Some(workspace)) if !workspace.trim().is_empty() => Some(workspace),
        (None, _) => None,
    }
}

fn merge_verification_commands(
    compiled_commands: Vec<String>,
    markdown: Option<&str>,
) -> Vec<String> {
    let mut commands = Vec::new();
    for command in compiled_commands {
        push_unique_command(&mut commands, command);
    }
    if commands.is_empty()
        && let Some(markdown) = markdown
    {
        for spec in planned_test_commands_from_markdown(markdown) {
            push_unique_command(&mut commands, spec.command.join(" "));
        }
    }
    commands
}

fn push_unique_command(commands: &mut Vec<String>, command: String) {
    let command = command.trim();
    if !command.is_empty() && !commands.iter().any(|existing| existing == command) {
        commands.push(command.to_string());
    }
}

fn push_optional_line(markdown: &mut String, label: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        markdown.push_str(&format!("- {label}: {}\n", value.trim()));
    }
}

fn push_markdown_section(markdown: &mut String, heading: &str, value: Option<&str>) {
    if let Some(value) = value.filter(|value| !value.trim().is_empty()) {
        markdown.push_str(&format!("\n## {heading}\n\n{}\n", value.trim()));
    }
}

fn push_string_list(markdown: &mut String, heading: &str, values: &[String]) {
    if values.is_empty() {
        return;
    }
    markdown.push_str(&format!("\n## {heading}\n\n"));
    for value in values {
        markdown.push_str("- ");
        markdown.push_str(value);
        markdown.push('\n');
    }
}

/// 写入范围规范段（legacy 路径：逐字取自 `LifecycleWorkItemRecord`）。
fn push_write_scope_section(markdown: &mut String, work_item: &LifecycleWorkItemRecord) {
    markdown.push_str(&write_scope_section_text(
        &work_item.exclusive_write_scopes,
        &work_item.forbidden_write_scopes,
    ));
}

/// 写入范围规范段的**唯一渲染器**（legacy 记录与 group 的 `write_policy` 共用）。
///
/// 段内措辞与机器判定严格对齐：
/// - 基准 = 工作树根相对路径（`git status --porcelain` 的路径域）；
/// - 匹配 = 前缀匹配（`worktree.rs` 的 scope 匹配：目录项与含 `*` 的项按第一个
///   `*` 之前的前缀 `starts_with`，其余按整路径相等），不是 shell glob；
/// - 空表不对称：exclusive 为空 = 不限制，forbidden 为空 = 无额外禁止；
/// - 范围之外默认拒绝（含 `.gitignore` 等全局文件）。
///
/// 列表项逐字输出：不排序、不改写、不加仓名前缀。
///
/// 调用方：legacy markdown 渲染（`push_write_scope_section`）、group 兜底
/// 上下文（`bound_coder_work_item_context`）、group 主路径 prompt 追加
/// （`coding_workspace_engine`）——三处共用本函数，避免文案漂移。
pub(crate) fn write_scope_section_text(exclusive: &[String], forbidden: &[String]) -> String {
    let mut section = String::new();
    section.push_str(&format!("\n## {WRITE_SCOPE_SECTION_HEADING}\n\n"));
    section.push_str("- 判定基准：工作树根相对路径，即 `git status --porcelain` 输出的路径；下面的列表逐字取自已确认 Work Item 记录，平台不做仓名前缀、`./` 或聚合根变换。\n");
    section.push_str("- 匹配语义：前缀匹配，不是 shell glob 展开。以 `/` 结尾的目录项与含 `*` 的项，取第一个 `*` 之前的前缀做 starts_with；不含 `*` 且不以 `/` 结尾的项按整路径精确相等；单项 `*` 等于不限。含 `*` 的项不做 glob 展开——首个 `*` 之前为空（如 `**/x`）时前缀为空，机器判定实际等于不限制，不要以为仍受约束。\n");
    section.push_str("- 多仓 attempt：scope 也可能以聚合根为基准书写（如 `busi/src/a.ts`），平台按成员前缀兼容「仓内相对」与「带前缀」两种写法。\n");
    section.push_str("- 空列表语义：exclusive 为空 = 不限制可写范围；forbidden 为空 = 无额外禁止。命中任一 forbidden 项即违规，即使同时命中 exclusive。\n");
    section.push_str("- 默认拒绝：以上范围之外的仓库文件一律不得修改（含 `.gitignore`、构建配置等全局文件）；若确有必要，请不要自行改动，在交付说明里标注，由平台裁决。\n");
    push_write_scope_list(
        &mut section,
        WRITE_SCOPE_EXCLUSIVE_LIST_HEADING,
        exclusive,
        "（未声明：exclusive 为空 = 不限制可写范围）",
    );
    push_write_scope_list(
        &mut section,
        WRITE_SCOPE_FORBIDDEN_LIST_HEADING,
        forbidden,
        "（未声明：forbidden 为空 = 无额外禁止）",
    );
    section
}

fn push_write_scope_list(markdown: &mut String, label: &str, scopes: &[String], undeclared: &str) {
    markdown.push_str(&format!("\n### {label}\n\n"));
    if scopes.is_empty() {
        markdown.push_str("- ");
        markdown.push_str(undeclared);
        markdown.push('\n');
        return;
    }
    for scope in scopes {
        markdown.push_str("- ");
        markdown.push_str(scope);
        markdown.push('\n');
    }
}

fn latest_assistant_artifact_markdown(session: &WorkspaceSessionRecord) -> Option<String> {
    session
        .messages
        .iter()
        .rev()
        .find(|message| matches!(message.role.as_str(), "assistant" | "provider"))
        .and_then(|message| {
            // 与 workspace engine 同一选择语义：只取唯一 gate-passing 候选。
            selected_workspace_artifact_markdown(&message.content, &session.workspace_type)
        })
        .filter(|content| !content.trim().is_empty())
}

pub(crate) fn select_work_item_markdown(
    version_markdown: Option<String>,
    session: &WorkspaceSessionRecord,
) -> Option<String> {
    match version_markdown {
        Some(markdown) if !planned_test_commands_from_markdown(&markdown).is_empty() => {
            Some(markdown)
        }
        Some(markdown) => latest_assistant_artifact_markdown(session).or(Some(markdown)),
        None => latest_assistant_artifact_markdown(session),
    }
}

#[cfg(test)]
mod write_scope_tests {
    use super::*;
    use crate::product::lifecycle_store::CreateWorkItemInput;
    use crate::product::models::{WorkItemKind, WorkItemPlanStatus};

    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_0001";
    const WORK_ITEM_ID: &str = "work_item_0001";

    fn render_compiled_markdown(exclusive: &[&str], forbidden: &[&str]) -> String {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let lifecycle = LifecycleStore::new(paths);
        lifecycle
            .create_work_item(CreateWorkItemInput {
                id: Some(WORK_ITEM_ID.to_string()),
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: "repository_0001".to_string(),
                title: "Write scope renderer".to_string(),
                planned_implementation_context: Some("render write scopes".to_string()),
                kind: WorkItemKind::Backend,
                exclusive_write_scopes: exclusive.iter().map(|scope| scope.to_string()).collect(),
                forbidden_write_scopes: forbidden.iter().map(|scope| scope.to_string()).collect(),
                plan_status: WorkItemPlanStatus::Confirmed,
                ..Default::default()
            })
            .expect("create work item");
        let work_item = lifecycle
            .list_work_items(PROJECT_ID, ISSUE_ID)
            .expect("list work items")
            .into_iter()
            .find(|item| item.id == WORK_ITEM_ID)
            .expect("work item record");
        compiled_work_item_markdown(&work_item, None, None)
    }

    #[test]
    fn compiled_work_item_markdown_renders_write_scope_section_verbatim() {
        let markdown = render_compiled_markdown(
            &["src/product/coding_work_item_context.rs", "web/src/"],
            &["tests/**"],
        );

        assert!(markdown.contains("## 写入范围（硬边界）"));
        assert_eq!(markdown.matches("## 写入范围（硬边界）").count(), 1);
        assert!(markdown.contains(
            "- 判定基准：工作树根相对路径，即 `git status --porcelain` 输出的路径；下面的列表逐字取自已确认 Work Item 记录，平台不做仓名前缀、`./` 或聚合根变换。"
        ));
        assert!(markdown.contains("- 匹配语义：前缀匹配，不是 shell glob 展开。"));
        assert!(markdown.contains("- 多仓 attempt：scope 也可能以聚合根为基准书写"));
        assert!(markdown.contains(
            "- 空列表语义：exclusive 为空 = 不限制可写范围；forbidden 为空 = 无额外禁止。命中任一 forbidden 项即违规，即使同时命中 exclusive。"
        ));
        assert!(markdown.contains(
            "- 默认拒绝：以上范围之外的仓库文件一律不得修改（含 `.gitignore`、构建配置等全局文件）；若确有必要，请不要自行改动，在交付说明里标注，由平台裁决。"
        ));
        assert!(markdown.contains(
            "### 允许修改范围（exclusive_write_scopes）\n\n- src/product/coding_work_item_context.rs\n- web/src/\n"
        ));
        assert!(markdown.contains("### 禁止修改范围（forbidden_write_scopes）\n\n- tests/**\n"));
        assert!(!markdown.contains("- （未声明"));
    }

    #[test]
    fn compiled_work_item_markdown_marks_empty_write_scopes_as_non_restricting() {
        let markdown = render_compiled_markdown(&[], &[]);

        assert!(markdown.contains("## 写入范围（硬边界）"));
        assert!(markdown.contains(
            "### 允许修改范围（exclusive_write_scopes）\n\n- （未声明：exclusive 为空 = 不限制可写范围）\n"
        ));
        assert!(markdown.contains(
            "### 禁止修改范围（forbidden_write_scopes）\n\n- （未声明：forbidden 为空 = 无额外禁止）\n"
        ));
        // 空列表仍是「范围已知」，markdown 里不得出现未知范围的措辞。
        assert!(!markdown.contains("范围未知"));
    }

    #[test]
    fn compiled_work_item_markdown_keeps_member_prefixed_scopes_verbatim() {
        let markdown =
            render_compiled_markdown(&["busi/src/a.ts", "busi/src/b.ts"], &["web/package.json"]);

        assert!(markdown.contains("- busi/src/a.ts\n- busi/src/b.ts\n"));
        assert!(markdown.contains("- web/package.json\n"));
        // 平台不做仓名前缀 / 聚合根变换：以聚合根为基准的写法原样保留。
        assert!(!markdown.contains("- src/a.ts\n"));
    }
}
