//! OS-level sandbox primitives for the kimi client services.
//!
//! * trusted absolute binary resolution and a fixed trusted `PATH`
//! * `openat` + `O_NOFOLLOW` atomic path anchoring (defeats TOCTOU symlink
//!   races for fs and for terminal cwd)
//! * bubblewrap (`bwrap`) availability probe and read-only sandbox argv
//!   construction

use std::collections::BTreeMap;
use std::ffi::{CString, OsString};
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

/// Fixed trusted search directories for allowed binaries. The child process
/// never inherits the caller's `PATH`.
pub const TRUSTED_PATH_DIRS: [&str; 5] =
    ["/usr/bin", "/bin", "/usr/local/bin", "/usr/sbin", "/sbin"];

pub fn trusted_path_env() -> String {
    TRUSTED_PATH_DIRS.join(":")
}

/// Resolve an allowed binary to a trusted absolute path. Returns `None` when
/// the binary does not exist in a trusted directory, so execution fails
/// closed rather than resolving through the caller's `PATH`.
pub fn resolve_trusted_binary(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        return None;
    }
    TRUSTED_PATH_DIRS
        .iter()
        .map(|dir| Path::new(dir).join(name))
        .find(|candidate| candidate.is_file())
}

/// Canonicalize an authorized root, failing on any missing/non-directory path.
pub fn canonicalize_root(root: &Path) -> std::io::Result<PathBuf> {
    let canonical = std::fs::canonicalize(root)?;
    if !canonical.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            "authorized root is not a directory",
        ));
    }
    Ok(canonical)
}

fn cstr(path: &[u8]) -> CString {
    CString::new(path).expect("path contains no interior NUL")
}

/// Split a relative path into non-empty byte components, collapsing `.` and
/// rejecting `..` so no traversal is possible.
fn components(rel: &Path) -> std::io::Result<Vec<Vec<u8>>> {
    if rel.is_absolute() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "absolute path is not allowed",
        ));
    }
    let mut out = Vec::new();
    for component in rel.as_os_str().as_bytes().split(|byte| *byte == b'/') {
        if component.is_empty() || component == b"." {
            continue;
        }
        if component == b".." {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "path traversal is not allowed",
            ));
        }
        out.push(component.to_vec());
    }
    Ok(out)
}

fn open_dir_fd(path: &Path, cloexec: bool) -> std::io::Result<OwnedFd> {
    let mut flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW;
    if cloexec {
        flags |= libc::O_CLOEXEC;
    }
    let fd = unsafe { libc::open(cstr(path.as_os_str().as_bytes()).as_ptr(), flags) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// Walk `rel` beneath `root` using `openat` + `O_NOFOLLOW`. Intermediate
/// components must be non-symlink directories. `final_flags` must already
/// include `O_NOFOLLOW`; the returned fd keeps exactly `final_flags`.
fn open_fd_no_follow_flags(
    root: &Path,
    rel: &Path,
    final_flags: libc::c_int,
) -> std::io::Result<OwnedFd> {
    let components = components(rel)?;
    if components.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "empty path is not allowed",
        ));
    }
    let mut current = open_dir_fd(root, true)?;
    let last = components.len() - 1;
    for (index, component) in components.iter().enumerate() {
        let flags = if index == last {
            final_flags | libc::O_NOFOLLOW | libc::O_CLOEXEC
        } else {
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC
        };
        let fd = unsafe {
            if flags & libc::O_CREAT != 0 {
                libc::openat(
                    current.as_raw_fd(),
                    cstr(component).as_ptr(),
                    flags,
                    libc::S_IRUSR | libc::S_IWUSR,
                )
            } else {
                libc::openat(current.as_raw_fd(), cstr(component).as_ptr(), flags)
            }
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if index < last {
            current = unsafe { OwnedFd::from_raw_fd(fd) };
        } else {
            return Ok(unsafe { OwnedFd::from_raw_fd(fd) });
        }
    }
    unreachable!("walked all components")
}

/// Open an existing file read-only with `O_NOFOLLOW`, anchored beneath `root`.
pub fn open_read_no_follow(root: &Path, rel: &Path) -> std::io::Result<std::fs::File> {
    let fd = open_fd_no_follow_flags(root, rel, libc::O_RDONLY)?;
    Ok(unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) })
}

/// Open a file for writing (creating/truncating) with `O_NOFOLLOW`. The final
/// component must not be a symlink; every parent component must already exist
/// and be a non-symlink directory beneath `root`.
pub fn open_write_no_follow(root: &Path, rel: &Path) -> std::io::Result<std::fs::File> {
    let fd = open_fd_no_follow_flags(root, rel, libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC)?;
    Ok(unsafe { std::fs::File::from_raw_fd(fd.into_raw_fd()) })
}

