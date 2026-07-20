use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock, mpsc};
use std::time::Duration;

#[derive(Clone)]
struct PlanRepairStartConsistencyPause {
    root: PathBuf,
    finding_id: String,
    reached_tx: mpsc::SyncSender<()>,
    release: Arc<(Mutex<bool>, Condvar)>,
}

pub(crate) struct PlanRepairStartConsistencyPauseGuard {
    root: PathBuf,
    finding_id: String,
    reached_rx: mpsc::Receiver<()>,
    release: Arc<(Mutex<bool>, Condvar)>,
}

static PLAN_REPAIR_START_CONSISTENCY_PAUSE: OnceLock<
    Mutex<Option<PlanRepairStartConsistencyPause>>,
> = OnceLock::new();

pub(crate) fn register_plan_repair_start_consistency_pause(
    root: PathBuf,
    finding_id: impl Into<String>,
) -> PlanRepairStartConsistencyPauseGuard {
    let finding_id = finding_id.into();
    let (reached_tx, reached_rx) = mpsc::sync_channel(1);
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let pause = PlanRepairStartConsistencyPause {
        root: root.clone(),
        finding_id: finding_id.clone(),
        reached_tx,
        release: release.clone(),
    };
    let previous = consistency_pause()
        .lock()
        .expect("plan repair start consistency pause lock")
        .replace(pause);
    assert!(
        previous.is_none(),
        "plan repair start pause already registered"
    );
    PlanRepairStartConsistencyPauseGuard {
        root,
        finding_id,
        reached_rx,
        release,
    }
}

impl PlanRepairStartConsistencyPauseGuard {
    pub(crate) fn wait_until_reached(&self, timeout: Duration) -> bool {
        self.reached_rx.recv_timeout(timeout).is_ok()
    }

    pub(crate) fn release(&self) {
        let (released, wake) = &*self.release;
        *released
            .lock()
            .expect("plan repair start pause release lock") = true;
        wake.notify_all();
    }
}

impl Drop for PlanRepairStartConsistencyPauseGuard {
    fn drop(&mut self) {
        self.release();
        let mut registered = consistency_pause()
            .lock()
            .expect("plan repair start consistency pause lock");
        if registered
            .as_ref()
            .is_some_and(|pause| pause.root == self.root && pause.finding_id == self.finding_id)
        {
            registered.take();
        }
    }
}

pub(super) fn maybe_pause_plan_repair_start_consistency_read(root: &Path, finding_id: &str) {
    let pause = consistency_pause()
        .lock()
        .expect("plan repair start consistency pause lock")
        .as_ref()
        .filter(|pause| pause.root == root && pause.finding_id == finding_id)
        .cloned();
    let Some(pause) = pause else {
        return;
    };
    let _ = pause.reached_tx.try_send(());
    let (released, wake) = &*pause.release;
    let mut released = released
        .lock()
        .expect("plan repair start pause release lock");
    while !*released {
        released = wake
            .wait(released)
            .expect("plan repair start pause release wait");
    }
}

fn consistency_pause() -> &'static Mutex<Option<PlanRepairStartConsistencyPause>> {
    PLAN_REPAIR_START_CONSISTENCY_PAUSE.get_or_init(|| Mutex::new(None))
}
