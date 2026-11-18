//! 驾驶舱日志打包：受限文件系统访问层（计划 T1）。
//!
//! 一切访问都锚定在 [`Root::open`] 取得的根目录 fd 上，用 `openat`
//! 逐级打开：目录级 `O_DIRECTORY | O_NOFOLLOW`，文件级
//! `O_NOFOLLOW | O_NONBLOCK`。父级或末级符号链接、以及打开过程中被
//! 替换的路径都无法把遍历引出根之外；全程不使用 `/proc`。
//!
//! 相对路径只接受由普通名字组成的片段序列，根目录本身用空路径
//! `Path::new("")` 表示；绝对路径、`..`、`.` 一律 `InvalidInput`。
//!
//! [`Root::file`] 对**已打开的 fd** 校验：必须是普通文件（FIFO 等特
//! 殊文件以 `O_NONBLOCK` 打开后立即按类型拒绝，绝不阻塞）且硬链接数
//! 为 1。
//!
//! [`Root::list`] 按名字节序排序返回目录项并逐项扣减 `remaining`
//! 预算；返回后 `remaining == 0` 表示结果可能被预算截断，调用方应按
//! 「可能不完整」上报，而不是丢弃已取得的条目。

use std::ffi::OsString;
use std::fs::File;
use std::io;
use std::path::Path;

#[cfg(unix)]
use rustix::fs::{AtFlags, Dir, FileType, Mode, OFlags};
#[cfg(unix)]
use std::ffi::OsStr;
#[cfg(unix)]
use std::os::fd::OwnedFd;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt as _;
#[cfg(unix)]
use std::path::Component;

/// 目录项分类。符号链接、FIFO、套接字、设备等一律归入 `Other`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    /// 子目录。
    Dir,
    /// 普通文件（硬链接数要到 [`Root::file`] 打开时才校验）。
    File,
    /// 其它类型（符号链接、FIFO、套接字、设备等）。
    Other,
}

/// [`Root::list`] 返回的单个目录项。
#[derive(Debug, Clone)]
pub(super) struct Entry {
    /// 目录项名字（不含路径）。
    pub(super) name: OsString,
    /// 目录项类型。
    pub(super) kind: Kind,
}

/// 以某个目录为根的受限访问器。
///
/// 内部只保存根目录 fd，且仅作为 `openat` 的锚点（从不直接对它
/// `readdir`），可以安全地跨线程共享；每次 `list`/`file` 都重新打开
/// 自己的目录/文件 fd，用完由 RAII 关闭。
#[derive(Debug)]
pub(super) struct Root {
    #[cfg(unix)]
    fd: OwnedFd,
}

#[cfg(unix)]
impl Root {
    /// 打开根目录：从 `/` 起逐分量 `O_DIRECTORY | O_NOFOLLOW` 打开，
    /// 根本身或任何一级祖先是符号链接都会失败（ELOOP），调用方按
    /// 「跳过并记录原因」处理；相对路径与含 `..`/`.` 的根直接
    /// `InvalidInput`。macOS 仅把开头的系统别名 `/tmp`、`/var` 换成
    /// `/private/...` 的物理对应，不 canonicalize 其余分量。
    pub(super) fn open(root: &Path) -> io::Result<Self> {
        let comps = absolute_components(root)?;
        let names: Vec<&OsStr> = comps.iter().map(OsString::as_os_str).collect();
        let slash = rustix::fs::open(
            "/",
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        let fd = open_dir_chain(&slash, &names)?;
        Ok(Self { fd })
    }

    /// 列出 `relative` 目录（空路径即根目录本身）的目录项。
    ///
    /// - 每产出一条目录项把 `*remaining` 减一；`.`/`..` 以及在
    ///   readdir 与 stat 之间消失的项不消耗预算；出错提前返回时
    ///   `*remaining` 保持不变。
    /// - 结果按名字节序排序，稳定可界定；返回后 `*remaining == 0`
    ///   即视为可能被预算截断（即使恰好读完也按可能截断处理）。
    /// - readdir 中途出现 I/O 错误时返回 `Err`，不静默截断。
    pub(super) fn list(&self, relative: &Path, remaining: &mut usize) -> io::Result<Vec<Entry>> {
        let comps = relative_components(relative)?;
        let fd = open_dir_chain(&self.fd, &comps)?;
        let mut dir = Dir::new(fd)?;
        let mut out = Vec::new();
        while *remaining != 0 {
            let entry = match dir.read() {
                Some(Ok(entry)) => entry,
                Some(Err(err)) => return Err(err.into()),
                None => break,
            };
            let raw_name = entry.file_name().to_bytes();
            if raw_name == b"." || raw_name == b".." {
                continue;
            }
            let name = OsString::from_vec(raw_name.to_vec());
            let kind = match entry.file_type() {
                FileType::Directory => Some(Kind::Dir),
                FileType::RegularFile => Some(Kind::File),
                // 个别文件系统不给 d_type：用不跟随符号链接的 statat 补全类型
                FileType::Unknown => classify_unknown(&dir, name.as_os_str())?,
                _ => Some(Kind::Other),
            };
            let Some(kind) = kind else { continue };
            out.push(Entry { name, kind });
            *remaining -= 1;
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    /// 打开 `relative` 指向的普通文件（至少要有一级名字）。
    ///
    /// 父级目录逐级 `O_NOFOLLOW` 打开；末级同样 `O_NOFOLLOW`，并对打
    /// 开后的 fd 校验：必须是普通文件且硬链接数为 1，否则
    /// `InvalidData`。FIFO/设备以 `O_NONBLOCK` 打开后立即被类型检查
    /// 拒绝，不会阻塞在 open 上。
    pub(super) fn file(&self, relative: &Path) -> io::Result<File> {
        let comps = relative_components(relative)?;
        let Some((name, parents)) = comps.split_last() else {
            return Err(invalid_input("file 需要至少一级相对名"));
        };
        let dir = open_dir_chain(&self.fd, parents)?;
        let fd = rustix::fs::openat(
            &dir,
            *name,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )?;
        let file = File::from(fd);
        let meta = file.metadata()?;
        if !meta.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "日志打包只接受普通文件",
            ));
        }
        if meta.nlink() != 1 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "日志打包拒绝硬链接文件",
            ));
        }
        Ok(file)
    }
}

