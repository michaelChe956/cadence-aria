use cadence_aria::daemon::discovery::{
    DaemonMetadata, PROTOCOL_VERSION, daemon_lock_path, daemon_metadata_path, default_socket_path,
    write_daemon_lock, write_daemon_metadata,
};
use cadence_aria::repl::discovery::{DiscoveryMode, DiscoveryPlan, resolve_daemon_connection};
use tempfile::tempdir;

#[test]
fn repl_discovery_cleans_stale_metadata_before_auto_start_plan() {
    let workspace = tempdir().expect("temp workspace");
    let socket_path = default_socket_path(workspace.path()).expect("socket path");
    write_daemon_metadata(
        workspace.path(),
        &DaemonMetadata {
            daemon_session_id: "sess_stale".to_string(),
            pid: u32::MAX,
            workspace_root: workspace.path().to_string_lossy().to_string(),
            socket_path: socket_path.to_string_lossy().to_string(),
            started_at: "2026-04-26T00:00:00Z".to_string(),
            protocol_version: PROTOCOL_VERSION.to_string(),
        },
    )
    .expect("metadata");
    write_daemon_lock(workspace.path(), u32::MAX).expect("lock");

    let plan = resolve_daemon_connection(workspace.path(), DiscoveryMode::AutoStart)
        .expect("discovery plan");

    assert_eq!(
        plan,
        DiscoveryPlan::StartDaemon {
            workspace_root: workspace.path().to_path_buf(),
            socket_path: default_socket_path(workspace.path()).expect("socket path")
        }
    );
    assert!(
        !daemon_metadata_path(workspace.path())
            .expect("metadata path")
            .exists()
    );
    assert!(
        !daemon_lock_path(workspace.path())
            .expect("lock path")
            .exists()
    );
}

#[test]
fn repl_discovery_no_start_returns_daemon_not_found() {
    let workspace = tempdir().expect("temp workspace");

    let error = resolve_daemon_connection(workspace.path(), DiscoveryMode::NoStart)
        .expect_err("no-start without daemon should fail");

    assert_eq!(error.code, "daemon_not_found");
}
