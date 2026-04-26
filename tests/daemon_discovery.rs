use cadence_aria::daemon::discovery::{
    daemon_runtime_dir, default_socket_path, inspect_daemon, read_daemon_metadata, workspace_hash,
    write_daemon_lock, write_daemon_metadata, DaemonMetadata, DaemonStatus, PROTOCOL_VERSION,
};
use sha2::{Digest, Sha256};
use tempfile::tempdir;

#[test]
fn workspace_hash_is_first_twelve_sha256_chars_of_canonical_path() {
    let workspace = tempdir().expect("temp workspace");
    let canonical = workspace.path().canonicalize().expect("canonical path");
    let expected = hex::encode(Sha256::digest(canonical.to_string_lossy().as_bytes()));

    assert_eq!(
        workspace_hash(workspace.path()).expect("workspace hash"),
        expected[..12]
    );
}

#[test]
fn default_daemon_paths_live_under_task_runtime_dir() {
    let workspace = tempdir().expect("temp workspace");
    let hash = workspace_hash(workspace.path()).expect("workspace hash");

    let runtime_dir = daemon_runtime_dir(workspace.path()).expect("runtime dir");
    let socket_path = default_socket_path(workspace.path()).expect("socket path");

    assert!(runtime_dir.ends_with(format!(".aria/runtime/daemon/{hash}")));
    assert!(socket_path.ends_with(format!(".aria/runtime/daemon/{hash}/daemon.sock")));
}

#[test]
fn metadata_and_lock_round_trip_with_snake_case_fields() {
    let workspace = tempdir().expect("temp workspace");
    let socket_path = default_socket_path(workspace.path()).expect("socket path");
    let metadata = DaemonMetadata {
        daemon_session_id: "sess_001".to_string(),
        pid: 12345,
        workspace_root: workspace.path().to_string_lossy().to_string(),
        socket_path: socket_path.to_string_lossy().to_string(),
        started_at: "2026-04-26T00:00:00Z".to_string(),
        protocol_version: PROTOCOL_VERSION.to_string(),
    };

    write_daemon_metadata(workspace.path(), &metadata).expect("write metadata");
    write_daemon_lock(workspace.path(), metadata.pid).expect("write lock");

    let stored = read_daemon_metadata(workspace.path()).expect("read metadata");
    let value = serde_json::to_value(stored).expect("metadata json");

    assert_eq!(value["daemon_session_id"], "sess_001");
    assert_eq!(value["protocol_version"], PROTOCOL_VERSION);
    assert!(value.get("daemonSessionId").is_none());
}

#[test]
fn inspect_daemon_marks_dead_pid_or_missing_socket_as_stale() {
    let workspace = tempdir().expect("temp workspace");
    let socket_path = default_socket_path(workspace.path()).expect("socket path");
    let metadata = DaemonMetadata {
        daemon_session_id: "sess_stale".to_string(),
        pid: u32::MAX,
        workspace_root: workspace.path().to_string_lossy().to_string(),
        socket_path: socket_path.to_string_lossy().to_string(),
        started_at: "2026-04-26T00:00:00Z".to_string(),
        protocol_version: PROTOCOL_VERSION.to_string(),
    };

    write_daemon_metadata(workspace.path(), &metadata).expect("write metadata");
    write_daemon_lock(workspace.path(), metadata.pid).expect("write lock");

    assert_eq!(
        inspect_daemon(workspace.path()).expect("inspect daemon"),
        DaemonStatus::Stale
    );
}
