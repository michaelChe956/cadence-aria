//! 知识库模板骨架（Task 5）。
//!
//! 两类模板都来自插件源根下的 Cadence-skills 模板（`knowledge-base-bootstrap`
//! 与 `knowledge-base-update` 两个 skill 的 `user-input/`）：
//! - 初始化输入模板 → 落地到 `<聚合根>/cadence/knowledge-base/user-input/`；
//! - 变更包模板 → 落地到
//!   `<聚合根>/cadence/knowledge-base/user-input/updates/CHANGE-<标识>/`。
//!
//! 插件源根既接受 `CadenceSkillsPaths::source_root()` 形态
//! （`~/.agents/Cadence-skills/cadence-init/skills`），也接受仓库根形态
//! （`~/.agents/Cadence-skills`，内部再拼 `cadence-init/skills`）——两种形态
//! 都按既有目录探测，探测不到 ⇒ `ProductStoreError::Io`（由上层映射为
//! skills 不可用）。
//!
//! 铁律：
//! - **只补缺失，绝不覆盖**（用户填的内容优先）：目标文件以
//!   `O_CREAT|O_EXCL` 排他创建，已存在（含悬空符号链接）一律跳过 ⇒ 不存在
//!   「先看存在再复制」的窗口，也不会跟随悬空链接改写根外文件；
//! - **目标树受根内约束**：目标目录逐段 `openat` + `O_NOFOLLOW` 打开 ⇒ 目标
//!   侧若是指向根外的目录链接，模板不会复制到根外；
//! - **变更包先暂存后发布**：先在独占暂存目录里完成复制，再**原子不替换**
//!   发布（`renameat2(RENAME_NOREPLACE)`）⇒ 复制失败不留下会触发幂等分支的
//!   半成品变更包；并发下别人先发布时按**幂等成功**处理（既有内容不被覆盖，
//!   空目录也不被替换）；暂存名**与标识长度无关**（否则 255 字节的合法标识
//!   会让暂存名超限）；
//! - **发布必须原子**：不支持 `RENAME_NOREPLACE` 的文件系统上**直接报错**，
//!   不用「先建占位空目录再改名」冒充发布（占位目录会被并发观察者误判成
//!   已发布的变更包，且改名失败时会留下永久空目录）；
//! - **碰撞目标必须是真实目录**：`EEXIST` 只说明名字被占用，占用者是普通
//!   文件或符号链接时报错（不得当成「已发布」返回）。
//! - 变更包目录已存在时拒绝新建并返回既有目录（幂等、不覆盖既有内容）；
//! - 变更包标识先做 `CHANGE-` 前缀归一，再校验只含 `[A-Za-z0-9._-]`（含路径
//!   分隔符或 `..` 一律拒绝）。

use std::ffi::OsStr;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};

use crate::product::json_store::ProductStoreError;

use super::files::{
    KnowledgeBaseRoot, create_new_at, kb_root, mkdirat, openat_dir, remove_tree_at,
    rename_noreplace_at, walk_dirs,
};

/// 初始化输入模板所属 skill 名。
const BOOTSTRAP_SKILL: &str = "knowledge-base-bootstrap";
/// 变更包模板所属 skill 名。
const UPDATE_SKILL: &str = "knowledge-base-update";
/// 变更包目录名前缀（归一后固定带此前缀）。
const CHANGE_PACKAGE_PREFIX: &str = "CHANGE-";
/// 知识库内变更包目录的父目录（相对知识库根）。
const CHANGES_DIR: [&str; 2] = ["user-input", "updates"];
/// 变更包暂存目录换名重试上限（暂存名撞名时重试）。
const STAGING_NAME_ATTEMPTS: usize = 64;

/// 把插件自带的初始化输入模板补到知识库的 `user-input/` 目录。
///
/// 返回本次**新建**的文件路径（相对知识库根，`/` 分隔）；已存在的文件
/// 保持原样，不计入返回值。
pub fn create_bootstrap_input_templates(
    aggregate_root: &Path,
    skills_source_root: &Path,
) -> Result<Vec<String>, ProductStoreError> {
    let source = skill_user_input_dir(skills_source_root, BOOTSTRAP_SKILL)?;
    let root = KnowledgeBaseRoot::open_or_create(aggregate_root)?;
    // 目标目录同样受根内约束：`user-input` 若是符号链接 ⇒ 拒绝（不复制到根外）。
    let destination = walk_dirs(&root.fd, &["user-input".to_string()], true)
        .map_err(|error| ProductStoreError::Io(format!("准备 user-input 目录失败：{error}")))?;
    let mut created = Vec::new();
    copy_missing(&source, &destination, "user-input", &mut created)?;
    Ok(created)
}