#[cfg(not(unix))]
impl Root {
    /// 非 Unix 平台没有可用的 fd 相对 `openat` 安全语义，显式失败。
    pub(super) fn open(root: &Path) -> io::Result<Self> {
        let _ = root;
        Err(unsupported())
    }

    pub(super) fn list(&self, relative: &Path, remaining: &mut usize) -> io::Result<Vec<Entry>> {
        let _ = (relative, remaining);
        Err(unsupported())
    }

    pub(super) fn file(&self, relative: &Path) -> io::Result<File> {
        let _ = relative;
        Err(unsupported())
    }
}

#[cfg(not(unix))]
fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "当前平台无法安全实现日志目录扫描",
    )
}

/// 相对路径只允许由普通名字组成；空路径（零个片段）表示根目录本身。
#[cfg(unix)]
fn relative_components(relative: &Path) -> io::Result<Vec<&OsStr>> {
    let mut comps = Vec::new();
    for comp in relative.components() {
        match comp {
            Component::Normal(name) => comps.push(name),
            _ => return Err(invalid_input("相对路径只能是普通名字序列")),
        }
    }
    Ok(comps)
}

/// 绝对根逐分量校验：拒绝相对路径及 `..`/`.` 等一切非普通分量；
/// macOS 仅把开头的系统别名 `/tmp`、`/var` 映射为物理对应
/// `/private/tmp`、`/private/var`，不 canonicalize 其余任何用户分量。
#[cfg(unix)]
fn absolute_components(root: &Path) -> io::Result<Vec<OsString>> {
    if !root.is_absolute() {
        return Err(invalid_input("根必须是绝对路径"));
    }
    let mut comps: Vec<OsString> = Vec::new();
    for comp in root.components() {
        match comp {
            Component::RootDir => {}
            Component::Normal(name) => comps.push(name.to_os_string()),
            _ => return Err(invalid_input("根路径只能由普通名字组成")),
        }
    }
    // 仅限开头的系统别名，其余分量一律按字面逐级打开
    #[cfg(target_os = "macos")]
    if comps
        .first()
        .is_some_and(|name| name == "tmp" || name == "var")
    {
        let alias = comps.remove(0);
        comps.insert(0, OsString::from("private"));
        comps.insert(1, alias);
    }
    Ok(comps)
}

