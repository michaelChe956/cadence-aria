// `templates.rs` 的内联测试（按仓内惯例拆 `*_tests.inc.rs` + `include!`）。
// 夹具一律落 /tmp（tempfile::TempDir）；符号链接用例仅在 unix 下成立。

#[cfg(test)]
mod knowledge_base_templates_tests {
    use super::{
        create_bootstrap_input_templates, create_change_package_template,
        create_change_package_template_with, publish_staging_with, rename_noreplace_at,
    };
    use crate::product::json_store::ProductStoreError;
    use std::ffi::OsStr;
    use std::os::fd::{AsRawFd, OwnedFd};
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

    // -----------------------------------------------------------------------
    // 修复轮 1 回归用例（Critical 4 + Important 6/7）
    // -----------------------------------------------------------------------

    /// Critical 4：目标 `user-input` 是指向根外的目录链接时，模板不得复制到根外。
    #[cfg(unix)]
    #[test]
    fn create_bootstrap_input_templates_rejects_symlinked_target_dir() {
        let (_skills_fixture, skills_root) = skills_source_fixture("skills");
        let (aggregate_fixture, aggregate_root) = aggregate_fixture();
        let outside = aggregate_fixture.path().join("outside");
        std::fs::create_dir_all(&outside).expect("建根外目录");
        let knowledge_base = aggregate_root.join("cadence/knowledge-base");
        std::fs::create_dir_all(&knowledge_base).expect("建知识库根");
        std::os::unix::fs::symlink(&outside, knowledge_base.join("user-input"))
            .expect("创建越界目录链接");

        let error = create_bootstrap_input_templates(&aggregate_root, &skills_root)
            .expect_err("目标目录是越界符号链接时必须被拒绝");
        assert!(matches!(error, ProductStoreError::Io(_)), "{error:?}");
        assert_eq!(
            std::fs::read_dir(&outside).expect("读取根外目录").count(),
            0,
            "模板不得复制到根外"
        );
    }

    /// Critical 4：变更包父目录（`user-input`）是根外链接时，变更包不得落到根外。
    #[cfg(unix)]
    #[test]
    fn create_change_package_template_rejects_symlinked_parent_dir() {
        let (_skills_fixture, skills_root) = skills_source_fixture("skills");
        let (aggregate_fixture, aggregate_root) = aggregate_fixture();
        let outside = aggregate_fixture.path().join("outside");
        std::fs::create_dir_all(&outside).expect("建根外目录");
        let knowledge_base = aggregate_root.join("cadence/knowledge-base");
        std::fs::create_dir_all(&knowledge_base).expect("建知识库根");
        std::os::unix::fs::symlink(&outside, knowledge_base.join("user-input"))
            .expect("创建越界目录链接");

        let error = create_change_package_template(&aggregate_root, &skills_root, "login-refactor")
            .expect_err("父目录是越界符号链接时必须被拒绝");
        assert!(matches!(error, ProductStoreError::Io(_)), "{error:?}");
        assert!(!outside.join("updates").exists(), "变更包不得复制到根外");
    }

    /// Important 6：目标位置是悬空符号链接时不得跟随（不得在根外创建文件），
    /// 且该链接不得被替换。
    #[cfg(unix)]
    #[test]
    fn create_bootstrap_input_templates_does_not_follow_dangling_symlink() {
        let (_skills_fixture, skills_root) = skills_source_fixture("skills");
        let (aggregate_fixture, aggregate_root) = aggregate_fixture();
        let outside = aggregate_fixture.path().join("outside");
        std::fs::create_dir_all(&outside).expect("建根外目录");
        let user_input = aggregate_root.join("cadence/knowledge-base/user-input");
        std::fs::create_dir_all(&user_input).expect("建 user-input 目录");
        let link = user_input.join("base-info.md");
        std::os::unix::fs::symlink(outside.join("base-info.md"), &link).expect("预置悬空链接");

        create_bootstrap_input_templates(&aggregate_root, &skills_root).expect("创建模板成功");

        assert!(
            link.symlink_metadata()
                .expect("读取链接元数据")
                .file_type()
                .is_symlink(),
            "悬空链接不得被替换成普通文件"
        );
        assert!(
            !outside.join("base-info.md").exists(),
            "不得跟随悬空链接在根外创建文件"
        );
    }

