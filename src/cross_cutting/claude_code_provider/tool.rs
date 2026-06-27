use super::*;

pub(crate) fn tool_use_description(tool_use: &ToolUseBlock) -> String {
    match tool_use.name.as_str() {
        "Bash" => tool_use
            .input
            .get("command")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        "Read" => tool_use
            .input
            .get("file_path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        "Edit" | "Write" => tool_use
            .input
            .get("file_path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        _ => tool_use
            .input
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    }
}

pub(crate) fn tool_use_command(tool_use: &ToolUseBlock) -> Option<String> {
    match tool_use.name.as_str() {
        "Bash" => tool_use
            .input
            .get("command")
            .and_then(Value::as_str)
            .map(String::from),
        "Read" => Some(format!(
            "read {}",
            tool_use
                .input
                .get("file_path")
                .and_then(Value::as_str)
                .unwrap_or("?")
        )),
        "Edit" => Some(format!(
            "edit {}",
            tool_use
                .input
                .get("file_path")
                .and_then(Value::as_str)
                .unwrap_or("?")
        )),
        "Write" => Some(format!(
            "write {}",
            tool_use
                .input
                .get("file_path")
                .and_then(Value::as_str)
                .unwrap_or("?")
        )),
        _ => None,
    }
}

pub(crate) fn output_preview(output: &str, max_bytes: usize) -> String {
    if output.len() <= max_bytes {
        return output.to_string();
    }

    let truncate_at = output
        .char_indices()
        .map(|(idx, _)| idx)
        .take_while(|idx| *idx <= max_bytes)
        .last()
        .unwrap_or(0);
    format!("{}...", &output[..truncate_at])
}

pub(crate) fn combine_stderr(process_stderr: String, write_stderr: String) -> String {
    match (process_stderr.trim(), write_stderr.trim()) {
        ("", "") => String::new(),
        (process, "") => process.to_string(),
        ("", write_error) => write_error.to_string(),
        (process, write_error) => format!("{process}\n{write_error}"),
    }
}

pub(crate) fn format_exit_failure(
    status: Result<ExitStatus, std::io::Error>,
    stderr: String,
) -> String {
    let status_text = match status {
        Ok(status) => format!("exit status: {status}"),
        Err(error) => format!("failed to wait for process: {error}"),
    };
    if stderr.trim().is_empty() {
        format!("Claude Code provider exited without result ({status_text})")
    } else {
        format!(
            "Claude Code provider exited without result ({status_text}); stderr: {}",
            stderr.trim()
        )
    }
}

pub(crate) async fn write_json_line(
    stdin: &Arc<Mutex<ChildStdin>>,
    value: &Value,
) -> Result<(), ProviderAdapterError> {
    let mut stdin = stdin.lock().await;
    let line = serde_json::to_string(value).map_err(|error| {
        ProviderAdapterError::parse_error(
            format!("invalid Claude control JSON: {error}"),
            String::new(),
            String::new(),
        )
    })?;
    stdin.write_all(line.as_bytes()).await.map_err(|error| {
        ProviderAdapterError::execution_failed(None, String::new(), error.to_string(), 0)
    })?;
    stdin.write_all(b"\n").await.map_err(|error| {
        ProviderAdapterError::execution_failed(None, String::new(), error.to_string(), 0)
    })?;
    stdin.flush().await.map_err(|error| {
        ProviderAdapterError::execution_failed(None, String::new(), error.to_string(), 0)
    })
}
