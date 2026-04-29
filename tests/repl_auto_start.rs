use cadence_aria::protocol::repl_wire::{Command, DetachRequest, HelloRequest, RequestEnvelope};
use cadence_aria::repl::discovery::{AutoStartOptions, start_daemon_and_wait_ready};
use cadence_aria::repl::transport::UnixJsonTransport;
use tempfile::tempdir;

#[tokio::test]
async fn repl_auto_start_spawns_daemon_and_waits_until_socket_is_ready() {
    let workspace = tempdir().expect("temp workspace");
    let aria_bin = env!("CARGO_BIN_EXE_aria");

    let mut started = start_daemon_and_wait_ready(AutoStartOptions {
        aria_bin: aria_bin.into(),
        workspace_root: workspace.path().to_path_buf(),
        serve_one: true,
        timeout_ms: 2_000,
    })
    .await
    .expect("auto-start daemon");

    assert!(started.socket_path.exists());

    let mut transport = UnixJsonTransport::connect(&started.socket_path)
        .await
        .expect("connect daemon");
    let hello = transport
        .send_request(
            RequestEnvelope::new(
                "req_hello_autostart",
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
            RequestEnvelope::new("req_detach_autostart", Command::Detach, DetachRequest {})
                .expect("detach request"),
        )
        .await
        .expect("detach response");
    assert!(detach.ok);

    let status = started.child.wait().await.expect("daemon exits");
    assert!(status.success());
}
