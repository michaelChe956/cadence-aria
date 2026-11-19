#[cfg(test)]
mod aggregate_index_tests {
    use super::*;
    use crate::product::logical_codebase::aggregate_index::AggregateIndexRecord;
    use super::tests::create_project_and_lc;

    /// 旧实现里的英文占位串：拼接构造，避免被全仓字面量门禁误判。
    fn legacy_placeholder() -> String {
        concat!("no stderr ", "summary").to_string()
    }

    fn seed_failed_aggregate_index_record(
        paths: &ProductAppPaths,
        lc_id: &str,
        command_id: &str,
    ) -> String {
        let store = AggregateIndexStore::for_lc(paths.clone(), lc_id);
        let mut record = AggregateIndexRecord::building(
            "aggregate_index_g3_failed_0001".to_string(),
            "project_0001".to_string(),
            2,
            Vec::new(),
            chrono::Utc::now().to_rfc3339(),
        );
        record.command_id = Some(command_id.to_string());
        store.create("project_0001", record).unwrap();
        store
            .mark_status(
                "project_0001",
                "aggregate_index_g3_failed_0001",
                AggregateIndexStatus::Failed,
                Some("codegraph_version_mismatch: expected 1.6.0, got 1.6.1".to_string()),
            )
            .unwrap();
        "aggregate_index_g3_failed_0001".to_string()
    }

    fn g3_retry_request(lc_id: &str, command_id: &str) -> BootstrapActionRequest {
        BootstrapActionRequest {
            command_id: command_id.to_string(),
            project_id: "project_0001".to_string(),
            logical_codebase_id: lc_id.to_string(),
            step: LogicalCodebaseBootstrapStep::AggregateIndexActive,
            action: BootstrapActionKind::Retry,
            expected_revision: Some(2),
            expected_object_id: "aggregate_index_g3_failed_0001".to_string(),
        }
    }

    /// G3（终局关闸缺口）：aggregate_index_active 步失败后，bootstrap
    /// actions retry 此前仅按 command_id 查重放记录——找不到即 NotFound
    /// 恒 500，无重建触发分支（现场 cmd-gapfix-a03-retry-1/2）。修复：
    /// 注入重建派发器（复用 `build_with_command_id` 的幂等命令语义）后，
    /// retry 用新 command_id 派发重建；同 command 重放不重复派发。
    #[test]
    fn aggregate_index_retry_dispatches_rebuild_and_replays_same_command() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let lc_id = create_project_and_lc(&paths, &temp.path().join("aggregate-root"));
        seed_failed_aggregate_index_record(&paths, &lc_id, "cmd-gapfix-a03-first");

        let dispatches = std::sync::Arc::new(AtomicUsize::new(0));
        let store_for_rebuild = AggregateIndexStore::for_lc(paths.clone(), &lc_id);
        let dispatches_for_rebuild = dispatches.clone();
        let dispatcher =
            std::sync::Arc::new(move |project_id: &str, command_id: &str, revision: u64| {
                dispatches_for_rebuild.fetch_add(1, Ordering::SeqCst);
                // 复刻 build_with_command_id 成功效果：落一条携带该命令
                // 身份的 Active 记录（membership_revision 对齐请求）。
                let mut record = AggregateIndexRecord::building(
                    format!("aggregate_index_retry_{command_id}"),
                    project_id.to_string(),
                    revision,
                    Vec::new(),
                    chrono::Utc::now().to_rfc3339(),
                );
                record.status = AggregateIndexStatus::Active;
                record.command_id = Some(command_id.to_string());
                store_for_rebuild
                    .create(project_id, record.clone())
                    .map(|_| record)
            });
        let service = LogicalCodebaseBootstrapService::new(paths.clone())
            .with_aggregate_index_rebuild(dispatcher);

        // 修复前现场：NotFound 恒 500（无重建分支）。
        let request = g3_retry_request(&lc_id, "cmd-gapfix-a03-retry-1");
        let outcome = service.dispatch_action(&request).expect("retry rebuilds");
        assert_eq!(outcome, BootstrapActionOutcome::Completed);
        assert_eq!(dispatches.load(Ordering::SeqCst), 1);
        // 重建派发参数经 durable 事实复核（project/command/revision）。
        let rebuilt = AggregateIndexStore::for_lc(paths.clone(), &lc_id)
            .get(
                "project_0001",
                "aggregate_index_retry_cmd-gapfix-a03-retry-1",
            )
            .expect("rebuilt record")
            .expect("rebuilt record present");
        assert_eq!(rebuilt.status, AggregateIndexStatus::Active);
        assert_eq!(
            rebuilt.command_id.as_deref(),
            Some("cmd-gapfix-a03-retry-1")
        );
        assert_eq!(rebuilt.membership_revision, 2);

