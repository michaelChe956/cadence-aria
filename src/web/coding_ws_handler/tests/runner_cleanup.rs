use std::time::Duration;

use tokio::sync::mpsc;

use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::web::coding_ws_handler::runner::spawn_coding_runner_panicking_after_registration;
use crate::web::coding_ws_handler::socket::abort::abort_attempt_while_draining_events;
use crate::web::coding_ws_handler::{CodingWsOutMessage, OutboundEventReceiver};
use crate::web::runtime::WebRuntime;
use crate::web::state::{CodingAttemptRunKey, CodingRunRegistry, WebAppState};

use super::seed_compiled_work_item_fixture;

#[tokio::test]
async fn spawned_runner_panic_removes_registry_registration() {
    let (tmp, app_paths, attempt) = seed_compiled_work_item_fixture();
    let state = WebAppState::new(
        tmp.path().to_path_buf(),
        WebRuntime::new_fake(tmp.path().to_path_buf()),
    );
    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    let (event_tx, _event_rx) = mpsc::channel(1);
    let panic_entered = spawn_coding_runner_panicking_after_registration(
        state.clone(),
        CodingAttemptStore::new(app_paths),
        event_tx,
        attempt,
    );
    tokio::time::timeout(Duration::from_millis(250), panic_entered)
        .await
        .expect("spawned runner did not reach panic probe")
        .expect("panic probe sender dropped");

    tokio::time::timeout(Duration::from_millis(250), async {
        while state.coding_runs.runner_count(&attempt_key) != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("panicked runner must remove its registry registration");
}

#[tokio::test]
async fn abort_drains_full_event_queue_until_runner_removes_registration() {
    let registry = CodingRunRegistry::default();
    let attempt_key =
        CodingAttemptRunKey::new("project_0001", "issue_0001", "coding_attempt_backpressure");
    let (command_tx, mut command_rx) = mpsc::channel(1);
    let run_id = registry
        .insert(&attempt_key, command_tx)
        .expect("backpressured runner");
    let (event_tx, event_rx) = mpsc::channel(1);
    let mut event_rx = OutboundEventReceiver::new(event_rx);
    let runner_registry = registry.clone();
    let runner_key = attempt_key.clone();
    let runner = tokio::spawn(async move {
        event_tx
            .send(CodingWsOutMessage::CodingProtocolError {
                code: "first".to_string(),
                message: "first event".to_string(),
            })
            .await
            .expect("first event");
        event_tx
            .send(CodingWsOutMessage::CodingProtocolError {
                code: "second".to_string(),
                message: "second event".to_string(),
            })
            .await
            .expect("second event");
        assert_eq!(
            command_rx.recv().await,
            Some(CodingRunnerCommand::AbortAttempt)
        );
        runner_registry.remove(&runner_key, run_id);
    });

    let drained = tokio::time::timeout(
        Duration::from_millis(250),
        abort_attempt_while_draining_events(&registry, &attempt_key, &mut event_rx),
    )
    .await
    .expect("abort must drain events while waiting for runner completion");
    runner.await.expect("backpressured runner task");

    assert_eq!(drained.aborted_runners, 1);
    let codes = drained
        .events
        .iter()
        .map(|event| match event {
            CodingWsOutMessage::CodingProtocolError { code, .. } => code.as_str(),
            other => panic!("unexpected drained event: {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(codes, vec!["first", "second"]);
    assert_eq!(registry.runner_count(&attempt_key), 0);
}
