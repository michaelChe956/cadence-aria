use super::*;

pub(crate) fn format_tool_call_input(input: &serde_json::Value) -> String {
    serde_json::to_string_pretty(input).unwrap_or_else(|_| input.to_string())
}

pub(crate) async fn forward_runner_command_to_provider(
    command: CodingRunnerCommand,
    provider_commands: &mpsc::Sender<ProviderCommand>,
    cancellation: &CancellationToken,
) -> bool {
    match command {
        CodingRunnerCommand::PermissionResponse {
            id,
            approved,
            reason,
        } => {
            send_provider_command_with_cancellation(
                provider_commands,
                ProviderCommand::PermissionResponse {
                    id,
                    approved,
                    reason,
                },
                cancellation,
            )
            .await
        }
        CodingRunnerCommand::ChoiceResponse {
            id,
            selected_option_ids,
            free_text,
        } => {
            send_provider_command_with_cancellation(
                provider_commands,
                ProviderCommand::ChoiceResponse {
                    id,
                    selected_option_ids,
                    free_text,
                    answers: vec![],
                },
                cancellation,
            )
            .await
        }
        CodingRunnerCommand::AbortAttempt => {
            provider_commands.try_send(ProviderCommand::Abort).is_ok()
        }
        CodingRunnerCommand::ProviderSelect { .. }
        | CodingRunnerCommand::StageGateConfirm { .. }
        | CodingRunnerCommand::RetryPush => true,
    }
}

pub(crate) async fn send_provider_command_with_cancellation(
    provider_commands: &mpsc::Sender<ProviderCommand>,
    command: ProviderCommand,
    cancellation: &CancellationToken,
) -> bool {
    let permit = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return false,
        permit = provider_commands.reserve() => permit,
    };
    let Ok(permit) = permit else {
        return false;
    };
    if cancellation.is_cancelled() {
        return false;
    }
    permit.send(command);
    true
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
        ProviderName::Pi => ProviderType::Pi,
        ProviderName::KimiCode => ProviderType::KimiCode,
        ProviderName::Fake => ProviderType::Fake,
    }
}

#[cfg(test)]
mod tests {
    use super::provider_type_for_name;
    use crate::product::models::ProviderName;
    use crate::protocol::contracts::ProviderType;

    #[test]
    fn provider_type_for_name_maps_pi() {
        assert_eq!(provider_type_for_name(&ProviderName::Pi), ProviderType::Pi);
    }

    #[test]
    fn provider_type_for_name_maps_kimi_code() {
        assert_eq!(
            provider_type_for_name(&ProviderName::KimiCode),
            ProviderType::KimiCode
        );
    }
}
