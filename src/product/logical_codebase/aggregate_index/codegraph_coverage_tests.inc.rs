    // ---- F6：自带 node 的三种受支持布局（实测 codegraph 1.6.2 真实资产） ----

    /// 造一份 SDK 包根（`<root>/lib/node_modules/@colbymchenry/codegraph`）
    /// 并返回（包根, `npm-sdk.js`）。
    fn sdk_package_dir(root: &Path) -> (PathBuf, PathBuf) {
        let package_root = root
            .join("lib")
            .join("node_modules")
            .join("@colbymchenry")
            .join("codegraph");
        std::fs::create_dir_all(&package_root).unwrap();
        let sdk = package_root.join("npm-sdk.js");
        std::fs::write(&sdk, "module.exports = {};").unwrap();
        (package_root, sdk)
    }

    fn platform_dir_name() -> String {
        format!("codegraph-{}", codegraph_platform_target())
    }

    fn executable_under(dir: &Path, name: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        executable_file(&dir.join(name))
    }

    /// ① nested：`<主包>/node_modules/@colbymchenry/codegraph-<target>/node`。
    #[test]
    fn bundled_node_resolves_nested_platform_layout() {
        let temp = tempfile::tempdir().unwrap();
        let (package_root, sdk) = sdk_package_dir(temp.path());
        let nested = executable_under(
            &package_root
                .join("node_modules")
                .join("@colbymchenry")
                .join(platform_dir_name()),
            node_executable_name(),
        );

        let candidates = bundled_node_candidates(&sdk, None);

        assert!(
            candidates.contains(&nested),
            "nested 布局必须命中：{candidates:?}"
        );
    }

    /// ② hoisted：平台包与主包**同级**（真实 1.6.2 布局，
    /// `/tmp/codegraph-ref/libtest/node_modules/@colbymchenry/{codegraph,codegraph-linux-x64}`）。
    #[test]
    fn bundled_node_resolves_hoisted_platform_layout() {
        let temp = tempfile::tempdir().unwrap();
        let (package_root, sdk) = sdk_package_dir(temp.path());
        let scope = package_root.parent().expect("scope dir");
        let hoisted = executable_under(&scope.join(platform_dir_name()), node_executable_name());

        let candidates = bundled_node_candidates(&sdk, None);

        assert!(
            candidates.contains(&hoisted),
            "hoisted（与主包同级）布局必须命中：{candidates:?}"
        );
    }

    /// ③ 自愈缓存：`~/.codegraph/bundles/<target>-<version>/node`。
    #[test]
    fn bundled_node_resolves_self_heal_cache_layout() {
        let temp = tempfile::tempdir().unwrap();
        let (_, sdk) = sdk_package_dir(&temp.path().join("prefix"));
        let home = temp.path().join("home");
        let cached = executable_under(
            &home
                .join(".codegraph")
                .join("bundles")
                .join(format!("{}-1.6.2", codegraph_platform_target())),
            node_executable_name(),
        );

        let candidates = bundled_node_candidates(&sdk, Some(&home));

        assert!(
            candidates.contains(&cached),
            "自愈缓存布局必须命中：{candidates:?}"
        );
    }

    /// PATH 上有旧 node（< 22.5）但**自带 node 可用** ⇒ 必须用自带 node，
    /// 不得因 PATH node 太旧而判 node 不可用。
    #[test]
    fn coverage_prefers_bundled_node_over_outdated_path_node() {
        let fixture = coverage_fixture();
        let (package_root, sdk) = sdk_package_dir(&fixture._temp.path().join("prefix"));
        let scope = package_root.parent().expect("scope dir");
        let bundled = executable_under(&scope.join(platform_dir_name()), node_executable_name());
        let path_node = executable_file(&fixture._temp.path().join("path-node"));
        let paths = CoveragePaths {
            install_prefix_sdk: Some(sdk.clone()),
            bundled_nodes: Vec::new(),
            path_node: Some(path_node),
            ..CoveragePaths::default()
        };
        fixture.runner.node_version("v18.20.0");

        let resolved = resolve_node_executable(&paths, &sdk, &fixture.runner_handle()).unwrap();

        assert_eq!(
            resolved, bundled,
            "自带 node 可用时不得回落（更不得因 PATH node 太旧而判不可用）"
        );
        assert!(
            fixture.runner.requests().is_empty(),
            "自带 node 命中时不得探测 PATH node"
        );
    }

    /// `node_unavailable` **不得阻断聚合验收**：按方案 §五 包 4「回退保底」，
    /// SDK/Node 不可用必须回退既有 CLI files 验收（保留 64 MiB 上限与专用
    /// 截断码），而不是直接失败。
    #[test]
    fn coverage_falls_back_to_cli_when_node_is_unavailable() {
        let fixture = coverage_fixture();
        let coverage = CodeGraphCoverage::with_paths(
            fixture.runner.clone(),
            CoveragePaths {
                install_prefix_sdk: Some(fixture.sdk.clone()),
                bundled_nodes: Vec::new(),
                path_node: None,
                ..CoveragePaths::default()
            },
            Some(fixture.scratch.clone()),
        );

        let error = coverage
            .coverage(&fixture.root, &fixture.members())
            .unwrap_err();

        assert!(
            matches!(&error, AggregateIndexError::Failed { code, .. }
                if *code == "codegraph_sdk_incompatible"),
            "node 不可用必须回退既有 CLI 验收（不得阻断），got: {error}"
        );
        assert!(
            error.to_string().contains("22.5"),
            "回退痕迹必须保留 Node.js ≥ 22.5 的诊断，got: {error}"
        );
    }

    // ---- 可诊断性（P0）：覆盖判定 / 回退 / 截断的结构化事实 ----

    /// 成员覆盖汇总（日志 `covered_count` / `uncovered_members` 的消费者）。
    #[test]
    fn coverage_summary_counts_covered_and_lists_uncovered_members() {
        let covered = BTreeMap::from([
            ("api".to_string(), true),
            ("web".to_string(), false),
            ("docs".to_string(), false),
        ]);

        let summary = summarize_coverage(&covered);

        assert_eq!(summary.member_count, 3);
        assert_eq!(summary.covered_count, 1);
        assert_eq!(
            summary.uncovered,
            vec!["docs".to_string(), "web".to_string()],
            "uncovered 必须列出成员名清单（顺序稳定）"
        );
    }

    /// 健康字段（state / pending / stale）原样抽取；取不到时字段缺席而不是
    /// 伪造默认值。
    #[test]
    fn coverage_health_extracts_state_pending_stale() {
        let index = serde_json::json!({"state": "stale", "pending": 3, "stale": true});

        let health = coverage_health(Some(&index));

        assert_eq!(health.state.as_deref(), Some("stale"));
        assert_eq!(health.pending, Some(3));
        assert_eq!(health.stale, Some(true));
        assert_eq!(coverage_health(None), CoverageHealth::default());
    }

    /// 自带 node 与自愈缓存 node 必须带**来源**标注（客户机 darwin-arm64
    /// 上「自带 / PATH / 缓存」一眼可分）。
    #[test]
    fn bundled_node_candidates_annotate_source() {
        let temp = tempfile::tempdir().unwrap();
        let (package_root, sdk) = sdk_package_dir(temp.path());
        let nested = executable_under(
            &package_root
                .join("node_modules")
                .join("@colbymchenry")
                .join(platform_dir_name()),
            node_executable_name(),
        );
        let cache_root = temp.path().join("cache");
        let cached = executable_under(
            &cache_root
                .join("bundles")
                .join(format!("{}-1.6.2", codegraph_platform_target())),
            node_executable_name(),
        );

        let resolutions = bundled_node_candidates_with_source(&sdk, Some(cache_root));

        assert!(
            resolutions
                .iter()
                .any(|resolution| resolution.path == nested && resolution.source == NodeSource::Bundled),
            "nested/hoisted 自带 node 的来源必须是 bundled: {resolutions:?}"
        );
        assert!(
            resolutions
                .iter()
                .any(|resolution| resolution.path == cached && resolution.source == NodeSource::Cache),
            "自愈缓存的 node 来源必须是 cache: {resolutions:?}"
        );
    }

    /// 解析出来的 node 必须带结构化来源（注入的 bundled 候选 ⇒ bundled）。
    #[test]
    fn node_resolution_reports_source() {
        let temp = tempfile::tempdir().unwrap();
        let (_, sdk) = sdk_package_dir(temp.path());
        let node = executable_file(&temp.path().join("bundled-node"));
        let paths = CoveragePaths {
            install_prefix_sdk: Some(sdk.clone()),
            bundled_nodes: vec![node.clone()],
            ..CoveragePaths::default()
        };
        let runner: Arc<dyn BoundedCommandRunner> = Arc::new(FakeCoverageRunner::default());

        let resolution = resolve_node_with_source(&paths, &sdk, &runner).unwrap();

        assert_eq!(resolution.path, node);
        assert_eq!(resolution.source, NodeSource::Bundled);
        assert_eq!(resolution.source.as_str(), "bundled");
    }

    /// SDK 不可用码 ⇒ CLI 回退触发原因（日志 `trigger` 字段的消费者）；
    /// 其它失败码不得被误判为「可回退」。
    #[test]
    fn sdk_failure_codes_map_to_cli_fallback_triggers() {
        use super::super::operation::{coverage_fallback_trigger, error_code};

        let not_found = AggregateIndexError::Failed {
            code: "codegraph_sdk_not_found",
            message: "missing sdk".to_string(),
        };
        assert_eq!(error_code(&not_found), "codegraph_sdk_not_found");
        assert_eq!(coverage_fallback_trigger(&not_found), Some("sdk_not_found"));

        let incompatible = AggregateIndexError::Failed {
            code: "codegraph_sdk_incompatible",
            message: "node too old".to_string(),
        };
        assert_eq!(
            coverage_fallback_trigger(&incompatible),
            Some("sdk_incompatible")
        );

        let truncated = AggregateIndexError::Failed {
            code: "codegraph_files_output_truncated",
            message: "truncated".to_string(),
        };
        assert_eq!(
            coverage_fallback_trigger(&truncated),
            None,
            "截断不是可回退原因（不得被悄悄吞掉）"
        );

        let degraded = AggregateIndexError::Degraded {
            code: "codegraph_sdk_not_found",
            message: "degraded".to_string(),
        };
        assert_eq!(
            coverage_fallback_trigger(&degraded),
            None,
            "Degraded 语义不得静默回退（与既有行为一致）"
        );
    }
