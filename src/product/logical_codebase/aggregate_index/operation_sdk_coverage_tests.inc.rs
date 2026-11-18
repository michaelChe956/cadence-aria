// operation.rs 的 SDK 成员覆盖验收测试（从 operation_tests.inc.rs 拆出以
// 满足 large_file_guard 的 1200 行硬线；本文件被 include! 插在 tests 模块
// 内部，只能用 `//` 行注释——`//!` 在模块中间是 E0753）。
// 覆盖探针（AggregateCoverageProbe）经 with_coverage_probe 注入桩，
// 不触真实用户 home / 不依赖宿主是否安装 codegraph 与 node。

    /// 覆盖探针桩：脚本化 SDK 覆盖/健康返回，或 SDK 缺失失败。
    struct StubCoverageProbe {
        result: Result<CoverageOutcome, AggregateIndexError>,
    }

    impl StubCoverageProbe {
        fn with_members(members: &[(&str, bool)]) -> Self {
            Self {
                result: Ok(CoverageOutcome {
                    member_covered: members
                        .iter()
                        .map(|(name, covered)| (name.to_string(), *covered))
                        .collect(),
                    stats: Some(serde_json::json!({"fileCount": 2})),
                    index: Some(
                        serde_json::json!({"state": "ready", "pending": 0, "stale": false}),
                    ),
                }),
            }
        }

        fn unhealthy_index() -> Self {
            Self {
                result: Ok(CoverageOutcome {
                    member_covered: BTreeMap::from([
                        ("api".to_string(), true),
                        ("web".to_string(), true),
                    ]),
                    stats: Some(serde_json::json!({"fileCount": 2})),
                    index: Some(
                        serde_json::json!({"state": "scanning", "pending": 3, "stale": true}),
                    ),
                }),
            }
        }

        fn sdk_not_found() -> Self {
            Self {
                result: Err(AggregateIndexError::Failed {
                    code: "codegraph_sdk_not_found",
                    message: "npm-sdk.js not found under any known prefix".to_string(),
                }),
            }
        }
    }

    impl AggregateCoverageProbe for StubCoverageProbe {
        fn coverage(
            &self,
            _root: &Path,
            _member_names: &[String],
        ) -> Result<CoverageOutcome, AggregateIndexError> {
            self.result.clone()
        }
    }

    /// 片3R ①跳过路径：SDK 探针报 web 未覆盖、盘检确认 web 目录确无
    /// 源码 ⇒ 不再当覆盖失败——世代照常 Active，web 快照落
    /// `skip_reason=Some(NoSource)` 且 `included` 语义不动；api 无跳过
    /// 事实。跳过成员被记下、无任何失败。
    #[test]
    fn sdk_member_coverage_skips_member_confirmed_source_less() {
        let fixture = aggregate_index_fixture();
        // CLI 侧文件齐全——若实现错误地走 CLI 路径，本测试将误绿；
        // 断言以 SDK 探针（web 未覆盖）为准。
        fixture.cli.files_return(["api/src/A.java", "web/src/B.ts"]);
        for query in EXCLUDED_QUERIES {
            fixture.cli.query_returns(query, serde_json::json!([]));
        }
        let operation = fixture.operation().with_coverage_probe(Arc::new(
            StubCoverageProbe::with_members(&[("api", true), ("web", false)]),
        ));

        let record = operation.build("project_0001", 3).unwrap();
        assert_eq!(record.status, AggregateIndexStatus::Active);
        // web 成员修订为 b*40（api 为 a*40），按修订定位避免依赖快照顺序。
        let web = record
            .member_snapshots
            .iter()
            .find(|snapshot| snapshot.revision == "b".repeat(40))
            .expect("web member snapshot must exist");
        assert_eq!(
            web.skip_reason,
            Some(super::super::types::AggregateIndexMemberSkipReason::NoSource)
        );
        assert!(web.included);
        let api = record
            .member_snapshots
            .iter()
            .find(|snapshot| snapshot.revision == "a".repeat(40))
            .expect("api member snapshot must exist");
        assert_eq!(api.skip_reason, None);
    }

    /// 片3R ②失败路径（磁盘事实变体）：web 目录确有真实源码、SDK 探针
    /// 仍报未覆盖 ⇒ 漏收失败。错误码沿用既有
    /// `aggregate_index_member_coverage_failed`（不新造码），文案必须带
    /// 磁盘事实前缀并点名 web，且不得说成用户没放源码。
    #[test]
    fn sdk_member_coverage_fails_when_disk_has_source_but_probe_uncovered() {
        let fixture = aggregate_index_fixture();
        fixture.cli.files_return(["api/src/A.java", "web/src/B.ts"]);
        for query in EXCLUDED_QUERIES {
            fixture.cli.query_returns(query, serde_json::json!([]));
        }
        // 夹具根在 /tmp；给 web 写入真实源码文件，盘检口径下属源码。
        let root = fixture
            .logical
            .load_manifest("project_0001")
            .unwrap()
            .unwrap()
            .provider_context_root;
        std::fs::write(root.join("web/src/B.ts"), "export const b = 1;\n").unwrap();
        let operation = fixture.operation().with_coverage_probe(Arc::new(
            StubCoverageProbe::with_members(&[("api", true), ("web", false)]),
        ));

        let error = operation.build("project_0001", 3).unwrap_err();
        assert!(
            matches!(&error, AggregateIndexError::Failed { code, .. }
                if *code == "aggregate_index_member_coverage_failed"),
            "unexpected error: {error}"
        );
        let message = error.to_string();
        assert!(
            message.contains("source found on disk"),
            "must carry the on-disk fact prefix, got: {message}"
        );
        assert!(
            message.contains("members [web]"),
            "must name the SDK-uncovered member, got: {message}"
        );
        assert!(
            !message.contains("no source found") && !message.contains("add a member"),
            "must not claim the member lacks source on disk, got: {message}"
        );
    }

    /// 健康闸：SDK 返回 stale=true 等异常信号 ⇒ 记 warning，不阻断激活。
    #[test]
    fn coverage_marks_index_unhealthy_as_warning() {
        let fixture = aggregate_index_fixture();
        fixture.cli.files_return(["api/src/A.java", "web/src/B.ts"]);
        for query in EXCLUDED_QUERIES {
            fixture.cli.query_returns(query, serde_json::json!([]));
        }
        let operation = fixture.operation().with_coverage_probe(Arc::new(
            StubCoverageProbe::unhealthy_index(),
        ));

        let record = operation.build("project_0001", 3).unwrap();
        assert_eq!(record.status, AggregateIndexStatus::Active);
        let warning = record.warning.unwrap_or_default();
        assert!(
            warning.contains("stale") && warning.contains("健康"),
            "unhealthy index must surface an auditable warning, got: {warning:?}"
        );
    }

    /// 回退保底：SDK 路径解析不到 ⇒ 回退既有 CLI files 验收（小仓行为
    /// 不回归），且在激活记录上留下可审计的回退 warning。
    #[test]
    fn coverage_falls_back_to_cli_when_sdk_missing() {
        let fixture = aggregate_index_fixture();
        fixture.cli.files_return(["api/src/A.java", "web/src/B.ts"]);
        for query in EXCLUDED_QUERIES {
            fixture.cli.query_returns(query, serde_json::json!([]));
        }
        let operation = fixture.operation().with_coverage_probe(Arc::new(
            StubCoverageProbe::sdk_not_found(),
        ));

        let record = operation.build("project_0001", 3).unwrap();
        assert_eq!(record.status, AggregateIndexStatus::Active);
        let warning = record.warning.unwrap_or_default();
        assert!(
            warning.contains("回退"),
            "fallback must leave an auditable warning, got: {warning:?}"
        );
    }
