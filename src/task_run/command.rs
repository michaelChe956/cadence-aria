use std::path::PathBuf;

use crate::cross_cutting::provider_adapter::DEFAULT_PROVIDER_TIMEOUT_SECS;
use crate::task_run::types::{ProviderMode, ReportMode, TaskRunError, TaskRunOptions};

pub fn parse_task_run_args(args: &[String]) -> Result<TaskRunOptions, TaskRunError> {
    let mut workspace = None;
    let mut request_text = None;
    let mut change_id = None;
    let mut provider_mode = ProviderMode::Real;
    let mut non_interactive = false;
    let mut timeout_secs = DEFAULT_PROVIDER_TIMEOUT_SECS;
    let mut report_mode = ReportMode::Text;

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--workspace" => {
                let value = required_value(args, index, "--workspace")?;
                workspace = Some(PathBuf::from(value));
                index += 2;
            }
            "--request" => {
                request_text = Some(required_value(args, index, "--request")?);
                index += 2;
            }
            "--change-id" => {
                change_id = Some(required_value(args, index, "--change-id")?);
                index += 2;
            }
            "--providers" => {
                let value = required_value(args, index, "--providers")?;
                provider_mode = match value.as_str() {
                    "real" => ProviderMode::Real,
                    other => {
                        return Err(TaskRunError::new(
                            "invalid_cli_args",
                            format!("unsupported --providers value: {other}"),
                        ));
                    }
                };
                index += 2;
            }
            "--timeout" => {
                let value = required_value(args, index, "--timeout")?;
                timeout_secs = value.parse::<u64>().map_err(|error| {
                    TaskRunError::new("invalid_cli_args", format!("invalid --timeout: {error}"))
                })?;
                index += 2;
            }
            "--report" => {
                let value = required_value(args, index, "--report")?;
                report_mode = match value.as_str() {
                    "text" => ReportMode::Text,
                    "json" => ReportMode::Json,
                    other => {
                        return Err(TaskRunError::new(
                            "invalid_cli_args",
                            format!("unsupported --report value: {other}"),
                        ));
                    }
                };
                index += 2;
            }
            "--non-interactive" => {
                non_interactive = true;
                index += 1;
            }
            other => {
                return Err(TaskRunError::new(
                    "invalid_cli_args",
                    format!("unknown task run argument: {other}"),
                ));
            }
        }
    }

    let workspace = workspace
        .ok_or_else(|| TaskRunError::new("invalid_cli_args", "task run requires --workspace"))?;
    let request_text = request_text
        .ok_or_else(|| TaskRunError::new("invalid_cli_args", "task run requires --request"))?;
    if request_text.trim().is_empty() {
        return Err(TaskRunError::new(
            "invalid_cli_args",
            "--request must not be empty",
        ));
    }

    Ok(TaskRunOptions {
        workspace,
        request_text,
        change_id,
        provider_mode,
        non_interactive,
        timeout_secs,
        report_mode,
    })
}

fn required_value(args: &[String], index: usize, flag: &str) -> Result<String, TaskRunError> {
    args.get(index + 1)
        .cloned()
        .ok_or_else(|| TaskRunError::new("invalid_cli_args", format!("{flag} requires a value")))
}