#[cfg(unix)]
fn invalid_input(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

/// 从根 fd 逐级 `openat` 打开 `comps` 指向的目录；空片段序列打开根
/// 本身（`"."`）。每级都 `O_NOFOLLOW | O_DIRECTORY`，中间 fd 用完即关。
#[cfg(unix)]
fn open_dir_chain(start: &OwnedFd, comps: &[&OsStr]) -> io::Result<OwnedFd> {
    let mut fd = open_dir_at(start, comps.first().copied().unwrap_or(OsStr::new(".")))?;
    for name in comps.iter().skip(1) {
        let next = open_dir_at(&fd, name)?;
        drop(fd);
        fd = next;
    }
    Ok(fd)
}

#[cfg(unix)]
fn open_dir_at(parent: &OwnedFd, name: &OsStr) -> io::Result<OwnedFd> {
    Ok(rustix::fs::openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?)
}

/// d_type 缺失（DT_UNKNOWN）时用 `statat(..., AT_SYMLINK_NOFOLLOW)` 补
/// 全类型；条目在扫描间隙消失则跳过（返回 `None`），其余错误上抛。
#[cfg(unix)]
fn classify_unknown(dir: &Dir, name: &OsStr) -> io::Result<Option<Kind>> {
    let fd = dir.fd()?;
    match rustix::fs::statat(fd, name, AtFlags::SYMLINK_NOFOLLOW).map_err(io::Error::from) {
        Ok(stat) => Ok(Some(match FileType::from_raw_mode(stat.st_mode) {
            FileType::Directory => Kind::Dir,
            FileType::RegularFile => Kind::File,
            _ => Kind::Other,
        })),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use rustix::fs::{Mode, OFlags};
    use std::fs;
    use std::io::Read;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// 一次性夹具目录：唯一名 + Drop 清理，所有产物都在 /tmp 下。
    struct Fixture(PathBuf);

    impl Fixture {
        fn new(tag: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir()
                .join(format!("logbundle-fs-{tag}-{}-{nanos}", std::process::id()));
            fs::create_dir_all(&dir).unwrap();
            // 夹具必须落在物理路径上：/tmp 等前缀可能是符号链接，会被
            // Root::open 的逐分量 O_NOFOLLOW 拒绝
            let dir = dir.canonicalize().unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        fn write(&self, name: &str, content: &str) {
            fs::write(self.0.join(name), content).unwrap();
        }

        fn read_via(&self, root: &Root, relative: &str) -> String {
            let mut text = String::new();
            Read::read_to_string(&mut root.file(Path::new(relative)).unwrap(), &mut text).unwrap();
            text
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn list_root_sorts_and_consumes_budget() {
        let fix = Fixture::new("list-root");
        fix.write("b.log", "B");
        fix.write("a.log", "A");
        fs::create_dir(fix.path().join("sub")).unwrap();
        std::os::unix::fs::symlink("a.log", fix.path().join("alias")).unwrap();

        let root = Root::open(fix.path()).unwrap();
        let mut remaining = usize::MAX;
        let entries = root.list(Path::new(""), &mut remaining).unwrap();

        let names: Vec<&OsStr> = entries.iter().map(|e| e.name.as_os_str()).collect();
        assert_eq!(
            names,
            vec![
                OsStr::new("a.log"),
                OsStr::new("alias"),
                OsStr::new("b.log"),
                OsStr::new("sub")
            ]
        );
        let kinds: Vec<Kind> = entries.iter().map(|e| e.kind).collect();
        assert_eq!(kinds, vec![Kind::File, Kind::Other, Kind::File, Kind::Dir]);
        // 只有真正产出的目录项消耗预算
        assert_eq!(remaining, usize::MAX - 4);
    }

    #[test]
    fn budget_truncates_and_zero_budget_lists_nothing() {
        let fix = Fixture::new("budget");
        for i in 0..5 {
            fix.write(&format!("f{i}.log"), "x");
        }
        let root = Root::open(fix.path()).unwrap();

        // 预算 3：返回 3 条，remaining 归零即「可能截断」信号
        let mut remaining = 3usize;
        let entries = root.list(Path::new(""), &mut remaining).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(remaining, 0);

        // 预算已耗尽：不再产出任何条目
        let mut zero = 0usize;
        assert!(root.list(Path::new(""), &mut zero).unwrap().is_empty());
        assert_eq!(zero, 0);
    }

    #[test]
    fn rejects_symlinked_parent_and_final_component() {
        let fix = Fixture::new("symlink");
        fs::create_dir_all(fix.path().join("real/sub")).unwrap();
        fix.write("real/sub/x.log", "X");
        std::os::unix::fs::symlink("real", fix.path().join("link")).unwrap();
        fix.write("target.log", "T");
        std::os::unix::fs::symlink("target.log", fix.path().join("alias.log")).unwrap();

        let root = Root::open(fix.path()).unwrap();
        // 父级是符号链接：list 与 file 都必须失败，且不消耗预算
        let mut remaining = usize::MAX;
        assert!(root.list(Path::new("link/sub"), &mut remaining).is_err());
        assert_eq!(remaining, usize::MAX);
        assert!(root.file(Path::new("link/sub/x.log")).is_err());
        // 末级是符号链接：同样失败（O_NOFOLLOW → ELOOP）
        assert!(root.file(Path::new("alias.log")).is_err());
        // 列表里符号链接归 Other
        let mut remaining = usize::MAX;
        let entries = root.list(Path::new(""), &mut remaining).unwrap();
        assert!(
            entries
                .iter()
                .any(|e| e.name == "alias.log" && e.kind == Kind::Other)
        );
        // 真实路径仍可正常多级读取
        assert_eq!(fix.read_via(&root, "real/sub/x.log"), "X");
    }

    #[test]
    fn rejects_hardlink_and_fifo_but_reads_plain_file() {
        let fix = Fixture::new("hardlink-fifo");
        fix.write("orig.log", "O");
        fs::hard_link(fix.path().join("orig.log"), fix.path().join("dup.log")).unwrap();
        fix.write("solo.log", "S");
        let dirfd = rustix::fs::open(
            fix.path(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        rustix::fs::mkfifoat(&dirfd, "pipe", Mode::from_raw_mode(0o600)).unwrap();
        drop(dirfd);

        let root = Root::open(fix.path()).unwrap();
        // 硬链接：两个名字的 nlink 都是 2，一律拒绝
        assert!(root.file(Path::new("orig.log")).is_err());
        assert!(root.file(Path::new("dup.log")).is_err());
        // FIFO：非阻塞打开后按类型拒绝；若实现退化成阻塞 open，本测试会挂起暴露问题
        assert_eq!(
            root.file(Path::new("pipe")).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        // 普通单链接文件可读
        assert_eq!(fix.read_via(&root, "solo.log"), "S");
        // 列表：FIFO 归 Other；硬链接仍标 File（打开时才拒绝）
        let mut remaining = usize::MAX;
        let entries = root.list(Path::new(""), &mut remaining).unwrap();
        assert!(
            entries
                .iter()
                .any(|e| e.name == "pipe" && e.kind == Kind::Other)
        );
        assert!(
            entries
                .iter()
                .any(|e| e.name == "dup.log" && e.kind == Kind::File)
        );
    }

    #[test]
    fn reads_names_with_spaces_and_unicode() {
        let fix = Fixture::new("unicode");
        fix.write("会 话 日 志.log", "你好 world");

        let root = Root::open(fix.path()).unwrap();
        assert_eq!(fix.read_via(&root, "会 话 日 志.log"), "你好 world");
        let mut remaining = usize::MAX;
        let entries = root.list(Path::new(""), &mut remaining).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "会 话 日 志.log");
        assert_eq!(entries[0].kind, Kind::File);
    }

    #[test]
    fn rejects_absolute_parent_and_dot_paths() {
        let fix = Fixture::new("bad-paths");
        fix.write("a.log", "A");
        let root = Root::open(fix.path()).unwrap();

        for bad in ["/etc", "..", "../escape", "a/../b", ".", "./a.log"] {
            let mut remaining = usize::MAX;
            assert!(
                root.list(Path::new(bad), &mut remaining).is_err(),
                "list({bad}) 应被拒绝"
            );
            assert_eq!(remaining, usize::MAX, "出错不得消耗预算");
            assert!(root.file(Path::new(bad)).is_err(), "file({bad}) 应被拒绝");
        }
        // file 不接受空路径；list 用空路径表示根
        assert!(root.file(Path::new("")).is_err());
        let mut remaining = usize::MAX;
        assert_eq!(root.list(Path::new(""), &mut remaining).unwrap().len(), 1);
    }

    #[test]
    fn rejects_symlinked_root_parent_and_relative_root() {
        let fix = Fixture::new("root-symlink");
        fix.write("a.log", "A");
        fs::create_dir_all(fix.path().join("realdir/inner")).unwrap();
        // 根本身是符号链接：从 `/` 逐分量 O_NOFOLLOW 打开必须失败
        let root_link = fix.path().join("root-link");
        std::os::unix::fs::symlink(fix.path().join("realdir"), &root_link).unwrap();
        assert!(Root::open(&root_link).is_err());
        // 根的父级是符号链接：同样失败
        let parent_link = fix.path().join("parent-link");
        std::os::unix::fs::symlink(fix.path().join("realdir"), &parent_link).unwrap();
        assert!(Root::open(&parent_link.join("inner")).is_err());
        // 相对根与含 `..` 的根直接拒绝
        assert!(Root::open(Path::new("relative/dir")).is_err());
        assert!(Root::open(Path::new("/tmp/../etc")).is_err());
        // 物理根仍然可用
        let root = Root::open(fix.path()).unwrap();
        assert_eq!(fix.read_via(&root, "a.log"), "A");
    }

    #[test]
    fn missing_paths_report_not_found() {
        let fix = Fixture::new("missing");
        let root = Root::open(fix.path()).unwrap();

        assert_eq!(
            root.file(Path::new("nope.log")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        let mut remaining = usize::MAX;
        assert_eq!(
            root.list(Path::new("nodir"), &mut remaining)
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
        assert!(Root::open(&std::env::temp_dir().join("logbundle-fs-definitely-absent")).is_err());
    }
}
