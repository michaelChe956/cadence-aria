use std::fs::{File, OpenOptions};
use std::io::ErrorKind;
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(test)]
use std::sync::{Mutex, OnceLock, mpsc};

use crate::product::json_store::ProductStoreError;

pub(crate) fn with_exclusive_lock<T>(
    target_path: &Path,
    operation: impl FnOnce() -> Result<T, ProductStoreError>,
) -> Result<T, ProductStoreError> {
    let _lock = ExclusiveFileLock::acquire(target_path)?;
    operation()
}

/// Acquires the exact lock file supplied by the caller without deriving another
/// name. Migration locks are externally specified, so applying the historic
/// target-derived lock convention would lock a different file.
pub(crate) fn with_exact_exclusive_lock<T>(
    lock_path: &Path,
    operation: impl FnOnce() -> Result<T, ProductStoreError>,
) -> Result<T, ProductStoreError> {
    if let Some(parent) = lock_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            ProductStoreError::Io(format!("create {}: {error}", parent.display()))
        })?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .map_err(|error| {
            ProductStoreError::Io(format!("open lock {}: {error}", lock_path.display()))
        })?;
    lock_file_exclusive(&file, lock_path)?;
    let result = operation();
    unlock_file(&file);
    result
}

pub(crate) struct ExclusiveFileLock {
    file: File,
    canonical_lock_path: PathBuf,
}

impl ExclusiveFileLock {
    pub(crate) fn acquire(target_path: &Path) -> Result<Self, ProductStoreError> {
        let (file, lock_path) = open_lock_file(target_path)?;
        #[cfg(test)]
        notify_lock_attempt(&lock_path);
        lock_file_exclusive(&file, &lock_path)?;
        Self::from_locked_file(file, lock_path)
    }

    fn from_locked_file(file: File, lock_path: PathBuf) -> Result<Self, ProductStoreError> {
        let canonical_lock_path = canonical_path_identity(&lock_path)?;
        Ok(Self {
            file,
            canonical_lock_path,
        })
    }

    pub(crate) fn canonical_lock_path(&self) -> &Path {
        &self.canonical_lock_path
    }

    pub(crate) async fn acquire_async(target_path: &Path) -> Result<Self, ProductStoreError> {
        const LOCK_RETRY_BACKOFF: std::time::Duration = std::time::Duration::from_millis(10);

        let (file, lock_path) = open_lock_file(target_path)?;
        loop {
            #[cfg(test)]
            notify_lock_attempt(&lock_path);
            if try_lock_file_exclusive(&file, &lock_path)? {
                return Self::from_locked_file(file, lock_path);
            }
            tokio::time::sleep(LOCK_RETRY_BACKOFF).await;
        }
    }
}

fn open_lock_file(target_path: &Path) -> Result<(File, PathBuf), ProductStoreError> {
    let lock_path = lock_path_for(target_path);
    if let Some(parent) = lock_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            ProductStoreError::Io(format!("create {}: {error}", parent.display()))
        })?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| {
            ProductStoreError::Io(format!("open lock {}: {error}", lock_path.display()))
        })?;
    Ok((file, lock_path))
}

#[cfg(test)]
struct LockAttemptHookEntry {
    registration_id: u64,
    sender: mpsc::Sender<()>,
}

#[cfg(test)]
pub(crate) struct LockAttemptHookGuard {
    lock_path: PathBuf,
    registration_id: u64,
}

#[cfg(test)]
static LOCK_ATTEMPT_HOOKS: OnceLock<Mutex<HashMap<PathBuf, LockAttemptHookEntry>>> =
    OnceLock::new();

#[cfg(test)]
static NEXT_LOCK_ATTEMPT_HOOK_ID: AtomicU64 = AtomicU64::new(1);

#[cfg(test)]
fn lock_attempt_hooks() -> &'static Mutex<HashMap<PathBuf, LockAttemptHookEntry>> {
    LOCK_ATTEMPT_HOOKS.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(test)]
pub(crate) fn register_lock_attempt_hook(
    target_path: &Path,
) -> (LockAttemptHookGuard, mpsc::Receiver<()>) {
    let lock_path = std::fs::canonicalize(lock_path_for(target_path))
        .expect("held test lock path should be canonicalizable");
    let registration_id = NEXT_LOCK_ATTEMPT_HOOK_ID.fetch_add(1, Ordering::Relaxed);
    let (sender, receiver) = mpsc::channel();
    let mut hooks = lock_attempt_hooks()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert!(
        !hooks.contains_key(&lock_path),
        "lock attempt hook already registered for {}",
        lock_path.display()
    );
    hooks.insert(
        lock_path.clone(),
        LockAttemptHookEntry {
            registration_id,
            sender,
        },
    );
    (
        LockAttemptHookGuard {
            lock_path,
            registration_id,
        },
        receiver,
    )
}

