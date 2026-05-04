use chrono::{SecondsFormat, Utc};
use serde_json::{Value, json};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

pub fn append_node_event(
    task_root: &Path,
    task_id: &str,
    node_id: &str,
    event_kind: &str,
    status: &str,
    details: Value,
) {
    let event = json!({
        "timestamp": Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true),
        "event_kind": event_kind,
        "task_id": task_id,
        "node_id": node_id,
        "status": status,
        "details": details,
    });
    let _ = append_jsonl(&task_root.join("logs/node-events.jsonl"), &event);
}

fn append_jsonl(path: &Path, value: &Value) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    Ok(())
}