/// 把插件自带的变更包模板补到知识库的 `user-input/updates/CHANGE-<标识>/`。
///
/// 目录已存在 ⇒ 拒绝新建、返回既有目录且不改动其中任何文件（幂等）。
pub fn create_change_package_template(
    aggregate_root: &Path,
    skills_source_root: &Path,
    identifier: &str,
) -> Result<PathBuf, ProductStoreError> {
    create_change_package_template_with(
        aggregate_root,
        skills_source_root,
        identifier,
        rename_noreplace_at,
    )
}

/// `create_change_package_template` 的可注入版本（`rename` 发布原语可替换）。
///
/// 注入点只用于回归测试：覆盖「文件系统不支持 `RENAME_NOREPLACE`」与
/// 「发布瞬间目标被并发占用成非目录」两条分支。
fn create_change_package_template_with(
    aggregate_root: &Path,
    skills_source_root: &Path,
    identifier: &str,
    rename: RenameFn,
) -> Result<PathBuf, ProductStoreError> {
    let normalized = normalize_change_identifier(identifier)?;
    let source = skill_user_input_dir(skills_source_root, UPDATE_SKILL)?.join("change-package");
    let root = KnowledgeBaseRoot::open_or_create(aggregate_root)?;
    let parent_components: Vec<String> = CHANGES_DIR.iter().map(|name| name.to_string()).collect();
    let parent = walk_dirs(&root.fd, &parent_components, true)
        .map_err(|error| ProductStoreError::Io(format!("准备变更包父目录失败：{error}")))?;
    let destination = kb_root(aggregate_root)
        .join(CHANGES_DIR[0])
        .join(CHANGES_DIR[1])
        .join(&normalized);
    match openat_dir(&parent, OsStr::new(&normalized)) {
        Ok(_) => {
            // 已存在 ⇒ 拒绝新建、不覆盖、不改动既有内容（幂等）。
            return Ok(destination);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) if error.raw_os_error() == Some(libc::ELOOP) => {
            return Err(ProductStoreError::Io(format!(
                "变更包目录已存在但不是真实目录：{normalized}"
            )));
        }
        Err(error) => {
            return Err(ProductStoreError::Io(format!(
                "检查变更包目录失败：{error}"
            )));
        }
    }
    // 独占暂存目录：复制全部完成后才改名发布 ⇒ 复制失败不留下会触发幂等
    // 分支的半成品变更包。
    let (staging, staging_dir) = create_staging_dir(&parent)?;
    let prefix = format!("{}/{normalized}", CHANGES_DIR.join("/"));
    let mut created = Vec::new();
    if let Err(error) = copy_missing(&source, &staging_dir, &prefix, &mut created) {
        let _ = remove_tree_at(&parent, OsStr::new(&staging));
        return Err(error);
    }
    drop(staging_dir);
    // 原子不替换发布：并发下别人先发布 ⇒ 幂等成功（不覆盖其内容）；普通
    // `rename` 会因目标非空而失败、空目标则被替换 ⇒ 不能用。
    match publish_staging_with(&parent, &staging, &normalized, rename) {
        Ok(true) => Ok(destination),
        Ok(false) => {
            let _ = remove_tree_at(&parent, OsStr::new(&staging));
            Ok(destination)
        }
        Err(error) => {
            let _ = remove_tree_at(&parent, OsStr::new(&staging));
            Err(error)
        }
    }
}

/// 在 `parent` 下创建**独占**暂存目录（撞名换名重试），返回 `(名字, 目录 fd)`。
fn create_staging_dir(parent: &OwnedFd) -> Result<(String, OwnedFd), ProductStoreError> {
    for _ in 0..STAGING_NAME_ATTEMPTS {
        let name = staging_name();
        match mkdirat(parent, OsStr::new(&name)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "创建变更包暂存目录失败：{error}"
                )));
            }
        }
        let fd = openat_dir(parent, OsStr::new(&name))
            .map_err(|error| ProductStoreError::Io(format!("打开变更包暂存目录失败：{error}")))?;
        return Ok((name, fd));
    }
    Err(ProductStoreError::Io(
        "创建变更包暂存目录失败：连续撞名".to_string(),
    ))
}

