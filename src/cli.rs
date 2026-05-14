use std::path::{Path, PathBuf};

use crate::daemon::discovery::{DaemonStatus, inspect_daemon};
use crate::daemon::runner::{run_daemon_serve_one, run_daemon_until_shutdown};
use crate::repl::discovery::{DiscoveryMode, resolve_daemon_connection};
use crate::task_run::command::parse_task_run_args;
use crate::task_run::orchestrator::TaskRunOrchestrator;
use crate::task_run::provider_factory::real_routing_provider;
use crate::task_run::types::{ReportMode, TaskRunRequest, TaskRunStatus};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliOutput {
    Text(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    pub code: String,
    pub message: String,
}

pub fn run_cli<I, S>(args: I) -> Result<CliOutput, CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args: Vec<String> = args.into_iter().map(Into::into).collect();
    match args.as_slice() {
        [command, subcommand, rest @ ..] if command == "daemon" && subcommand == "status" => {
            let workspace = parse_workspace(rest)?;
            let status = inspect_daemon(&workspace).map_err(internal_error)?;
            Ok(CliOutput::Text(match status {
                DaemonStatus::NotFound => "daemon_not_found".to_string(),
                DaemonStatus::Active => "daemon_active".to_string(),
                DaemonStatus::Stale => "daemon_stale".to_string(),
            }))
        }
        [command, subcommand, rest @ ..] if command == "daemon" && subcommand == "run" => {
            let workspace = parse_workspace(rest)?;
            let serve_one = rest.iter().any(|item| item == "--serve-one");
            if serve_one {
                Ok(CliOutput::Text(format!(
                    "daemon_run_serve_one:{}",
                    workspace.to_string_lossy()
                )))
            } else {
                Ok(CliOutput::Text(format!(
                    "daemon_run:{}",
                    workspace.to_string_lossy()
                )))
            }
        }
        [command, rest @ ..] if command == "repl" => {
            let workspace = parse_workspace(rest)?;
            let mode = if rest.iter().any(|item| item == "--no-start") {
                DiscoveryMode::NoStart
            } else {
                DiscoveryMode::AutoStart
            };
            let plan = resolve_daemon_connection(&workspace, mode)?;
            Ok(CliOutput::Text(format!("{plan:?}")))
        }
        [command, subcommand, ..] if command == "task" && subcommand == "run" => Err(CliError {
            code: "task_run_requires_async".to_string(),
            message: "task run is only available through run_cli_async".to_string(),
        }),
        [command, rest @ ..] if command == "tui" => {
            let workspace = parse_workspace(rest)?;
            let task_id = parse_task_id(rest)?;
            if rest.iter().any(|item| item == "--check") {
                crate::tui::app::check_tui_browse(&workspace, task_id.as_deref())
                    .map_err(task_run_error)?;
            }
            Ok(CliOutput::Text(match task_id {
                Some(task_id) => format!("tui_browse:{}:{task_id}", workspace.to_string_lossy()),
                None => format!("tui_browse:{}", workspace.to_string_lossy()),
            }))
        }
        [command, rest @ ..] if command == "web" => {
            let options = parse_web_options(rest)?;
            if options.check {
                return Ok(CliOutput::Text(format!(
                    "web_check_ok:{}:{}:{}",
                    options.workspace.to_string_lossy(),
                    options.host,
                    options
                        .port
                        .map(|port| port.to_string())
                        .unwrap_or_else(|| "auto".to_string())
                )));
            }
            Err(CliError {
                code: "web_requires_async".to_string(),
                message: "web server is only available through run_cli_async".to_string(),
            })
        }
        _ => Err(CliError {
            code: "invalid_cli_args".to_string(),
            message: "expected daemon status, repl, task run, tui, or web command".to_string(),
        }),
    }
}

