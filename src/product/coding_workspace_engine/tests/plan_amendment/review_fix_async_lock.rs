use std::time::Duration;

use super::*;
use crate::product::coding_attempt_store::locking::ExclusiveFileLock;

#[tokio::test(flavor = "current_thread")]
async fn coding_amendment_arbitration_contention_does_not_block_current_thread_runtime() {
    let fixture = amendment_fixture().await;
    let lock_target = fixture
        .store
        .attempt_dir(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .join("amendment-application-arbitration");
    let guard = ExclusiveFileLock::acquire(&lock_target).unwrap();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release = std::thread::spawn(move || {
        let released_by_runtime = release_rx.recv_timeout(Duration::from_secs(2)).is_ok();
        drop(guard);
        released_by_runtime
    });
    let runtime_release = tokio::spawn(async move {
        let _ = release_tx.send(());
    });

    fixture
        .engine
        .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
        .await
        .unwrap();
    runtime_release.await.unwrap();

    assert!(
        release.join().unwrap(),
        "Tokio current-thread worker was blocked until the watchdog released the flock"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn coding_amendment_arbitration_waiters_exceeding_workers_still_make_progress() {
    let fixture = amendment_fixture().await;
    let lock_target = fixture
        .store
        .attempt_dir(
            &fixture.attempt.project_id,
            &fixture.attempt.issue_id,
            &fixture.attempt.id,
        )
        .join("amendment-application-arbitration");
    let guard = ExclusiveFileLock::acquire(&lock_target).unwrap();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release = std::thread::spawn(move || {
        let released_by_runtime = release_rx.recv_timeout(Duration::from_secs(2)).is_ok();
        drop(guard);
        released_by_runtime
    });
    let (started_tx, mut started_rx) = mpsc::channel(6);
    let mut event_consumers = Vec::new();
    let mut waiters = Vec::new();
    for _ in 0..6 {
        let attempt = fixture.attempt.clone();
        let manifest = fixture.manifest.clone();
        let store = fixture.store.clone();
        let (event_tx, mut event_rx) = mpsc::channel(8);
        event_consumers.push(tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                crate::web::coding_ws_handler::delivery_ack::confirm_plan_amendment_socket_write(
                    &event,
                );
            }
        }));
        let started_tx = started_tx.clone();
        waiters.push(tokio::spawn(async move {
            started_tx.send(()).await.unwrap();
            CodingWorkspaceEngine::new(store, GitWorkspaceService::new(), event_tx)
                .apply_plan_amendment(&attempt, &manifest)
                .await
        }));
    }
    drop(started_tx);
    for _ in 0..6 {
        tokio::time::timeout(Duration::from_secs(3), started_rx.recv())
            .await
            .expect("arbitration waiter could not start while workers were available")
            .expect("arbitration waiter start channel closed");
    }
    release_tx
        .send(())
        .expect("flock holder watchdog released before async waiters made progress");

    for waiter in waiters {
        tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .expect("arbitration waiter deadlocked")
            .unwrap()
            .unwrap();
    }
    for consumer in event_consumers {
        consumer.abort();
    }

    assert!(
        release.join().unwrap(),
        "all Tokio workers were blocked until the watchdog released the flock"
    );
}