/// 变更包暂存目录名（**与标识长度无关**的固定长度短名）。
///
/// 暂存名若带上标识，248 字节的合法标识（最终目录名 255 字节，已达单文件名
/// 上限）会让暂存名必然超限 ⇒ `ENAMETOOLONG`，原本可创建的合法标识反而失败。
/// 故暂存名只带进程号与自增序号，长度恒定。
fn staging_name() -> String {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let sequence = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!(".kb-staging-{}-{sequence}", std::process::id())
}

/// 改名发布原语（可注入，便于回归测试覆盖「不支持 `RENAME_NOREPLACE`」分支）。
type RenameFn = fn(&OwnedFd, &OsStr, &OsStr) -> std::io::Result<()>;

/// 发布暂存目录到最终目录（`rename` 原语可注入）。
///
/// 三条硬约束：
/// 1. **原子不替换**：用 `renameat2(RENAME_NOREPLACE)` 发布，目标已存在时报
///    `EEXIST`（**不像 `renameat` 那样替换空目录**）⇒ 不会被并发写入悄悄覆盖。
/// 2. **碰撞目标必须是真实目录**：`EEXIST` 只说明「这个名字已被占用」，**不**
///    说明占用者是目录。碰撞后必须以父目录 fd 做 **no-follow 目录核验**
///    （`openat(O_DIRECTORY|O_NOFOLLOW)`）：占用者是普通文件或符号链接 ⇒
///    **不得**判成幂等成功（否则会把一个文件/越界链接当成已发布的变更包
///    返回），一律报 `Io`。
/// 3. **不支持 `RENAME_NOREPLACE` 时直接报错**：降级路径（先建占位空目录再
///    改名）会在「占位」与「发布」之间暴露一个**未发布的空目录** ⇒ 既不是
///    原子发布，又会让并发观察者把占位目录误判成「已发布的变更包」（幂等
///    分支），甚至在改名失败时留下永久占位目录。故此处**不做降级**，宁可让
///    该平台上的变更包创建失败并给出可读原因，也不返回半成品或误报成功。
fn publish_staging_with(
    parent: &OwnedFd,
    staging: &str,
    target: &str,
    rename: RenameFn,
) -> Result<bool, ProductStoreError> {
    match rename(parent, OsStr::new(staging), OsStr::new(target)) {
        Ok(()) => Ok(true),
        // 目标已存在（`RENAME_NOREPLACE` ⇒ `EEXIST`；部分实现给 `ENOTEMPTY`）
        // ⇒ 可能是并发发布 ⇒ 必须先核验占用者确实是**真实目录**。
        Err(error) if is_target_exists(&error) => {
            match openat_dir(parent, OsStr::new(target)) {
                // 真实目录 ⇒ 并发已被发布 ⇒ 幂等成功（既有内容不被覆盖）。
                Ok(_) => Ok(false),
                // 普通文件 / 符号链接 / 其它非目录 ⇒ 不得判成幂等成功。
                Err(probe) => Err(ProductStoreError::Io(format!(
                    "变更包目录已存在但不是真实目录：{target}：{probe}"
                ))),
            }
        }
        // 内核/文件系统不支持 `RENAME_NOREPLACE` ⇒ 明确报错（**不降级**，
        // 避免暴露未发布的占位目录，见上文约束 3）。
        Err(error)
            if error.raw_os_error() == Some(libc::ENOSYS)
                || error.raw_os_error() == Some(libc::EINVAL) =>
        {
            Err(ProductStoreError::Io(format!(
                "当前文件系统不支持原子不替换发布（RENAME_NOREPLACE）：{error}"
            )))
        }
        Err(error) => Err(ProductStoreError::Io(format!(
            "发布变更包目录失败：{error}"
        ))),
    }
}

/// 目标已存在（并发发布）判定。
fn is_target_exists(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(libc::EEXIST)
        || error.raw_os_error() == Some(libc::ENOTEMPTY)
        || error.kind() == std::io::ErrorKind::AlreadyExists
}

