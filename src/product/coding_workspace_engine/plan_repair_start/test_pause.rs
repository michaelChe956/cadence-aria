use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock, mpsc};
use std::time::Duration;

#[derive(Clone)]
struct PlanRepairStartSnapshotRequestPause {
    root: PathBuf,
    finding_id: String,
    reached_tx: mpsc::SyncSender<()>,
    release: Arc<(Mutex<bool>, Condvar)>,
}

pub(crate) struct PlanRepairStartSnapshotRequestPauseGuard {
    root: PathBuf,
    finding_id: String,
    reached_rx: mpsc::Receiver<()>,
    release: Arc<(Mutex<bool>, Condvar)>,
}

static PLAN_REPAIR_START_SNAPSHOT_REQUEST_PAUSE: OnceLock<
    Mutex<Option<PlanRepairStartSnapshotRequestPause>>,
> = OnceLock::new();

pub(crate) fn register_plan_repair_start_snapshot_request_pause(
    root: PathBuf,
    finding_id: impl Into<String>,
) -> PlanRepairStartSnapshotRequestPauseGuard {
    let finding_id = finding_id.into();
    let (reached_tx, reached_rx) = mpsc::sync_channel(1);
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let pause = PlanRepairStartSnapshotRequestPause {
        root: root.clone(),
        finding_id: finding_id.clone(),
        reached_tx,
        release: release.clone(),
    };
    let previous = snapshot_request_pause()
        .lock()
        .expect("plan repair start snapshot/request pause lock")
        .replace(pause);
    assert!(
        previous.is_none(),
        "plan repair start pause already registered"
    );
    PlanRepairStartSnapshotRequestPauseGuard {
        root,
        finding_id,
        reached_rx,
        release,
    }
}

impl PlanRepairStartSnapshotRequestPauseGuard {
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

impl Drop for PlanRepairStartSnapshotRequestPauseGuard {
    fn drop(&mut self) {
        self.release();
        let mut registered = snapshot_request_pause()
            .lock()
            .expect("plan repair start snapshot/request pause lock");
        if registered
            .as_ref()
            .is_some_and(|pause| pause.root == self.root && pause.finding_id == self.finding_id)
        {
            registered.take();
        }
    }
}

pub(super) fn maybe_pause_plan_repair_start_snapshot_request_boundary(
    root: &Path,
    finding_id: &str,
) {
    let pause = snapshot_request_pause()
        .lock()
        .expect("plan repair start snapshot/request pause lock")
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

fn snapshot_request_pause() -> &'static Mutex<Option<PlanRepairStartSnapshotRequestPause>> {
    PLAN_REPAIR_START_SNAPSHOT_REQUEST_PAUSE.get_or_init(|| Mutex::new(None))
}
