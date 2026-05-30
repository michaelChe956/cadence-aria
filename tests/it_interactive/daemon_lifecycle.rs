use cadence_aria::daemon::discovery::{
    DaemonStatus, daemon_lock_path, daemon_metadata_path, default_socket_path, inspect_daemon,
};
use cadence_aria::daemon::runner::run_daemon_until_shutdown;
use cadence_aria::protocol::repl_wire::{Command, DetachRequest, HelloRequest, RequestEnvelope};
use cadence_aria::repl::transport::UnixJsonTransport;
use tempfile::tempdir;
use tokio::sync::oneshot;

#[tokio::test]
async fn daemon_run_writes_metadata_accepts_wire_messages_and_cleans_up_on_shutdown() {
    let workspace = tempdir().expect("temp workspace");
    let socket_path = default_socket_path(workspace.path()).expect("socket path");
    let (shutdown_tx, shutdown_rx) = oneshot::channel();
    let workspace_root = workspace.path().to_path_buf();

    let daemon = tokio::spawn(async move {
        run_daemon_until_shutdown(&workspace_root, None, shutdown_rx)
            .await
            .expect("daemon lifecycle");
    });

    wait_for_ready(workspace.path()).await;
    assert_eq!(
        inspect_daemon(workspace.path()).expect("inspect"),
        DaemonStatus::Active
    );

    let mut transport = UnixJsonTransport::connect(&socket_path)
        .await
        .expect("connect daemon");
    let hello = transport
        .send_request(
            RequestEnvelope::new(
                "req_hello",
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
            RequestEnvelope::new("req_detach", Command::Detach, DetachRequest {})
                .expect("detach request"),
        )
        .await
        .expect("detach response");
    assert!(detach.ok);

    shutdown_tx.send(()).expect("shutdown send");
    daemon.await.expect("daemon task");

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
    for _ in 0..100 {
        if metadata_path.exists() && socket_path.exists() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("daemon did not become ready");
}
