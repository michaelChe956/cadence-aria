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

/// Verify the path stays inside the authorized root lexically before any
/// syscall. Absolute paths inside the root are converted to relative paths.
fn validate_relative(root: &Path, path: &str) -> Result<PathBuf, FsError> {
    let p = Path::new(path);
    if p.is_absolute() {
        let root_canonical = root
            .canonicalize()
            .map_err(|_| FsError::OutOfRoot(path.to_string()))?;
        if !p.starts_with(&root_canonical) {
            return Err(FsError::OutOfRoot(path.to_string()));
        }
        let rel = p
            .strip_prefix(&root_canonical)
            .map_err(|_| FsError::OutOfRoot(path.to_string()))?;
        for component in rel.components() {
            match component {
                Component::ParentDir => return Err(FsError::SymlinkOrTraversal(path.to_string())),
                Component::CurDir | Component::Normal(_) => {}
                Component::RootDir | Component::Prefix(_) => unreachable!(),
            }
        }
        return Ok(rel.to_path_buf());
    }

    for component in p.components() {
        match component {
            Component::ParentDir => return Err(FsError::SymlinkOrTraversal(path.to_string())),
            Component::RootDir | Component::Prefix(_) => {
                return Err(FsError::OutOfRoot(path.to_string()));
            }
            Component::CurDir | Component::Normal(_) => {}
        }
    }
    Ok(p.to_path_buf())
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

/// 基线树文本读取（REQ-PIB-02 通道层路由）：`git -C <repo> show
/// refs/heads/<branch>:<path>`——不 checkout、不触工作区（工作区脏值与
/// `.worktrees/` 兄弟件不可见，F-57 形态根除），参数化 argv 不经 shell。
/// 路径必须为树内相对路径（拒绝对路径/`..`/空）；树内不存在（或引用不可
/// 解析）→ NotFound；非 utf8 → NotUtf8。
pub(super) fn read_baseline_text_file(
    repo_path: &Path,
    branch: &str,
    path: &str,
) -> Result<String, FsError> {
    let relative = validate_tree_relative_path(path)?;
    let reference = format!("refs/heads/{branch}");
    let tree_spec = format!("{reference}:{}", relative.display());
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .arg("show")
        .arg(&tree_spec)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|error| FsError::Io(format!("git show {tree_spec}: {error}")))?;
    if !output.status.success() {
        return Err(FsError::NotFound(format!(
            "not found in baseline tree {reference}: {path}"
        )));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| FsError::NotUtf8(format!("baseline tree file {path}: {error}")))
}

/// 基线会话读取路径的 cwd 锚定归一（F-58）：kimi 原生 Read 在 ACP 模式下
/// 经 `fs/read_text_file` 委托 host（capabilities `fs.readTextFile`），且
/// 总发送按其 cwd 解析后的**绝对路径**（现场：issue_0002 session_0007 的
/// `AGENTS.md`/`CLAUDE.md` 全部以 `/…/<repo>/AGENTS.md` 形态被
/// `validate_tree_relative_path` 以对路径拒绝）。与 `validate_relative` 对
/// 根内绝对路径的容忍同构：绝对路径词法位于会话根（canonical）内 → 剥前缀
/// 得树内相对路径（worktree 检出与主树共享路径命名空间，树查找以会话根
/// 为锚）；相对路径严格透传；根外绝对路径、`..`、空一律拒绝（fail-closed
/// 不变：`.worktrees` 兄弟件、宿主任意路径、/tmp 均不可达）。
pub(super) fn baseline_tree_relative_path(root: &Path, path: &str) -> Result<PathBuf, FsError> {
    let p = Path::new(path);
    if !p.is_absolute() {
        return validate_tree_relative_path(path);
    }
    let root_canonical = root
        .canonicalize()
        .map_err(|_| FsError::OutOfRoot(path.to_string()))?;
    let rel = p
        .strip_prefix(&root_canonical)
        .map_err(|_| FsError::OutOfRoot(path.to_string()))?;
    for component in rel.components() {
        match component {
            Component::CurDir | Component::Normal(_) => {}
            Component::ParentDir => return Err(FsError::SymlinkOrTraversal(path.to_string())),
            Component::RootDir | Component::Prefix(_) => {
                return Err(FsError::OutOfRoot(path.to_string()));
            }
        }
    }
    if rel.as_os_str().is_empty() {
        return Err(FsError::OutOfRoot(path.to_string()));
    }
    Ok(rel.to_path_buf())
}

