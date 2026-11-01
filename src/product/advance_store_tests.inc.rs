#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn record() -> AdvanceRecord {
        AdvanceRecord {
            id: "advance_0001".into(),
            command_id: "command_0001".into(),
            project_id: "project_0001".into(),
            issue_id: "issue_0001".into(),
            plan_id: "plan_0001".into(),
            plan_revision_id: "revision_0001".into(),
            attempt_id: Some("attempt_0001".into()),
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
            target_attempts: Vec::new(),
            status: AdvanceStatus::Ready,
            workspace_entry: Some("/workspaces/attempt_0001".into()),
            error: None,
            created_at: "2026-08-31T00:00:00Z".into(),
            updated_at: "2026-08-31T00:00:01Z".into(),
        }
    }

    #[test]
    fn advance_record_store_supports_both_idempotency_indexes() {
        let root = TempDir::new().unwrap();
        let store = AdvanceStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let value = record();
        assert_eq!(store.put_record(&value).unwrap(), value);
        assert_eq!(
            store
                .get_advance_by_command_id("project_0001", "issue_0001", "command_0001")
                .unwrap(),
            Some(value.clone())
        );
        assert_eq!(
            store
                .get_advance_for_plan("project_0001", "issue_0001", "plan_0001")
                .unwrap(),
            Some(value)
        );
    }

    #[test]
    fn persist_advance_record_if_absent_is_idempotent_and_binds_revision() {
        let root = TempDir::new().unwrap();
        let store = AdvanceStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let input = AdvanceInput {
            command_id: "command_0001".into(),
            project_id: "project_0001".into(),
            issue_id: "issue_0001".into(),
            plan_id: "plan_0001".into(),
        };
        let first = store
            .persist_advance_record_if_absent(
                &input,
                "revision_0001",
                crate::product::coding_models::CodingStartRunPolicy::Manual,
            )
            .unwrap();
        let replay = store
            .persist_advance_record_if_absent(
                &input,
                "revision_changed",
                crate::product::coding_models::CodingStartRunPolicy::Manual,
            )
            .unwrap();
        assert_eq!(first, replay);
        assert_eq!(replay.plan_revision_id, "revision_0001");
        assert_eq!(replay.status, AdvanceStatus::Initializing);
    }

    #[test]
    fn persist_advance_record_if_absent_replays_by_plan_before_new_command() {
        let root = TempDir::new().unwrap();
        let store = AdvanceStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let first_input = AdvanceInput {
            command_id: "command_0001".into(),
            project_id: "project_0001".into(),
            issue_id: "issue_0001".into(),
            plan_id: "plan_0001".into(),
        };
        let second_input = AdvanceInput {
            command_id: "command_0002".into(),
            ..first_input.clone()
        };
        let first = store
            .persist_advance_record_if_absent(
                &first_input,
                "revision_0001",
                crate::product::coding_models::CodingStartRunPolicy::Manual,
            )
            .unwrap();
        assert_eq!(
            store
                .persist_advance_record_if_absent(
                    &second_input,
                    "revision_0002",
                    crate::product::coding_models::CodingStartRunPolicy::Manual,
                )
                .unwrap(),
            first
        );
    }

    #[test]
    fn advance_initialization_journal_rejects_identity_drift() {
        let record = record();
        let journal = AdvanceInitializationJournal {
            advance_id: "advance_other".into(),
            plan_id: record.plan_id.clone(),
            attempt_id: "attempt_0001".into(),
            target_attempt_ids: Vec::new(),
            phase: AdvanceInitializationPhase::JournalPrepared,
            error: None,
            created_at: record.created_at.clone(),
            updated_at: record.updated_at.clone(),
        };
        assert!(matches!(
            validate_advance_initialization_journal(&journal, &record),
            Err(ProductStoreError::IdentityMismatch { .. })
        ));
    }

    fn target_binding(target: &str, attempt: &str) -> AdvanceTargetAttemptBinding {
        AdvanceTargetAttemptBinding {
            target_repository_id: target.to_string(),
            attempt_id: attempt.to_string(),
        }
    }

    #[test]
    fn advance_ready_judgment_keeps_exact_match_semantics_for_single_target() {
        // REQ-ADV-02（约束 5 单 target 零变化）：单值 attempt_id 精确匹配语义逐字节
        // 保持——空 target_attempts 集下仅精确匹配放行，其余一律 fail-closed。
        let root = TempDir::new().unwrap();
        let store = AdvanceStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let mut value = record();
        value.target_attempts = Vec::new();
        store.put_record(&value).unwrap();

        assert!(
            store
                .advance_is_ready_for_attempt(
                    "project_0001",
                    "issue_0001",
                    "plan_0001",
                    "attempt_0001"
                )
                .unwrap()
        );
        assert!(
            !store
                .advance_is_ready_for_attempt(
                    "project_0001",
                    "issue_0001",
                    "plan_0001",
                    "attempt_other"
                )
                .unwrap()
        );
        assert!(
            !store
                .advance_is_ready_for_attempt(
                    "project_0001",
                    "issue_0001",
                    "plan_missing",
                    "attempt_0001"
                )
                .unwrap()
        );
    }

    #[test]
    fn advance_ready_judgment_accepts_target_attempt_set_membership() {
        // REQ-ADV-01/02（OQ1）：多 target record 的判定=精确匹配 OR 集合包含——
        // 每 target-attempt 各自 per-attempt 放行。
        let root = TempDir::new().unwrap();
        let store = AdvanceStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let mut value = record();
        value.attempt_id = Some("attempt_first".into());
        value.target_attempts = vec![
            target_binding("11111111-1111-1111-1111-111111111111", "attempt_first"),
            target_binding("22222222-2222-2222-2222-222222222222", "attempt_second"),
        ];
        store.put_record(&value).unwrap();

        assert!(
            store
                .advance_is_ready_for_attempt(
                    "project_0001",
                    "issue_0001",
                    "plan_0001",
                    "attempt_first"
                )
                .unwrap()
        );
        assert!(
            store
                .advance_is_ready_for_attempt(
                    "project_0001",
                    "issue_0001",
                    "plan_0001",
                    "attempt_second"
                )
                .unwrap()
        );
        // 双不匹配 fail-closed（守卫零变化红线）。
        assert!(
            !store
                .advance_is_ready_for_attempt(
                    "project_0001",
                    "issue_0001",
                    "plan_0001",
                    "attempt_unbound"
                )
                .unwrap()
        );
    }

    #[test]
    fn advance_ready_judgment_fails_closed_when_not_ready_or_unbound() {
        let root = TempDir::new().unwrap();
        let store = AdvanceStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let mut value = record();
        value.status = AdvanceStatus::Initializing;
        value.target_attempts = vec![target_binding(
            "11111111-1111-1111-1111-111111111111",
            "attempt_0001",
        )];
        store.put_record(&value).unwrap();

        // 未 Ready 一律拒绝（缺记录由上一测试覆盖）。
        assert!(
            !store
                .advance_is_ready_for_attempt(
                    "project_0001",
                    "issue_0001",
                    "plan_0001",
                    "attempt_0001"
                )
                .unwrap()
        );
    }

    #[test]
    fn legacy_advance_record_json_deserializes_without_target_attempts() {
        // REQ-MTG-03 scenario「绑定关系 additive 扩展兼容存量」：本 change 前形态
        // JSON（无 target_attempts 键）反序列化 → 空集，判定路径与语义零变化。
        let legacy_json = serde_json::json!({
            "id": "advance_0001",
            "command_id": "command_0001",
            "project_id": "project_0001",
            "issue_id": "issue_0001",
            "plan_id": "plan_0001",
            "plan_revision_id": "revision_0001",
            "attempt_id": "attempt_0001",
            "status": "ready",
            "workspace_entry": "/workspaces/attempt_0001",
            "error": null,
            "created_at": "2026-08-31T00:00:00Z",
            "updated_at": "2026-08-31T00:00:01Z"
        });
        let value: AdvanceRecord = serde_json::from_value(legacy_json).unwrap();
        assert_eq!(value.attempt_id.as_deref(), Some("attempt_0001"));
        assert!(value.target_attempts.is_empty());
        // 序列化往返：空集不落键（additive-only wire）。
        let serialized = serde_json::to_value(&value).unwrap();
        assert!(serialized.get("target_attempts").is_none());
    }

    #[test]
    fn advance_initialization_journal_legacy_json_defaults_target_attempt_ids() {
        let legacy_json = serde_json::json!({
            "advance_id": "advance_0001",
            "plan_id": "plan_0001",
            "attempt_id": "attempt_0001",
            "phase": "journal_prepared",
            "error": null,
            "created_at": "2026-08-31T00:00:00Z",
            "updated_at": "2026-08-31T00:00:01Z"
        });
        let journal: AdvanceInitializationJournal = serde_json::from_value(legacy_json).unwrap();
        assert_eq!(journal.attempt_id, "attempt_0001");
        assert!(journal.target_attempt_ids.is_empty());
    }

    // ─── 级联删除 Task D1.2c：advance record/journal/retry 闭包 ───

    use crate::product::entity_deletion_store::{
        DeletionEntityKind, DeletionPhase, EntityDeletionManifest, EntityDeletionStore, EntityScope,
    };
    use crate::product::entity_operation_store::EntityOperationStore;
    use crate::product::models::automation::OperationState;

    fn cascade_store() -> (TempDir, AdvanceStore) {
        let root = TempDir::new().unwrap();
        let store = AdvanceStore::new(ProductAppPaths::new(root.path().join(".aria")));
        (root, store)
    }

    fn cascade_record(
        id: &str,
        command_id: &str,
        plan_id: &str,
        attempt_id: Option<&str>,
        status: AdvanceStatus,
    ) -> AdvanceRecord {
        AdvanceRecord {
            id: id.into(),
            command_id: command_id.into(),
            project_id: "project_0001".into(),
            issue_id: "issue_0001".into(),
            plan_id: plan_id.into(),
            plan_revision_id: "revision_0001".into(),
            attempt_id: attempt_id.map(Into::into),
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
            target_attempts: Vec::new(),
            status,
            workspace_entry: attempt_id.map(|attempt| format!("/workspaces/{attempt}")),
            error: None,
            created_at: "2026-10-13T00:00:00Z".into(),
            updated_at: "2026-10-13T00:00:01Z".into(),
        }
    }

    fn cascade_journal(
        advance_id: &str,
        plan_id: &str,
        attempt_id: &str,
    ) -> AdvanceInitializationJournal {
        AdvanceInitializationJournal {
            advance_id: advance_id.into(),
            plan_id: plan_id.into(),
            attempt_id: attempt_id.into(),
            target_attempt_ids: Vec::new(),
            phase: AdvanceInitializationPhase::JournalPrepared,
            error: None,
            created_at: "2026-10-13T00:00:00Z".into(),
            updated_at: "2026-10-13T00:00:01Z".into(),
        }
    }

    fn cascade_retry(command_id: &str, advance_id: &str) -> RetryInitializationRecord {
        RetryInitializationRecord {
            retry_id: format!("advance_retry_{command_id}"),
            command_id: command_id.into(),
            advance_id: advance_id.into(),
            attempt_id: "attempt_0001".into(),
            state: OperationState::Accepted,
            checkpoint: AdvanceInitializationPhase::JournalPrepared,
            created_at: "2026-10-13T00:00:00Z".into(),
            updated_at: "2026-10-13T00:00:01Z".into(),
        }
    }

    fn seed_retry(store: &AdvanceStore, command_id: &str, advance_id: &str) {
        write_json(
            &store
                .retry_path("project_0001", "issue_0001", command_id)
                .unwrap(),
            &cascade_retry(command_id, advance_id),
        )
        .unwrap();
    }

    #[test]
    fn cascade_d1_2c_advance_owner_purge_two_plans_accurate_attribution() {
        // REQ-GRP-01「Plan 所属 advance 整条内容随删除清除」：同 issue 两 Plan 的
        // record/journal/retry 按真实 plan_id/advance_id 归属，只清目标 Plan，
        // 不删他 Plan 对象、不整删 advance-records/advance-retries 根。
        let (_root, store) = cascade_store();
        let (p, i) = ("project_0001", "issue_0001");

        let first = cascade_record(
            "advance_command_0001",
            "command_0001",
            "plan_0001",
            Some("attempt_0001"),
            AdvanceStatus::Ready,
        );
        store.put_record(&first).unwrap();
        // 存量同 plan 多 record（legacy）：put_record 的 plan 唯一性检查会拒，
        // 直接落盘模拟历史数据——list 必须全 issue 枚举而非仅首条。
        let legacy = cascade_record(
            "advance_command_0002",
            "command_0002",
            "plan_0001",
            None,
            AdvanceStatus::Aborted,
        );
        write_json(
            &store.path_for(p, i, "advance_command_0002").unwrap(),
            &legacy,
        )
        .unwrap();
        let other = cascade_record(
            "advance_command_0003",
            "command_0003",
            "plan_0002",
            Some("attempt_0002"),
            AdvanceStatus::Ready,
        );
        store.put_record(&other).unwrap();
        store
            .save_advance_initialization(
                &first,
                &cascade_journal("advance_command_0001", "plan_0001", "attempt_0001"),
            )
            .unwrap();
        store
            .save_advance_initialization(
                &other,
                &cascade_journal("advance_command_0003", "plan_0002", "attempt_0002"),
            )
            .unwrap();
        seed_retry(&store, "retry_cmd_0001", "advance_command_0001");
        seed_retry(&store, "retry_cmd_0002", "advance_command_0003");

        let target = store.list_plan_artifacts(p, i, "plan_0001").unwrap();
        assert_eq!(
            target.advance_ids,
            vec!["advance_command_0001", "advance_command_0002"]
        );
        assert_eq!(target.retry_command_ids, vec!["retry_cmd_0001"]);
        let bystander = store.list_plan_artifacts(p, i, "plan_0002").unwrap();
        assert_eq!(bystander.advance_ids, vec!["advance_command_0003"]);
        assert_eq!(bystander.retry_command_ids, vec!["retry_cmd_0002"]);

        let scope = EntityScope::new(p, i, DeletionEntityKind::Plan, "plan_0001");
        let operations = EntityOperationStore::new(store.app_paths());
        let deletions = EntityDeletionStore::new(store.app_paths());
        let lease = operations.try_acquire_delete(&scope).unwrap();
        assert_eq!(
            deletions.begin(&lease).unwrap().phase,
            DeletionPhase::Purging
        );
        // purging 阶段仍可枚举（§0.5：purging/deleted scope 允许 list 查询）。
        assert_eq!(
            store.list_plan_artifacts(p, i, "plan_0001").unwrap(),
            target
        );
        // pending manifest 捕获 advance/retry 纯 ID 集（重入事实）。
        let mut manifest = EntityDeletionManifest::empty(scope.clone());
        manifest.advance_ids = target.advance_ids.clone();
        manifest.advance_retry_command_ids = target.retry_command_ids.clone();
        deletions.save_pending_manifest(&lease, &manifest).unwrap();
        assert_eq!(
            deletions
                .load_pending_manifest(&lease)
                .unwrap()
                .unwrap()
                .advance_ids,
            vec!["advance_command_0001", "advance_command_0002"]
        );

        // 清理序 retry→journal→record；missing 幂等（同 lease 二次 purge）。
        store.purge_plan_artifacts(&lease, &target).unwrap();
        store.purge_plan_artifacts(&lease, &target).unwrap();

        assert!(
            store
                .get_advance_for_plan(p, i, "plan_0001")
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .get_advance_by_command_id(p, i, "command_0001")
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .get_advance_by_command_id(p, i, "command_0002")
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .get_retry_initialization(p, i, "retry_cmd_0001")
                .unwrap()
                .is_none()
        );
        assert!(
            !store
                .root(p, i)
                .unwrap()
                .join("plan_0001.initialization.json")
                .is_file()
        );
        // 他 Plan 原样保留。
        assert_eq!(
            store
                .get_advance_for_plan(p, i, "plan_0002")
                .unwrap()
                .unwrap()
                .id,
            "advance_command_0003"
        );
        assert!(
            store
                .get_retry_initialization(p, i, "retry_cmd_0002")
                .unwrap()
                .is_some()
        );
        assert!(
            store
                .root(p, i)
                .unwrap()
                .join("plan_0002.initialization.json")
                .is_file()
        );
        // 不整删 advance-records/advance-retries 根。
        assert!(store.root(p, i).unwrap().is_dir());
        assert!(store.retries_root(p, i).unwrap().is_dir());

        // 真实协调序：清理成功后先移除 pending manifest 再 complete（D1.0 契约）。
        deletions.remove_pending_manifest(&lease).unwrap();
        assert_eq!(
            deletions.complete(&lease).unwrap().phase,
            DeletionPhase::Deleted
        );
        // Ready 准入：删除事实优先；未删 Plan 原精确 join 不变。
        assert!(
            !store
                .advance_is_ready_for_attempt(p, i, "plan_0001", "attempt_0001")
                .unwrap()
        );
        assert!(
            store
                .advance_is_ready_for_attempt(p, i, "plan_0002", "attempt_0002")
                .unwrap()
        );
    }

    #[test]
    fn cascade_d1_2c_deleted_plan_rejects_replay_ready_and_late_writes() {
        // REQ-GRP-01「删除终止 advance 重放与 Ready 放行」：已删除身份的同/异
        // command 重放、retry 与迟到 owner 写依据删除事实优先拒绝，不建目录；
        // 旧 Ready 残留查询 false；未删 Plan 原幂等合同由既有测试守护。
        let (_root, store) = cascade_store();
        let (p, i) = ("project_0001", "issue_0001");
        let record = cascade_record(
            "advance_command_0001",
            "command_0001",
            "plan_0001",
            Some("attempt_0001"),
            AdvanceStatus::Ready,
        );
        store.put_record(&record).unwrap();
        let journal = cascade_journal("advance_command_0001", "plan_0001", "attempt_0001");
        store
            .save_advance_initialization(&record, &journal)
            .unwrap();
        seed_retry(&store, "retry_cmd_0001", "advance_command_0001");

        let scope = EntityScope::new(p, i, DeletionEntityKind::Plan, "plan_0001");
        let operations = EntityOperationStore::new(store.app_paths());
        let deletions = EntityDeletionStore::new(store.app_paths());
        let lease = operations.try_acquire_delete(&scope).unwrap();
        deletions.begin(&lease).unwrap();
        let artifacts = store.list_plan_artifacts(p, i, "plan_0001").unwrap();
        store.purge_plan_artifacts(&lease, &artifacts).unwrap();
        assert_eq!(
            deletions.complete(&lease).unwrap().phase,
            DeletionPhase::Deleted
        );
        drop(lease);

        // 模拟完成后全局清理：advance 两根整体移除（根/摘要均缺）。
        std::fs::remove_dir_all(store.root(p, i).unwrap()).unwrap();
        std::fs::remove_dir_all(store.retries_root(p, i).unwrap()).unwrap();

        fn assert_deleting<T>(result: Result<T, ProductStoreError>) {
            assert!(matches!(
                result,
                Err(ProductStoreError::Conflict { kind, .. }) if kind == "entity_deleting"
            ));
        }
        // 同 command_id 重放 → 拒绝（不复活旧 record）。
        let replay = AdvanceInput {
            command_id: "command_0001".into(),
            project_id: p.into(),
            issue_id: i.into(),
            plan_id: "plan_0001".into(),
        };
        assert_deleting(store.persist_advance_record_if_absent(
            &replay,
            "revision_0001",
            crate::product::coding_models::CodingStartRunPolicy::Manual,
        ));
        // 不同 command_id 新 advance → 同样拒绝。
        let replay_fresh = AdvanceInput {
            command_id: "command_0009".into(),
            ..replay.clone()
        };
        assert_deleting(store.persist_advance_record_if_absent(
            &replay_fresh,
            "revision_0001",
            crate::product::coding_models::CodingStartRunPolicy::Manual,
        ));
        // 迟到 put/update/journal/reopen 全部拒绝且不重建目录。
        let late = cascade_record(
            "advance_command_0009",
            "command_0009",
            "plan_0001",
            None,
            AdvanceStatus::Initializing,
        );
        assert_deleting(store.put_record(&late));
        let mut updated = record.clone();
        updated.status = AdvanceStatus::Running;
        assert_deleting(store.update_record(&updated));
        assert_deleting(store.save_advance_initialization(&record, &journal));
        assert_deleting(store.load_or_prepare_advance_initialization_for_attempts(
            &record,
            "attempt_0001",
            &[],
        ));
        assert_deleting(store.advance_initialization_phase(
            &record,
            &journal,
            AdvanceInitializationPhase::AttemptPersisted,
        ));
        assert_deleting(store.reopen_failed_record_for_retry(&record));
        // retry 更新经 advance_id 反查 plan：record 已不存在 → fail-closed 拒绝。
        assert!(
            store
                .update_retry_initialization_state(
                    p,
                    i,
                    &cascade_retry("retry_cmd_0001", "advance_command_0001"),
                    OperationState::NeedsHuman,
                )
                .is_err()
        );
        assert!(!store.root(p, i).unwrap().is_dir());
        assert!(!store.retries_root(p, i).unwrap().is_dir());

        // 旧 Ready 残留查询：删除事实优先于残留 record 内容。
        write_json(
            &store.path_for(p, i, "advance_command_0001").unwrap(),
            &record,
        )
        .unwrap();
        assert!(
            !store
                .advance_is_ready_for_attempt(p, i, "plan_0001", "attempt_0001")
                .unwrap()
        );
    }

    #[test]
    fn cascade_d1_2c_list_reports_orphan_journal_without_plan_root() {
        // §0.5 list-by-owner：孤儿 initialization journal 按 {plan_id} 文件名归属
        // 仍可枚举（不依赖 Plan 根/pending），并按真实归属清掉；根缺失=已清。
        let (_root, store) = cascade_store();
        let (p, i) = ("project_0001", "issue_0001");
        let other = cascade_record(
            "advance_command_0003",
            "command_0003",
            "plan_0002",
            Some("attempt_0002"),
            AdvanceStatus::Ready,
        );
        store.put_record(&other).unwrap();
        // plan_0001 无 record，仅有孤儿 journal（模拟清理中断残留）。
        let journal_path = store
            .root(p, i)
            .unwrap()
            .join("plan_0001.initialization.json");
        write_json(
            &journal_path,
            &cascade_journal("advance_command_0001", "plan_0001", "attempt_0001"),
        )
        .unwrap();

        let artifacts = store.list_plan_artifacts(p, i, "plan_0001").unwrap();
        assert_eq!(artifacts.advance_ids, vec!["advance_command_0001"]);
        assert!(artifacts.retry_command_ids.is_empty());

        let scope = EntityScope::new(p, i, DeletionEntityKind::Plan, "plan_0001");
        let operations = EntityOperationStore::new(store.app_paths());
        let deletions = EntityDeletionStore::new(store.app_paths());
        let lease = operations.try_acquire_delete(&scope).unwrap();
        deletions.begin(&lease).unwrap();
        store.purge_plan_artifacts(&lease, &artifacts).unwrap();
        assert!(!journal_path.is_file());
        // 根与他 Plan 对象不动。
        assert!(store.root(p, i).unwrap().is_dir());
        assert!(
            store
                .get_advance_for_plan(p, i, "plan_0002")
                .unwrap()
                .is_some()
        );
        deletions.complete(&lease).unwrap();
        assert!(
            store
                .list_plan_artifacts(p, i, "plan_0001")
                .unwrap()
                .advance_ids
                .is_empty()
        );
        // 根整体缺失=已清：空集、不报错、不建目录。
        std::fs::remove_dir_all(store.root(p, i).unwrap()).unwrap();
        assert_eq!(
            store.list_plan_artifacts(p, i, "plan_0001").unwrap(),
            AdvancePlanArtifacts::default()
        );
        assert!(!store.root(p, i).unwrap().is_dir());
    }

    #[test]
    fn cascade_d1_2c_list_fails_closed_on_corrupt_or_misjoined_artifacts() {
        // 坏 JSON/错 join 一律报错，不当空集（fail-closed）。
        let (p, i) = ("project_0001", "issue_0001");
        // a) record 坏 JSON。
        {
            let (_root, store) = cascade_store();
            let root = store.root(p, i).unwrap();
            std::fs::create_dir_all(&root).unwrap();
            std::fs::write(root.join("advance_broken.json"), b"{ not json").unwrap();
            assert!(store.list_plan_artifacts(p, i, "plan_0001").is_err());
        }
        // b) journal 内容 plan_id 与文件名键不符。
        {
            let (_root, store) = cascade_store();
            write_json(
                &store
                    .root(p, i)
                    .unwrap()
                    .join("plan_0001.initialization.json"),
                &cascade_journal("advance_command_0001", "plan_0002", "attempt_0001"),
            )
            .unwrap();
            assert!(matches!(
                store.list_plan_artifacts(p, i, "plan_0001"),
                Err(ProductStoreError::IdentityMismatch { .. })
            ));
        }
        // c) retry 文件名与内容 command_id 不符（错 join）。
        {
            let (_root, store) = cascade_store();
            write_json(
                &store
                    .retries_root(p, i)
                    .unwrap()
                    .join("retry_cmd_0001.json"),
                &cascade_retry("retry_cmd_other", "advance_command_0001"),
            )
            .unwrap();
            assert!(matches!(
                store.list_plan_artifacts(p, i, "plan_0001"),
                Err(ProductStoreError::IdentityMismatch { .. })
            ));
        }
        // d) journal.advance_id 指向他 Plan 现存 record（错 join）。
        {
            let (_root, store) = cascade_store();
            let other = cascade_record(
                "advance_command_0003",
                "command_0003",
                "plan_0002",
                Some("attempt_0002"),
                AdvanceStatus::Ready,
            );
            store.put_record(&other).unwrap();
            write_json(
                &store
                    .root(p, i)
                    .unwrap()
                    .join("plan_0001.initialization.json"),
                &cascade_journal("advance_command_0003", "plan_0001", "attempt_0001"),
            )
            .unwrap();
            assert!(matches!(
                store.list_plan_artifacts(p, i, "plan_0001"),
                Err(ProductStoreError::IdentityMismatch { .. })
            ));
        }
    }

    #[test]
    fn cascade_d1_2c_persist_window_deletion_never_yields_stale_ok_or_identity_reclass() {
        // P3 修复轮（REQ-GRP-01 删除身份优先）：「预检后/put 前恰 begin 删除」
        // 微秒窗口内，persist 不得把陈旧 record 当幂等 Ok 命中，也不得把
        // put_record 的 entity_deleting 改判为 advance_record_identity。
        persist_deletion_window(/* seed_ours= */ true);
        persist_deletion_window(/* seed_ours= */ false);
    }

    fn persist_deletion_window(seed_ours: bool) {
        use std::sync::{Arc, Barrier};
        use std::time::Duration;

        use crate::product::coding_attempt_store::locking::{
            ExclusiveFileLock, register_lock_attempt_hook,
        };
        use crate::product::entity_operation_store::EntityOperationError;

        let (_root, store) = cascade_store();
        let (p, i) = ("project_0001", "issue_0001");
        if seed_ours {
            let ours = cascade_record(
                "advance_command_0001",
                "command_0001",
                "plan_0001",
                Some("attempt_0001"),
                AdvanceStatus::Ready,
            );
            store.put_record(&ours).unwrap();
        } else {
            // 异 plan record 使根存在：重放读取根锁但不命中，put 走新建路径。
            let other = cascade_record(
                "advance_command_0003",
                "command_0003",
                "plan_0002",
                Some("attempt_0002"),
                AdvanceStatus::Ready,
            );
            store.put_record(&other).unwrap();
        }

        let root = store.root(p, i).unwrap();
        let held = ExclusiveFileLock::acquire(&root).expect("hold advance root lock");
        // 持锁后注册 hook：主线程自身的 acquire 不触发通知。
        let (_hook, attempts) = register_lock_attempt_hook(&root);

        let worker_store = store.clone();
        let input = AdvanceInput {
            command_id: "command_0001".into(),
            project_id: p.into(),
            issue_id: i.into(),
            plan_id: "plan_0001".into(),
        };
        let started = Arc::new(Barrier::new(2));
        let worker_started = Arc::clone(&started);
        let worker = std::thread::spawn(move || {
            worker_started.wait();
            worker_store.persist_advance_record_if_absent(
                &input,
                "revision_0001",
                crate::product::coding_models::CodingStartRunPolicy::Manual,
            )
        });
        started.wait();

        // worker 停在重放读的根锁获取上：预检已过、put 未至的注入窗口。
        attempts
            .recv_timeout(Duration::from_secs(5))
            .expect("persist worker must contend on the advance root lock");

        let scope = EntityScope::new(p, i, DeletionEntityKind::Plan, "plan_0001");
        let operations = EntityOperationStore::new(store.app_paths());
        let deletions = EntityDeletionStore::new(store.app_paths());
        let deletion_began = match operations.try_acquire_delete(&scope) {
            Ok(lease) => {
                assert_eq!(
                    deletions.begin(&lease).unwrap().phase,
                    DeletionPhase::Purging
                );
                drop(lease);
                true
            }
            Err(EntityOperationError::Busy) => false,
            Err(other) => panic!("unexpected delete admission: {other:?}"),
        };

        drop(held);
        let result = worker.join().expect("persist worker thread");

        if deletion_began {
            // 窗口内删除事实已立：不得陈旧 Ok（seed_ours=true 旧路径）、不得
            // 改判 identity（seed_ours=false 旧路径），必须上抛 entity_deleting。
            assert!(
                matches!(
                    result,
                    Err(ProductStoreError::Conflict { kind, .. }) if kind == "entity_deleting"
                ),
                "persist must surface entity_deleting after deletion began, got {result:?}"
            );
        } else {
            // 准入跨全流程持有：persist 活着完成、删除被推迟（无墓碑）。
            let record = result.expect("live persist must complete while holding admission");
            assert_eq!(record.plan_id, "plan_0001");
            assert!(deletions.get(&scope).unwrap().is_none());
        }
    }

    // ─── H2b（设计 A 节，整组删除与重启）：旧 Ready 指向已死全集识别 + 零成员清后新轮绑定 ───

    fn h2b_ready_record(
        id: &str,
        command_id: &str,
        plan_id: &str,
        targets: &[(&str, &str)],
    ) -> AdvanceRecord {
        AdvanceRecord {
            id: id.into(),
            command_id: command_id.into(),
            project_id: "project_0001".into(),
            issue_id: "issue_0001".into(),
            plan_id: plan_id.into(),
            plan_revision_id: "revision_0001".into(),
            attempt_id: targets.first().map(|(_, attempt)| (*attempt).into()),
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
            target_attempts: targets
                .iter()
                .map(|(target, attempt)| target_binding(target, attempt))
                .collect(),
            status: AdvanceStatus::Ready,
            workspace_entry: None,
            error: None,
            created_at: "2026-10-14T00:00:00Z".into(),
            updated_at: "2026-10-14T00:00:01Z".into(),
        }
    }

    #[test]
    fn h2b_incident_ready_plus_ready_journal_all_dead_is_all_targets_dead() {
        // 事故形态：旧 Ready record 与外层 journal（Ready 相位）指向同一四个
        // 已删 attempts，现存集为空 → AllTargetsDead，交零成员闭包完整重置。
        let (_root, store) = cascade_store();
        let (p, i) = ("project_0001", "issue_0001");
        let dead = [
            ("repo_a", "attempt_0001"),
            ("repo_b", "attempt_0002"),
            ("repo_c", "attempt_0003"),
            ("repo_d", "attempt_0004"),
        ];
        let record = h2b_ready_record("advance_command_0001", "command_0001", "plan_0001", &dead);
        store.put_record(&record).unwrap();
        let mut journal = cascade_journal("advance_command_0001", "plan_0001", "attempt_0001");
        journal.target_attempt_ids = dead
            .iter()
            .map(|(_, attempt)| (*attempt).to_string())
            .collect();
        journal.phase = AdvanceInitializationPhase::Ready;
        store
            .save_advance_initialization(&record, &journal)
            .unwrap();

        let facts = store.plan_execution_references(p, i, "plan_0001").unwrap();
        assert_eq!(facts.records, vec![record.clone()]);
        assert_eq!(facts.initialization_journal.as_ref(), Some(&journal));
        assert_eq!(
            facts.ready_bound_attempt_ids(),
            vec![
                "attempt_0001".to_string(),
                "attempt_0002".to_string(),
                "attempt_0003".to_string(),
                "attempt_0004".to_string(),
            ]
        );
        assert_eq!(
            facts.journal_bound_attempt_ids(),
            vec![
                "attempt_0001".to_string(),
                "attempt_0002".to_string(),
                "attempt_0003".to_string(),
                "attempt_0004".to_string(),
            ]
        );
        assert_eq!(
            facts.classify_stale_references(&[]),
            AdvanceStaleReferenceVerdict::AllTargetsDead {
                dead_attempt_ids: vec![
                    "attempt_0001".into(),
                    "attempt_0002".into(),
                    "attempt_0003".into(),
                    "attempt_0004".into(),
                ],
            }
        );
    }

    #[test]
    fn h2b_partially_dead_members_keep_partial_verdict() {
        // 删 2/4：PartiallyDead——不缩水全集、不自动清，走清剩余出口。
        let (_root, store) = cascade_store();
        let (p, i) = ("project_0001", "issue_0001");
        let targets = [
            ("repo_a", "attempt_0001"),
            ("repo_b", "attempt_0002"),
            ("repo_c", "attempt_0003"),
            ("repo_d", "attempt_0004"),
        ];
        let record = h2b_ready_record(
            "advance_command_0001",
            "command_0001",
            "plan_0001",
            &targets,
        );
        store.put_record(&record).unwrap();

        let facts = store.plan_execution_references(p, i, "plan_0001").unwrap();
        assert_eq!(
            facts.classify_stale_references(&["attempt_0001".into(), "attempt_0002".into()]),
            AdvanceStaleReferenceVerdict::PartiallyDead {
                dead_attempt_ids: vec!["attempt_0003".into(), "attempt_0004".into()],
                alive_attempt_ids: vec!["attempt_0001".into(), "attempt_0002".into()],
            }
        );
    }

    #[test]
    fn h2b_prefix_journal_below_attempt_persisted_is_not_a_broken_reference() {
        // 设计 A 节 line 74：未越 AttemptPersisted 的 journal 绑定是初始化前缀
        //（attempt 尚未物化属预期缺失）——Initializing record + Prepared
        // journal 指向缺失 attempts 不判断裂/重置，Intact 续原协议。
        let (_root, store) = cascade_store();
        let (p, i) = ("project_0001", "issue_0001");
        let mut record = h2b_ready_record("advance_command_0001", "command_0001", "plan_0001", &[]);
        record.status = AdvanceStatus::Initializing;
        store.put_record(&record).unwrap();
        let mut journal = cascade_journal("advance_command_0001", "plan_0001", "attempt_0001");
        journal.target_attempt_ids = vec!["attempt_0001".into(), "attempt_0002".into()];
        journal.phase = AdvanceInitializationPhase::JournalPrepared;
        store
            .save_advance_initialization(&record, &journal)
            .unwrap();

        let facts = store.plan_execution_references(p, i, "plan_0001").unwrap();
        assert_eq!(
            facts.classify_stale_references(&[]),
            AdvanceStaleReferenceVerdict::Intact
        );
    }

    #[test]
    fn h2b_journal_past_attempt_persisted_alone_qualifies_as_dead_full_set() {
        // 旧 Ready 缺位的中断变体：record 非 Ready，但外层 journal 已越
        // AttemptPersisted 且指向缺失 attempts → 同样构成断裂全集。
        let (_root, store) = cascade_store();
        let (p, i) = ("project_0001", "issue_0001");
        let mut record = h2b_ready_record(
            "advance_command_0001",
            "command_0001",
            "plan_0001",
            &[("repo_a", "attempt_0001")],
        );
        record.status = AdvanceStatus::Aborted;
        store.put_record(&record).unwrap();
        let mut journal = cascade_journal("advance_command_0001", "plan_0001", "attempt_0001");
        journal.target_attempt_ids = vec!["attempt_0001".into(), "attempt_0002".into()];
        journal.phase = AdvanceInitializationPhase::AttemptPersisted;
        store
            .save_advance_initialization(&record, &journal)
            .unwrap();

        let facts = store.plan_execution_references(p, i, "plan_0001").unwrap();
        assert_eq!(
            facts.classify_stale_references(&[]),
            AdvanceStaleReferenceVerdict::AllTargetsDead {
                dead_attempt_ids: vec!["attempt_0001".into(), "attempt_0002".into()],
            }
        );
    }

    #[test]
    fn h2b_verdicts_for_alive_references_and_empty_plans() {
        // 全存活 → Intact；无 record 无 journal → NoReferences；仅非 Ready
        // records → Intact（在途初始化，不判重置）。
        let (_root, store) = cascade_store();
        let (p, i) = ("project_0001", "issue_0001");
        assert_eq!(
            store
                .plan_execution_references(p, i, "plan_0001")
                .unwrap()
                .classify_stale_references(&[]),
            AdvanceStaleReferenceVerdict::NoReferences
        );

        let mut initializing =
            h2b_ready_record("advance_command_0001", "command_0001", "plan_0001", &[]);
        initializing.status = AdvanceStatus::Initializing;
        store.put_record(&initializing).unwrap();
        assert_eq!(
            store
                .plan_execution_references(p, i, "plan_0001")
                .unwrap()
                .classify_stale_references(&[]),
            AdvanceStaleReferenceVerdict::Intact
        );

        let ready = h2b_ready_record(
            "advance_command_0002",
            "command_0002",
            "plan_0002",
            &[("repo_a", "attempt_0001")],
        );
        store.put_record(&ready).unwrap();
        assert_eq!(
            store
                .plan_execution_references(p, i, "plan_0002")
                .unwrap()
                .classify_stale_references(&["attempt_0001".into()]),
            AdvanceStaleReferenceVerdict::Intact
        );
    }

    #[test]
    fn h2b_zero_member_purge_then_new_round_binds_new_command() {
        // 零成员闭包 advance 半边（复用 D1.2c 闭包，不立墓碑）：list → Plan
        // 独占 lease → purge（retry→journal→record，幂等）→ 新 command 正常
        // 绑定新轮新 attempts——「正常 advance 绑定新轮」，旧 ID 不复活。
        let (_root, store) = cascade_store();
        let (p, i) = ("project_0001", "issue_0001");
        let stale = h2b_ready_record(
            "advance_command_0001",
            "command_0001",
            "plan_0001",
            &[("repo_a", "attempt_0001"), ("repo_b", "attempt_0002")],
        );
        store.put_record(&stale).unwrap();
        let mut journal = cascade_journal("advance_command_0001", "plan_0001", "attempt_0001");
        journal.target_attempt_ids = vec!["attempt_0001".into(), "attempt_0002".into()];
        journal.phase = AdvanceInitializationPhase::Ready;
        store.save_advance_initialization(&stale, &journal).unwrap();
        seed_retry(&store, "retry_cmd_0001", "advance_command_0001");

        let artifacts = store.list_plan_artifacts(p, i, "plan_0001").unwrap();
        assert_eq!(artifacts.advance_ids, vec!["advance_command_0001"]);
        assert_eq!(artifacts.retry_command_ids, vec!["retry_cmd_0001"]);
        let scope = EntityScope::new(p, i, DeletionEntityKind::Plan, "plan_0001");
        let operations = EntityOperationStore::new(store.app_paths());
        let lease = operations.try_acquire_delete(&scope).unwrap();
        // Coding 重置不调用 begin/complete——不给保留的 Plan 立永久墓碑
        //（设计 A 节注意），租约本身即安全清理窗口。
        store.purge_plan_artifacts(&lease, &artifacts).unwrap();
        store.purge_plan_artifacts(&lease, &artifacts).unwrap();
        drop(lease);

        assert!(
            store
                .get_advance_for_plan(p, i, "plan_0001")
                .unwrap()
                .is_none()
        );
        assert!(
            store
                .get_retry_initialization(p, i, "retry_cmd_0001")
                .unwrap()
                .is_none()
        );
        assert!(
            !store
                .root(p, i)
                .unwrap()
                .join("plan_0001.initialization.json")
                .is_file()
        );

        // 新轮身份：新 command/新 attempt IDs，正常 Ready 放行。
        let fresh = h2b_ready_record(
            "advance_command_0002",
            "command_0002",
            "plan_0001",
            &[("repo_a", "attempt_1001"), ("repo_b", "attempt_1002")],
        );
        assert_eq!(store.put_record(&fresh).unwrap(), fresh);
        assert!(
            store
                .advance_is_ready_for_attempt(p, i, "plan_0001", "attempt_1001")
                .unwrap()
        );
        assert!(
            !store
                .advance_is_ready_for_attempt(p, i, "plan_0001", "attempt_0001")
                .unwrap()
        );
    }
}
