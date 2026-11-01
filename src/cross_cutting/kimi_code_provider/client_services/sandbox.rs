//! fs path-safety primitives for the kimi client services.
//!
//! 批2 Task 6 后本模块只保留 fs `no-follow`/`openat` 原子路径工具(fs
//! read/write 锚定、terminal cwd 锚定与 canonical 化):bwrap 探测/argv、
//! 受信二进制解析与固定 PATH、git bind 面解析已随 Host 单路径一并删除。

use std::ffi::CString;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

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
                    // `libc::S_IRUSR`/`S_IWUSR` 的类型随平台而变（macOS 为
                    // u16），而 `openat` 的模式位是可变参数（`c_uint`）→ 显式
                    // 转宽，macOS 编译通过（Linux 上等价）。
                    libc::S_IRUSR as libc::c_uint | libc::S_IWUSR as libc::c_uint,
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
/// `/proc/self/fd/N`. The anchored inode remains fixed even if the directory
/// is later swapped for a symlink.
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