/// 基线树路径校验：仅接受树内相对路径。与 `validate_relative` 的根锚定
/// 校验不同——树读取无「授权根」概念，`..` 与对路径一律拒绝（git show 的
/// `<ref>:<path>` 语法本身不接受绝对路径，此处前置拒绝给出统一错误形态）。
fn validate_tree_relative_path(path: &str) -> Result<PathBuf, FsError> {
    let p = Path::new(path);
    if path.trim().is_empty() {
        return Err(FsError::OutOfRoot(path.to_string()));
    }
    if p.is_absolute() {
        return Err(FsError::OutOfRoot(path.to_string()));
    }
    for component in p.components() {
        match component {
            Component::ParentDir | Component::CurDir => {
                return Err(FsError::SymlinkOrTraversal(path.to_string()));
            }
            Component::Normal(_) => {}
            Component::RootDir | Component::Prefix(_) => {
                return Err(FsError::OutOfRoot(path.to_string()));
            }
        }
    }
    Ok(p.to_path_buf())
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
    fn absolute_path_inside_root_reads() {
        let root = tempfile::tempdir().expect("root");
        let file = root.path().join("a/b.txt");
        std::fs::create_dir_all(file.parent().expect("parent")).expect("mkdir");
        std::fs::write(&file, "inside").expect("write");

        assert_eq!(
            read_text_file(root.path(), file.to_str().expect("utf-8 path")).expect("read"),
            "inside"
        );
    }

    #[test]
    fn absolute_path_outside_root_rejected() {
        let root = tempfile::tempdir().expect("root");
        let outside = tempfile::NamedTempFile::new().expect("outside file");

        assert!(matches!(
            read_text_file(root.path(), outside.path().to_str().expect("utf-8 path")),
            Err(FsError::OutOfRoot(_))
        ));
    }

    #[test]
    fn absolute_path_with_parent_dir_rejected() {
        let root = tempfile::tempdir().expect("root");
        let directory = root.path().join("a");
        std::fs::create_dir(&directory).expect("mkdir");
        std::fs::write(root.path().join("b.txt"), "inside").expect("write");
        let path_with_parent = format!("{}/../b.txt", directory.display());

        assert!(matches!(
            read_text_file(root.path(), &path_with_parent),
            Err(FsError::SymlinkOrTraversal(_))
        ));
    }

    #[test]
    fn prefix_confusion_rejected() {
        let parent = tempfile::tempdir().expect("parent");
        let root = parent.path().join("xxx_root");
        let evil = parent.path().join("xxx_root_evil");
        std::fs::create_dir_all(&root).expect("root mkdir");
        std::fs::create_dir_all(&evil).expect("evil mkdir");
        let evil_file = evil.join("secret.txt");
        std::fs::write(&evil_file, "secret").expect("write");

        assert!(matches!(
            read_text_file(&root, evil_file.to_str().expect("utf-8 path")),
            Err(FsError::OutOfRoot(_))
        ));
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
    fn relative_path_regression() {
        let root = tempfile::tempdir().expect("root");
        std::fs::write(root.path().join("relative.txt"), "relative").expect("write");

        assert_eq!(
            read_text_file(root.path(), "./relative.txt").expect("read"),
            "relative"
        );
        assert!(matches!(
            read_text_file(root.path(), "../outside"),
            Err(FsError::SymlinkOrTraversal(_))
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
    #[test]
    fn baseline_tree_relative_path_strips_absolute_paths_inside_root() {
        let root = tempfile::tempdir().expect("root");
        std::fs::create_dir(root.path().join("sub")).expect("mkdir");

        assert_eq!(
            baseline_tree_relative_path(
                root.path(),
                &root.path().join("AGENTS.md").to_string_lossy()
            )
            .expect("root-level absolute path"),
            PathBuf::from("AGENTS.md")
        );
        assert_eq!(
            baseline_tree_relative_path(
                root.path(),
                &root.path().join("sub/f.txt").to_string_lossy()
            )
            .expect("nested absolute path"),
            PathBuf::from("sub/f.txt")
        );
    }

    #[test]
    fn baseline_tree_relative_path_rejects_absolute_paths_outside_root() {
        let root = tempfile::tempdir().expect("root");
        assert!(matches!(
            baseline_tree_relative_path(root.path(), "/etc/passwd"),
            Err(FsError::OutOfRoot(_))
        ));
        assert!(matches!(
            baseline_tree_relative_path(root.path(), "/tmp/gomoku.pid"),
            Err(FsError::OutOfRoot(_))
        ));
        // Prefix confusion: `/root_evil` is not beneath `/root`.
        let parent = tempfile::tempdir().expect("parent");
        let root = parent.path().join("xxx_root");
        std::fs::create_dir_all(&root).expect("root mkdir");
        let evil = parent.path().join("xxx_root_evil/secret.txt");
        assert!(matches!(
            baseline_tree_relative_path(&root, &evil.to_string_lossy()),
            Err(FsError::OutOfRoot(_))
        ));
    }

    #[test]
    fn baseline_tree_relative_path_rejects_traversal_and_empty() {
        let root = tempfile::tempdir().expect("root");
        // Absolute path whose remainder escapes after prefix strip.
        let escaping = format!("{}/sub/../outside.txt", root.path().display());
        assert!(matches!(
            baseline_tree_relative_path(root.path(), &escaping),
            Err(FsError::SymlinkOrTraversal(_))
        ));
        // Relative `..` traversal and empty stay rejected (strict passthrough).
        assert!(matches!(
            baseline_tree_relative_path(root.path(), "../outside.txt"),
            Err(FsError::SymlinkOrTraversal(_))
        ));
        assert!(matches!(
            baseline_tree_relative_path(root.path(), "  "),
            Err(FsError::OutOfRoot(_))
        ));
        // Plain relative paths pass through untouched.
        assert_eq!(
            baseline_tree_relative_path(root.path(), "sub/f.txt").expect("relative"),
            PathBuf::from("sub/f.txt")
        );
    }
}
