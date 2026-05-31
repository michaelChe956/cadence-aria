use cadence_aria::cli::{CliOutput, run_cli};
use cadence_aria::daemon::discovery::{
    DaemonMetadata, PROTOCOL_VERSION, default_socket_path, write_daemon_lock, write_daemon_metadata,
};
use tempfile::tempdir;

#[test]
fn daemon_status_reports_not_found_for_clean_workspace() {
    let workspace = tempdir().expect("temp workspace");

    let output = run_cli([
        "daemon",
        "status",
        "--workspace",
        workspace.path().to_str().expect("workspace path"),
    ])
    .expect("daemon status");

    assert_eq!(output, CliOutput::Text("daemon_not_found".to_string()));
}

#[test]
fn repl_no_start_reports_daemon_not_found() {
    let workspace = tempdir().expect("temp workspace");

    let error = run_cli([
        "repl",
        "--workspace",
        workspace.path().to_str().expect("workspace path"),
        "--no-start",
    ])
    .expect_err("repl --no-start should fail without daemon");

    assert_eq!(error.code, "daemon_not_found");
}

#[test]
fn tui_command_is_not_available() {
    let workspace = tempdir().expect("temp workspace");

    let error = run_cli([
        "tui",
        "--workspace",
        workspace.path().to_str().expect("workspace path"),
    ])
    .expect_err("tui command should be removed");

    assert_eq!(error.code, "invalid_cli_args");
    assert!(!error.message.contains("tui"));
}

#[test]
fn daemon_status_reports_stale_for_dead_lock() {
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

    let output = run_cli([
        "daemon",
        "status",
        "--workspace",
        workspace.path().to_str().expect("workspace path"),
    ])
    .expect("daemon status");

    assert_eq!(output, CliOutput::Text("daemon_stale".to_string()));
}