/// 标识归一：先剥掉已有的 `CHANGE-` 前缀（大小写不敏感），再补规范前缀；
/// 归一后只允许 `[A-Za-z0-9._-]`，且不含 `..` 与路径分隔符。
fn normalize_change_identifier(identifier: &str) -> Result<String, ProductStoreError> {
    let trimmed = identifier.trim();
    if trimmed.is_empty() {
        return Err(invalid_identifier(identifier, "标识为空"));
    }
    let prefix_bytes = CHANGE_PACKAGE_PREFIX.len();
    let body = match trimmed.get(..prefix_bytes) {
        Some(prefix) if prefix.eq_ignore_ascii_case(CHANGE_PACKAGE_PREFIX) => {
            &trimmed[prefix_bytes..]
        }
        _ => trimmed,
    };
    if body.is_empty() {
        return Err(invalid_identifier(
            identifier,
            "标识在 `CHANGE-` 前缀之后为空",
        ));
    }
    if body == "." || body.contains("..") {
        return Err(invalid_identifier(identifier, "标识不得包含 `..`"));
    }
    if body.contains('/') || body.contains('\\') {
        return Err(invalid_identifier(identifier, "标识不得包含路径分隔符"));
    }
    if !body
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '.' || ch == '_' || ch == '-')
    {
        return Err(invalid_identifier(
            identifier,
            "标识只允许字母、数字、点、下划线与连字符",
        ));
    }
    Ok(format!("{CHANGE_PACKAGE_PREFIX}{body}"))
}

/// 非法标识的统一错误形态。
fn invalid_identifier(identifier: &str, reason: &str) -> ProductStoreError {
    ProductStoreError::InvalidRecord {
        kind: "knowledge_base_change_package_identifier",
        reason: format!("{reason}：{identifier}"),
    }
}

/// 定位 skill 的 `user-input/` 源目录（两种源根形态都支持）。
fn skill_user_input_dir(
    skills_source_root: &Path,
    skill: &str,
) -> Result<PathBuf, ProductStoreError> {
    let candidates = [
        skills_source_root.join(skill).join("user-input"),
        skills_source_root
            .join("cadence-init")
            .join("skills")
            .join(skill)
            .join("user-input"),
    ];
    for candidate in &candidates {
        if candidate.is_dir() {
            return Ok(candidate.clone());
        }
    }
    Err(ProductStoreError::Io(format!(
        "知识库模板源不可用：{} 下找不到 {} 的 user-input 目录",
        skills_source_root.display(),
        skill
    )))
}

/// 只复制缺失的文件（已存在的一律保留用户内容），并记录新建的相对路径。
///
/// 目标侧全部走目录 fd（`openat` + `O_NOFOLLOW`）：目录逐段打开（符号链接
/// 一律拒绝）、文件以 `O_CREAT|O_EXCL` 排他创建 ⇒ 不存在「先看存在再复制」
/// 的窗口，悬空符号链接也不会被跟随。
fn copy_missing(
    source: &Path,
    destination: &OwnedFd,
    prefix: &str,
    created: &mut Vec<String>,
) -> Result<(), ProductStoreError> {
    if !source.is_dir() {
        return Err(ProductStoreError::Io(format!(
            "知识库模板源不可用：{} 不是目录",
            source.display()
        )));
    }
    let entries = std::fs::read_dir(source).map_err(|error| {
        ProductStoreError::Io(format!("read_dir {}: {error}", source.display()))
    })?;
    for entry in entries {
        let entry =
            entry.map_err(|error| ProductStoreError::Io(format!("read_dir entry: {error}")))?;
        let file_type = entry.file_type().map_err(|error| {
            ProductStoreError::Io(format!("file_type {}: {error}", entry.path().display()))
        })?;
        if file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name();
        let source_path = entry.path();
        if file_type.is_dir() {
            match mkdirat(destination, &name) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(ProductStoreError::Io(format!(
                        "create {}: {error}",
                        source_path.display()
                    )));
                }
            }
            let child = openat_dir(destination, &name).map_err(|error| {
                ProductStoreError::Io(format!(
                    "打开目标目录 {} 失败：{error}",
                    source_path.display()
                ))
            })?;
            copy_missing(
                &source_path,
                &child,
                &format!("{prefix}/{}", name.to_string_lossy()),
                created,
            )?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let content = std::fs::read(&source_path).map_err(|error| {
            ProductStoreError::Io(format!("read {}: {error}", source_path.display()))
        })?;
        match create_new_at(destination, &name, &content) {
            Ok(()) => created.push(format!("{prefix}/{}", name.to_string_lossy())),
            // 已存在（含悬空符号链接）⇒ 跳过，绝不覆盖用户内容。
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "copy {} -> {prefix}/{}: {error}",
                    source_path.display(),
                    name.to_string_lossy()
                )));
            }
        }
    }
    Ok(())
}

include!("templates_tests.inc.rs");
