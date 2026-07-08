use super::*;

pub(crate) fn parse_compile_verification_scope(value: Option<&str>) -> VerificationScope {
    match value.unwrap_or_default() {
        "unit" => VerificationScope::Unit,
        "integration" => VerificationScope::Integration,
        "e2e" => VerificationScope::E2e,
        "build" => VerificationScope::Build,
        "lint" => VerificationScope::Lint,
        "manual" => VerificationScope::Manual,
        _ => VerificationScope::Custom,
    }
}

pub(crate) fn parse_compile_confidence(value: Option<&str>) -> RepositoryProfileConfidence {
    match value.unwrap_or("high") {
        "low" => RepositoryProfileConfidence::Low,
        "medium" => RepositoryProfileConfidence::Medium,
        _ => RepositoryProfileConfidence::High,
    }
}

pub(crate) fn parse_compile_fallback_policy(value: Option<&str>) -> VerificationFallbackPolicy {
    match value.unwrap_or("manual_gate") {
        "repair_provider_output" => VerificationFallbackPolicy::RepairProviderOutput,
        _ => VerificationFallbackPolicy::ManualGate,
    }
}

pub(crate) fn parse_compile_safety(value: Option<&str>) -> VerificationCommandSafety {
    match value.unwrap_or("approved") {
        "needs_manual_review" => VerificationCommandSafety::NeedsManualReview,
        _ => VerificationCommandSafety::Approved,
    }
}

pub(crate) fn json_string_array(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn normalize_compile_verification_cwd(
    cwd: &str,
    repository_path: Option<&std::path::Path>,
) -> String {
    if cwd.is_empty() {
        return String::new();
    }
    let cwd_path = std::path::Path::new(cwd);
    if !cwd_path.is_absolute() {
        return cwd.to_string();
    }
    let Some(repository_path) = repository_path else {
        return cwd.to_string();
    };
    if !repository_path.is_absolute() {
        return cwd.to_string();
    }
    let Ok(relative_path) = cwd_path.strip_prefix(repository_path) else {
        return cwd.to_string();
    };
    if relative_path.as_os_str().is_empty() {
        String::new()
    } else {
        relative_path.to_string_lossy().to_string()
    }
}

pub(crate) fn parse_compile_verification_plan(
    value: &serde_json::Value,
    id: String,
    project_id: String,
    issue_id: String,
    work_item_id: String,
    now: String,
    repository_path: Option<&std::path::Path>,
) -> VerificationPlan {
    let commands = value
        .get("commands")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .enumerate()
                .map(|(index, command)| VerificationCommand {
                    id: command
                        .get("id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("cmd_{:03}", index + 1)),
                    label: command
                        .get("label")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("验证命令")
                        .to_string(),
                    command: command
                        .get("command")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    cwd: normalize_compile_verification_cwd(
                        command
                            .get("cwd")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default(),
                        repository_path,
                    ),
                    purpose: command
                        .get("purpose")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    required: command
                        .get("required")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                    timeout_seconds: command
                        .get("timeout_seconds")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(120),
                    source: VerificationCommandSource::Provider,
                    safety: parse_compile_safety(
                        command.get("safety").and_then(serde_json::Value::as_str),
                    ),
                })
                .collect()
        })
        .unwrap_or_default();
    let manual_checks = value
        .get("manual_checks")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .enumerate()
                .map(|(index, check)| VerificationManualCheck {
                    id: check
                        .get("id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("manual_{:03}", index + 1)),
                    label: check
                        .get("label")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("人工检查")
                        .to_string(),
                    instructions: check
                        .get("instructions")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    required: check
                        .get("required")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default();

    VerificationPlan {
        id,
        project_id,
        issue_id,
        work_item_id,
        repository_profile_ref: None,
        provider_run_ref: None,
        scope: parse_compile_verification_scope(
            value.get("scope").and_then(serde_json::Value::as_str),
        ),
        commands,
        manual_checks,
        required_gates: json_string_array(value.get("required_gates")),
        risk_notes: json_string_array(value.get("risk_notes")),
        confidence: parse_compile_confidence(
            value.get("confidence").and_then(serde_json::Value::as_str),
        ),
        fallback_policy: parse_compile_fallback_policy(
            value
                .get("fallback_policy")
                .and_then(serde_json::Value::as_str),
        ),
        created_at: now.clone(),
        updated_at: now,
    }
}
