use std::fs::{self, File, OpenOptions};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(test)]
use std::sync::{Mutex, OnceLock, mpsc};

#[cfg(unix)]
use std::os::fd::AsRawFd;

use serde::{Serialize, de::DeserializeOwned};

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::WorkItemPlanLineage;

mod amendment_publication;
mod candidate_package;
mod dependency;
mod handoff;
mod initial_publication;
mod paths;
mod plan;
mod presentation;
mod projection;
mod purge;
mod repair;
mod work_item;

pub use amendment_publication::PlanAmendmentPublicationIds;
#[cfg(test)]
pub(crate) use amendment_publication::{
    PlanAmendmentPublicationCheckpoint, register_plan_amendment_publication_failpoint,
};
#[cfg(test)]
pub(crate) use initial_publication::InitialPlanPublicationCheckpoint;
pub use initial_publication::{
    InitialPlanPublicationArtifacts, InitialPlanPublicationIds, InitialPlanPublicationJournal,
    InitialPlanPublicationPhase, InitialWorkItemPublicationArtifacts,
    InitialWorkItemPublicationIds,
};
pub use plan::ActiveAmendmentReleaseOutcome;
#[cfg(test)]
pub(crate) use repair::register_repair_request_status_failpoint;

#[derive(Debug, Clone)]
pub struct WorkItemRevisionStore {
    paths: ProductAppPaths,
}

impl WorkItemRevisionStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    fn ensure_plan_scope(
        &self,
        lineage: &WorkItemPlanLineage,
    ) -> Result<WorkItemPlanLineage, ProductStoreError> {
        validate_relative_id(&lineage.project_id)?;
        validate_relative_id(&lineage.issue_id)?;
        validate_relative_id(&lineage.id)?;
        let stored = self.get_plan_lineage(&lineage.project_id, &lineage.issue_id, &lineage.id)?;
        if stored.project_id != lineage.project_id
            || stored.issue_id != lineage.issue_id
            || stored.id != lineage.id
        {
            return Err(identity_mismatch("work_item_plan_lineage", &lineage.id));
        }
        Ok(stored)
    }
}

fn identity_mismatch(kind: &'static str, id: &str) -> ProductStoreError {
    ProductStoreError::IdentityMismatch {
        kind,
        id: id.to_string(),
    }
}

fn read_required_json<T: DeserializeOwned>(
    path: &Path,
    kind: &'static str,
    id: &str,
) -> Result<T, ProductStoreError> {
    if !path_exists(path)? {
        return Err(ProductStoreError::NotFound {
            kind,
            id: id.to_string(),
        });
    }
    read_json(path)
}

fn write_immutable<T>(
    path: &Path,
    kind: &'static str,
    id: &str,
    value: &T,
) -> Result<(), ProductStoreError>
where
    T: Serialize + DeserializeOwned + PartialEq,
{
    with_exclusive_lock(path, || {
        if path_exists(path)? {
            let existing: T = read_json(path)?;
            if existing == *value {
                return Ok(());
            }
            return Err(identity_mismatch(kind, id));
        }
        write_json(path, value)
    })
}

fn with_exclusive_lock<T>(
    target_path: &Path,
    operation: impl FnOnce() -> Result<T, ProductStoreError>,
) -> Result<T, ProductStoreError> {
    let _lock = ExclusiveFileLock::acquire(target_path)?;
    operation()
}

struct ExclusiveFileLock {
    file: File,
}

impl ExclusiveFileLock {
    fn acquire(target_path: &Path) -> Result<Self, ProductStoreError> {
        let lock_path = lock_path_for(target_path);
        if let Some(parent) = lock_path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|error| {
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
        #[cfg(test)]
        notify_lock_attempt(&lock_path);
        lock_file_exclusive(&file, &lock_path)?;
        Ok(Self { file })
    }
}

impl Drop for ExclusiveFileLock {
    fn drop(&mut self) {
        unlock_file(&self.file);
    }
}

#[cfg(test)]
struct LockAttemptHookEntry {
    registration_id: u64,
    sender: mpsc::Sender<()>,
}

#[cfg(test)]
struct LockAttemptHookGuard {
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
fn register_lock_attempt_hook(target_path: &Path) -> (LockAttemptHookGuard, mpsc::Receiver<()>) {
    let lock_path = fs::canonicalize(lock_path_for(target_path))
        .expect("held test lock path should be canonicalizable");
    let registration_id = NEXT_LOCK_ATTEMPT_HOOK_ID.fetch_add(1, Ordering::Relaxed);
    let (sender, receiver) = mpsc::channel();
    let mut hooks = lock_attempt_hooks()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if hooks.contains_key(&lock_path) {
        drop(hooks);
        panic!(
            "lock attempt hook already registered for {}",
            lock_path.display()
        );
    }
    hooks.insert(
        lock_path.clone(),
        LockAttemptHookEntry {
            registration_id,
            sender,
        },
    );
    drop(hooks);
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
    let Ok(lock_path) = fs::canonicalize(lock_path) else {
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

fn lock_path_for(target_path: &Path) -> PathBuf {
    let file_name = target_path
        .file_name()
        .map(|value| value.to_string_lossy())
        .unwrap_or_else(|| "work-item-revision".into());
    target_path.with_file_name(format!(".{file_name}.lock"))
}

#[cfg(unix)]
fn lock_file_exclusive(file: &File, lock_path: &Path) -> Result<(), ProductStoreError> {
    loop {
        // SAFETY: flock only reads the valid file descriptor and does not retain any Rust pointer.
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
fn unlock_file(file: &File) {
    // SAFETY: flock only reads the valid file descriptor and does not retain any Rust pointer.
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
fn unlock_file(_file: &File) {}

fn path_exists(path: &Path) -> Result<bool, ProductStoreError> {
    match fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(ProductStoreError::Io(format!(
            "metadata {}: {error}",
            path.display()
        ))),
    }
}

fn json_file_paths(path: &Path) -> Result<Vec<PathBuf>, ProductStoreError> {
    if !path_exists(path)? {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(path)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?
    {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", path.display()))
        })?;
        let entry_path = entry.path();
        if entry
            .file_type()
            .map_err(|error| {
                ProductStoreError::Io(format!("read {} entry type: {error}", entry_path.display()))
            })?
            .is_file()
            && entry_path.extension().and_then(|value| value.to_str()) == Some("json")
        {
            entries.push(entry_path);
        }
    }
    entries.sort();
    Ok(entries)
}

#[cfg(test)]
mod tests;
