use crate::product::app_paths::ProductAppPaths;
use crate::product::artifact_extraction::extract_artifact_content;
use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    LifecycleWorkItemRecord, VerificationPlan, WorkspaceSessionRecord, WorkspaceSessionStatus,
    WorkspaceType,
};
use crate::product::test_executor::planned_test_commands_from_markdown;
use crate::product::work_item_plan_store::WorkItemPlanStore;

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
        Err(ProductStoreError::NotFound { kind, .. }) if kind == "work_item_plan_lineage" => {
            Ok(false)
        }
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

    Ok(CompiledCodingWorkItemContext {
        markdown: Some(format!(
            "# Bound Coder Context\n\n- Plan revision: {}\n- Work item revision: {}\n- Coder projection hash: {}\n\n## Coder Projection\n\n{}\n",
            runtime.binding.plan_revision_id,
            runtime.binding.work_item_revision_id,
            coder_projection_hash,
            projection_json,
        )),
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
        || work_item
            .planned_handoff_summary
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
    push_markdown_section(
        &mut markdown,
        "Planned Handoff Summary",
        work_item.planned_handoff_summary.as_deref(),
    );
    push_string_list(&mut markdown, "Story Spec IDs", &work_item.story_spec_ids);
    push_string_list(&mut markdown, "Design Spec IDs", &work_item.design_spec_ids);
    push_string_list(&mut markdown, "Depends On", &work_item.depends_on);
    push_string_list(
        &mut markdown,
        "Required Handoff From",
        &work_item.required_handoff_from,
    );
    push_string_list(
        &mut markdown,
        "Exclusive Write Scopes",
        &work_item.exclusive_write_scopes,
    );
    push_string_list(
        &mut markdown,
        "Forbidden Write Scopes",
        &work_item.forbidden_write_scopes,
    );

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

fn latest_assistant_artifact_markdown(session: &WorkspaceSessionRecord) -> Option<String> {
    session
        .messages
        .iter()
        .rev()
        .find(|message| matches!(message.role.as_str(), "assistant" | "provider"))
        .map(|message| extract_artifact_content(&message.content))
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