#[cfg(test)]
fn notify_lock_attempt(lock_path: &Path) {
    let Ok(lock_path) = std::fs::canonicalize(lock_path) else {
        return;
    };
    let sender = {
        let hooks = lock_attempt_hooks()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        hooks.get(&lock_path).map(|entry| entry.sender.clone())
    };
    if let Some(sender) = sender {
        let _ = sender.send(());
    }
}

#[cfg(test)]
impl Drop for LockAttemptHookGuard {
    fn drop(&mut self) {
        let mut hooks = lock_attempt_hooks()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if hooks
            .get(&self.lock_path)
            .is_some_and(|entry| entry.registration_id == self.registration_id)
        {
            hooks.remove(&self.lock_path);
        }
    }
}

impl Drop for ExclusiveFileLock {
    fn drop(&mut self) {
        unlock_file(&self.file);
    }
}

pub(crate) fn canonical_path_identity(path: &Path) -> Result<PathBuf, ProductStoreError> {
    let mut current = path;
    let mut missing = Vec::new();
    while !current.exists() {
        let name = current.file_name().ok_or_else(|| {
            ProductStoreError::Io(format!("canonicalize path identity: {}", path.display()))
        })?;
        missing.push(name.to_os_string());
        current = current.parent().ok_or_else(|| {
            ProductStoreError::Io(format!("canonicalize path identity: {}", path.display()))
        })?;
    }
    let mut canonical = std::fs::canonicalize(current).map_err(|error| {
        ProductStoreError::Io(format!("canonicalize {}: {error}", current.display()))
    })?;
    for component in missing.into_iter().rev() {
        canonical.push(component);
    }
    Ok(canonical)
}

pub(crate) fn canonical_lock_path_identity(
    target_path: &Path,
) -> Result<PathBuf, ProductStoreError> {
    canonical_path_identity(&lock_path_for(target_path))
}

fn lock_path_for(target_path: &Path) -> PathBuf {
    let file_name = target_path
        .file_name()
        .map(|value| value.to_string_lossy())
        .unwrap_or_else(|| "coding-attempt-store".into());
    target_path.with_file_name(format!(".{file_name}.lock"))
}

#[cfg(unix)]
fn lock_file_exclusive(file: &File, lock_path: &Path) -> Result<(), ProductStoreError> {
    loop {
        // SAFETY: flock only reads the valid file descriptor and retains no Rust pointer.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != ErrorKind::Interrupted {
            return Err(ProductStoreError::Io(format!(
                "lock {}: {error}",
                lock_path.display()
            )));
        }
    }
}

#[cfg(unix)]
fn try_lock_file_exclusive(file: &File, lock_path: &Path) -> Result<bool, ProductStoreError> {
    // SAFETY: flock only reads the valid file descriptor and retains no Rust pointer.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::Interrupted) {
        return Ok(false);
    }
    Err(ProductStoreError::Io(format!(
        "try lock {}: {error}",
        lock_path.display()
    )))
}

#[cfg(unix)]
fn unlock_file(file: &File) {
    // SAFETY: flock only reads the valid file descriptor and retains no Rust pointer.
    let _ = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
}

#[cfg(not(unix))]
fn lock_file_exclusive(_file: &File, lock_path: &Path) -> Result<(), ProductStoreError> {
    Err(ProductStoreError::Io(format!(
        "file locking is unsupported on this platform: {}",
        lock_path.display()
    )))
}

#[cfg(not(unix))]
fn try_lock_file_exclusive(_file: &File, lock_path: &Path) -> Result<bool, ProductStoreError> {
    Err(ProductStoreError::Io(format!(
        "file locking is unsupported on this platform: {}",
        lock_path.display()
    )))
}

