//! 知识库文件读写授权面（Task 5）。
//!
//! 知识库根 = canonical 聚合根下的 `cadence/knowledge-base`（owner 裁决：知识库
//! 落在 LC 聚合根一份）。本模块只做**文件面**：目录列举、单文件读、单文件写。
//!
//! 路径授权四道（缺一不可）：
//! 1. 只接受相对路径（绝对路径、`..` 组件一律拒绝）；
//! 2. 解析后的真实目标必须仍在 `kb_root` 内（跟随符号链接解析，越界即拒）；
//! 3. 写白名单按**解析后的真实目标**复检 ⇒ 根内别名（`alias.md ->
//!    manifest.yaml`）不能借道覆盖机器管理文件；
//! 4. 读：必须是常规文件且不超过 1 MiB；写：白名单 = 全域 `*.md` +
//!    `user-input/**` 下 `*.sql`。
//!
//! **非常规文件**：读取用 `O_NONBLOCK` 打开 ⇒ 没有写端的 FIFO 也能立即打开
//! 成功（不阻塞在 `openat` 上），随后在**同一句柄**上核验文件类型
//! （`fstat`）⇒ 非常规文件一律 `NotFound`。
//!
//! **锚定语义（消除授权检查与实际操作之间的路径替换窗口）**：所有实际文件
//! 操作都通过知识库根的**目录 fd** 逐段 `openat` + `O_NOFOLLOW` 完成 ⇒ 授权
//! 通过之后父目录被换成符号链接，读写也不会被导向根外（`ELOOP` ⇒ 拒绝）。
//!
//! 乐观并发：写请求可带读时拿到的 `base_digest`（sha256 小写十六进制）；
//! 与服务端当前 digest 不符 ⇒ `Conflict`（不静默覆盖）；给了但文件不存在 ⇒
//! `NotFound`；不带 ⇒ 直接覆盖。摘要比较与提交落在**同一把按真实目标索引的
//! 锁**内 ⇒ 两个携带同一有效摘要的并发写者只有一个能成功。
//!
//! 原子写：同目录**排他**临时文件（`O_CREAT|O_EXCL|O_NOFOLLOW` ⇒ 不跟随预置
//! 符号链接、不截断其目标；撞名换名重试）+ `renameat` 提交（写完不留残留）。
//!
//! 运行期禁写：`operation_running == true` ⇒ `OperationRunning`（避免与
//! provider turn 抢写同一目录）。

use std::collections::HashMap;
use std::ffi::{CString, OsStr};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, Weak};

use crate::cross_cutting::document_ops::compute_sha256;
use crate::product::json_store::ProductStoreError;

/// 单文件读取上限：1 MiB。
const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// 知识库根相对聚合根的组件（`cadence/knowledge-base`）。
const KB_ROOT_COMPONENTS: [&str; 2] = ["cadence", "knowledge-base"];

/// 临时文件换名重试上限（撞名时依次换名）。
const TEMP_NAME_ATTEMPTS: usize = 64;

/// 知识库文件树中的一个节点（相对知识库根的路径，`/` 分隔）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeBaseFileNode {
    /// 相对知识库根的路径（`/` 分隔）。
    pub path: String,
    /// 是否是目录。
    pub is_dir: bool,
    /// 文件字节数（目录恒为 0）。
    pub size: u64,
    /// 是否落在写白名单内（`*.md` 全域 + `user-input/**` 下的 `*.sql`）。
    pub editable: bool,
}

/// 知识库单文件内容与其摘要。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnowledgeBaseFileContent {
    /// 相对知识库根的路径（`/` 分隔）。
    pub path: String,
    /// 文件文本内容。
    pub content: String,
    /// 内容 sha256 摘要（小写十六进制）。
    pub digest: String,
}

/// 文件面错误（六个稳定语义，HTTP 层逐一映射）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum KnowledgeBaseFileError {
    /// 路径越出知识库根（绝对路径、`..`、符号链接逃逸）。
    #[error("knowledge_base_file_path_escapes")]
    PathEscapes,
    /// 目标不存在或不可读。
    #[error("knowledge_base_file_not_found")]
    NotFound,
    /// 目标不在写白名单内。
    #[error("knowledge_base_file_not_editable")]
    NotEditable,
    /// 目标超过单文件大小上限。
    #[error("knowledge_base_file_too_large")]
    TooLarge,
    /// 乐观并发基线摘要与服务端当前摘要不一致。
    #[error("knowledge_base_file_conflict")]
    Conflict,
    /// 知识库 operation 运行期禁止写入。
    #[error("knowledge_base_operation_running")]
    OperationRunning,
}