    /// Important 7：复制失败不得留下半成品变更包目录；重试必须补齐全部文档。
    #[cfg(unix)]
    #[test]
    fn create_change_package_template_leaves_no_partial_package_on_failure() {
        let (_skills_fixture, skills_root) = skills_source_fixture("skills");
        let (_aggregate_fixture, aggregate_root) = aggregate_fixture();
        let secret =
            skills_root.join("knowledge-base-update/user-input/change-package/verification.md");
        std::fs::set_permissions(
            &secret,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o000),
        )
        .expect("设置源文件权限");
        if unsafe { libc::geteuid() } == 0 {
            // root 绕过权限位 ⇒ 无法注入复制失败，只验证正常路径。
            std::fs::set_permissions(
                &secret,
                <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o644),
            )
            .expect("恢复源文件权限");
            return;
        }

        let error = create_change_package_template(&aggregate_root, &skills_root, "login-refactor")
            .expect_err("复制失败必须报错");
        assert!(matches!(error, ProductStoreError::Io(_)), "{error:?}");
        let changes = aggregate_root.join("cadence/knowledge-base/user-input/updates");
        assert!(
            !changes.join("CHANGE-login-refactor").exists(),
            "失败后不得留下半成品变更包目录"
        );
        assert_eq!(
            std::fs::read_dir(&changes)
                .expect("读取变更包父目录")
                .filter_map(|entry| entry.ok())
                .count(),
            0,
            "失败后不得留下暂存目录"
        );

        std::fs::set_permissions(
            &secret,
            <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o644),
        )
        .expect("恢复源文件权限");
        let directory =
            create_change_package_template(&aggregate_root, &skills_root, "login-refactor")
                .expect("重试必须成功");
        for name in CHANGE_PACKAGE_FILES {
            assert!(
                directory.join(name).exists(),
                "重试后变更包文档缺失：{name}"
            );
        }
    }

    // -----------------------------------------------------------------------
    // 修复轮 2 回归用例（Important 2 / Important 3）
    // -----------------------------------------------------------------------

    /// Important 2：发布必须**原子不替换** —— 目标已存在（含空目录）时不得被
    /// 替换，且按「并发已被发布」判成幂等成功。
    #[test]
    fn publish_staging_does_not_replace_existing_target() {
        use std::os::fd::OwnedFd;

        let fixture = tempfile::TempDir::new().expect("创建 /tmp 夹具");
        let parent_path = fixture.path().to_path_buf();
        let parent_file = std::fs::File::open(&parent_path).expect("打开父目录");
        let parent: OwnedFd = parent_file.into();

        // 情形一：目标不存在 ⇒ 发布成功，内容落在目标目录。
        std::fs::create_dir(parent_path.join("staging")).expect("建暂存目录");
        std::fs::write(parent_path.join("staging/change-summary.md"), "变更包原文")
            .expect("写暂存内容");
        assert!(
            publish_staging_with(&parent, "staging", "target", rename_noreplace_at)
                .expect("发布不得报错"),
            "目标不存在时必须发布成功"
        );
        assert!(
            parent_path.join("target/change-summary.md").exists(),
            "发布后内容必须落在目标目录"
        );
        assert!(
            !parent_path.join("staging").exists(),
            "发布成功后暂存目录必须消失"
        );

        // 情形二：目标已存在（**空目录**）⇒ 幂等成功，且目标不得被替换。
        std::fs::create_dir(parent_path.join("staging2")).expect("建暂存目录");
        std::fs::write(parent_path.join("staging2/change-summary.md"), "并发内容")
            .expect("写暂存内容");
        std::fs::create_dir(parent_path.join("target2")).expect("建已存在的空目标目录");
        assert!(
            !publish_staging_with(&parent, "staging2", "target2", rename_noreplace_at)
                .expect("发布不得报错"),
            "目标已存在时必须判成「并发已发布」（幂等）"
        );
        assert!(
            parent_path
                .join("target2")
                .read_dir()
                .expect("读取目标目录")
                .next()
                .is_none(),
            "已存在的空目标目录不得被替换成暂存内容"
        );
        assert!(
            parent_path.join("staging2").exists(),
            "未发布的暂存目录必须保留（由调用方清理）"
        );
    }

    /// Important 2：同一标识的并发创建都必须成功（幂等），不得因发布竞态报错。
    #[test]
    fn create_change_package_template_is_idempotent_under_concurrency() {
        use std::sync::{Arc, Barrier};

        let (_skills_fixture, skills_root) = skills_source_fixture("skills");
        let (_aggregate_fixture, aggregate_root) = aggregate_fixture();
        let barrier = Arc::new(Barrier::new(2));

        let mut handles = Vec::new();
        for _ in 0..2 {
            let skills_root = skills_root.clone();
            let aggregate_root = aggregate_root.clone();
            let barrier = Arc::clone(&barrier);
            handles.push(std::thread::spawn(move || {
                barrier.wait();
                create_change_package_template(&aggregate_root, &skills_root, "login-refactor")
            }));
        }
        let outcomes: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().expect("创建线程结束"))
            .collect();

        for outcome in &outcomes {
            assert!(
                outcome.is_ok(),
                "并发创建必须都成功（幂等），不得因发布竞态报错：{outcomes:?}"
            );
        }
        let directories: Vec<&PathBuf> = outcomes
            .iter()
            .filter_map(|outcome| outcome.as_ref().ok())
            .collect();
        assert_eq!(
            directories[0], directories[1],
            "并发创建必须返回同一目录：{outcomes:?}"
        );
        for name in CHANGE_PACKAGE_FILES {
            assert!(
                directories[0].join(name).exists(),
                "并发创建后变更包文档缺失：{name}"
            );
        }
        assert!(
            staging_leftovers(&aggregate_root).is_empty(),
            "并发创建不得留下暂存目录：{:?}",
            staging_leftovers(&aggregate_root)
        );
    }

    /// Important 3：暂存目录名不得随标识长度增长 ⇒ 255 字节（单文件名上限）的
    /// 合法标识必须仍能创建变更包。
    #[test]
    fn create_change_package_template_accepts_maximum_length_identifier() {
        let (_skills_fixture, skills_root) = skills_source_fixture("skills");
        let (_aggregate_fixture, aggregate_root) = aggregate_fixture();

        // 248 字节标识 + `CHANGE-` 前缀 = 255 字节（正好是单文件名上限）。
        let identifier = "a".repeat(248);
        let directory = create_change_package_template(&aggregate_root, &skills_root, &identifier)
            .expect("255 字节的合法标识必须能创建变更包（不得 ENAMETOOLONG）");

        assert_eq!(
            directory
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .map(|name| name.len()),
            Some(255),
            "最终目录名应为 255 字节"
        );
        for name in CHANGE_PACKAGE_FILES {
            assert!(directory.join(name).exists(), "变更包文档缺失：{name}");
        }
        assert!(
            staging_leftovers(&aggregate_root).is_empty(),
            "不得留下暂存目录：{:?}",
            staging_leftovers(&aggregate_root)
        );
    }

    /// 变更包父目录下的暂存目录残留清单（暂存名以 `.kb-staging` 开头）。
    fn staging_leftovers(aggregate_root: &Path) -> Vec<String> {
        let changes = aggregate_root.join("cadence/knowledge-base/user-input/updates");
        std::fs::read_dir(changes)
            .expect("读取变更包父目录")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with(".kb-staging"))
            .collect()
    }

    // -----------------------------------------------------------------------
    // 修复轮 3 回归用例（原子发布 + 碰撞目标类型核验）
    // -----------------------------------------------------------------------

    /// 在 `dir` 下以 `openat` 植入一个普通文件（模拟「初始检查之后、发布之前」
    /// 目标名被普通文件占用）。
    fn plant_regular_file(dir: &OwnedFd, name: &str) -> std::io::Result<()> {
        let name = std::ffi::CString::new(name).expect("文件名不含内嵌 NUL");
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                name.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_CLOEXEC,
                0o644 as libc::c_uint,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        unsafe { libc::close(fd) };
        Ok(())
    }

    /// 注入的改名原语：先在目标名上植入普通文件，再执行真实的
    /// `RENAME_NOREPLACE` 改名（⇒ 必然 `EEXIST`）。
    fn rename_planting_regular_file(
        dir: &OwnedFd,
        from: &OsStr,
        to: &OsStr,
    ) -> std::io::Result<()> {
        let name = to.to_string_lossy().into_owned();
        plant_regular_file(dir, &name)?;
        rename_noreplace_at(dir, from, to)
    }

    /// 注入的改名原语：模拟文件系统/内核不支持 `RENAME_NOREPLACE`。
    fn rename_unsupported(_dir: &OwnedFd, _from: &OsStr, _to: &OsStr) -> std::io::Result<()> {
        Err(std::io::Error::from_raw_os_error(libc::ENOSYS))
    }

    /// 旧 Important 2 / 新 Important 1：碰撞目标是**普通文件**时不得判成
    /// 幂等成功（必须报错，且不得替换/删除占用者）。
    #[test]
    fn publish_staging_rejects_collision_with_regular_file() {
        let fixture = tempfile::TempDir::new().expect("创建 /tmp 夹具");
        let parent_path = fixture.path().to_path_buf();
        let parent_file = std::fs::File::open(&parent_path).expect("打开父目录");
        let parent: OwnedFd = parent_file.into();

        std::fs::create_dir(parent_path.join("staging")).expect("建暂存目录");
        std::fs::write(parent_path.join("staging/change-summary.md"), "变更包原文")
            .expect("写暂存内容");
        std::fs::write(parent_path.join("target"), "占位内容").expect("预置普通文件占用目标名");

        let error = publish_staging_with(&parent, "staging", "target", rename_noreplace_at)
            .expect_err("碰撞目标是普通文件时必须报错（不得判成幂等成功）");
        assert!(matches!(error, ProductStoreError::Io(_)), "{error:?}");
        assert_eq!(
            std::fs::read_to_string(parent_path.join("target")).expect("读取占用文件"),
            "占位内容",
            "占用者不得被替换或删除"
        );
        assert!(
            parent_path.join("staging").exists(),
            "未发布的暂存目录必须保留（由调用方清理）"
        );
    }

    /// 新 Important 2：碰撞目标是**符号链接**时不得判成幂等成功（否则可能把
    /// 指向知识库外的链接当成已发布的变更包返回）。
    #[cfg(unix)]
    #[test]
    fn publish_staging_rejects_collision_with_symlink() {
        let fixture = tempfile::TempDir::new().expect("创建 /tmp 夹具");
        let parent_path = fixture.path().to_path_buf();
        let outside = fixture.path().join("outside");
        std::fs::create_dir_all(&outside).expect("建根外目录");
        let parent_file = std::fs::File::open(&parent_path).expect("打开父目录");
        let parent: OwnedFd = parent_file.into();

        std::fs::create_dir(parent_path.join("staging")).expect("建暂存目录");
        std::fs::write(parent_path.join("staging/change-summary.md"), "变更包原文")
            .expect("写暂存内容");
        std::os::unix::fs::symlink(&outside, parent_path.join("target")).expect("预置符号链接占用");

        let error = publish_staging_with(&parent, "staging", "target", rename_noreplace_at)
            .expect_err("碰撞目标是符号链接时必须报错（不得判成幂等成功）");
        assert!(matches!(error, ProductStoreError::Io(_)), "{error:?}");
        assert!(
            parent_path
                .join("target")
                .symlink_metadata()
                .expect("读取链接元数据")
                .file_type()
                .is_symlink(),
            "符号链接不得被替换"
        );
        assert_eq!(
            std::fs::read_dir(&outside).expect("读取根外目录").count(),
            0,
            "不得把根外目录当成变更包发布"
        );
    }

    /// 新 Important 2（端到端）：初始检查之后目标名被普通文件占用时，创建
    /// 变更包必须报错，且不得留下变更包目录或暂存目录。
    #[test]
    fn create_change_package_template_rejects_collision_with_non_directory() {
        let (_skills_fixture, skills_root) = skills_source_fixture("skills");
        let (_aggregate_fixture, aggregate_root) = aggregate_fixture();

        let error = create_change_package_template_with(
            &aggregate_root,
            &skills_root,
            "login-refactor",
            rename_planting_regular_file,
        )
        .expect_err("碰撞目标是普通文件时必须报错");
        assert!(matches!(error, ProductStoreError::Io(_)), "{error:?}");

        let changes = aggregate_root.join("cadence/knowledge-base/user-input/updates");
        let target = changes.join("CHANGE-login-refactor");
        assert!(
            !target.is_dir(),
            "不得把占位的普通文件当成已发布的变更包目录"
        );
        assert!(target.is_file(), "占用者不得被替换或删除");
        assert!(
            staging_leftovers(&aggregate_root).is_empty(),
            "失败后不得留下暂存目录：{:?}",
            staging_leftovers(&aggregate_root)
        );
    }

    /// 新 Important 1：文件系统不支持 `RENAME_NOREPLACE` 时必须**明确报错**，
    /// 且不得留下任何最终目录或占位目录（不做非原子降级）。
    #[test]
    fn create_change_package_template_reports_unsupported_noreplace() {
        let (_skills_fixture, skills_root) = skills_source_fixture("skills");
        let (_aggregate_fixture, aggregate_root) = aggregate_fixture();

        let error = create_change_package_template_with(
            &aggregate_root,
            &skills_root,
            "login-refactor",
            rename_unsupported,
        )
        .expect_err("不支持 RENAME_NOREPLACE 时必须明确报错");
        assert!(matches!(error, ProductStoreError::Io(_)), "{error:?}");

        let changes = aggregate_root.join("cadence/knowledge-base/user-input/updates");
        assert!(
            !changes.join("CHANGE-login-refactor").exists(),
            "不得留下最终变更包目录（或占位目录）"
        );
        assert_eq!(
            std::fs::read_dir(&changes)
                .expect("读取变更包父目录")
                .filter_map(|entry| entry.ok())
                .count(),
            0,
            "不得留下占位目录或暂存目录"
        );
    }

    /// 新 Important 1（发布层）：`RENAME_NOREPLACE` 不可用时发布必须报错，
    /// 不得先建占位目录再改名（占位目录会被误判成已发布的变更包）。
    #[test]
    fn publish_staging_reports_unsupported_noreplace_without_placeholder() {
        let fixture = tempfile::TempDir::new().expect("创建 /tmp 夹具");
        let parent_path = fixture.path().to_path_buf();
        let parent_file = std::fs::File::open(&parent_path).expect("打开父目录");
        let parent: OwnedFd = parent_file.into();

        std::fs::create_dir(parent_path.join("staging")).expect("建暂存目录");
        std::fs::write(parent_path.join("staging/change-summary.md"), "变更包原文")
            .expect("写暂存内容");

        let error = publish_staging_with(&parent, "staging", "target", rename_unsupported)
            .expect_err("不支持 RENAME_NOREPLACE 时必须报错");
        assert!(matches!(error, ProductStoreError::Io(_)), "{error:?}");
        assert!(!parent_path.join("target").exists(), "不得创建占位目录");
        assert!(
            parent_path.join("staging").exists(),
            "未发布的暂存目录必须保留（由调用方清理）"
        );
    }
}
