use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};

pub const PROTOCOL_VERSION: &str = crate::protocol::repl_wire::PROTOCOL_VERSION;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct DaemonMetadata {
    pub daemon_session_id: String,
    pub pid: u32,
    pub workspace_root: String,
    pub socket_path: String,
    pub started_at: String,
    pub protocol_version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonStatus {
    NotFound,
    Active,
    Stale,
}

pub fn workspace_hash(workspace_root: &Path) -> anyhow::Result<String> {
    let canonical = workspace_root.canonicalize()?;
    let digest = Sha256::digest(canonical.to_string_lossy().as_bytes());
    Ok(hex::encode(digest)[..12].to_string())
}

pub fn daemon_runtime_dir(workspace_root: &Path) -> anyhow::Result<PathBuf> {
    Ok(workspace_root
        .join(".aria")
        .join("runtime")
        .join("daemon")
        .join(workspace_hash(workspace_root)?))
}

pub fn default_socket_path(workspace_root: &Path) -> anyhow::Result<PathBuf> {
    Ok(daemon_runtime_dir(workspace_root)?.join("daemon.sock"))
}

pub fn daemon_metadata_path(workspace_root: &Path) -> anyhow::Result<PathBuf> {
    Ok(daemon_runtime_dir(workspace_root)?.join("daemon.json"))
}

pub fn daemon_lock_path(workspace_root: &Path) -> anyhow::Result<PathBuf> {
    Ok(daemon_runtime_dir(workspace_root)?.join("daemon.lock"))
}

pub fn write_daemon_metadata(
    workspace_root: &Path,
    metadata: &DaemonMetadata,
) -> anyhow::Result<()> {
    let path = daemon_metadata_path(workspace_root)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_vec_pretty(metadata)?)?;
    Ok(())
}

pub fn read_daemon_metadata(workspace_root: &Path) -> anyhow::Result<DaemonMetadata> {
    let path = daemon_metadata_path(workspace_root)?;
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}

pub fn write_daemon_lock(workspace_root: &Path, pid: u32) -> anyhow::Result<()> {
    let path = daemon_lock_path(workspace_root)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, format!("{pid}\n"))?;
    Ok(())
}

pub fn inspect_daemon(workspace_root: &Path) -> anyhow::Result<DaemonStatus> {
    inspect_daemon_with_pid_checker(workspace_root, pid_is_alive)
}

pub fn inspect_daemon_with_pid_checker(
    workspace_root: &Path,
    pid_checker: impl Fn(u32) -> bool,
) -> anyhow::Result<DaemonStatus> {
    let metadata_path = daemon_metadata_path(workspace_root)?;
    let lock_path = daemon_lock_path(workspace_root)?;

    if !metadata_path.exists() && !lock_path.exists() {
        return Ok(DaemonStatus::NotFound);
    }

    let metadata = match read_daemon_metadata(workspace_root) {
        Ok(metadata) => metadata,
        Err(_) => return Ok(DaemonStatus::Stale),
    };

    if metadata.protocol_version != PROTOCOL_VERSION {
        return Ok(DaemonStatus::Stale);
    }

    let socket_exists = Path::new(&metadata.socket_path).exists();
    if pid_checker(metadata.pid) && socket_exists {
        Ok(DaemonStatus::Active)
    } else {
        Ok(DaemonStatus::Stale)
    }
}

fn pid_is_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }

    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn kill(pid: i32, sig: i32) -> i32;
        }

        unsafe { kill(pid as i32, 0) == 0 }
    }

    #[cfg(not(unix))]
    {
        pid == std::process::id()
    }
}