/// 知识库根：canonical 聚合根下的 `cadence/knowledge-base`。
pub fn kb_root(aggregate_root: &Path) -> PathBuf {
    aggregate_root.join("cadence").join("knowledge-base")
}

/// 列举知识库根下的全部文件与目录（相对知识库根的路径）。
///
/// 知识库根不存在 ⇒ 空列表（知识库尚未初始化，不算错误）；元数据查询的其他
/// 失败（权限、链接循环、路径不是目录等）⇒ `ProductStoreError::Io`（不得伪装
/// 成「尚未初始化」）；符号链接不入树（避免把越界目标暴露成可读条目）。
pub fn list_knowledge_base_files(
    aggregate_root: &Path,
) -> Result<Vec<KnowledgeBaseFileNode>, ProductStoreError> {
    let root = kb_root(aggregate_root);
    match std::fs::metadata(&root) {
        Ok(metadata) => {
            if !metadata.is_dir() {
                return Err(ProductStoreError::Io(format!(
                    "知识库根不是目录：{}",
                    root.display()
                )));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Vec::new());
        }
        Err(error) => {
            return Err(ProductStoreError::Io(format!(
                "stat {}: {error}",
                root.display()
            )));
        }
    }
    let anchor = KnowledgeBaseRoot::open_existing(aggregate_root)
        .map_err(|error| ProductStoreError::Io(format!("锚定知识库根失败：{error}")))?;
    let mut nodes = Vec::new();
    collect_nodes(&anchor.fd, "", &mut nodes)?;
    nodes.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(nodes)
}

/// 读取知识库内的单个文件（只读授权：路径合法 + 常规文件 + 不超过 1 MiB）。
///
/// 读取绑定到知识库根的目录 fd（`openat` + `O_NOFOLLOW`）⇒ 授权检查之后
/// 目标被换成符号链接也读不到根外内容。
pub fn read_knowledge_base_file(
    aggregate_root: &Path,
    rel_path: &str,
) -> Result<KnowledgeBaseFileContent, KnowledgeBaseFileError> {
    let components = relative_components(rel_path)?;
    let root = KnowledgeBaseRoot::open_existing(aggregate_root)?;
    let resolved = resolve_within_root(&root.canonical, &components)?;
    let Some((parent_components, file_name)) = resolved.split_last() else {
        return Err(KnowledgeBaseFileError::NotFound);
    };
    let parent = walk_dirs(&root.fd, &parent_components, false).map_err(map_io_error)?;
    let mut file = open_file_at(&parent, OsStr::new(&file_name)).map_err(map_io_error)?;
    let metadata = file.metadata().map_err(map_io_error)?;
    if !metadata.is_file() {
        return Err(KnowledgeBaseFileError::NotFound);
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(KnowledgeBaseFileError::TooLarge);
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).map_err(map_io_error)?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err(KnowledgeBaseFileError::TooLarge);
    }
    let content = String::from_utf8(bytes).map_err(|_| KnowledgeBaseFileError::NotFound)?;
    let digest = compute_sha256(content.as_bytes());
    Ok(KnowledgeBaseFileContent {
        path: components.join("/"),
        content,
        digest,
    })
}

