    // ---- S-C：自愈缓存根遵循 CODEGRAPH_INSTALL_DIR + 异平台兜底包校验 ----

    /// 本机可执行格式的 4 字节头。
    fn native_executable_header() -> &'static [u8] {
        #[cfg(target_os = "linux")]
        {
            b"\x7fELF"
        }
        #[cfg(target_os = "macos")]
        {
            b"\xcf\xfa\xed\xfe"
        }
        #[cfg(target_os = "windows")]
        {
            b"MZ\x90\x00"
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            b"\x7fELF"
        }
    }

    /// 保证**不等于**本机格式的头（模拟异平台残留包里的 node）。
    fn foreign_executable_header() -> &'static [u8] {
        #[cfg(target_os = "linux")]
        {
            b"MZ\x90\x00"
        }
        #[cfg(not(target_os = "linux"))]
        {
            b"\x7fELF"
        }
    }

    fn executable_with_bytes(path: &Path, bytes: &[u8]) -> PathBuf {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, bytes).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(path).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(path, permissions).unwrap();
        }
        path.to_path_buf()
    }

    /// 与当前平台**不同**的目录名（异平台残留包的目录名）。
    fn foreign_platform_dir_name() -> String {
        if codegraph_platform_target() == "linux-x64" {
            "codegraph-darwin-arm64".to_string()
        } else {
            "codegraph-linux-x64".to_string()
        }
    }

    /// 自愈缓存根：上游 `npm-shim.js:113` / `npm-sdk.js:68` 是
    /// `CODEGRAPH_INSTALL_DIR || ~/.codegraph` ⇒ 环境变量优先，空值视为未设置。
    #[test]
    fn codegraph_cache_root_prefers_install_dir_over_home() {
        let home = Path::new("/home/dev");
        assert_eq!(
            codegraph_cache_root(Some(home), Some(PathBuf::from("/opt/codegraph"))),
            Some(PathBuf::from("/opt/codegraph")),
            "CODEGRAPH_INSTALL_DIR 必须优先于 ~/.codegraph"
        );
        assert_eq!(
            codegraph_cache_root(Some(home), None),
            Some(home.join(".codegraph")),
            "未设置时保持 ~/.codegraph 默认"
        );
        assert_eq!(
            codegraph_cache_root(Some(home), Some(PathBuf::new())),
            Some(home.join(".codegraph")),
            "空值视为未设置"
        );
        assert_eq!(codegraph_cache_root(None, None), None);
    }

    /// 指定缓存根（`CODEGRAPH_INSTALL_DIR`）下的自愈缓存必须命中。
    #[test]
    fn bundled_node_honors_codegraph_install_dir() {
        let temp = tempfile::tempdir().unwrap();
        let (_, sdk) = sdk_package_dir(&temp.path().join("prefix"));
        let install_dir = temp.path().join("cg-install");
        let cached = executable_under(
            &install_dir
                .join("bundles")
                .join(format!("{}-1.6.2", codegraph_platform_target())),
            node_executable_name(),
        );

        let candidates = bundled_node_candidates_with_cache_root(&sdk, Some(install_dir.clone()));

        assert!(
            candidates.contains(&cached),
            "CODEGRAPH_INSTALL_DIR 下的自愈缓存必须命中：{candidates:?}"
        );
    }

    /// 异平台残留包里的 node 在 `+x` 口径下同样「可执行」，但二进制格式不属于
    /// 本机 ⇒ 不得作兜底候选（否则必然 exec 失败、硬失败且绕过 CLI 回退）。
    #[test]
    fn platform_package_nodes_skips_foreign_package_without_native_binary() {
        let temp = tempfile::tempdir().unwrap();
        let scope = temp.path().join("@colbymchenry");
        let current = executable_under(&scope.join(platform_dir_name()), node_executable_name());
        let foreign = executable_with_bytes(
            &scope
                .join(foreign_platform_dir_name())
                .join(node_executable_name()),
            foreign_executable_header(),
        );

        let candidates = platform_package_nodes(&scope, &codegraph_platform_target());

        assert!(
            candidates.contains(&current),
            "当前平台包必须命中：{candidates:?}"
        );
        assert!(
            !candidates.contains(&foreign),
            "异平台残留包（二进制格式不符）不得作兜底候选：{candidates:?}"
        );
    }

    /// 兜底包的采用条件是**可执行性校验通过**而不是目录名：目录名不是当前
    /// 平台、但二进制确实是本机格式（手工放置 / 符号目录）⇒ 可以采用。
    #[test]
    fn platform_package_nodes_keeps_foreign_dir_with_native_binary() {
        let temp = tempfile::tempdir().unwrap();
        let scope = temp.path().join("@colbymchenry");
        let usable = executable_with_bytes(
            &scope
                .join(foreign_platform_dir_name())
                .join(node_executable_name()),
            native_executable_header(),
        );

        let candidates = platform_package_nodes(&scope, &codegraph_platform_target());

        assert!(
            candidates.contains(&usable),
            "本机格式的兜底包必须可以采用：{candidates:?}"
        );
    }
