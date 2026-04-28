use std::path::{Path, PathBuf};

use crate::daemon::discovery::{inspect_daemon, DaemonStatus};
use crate::daemon::runner::{run_daemon_serve_one, run_daemon_until_shutdown};
use crate::repl::discovery::{resolve_daemon_connection, DiscoveryMode};

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
        _ => Err(CliError {
            code: "invalid_cli_args".to_string(),
            message: "expected daemon status or repl command".to_string(),
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
        _ => run_cli(args),
    }
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

fn internal_error(error: impl std::fmt::Display) -> CliError {
    CliError {
        code: "internal_error".to_string(),
        message: error.to_string(),
    }
}