/// 写入知识库内的单个文件（白名单 + 乐观并发 + 原子写 + 运行期禁写）。
///
/// 返回写入后内容的 sha256 摘要（小写十六进制）。
pub fn write_knowledge_base_file(
    aggregate_root: &Path,
    rel_path: &str,
    content: &str,
    base_digest: Option<&str>,
    operation_running: bool,
) -> Result<String, KnowledgeBaseFileError> {
    if operation_running {
        return Err(KnowledgeBaseFileError::OperationRunning);
    }
    let components = relative_components(rel_path)?;
    let requested = components.join("/");
    if !is_editable_relative(&requested) {
        return Err(KnowledgeBaseFileError::NotEditable);
    }
    let root = KnowledgeBaseRoot::open_existing(aggregate_root)?;
    let resolved = resolve_within_root(&root.canonical, &components)?;
    // 白名单按**解析后的真实目标**复检：根内别名（`alias.md -> manifest.yaml`）
    // 不得借道覆盖机器管理文件。
    if !is_editable_relative(&resolved.relative()) {
        return Err(KnowledgeBaseFileError::NotEditable);
    }
    let Some((parent_components, file_name)) = resolved.split_last() else {
        return Err(KnowledgeBaseFileError::NotEditable);
    };
    // 摘要比较与提交落在同一把按真实目标索引的锁内 ⇒ 并发写者只有一个成功。
    let mut target_key = root.canonical.clone();
    for component in &resolved.components {
        target_key.push(component);
    }
    let lock = target_lock(&target_key);
    let _guard = lock.lock().unwrap_or_else(|poison| poison.into_inner());
    // 锁内重新锚定并走查父目录（`openat` + `O_NOFOLLOW`）⇒ 授权通过之后父
    // 目录被换成符号链接也写不到根外。
    let root = KnowledgeBaseRoot::open_existing(aggregate_root)?;
    let parent = walk_dirs(&root.fd, &parent_components, true).map_err(map_io_error)?;
    if openat_dir(&parent, OsStr::new(&file_name)).is_ok() {
        // 目标是目录 ⇒ 不可写（与既有语义一致）。
        return Err(KnowledgeBaseFileError::NotEditable);
    }
    if let Some(expected) = base_digest {
        let mut current = open_file_at(&parent, OsStr::new(&file_name)).map_err(map_io_error)?;
        let mut bytes = Vec::new();
        current.read_to_end(&mut bytes).map_err(map_io_error)?;
        if compute_sha256(&bytes) != expected {
            return Err(KnowledgeBaseFileError::Conflict);
        }
    }
    let (temporary_name, mut temporary) = create_temporary_at(&parent, OsStr::new(&file_name))?;
    if let Err(error) = temporary.write_all(content.as_bytes()) {
        let _ = unlink_at(&parent, OsStr::new(&temporary_name), false);
        return Err(map_io_error(error));
    }
    if let Err(error) = temporary.flush() {
        let _ = unlink_at(&parent, OsStr::new(&temporary_name), false);
        return Err(map_io_error(error));
    }
    drop(temporary);
    if let Err(error) = rename_at(&parent, OsStr::new(&temporary_name), OsStr::new(&file_name)) {
        let _ = unlink_at(&parent, OsStr::new(&temporary_name), false);
        return Err(map_io_error(error));
    }
    Ok(compute_sha256(content.as_bytes()))
}

/// 知识库根的锚定句柄：目录 fd（锚定 inode，路径被替换也不受影响）+ canonical
/// 根路径（用于相对路径计算与锁键）。
pub(crate) struct KnowledgeBaseRoot {
    /// 知识库根的目录 fd。
    pub(crate) fd: OwnedFd,
    /// 知识库根的 canonical 路径。
    pub(crate) canonical: PathBuf,
}

impl KnowledgeBaseRoot {
    /// 锚定**既有**知识库根（不存在 ⇒ `NotFound`；链上有符号链接 ⇒ `PathEscapes`）。
    pub(crate) fn open_existing(aggregate_root: &Path) -> Result<Self, KnowledgeBaseFileError> {
        Self::anchor(aggregate_root, false).map_err(map_io_error)
    }

    /// 锚定知识库根，缺失时按需创建（模板面用；错误统一为 `Io`）。
    pub(crate) fn open_or_create(aggregate_root: &Path) -> Result<Self, ProductStoreError> {
        Self::anchor(aggregate_root, true)
            .map_err(|error| ProductStoreError::Io(format!("锚定知识库根失败：{error}")))
    }

    /// 从 canonical 聚合根逐段 `openat`（`O_DIRECTORY|O_NOFOLLOW`）走到知识库根。
    fn anchor(aggregate_root: &Path, create_missing: bool) -> std::io::Result<Self> {
        let aggregate_canonical = std::fs::canonicalize(aggregate_root)?;
        if !aggregate_canonical.is_dir() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotADirectory,
                "聚合根不是目录",
            ));
        }
        let mut current = open_dir_nofollow(&aggregate_canonical)?;
        for name in KB_ROOT_COMPONENTS {
            match openat_dir(&current, OsStr::new(name)) {
                Ok(fd) => current = fd,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound && create_missing => {
                    match mkdirat(&current, OsStr::new(name)) {
                        Ok(()) => {}
                        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                        Err(error) => return Err(error),
                    }
                    current = openat_dir(&current, OsStr::new(name))?;
                }
                Err(error) => return Err(error),
            }
        }
        let canonical = std::fs::canonicalize(kb_root(aggregate_root))?;
        if !canonical.starts_with(&aggregate_canonical) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "知识库根不在聚合根内",
            ));
        }
        Ok(Self {
            fd: current,
            canonical,
        })
    }
}

