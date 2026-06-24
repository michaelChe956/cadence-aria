use chrono::Utc;
use serde_json::Value;

use crate::product::coding_models::{TestPlan, TestPlanStep};

use super::types::{ProviderTestPlanPayload, TesterAgentError};

pub fn parse_test_plan_payload(
    attempt_id: &str,
    plan_id: &str,
    raw_output: &str,
    raw_provider_output_ref: Option<String>,
) -> Result<TestPlan, TesterAgentError> {
    let json_text = extract_json_payload(raw_output)
        .ok_or_else(|| TesterAgentError::Plan("missing_json_object".to_string()))?;
    let payload: ProviderTestPlanPayload = serde_json::from_str(&json_text)
        .map_err(|error| TesterAgentError::Plan(format!("invalid_json: {error}")))?;
    validate_test_plan_payload(&payload)?;
    Ok(TestPlan {
        id: plan_id.to_string(),
        attempt_id: attempt_id.to_string(),
        role_run_id: None,
        run_no: None,
        summary: payload.summary,
        context_warnings: payload.context_warnings,
        assumptions: payload.assumptions,
        steps: payload.steps,
        created_at: Utc::now().to_rfc3339(),
        raw_provider_output_ref,
    })
}

fn extract_json_payload(raw_output: &str) -> Option<String> {
    let trimmed = raw_output.trim();
    if trimmed.starts_with('{') && trimmed.ends_with('}') {
        return Some(trimmed.to_string());
    }

    let mut in_json_fence = false;
    let mut fenced_lines = Vec::new();
    for line in raw_output.lines() {
        let trimmed_line = line.trim();
        if trimmed_line.starts_with("```") {
            if in_json_fence {
                return Some(fenced_lines.join("\n"));
            }
            let fence_label = trimmed_line.trim_start_matches('`').trim();
            if fence_label.is_empty() || fence_label.eq_ignore_ascii_case("json") {
                in_json_fence = true;
                fenced_lines.clear();
            }
            continue;
        }
        if in_json_fence {
            fenced_lines.push(line);
        }
    }

    let start = raw_output.find('{')?;
    let end = raw_output.rfind('}')?;
    if end <= start {
        return None;
    }
    Some(raw_output[start..=end].to_string())
}

fn validate_test_plan_payload(payload: &ProviderTestPlanPayload) -> Result<(), TesterAgentError> {
    require_non_empty("summary", &payload.summary)?;
    if payload.steps.is_empty() {
        return Err(TesterAgentError::Plan("steps_empty".to_string()));
    }
    let mut seen_step_ids = std::collections::HashSet::new();
    for step in &payload.steps {
        require_non_empty("step.id", &step.id)?;
        if !seen_step_ids.insert(step.id.clone()) {
            return Err(TesterAgentError::Plan(format!(
                "duplicate_step_id: {}",
                step.id
            )));
        }
        require_non_empty("step.title", &step.title)?;
        require_non_empty("step.intent", &step.intent)?;
        require_non_empty("step.evidence_expectation", &step.evidence_expectation)?;
        validate_step_traceability(step)?;
        validate_step_command(step)?;
    }
    Ok(())
}

fn validate_step_traceability(step: &TestPlanStep) -> Result<(), TesterAgentError> {
    if !step.required || is_context_gathering_step(step) {
        return Ok(());
    }
    let has_trace = !step.related_requirements.is_empty()
        || !step.related_design_constraints.is_empty()
        || !step.related_work_item_tasks.is_empty();
    if has_trace {
        return Ok(());
    }
    Err(TesterAgentError::Plan(format!(
        "step_traceability_empty: {}",
        step.id
    )))
}

fn is_context_gathering_step(step: &TestPlanStep) -> bool {
    let text = format!(
        "{} {} {} {}",
        step.id, step.title, step.intent, step.evidence_expectation
    )
    .to_ascii_lowercase();
    matches!(
        step.tool,
        crate::product::coding_models::TestPlanTool::ReadFile
    ) || text.contains("rules")
        || text.contains("规则")
        || text.contains("diff")
        || text.contains("status")
        || text.contains("上下文")
        || text.contains("context")
        || text.contains("search")
        || text.contains("锚点")
}

fn validate_step_command(step: &TestPlanStep) -> Result<(), TesterAgentError> {
    if let Some(parts) = command_parts_from_value(&step.command_or_tool_input)
        && is_cargo_lib_command_with_multiple_filters(&parts)
    {
        return Err(TesterAgentError::Plan(format!(
            "cargo_lib_multiple_filters: {}",
            step.id
        )));
    }
    Ok(())
}

fn command_parts_from_value(input: &Value) -> Option<Vec<String>> {
    let command = input.get("command")?;
    match command {
        Value::String(value) => Some(split_shell_words(value)),
        Value::Array(values) => Some(
            values
                .iter()
                .filter_map(|value| value.as_str().map(ToString::to_string))
                .collect(),
        ),
        _ => None,
    }
}

fn split_shell_words(value: &str) -> Vec<String> {
    value.split_whitespace().map(ToString::to_string).collect()
}

fn is_cargo_lib_command_with_multiple_filters(parts: &[String]) -> bool {
    if parts.len() < 5 {
        return false;
    }
    if parts.first().map(String::as_str) != Some("cargo") {
        return false;
    }
    if parts.get(1).map(String::as_str) != Some("test") {
        return false;
    }
    let Some(lib_index) = parts.iter().position(|part| part == "--lib") else {
        return false;
    };
    let filters = parts[lib_index + 1..]
        .iter()
        .filter(|part| !part.starts_with('-'))
        .count();
    filters > 1
}

fn require_non_empty(field: &str, value: &str) -> Result<(), TesterAgentError> {
    if value.trim().is_empty() {
        return Err(TesterAgentError::Plan(format!("{field}_empty")));
    }
    Ok(())
}
