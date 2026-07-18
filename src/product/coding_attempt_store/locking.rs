use std::fs::{File, OpenOptions};
use std::io::ErrorKind;
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};

use crate::product::json_store::ProductStoreError;

pub(super) fn with_exclusive_lock<T>(
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
        lock_file_exclusive(&file, &lock_path)?;
        Ok(Self { file })
    }
}

impl Drop for ExclusiveFileLock {
    fn drop(&mut self) {
        unlock_file(&self.file);
    }
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
fn unlock_file(_file: &File) {}