/// 解析后的授权目标：相对知识库根的**真实**组件（已跟随符号链接解析）。
struct ResolvedTarget {
    components: Vec<String>,
}

impl ResolvedTarget {
    /// 相对知识库根的展示路径（`/` 分隔）。
    fn relative(&self) -> String {
        self.components.join("/")
    }

    /// 拆成（父目录组件, 文件名）。
    fn split_last(&self) -> Option<(Vec<String>, String)> {
        let mut components = self.components.clone();
        let name = components.pop()?;
        Some((components, name))
    }
}

/// 解析目标路径并校验真实目标仍在知识库根内（跟随符号链接）。
///
/// 目标不存在时按「最深存在祖先」定位：祖先必须落在知识库根内，缺失的尾部
/// 组件原样接回 ⇒ 越界的符号链接（无论指向文件还是目录）一律 `PathEscapes`。
fn resolve_within_root(
    root: &Path,
    components: &[String],
) -> Result<ResolvedTarget, KnowledgeBaseFileError> {
    let mut probe = root.to_path_buf();
    for component in components {
        probe.push(component);
    }
    let mut trailing: Vec<String> = Vec::new();
    loop {
        match std::fs::symlink_metadata(&probe) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = probe
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or(KnowledgeBaseFileError::PathEscapes)?
                    .to_string();
                trailing.push(name);
                probe = probe
                    .parent()
                    .ok_or(KnowledgeBaseFileError::PathEscapes)?
                    .to_path_buf();
            }
            Err(_) => return Err(KnowledgeBaseFileError::PathEscapes),
        }
    }
    let canonical_probe = probe
        .canonicalize()
        .map_err(|_| KnowledgeBaseFileError::PathEscapes)?;
    if !canonical_probe.starts_with(root) {
        return Err(KnowledgeBaseFileError::PathEscapes);
    }
    let mut resolved = canonical_probe;
    for name in trailing.iter().rev() {
        resolved.push(name);
    }
    if !resolved.starts_with(root) {
        return Err(KnowledgeBaseFileError::PathEscapes);
    }
    let resolved_components: Vec<String> = resolved
        .strip_prefix(root)
        .map_err(|_| KnowledgeBaseFileError::PathEscapes)?
        .components()
        .map(|component| match component {
            Component::Normal(name) => name
                .to_str()
                .map(|name| name.to_string())
                .ok_or(KnowledgeBaseFileError::PathEscapes),
            _ => Err(KnowledgeBaseFileError::PathEscapes),
        })
        .collect::<Result<Vec<String>, KnowledgeBaseFileError>>()?;
    Ok(ResolvedTarget {
        components: resolved_components,
    })
}

/// 写白名单：全域 `*.md` + `user-input/**` 下 `*.sql`。
fn is_editable_relative(rel_path: &str) -> bool {
    let path = Path::new(rel_path);
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    if extension.eq_ignore_ascii_case("md") {
        return true;
    }
    if !extension.eq_ignore_ascii_case("sql") {
        return false;
    }
    matches!(
        path.components().next(),
        Some(Component::Normal(name)) if name == "user-input"
    )
}

/// 相对路径片段解析：只接受普通组件，拒绝绝对路径与 `..`。
fn relative_components(rel_path: &str) -> Result<Vec<String>, KnowledgeBaseFileError> {
    if rel_path.trim().is_empty() {
        return Err(KnowledgeBaseFileError::PathEscapes);
    }
    let mut components = Vec::new();
    for component in Path::new(rel_path).components() {
        match component {
            Component::Normal(name) => {
                let text = name
                    .to_str()
                    .ok_or(KnowledgeBaseFileError::PathEscapes)?
                    .to_string();
                if text.is_empty() {
                    return Err(KnowledgeBaseFileError::PathEscapes);
                }
                components.push(text);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(KnowledgeBaseFileError::PathEscapes);
            }
        }
    }
    if components.is_empty() {
        return Err(KnowledgeBaseFileError::PathEscapes);
    }
    Ok(components)
}

