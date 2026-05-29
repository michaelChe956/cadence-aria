use cadence_aria::daemon::discovery::{
    daemon_lock_path, daemon_metadata_path, default_socket_path,
};
use cadence_aria::protocol::repl_wire::{Command, DetachRequest, HelloRequest, RequestEnvelope};
use cadence_aria::repl::transport::UnixJsonTransport;
use std::process::Command as ProcessCommand;
use tempfile::tempdir;

#[tokio::test]
async fn aria_daemon_run_starts_socket_and_serves_wire_request() {
    let workspace = tempdir().expect("temp workspace");
    let socket_path = default_socket_path(workspace.path()).expect("socket path");
    let aria_bin = env!("CARGO_BIN_EXE_aria");
    let mut child = ProcessCommand::new(aria_bin)
        .args([
            "daemon",
            "run",
            "--workspace",
            workspace.path().to_str().expect("workspace path"),
            "--serve-one",
        ])
        .spawn()
        .expect("spawn aria daemon");

    wait_for_ready(workspace.path()).await;

    let mut transport = UnixJsonTransport::connect(&socket_path)
        .await
        .expect("connect daemon");
    let hello = transport
        .send_request(
            RequestEnvelope::new(
                "req_hello_process",
                Command::Hello,
                HelloRequest {
                    last_seen_event_id: None,
                },
            )
            .expect("hello request"),
        )
        .await
        .expect("hello response");
    assert!(hello.ok);

    let detach = transport
        .send_request(
            RequestEnvelope::new("req_detach_process", Command::Detach, DetachRequest {})
                .expect("detach request"),
        )
        .await
        .expect("detach response");
    assert!(detach.ok);

    let status = child.wait().expect("daemon process exits");
    assert!(status.success());
    assert!(
        !daemon_lock_path(workspace.path())
            .expect("lock path")
            .exists()
    );
    assert!(!socket_path.exists());
}

async fn wait_for_ready(workspace_root: &std::path::Path) {
    let metadata_path = daemon_metadata_path(workspace_root).expect("metadata path");
    let socket_path = default_socket_path(workspace_root).expect("socket path");
    for _ in 0..500 {
        if metadata_path.exists() && socket_path.exists() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("daemon process did not become ready");
}
