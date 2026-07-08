use std::path::PathBuf;

use crate::product::app_paths::ProductAppPaths;
use crate::product::artifact_extraction::extract_artifact_content;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingAgentRole, CodingChatEntry, CodingContextNote, CodingEntryType, CodingExecutionAttempt,
    CodingExecutionStage, CodingProviderPermissionMode, CodingProviderRole, CodingStageGateState,
    CodingStageGateStatus,
};
use crate::product::coding_workspace_engine::CodingExecutionContext;
use crate::product::coding_workspace_engine::CodingWorkspaceEngineError;
use crate::product::coding_workspace_runner::{
    apply_provider_selection_to_snapshots, coding_provider_role_for_stage,
    parse_coding_provider_role,
};
use crate::product::json_store::ProductStoreError;

use super::active_coding_timeline_node_id;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::{
    LifecycleWorkItemRecord, ProviderName, VerificationPlan, WorkItemExecutionPlanStatus,
    WorkspaceSessionRecord, WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::repository_store::RepositoryStore;
use crate::product::test_executor::planned_test_commands_from_markdown;
use crate::product::work_item_plan_store::WorkItemPlanStore;

pub(crate) fn current_work_item_id_for_attempt(attempt: &CodingExecutionAttempt) -> &str {
    attempt
        .current_work_item_id
        .as_deref()
        .unwrap_or(&attempt.work_item_id)
}

#[derive(Debug, Clone, Default)]
struct CompiledWorkItemContext {
    markdown: Option<String>,
    verification_commands: Vec<String>,
    needs_workspace_artifact_fallback: bool,
}

pub(crate) fn coding_execution_context(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<CodingExecutionContext, ProductStoreError> {
    let current_work_item_id = current_work_item_id_for_attempt(attempt);
    let lifecycle = LifecycleStore::new(app_paths.clone());

    let compiled_context =
        compiled_work_item_context(&lifecycle, app_paths, attempt, current_work_item_id)?;
    let workspace_markdown = if compiled_context.markdown.is_none()
        || compiled_context.needs_workspace_artifact_fallback
    {
        workspace_artifact_work_item_markdown(&lifecycle, attempt, current_work_item_id)?
    } else {
        None
    };
    let work_item_markdown =
        merge_work_item_markdown(compiled_context.markdown, workspace_markdown);
    let verification_commands = merge_verification_commands(
        compiled_context.verification_commands,
        work_item_markdown.as_deref(),
    );

    Ok(CodingExecutionContext {
        work_item_markdown,
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
    push_markdown_section(
        &mut markdown,
        "Draft Implementation Context",
        Some(&draft.candidate.implementation_context),
    );
    push_markdown_section(
        &mut markdown,
        "Draft Handoff Summary",
        Some(&draft.candidate.handoff_summary),
    );
    push_string_list(
        &mut markdown,
        "Draft Exclusive Write Scopes",
        &draft.candidate.exclusive_write_scopes,
    );
    push_string_list(
        &mut markdown,
        "Draft Forbidden Write Scopes",
        &draft.candidate.forbidden_write_scopes,
    );
    push_string_list(
        &mut markdown,
        "Draft Depends On Outline IDs",
        &draft.candidate.depends_on_outline_ids,
    );
    push_string_list(
        &mut markdown,
        "Draft Required Handoff From Outline IDs",
        &draft.candidate.required_handoff_from_outline_ids,
    );
    if !draft.candidate.verification_plan.is_null() {
        push_markdown_section(
            &mut markdown,
            "Draft Verification Plan JSON",
            Some(&draft.candidate.verification_plan.to_string()),
        );
    }

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

pub(crate) fn ensure_work_item_execution_plan_confirmed(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<(), CodingWorkspaceEngineError> {
    let current_work_item_id = current_work_item_id_for_attempt(attempt);
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let work_items = lifecycle.list_work_items(&attempt.project_id, &attempt.issue_id)?;
    let Some(work_item) = work_items
        .iter()
        .find(|item| item.id == current_work_item_id)
    else {
        return Ok(());
    };
    if !work_item.require_execution_plan_confirm {
        return Ok(());
    }

    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let plan = coding_store.get_work_item_execution_plan(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    match plan.map(|p| p.status) {
        Some(WorkItemExecutionPlanStatus::Confirmed) => Ok(()),
        _ => Err(CodingWorkspaceEngineError::ExecutionPlanNotConfirmed(
            attempt.id.clone(),
        )),
    }
}

pub(crate) fn repository_path_for_attempt(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<PathBuf, CodingWorkspaceEngineError> {
    let current_work_item_id = current_work_item_id_for_attempt(attempt);
    let work_item = LifecycleStore::new(app_paths.clone())
        .list_work_items(&attempt.project_id, &attempt.issue_id)?
        .into_iter()
        .find(|work_item| work_item.id == current_work_item_id)
        .ok_or_else(|| ProductStoreError::NotFound {
            kind: "work_item",
            id: current_work_item_id.to_string(),
        })?;
    RepositoryStore::new(app_paths.clone())
        .list(&attempt.project_id)?
        .into_iter()
        .find(|repository| repository.id == work_item.repository_id)
        .map(|repository| repository.path)
        .ok_or({
            CodingWorkspaceEngineError::Store(ProductStoreError::NotFound {
                kind: "repository",
                id: work_item.repository_id,
            })
        })
}

pub(crate) fn update_provider_selection(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    role: &str,
    provider: ProviderName,
) -> Result<(CodingExecutionAttempt, CodingProviderRole, ProviderName), ProductStoreError> {
    let mut snapshot = attempt.provider_config_snapshot.clone();
    let mut role_snapshot = coding_store.get_role_provider_config_snapshot(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    let changed_provider = provider.clone();
    let changed_role =
        apply_provider_selection_to_snapshots(role, provider, &mut snapshot, &mut role_snapshot)
            .map_err(ProductStoreError::Io)?;
    let updated = coding_store.update_attempt_provider_config_snapshot(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        snapshot,
    )?;
    coding_store.update_role_provider_config_snapshot(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        role_snapshot,
    )?;
    Ok((updated, changed_role, changed_provider))
}

pub(crate) fn update_provider_permission_mode(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    role: &str,
    permission_mode: CodingProviderPermissionMode,
) -> Result<(CodingProviderRole, ProviderName), ProductStoreError> {
    let parsed_role = parse_coding_provider_role(role)
        .ok_or_else(|| ProductStoreError::Io(format!("unknown coding role: {role}")))?;
    let mut role_snapshot = coding_store.get_role_provider_config_snapshot(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )?;
    let provider = role_snapshot.provider_for_role(&parsed_role).clone();
    role_snapshot.set_permission_mode_for_role(&parsed_role, permission_mode);
    coding_store.update_role_provider_config_snapshot(
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
        role_snapshot,
    )?;
    Ok((parsed_role, provider))
}

pub(crate) fn provider_selection_targets_current_running_stage(
    attempt: &CodingExecutionAttempt,
    role: &str,
) -> bool {
    if attempt.status != crate::product::coding_models::CodingAttemptStatus::Running {
        return false;
    }
    let Some(current_role) = coding_provider_role_for_stage(&attempt.stage) else {
        return false;
    };
    parse_coding_provider_role(role).as_ref() == Some(&current_role)
}

pub(crate) fn confirm_open_stage_gate(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    stage: &CodingExecutionStage,
) -> Result<Option<CodingStageGateState>, ProductStoreError> {
    let Some(gate) = coding_store
        .list_open_stage_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        .into_iter()
        .find(|gate| gate.stage == *stage)
    else {
        return Ok(None);
    };
    coding_store
        .update_stage_gate_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate.gate_id,
            CodingStageGateStatus::Confirmed,
        )
        .map(Some)
}

pub(crate) fn context_note_chat_entry(
    coding_store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    note: CodingContextNote,
) -> Result<CodingChatEntry, ProductStoreError> {
    let timeline_nodes =
        coding_store.get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
    Ok(CodingChatEntry {
        id: chat_entry_id_for_context_note(&note.id),
        attempt_id: attempt.id.clone(),
        node_id: active_coding_timeline_node_id(&timeline_nodes),
        role: CodingAgentRole::Author,
        entry_type: CodingEntryType::UserMessage,
        content: Some(note.content),
        metadata: Some(serde_json::json!({
            "context_note_id": note.id,
        })),
        created_at: note.created_at,
    })
}

fn chat_entry_id_for_context_note(note_id: &str) -> String {
    note_id.replacen("coding_context_note", "coding_chat_entry", 1)
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
