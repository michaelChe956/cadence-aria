use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

use tokio::sync::oneshot;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum TesterToolCommitTestPoint {
    BeforeProviderSend,
    AfterProviderSend,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TesterToolCommitPauseKey {
    root: PathBuf,
    point: TesterToolCommitTestPoint,
}

struct TesterToolCommitPause {
    registration_id: u64,
    reached_tx: oneshot::Sender<()>,
    resume_rx: oneshot::Receiver<()>,
}

pub(crate) struct TesterToolCommitPauseGuard {
    key: TesterToolCommitPauseKey,
    registration_id: u64,
}

static NEXT_REGISTRATION_ID: AtomicU64 = AtomicU64::new(1);
static PAUSES: OnceLock<Mutex<HashMap<TesterToolCommitPauseKey, TesterToolCommitPause>>> =
    OnceLock::new();

fn pauses() -> &'static Mutex<HashMap<TesterToolCommitPauseKey, TesterToolCommitPause>> {
    PAUSES.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn register_tester_tool_commit_pause(
    root: &Path,
    point: TesterToolCommitTestPoint,
) -> (
    TesterToolCommitPauseGuard,
    oneshot::Receiver<()>,
    oneshot::Sender<()>,
) {
    let key = TesterToolCommitPauseKey {
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
            TesterToolCommitPause {
                registration_id,
                reached_tx,
                resume_rx,
            },
        );
    assert!(
        previous.is_none(),
        "tester tool commit pause already registered"
    );
    (
        TesterToolCommitPauseGuard {
            key,
            registration_id,
        },
        reached_rx,
        resume_tx,
    )
}

pub(super) async fn pause_tester_tool_commit_if_configured(
    root: &Path,
    point: TesterToolCommitTestPoint,
) {
    let key = TesterToolCommitPauseKey {
        root: root.to_path_buf(),
        point,
    };
    let pause = pauses()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&key);
    if let Some(pause) = pause {
        let _ = pause.reached_tx.send(());
        let _ = pause.resume_rx.await;
    }
}

impl Drop for TesterToolCommitPauseGuard {
    fn drop(&mut self) {
        let mut pauses = pauses()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if pauses
            .get(&self.key)
            .is_some_and(|pause| pause.registration_id == self.registration_id)
        {
            pauses.remove(&self.key);
        }
    }
}