pub async fn run_cli_async<I, S>(args: I) -> Result<CliOutput, CliError>
where
    I: IntoIterator<Item = S>,
    S: Into<String>,
{
    let args: Vec<String> = args.into_iter().map(Into::into).collect();
    match args.as_slice() {
        [command, subcommand, rest @ ..] if command == "task" && subcommand == "run" => {
            let options = parse_task_run_args(rest).map_err(task_run_error)?;
            let change_id = options
                .change_id
                .clone()
                .unwrap_or_else(|| "aria-login-jwt".to_string());
            let provider = real_routing_provider().map_err(task_run_error)?;
            let outcome = TaskRunOrchestrator::run_with_provider(
                TaskRunRequest {
                    task_id: None,
                    workspace: options.workspace,
                    request_text: options.request_text,
                    change_id,
                    provider_mode: options.provider_mode,
                    non_interactive: options.non_interactive,
                    timeout_secs: options.timeout_secs,
                },
                &provider,
            )
            .map_err(task_run_error)?;
            let text = match options.report_mode {
                ReportMode::Text => format!(
                    "task_id={}\nchange_id={}\nstatus={}\nreport={}",
                    outcome.task_id,
                    outcome.change_id,
                    task_status_text(&outcome.status),
                    outcome.report_path.to_string_lossy()
                ),
                ReportMode::Json => serde_json::to_string_pretty(&serde_json::json!({
                    "task_id": outcome.task_id,
                    "change_id": outcome.change_id,
                    "status": task_status_text(&outcome.status),
                    "report_path": outcome.report_path,
                    "openspec_change_dir": outcome.openspec_change_dir,
                    "provider_run_refs": outcome.provider_run_refs,
                    "testing_report_path": outcome.testing_report_path,
                    "final_summary_path": outcome.final_summary_path,
                    "blocked_report_path": outcome.blocked_report_path,
                }))
                .map_err(internal_error)?,
            };
            Ok(CliOutput::Text(text))
        }
        [command, subcommand, rest @ ..] if command == "daemon" && subcommand == "run" => {
            let workspace = parse_workspace(rest)?;
            let socket = parse_socket(rest);
            if rest.iter().any(|item| item == "--serve-one") {
                run_daemon_serve_one(&workspace, socket)
                    .await
                    .map_err(internal_error)?;
            } else {
                let (_shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
                run_daemon_until_shutdown(&workspace, socket, shutdown_rx)
                    .await
                    .map_err(internal_error)?;
            }
            Ok(CliOutput::Text(String::new()))
        }
        [command, rest @ ..] if command == "web" => {
            let options = parse_web_options(rest)?;
            if options.check {
                return run_cli(args);
            }
            crate::web::app::serve_web(options.workspace, options.host, options.port)
                .await
                .map_err(internal_error)?;
            Ok(CliOutput::Text(String::new()))
        }
        _ => run_cli(args),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WebOptions {
    workspace: PathBuf,
    host: String,
    port: Option<u16>,
    check: bool,
}

fn parse_web_options(args: &[String]) -> Result<WebOptions, CliError> {
    let workspace = parse_workspace(args)?;
    let host = parse_value(args, "--host").unwrap_or_else(|| "127.0.0.1".to_string());
    let port = parse_value(args, "--port")
        .map(|value| value.parse::<u16>())
        .transpose()
        .map_err(|error| CliError {
            code: "invalid_cli_args".to_string(),
            message: format!("--port must be a u16: {error}"),
        })?;
    Ok(WebOptions {
        workspace,
        host,
        port,
        check: args.iter().any(|item| item == "--check"),
    })
}

fn parse_value(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|window| window[0] == flag)
        .map(|window| window[1].clone())
}

fn parse_workspace(args: &[String]) -> Result<PathBuf, CliError> {
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--workspace" {
            let value = args.get(index + 1).ok_or_else(|| CliError {
                code: "invalid_cli_args".to_string(),
                message: "--workspace requires a path".to_string(),
            })?;
            return Ok(Path::new(value).to_path_buf());
        }
        index += 1;
    }

    std::env::current_dir().map_err(internal_error)
}

fn parse_socket(args: &[String]) -> Option<PathBuf> {
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--socket" {
            return args.get(index + 1).map(PathBuf::from);
        }
        index += 1;
    }
    None
}

fn parse_task_id(args: &[String]) -> Result<Option<String>, CliError> {
    let mut index = 0;
    while index < args.len() {
        if args[index] == "--task-id" {
            let value = args.get(index + 1).ok_or_else(|| CliError {
                code: "invalid_cli_args".to_string(),
                message: "--task-id requires a value".to_string(),
            })?;
            return Ok(Some(value.clone()));
        }
        index += 1;
    }
    Ok(None)
}

fn internal_error(error: impl std::fmt::Display) -> CliError {
    CliError {
        code: "internal_error".to_string(),
        message: error.to_string(),
    }
}

fn task_run_error(error: crate::task_run::types::TaskRunError) -> CliError {
    CliError {
        code: error.code,
        message: error.message,
    }
}

fn task_status_text(status: &TaskRunStatus) -> &'static str {
    match status {
        TaskRunStatus::Completed => "completed",
        TaskRunStatus::Failed => "failed",
        TaskRunStatus::BlockedByGate => "blocked_by_gate",
    }
}