/// 目标串行化锁：按**真实目标路径**索引的进程内互斥锁。
///
/// 摘要比较与提交必须落在同一把锁内 ⇒ 两个携带同一有效摘要的并发写者，
/// 只有先进入临界区的那个能提交，后者读到新摘要 ⇒ `Conflict`。
fn target_lock(key: &Path) -> Arc<Mutex<()>> {
    static REGISTRY: LazyLock<Mutex<HashMap<PathBuf, Weak<Mutex<()>>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));
    let mut map = REGISTRY.lock().unwrap_or_else(|poison| poison.into_inner());
    if let Some(lock) = map.get(key).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    map.insert(key.to_path_buf(), Arc::downgrade(&lock));
    map.retain(|_, weak| weak.strong_count() > 0);
    lock
}

/// io 错误 ⇒ 文件面稳定语义：符号链接/结构越界 ⇒ `PathEscapes`，其余 ⇒ `NotFound`。
fn map_io_error(error: std::io::Error) -> KnowledgeBaseFileError {
    match error.raw_os_error() {
        Some(libc::ELOOP) | Some(libc::ENOTDIR) => KnowledgeBaseFileError::PathEscapes,
        _ => KnowledgeBaseFileError::NotFound,
    }
}

// ---------------------------------------------------------------------------
// 锚定文件系统原语（`openat` + `O_NOFOLLOW`）
// ---------------------------------------------------------------------------

/// 构造以 NUL 结尾的路径参数（路径不含内嵌 NUL）。
fn cstr(bytes: &[u8]) -> CString {
    CString::new(bytes).expect("路径不含内嵌 NUL")
}

