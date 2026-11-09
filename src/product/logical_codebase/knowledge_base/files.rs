//! 知识库文件读写授权面（Task 5）。
//!
//! 知识库根 = canonical 聚合根下的 `cadence/knowledge-base`（owner 裁决：知识库
//! 落在 LC 聚合根一份）。本模块只做**文件面**：目录列举、单文件读、单文件写。
//!
//! 路径授权四道（缺一不可）：
//! 1. 只接受相对路径（绝对路径、`..` 组件一律拒绝）；
//! 2. 解析后 `canonicalize`（跟随符号链接）再判定必须仍在 `kb_root` 内
//!    ⇒ 禁符号链接逃逸；
//! 3. 读：必须是常规文件，且大小不超过 1 MiB；
//! 4. 写：白名单 = 全域 `*.md` + `user-input/**` 下 `*.sql`；`manifest.yaml` /
//!    `relation-graph.yaml` 等机器管理与派生文件天然不可写。
//!
//! 乐观并发：写请求可带读时拿到的 `base_digest`（sha256 小写十六进制）；
//! 与服务端当前 digest 不符 ⇒ `Conflict`（不静默覆盖）；给了但文件不存在 ⇒
//! `NotFound`；不带 ⇒ 直接覆盖。
//!
//! 原子写：同目录临时文件 + `std::fs::rename`（写完不留 `.tmp` 残留）。
//!
//! 运行期禁写：`operation_running == true` ⇒ `OperationRunning`（避免与
//! provider turn 抢写同一目录）。

use std::path::{Component, Path, PathBuf};

use crate::cross_cutting::document_ops::compute_sha256;
use crate::product::json_store::ProductStoreError;

/// 单文件读取上限：1 MiB。
const MAX_FILE_BYTES: u64 = 1024 * 1024;

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
/// 知识库根不存在 ⇒ 空列表（知识库尚未初始化，不算错误）；符号链接不入树
/// （避免把越界目标暴露成可读条目）。
pub fn list_knowledge_base_files(
    aggregate_root: &Path,
) -> Result<Vec<KnowledgeBaseFileNode>, ProductStoreError> {
    let root = kb_root(aggregate_root);
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut nodes = Vec::new();
    collect_nodes(&root, &root, &mut nodes)?;
    nodes.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(nodes)
}

/// 读取知识库内的单个文件（只读授权：路径合法 + 常规文件 + 不超过 1 MiB）。
pub fn read_knowledge_base_file(
    aggregate_root: &Path,
    rel_path: &str,
) -> Result<KnowledgeBaseFileContent, KnowledgeBaseFileError> {
    let root = kb_root(aggregate_root);
    if !root.is_dir() {
        return Err(KnowledgeBaseFileError::NotFound);
    }
    let components = relative_components(rel_path)?;
    let target = resolve_within_root(&root, &components)?;
    let metadata = std::fs::metadata(&target).map_err(|_| KnowledgeBaseFileError::NotFound)?;
    if !metadata.is_file() {
        return Err(KnowledgeBaseFileError::NotFound);
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(KnowledgeBaseFileError::TooLarge);
    }
    let bytes = std::fs::read(&target).map_err(|_| KnowledgeBaseFileError::NotFound)?;
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
    let relative = components.join("/");
    if !is_editable_relative(&relative) {
        return Err(KnowledgeBaseFileError::NotEditable);
    }
    let root = kb_root(aggregate_root);
    if !root.is_dir() {
        return Err(KnowledgeBaseFileError::NotFound);
    }
    let target = resolve_within_root(&root, &components)?;
    if target.is_dir() {
        return Err(KnowledgeBaseFileError::NotEditable);
    }
    if let Some(expected) = base_digest {
        let current = std::fs::read(&target).map_err(|_| KnowledgeBaseFileError::NotFound)?;
        if compute_sha256(&current) != expected {
            return Err(KnowledgeBaseFileError::Conflict);
        }
    }
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|_| KnowledgeBaseFileError::NotFound)?;
    }
    let temporary = temporary_path(&target);
    std::fs::write(&temporary, content).map_err(|_| {
        let _ = std::fs::remove_file(&temporary);
        KnowledgeBaseFileError::NotFound
    })?;
    if std::fs::rename(&temporary, &target).is_err() {
        let _ = std::fs::remove_file(&temporary);
        return Err(KnowledgeBaseFileError::NotFound);
    }
    Ok(compute_sha256(content.as_bytes()))
}

