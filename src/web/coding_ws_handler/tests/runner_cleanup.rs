use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::pin::Pin;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::extract::ws::Message;
use axum::http::{Method, Request, StatusCode};
use futures_util::Sink;
use tokio::sync::mpsc;
use tower::ServiceExt;

use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingAttemptScope, CodingAttemptStatus, CodingExecutionAttempt,
};
use crate::product::coding_workspace_runner::CodingRunnerCommand;
use crate::product::lifecycle_store::{LifecycleStore, UpsertIssueSharedWorktreeInput};
use crate::product::models::{AmendmentResumeMode, AmendmentResumeTarget, PlanAmendmentManifest};
use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
use crate::web::app::build_web_router;
use crate::web::coding_ws_handler::delivery_ack::register_plan_amendment_socket_write;
use crate::web::coding_ws_handler::runner::{
    CodingRunnerStartProbe, spawn_coding_runner, spawn_coding_runner_panicking_after_registration,
    spawn_coding_runner_with_start_probe,
};
use crate::web::coding_ws_handler::socket::abort::abort_attempt_while_draining_events;
use crate::web::coding_ws_handler::{CodingWsOutMessage, OutboundEventReceiver, send_coding_event};
use crate::web::runtime::WebRuntime;
use crate::web::state::{CodingAttemptRunKey, CodingRunRegistry, WebAppState};
use tempfile::TempDir;

use super::seed_compiled_work_item_fixture;

struct PendingSocketSink {
    flush_entered: Arc<AtomicBool>,
}

impl Sink<Message> for PendingSocketSink {
    type Error = io::Error;

