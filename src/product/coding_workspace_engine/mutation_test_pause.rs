use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use tokio::sync::oneshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum CodingMutationTestPoint {
    GroupCompletionRunning,
    GroupCompletionCompletedRetry,
    ProviderFailure,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CodingMutationPauseKey {
    root: PathBuf,
    point: CodingMutationTestPoint,
}

struct CodingMutationPauseEntry {
    registration_id: u64,
    reached_tx: oneshot::Sender<()>,
    resume_rx: oneshot::Receiver<()>,
}

pub(crate) struct CodingMutationTestPauseGuard {
    key: CodingMutationPauseKey,
    registration_id: u64,
}

static NEXT_REGISTRATION_ID: AtomicU64 = AtomicU64::new(1);
static PAUSES: OnceLock<Mutex<HashMap<CodingMutationPauseKey, CodingMutationPauseEntry>>> =
    OnceLock::new();

fn pauses() -> &'static Mutex<HashMap<CodingMutationPauseKey, CodingMutationPauseEntry>> {
    PAUSES.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn register_coding_mutation_test_pause(
    root: &Path,
    point: CodingMutationTestPoint,
) -> (
    CodingMutationTestPauseGuard,
    oneshot::Receiver<()>,
    oneshot::Sender<()>,
) {
    let key = CodingMutationPauseKey {
        root: root.to_path_buf(),
        point,
    };
    let registration_id = NEXT_REGISTRATION_ID.fetch_add(1, Ordering::Relaxed);
    let (reached_tx, reached_rx) = oneshot::channel();
    let (resume_tx, resume_rx) = oneshot::channel();
    let previous = pauses()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(
            key.clone(),
            CodingMutationPauseEntry {
                registration_id,
                reached_tx,
                resume_rx,
            },
        );
    assert!(
        previous.is_none(),
        "coding mutation pause already registered"
    );
    (
        CodingMutationTestPauseGuard {
            key,
            registration_id,
        },
        reached_rx,
        resume_tx,
    )
}

pub(crate) async fn pause_coding_mutation_for_test(root: &Path, point: CodingMutationTestPoint) {
    let key = CodingMutationPauseKey {
        root: root.to_path_buf(),
        point,
    };
    let entry = pauses()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&key);
    if let Some(entry) = entry {
        let _ = entry.reached_tx.send(());
        let _ = entry.resume_rx.await;
    }
}

impl Drop for CodingMutationTestPauseGuard {
    fn drop(&mut self) {
        let mut pauses = pauses()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if pauses
            .get(&self.key)
            .is_some_and(|entry| entry.registration_id == self.registration_id)
        {
            pauses.remove(&self.key);
        }
    }
}
