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

    /// SDK 成员覆盖验收（2026-11-13 切片）：探针返回的 hasFilesUnder 判定
    /// 直接决定覆盖结论；未覆盖成员被点名，错误码沿用既有
    /// `aggregate_index_member_coverage_failed`（不新造码）。
    #[test]
    fn sdk_member_coverage_names_uncovered_members() {
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

        let error = operation.build("project_0001", 3).unwrap_err();
        assert!(
            matches!(&error, AggregateIndexError::Failed { code, .. }
                if *code == "aggregate_index_member_coverage_failed"),
            "unexpected error: {error}"
        );
        assert!(
            error.to_string().contains("members [web]"),
            "must name the SDK-uncovered member, got: {error}"
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
