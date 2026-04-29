use std::path::{Path, PathBuf};

use tokio::net::UnixListener;
use tokio::sync::oneshot;

use crate::daemon::discovery::{
    DaemonMetadata, PROTOCOL_VERSION, default_socket_path, write_daemon_lock, write_daemon_metadata,
};
use crate::daemon::{handle_stream, state_machine::DaemonState};

pub async fn run_daemon_serve_one(
    workspace_root: &Path,
    socket_override: Option<PathBuf>,
) -> anyhow::Result<()> {
    let socket_path = socket_override.unwrap_or(default_socket_path(workspace_root)?);
    let listener = prepare_listener(workspace_root, &socket_path)?;
    let mut state = DaemonState::bootstrap(workspace_root)?;
    let (stream, _) = listener.accept().await?;
    handle_stream(stream, &mut state).await?;
    cleanup_daemon_files(workspace_root, &socket_path);
    Ok(())
}

pub async fn run_daemon_until_shutdown(
    workspace_root: &Path,
    socket_override: Option<PathBuf>,
    mut shutdown: oneshot::Receiver<()>,
) -> anyhow::Result<()> {
    let socket_path = socket_override.unwrap_or(default_socket_path(workspace_root)?);
    let listener = prepare_listener(workspace_root, &socket_path)?;
    let mut state = DaemonState::bootstrap(workspace_root)?;

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                break;
            }
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                handle_stream(stream, &mut state).await?;
            }
        }
    }

    cleanup_daemon_files(workspace_root, &socket_path);
    Ok(())
}

fn prepare_listener(workspace_root: &Path, socket_path: &Path) -> anyhow::Result<UnixListener> {
    if socket_path.exists() {
        std::fs::remove_file(socket_path)?;
    }
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let listener = UnixListener::bind(socket_path)?;
    let metadata = DaemonMetadata {
        daemon_session_id: format!("sess_{}", uuid::Uuid::new_v4().simple()),
        pid: std::process::id(),
        workspace_root: workspace_root.to_string_lossy().to_string(),
        socket_path: socket_path.to_string_lossy().to_string(),
        started_at: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        protocol_version: PROTOCOL_VERSION.to_string(),
    };
    write_daemon_lock(workspace_root, metadata.pid)?;
    write_daemon_metadata(workspace_root, &metadata)?;
    Ok(listener)
}

fn cleanup_daemon_files(workspace_root: &Path, socket_path: &Path) {
    if socket_path.exists() {
        let _ = std::fs::remove_file(socket_path);
    }
    if let Ok(lock_path) = crate::daemon::discovery::daemon_lock_path(workspace_root)
        && lock_path.exists()
    {
        let _ = std::fs::remove_file(lock_path);
    }
}
