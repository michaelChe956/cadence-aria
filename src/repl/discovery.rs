use std::fs;
use std::path::{Path, PathBuf};

use tokio::process::Child;

use crate::cli::CliError;
use crate::daemon::discovery::{
    DaemonStatus, daemon_lock_path, daemon_metadata_path, default_socket_path, inspect_daemon,
    read_daemon_metadata,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscoveryMode {
    AutoStart,
    NoStart,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryPlan {
    Connect {
        socket_path: PathBuf,
    },
    StartDaemon {
        workspace_root: PathBuf,
        socket_path: PathBuf,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoStartOptions {
    pub aria_bin: PathBuf,
    pub workspace_root: PathBuf,
    pub serve_one: bool,
    pub timeout_ms: u64,
}

pub struct StartedDaemon {
    pub child: Child,
    pub socket_path: PathBuf,
}

pub fn resolve_daemon_connection(
    workspace_root: &Path,
    mode: DiscoveryMode,
) -> Result<DiscoveryPlan, CliError> {
    match inspect_daemon(workspace_root).map_err(internal_error)? {
        DaemonStatus::Active => {
            let metadata = read_daemon_metadata(workspace_root).map_err(internal_error)?;
            Ok(DiscoveryPlan::Connect {
                socket_path: PathBuf::from(metadata.socket_path),
            })
        }
        DaemonStatus::NotFound => match mode {
            DiscoveryMode::NoStart => Err(CliError {
                code: "daemon_not_found".to_string(),
                message: "daemon was not found and --no-start was set".to_string(),
            }),
            DiscoveryMode::AutoStart => Ok(DiscoveryPlan::StartDaemon {
                workspace_root: workspace_root.to_path_buf(),
                socket_path: default_socket_path(workspace_root).map_err(internal_error)?,
            }),
        },
        DaemonStatus::Stale => {
            cleanup_stale_daemon(workspace_root)?;
            match mode {
                DiscoveryMode::NoStart => Err(CliError {
                    code: "daemon_not_found".to_string(),
                    message: "stale daemon metadata was removed and --no-start was set".to_string(),
                }),
                DiscoveryMode::AutoStart => Ok(DiscoveryPlan::StartDaemon {
                    workspace_root: workspace_root.to_path_buf(),
                    socket_path: default_socket_path(workspace_root).map_err(internal_error)?,
                }),
            }
        }
    }
}

fn cleanup_stale_daemon(workspace_root: &Path) -> Result<(), CliError> {
    for path in [
        daemon_metadata_path(workspace_root).map_err(internal_error)?,
        daemon_lock_path(workspace_root).map_err(internal_error)?,
        default_socket_path(workspace_root).map_err(internal_error)?,
    ] {
        if path.exists() {
            fs::remove_file(path).map_err(internal_error)?;
        }
    }
    Ok(())
}

pub async fn start_daemon_and_wait_ready(
    options: AutoStartOptions,
) -> Result<StartedDaemon, CliError> {
    let socket_path = default_socket_path(&options.workspace_root).map_err(internal_error)?;
    let mut command = tokio::process::Command::new(&options.aria_bin);
    command.args([
        "daemon",
        "run",
        "--workspace",
        options.workspace_root.to_str().ok_or_else(|| CliError {
            code: "invalid_workspace".to_string(),
            message: "workspace path is not valid utf-8".to_string(),
        })?,
    ]);
    if options.serve_one {
        command.arg("--serve-one");
    }

    let child = command.spawn().map_err(internal_error)?;
    wait_until_ready(&options.workspace_root, options.timeout_ms).await?;
    Ok(StartedDaemon { child, socket_path })
}

async fn wait_until_ready(workspace_root: &Path, timeout_ms: u64) -> Result<(), CliError> {
    let metadata_path = daemon_metadata_path(workspace_root).map_err(internal_error)?;
    let socket_path = default_socket_path(workspace_root).map_err(internal_error)?;
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_millis(timeout_ms);

    loop {
        if metadata_path.exists() && socket_path.exists() {
            return Ok(());
        }

        if start.elapsed() >= timeout {
            return Err(CliError {
                code: "daemon_start_timeout".to_string(),
                message: "daemon did not become ready before timeout".to_string(),
            });
        }

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

fn internal_error(error: impl std::fmt::Display) -> CliError {
    CliError {
        code: "internal_error".to_string(),
        message: error.to_string(),
    }
}
