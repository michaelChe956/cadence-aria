#[cfg(test)]
mod aggregate_index_tests {
    use super::*;
    use crate::product::logical_codebase::aggregate_index::AggregateIndexRecord;
    use super::tests::create_project_and_lc;

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
