//! `fs/read_text_file` / `fs/write_text_file` implementations.
//!
//! All paths are resolved beneath the authorized root with `openat` +
//! `O_NOFOLLOW` atomic semantics. New-file creation validates every parent
//! directory on the walk, so an out-of-root parent or a symlink anywhere in
//! the chain is rejected before the write.

use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use super::sandbox::{open_read_no_follow, open_write_no_follow};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsError {
    OutOfRoot(String),
    SymlinkOrTraversal(String),
    NotFound(String),
    NotUtf8(String),
    PermissionDenied(String),
    Io(String),
}

impl std::fmt::Display for FsError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FsError::OutOfRoot(detail) => write!(
                formatter,
                "fs path is outside the authorized root: {detail}"
            ),
            FsError::SymlinkOrTraversal(detail) => write!(
                formatter,
                "fs path traverses a symlink or escapes: {detail}"
            ),
            FsError::NotFound(detail) => write!(formatter, "fs path not found: {detail}"),
            FsError::NotUtf8(detail) => write!(formatter, "fs file is not valid UTF-8: {detail}"),
            FsError::PermissionDenied(detail) => {
                write!(formatter, "fs permission denied: {detail}")
            }
            FsError::Io(detail) => write!(formatter, "fs io error: {detail}"),
        }
    }
}

impl std::error::Error for FsError {}

/// Verify the path is a relative path that stays inside the authorized root
/// lexically (absolute paths and `..` are rejected before any syscall).
fn validate_relative(_root: &Path, path: &str) -> Result<PathBuf, FsError> {
    let rel = Path::new(path);
    for component in rel.components() {
        match component {
            Component::ParentDir => {
                return Err(FsError::SymlinkOrTraversal(path.to_string()));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(FsError::OutOfRoot(path.to_string()));
            }
            Component::CurDir | Component::Normal(_) => {}
        }
    }
    Ok(rel.to_path_buf())
}

pub fn read_text_file(root: &Path, path: &str) -> Result<String, FsError> {
    let rel = validate_relative(root, path)?;
    let mut file = open_read_no_follow(root, &rel).map_err(map_open_error)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| FsError::Io(error.to_string()))?;
    String::from_utf8(bytes).map_err(|error| FsError::NotUtf8(error.utf8_error().to_string()))
}

pub fn write_text_file(root: &Path, path: &str, content: &str) -> Result<(), FsError> {
    let rel = validate_relative(root, path)?;
    let mut file = open_write_no_follow(root, &rel).map_err(map_open_error)?;
    file.write_all(content.as_bytes())
        .map_err(|error| FsError::Io(error.to_string()))?;
    Ok(())
}

fn map_open_error(error: std::io::Error) -> FsError {
    match error.raw_os_error() {
        Some(libc::ELOOP) => FsError::SymlinkOrTraversal(error.to_string()),
        Some(libc::ENOENT) => FsError::NotFound(error.to_string()),
        Some(libc::EACCES) | Some(libc::EPERM) => FsError::PermissionDenied(error.to_string()),
        _ => match error.kind() {
            std::io::ErrorKind::PermissionDenied => FsError::PermissionDenied(error.to_string()),
            std::io::ErrorKind::NotFound => FsError::NotFound(error.to_string()),
            _ => FsError::Io(error.to_string()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes_within_root() {
        let root = tempfile::tempdir().expect("root");
        write_text_file(root.path(), "a.txt", "hello").expect("write");
        assert_eq!(read_text_file(root.path(), "a.txt").expect("read"), "hello");

        // New file in an existing subdirectory.
        std::fs::create_dir(root.path().join("sub")).expect("mkdir");
        write_text_file(root.path(), "sub/b.txt", "nested").expect("write nested");
        assert_eq!(
            read_text_file(root.path(), "sub/b.txt").expect("read nested"),
            "nested"
        );
    }

    #[test]
    fn rejects_absolute_and_traversal_paths() {
        let root = tempfile::tempdir().expect("root");
        assert!(matches!(
            read_text_file(root.path(), "/etc/passwd"),
            Err(FsError::OutOfRoot(_))
        ));
        assert!(matches!(
            read_text_file(root.path(), "../outside"),
            Err(FsError::SymlinkOrTraversal(_)) | Err(FsError::NotFound(_))
        ));
        assert!(matches!(
            write_text_file(root.path(), "../outside.txt", "x"),
            Err(FsError::SymlinkOrTraversal(_)) | Err(FsError::NotFound(_))
        ));
    }

    #[test]
    fn rejects_symlink_escape_and_internal_symlink() {
        let root = tempfile::tempdir().expect("root");
        let outside = tempfile::tempdir().expect("outside");
        std::fs::write(outside.path().join("secret.txt"), b"secret").expect("secret");
        std::os::unix::fs::symlink(
            outside.path().join("secret.txt"),
            root.path().join("leak.txt"),
        )
        .expect("symlink");
        assert!(matches!(
            read_text_file(root.path(), "leak.txt"),
            Err(FsError::SymlinkOrTraversal(_))
        ));
        // Symlink pointing inside the root is also rejected (no-follow).
        std::fs::write(root.path().join("real.txt"), b"real").expect("real");
        std::os::unix::fs::symlink("real.txt", root.path().join("link.txt")).expect("symlink");
        assert!(matches!(
            read_text_file(root.path(), "link.txt"),
            Err(FsError::SymlinkOrTraversal(_))
        ));
        assert!(matches!(
            write_text_file(root.path(), "link.txt", "overwrite"),
            Err(FsError::SymlinkOrTraversal(_))
        ));
    }

    #[test]
    fn rejects_new_file_parent_out_of_root_or_missing() {
        let root = tempfile::tempdir().expect("root");
        assert!(matches!(
            write_text_file(root.path(), "missing/child.txt", "x"),
            Err(FsError::NotFound(_))
        ));
    }

    #[test]
    fn rejects_non_utf8_file() {
        let root = tempfile::tempdir().expect("root");
        std::fs::write(root.path().join("bin.dat"), [0xff, 0xfe, 0x00]).expect("write binary");
        assert!(matches!(
            read_text_file(root.path(), "bin.dat"),
            Err(FsError::NotUtf8(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_permission_denied() {
        use std::os::unix::fs::PermissionsExt;
        let root = tempfile::tempdir().expect("root");
        std::fs::write(root.path().join("secret.txt"), b"secret").expect("write");
        std::fs::set_permissions(
            root.path().join("secret.txt"),
            std::fs::Permissions::from_mode(0o000),
        )
        .expect("chmod");
        if unsafe { libc::geteuid() } == 0 {
            // Root bypasses mode bits; the traversal/utf8/symlink cases above
            // still cover the safety surface.
            return;
        }
        assert!(matches!(
            read_text_file(root.path(), "secret.txt"),
            Err(FsError::PermissionDenied(_))
        ));
    }

    #[test]
    fn write_replaces_existing_file_contents() {
        let root = tempfile::tempdir().expect("root");
        write_text_file(root.path(), "f.txt", "first").expect("write");
        write_text_file(root.path(), "f.txt", "second").expect("overwrite");
        assert_eq!(
            read_text_file(root.path(), "f.txt").expect("read"),
            "second"
        );
    }
}
