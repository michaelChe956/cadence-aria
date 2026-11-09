//! 知识库模板骨架（Task 5）。
//!
//! 两类模板都来自插件源根下的 Cadence-skills 模板（`knowledge-base-bootstrap`
//! 与 `knowledge-base-update` 两个 skill 的 `user-input/`）：
//! - 初始化输入模板 → 落地到 `<聚合根>/cadence/knowledge-base/user-input/`；
//! - 变更包模板 → 落地到
//!   `<聚合根>/cadence/knowledge-base/user-input/updates/CHANGE-<标识>/`。
//!
//! 铁律：**只补缺失，绝不覆盖**（用户填的内容优先）；变更包目录已存在时
//! 拒绝新建并返回既有目录（幂等）；变更包标识先做 `CHANGE-` 前缀归一，再
//! 校验只含 `[A-Za-z0-9._-]`（含路径分隔符或 `..` 一律拒绝）。
//!
//! 插件源根缺失（Cadence-skills 未装）⇒ `ProductStoreError::Io`，由上层
//! 映射为 skills 不可用。

use std::path::{Path, PathBuf};

use crate::product::json_store::ProductStoreError;

/// 初始化输入模板所属 skill 名。
const BOOTSTRAP_SKILL: &str = "knowledge-base-bootstrap";
/// 变更包模板所属 skill 名。
const UPDATE_SKILL: &str = "knowledge-base-update";
/// 变更包目录名前缀（归一后固定带此前缀）。
const CHANGE_PACKAGE_PREFIX: &str = "CHANGE-";

/// 把插件自带的初始化输入模板补到知识库的 `user-input/` 目录。
///
/// 返回本次**新建**的文件路径（相对知识库根，`/` 分隔）；已存在的文件
/// 保持原样，不计入返回值。
pub fn create_bootstrap_input_templates(
    aggregate_root: &Path,
    skills_source_root: &Path,
) -> Result<Vec<String>, ProductStoreError> {
    let _ = (aggregate_root, skills_source_root);
    unimplemented!("Task 5：初始化输入模板创建待实现");
}

/// 把插件自带的变更包模板补到知识库的 `user-input/updates/CHANGE-<标识>/`。
///
/// 目录已存在 ⇒ 拒绝新建、返回既有目录且不改动其中任何文件（幂等）。
pub fn create_change_package_template(
    aggregate_root: &Path,
    skills_source_root: &Path,
    identifier: &str,
) -> Result<PathBuf, ProductStoreError> {
    let _ = (aggregate_root, skills_source_root, identifier);
    unimplemented!("Task 5：变更包模板创建待实现");
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
    /// `layout` 决定 skill 目录的层级：`"skills"` ⇒ `<root>/<skill>/`，
    /// `"repository"` ⇒ `<root>/cadence-init/skills/<skill>/`。
    fn skills_source_fixture(layout: &str) -> (tempfile::TempDir, PathBuf) {
        let fixture = tempfile::TempDir::new().expect("创建 /tmp 夹具");
        let root = fixture.path().to_path_buf();
        let skills_root = if layout == "repository" {
            root.join("cadence-init").join("skills")
        } else {
            root.join("skills")
        };
        let bootstrap = skills_root.join("knowledge-base-bootstrap").join("user-input");
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
        (fixture, skills_root)
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
        assert!(
            user_input.join("api-scope.md").exists(),
            "缺失模板应被创建"
        );
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
    fn create_change_package_template_normalizes_identifier() {
        let (_skills_fixture, skills_root) = skills_source_fixture("skills");
        let (_aggregate_fixture, aggregate_root) = aggregate_fixture();

        let directory =
            create_change_package_template(&aggregate_root, &skills_root, "login-refactor")
                .expect("创建变更包模板成功");

        assert_eq!(
            directory.file_name().map(|name| name.to_string_lossy().into_owned()),
            Some("CHANGE-login-refactor".to_string())
        );
        for name in CHANGE_PACKAGE_FILES {
            assert!(
                directory.join(name).exists(),
                "变更包文档缺失：{name}"
            );
        }
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

        let first =
            create_change_package_template(&aggregate_root, &skills_root, "login-refactor")
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
