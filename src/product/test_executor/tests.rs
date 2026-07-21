use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Duration;

use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::{TestExecutorError, write_test_artifacts};

struct ArtifactWritePause {
    root: PathBuf,
    entered: Option<oneshot::Sender<PathBuf>>,
    release: Arc<(Mutex<bool>, Condvar)>,
}

pub(super) struct ArtifactWritePauseGuard {
    entered: oneshot::Receiver<PathBuf>,
    release: Arc<(Mutex<bool>, Condvar)>,
}

static ARTIFACT_WRITE_PAUSE: OnceLock<Mutex<Option<ArtifactWritePause>>> = OnceLock::new();

fn artifact_write_pause() -> &'static Mutex<Option<ArtifactWritePause>> {
    ARTIFACT_WRITE_PAUSE.get_or_init(|| Mutex::new(None))
}

fn register_artifact_write_pause(root: &Path) -> ArtifactWritePauseGuard {
    let (entered_tx, entered) = oneshot::channel();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    *artifact_write_pause()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(ArtifactWritePause {
        root: root.to_path_buf(),
        entered: Some(entered_tx),
        release: Arc::clone(&release),
    });
    ArtifactWritePauseGuard { entered, release }
}

pub(super) fn pause_artifact_write_if_configured(path: &Path) {
    let pause = {
        let mut registered = artifact_write_pause()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if registered
            .as_ref()
            .is_some_and(|pause| path.starts_with(&pause.root))
        {
            registered.take()
        } else {
            None
        }
    };
    let Some(mut pause) = pause else {
        return;
    };
    if let Some(entered) = pause.entered.take() {
        let _ = entered.send(path.to_path_buf());
    }
    let (released, wake) = &*pause.release;
    let mut released = released
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    while !*released {
        released = wake
            .wait(released)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
}

impl ArtifactWritePauseGuard {
    async fn entered(self) -> (PathBuf, Arc<(Mutex<bool>, Condvar)>) {
        let path = tokio::time::timeout(Duration::from_millis(500), self.entered)
            .await
            .expect("artifact write must enter pause")
            .expect("artifact write pause sender");
        (path, self.release)
    }
}

fn release_artifact_write(release: &Arc<(Mutex<bool>, Condvar)>) {
    let (released, wake) = &**release;
    *released
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
    wake.notify_all();
}

#[tokio::test]
async fn cancellation_during_settled_write_never_publishes_or_recreates_artifacts() {
    let root = tempfile::tempdir().expect("tempdir");
    let output = root.path().join("test-output");
    let stdout = output.join("tool_call_0001.stdout.log");
    let stderr = output.join("tool_call_0001.stderr.log");
    let pause = register_artifact_write_pause(&output);
    let cancellation = CancellationToken::new();
    let write = tokio::spawn({
        let stdout = stdout.clone();
        let stderr = stderr.clone();
        let cancellation = cancellation.clone();
        async move { write_test_artifacts(&stdout, &stderr, b"stdout", b"stderr", &cancellation).await }
    });
    let (writing_path, release) = pause.entered().await;
    assert_ne!(
        writing_path, stdout,
        "writes must settle in a temporary file"
    );

    cancellation.cancel();
    release_artifact_write(&release);
    let result = write.await.expect("artifact write task");

    assert!(matches!(result, Err(TestExecutorError::Cancelled)));
    assert!(!stdout.exists());
    assert!(!stderr.exists());
    let leftovers = std::fs::read_dir(&output)
        .expect("artifact output directory")
        .collect::<Result<Vec<_>, _>>()
        .expect("artifact output entries");
    assert!(
        leftovers.is_empty(),
        "temporary artifacts leaked: {leftovers:?}"
    );
}
