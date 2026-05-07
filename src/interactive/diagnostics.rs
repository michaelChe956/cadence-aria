use std::path::Path;

use serde_json::{Value, json};

use crate::task_run::types::TaskRunError;

pub fn classify_task_diagnostics(
    task_root: &Path,
    state: &Value,
) -> Result<Vec<Value>, TaskRunError> {
    let blocked = read_json_optional(&task_root.join("reports/blocked-report.json"))?;
    let testing = read_json_optional(&task_root.join("reports/testing-report.json"))?;
    let mut diagnostics = Vec::new();

    if let Some(blocked) = blocked {
        let reason = blocked
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("blocked_by_gate");
        let next_node = blocked.get("next_node").and_then(Value::as_str);
        let testing_text = testing
            .as_ref()
            .map(|value| value.to_string())
            .unwrap_or_default();
        let category = if testing_text.contains("allowed_write_scope=[]")
            || testing_text.contains("cadence/designs")
            || testing_text.contains("cadence/reports")
        {
            "contract_write_scope_blocked"
        } else {
            "gate_blocked"
        };
        diagnostics.push(json!({
            "category": category,
            "severity": "blocking",
            "status": "blocked_by_gate",
            "reason": reason,
            "next_node": next_node,
            "task_id": state.get("task_id").cloned().unwrap_or(Value::Null),
            "current_worktask": state.get("current_worktask").cloned().unwrap_or(Value::Null),
        }));
    }

    Ok(diagnostics)
}

fn read_json_optional(path: &Path) -> Result<Option<Value>, TaskRunError> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(path).map_err(|error| {
        TaskRunError::new(
            "interactive_diagnostics_io",
            format!("read {}: {error}", path.display()),
        )
    })?;
    serde_json::from_slice(&bytes).map(Some).map_err(|error| {
        TaskRunError::new(
            "interactive_diagnostics_json",
            format!("parse {}: {error}", path.display()),
        )
    })
}