#[cfg(not(unix))]
fn unlock_file(_file: &File) {}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use tempfile::tempdir;

    use super::ExclusiveFileLock;

    #[test]
    fn cancelled_async_file_lock_contender_does_not_delay_runtime_shutdown() {
        let tmp = tempdir().expect("tempdir");
        let target = tmp.path().join("cancelled-contender-lock");
        let holder = ExclusiveFileLock::acquire(&target).expect("holder lock");
        let (_hook, attempted_rx) = super::register_lock_attempt_hook(&target);
        let contender_target = target.clone();
        let (abort_tx, abort_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let runtime_thread = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("current-thread runtime");
            runtime.block_on(async move {
                let contender = tokio::spawn(async move {
                    ExclusiveFileLock::acquire_async(&contender_target).await
                });
                abort_tx
                    .send(contender.abort_handle())
                    .expect("send contender abort handle");
                let _ = contender.await;
            });
            drop(runtime);
            let _ = finished_tx.send(());
        });

        attempted_rx
            .recv_timeout(Duration::from_millis(250))
            .expect("contender reached flock");
        abort_rx
            .recv_timeout(Duration::from_millis(250))
            .expect("contender abort handle")
            .abort();
        let finished_before_release = finished_rx.recv_timeout(Duration::from_millis(150)).is_ok();
        drop(holder);
        runtime_thread.join().expect("runtime thread");

        assert!(
            finished_before_release,
            "cancelled async contender left an unbounded blocking flock waiter"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn releasing_holder_does_not_resume_cancelled_lock_business() {
        let tmp = tempdir().expect("tempdir");
        let target = tmp.path().join("cancelled-business-lock");
        let holder = ExclusiveFileLock::acquire(&target).expect("holder lock");
        let acquired = Arc::new(AtomicBool::new(false));
        let contender_acquired = Arc::clone(&acquired);
        let contender_target = target.clone();
        let contender = tokio::spawn(async move {
            let _guard = ExclusiveFileLock::acquire_async(&contender_target)
                .await
                .expect("async contender");
            contender_acquired.store(true, Ordering::SeqCst);
        });
        tokio::time::sleep(Duration::from_millis(25)).await;
        contender.abort();
        let _ = contender.await;

        drop(holder);
        tokio::time::sleep(Duration::from_millis(50)).await;

        assert!(
            !acquired.load(Ordering::SeqCst),
            "cancelled contender resumed business after holder release"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn async_file_lock_does_not_block_current_thread_runtime() {
        let tmp = tempdir().expect("tempdir");
        let target = tmp.path().join("current-thread-lock");
        let holder = ExclusiveFileLock::acquire(&target).expect("holder lock");
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            drop(holder);
        });
        let started = Instant::now();
        let heartbeat = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            started.elapsed()
        });
        let contender_target = target.clone();
        let contender = tokio::spawn(async move {
            ExclusiveFileLock::acquire_async(&contender_target)
                .await
                .expect("async contender")
        });

        let heartbeat_elapsed = heartbeat.await.expect("heartbeat task");
        assert!(
            heartbeat_elapsed < Duration::from_millis(100),
            "blocking flock delayed current-thread heartbeat by {heartbeat_elapsed:?}"
        );
        let acquired = tokio::time::timeout(Duration::from_secs(1), contender)
            .await
            .expect("async contender timeout")
            .expect("async contender task");
        drop(acquired);
        release.join().expect("holder release thread");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn async_file_lock_contenders_do_not_exhaust_tokio_workers() {
        let tmp = tempdir().expect("tempdir");
        let target = tmp.path().join("multi-contender-lock");
        let holder = ExclusiveFileLock::acquire(&target).expect("holder lock");
        let (_hook, attempts_rx) = super::register_lock_attempt_hook(&target);
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            drop(holder);
        });
        let started = Instant::now();
        let heartbeat = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            started.elapsed()
        });
        let acquisitions = Arc::new(AtomicUsize::new(0));
        let contenders = (0..8)
            .map(|_| {
                let target = target.clone();
                let acquisitions = Arc::clone(&acquisitions);
                tokio::spawn(async move {
                    let guard = ExclusiveFileLock::acquire_async(&target)
                        .await
                        .expect("async contender");
                    acquisitions.fetch_add(1, Ordering::SeqCst);
                    tokio::task::yield_now().await;
                    drop(guard);
                })
            })
            .collect::<Vec<_>>();

        let heartbeat_elapsed = heartbeat.await.expect("heartbeat task");
        assert!(
            heartbeat_elapsed < Duration::from_millis(100),
            "file-lock contenders exhausted Tokio workers for {heartbeat_elapsed:?}"
        );
        tokio::time::timeout(Duration::from_secs(2), async {
            for contender in contenders {
                contender.await.expect("contender task");
            }
        })
        .await
        .expect("all async contenders must finish");
        release.join().expect("holder release thread");
        assert_eq!(acquisitions.load(Ordering::SeqCst), 8);
        let attempt_count = attempts_rx.try_iter().count();
        assert!(attempt_count >= 8, "each contender must attempt the lock");
        assert!(
            attempt_count < 400,
            "lock retry loop appears busy: {attempt_count} attempts"
        );
    }
}