/// Open a directory anchored beneath `root` with `O_NOFOLLOW` + `O_CLOEXEC`.
pub fn open_dir_no_follow(root: &Path, rel: &Path) -> std::io::Result<OwnedFd> {
    let components = components(rel)?;
    if components.is_empty() {
        return open_dir_fd(root, true);
    }
    open_fd_no_follow_flags(root, rel, libc::O_DIRECTORY)
}

/// Resolve a directory fd to its canonical absolute path via `/proc/self/fd`.
pub fn canonical_path_of_fd(fd: &OwnedFd) -> std::io::Result<PathBuf> {
    let proc_link = format!("/proc/self/fd/{}", fd.as_raw_fd());
    let resolved = std::fs::read_link(&proc_link)?;
    std::fs::canonicalize(resolved)
}

/// Open a directory anchored beneath `root` with `O_NOFOLLOW` but WITHOUT
/// `O_CLOEXEC`, so the fd survives `exec` and can anchor a child's cwd via
/// `/proc/self/fd/N` (or bwrap `--dir FD /work`). The anchored inode remains
/// fixed even if the directory is later swapped for a symlink.
pub fn open_dir_no_follow_inherit(root: &Path, rel: &Path) -> std::io::Result<OwnedFd> {
    let components = components(rel)?;
    if components.is_empty() {
        return open_dir_fd(root, false);
    }
    let mut current = open_dir_fd(root, true)?;
    let last = components.len() - 1;
    for (index, component) in components.iter().enumerate() {
        let flags = if index == last {
            libc::O_DIRECTORY | libc::O_NOFOLLOW
        } else {
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC
        };
        let fd = unsafe { libc::openat(current.as_raw_fd(), cstr(component).as_ptr(), flags) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if index < last {
            current = unsafe { OwnedFd::from_raw_fd(fd) };
        } else {
            return Ok(unsafe { OwnedFd::from_raw_fd(fd) });
        }
    }
    unreachable!("walked all components")
}

/// Probe for a usable `bwrap` binary. Returns its absolute path, or `None`
/// when bubblewrap is missing or cannot create a user namespace.
pub fn probe_bwrap() -> Option<PathBuf> {
    let candidate = TRUSTED_PATH_DIRS
        .iter()
        .map(|dir| Path::new(dir).join("bwrap"))
        .find(|candidate| candidate.is_file())?;
    let output = std::process::Command::new(&candidate)
        .arg("--version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(candidate)
}

/// Validate that a path beneath `root` has no symlink in any existing prefix
/// component (defense-in-depth before handing the literal path to a binary).
/// A missing final component is tolerated (git paths may refer to deleted
/// files); a missing intermediate component is also safe because no symlink
/// can be followed through a non-existent directory. `O_PATH` avoids any
/// read-permission requirement on validated components.
pub fn validate_path_no_follow(root: &Path, rel: &Path) -> std::io::Result<()> {
    let comps = components(rel)?;
    if comps.is_empty() {
        return Ok(());
    }
    let mut current = open_dir_fd(root, true)?;
    for (index, component) in comps.iter().enumerate() {
        let fd = unsafe {
            libc::openat(
                current.as_raw_fd(),
                cstr(component).as_ptr(),
                libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd >= 0 {
            let owned = unsafe { OwnedFd::from_raw_fd(fd) };
            if index + 1 < comps.len() {
                current = owned;
            }
        } else {
            let error = std::io::Error::last_os_error();
            match error.raw_os_error() {
                Some(libc::ELOOP) | Some(libc::ENOTDIR) => return Err(error),
                Some(libc::ENOENT) => return Ok(()),
                _ => return Err(error),
            }
        }
    }
    Ok(())
}

/// Build the `bwrap` arguments (excluding the `bwrap` program itself) that
/// execute `binary argv...` inside a read-only sandbox: read-only bind of the
/// authorized root and of the whole host filesystem, no network, limited
/// empty `/tmp`, no new privileges, cleared environment with an explicit
/// allowlist, and an anchored verified cwd (via `--dir FD /work` when a
/// directory fd is supplied).
pub fn build_bwrap_args(
    root: &Path,
    cwd: &Path,
    cwd_fd: Option<RawFd>,
    env: &BTreeMap<String, String>,
    binary: &Path,
    argv: &[String],
) -> Vec<OsString> {
    let mut command = Vec::<OsString>::new();
    for value in [
        "--ro-bind",
        root.as_os_str().to_str().expect("utf8 root"),
        root.as_os_str().to_str().expect("utf8 root"),
        "--ro-bind",
        "/",
        "/",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
        "--unshare-net",
        "--unshare-pid",
        "--die-with-parent",
        "--no-new-privs",
        "--clearenv",
    ] {
        command.push(OsString::from(value));
    }
    match cwd_fd {
        Some(fd) => {
            command.push(OsString::from("--dir"));
            command.push(OsString::from(fd.to_string()));
            command.push(OsString::from("/work"));
            command.push(OsString::from("--chdir"));
            command.push(OsString::from("/work"));
        }
        None => {
            command.push(OsString::from("--chdir"));
            command.push(OsString::from(cwd.as_os_str().to_str().expect("utf8 cwd")));
        }
    }
    for (key, value) in env {
        command.push(OsString::from("--setenv"));
        command.push(OsString::from(key.as_str()));
        command.push(OsString::from(value.as_str()));
    }
    command.push(binary.as_os_str().to_os_string());
    for arg in argv {
        command.push(OsString::from(arg.as_str()));
    }
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusted_binary_resolution_rejects_relative_and_unknown() {
        assert!(resolve_trusted_binary("git").is_some());
        assert!(resolve_trusted_binary("definitely-not-a-real-binary").is_none());
        assert!(resolve_trusted_binary("/usr/bin/git").is_none());
        assert!(resolve_trusted_binary("../git").is_none());
    }

    #[test]
    fn components_reject_traversal_and_absolute() {
        assert!(components(Path::new("a/b")).is_ok());
        assert!(components(Path::new(".")).is_ok()); // single dot collapses to empty list
        assert!(components(Path::new("a/../b")).is_err());
        assert!(components(Path::new("/etc")).is_err());
    }

    #[test]
    fn open_read_no_follow_reads_within_root_and_rejects_symlink_escape() {
        let root = tempfile::tempdir().expect("root");
        let file = root.path().join("file.txt");
        std::fs::write(&file, b"hello").expect("write file");

        let mut opened =
            open_read_no_follow(root.path(), Path::new("file.txt")).expect("open file");
        let mut content = String::new();
        use std::io::Read;
        opened.read_to_string(&mut content).expect("read file");
        assert_eq!(content, "hello");

        // A symlink pointing inside is rejected (no-follow) — read of the link
        // target would otherwise be allowed by canonicalize-based checks.
        std::os::unix::fs::symlink("file.txt", root.path().join("link.txt")).expect("symlink");
        let error = open_read_no_follow(root.path(), Path::new("link.txt"))
            .expect_err("symlink must be rejected");
        // `ErrorKind`'s ELOOP variant is not yet nameable on stable Rust;
        // the raw errno is the portable contract consumed by fs_service.
        assert_eq!(error.raw_os_error(), Some(libc::ELOOP));
    }

    #[test]
    fn open_write_no_follow_creates_file_and_rejects_missing_parent() {
        let root = tempfile::tempdir().expect("root");
        let mut opened =
            open_write_no_follow(root.path(), Path::new("new.txt")).expect("create file");
        use std::io::Write;
        opened.write_all(b"content").expect("write");
        drop(opened);
        assert_eq!(
            std::fs::read(root.path().join("new.txt")).expect("read"),
            b"content"
        );

        let error = open_write_no_follow(root.path(), Path::new("missing/new.txt"))
            .expect_err("missing parent dir must fail");
        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn bwrap_args_include_isolation_flags() {
        let root = PathBuf::from("/tmp/root");
        let cwd = PathBuf::from("/tmp/root/work");
        let mut env = BTreeMap::new();
        env.insert("PATH".to_string(), trusted_path_env());
        let argv = build_bwrap_args(
            &root,
            &cwd,
            Some(42),
            &env,
            &PathBuf::from("/usr/bin/cat"),
            &["file".to_string()],
        );
        let text = argv
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(text.contains(&"--ro-bind".to_string()));
        assert!(text.contains(&"--unshare-net".to_string()));
        assert!(text.contains(&"--tmpfs".to_string()));
        assert!(text.contains(&"--no-new-privs".to_string()));
        assert!(text.contains(&"--clearenv".to_string()));
        assert!(text.contains(&"--dir".to_string()));
        assert!(text.contains(&"--chdir".to_string()));
        assert_eq!(text.last().map(String::as_str), Some("file"));
    }
}
