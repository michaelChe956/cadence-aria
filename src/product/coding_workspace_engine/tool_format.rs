use super::*;

pub(crate) fn format_tool_call_input(input: &serde_json::Value) -> String {
    serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string())
}

pub(crate) async fn forward_runner_command_to_provider(
    command: CodingRunnerCommand,
    provider_commands: &mpsc::Sender<ProviderCommand>,
) -> bool {
    match command {
        CodingRunnerCommand::PermissionResponse {
            id,
            approved,
            reason,
        } => provider_commands
            .send(ProviderCommand::PermissionResponse {
                id,
                approved,
                reason,
            })
            .await
            .is_ok(),
        CodingRunnerCommand::ChoiceResponse {
            id,
            selected_option_ids,
            free_text,
        } => provider_commands
            .send(ProviderCommand::ChoiceResponse {
                id,
                selected_option_ids,
                free_text,
            })
            .await
            .is_ok(),
        CodingRunnerCommand::AbortAttempt => {
            provider_commands.send(ProviderCommand::Abort).await.is_ok()
        }
        CodingRunnerCommand::ProviderSelect { .. }
        | CodingRunnerCommand::StageGateConfirm { .. } => true,
    }
}

pub(crate) fn extract_tool_command(input: &serde_json::Value) -> Option<String> {
    let command = input.get("command").or_else(|| input.get("cmd"))?;
    if let Some(command) = command.as_str() {
        return Some(command.to_string());
    }
    command.as_array().and_then(|parts| {
        parts
            .iter()
            .map(serde_json::Value::as_str)
            .collect::<Option<Vec<_>>>()
            .map(|parts| parts.join(" "))
            .filter(|command| !command.trim().is_empty())
    })
}

pub(crate) fn worktree_path_for_attempt(
    repo_path: &Path,
    attempt: &CodingExecutionAttempt,
) -> PathBuf {
    if let Some(issue_id) = attempt.branch_name.strip_prefix("aria/issues/") {
        return repo_path
            .join(".worktrees")
            .join("aria-issues")
            .join(issue_id);
    }
    repo_path
        .join(".worktrees")
        .join("aria-work-items")
        .join(&attempt.work_item_id)
        .join(format!("attempt-{}", attempt.attempt_no))
}

pub(crate) fn provider_type_for_name(provider: &ProviderName) -> ProviderType {
    match provider {
        ProviderName::ClaudeCode => ProviderType::ClaudeCode,
        ProviderName::Codex => ProviderType::Codex,
        ProviderName::Fake => ProviderType::Fake,
    }
}