/// 递归收集目录节点（符号链接不入树，避免越界读取）。
fn collect_nodes(
    root: &Path,
    dir: &Path,
    nodes: &mut Vec<KnowledgeBaseFileNode>,
) -> Result<(), ProductStoreError> {
    let entries = std::fs::read_dir(dir)
        .map_err(|error| ProductStoreError::Io(format!("read_dir {}: {error}", dir.display())))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| ProductStoreError::Io(format!("read_dir entry: {error}")))?;
        let file_type = entry.file_type().map_err(|error| {
            ProductStoreError::Io(format!("file_type {}: {error}", entry.path().display()))
        })?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        let relative = relative_display(root, &path);
        if file_type.is_dir() {
            nodes.push(KnowledgeBaseFileNode {
                path: relative,
                is_dir: true,
                size: 0,
                editable: false,
            });
            collect_nodes(root, &path, nodes)?;
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

/// 解析目标路径并校验仍在知识库根内（跟随符号链接）。
///
/// 目标不存在时按「最深存在祖先」定位：祖先必须落在知识库根内，缺失的尾部
/// 组件原样接回 ⇒ 越界的符号链接（无论指向文件还是目录）一律 `PathEscapes`。
fn resolve_within_root(
    root: &Path,
    components: &[String],
) -> Result<PathBuf, KnowledgeBaseFileError> {
    let canonical_root = root
        .canonicalize()
        .map_err(|_| KnowledgeBaseFileError::PathEscapes)?;
    let mut probe = root.to_path_buf();
    for component in components {
        probe.push(component);
    }
    let mut trailing: Vec<String> = Vec::new();
    loop {
        match probe.symlink_metadata() {
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
    if !canonical_probe.starts_with(&canonical_root) {
        return Err(KnowledgeBaseFileError::PathEscapes);
    }
    let mut resolved = canonical_probe;
    for name in trailing.iter().rev() {
        resolved.push(name);
    }
    if !resolved.starts_with(&canonical_root) {
        return Err(KnowledgeBaseFileError::PathEscapes);
    }
    Ok(resolved)
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

/// 同目录临时文件路径（写完由 `rename` 顶替，不留 `.tmp` 残留）。
///
/// 名称带进程号与自增序号 ⇒ 同进程内并发写同一目标也不会撞名。
fn temporary_path(target: &Path) -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "knowledge-base".to_string());
    let sequence = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    target.with_file_name(format!(".{name}.tmp-{}-{sequence}", std::process::id()))
}

/// 相对知识库根的展示路径（`/` 分隔）。
fn relative_display(root: &Path, path: &Path) -> String {
    let relative = path.strip_prefix(root).unwrap_or(path);
    relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod knowledge_base_files_tests {
    use super::{
        KnowledgeBaseFileError, kb_root, list_knowledge_base_files, read_knowledge_base_file,
        write_knowledge_base_file,
    };
    use std::path::PathBuf;

    /// 夹具只落 /tmp：独占一个聚合根，并预建知识库根。
    fn kb_fixture() -> (tempfile::TempDir, PathBuf) {
        let fixture = tempfile::TempDir::new().expect("创建 /tmp 夹具");
        let aggregate_root = fixture.path().to_path_buf();
        std::fs::create_dir_all(kb_root(&aggregate_root)).expect("创建知识库根");
        (fixture, aggregate_root)
    }

    #[test]
    fn read_rejects_parent_directory_escape() {
        let (_fixture, aggregate_root) = kb_fixture();
        let error = read_knowledge_base_file(&aggregate_root, "../../etc/passwd")
            .expect_err("父目录逃逸必须被拒绝");
        assert_eq!(error, KnowledgeBaseFileError::PathEscapes);
    }

    #[test]
    fn read_rejects_absolute_path() {
        let (_fixture, aggregate_root) = kb_fixture();
        let error = read_knowledge_base_file(&aggregate_root, "/etc/passwd")
            .expect_err("绝对路径必须被拒绝");
        assert_eq!(error, KnowledgeBaseFileError::PathEscapes);
    }

    #[cfg(unix)]
    #[test]
    fn read_rejects_symlink_escaping_root() {
        let (fixture, aggregate_root) = kb_fixture();
        let outside = fixture.path().join("outside.md");
        std::fs::write(&outside, "外部内容").expect("写入根外文件");
        let link = kb_root(&aggregate_root).join("link.md");
        std::os::unix::fs::symlink(&outside, &link).expect("创建越界符号链接");

        let error = read_knowledge_base_file(&aggregate_root, "link.md")
            .expect_err("越界符号链接必须被拒绝");
        assert_eq!(error, KnowledgeBaseFileError::PathEscapes);
    }

    #[test]
    fn write_rejects_manifest_yaml() {
        let (_fixture, aggregate_root) = kb_fixture();
        let error = write_knowledge_base_file(&aggregate_root, "manifest.yaml", "x", None, false)
            .expect_err("manifest.yaml 不可写");
        assert_eq!(error, KnowledgeBaseFileError::NotEditable);
    }

    #[test]
    fn write_rejects_non_md_outside_user_input() {
        let (_fixture, aggregate_root) = kb_fixture();
        let error =
            write_knowledge_base_file(&aggregate_root, "evidence/foo.txt", "x", None, false)
                .expect_err("user-input 之外的非 md 不可写");
        assert_eq!(error, KnowledgeBaseFileError::NotEditable);
    }

    #[test]
    fn write_allows_sql_under_user_input() {
        let (_fixture, aggregate_root) = kb_fixture();
        std::fs::create_dir_all(kb_root(&aggregate_root).join("user-input"))
            .expect("创建 user-input 目录");
        let digest = write_knowledge_base_file(
            &aggregate_root,
            "user-input/database-ddl.sql",
            "CREATE TABLE demo (id INT);",
            None,
            false,
        )
        .expect("user-input 下的 sql 可写");
        assert_eq!(digest.len(), 64);
        assert!(digest.chars().all(|ch| ch.is_ascii_hexdigit()));
        assert_eq!(
            std::fs::read_to_string(kb_root(&aggregate_root).join("user-input/database-ddl.sql"))
                .expect("读取写入结果"),
            "CREATE TABLE demo (id INT);"
        );
    }

    #[test]
    fn write_conflicts_when_digest_mismatches() {
        let (_fixture, aggregate_root) = kb_fixture();
        write_knowledge_base_file(&aggregate_root, "notes.md", "A", None, false)
            .expect("首次写入成功");
        let stale = crate::cross_cutting::document_ops::compute_sha256(b"stale");

        let error = write_knowledge_base_file(
            &aggregate_root,
            "notes.md",
            "B",
            Some(stale.as_str()),
            false,
        )
        .expect_err("摘要不符必须冲突");
        assert_eq!(error, KnowledgeBaseFileError::Conflict);
        assert_eq!(
            std::fs::read_to_string(kb_root(&aggregate_root).join("notes.md")).expect("读取文件"),
            "A"
        );
    }

    #[test]
    fn write_rejected_while_operation_running() {
        let (_fixture, aggregate_root) = kb_fixture();
        let error = write_knowledge_base_file(&aggregate_root, "notes.md", "A", None, true)
            .expect_err("运行期禁写");
        assert_eq!(error, KnowledgeBaseFileError::OperationRunning);
    }

    #[test]
    fn write_is_atomic_and_leaves_no_temp_files() {
        let (_fixture, aggregate_root) = kb_fixture();
        write_knowledge_base_file(&aggregate_root, "notes.md", "A", None, false).expect("写入成功");

        let leftovers: Vec<String> = std::fs::read_dir(kb_root(&aggregate_root))
            .expect("读取知识库根")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "留下临时文件：{leftovers:?}");
        assert_eq!(
            std::fs::read_to_string(kb_root(&aggregate_root).join("notes.md")).expect("读取文件"),
            "A"
        );
    }

    #[test]
    fn write_rejects_when_base_digest_targets_missing_file() {
        let (_fixture, aggregate_root) = kb_fixture();
        let error = write_knowledge_base_file(
            &aggregate_root,
            "notes.md",
            "A",
            Some("0000000000000000000000000000000000000000000000000000000000000000"),
            false,
        )
        .expect_err("给了摘要但文件不存在必须报未找到");
        assert_eq!(error, KnowledgeBaseFileError::NotFound);
    }

    #[test]
    fn write_accepts_matching_base_digest() {
        let (_fixture, aggregate_root) = kb_fixture();
        let first = write_knowledge_base_file(&aggregate_root, "notes.md", "A", None, false)
            .expect("首次写入成功");
        let second = write_knowledge_base_file(
            &aggregate_root,
            "notes.md",
            "B",
            Some(first.as_str()),
            false,
        )
        .expect("摘要一致应允许写入");
        assert_eq!(
            std::fs::read_to_string(kb_root(&aggregate_root).join("notes.md")).expect("读取文件"),
            "B"
        );
        assert_eq!(second.len(), 64);
    }

    #[test]
    fn kb_root_points_at_canonical_aggregate_child() {
        assert_eq!(
            kb_root(std::path::Path::new("/tmp/aggregate")),
            PathBuf::from("/tmp/aggregate/cadence/knowledge-base")
        );
    }

    #[test]
    fn list_reports_editable_flags_for_tree() {
        let (_fixture, aggregate_root) = kb_fixture();
        std::fs::create_dir_all(kb_root(&aggregate_root).join("user-input")).expect("建目录");
        std::fs::write(kb_root(&aggregate_root).join("manifest.yaml"), "schema").expect("写文件");
        std::fs::write(kb_root(&aggregate_root).join("notes.md"), "笔记").expect("写文件");
        std::fs::write(
            kb_root(&aggregate_root).join("user-input/database-ddl.sql"),
            "CREATE TABLE demo (id INT);",
        )
        .expect("写文件");

        let nodes = list_knowledge_base_files(&aggregate_root).expect("列举知识库文件树");
        let editable = |path: &str| {
            nodes
                .iter()
                .find(|node| node.path == path)
                .map(|node| node.editable)
        };
        assert_eq!(editable("notes.md"), Some(true));
        assert_eq!(editable("manifest.yaml"), Some(false));
        assert_eq!(editable("user-input/database-ddl.sql"), Some(true));
        assert!(
            nodes
                .iter()
                .any(|node| node.path == "user-input" && node.is_dir)
        );
    }
}
