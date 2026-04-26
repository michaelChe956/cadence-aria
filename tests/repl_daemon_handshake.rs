use cadence_aria::daemon::{serve_one_connection, state_machine::DaemonState};
use cadence_aria::protocol::repl_wire::{
    AttachRequest, Command, DetachRequest, GetStatusRequest, HelloRequest, ListArtifactsRequest,
    NewTaskRequest, RequestEnvelope, SubscribeRequest,
};
use cadence_aria::repl::transport::UnixJsonTransport;
use tempfile::tempdir;

#[test]
fn daemon_handles_minimum_repl_handshake_and_task_commands() {
    let workspace = tempdir().expect("temp workspace");
    let mut state = DaemonState::bootstrap(workspace.path()).expect("bootstrap daemon state");

    let hello = state
        .handle_request(
            RequestEnvelope::new(
                "req_hello",
                Command::Hello,
                HelloRequest {
                    last_seen_event_id: None,
                },
            )
            .expect("hello request"),
        )
        .expect("hello response");
    assert!(hello.ok);
    assert_eq!(hello.command, Command::Hello);

    let attach = state
        .handle_request(
            RequestEnvelope::new(
                "req_attach",
                Command::Attach,
                AttachRequest {
                    reconnect_token: None,
                },
            )
            .expect("attach request"),
        )
        .expect("attach response");
    assert!(attach.ok);

    let subscribe = state
        .handle_request(
            RequestEnvelope::new(
                "req_subscribe",
                Command::Subscribe,
                SubscribeRequest { event_types: None },
            )
            .expect("subscribe request"),
        )
        .expect("subscribe response");
    assert!(subscribe.ok);

    let new_task = state
        .handle_request(
            RequestEnvelope::new(
                "req_task",
                Command::NewTask,
                NewTaskRequest {
                    request_text: "REPL 联调任务".to_string(),
                    requested_change_id: None,
                },
            )
            .expect("new task request"),
        )
        .expect("new task response");
    assert!(new_task.ok);

    let get_status = state
        .handle_request(
            RequestEnvelope::new(
                "req_status",
                Command::GetStatus,
                GetStatusRequest { task_id: None },
            )
            .expect("status request"),
        )
        .expect("status response");
    assert!(get_status.ok);

    let task_id = new_task.payload.as_ref().expect("payload")["task_id"]
        .as_str()
        .expect("task id")
        .to_string();
    let artifacts = state
        .handle_request(
            RequestEnvelope::new(
                "req_artifacts",
                Command::ListArtifacts,
                ListArtifactsRequest {
                    task_id: task_id.clone(),
                    artifact_kind: Some("intake_brief".to_string()),
                },
            )
            .expect("list artifacts request"),
        )
        .expect("list artifacts response");
    assert!(artifacts.ok);
    assert_eq!(
        artifacts.payload.as_ref().expect("artifact payload")["artifacts"]
            .as_array()
            .expect("artifact array")
            .len(),
        1
    );

    let filtered_artifacts = state
        .handle_request(
            RequestEnvelope::new(
                "req_artifacts_filtered",
                Command::ListArtifacts,
                ListArtifactsRequest {
                    task_id,
                    artifact_kind: Some("spec".to_string()),
                },
            )
            .expect("filtered artifacts request"),
        )
        .expect("filtered artifacts response");
    assert!(filtered_artifacts.ok);
    assert_eq!(
        filtered_artifacts
            .payload
            .as_ref()
            .expect("filtered artifact payload")["artifacts"]
            .as_array()
            .expect("filtered artifact array")
            .len(),
        0
    );

    let detach = state
        .handle_request(
            RequestEnvelope::new("req_detach", Command::Detach, DetachRequest {})
                .expect("detach request"),
        )
        .expect("detach response");
    assert!(detach.ok);
}

#[tokio::test]
async fn repl_and_daemon_exchange_wire_messages_over_unix_socket() {
    let workspace = tempdir().expect("temp workspace");
    let socket_path = workspace.path().join("daemon.sock");
    let server_workspace = workspace.path().to_path_buf();
    let server_socket = socket_path.clone();

    let server = tokio::spawn(async move {
        serve_one_connection(&server_workspace, &server_socket)
            .await
            .expect("daemon socket server");
    });

    wait_for_socket(&socket_path).await;

    let mut transport = UnixJsonTransport::connect(&socket_path)
        .await
        .expect("connect transport");

    let hello = transport
        .send_request(
            RequestEnvelope::new(
                "req_hello_socket",
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

    let attach = transport
        .send_request(
            RequestEnvelope::new(
                "req_attach_socket",
                Command::Attach,
                AttachRequest {
                    reconnect_token: None,
                },
            )
            .expect("attach request"),
        )
        .await
        .expect("attach response");
    assert!(attach.ok);

    let detach = transport
        .send_request(
            RequestEnvelope::new("req_detach_socket", Command::Detach, DetachRequest {})
                .expect("detach request"),
        )
        .await
        .expect("detach response");
    assert!(detach.ok);

    server.await.expect("server task");
}

async fn wait_for_socket(socket_path: &std::path::Path) {
    for _ in 0..50 {
        if socket_path.exists() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("socket was not created");
}