        // 同 command 重放：返回同一 Active 事实（Completed），不再派发。
        let replay = service
            .dispatch_action(&request)
            .expect("replay returns durable result");
        assert_eq!(replay, BootstrapActionOutcome::Completed);
        assert_eq!(dispatches.load(Ordering::SeqCst), 1);
    }

    /// G3 语义收口：未注入重建派发器（旧构造）时保持既有 NotFound——
    /// 不静默假成功；由 web 层生产注入收口。
    #[test]
    fn aggregate_index_retry_without_dispatcher_keeps_not_found() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let lc_id = create_project_and_lc(&paths, &temp.path().join("aggregate-root"));
        seed_failed_aggregate_index_record(&paths, &lc_id, "cmd-gapfix-a03-first");

        let service = LogicalCodebaseBootstrapService::new(paths);
        let error = service
            .dispatch_action(&g3_retry_request(&lc_id, "cmd-gapfix-a03-retry-2"))
            .expect_err("no dispatcher keeps fail-closed NotFound");
        assert!(matches!(
            error,
            ProductStoreError::NotFound {
                kind: "aggregate_index_command",
                ..
            }
        ));
    }

    /// ① UI 语义：覆盖不足（成员无可索引代码文件）的 Failed 记录投影
    /// 专用 reason_code（aggregate_index_member_coverage_failed），前端
    /// 呈现「聚合索引失败:成员代码覆盖不足」业务标题；其它 CLI 失败仍
    /// 回落通用 aggregate_index_failed。沿既有失败卡通道，不新建卡型。
    #[test]
    fn aggregate_coverage_failure_projects_dedicated_reason_code() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let lc_id = create_project_and_lc(&paths, &temp.path().join("aggregate-root"));
        let store = AggregateIndexStore::for_lc(paths.clone(), &lc_id);
        store
            .create(
                "project_0001",
                AggregateIndexRecord::building(
                    "aggregate_index_cov_failed_0001".to_string(),
                    "project_0001".to_string(),
                    2,
                    Vec::new(),
                    chrono::Utc::now().to_rfc3339(),
                ),
            )
            .unwrap();
        store
            .mark_status(
                "project_0001",
                "aggregate_index_cov_failed_0001",
                AggregateIndexStatus::Failed,
                Some(
                    "aggregate_index_failed:aggregate_index_member_coverage_failed: \
                     no indexable code file under included members [api, web]; each included \
                     member needs at least 1 indexable code file"
                        .to_string(),
                ),
            )
            .unwrap();

        let projection = LogicalCodebaseBootstrapProjector::new(paths.clone())
            .project("project_0001", &lc_id)
            .unwrap();
        let step = projection
            .steps
            .iter()
            .find(|step| step.step == LogicalCodebaseBootstrapStep::AggregateIndexActive)
            .unwrap();
        assert_eq!(step.status, LogicalCodebaseBootstrapStepStatus::Failed);
        let failure = step.failure.as_ref().expect("coverage failure detail");
        assert_eq!(
            failure.reason_code, "aggregate_index_member_coverage_failed",
            "coverage failure must project the dedicated reason code"
        );
        assert!(failure.detail.contains("members [api, web]"));
        // 沿既有通道：失败步仍允许 Retry（G3 派发 init+sync 重建）。
        assert!(step.allowed_actions.contains(&BootstrapActionKind::Retry));

        // 对照：非覆盖类 CLI 失败保持通用 reason_code。
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let lc_id = create_project_and_lc(&paths, &temp.path().join("aggregate-root"));
        seed_failed_aggregate_index_record(&paths, &lc_id, "cmd-semantics-control");
        let projection = LogicalCodebaseBootstrapProjector::new(paths)
            .project("project_0001", &lc_id)
            .unwrap();
        let step = projection
            .steps
            .iter()
            .find(|step| step.step == LogicalCodebaseBootstrapStep::AggregateIndexActive)
            .unwrap();
        assert_eq!(step.status, LogicalCodebaseBootstrapStepStatus::Failed);
        assert_eq!(
            step.failure.as_ref().unwrap().reason_code,
            "aggregate_index_failed"
        );
    }

    /// 测试点 #15（REQ-LAYOUT-01 链路 A 稳定码）：`warning` 里的布局码
    /// 必须投影成**自己的**稳定 reason_code（不是通用 `aggregate_index_failed`），
    /// 且 copy.action_zh 是该布局码的精确动作（不是「重试」兜底）；
    /// 覆盖不足码既有行为不回归；其它 warning 仍是通用码。
    #[test]
    fn aggregate_index_layout_unsupported_projects_its_own_stable_code() {
        use crate::product::logical_codebase::failure_copy::describe_code;

        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let lc_id = create_project_and_lc(&paths, &temp.path().join("aggregate-root"));
        let store = AggregateIndexStore::for_lc(paths.clone(), &lc_id);
        store
            .create(
                "project_0001",
                AggregateIndexRecord::building(
                    "aggregate_index_layout_failed_0001".to_string(),
                    "project_0001".to_string(),
                    2,
                    Vec::new(),
                    chrono::Utc::now().to_rfc3339(),
                ),
            )
            .unwrap();
        store
            .mark_status(
                "project_0001",
                "aggregate_index_layout_failed_0001",
                AggregateIndexStatus::Failed,
                Some(
                    "aggregate_index_failed:aggregate_index_layout_unsupported: checkout root \
                     name of member api is not a single path segment"
                        .to_string(),
                ),
            )
            .unwrap();

        let projection = LogicalCodebaseBootstrapProjector::new(paths.clone())
            .project("project_0001", &lc_id)
            .unwrap();
        let step = projection
            .steps
            .iter()
            .find(|step| step.step == LogicalCodebaseBootstrapStep::AggregateIndexActive)
            .unwrap();
        assert_eq!(step.status, LogicalCodebaseBootstrapStepStatus::Failed);
        let failure = step.failure.as_ref().expect("layout failure detail");
        assert_eq!(
            failure.reason_code, "aggregate_index_layout_unsupported",
            "布局码必须投影成自己的稳定码，不能被压成 aggregate_index_failed"
        );
        let copy = failure.copy.as_ref().expect("链路 A 必须带同源成品文案");
        assert_eq!(copy.machine_line, "aggregate_index_layout_unsupported");
        assert_eq!(
            copy.action_zh,
            describe_code("aggregate_index_layout_unsupported", None).action_zh,
            "copy.action_zh 必须是布局码的精确动作"
        );
        assert!(
            copy.reason_zh.contains("一层直接子目录"),
            "布局码必须给出具体规则依据：{}",
            copy.reason_zh
        );
        // triage 只从 copy.triage 派生。
        assert_eq!(failure.triage.as_deref(), Some("environment"));
        // 索引链的 `detail` 是**脱敏后的原始诊断**（内层 warning 全文）：三段中文
        // 由 `copy` 承载并已在上面逐条断言，这里不再要求 detail 重复一份序列化
        // 结果（Plan §3.1 只要求 record 链的两处用 render_detail）。
        assert!(
            failure.detail.contains("aggregate_index_layout_unsupported"),
            "detail 必须保留内层稳定码：{}",
            failure.detail
        );
        assert!(
            failure.detail.contains("not a single path segment"),
            "detail 必须保留原始诊断正文：{}",
            failure.detail
        );
        assert!(!failure.detail.contains(&legacy_placeholder()));
    }

    /// 测试点 #12（兼容）：`BootstrapFailure` 旧响应没有 `triage`/`copy` 键
    /// 时反序列化成功且为 None；带值时原样透出（triage 来自 copy）。
    #[test]
    fn bootstrap_failure_deserializes_without_triage_and_copy() {
        let legacy: BootstrapFailure = serde_json::from_value(serde_json::json!({
            "reason_code": "aggregate_pre_check_failed",
            "detail": "stage aggregate_initialization: legacy record",
            "retryable": true,
            "external_side_effect": "aggregate_initialization_provider_turn",
        }))
        .expect("旧响应必须仍可反序列化");
        assert_eq!(legacy.triage, None);
        assert_eq!(legacy.copy, None);

        let copy = crate::product::logical_codebase::failure_copy::describe_code(
            "aggregate_root_is_git",
            None,
        );
        let current = BootstrapFailure::with_copy(
            "aggregate_root_is_git".to_string(),
            "问题：逻辑代码库根目录不能是 git 仓库".to_string(),
            false,
            "none".to_string(),
            copy.clone(),
        );
        assert_eq!(current.triage.as_deref(), Some("environment"));
        assert_eq!(current.copy.as_ref(), Some(&copy));
        let round_tripped: BootstrapFailure =
            serde_json::from_value(serde_json::to_value(&current).unwrap()).unwrap();
        assert_eq!(round_tripped, current);
        assert_eq!(
            round_tripped.copy.as_ref().map(|copy| copy.reason_zh.as_str()),
            Some(copy.reason_zh.as_str())
        );
    }

    /// 片4B（REQ-BOOT-10 / REQ-IND-04）：冷启动投影的失败面必须经**产品层
    /// 唯一封套解析入口**取「内层稳定码 + 带点名的三段成品」，与 HTTP 出口
    /// 同码同文案。旧实现用 `warning.contains("<code>")` 裸前缀判断，三个
    /// 补充失败码会被压成通用 `aggregate_index_failed`。
    fn seed_failed_index_with_warning(
        paths: &ProductAppPaths,
        lc_id: &str,
        record_id: &str,
        warning: &str,
    ) {
        let store = AggregateIndexStore::for_lc(paths.clone(), lc_id);
        store
            .create(
                "project_0001",
                AggregateIndexRecord::building(
                    record_id.to_string(),
                    "project_0001".to_string(),
                    2,
                    Vec::new(),
                    chrono::Utc::now().to_rfc3339(),
                ),
            )
            .unwrap();
        store
            .mark_status(
                "project_0001",
                record_id,
                AggregateIndexStatus::Failed,
                Some(warning.to_string()),
            )
            .unwrap();
    }

    /// 投影 `aggregate_index_active` 步并取它的失败载体（失败步必有 failure）。
    fn projected_aggregate_failure(
        paths: &ProductAppPaths,
        lc_id: &str,
    ) -> (LogicalCodebaseBootstrapStepStatus, BootstrapFailure) {
        let projection = LogicalCodebaseBootstrapProjector::new(paths.clone())
            .project("project_0001", lc_id)
            .unwrap();
        let step = projection
            .steps
            .iter()
            .find(|step| step.step == LogicalCodebaseBootstrapStep::AggregateIndexActive)
            .unwrap();
        let failure = step
            .failure
            .clone()
            .expect("aggregate index failure detail");
        (step.status, failure)
    }

    /// 红测试 #1（片4B）：源码只在被排除的工作树目录里（第六条补充失败码）
    /// ⇒ 冷启动必须外露**自己的**稳定码，不得落通用 `aggregate_index_failed`。
    #[test]
    fn aggregate_source_excluded_projects_dedicated_reason_code() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let lc_id = create_project_and_lc(&paths, &temp.path().join("aggregate-root"));
        seed_failed_index_with_warning(
            &paths,
            &lc_id,
            "aggregate_index_source_excluded_0001",
            "aggregate_index_failed:aggregate_index_member_source_excluded: member api has Aria \
             sources only under .worktrees/; included members [api]",
        );

        let (status, failure) = projected_aggregate_failure(&paths, &lc_id);
        assert_eq!(status, LogicalCodebaseBootstrapStepStatus::Failed);
        assert_eq!(
            failure.reason_code, "aggregate_index_member_source_excluded",
            "补充失败码必须原样外露，不得落成 aggregate_index_failed"
        );
        let copy = failure.copy.as_ref().expect("失败面必须带同源成品文案");
        assert_eq!(copy.machine_line, "aggregate_index_member_source_excluded");
        assert!(!copy.title_zh.trim().is_empty());
        assert!(!copy.reason_zh.trim().is_empty());
        assert!(!copy.action_zh.trim().is_empty());
        // 点名：封套事实里的成员名必须进原因段（该码模板的原因段带成员位）。
        assert!(
            copy.reason_zh.contains("api"),
            "成员名必须进原因段：{}",
            copy.reason_zh
        );
    }

    /// 红测试 #2（片4B）：盘检无法确认（第六条补充失败码）⇒ 冷启动外露
    /// 自己的稳定码，且**不得**宣称该成员没有源码（文案取自同一张码表）。
    #[test]
    fn aggregate_scan_incomplete_projects_dedicated_reason_code() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let lc_id = create_project_and_lc(&paths, &temp.path().join("aggregate-root"));
        seed_failed_index_with_warning(
            &paths,
            &lc_id,
            "aggregate_index_scan_incomplete_0001",
            "aggregate_index_failed:aggregate_index_source_scan_incomplete: scan budget \
             exhausted for member api; included members [api]",
        );

        let (status, failure) = projected_aggregate_failure(&paths, &lc_id);
        assert_eq!(status, LogicalCodebaseBootstrapStepStatus::Failed);
        assert_eq!(
            failure.reason_code, "aggregate_index_source_scan_incomplete",
            "补充失败码必须原样外露，不得落成 aggregate_index_failed"
        );
        let copy = failure.copy.as_ref().expect("失败面必须带同源成品文案");
        assert_eq!(copy.machine_line, "aggregate_index_source_scan_incomplete");
        assert!(!copy.title_zh.trim().is_empty());
        assert!(!copy.reason_zh.trim().is_empty());
        assert!(!copy.action_zh.trim().is_empty());
        // 该码模板的点名位在问题段（标题），不在原因段。
        assert!(
            copy.title_zh.contains("api"),
            "成员名必须进成品：{}",
            copy.title_zh
        );
    }

    /// 红测试 #3（片4B）：全员确认无源码（第六条补充失败码）⇒ 冷启动外露
    /// 自己的稳定码。
    #[test]
    fn aggregate_all_members_source_less_projects_dedicated_reason_code() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let lc_id = create_project_and_lc(&paths, &temp.path().join("aggregate-root"));
        seed_failed_index_with_warning(
            &paths,
            &lc_id,
            "aggregate_index_all_source_less_0001",
            "aggregate_index_failed:aggregate_index_all_members_source_less: no Aria source \
             under any included member; included members [api, web]",
        );

        let (status, failure) = projected_aggregate_failure(&paths, &lc_id);
        assert_eq!(status, LogicalCodebaseBootstrapStepStatus::Failed);
        assert_eq!(
            failure.reason_code, "aggregate_index_all_members_source_less",
            "补充失败码必须原样外露，不得落成 aggregate_index_failed"
        );
        let copy = failure.copy.as_ref().expect("失败面必须带同源成品文案");
        assert_eq!(copy.machine_line, "aggregate_index_all_members_source_less");
        assert!(!copy.title_zh.trim().is_empty());
        assert!(!copy.reason_zh.trim().is_empty());
        assert!(!copy.action_zh.trim().is_empty());
        // 该码模板无成员位 ⇒ 点名不可断言；改为断言与共享入口逐字一致。
        let envelope = "aggregate_index_failed:aggregate_index_all_members_source_less: no Aria \
                        source under any included member; included members [api, web]";
        let (shared_code, shared_copy) =
            crate::product::logical_codebase::failure_copy::describe_envelope(envelope)
                .expect("共享入口必须能解析该封套");
        assert_eq!(shared_code, "aggregate_index_all_members_source_less");
        assert_eq!(copy.title_zh, shared_copy.title_zh);
        assert_eq!(copy.reason_zh, shared_copy.reason_zh);
        assert_eq!(copy.action_zh, shared_copy.action_zh);
        assert_eq!(copy.machine_line, shared_copy.machine_line);
    }

    /// 红测试 #4（片4B · F2 点名守卫）：同一封套下，冷启动投影的成品与
    /// **共享产品层入口**（`failure_copy::describe_envelope`）逐字一致，
    /// 且成员名**没有**在冷启动出口丢掉。
    #[test]
    fn bootstrap_projection_matches_shared_envelope_entry() {
        use crate::product::logical_codebase::failure_copy::{describe_code, describe_envelope};

        let envelope = "aggregate_index_failed:aggregate_index_member_coverage_failed: \
             no indexable code file under included members [gray-config, msha-sdk]; each included \
             member needs at least 1 indexable code file";
        let fact = "no indexable code file under included members [gray-config, msha-sdk]; \
             each included member needs at least 1 indexable code file";

        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let lc_id = create_project_and_lc(&paths, &temp.path().join("aggregate-root"));
        seed_failed_index_with_warning(
            &paths,
            &lc_id,
            "aggregate_index_shared_entry_0001",
            envelope,
        );

        let (status, failure) = projected_aggregate_failure(&paths, &lc_id);
        assert_eq!(status, LogicalCodebaseBootstrapStepStatus::Failed);
        let copy = failure.copy.as_ref().expect("失败面必须带同源成品文案");

        let (shared_code, shared_copy) =
            describe_envelope(envelope).expect("共享入口必须能解析该封套");
        assert_eq!(shared_code, "aggregate_index_member_coverage_failed");
        assert_eq!(failure.reason_code, shared_code);
        assert_eq!(copy.title_zh, shared_copy.title_zh);
        assert_eq!(copy.reason_zh, shared_copy.reason_zh);
        assert_eq!(copy.action_zh, shared_copy.action_zh);
        assert_eq!(copy.machine_line, shared_copy.machine_line);

        // 点名守卫：成员名必须在冷启动出口的成品里。
        assert!(
            copy.reason_zh.contains("gray-config"),
            "冷启动出口不得丢掉成员点名：{}",
            copy.reason_zh
        );
        assert!(
            copy.reason_zh.contains("msha-sdk"),
            "冷启动出口不得丢掉成员点名：{}",
            copy.reason_zh
        );
        // 对照：不经封套入口（无 subject）的查表丢点名 ⇒ 本用例不是靠
        // 「两边都丢点名」通过的。
        let control = describe_code("aggregate_index_member_coverage_failed", Some(fact));
        assert!(
            !control.reason_zh.contains("gray-config") && !control.reason_zh.contains("msha-sdk"),
            "对照入口不应带点名：{}",
            control.reason_zh
        );
    }

    /// 红测试 #5（片4B）：裸前缀判断已删除——正文里偶然出现某个失败码的
    /// 历史告警**不得**被当成该失败码（旧实现按 `contains` 猜码）。
    #[test]
    fn bootstrap_does_not_match_failure_codes_by_bare_prefix() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let lc_id = create_project_and_lc(&paths, &temp.path().join("aggregate-root"));
        seed_failed_index_with_warning(
            &paths,
            &lc_id,
            "aggregate_index_bare_prefix_0001",
            "aggregate_index_failed: abort before member scan: \
             aggregate_index_member_coverage_failed phase never started",
        );

        let (status, failure) = projected_aggregate_failure(&paths, &lc_id);
        assert_eq!(status, LogicalCodebaseBootstrapStepStatus::Failed);
        assert_eq!(
            failure.reason_code, "aggregate_index_failed",
            "正文里出现的失败码不得被裸前缀匹配成专用码"
        );
    }

    /// 红测试 #6（片4B）：历史记录（非封套形态的旧告警）仍渲染旧文案——
    /// 通用码 + 同一张码表里的成品，不因改用封套入口而变样。
    #[test]
    fn bootstrap_legacy_warning_keeps_generic_code_and_copy() {
        use crate::product::logical_codebase::failure_copy::describe_code;

        let warning = "codegraph_version_mismatch: expected 1.6.0, got 1.6.1";
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let lc_id = create_project_and_lc(&paths, &temp.path().join("aggregate-root"));
        seed_failed_index_with_warning(
            &paths,
            &lc_id,
            "aggregate_index_legacy_warning_0001",
            warning,
        );

        let (status, failure) = projected_aggregate_failure(&paths, &lc_id);
        assert_eq!(status, LogicalCodebaseBootstrapStepStatus::Failed);
        assert_eq!(failure.reason_code, "aggregate_index_failed");
        let expected = describe_code("aggregate_index_failed", Some(warning));
        let copy = failure.copy.as_ref().expect("失败面必须带同源成品文案");
        assert_eq!(copy.title_zh, expected.title_zh);
        assert_eq!(copy.reason_zh, expected.reason_zh);
        assert_eq!(copy.action_zh, expected.action_zh);
        assert_eq!(copy.machine_line, expected.machine_line);
    }

    /// 红测试 #7（片4B）：冷启动投影源码里不得再有裸前缀失败码判断
    /// （`warning.contains("<code>")`）——契约要求两出口都经唯一解析入口。
    #[test]
    fn bootstrap_projector_source_has_no_bare_prefix_failure_matching() {
        let source_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src/product/logical_codebase/bootstrap_projector.inc.rs");
        let source =
            std::fs::read_to_string(&source_path).expect("冷启动投影源码必须可读（源码守卫用）");
        assert!(
            !source.contains(".contains(\"aggregate_index_"),
            "冷启动投影不得用裸前缀 contains 猜失败码"
        );
    }

    // Task 2（aggregate-policy-root-publication）：桩等待谓词专用测试。
    // 按既有 .inc 拆分先例（tests_parts）把本主题测试拆入独立 part,
    /// 政策漂移只允许一个可执行动作：重发生成新 revision。
    /// 任何"空转按钮"(retry/revalidate)都不得再出现在政策步骤的动作集里。
    #[test]
    fn policy_drift_advertises_only_republish() {
        assert_eq!(
            super::policy_digest_drift_actions(),
            vec![crate::product::logical_codebase::BootstrapActionKind::Republish]
        );
        assert_eq!(
            super::policy_reference_drift_actions(),
            vec![crate::product::logical_codebase::BootstrapActionKind::Republish]
        );
    }
}
