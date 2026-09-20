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

/// Resolve the git directory paths that must additionally be bound
/// read-write inside a writable-root sandbox for `git add`/`git commit` to
/// work from the authorized root (F-17). A standard repository keeps its
/// `.git` inside the root — already covered by the rw root bind, so nothing
/// extra is returned; a linked worktree (`.git` pointing at
/// `<repo>/.git/worktrees/<name>`) keeps both its git dir and common dir
/// outside the root, and the common dir — an ancestor of the git dir — is
/// returned so a single bind covers both. Non-git directories resolve to an
/// empty list.
///
/// Safety note: a sandboxed coder can only rewrite the `.git` pointer to a
/// location it can itself write — which is inside the authorized root and
/// therefore filtered out — or to an already-valid git dir such as the main
/// repository's, which this bind is meant to expose anyway; `rev-parse`
/// fails on anything else, so the extra binds cannot be aimed at arbitrary
/// host paths.
pub fn resolve_writable_git_paths(root: &Path) -> Vec<PathBuf> {
    let Some(git) = resolve_trusted_binary("git") else {
        return Vec::new();
    };
    let Ok(output) = std::process::Command::new(git)
        .arg("-C")
        .arg(root)
        .args(["rev-parse", "--absolute-git-dir", "--git-common-dir"])
        .output()
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new(); // not a git repository
    }
    let mut outside: Vec<PathBuf> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let path = Path::new(line);
            let absolute = if path.is_absolute() {
                path.to_path_buf()
            } else {
                root.join(path)
            };
            absolute.canonicalize().unwrap_or(absolute)
        })
        .filter(|path| !path.starts_with(root))
        .collect();
    outside.sort();
    outside.dedup();
    // The git dir of a linked worktree lives beneath its common dir, so the
    // common dir bind alone covers both; drop paths covered by another.
    outside
        .iter()
        .filter(|path| {
            !outside
                .iter()
                .any(|other| other != *path && path.starts_with(other))
        })
        .cloned()
        .collect()
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
/// execute `binary argv...` inside a sandbox: read-only bind of the whole
/// host filesystem, no network, limited empty `/tmp`, no new privileges,
/// cleared environment with an explicit allowlist, and an anchored verified
/// cwd. The authorized root is bound read-only by default; the coding
/// (Executor) role passes `writable_root` so its worktree — whose contract
/// includes the TDD write path and commit responsibility (F-17) — is bound
/// read-write instead, together with any `writable_binds` paths resolved
/// outside it (the git dir of a linked worktree). Everything else outside
/// the authorized root stays read-only in both modes. The extra writable
/// mounts are passed positionally and consumed exactly once at this single
/// call site; grouping them into a struct would add indirection for no
/// reuse.
#[allow(clippy::too_many_arguments)]
pub fn build_bwrap_args(
    root: &Path,
    cwd: &Path,
    cwd_fd: Option<RawFd>,
    writable_root: bool,
    writable_binds: &[PathBuf],
    env: &BTreeMap<String, String>,
    binary: &Path,
    argv: &[String],
) -> Vec<OsString> {
    let root_str = root.as_os_str().to_str().expect("utf8 root");
    // The root mount MUST come after `--ro-bind / /`: bubblewrap applies
    // mounts in argv order and a later mount on an ancestor path (/) shadows
    // earlier mounts beneath it, so a rw root bind placed first would be
    // silently covered by the read-only host root (verified live).
    let root_flag = if writable_root { "--bind" } else { "--ro-bind" };
    let mut command = Vec::<OsString>::new();
    for value in [
        "--ro-bind",
        "/",
        "/",
        root_flag,
        root_str,
        root_str,
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
        "--unshare-net",
        "--unshare-pid",
        "--die-with-parent",
        // `--no-new-privs` is deliberately absent: bubblewrap 0.11 removed
        // the option (it is always enforced) and rejects it with
        // "Unknown option", which made every auto-mode terminal exit 1.
        "--clearenv",
    ] {
        command.push(OsString::from(value));
    }
    // Extra read-write mounts for the coding role (F-17): the git dirs of a
    // linked worktree live outside the authorized root (`<repo>/.git/...`),
    // so the rw root bind alone cannot make `git add`/`git commit` work —
    // they are resolved by the caller and bound here at their own paths,
    // after the read-only host root for the same shadowing reason. A path
    // beneath /tmp also regains visibility lost to the private tmpfs.
    for path in writable_binds {
        let bind = path.as_os_str().to_str().expect("utf8 git path");
        for value in ["--bind", bind, bind] {
            command.push(OsString::from(value));
        }
    }
    match cwd_fd {
        Some(fd) => {
            // Bind the openat-anchored cwd directory fd under the /tmp tmpfs
            // (`--bind-fd FD DEST`, read-only variant for read-only roots):
            // the root filesystem is ro-bound, so bwrap cannot create the
            // /work mountpoint there. (The previous `--dir FD /work` form
            // passed an FD to an option that takes none and made bwrap abort
            // with exit 1.) The cwd anchor follows the root mount mode — a
            // read-only anchor inside a writable-root sandbox would still
            // kill `git add`/`commit` in the anchored cwd.
            command.push(OsString::from(if writable_root {
                "--bind-fd"
            } else {
                "--ro-bind-fd"
            }));
            command.push(OsString::from(fd.to_string()));
            command.push(OsString::from("/tmp/work"));
            command.push(OsString::from("--chdir"));
            command.push(OsString::from("/tmp/work"));
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
            false,
            &[],
            &env,
            &PathBuf::from("/usr/bin/cat"),
            &["file".to_string()],
        );
        let text = argv
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        // Host root stays read-only and the authorized root mounts AFTER it
        // (a later ancestor mount would shadow an earlier root bind).
        let ro_host = text
            .iter()
            .position(|arg| arg == "--ro-bind")
            .expect("--ro-bind present");
        assert_eq!(text[ro_host + 1], "/");
        assert_eq!(text[ro_host + 2], "/");
        let ro_root = text
            .iter()
            .skip(ro_host + 3)
            .position(|arg| arg == "--ro-bind")
            .map(|offset| offset + ro_host + 3)
            .expect("root ro-bind follows the host root bind");
        assert_eq!(text[ro_root + 1], "/tmp/root");
        assert_eq!(text[ro_root + 2], "/tmp/root");
        assert!(text.contains(&"--unshare-net".to_string()));
        assert!(text.contains(&"--tmpfs".to_string()));
        assert!(text.contains(&"--clearenv".to_string()));
        // The cwd anchor is bound with `--ro-bind-fd FD DEST` and executed
        // from /work. `--no-new-privs` is intentionally absent: bubblewrap
        // 0.11 removed the option (always enforced) and rejects it.
        let fd_index = text
            .iter()
            .position(|arg| arg == "--ro-bind-fd")
            .expect("--ro-bind-fd present");
        assert_eq!(text[fd_index + 1], "42");
        assert_eq!(text[fd_index + 2], "/tmp/work");
        assert!(text.contains(&"--chdir".to_string()));
        assert_eq!(text.last().map(String::as_str), Some("file"));
    }

    /// F-17:coder(Executor)契约面——授权根以 rw bind 进入沙箱,且必须
    /// 挂在只读宿主根之后(后挂祖先会遮蔽先前子挂载);cwd 锚定同步 rw。
    /// 宿主根与其余隔离旗标保持不变。
    #[test]
    fn bwrap_args_writable_root_binds_root_rw_after_read_only_host() {
        let root = PathBuf::from("/tmp/root");
        let cwd = PathBuf::from("/tmp/root/work");
        let mut env = BTreeMap::new();
        env.insert("PATH".to_string(), trusted_path_env());
        let argv = build_bwrap_args(
            &root,
            &cwd,
            Some(42),
            true,
            &[],
            &env,
            &PathBuf::from("/usr/bin/git"),
            &["status".to_string()],
        );
        let text = argv
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        // Writable root: `--bind root root`, never a ro root bind.
        let bind = text
            .iter()
            .position(|arg| arg == "--bind")
            .expect("--bind present");
        assert_eq!(text[bind + 1], "/tmp/root");
        assert_eq!(text[bind + 2], "/tmp/root");
        // The rw root bind must come after the read-only host root bind.
        let ro_host = text
            .iter()
            .position(|arg| arg == "--ro-bind")
            .expect("host --ro-bind still present");
        assert_eq!(text[ro_host + 1], "/");
        assert_eq!(text[ro_host + 2], "/");
        assert!(bind > ro_host, "rw root bind must follow --ro-bind / /");
        assert_eq!(
            text.iter().filter(|arg| *arg == "--ro-bind").count(),
            1,
            "exactly one ro bind: the host root"
        );
        // The cwd anchor follows the root mount mode.
        let fd_index = text
            .iter()
            .position(|arg| arg == "--bind-fd")
            .expect("--bind-fd present");
        assert_eq!(text[fd_index + 1], "42");
        assert_eq!(text[fd_index + 2], "/tmp/work");
        assert!(!text.contains(&"--ro-bind-fd".to_string()));
        // Isolation boundary unchanged.
        for flag in [
            "--unshare-net",
            "--unshare-pid",
            "--die-with-parent",
            "--clearenv",
        ] {
            assert!(text.contains(&flag.to_string()), "{flag} missing");
        }
        assert_eq!(text.last().map(String::as_str), Some("status"));
    }

    /// fix round 1(F-17/P1):linked worktree 的 gitdir/commondir 在授权根外,
    /// 由 `writable_binds` 以 rw bind 挂进沙箱(root bind 之后、同样在只读
    /// 宿主根之后)。
    #[test]
    fn bwrap_args_bind_extra_writable_git_paths_after_ro_host() {
        let root = PathBuf::from("/tmp/root");
        let cwd = PathBuf::from("/tmp/root/work");
        let git_common = PathBuf::from("/repo/.git");
        let mut env = BTreeMap::new();
        env.insert("PATH".to_string(), trusted_path_env());
        let argv = build_bwrap_args(
            &root,
            &cwd,
            Some(42),
            true,
            &[git_common.clone()],
            &env,
            &PathBuf::from("/usr/bin/git"),
            &["status".to_string()],
        );
        let text = argv
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        // Exactly two rw binds: the root and the extra git path, both after
        // the read-only host root.
        let binds: Vec<usize> = text
            .iter()
            .enumerate()
            .filter(|(_, arg)| *arg == "--bind")
            .map(|(index, _)| index)
            .collect();
        assert_eq!(binds.len(), 2, "{text:?}");
        let ro_host = text
            .iter()
            .position(|arg| arg == "--ro-bind")
            .expect("host --ro-bind");
        assert_eq!(text[ro_host + 1], "/");
        for bind in &binds {
            assert!(bind > &ro_host, "rw binds must follow --ro-bind / /");
        }
        assert_eq!(text[binds[0] + 1], "/tmp/root");
        assert_eq!(text[binds[1] + 1], "/repo/.git");
        assert_eq!(text[binds[1] + 2], "/repo/.git");
        // Isolation boundary unchanged.
        assert!(text.contains(&"--unshare-net".to_string()));
        assert!(text.contains(&"--unshare-pid".to_string()));
    }

    /// resolve_writable_git_paths 三形态(真机 git):非 git 目录空、标准仓
    /// 空(gitdir 在根内,由 rw root bind 覆盖)、linked worktree 返回根外
    /// commondir 一条(gitdir 是其后代,一条 bind 覆盖两者)。
    #[test]
    fn resolve_writable_git_paths_covers_linked_worktree_plain_and_non_git() {
        let base = tempfile::tempdir().expect("workspace");
        let base = base.path().canonicalize().expect("canonical base");

        // Non-git directory: nothing extra to write.
        let plain = base.join("plain");
        std::fs::create_dir_all(&plain).expect("mkdir plain");
        assert!(
            resolve_writable_git_paths(&plain).is_empty(),
            "non-git root needs no extra binds"
        );

        // Standard repository: the git dir lives inside the root.
        let main = base.join("main");
        host_git(&["init", "-q", "main"], &base);
        host_git(
            &[
                "-c",
                "user.name=f17",
                "-c",
                "user.email=f17@example.com",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "base",
            ],
            &main,
        );
        assert!(
            resolve_writable_git_paths(&main).is_empty(),
            "plain repo gitdir is covered by the rw root bind"
        );

        // Linked worktree: only the out-of-root common dir is returned.
        host_git(&["worktree", "add", "../wt", "-b", "f17wt"], &main);
        let wt = base.join("wt");
        let wt = wt.canonicalize().expect("canonical worktree");
        assert_eq!(
            resolve_writable_git_paths(&wt),
            vec![main.join(".git").canonicalize().expect("commondir")],
        );
    }

    fn host_git(args: &[&str], cwd: &Path) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .status()
            .expect("host git");
        assert!(status.success(), "host git {args:?} failed");
    }
}
