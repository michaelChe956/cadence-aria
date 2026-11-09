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
//! 铁律：**只补缺失，绝不覆盖**（用户填的内容优先）；变更包目录已存在时
//! 拒绝新建并返回既有目录（幂等）；变更包标识先做 `CHANGE-` 前缀归一，再
//! 校验只含 `[A-Za-z0-9._-]`（含路径分隔符或 `..` 一律拒绝）。

use std::path::{Path, PathBuf};

use crate::product::json_store::ProductStoreError;

use super::files::kb_root;

/// 初始化输入模板所属 skill 名。
const BOOTSTRAP_SKILL: &str = "knowledge-base-bootstrap";
/// 变更包模板所属 skill 名。
const UPDATE_SKILL: &str = "knowledge-base-update";
/// 变更包目录名前缀（归一后固定带此前缀）。
const CHANGE_PACKAGE_PREFIX: &str = "CHANGE-";
/// 知识库内变更包目录的父目录（相对知识库根）。
const CHANGES_DIR: [&str; 2] = ["user-input", "updates"];

/// 把插件自带的初始化输入模板补到知识库的 `user-input/` 目录。
///
/// 返回本次**新建**的文件路径（相对知识库根，`/` 分隔）；已存在的文件
/// 保持原样，不计入返回值。
pub fn create_bootstrap_input_templates(
    aggregate_root: &Path,
    skills_source_root: &Path,
) -> Result<Vec<String>, ProductStoreError> {
    let source = skill_user_input_dir(skills_source_root, BOOTSTRAP_SKILL)?;
    let root = kb_root(aggregate_root);
    let destination = root.join("user-input");
    std::fs::create_dir_all(&destination).map_err(|error| {
        ProductStoreError::Io(format!("create {}: {error}", destination.display()))
    })?;
    let mut created = Vec::new();
    copy_missing(&source, &destination, &root, &mut created)?;
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
    let normalized = normalize_change_identifier(identifier)?;
    let source = skill_user_input_dir(skills_source_root, UPDATE_SKILL)?.join("change-package");
    let root = kb_root(aggregate_root);
    let mut destination = root.clone();
    for component in CHANGES_DIR {
        destination.push(component);
    }
    destination.push(&normalized);
    if destination.exists() {
        // 已存在 ⇒ 拒绝新建、不覆盖、不改动既有内容。
        return Ok(destination);
    }
    std::fs::create_dir_all(&destination).map_err(|error| {
        ProductStoreError::Io(format!("create {}: {error}", destination.display()))
    })?;
    let mut created = Vec::new();
    copy_missing(&source, &destination, &root, &mut created)?;
    Ok(destination)
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
fn copy_missing(
    source: &Path,
    destination: &Path,
    root: &Path,
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
        let destination_path = destination.join(&name);
        if file_type.is_dir() {
            std::fs::create_dir_all(&destination_path).map_err(|error| {
                ProductStoreError::Io(format!("create {}: {error}", destination_path.display()))
            })?;
            copy_missing(&source_path, &destination_path, root, created)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        if destination_path.exists() {
            // 已存在 ⇒ 绝不覆盖（用户填的内容优先）。
            continue;
        }
        std::fs::copy(&source_path, &destination_path).map_err(|error| {
            ProductStoreError::Io(format!(
                "copy {} -> {}: {error}",
                source_path.display(),
                destination_path.display()
            ))
        })?;
        created.push(relative_display(root, &destination_path));
    }
    Ok(())
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
mod knowledge_base_templates_tests {
    use super::{create_bootstrap_input_templates, create_change_package_template};
    use crate::product::json_store::ProductStoreError;
    use std::path::{Path, PathBuf};

    /// 初始化输入模板的九份源文件（与插件 `knowledge-base-bootstrap/user-input/`
    /// 的实际构成一致）。
    const BOOTSTRAP_FILES: [&str; 9] = [
        "api-scope.md",
        "base-info.md",
        "configuration-scope.md",
        "data-model-scope.md",
        "database-ddl.sql",
        "middleware-scope.md",
        "page-scope.md",
        "product.md",
        "project-scope.md",
    ];

    /// 变更包模板的五份文档（与插件 `knowledge-base-update/user-input/change-package/`
    /// 的实际构成一致）。
    const CHANGE_PACKAGE_FILES: [&str; 5] = [
        "change-summary.md",
        "code-change.md",
        "configuration-change.md",
        "database-change.md",
        "verification.md",
    ];

    /// 建一份插件源根夹具（只落 /tmp）。
    ///
    /// 返回 `(夹具, 传给函数的源根)`：`"skills"` ⇒ `<root>/skills`（等价
    /// `CadenceSkillsPaths::source_root()` 形态）；`"repository"` ⇒ `<root>`
    /// （仓库根形态，函数内部再拼 `cadence-init/skills`）。
    fn skills_source_fixture(layout: &str) -> (tempfile::TempDir, PathBuf) {
        let fixture = tempfile::TempDir::new().expect("创建 /tmp 夹具");
        let root = fixture.path().to_path_buf();
        let (source_root, skills_root) = if layout == "repository" {
            (root.clone(), root.join("cadence-init").join("skills"))
        } else {
            (root.join("skills"), root.join("skills"))
        };
        let bootstrap = skills_root
            .join("knowledge-base-bootstrap")
            .join("user-input");
        std::fs::create_dir_all(&bootstrap).expect("创建 bootstrap 源目录");
        for name in BOOTSTRAP_FILES {
            std::fs::write(bootstrap.join(name), format!("模板原文 {name}")).expect("写源模板");
        }
        let change_package = skills_root
            .join("knowledge-base-update")
            .join("user-input")
            .join("change-package");
        std::fs::create_dir_all(change_package.join("attachments")).expect("创建变更包源目录");
        for name in CHANGE_PACKAGE_FILES {
            std::fs::write(change_package.join(name), format!("变更包原文 {name}"))
                .expect("写源模板");
        }
        std::fs::write(
            change_package.join("attachments").join("README.md"),
            "附件说明",
        )
        .expect("写附件说明");
        (fixture, source_root)
    }

    /// 知识库聚合根夹具（只落 /tmp）。
    fn aggregate_fixture() -> (tempfile::TempDir, PathBuf) {
        let fixture = tempfile::TempDir::new().expect("创建 /tmp 夹具");
        let aggregate_root = fixture.path().join("aggregate-root");
        std::fs::create_dir_all(&aggregate_root).expect("创建聚合根");
        (fixture, aggregate_root)
    }

    #[test]
    fn create_bootstrap_input_templates_does_not_overwrite_existing() {
        let (_skills_fixture, skills_root) = skills_source_fixture("skills");
        let (_aggregate_fixture, aggregate_root) = aggregate_fixture();
        let user_input = aggregate_root.join("cadence/knowledge-base/user-input");
        std::fs::create_dir_all(&user_input).expect("创建 user-input 目录");
        std::fs::write(user_input.join("base-info.md"), "用户已填写的内容").expect("预置用户内容");

        let created =
            create_bootstrap_input_templates(&aggregate_root, &skills_root).expect("创建模板成功");

        assert_eq!(
            std::fs::read_to_string(user_input.join("base-info.md")).expect("读取 base-info.md"),
            "用户已填写的内容",
            "已存在文件不得被覆盖"
        );
        assert!(user_input.join("api-scope.md").exists(), "缺失模板应被创建");
        assert!(
            created.iter().any(|path| path == "user-input/api-scope.md"),
            "新建清单应含 api-scope.md：{created:?}"
        );
        assert!(
            !created.iter().any(|path| path == "user-input/base-info.md"),
            "已存在文件不应计入新建清单：{created:?}"
        );
    }

    #[test]
    fn create_bootstrap_input_templates_creates_all_nine_inputs() {
        let (_skills_fixture, skills_root) = skills_source_fixture("skills");
        let (_aggregate_fixture, aggregate_root) = aggregate_fixture();

        let created =
            create_bootstrap_input_templates(&aggregate_root, &skills_root).expect("创建模板成功");

        assert_eq!(created.len(), BOOTSTRAP_FILES.len());
        let user_input = aggregate_root.join("cadence/knowledge-base/user-input");
        for name in BOOTSTRAP_FILES {
            assert!(user_input.join(name).exists(), "输入模板缺失：{name}");
        }
    }

    #[test]
    fn create_change_package_template_normalizes_identifier() {
        let (_skills_fixture, skills_root) = skills_source_fixture("skills");
        let (_aggregate_fixture, aggregate_root) = aggregate_fixture();

        let directory =
            create_change_package_template(&aggregate_root, &skills_root, "login-refactor")
                .expect("创建变更包模板成功");

        assert_eq!(
            directory
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            Some("CHANGE-login-refactor".to_string())
        );
        for name in CHANGE_PACKAGE_FILES {
            assert!(directory.join(name).exists(), "变更包文档缺失：{name}");
        }
    }

    #[test]
    fn create_change_package_template_keeps_existing_prefix() {
        let (_skills_fixture, skills_root) = skills_source_fixture("skills");
        let (_aggregate_fixture, aggregate_root) = aggregate_fixture();

        let directory =
            create_change_package_template(&aggregate_root, &skills_root, "CHANGE-20261109-0001")
                .expect("创建变更包模板成功");

        assert_eq!(
            directory
                .file_name()
                .map(|name| name.to_string_lossy().into_owned()),
            Some("CHANGE-20261109-0001".to_string())
        );
    }

    #[test]
    fn create_change_package_template_rejects_unsafe_identifier() {
        let (_skills_fixture, skills_root) = skills_source_fixture("skills");
        let (_aggregate_fixture, aggregate_root) = aggregate_fixture();

        let error = create_change_package_template(&aggregate_root, &skills_root, "../evil")
            .expect_err("非法标识必须被拒绝");
        assert!(
            matches!(error, ProductStoreError::InvalidRecord { .. }),
            "非法标识应报不可记录的标识：{error:?}"
        );
    }

    #[test]
    fn create_change_package_template_is_idempotent() {
        let (_skills_fixture, skills_root) = skills_source_fixture("skills");
        let (_aggregate_fixture, aggregate_root) = aggregate_fixture();

        let first = create_change_package_template(&aggregate_root, &skills_root, "login-refactor")
            .expect("首次创建成功");
        std::fs::write(first.join("change-summary.md"), "用户已填写").expect("预置用户内容");

        let second =
            create_change_package_template(&aggregate_root, &skills_root, "login-refactor")
                .expect("二次创建应幂等");

        assert_eq!(first, second, "二次创建应返回同一目录");
        assert_eq!(
            std::fs::read_to_string(first.join("change-summary.md")).expect("读取变更包文档"),
            "用户已填写",
            "二次创建不得覆盖既有内容"
        );
    }

    #[test]
    fn create_templates_reports_io_when_skills_source_missing() {
        let (_aggregate_fixture, aggregate_root) = aggregate_fixture();
        let missing = Path::new("/tmp/cadence-kb-missing-skills-source");

        let error = create_bootstrap_input_templates(&aggregate_root, missing)
            .expect_err("插件源根缺失必须报 IO 错误");
        assert!(matches!(error, ProductStoreError::Io(_)), "{error:?}");

        let error = create_change_package_template(&aggregate_root, missing, "login-refactor")
            .expect_err("插件源根缺失必须报 IO 错误");
        assert!(matches!(error, ProductStoreError::Io(_)), "{error:?}");
    }

    #[test]
    fn create_templates_accepts_repository_layout_source_root() {
        let (_skills_fixture, skills_root) = skills_source_fixture("repository");
        let (_aggregate_fixture, aggregate_root) = aggregate_fixture();

        let created =
            create_bootstrap_input_templates(&aggregate_root, &skills_root).expect("创建模板成功");
        assert!(!created.is_empty(), "应新建模板文件");
    }
}