/// 以 `O_DIRECTORY|O_NOFOLLOW` 打开目录（锚定 inode）。
fn open_dir_nofollow(path: &Path) -> std::io::Result<OwnedFd> {
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let fd = unsafe { libc::open(cstr(path.as_os_str().as_bytes()).as_ptr(), flags) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// 在 `dir` 下以 `O_DIRECTORY|O_NOFOLLOW` 打开子目录（不跟随符号链接）。
pub(crate) fn openat_dir(dir: &OwnedFd, name: &OsStr) -> std::io::Result<OwnedFd> {
    let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let fd = unsafe { libc::openat(dir.as_raw_fd(), cstr(name.as_bytes()).as_ptr(), flags) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { OwnedFd::from_raw_fd(fd) })
}

/// 在 `dir` 下创建子目录（`mkdirat`；已存在 ⇒ `AlreadyExists`）。
pub(crate) fn mkdirat(dir: &OwnedFd, name: &OsStr) -> std::io::Result<()> {
    let result = unsafe {
        libc::mkdirat(
            dir.as_raw_fd(),
            cstr(name.as_bytes()).as_ptr(),
            0o755 as libc::mode_t,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// 从 `root` 逐段走查子目录（`create_missing` 时补建缺失段），返回末段目录 fd。
///
/// 每一段都用 `O_NOFOLLOW` 打开 ⇒ 链上的符号链接（含授权检查之后被替换的
/// 父目录）一律 `ELOOP` 拒绝，不会把操作导向根外。
pub(crate) fn walk_dirs(
    root: &OwnedFd,
    components: &[String],
    create_missing: bool,
) -> std::io::Result<OwnedFd> {
    let mut current = root.try_clone()?;
    for name in components {
        match openat_dir(&current, OsStr::new(name)) {
            Ok(fd) => current = fd,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && create_missing => {
                match mkdirat(&current, OsStr::new(name)) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(error),
                }
                current = openat_dir(&current, OsStr::new(name))?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(current)
}

/// 在 `dir` 下只读打开文件（`O_NOFOLLOW` ⇒ 不跟随符号链接）。
///
/// `O_NONBLOCK` ⇒ **没有写端的 FIFO 也会立即打开成功**（`openat` 不阻塞），
/// 随后的**同一句柄**元数据核验（`fstat` ⇒ `is_file()`）把它判为非常规文件
/// ⇒ `NotFound`；修复前是「先 `openat` 后判类型」，遇到无写端 FIFO 会永久
/// 阻塞在打开动作上，永远走不到类型检查。
fn open_file_at(dir: &OwnedFd, name: &OsStr) -> std::io::Result<std::fs::File> {
    let flags = libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC;
    let fd = unsafe { libc::openat(dir.as_raw_fd(), cstr(name.as_bytes()).as_ptr(), flags) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

/// 在 `dir` 下**排他**创建文件（`O_CREAT|O_EXCL|O_NOFOLLOW`）。
///
/// 已存在 ⇒ `AlreadyExists`（含悬空符号链接 ⇒ 不跟随、不改写其目标）。
pub(crate) fn create_exclusive_at(dir: &OwnedFd, name: &OsStr) -> std::io::Result<std::fs::File> {
    let flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC;
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            cstr(name.as_bytes()).as_ptr(),
            flags,
            0o666 as libc::c_uint,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

/// 在 `dir` 下排他创建文件并写入内容（已存在 ⇒ `AlreadyExists`，不覆盖）。
pub(crate) fn create_new_at(dir: &OwnedFd, name: &OsStr, content: &[u8]) -> std::io::Result<()> {
    let mut file = create_exclusive_at(dir, name)?;
    file.write_all(content)?;
    file.flush()?;
    Ok(())
}

/// 同目录排他临时文件（`O_CREAT|O_EXCL|O_NOFOLLOW`），撞名换名重试。
///
/// 排他创建 ⇒ 预置的同名符号链接不会被跟随、其目标不会被截断；撞名时换名
/// 重试，写入始终持有句柄（不按路径二次打开）。
fn create_temporary_at(
    dir: &OwnedFd,
    target_name: &OsStr,
) -> Result<(String, std::fs::File), KnowledgeBaseFileError> {
    let target = target_name.to_string_lossy().into_owned();
    let pid = std::process::id();
    for attempt in 0..TEMP_NAME_ATTEMPTS {
        let name = if attempt == 0 {
            format!(".{target}.write-{pid}")
        } else {
            format!(".{target}.write-{pid}-{attempt}")
        };
        match create_exclusive_at(dir, OsStr::new(&name)) {
            Ok(file) => return Ok((name, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(map_io_error(error)),
        }
    }
    Err(KnowledgeBaseFileError::Conflict)
}

/// 目录内**不替换**改名（同目录 ⇒ 原子提交）。
///
/// 目标已存在 ⇒ `EEXIST`（**不像 `renameat` 那样替换空目录**）⇒ 调用方可把
/// 「已被并发发布」判成幂等成功，而不是并发竞态下的失败。
///
/// 各平台实现（**同一语义，不同系统调用**）：
/// - Linux：`renameat2(RENAME_NOREPLACE)`；
/// - macOS：`renamex_np(RENAME_EXCL)`；
/// - 其它目标：退回「先查存在性再改名」（**非原子**，只保证不覆盖已存在目标）。
///
/// 内核或文件系统不支持该标志时返回 `ENOSYS` / `EINVAL`（由调用方降级）。
#[cfg(target_os = "linux")]
pub(crate) fn rename_noreplace_at(dir: &OwnedFd, from: &OsStr, to: &OsStr) -> std::io::Result<()> {
    // 用 `syscall` 而非 `libc::renameat2`：后者只在部分 libc 目标（gnu/android）
    // 上导出，`syscall(SYS_renameat2, …)` 在全部 Linux 目标可用。
    let result = unsafe {
        libc::syscall(
            libc::SYS_renameat2,
            dir.as_raw_fd(),
            cstr(from.as_bytes()).as_ptr(),
            dir.as_raw_fd(),
            cstr(to.as_bytes()).as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// macOS 的「不替换改名」：`renamex_np(RENAME_EXCL)`（与 Linux 的
/// `RENAME_NOREPLACE` 同语义）。目标已存在 ⇒ `EEXIST`。
#[cfg(target_os = "macos")]
pub(crate) fn rename_noreplace_at(dir: &OwnedFd, from: &OsStr, to: &OsStr) -> std::io::Result<()> {
    let result = unsafe {
        libc::renamex_np(
            cstr(from.as_bytes()).as_ptr(),
            cstr(to.as_bytes()).as_ptr(),
            libc::RENAME_EXCL,
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let _ = dir;
    Ok(())
}

/// 其它目标（无「不替换改名」系统调用）：先查目标是否存在，存在即报
/// `AlreadyExists`，否则普通改名。**非原子**：并发下存在极窄窗口 —— 只在
/// 不提供该语义的平台上退化，Linux/macOS 走上面的原子实现。
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) fn rename_noreplace_at(dir: &OwnedFd, from: &OsStr, to: &OsStr) -> std::io::Result<()> {
    let target = std::path::Path::new("/proc/self/fd")
        .join(dir.as_raw_fd().to_string())
        .join(to);
    if target.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "目标已存在",
        ));
    }
    rename_at(dir, from, to)
}

/// 目录内改名（`renameat`，同目录 ⇒ 原子提交）。
pub(crate) fn rename_at(dir: &OwnedFd, from: &OsStr, to: &OsStr) -> std::io::Result<()> {
    let result = unsafe {
        libc::renameat(
            dir.as_raw_fd(),
            cstr(from.as_bytes()).as_ptr(),
            dir.as_raw_fd(),
            cstr(to.as_bytes()).as_ptr(),
        )
    };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// 目录内删除（`unlinkat`；`is_dir` ⇒ 删除空目录）。
pub(crate) fn unlink_at(dir: &OwnedFd, name: &OsStr, is_dir: bool) -> std::io::Result<()> {
    let flags = if is_dir { libc::AT_REMOVEDIR } else { 0 };
    let result = unsafe { libc::unlinkat(dir.as_raw_fd(), cstr(name.as_bytes()).as_ptr(), flags) };
    if result != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// 目录内递归删除一棵树（锚定在 `parent` fd 上，不按路径二次解析）。
pub(crate) fn remove_tree_at(parent: &OwnedFd, name: &OsStr) -> std::io::Result<()> {
    let dir = match openat_dir(parent, name) {
        Ok(fd) => fd,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let listing = std::fs::read_dir(format!("/proc/self/fd/{}", dir.as_raw_fd()))?;
    for entry in listing {
        let entry = entry?;
        let child = entry.file_name();
        if child == OsStr::new(".") || child == OsStr::new("..") {
            continue;
        }
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            remove_tree_at(&dir, &child)?;
        } else {
            unlink_at(&dir, &child, false)?;
        }
    }
    unlink_at(parent, name, true)
}

/// 递归收集目录节点（符号链接不入树，避免越界读取）。
///
/// 目录遍历绑定到根 fd（子目录用 `openat` + `O_NOFOLLOW` 打开）⇒ 遍历途中
/// 被换成符号链接的目录不会被跟随。
fn collect_nodes(
    dir: &OwnedFd,
    prefix: &str,
    nodes: &mut Vec<KnowledgeBaseFileNode>,
) -> Result<(), ProductStoreError> {
    let listing = std::fs::read_dir(format!("/proc/self/fd/{}", dir.as_raw_fd()))
        .map_err(|error| ProductStoreError::Io(format!("read_dir {}: {error}", dir.as_raw_fd())))?;
    for entry in listing {
        let entry =
            entry.map_err(|error| ProductStoreError::Io(format!("read_dir entry: {error}")))?;
        let name = entry.file_name();
        let relative = if prefix.is_empty() {
            name.to_string_lossy().into_owned()
        } else {
            format!("{prefix}/{}", name.to_string_lossy())
        };
        let file_type = entry
            .file_type()
            .map_err(|error| ProductStoreError::Io(format!("file_type {}: {error}", relative)))?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            nodes.push(KnowledgeBaseFileNode {
                path: relative.clone(),
                is_dir: true,
                size: 0,
                editable: false,
            });
            match openat_dir(dir, &name) {
                Ok(child) => collect_nodes(&child, &relative, nodes)?,
                Err(error)
                    if error.raw_os_error() == Some(libc::ELOOP)
                        || error.raw_os_error() == Some(libc::ENOTDIR) =>
                {
                    // 遍历途中被换成符号链接/非目录 ⇒ 跳过该条目（不跟随）。
                    continue;
                }
                Err(error) => {
                    return Err(ProductStoreError::Io(format!(
                        "open_dir {}: {error}",
                        relative
                    )));
                }
            }
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let size = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
        let editable = is_editable_relative(&relative);
        nodes.push(KnowledgeBaseFileNode {
            path: relative,
            is_dir: false,
            size,
            editable,
        });
    }
    Ok(())
}

include!("files_tests.inc.rs");