    fn poll_ready(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn start_send(self: Pin<&mut Self>, _message: Message) -> Result<(), Self::Error> {
        Ok(())
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        self.flush_entered.store(true, Ordering::SeqCst);
        Poll::Pending
    }

    fn poll_close(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
}

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
async fn runner_cancellation_waits_for_business_future_cleanup_before_registry_remove() {
    let (tmp, app_paths, attempt) = seed_compiled_work_item_fixture();
    let state = WebAppState::new(
        tmp.path().to_path_buf(),
        WebRuntime::new_fake(tmp.path().to_path_buf()),
    );
    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    let (event_tx, _event_rx) = mpsc::channel(1);
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let (provider_entry_tx, provider_entry_rx) = tokio::sync::oneshot::channel();
    let (_continue_tx, continue_rx) = tokio::sync::oneshot::channel();
    let _command_tx = spawn_coding_runner_with_start_probe(
        state.clone(),
        CodingAttemptStore::new(app_paths),
        event_tx,
        attempt,
        CodingRunnerStartProbe {
            events: Arc::clone(&events),
            provider_entry_tx,
            continue_rx,
        },
    );
    provider_entry_rx.await.expect("runner reached start probe");

    tokio::time::timeout(
        Duration::from_millis(250),
        state.coding_runs.abort_attempt(&attempt_key),
    )
    .await
    .expect("cooperative cancellation must converge");

    assert_eq!(state.coding_runs.runner_count(&attempt_key), 0);
    assert_eq!(
        events.lock().expect("runner events").as_slice(),
        ["provider_entry", "cancelled_before_provider"],
        "registry remove must happen only after the business future observes cancellation"
    );
}

#[tokio::test]
async fn abort_drains_full_event_queue_until_runner_removes_registration() {
    let registry = CodingRunRegistry::default();
    let attempt_key =
        CodingAttemptRunKey::new("project_0001", "issue_0001", "coding_attempt_backpressure");
    let (command_tx, mut command_rx) = mpsc::channel(1);
    let run_id = registry
        .insert_cancellable(&attempt_key, command_tx)
        .expect("backpressured runner")
        .run_id;
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

#[tokio::test]
async fn registry_abort_cancels_real_runner_blocked_on_full_event_queue() {
    let (tmp, app_paths, attempt) = seed_compiled_work_item_fixture();
    let state = WebAppState::new(
        tmp.path().to_path_buf(),
        WebRuntime::new_fake(tmp.path().to_path_buf()),
    );
    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    let (event_tx, _event_rx) = mpsc::channel(1);
    event_tx
        .try_send(CodingWsOutMessage::CodingPong)
        .expect("fill outbound event queue");
    let _command_tx = spawn_coding_runner(
        state.clone(),
        CodingAttemptStore::new(app_paths),
        event_tx,
        attempt,
    )
    .expect("spawn coding runner");

    let cancelled = tokio::time::timeout(
        Duration::from_millis(250),
        state.coding_runs.abort_attempt(&attempt_key),
    )
    .await
    .expect("registry abort must cancel a runner blocked on outbound backpressure");

    assert_eq!(cancelled, 1);
    assert_eq!(state.coding_runs.runner_count(&attempt_key), 0);
}

#[tokio::test]
async fn http_abort_cancels_full_event_queue_runner_and_releases_lease() {
    let (tmp, app_paths, state, attempt) = seed_http_attempt_with_lease();
    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    let (event_tx, _event_rx) = mpsc::channel(1);
    event_tx
        .try_send(CodingWsOutMessage::CodingPong)
        .expect("fill outbound event queue");
    let _command_tx = spawn_coding_runner(
        state.clone(),
        CodingAttemptStore::new(app_paths.clone()),
        event_tx,
        attempt.clone(),
    )
    .expect("spawn coding runner");

    let response = send_attempt_request(&state, &attempt, Method::POST, "/abort").await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("abort response body");
    let body: serde_json::Value = serde_json::from_slice(&body).expect("abort response json");
    assert_eq!(body["status"], "aborted");
    assert_eq!(state.coding_runs.runner_count(&attempt_key), 0);
    let stored = CodingAttemptStore::new(app_paths.clone())
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("aborted attempt");
    assert_eq!(stored.status, CodingAttemptStatus::Aborted);
    assert_issue_lease_released(&app_paths, &attempt);
    drop(tmp);
}

#[tokio::test]
async fn http_delete_cancels_full_event_queue_runner_and_removes_attempt() {
    let (tmp, app_paths, state, attempt) = seed_http_attempt_with_lease();
    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    let (event_tx, _event_rx) = mpsc::channel(1);
    event_tx
        .try_send(CodingWsOutMessage::CodingPong)
        .expect("fill outbound event queue");
    let _command_tx = spawn_coding_runner(
        state.clone(),
        CodingAttemptStore::new(app_paths.clone()),
        event_tx,
        attempt.clone(),
    )
    .expect("spawn coding runner");

    let response = send_attempt_request(&state, &attempt, Method::DELETE, "").await;

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(state.coding_runs.runner_count(&attempt_key), 0);
    assert!(
        CodingAttemptStore::new(app_paths.clone())
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .is_err(),
        "delete must remove attempt record"
    );
    assert_issue_lease_released(&app_paths, &attempt);
    drop(tmp);
}

#[tokio::test]
async fn http_abort_is_not_blocked_by_pending_socket_writer_or_acknowledges_delivery() {
    let (tmp, app_paths, state, attempt) = seed_http_attempt_with_lease();
    let (event_tx, _event_rx) = mpsc::channel(1);
    event_tx
        .try_send(CodingWsOutMessage::CodingPong)
        .expect("fill outbound event queue");
    let _command_tx = spawn_coding_runner(
        state.clone(),
        CodingAttemptStore::new(app_paths.clone()),
        event_tx,
        attempt.clone(),
    )
    .expect("spawn coding runner");

    let event_id = "event_http_abort_pending_socket";
    let waiter = register_plan_amendment_socket_write(event_id).expect("delivery waiter");
    let flush_entered = Arc::new(AtomicBool::new(false));
    let writer_flush_entered = Arc::clone(&flush_entered);
    let event = plan_amendment_event(event_id);
    let writer = tokio::spawn(async move {
        let mut socket = PendingSocketSink {
            flush_entered: writer_flush_entered,
        };
        send_coding_event(&mut socket, &event).await
    });
    tokio::time::timeout(Duration::from_millis(250), async {
        while !flush_entered.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("socket writer must enter pending flush");

    let response = send_attempt_request(&state, &attempt, Method::POST, "/abort").await;

    assert_eq!(response.status(), StatusCode::OK);
    writer.abort();
    assert!(
        writer
            .await
            .expect_err("writer must be cancelled")
            .is_cancelled()
    );
    let error = tokio::time::timeout(Duration::from_millis(250), waiter.wait())
        .await
        .expect("cancelled writer must settle delivery acknowledgement")
        .expect_err("HTTP abort must not acknowledge a pending socket write");
    assert!(
        error
            .to_string()
            .contains("plan_amendment_socket_write_failed")
    );
    assert_eq!(
        CodingAttemptStore::new(app_paths.clone())
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("aborted attempt")
            .status,
        CodingAttemptStatus::Aborted
    );
    assert_issue_lease_released(&app_paths, &attempt);
    drop(tmp);
}

#[tokio::test]
async fn http_abort_completes_after_runner_panics() {
    let (tmp, app_paths, state, attempt) = seed_http_attempt_with_lease();
    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    let (event_tx, _event_rx) = mpsc::channel(1);
    let panic_entered = spawn_coding_runner_panicking_after_registration(
        state.clone(),
        CodingAttemptStore::new(app_paths.clone()),
        event_tx,
        attempt.clone(),
    );
    panic_entered.await.expect("runner panic probe");

    let response = send_attempt_request(&state, &attempt, Method::POST, "/abort").await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(state.coding_runs.runner_count(&attempt_key), 0);
    assert_eq!(
        CodingAttemptStore::new(app_paths.clone())
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("aborted attempt")
            .status,
        CodingAttemptStatus::Aborted
    );
    assert_issue_lease_released(&app_paths, &attempt);
    drop(tmp);
}

#[tokio::test]
async fn http_delete_completes_when_command_receiver_is_closed() {
    let (tmp, app_paths, state, attempt) = seed_http_attempt_with_lease();
    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    let (command_tx, command_rx) = mpsc::channel(1);
    state
        .coding_runs
        .insert_cancellable(&attempt_key, command_tx)
        .expect("closed receiver registration");
    drop(command_rx);

    let response = send_attempt_request(&state, &attempt, Method::DELETE, "").await;

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(state.coding_runs.runner_count(&attempt_key), 0);
    assert!(
        CodingAttemptStore::new(app_paths.clone())
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .is_err(),
        "delete must remove attempt after closed receiver cleanup"
    );
    assert_issue_lease_released(&app_paths, &attempt);
    drop(tmp);
}

fn seed_http_attempt_with_lease() -> (
    TempDir,
    crate::product::app_paths::ProductAppPaths,
    WebAppState,
    CodingExecutionAttempt,
) {
    let (tmp, app_paths, mut attempt) = seed_compiled_work_item_fixture();
    attempt.scope = CodingAttemptScope::WorkItem;
    attempt.work_item_group_id = None;
    attempt.current_work_item_id = None;
    attempt.worktree_path = None;
    attempt.status = CodingAttemptStatus::Running;
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    coding_store
        .save_coding_attempt(&attempt)
        .expect("persist HTTP attempt fixture");

    let repository_path = tmp.path().join("repository");
    fs::create_dir_all(&repository_path).expect("repository directory");
    assert!(
        Command::new("git")
            .args(["init", "--initial-branch=main"])
            .arg(&repository_path)
            .status()
            .expect("git init")
            .success(),
        "git init must succeed"
    );
    let repository = RepositoryStore::new(app_paths.clone())
        .create(CreateRepositoryInput {
            project_id: attempt.project_id.clone(),
            name: "repository".to_string(),
            path: repository_path,
            default_policy_preset: None,
            default_provider_mode: Some("fake".to_string()),
            idempotency_key: "runner-cleanup-repository".to_string(),
        })
        .expect("repository record");
    assert_eq!(repository.id, "repository_0001");

    let lifecycle = LifecycleStore::new(app_paths.clone());
    lifecycle
        .upsert_issue_shared_worktree(UpsertIssueSharedWorktreeInput {
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            repository_id: repository.id,
            branch_name: attempt.branch_name.clone(),
            worktree_path: tmp.path().join("missing-shared-worktree"),
            base_branch: attempt.base_branch.clone(),
        })
        .expect("shared worktree");
    let lease = lifecycle
        .try_acquire_issue_worktree_lock(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.work_item_id,
            &attempt.id,
        )
        .expect("attempt worktree lease");
    assert!(lease.acquired);

    let state = WebAppState::new(
        tmp.path().to_path_buf(),
        WebRuntime::new_fake(tmp.path().to_path_buf()),
    );
    (tmp, app_paths, state, attempt)
}

async fn send_attempt_request(
    state: &WebAppState,
    attempt: &CodingExecutionAttempt,
    method: Method,
    suffix: &str,
) -> axum::response::Response {
    let uri = format!(
        "/api/projects/{}/issues/{}/coding-attempts/{}{}",
        attempt.project_id, attempt.issue_id, attempt.id, suffix
    );
    tokio::time::timeout(
        Duration::from_millis(250),
        build_web_router(state.clone()).oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .expect("attempt request"),
        ),
    )
    .await
    .expect("HTTP attempt mutation must complete without runner backpressure")
    .expect("HTTP attempt response")
}

fn assert_issue_lease_released(
    app_paths: &crate::product::app_paths::ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) {
    let shared = LifecycleStore::new(app_paths.clone())
        .get_issue_shared_worktree(&attempt.project_id, &attempt.issue_id)
        .expect("shared worktree lookup");
    // DELETE 路径下若该 issue 无其他 attempt 记录，会一并清理 shared-worktree.json
    // （spec `harden-coding-attempt-deletion`：条件清理 shared-worktree）。此时记录
    // 不存在视为已彻底释放。abort 路径仅释放 lock_owner、保留 json，仍按原断言校验。
    match shared {
        None => {}
        Some(record) => {
            assert!(record.current_active_work_item_id.is_none());
            assert!(record.current_lock_owner_id.is_none());
        }
    }
}

fn plan_amendment_event(event_id: &str) -> CodingWsOutMessage {
    CodingWsOutMessage::PlanAmendmentUpdated {
        event_id: event_id.to_string(),
        amendment: Box::new(PlanAmendmentManifest {
            id: "plan_amendment_http_abort".to_string(),
            repair_request_id: "plan_repair_request_http_abort".to_string(),
            previous_plan_revision_id: "plan_revision_0001".to_string(),
            new_plan_revision_id: "plan_revision_0002".to_string(),
            revised_work_items: BTreeMap::new(),
            superseded_revisions: Vec::new(),
            dependency_graph_changes: Vec::new(),
            contract_deltas: Vec::new(),
            unaffected_units: Vec::new(),
            revalidation_required_units: Vec::new(),
            stale_units: Vec::new(),
            replacement_units: BTreeMap::new(),
            resume_target: AmendmentResumeTarget {
                logical_work_item_id: "work_item_http_abort".to_string(),
                mode: AmendmentResumeMode::Reexecute,
            },
            created_at: "2026-07-21T00:00:00Z".to_string(),
        }),
    }
}
