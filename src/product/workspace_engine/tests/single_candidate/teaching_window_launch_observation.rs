// Task 4（REQ-WSC-28C，tasks 2.3/2.4 接线面，bound-sc-generation-teaching）：
// 教学重驱真实启动的同步回填接线用例——SC 生成 run 驱动入口在
// `start_work_item_plan_author` 返回（spawn 身份已由 adapter/gateway 内
// `confirm_started_resource` 链确认）之后、进入事件驱动循环之前，同步回填
// `provider_start_ledger` 占位条目（原位补全，不另插第二条）；回填失败
// fail-closed（先迁移 LaunchUnknown 再走既有失败收口停 run，不自动重播）；
// 会话尾收口把 Claimed 未回填 attempt 持久化为 LaunchUnknown（provider 资源
// 注销在 adapter 的 `release_after_session_tail` 内，验证点在调用侧——
// cross_cutting 不引 store）。另含 Task 3 评审 Minor-1 的 store 加固钉子：
// observe 拒已关窗口、close 拒在飞后继 attempt。
mod teaching_window_launch_observation {
    use super::*;
    use crate::cross_cutting::provider_adapter::ProviderAdapterError;
    use crate::cross_cutting::streaming_provider::ProviderSession;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::checkpoint_store::CheckpointStore;
    use crate::product::json_store::write_json;
    use crate::product::lifecycle_store::{
        CreateWorkspaceSessionInput, ScTeachingClaimOutcome, ScTeachingEstablishOutcome,
        WorkItemPlanSessionOptions,
    };
    use crate::product::models::{ProviderName, WorkspaceType};
    use crate::product::planning_failure::PlanningFailureKind;
    use crate::product::work_item_plan_policy::{
        ProviderStartLedgerEntry, ScGenerationTeachingWindow, ScTeachingAttemptLifecycle,
        ScTeachingAttemptOutcome, ScTeachingAttemptRef, ScTeachingWindowRequest,
        ScTeachingWindowStatus,
    };

    fn digest64(seed: &str) -> String {
        format!("{seed:0<64}")
    }

    fn sc_teaching_request() -> ScTeachingWindowRequest {
        ScTeachingWindowRequest {
            provider_path_admissible: true,
            failure_source_ref: Some("work_item_plan_source:s1:rev_0001".to_string()),
            failure_source_sha256: digest64("1a"),
            failure_typed_kinds: vec![PlanningFailureKind::InvalidWorkItemHeading {
                line: 3,
                field: "ears".to_string(),
                raw_line: "## ears".to_string(),
                example: "### ears".to_string(),
            }],
            business_request_digest: digest64("2b"),
            material_digests: vec![digest64("3c"), digest64("4d")],
            context_digest: digest64("5e"),
            allowed_edit_closure: "仅允许修改 ears 与 verification 段".to_string(),
            instruction_digest: digest64("6f"),
            prompt_ref: "build_work_item_plan_compile_reredrive_prompt".to_string(),
            prompt_digest: digest64("7a"),
        }
    }

    /// 引擎级夹具：SC 会话（author=codex）→ 原生成 reserve（前缀 :0）→ 窗口
    /// establish（approved, max=2）→ claim attempt2（含 started=false 占位
    /// 条目，键由 `next_sc_author_launch_ordinal` 派生 = :1，不与 A 路径共键）。
    fn sc_teaching_launch_fixture() -> (
        tempfile::TempDir,
        LifecycleStore,
        WorkspaceEngine,
        ScTeachingAttemptRef,
    ) {
        let temp = tempfile::tempdir().expect("fixture tempdir");
        let store = LifecycleStore::new(ProductAppPaths::new(temp.path().to_path_buf()));
        let session = store
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "entity_task4".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Codex,
                reviewer_provider: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: Some(WorkItemPlanSessionOptions {
                    flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                    run_policy: RunPolicy::Interactive,
                    rollout_snapshot: true,
                }),
            })
            .expect("create candidate session");
        let key = format!("single_candidate_author:{}:0", session.id);
        let (reserved, did_start) = store
            .reserve_single_candidate_provider_start(&session, &key)
            .expect("reserve original generation launch");
        assert!(did_start);
        let established = match store
            .establish_sc_teaching_window(&reserved, &sc_teaching_request())
            .expect("establish teaching window")
        {
            ScTeachingEstablishOutcome::Established {
                record, approved, ..
            } => {
                assert!(approved, "四项判据齐备必须批准");
                record
            }
            ScTeachingEstablishOutcome::Existing { .. } => {
                panic!("fresh chain must establish a new window")
            }
        };
        let claimed = match store
            .claim_sc_teaching_attempt(&established, 0)
            .expect("claim teaching attempt")
        {
            ScTeachingClaimOutcome::Granted(record) => record,
            other => panic!("first claim must grant attempt 2, got {other:?}"),
        };
        let attempt_ref = ScTeachingAttemptRef {
            generation_seq: 0,
            attempt_no: 2,
            attempt_key: claimed.sc_generation_teaching_windows[0].attempts[1]
                .attempt_key
                .clone(),
        };
        let session = WorkspaceSession::from_record(claimed);
        let (event_tx, _event_rx) = mpsc::channel(64);
        let engine = WorkspaceEngine::new_persistent(
            std::sync::Arc::new(CheckpointStore::new(temp.path().to_path_buf())),
            store.clone(),
            event_tx,
            session,
        );
        (temp, store, engine, attempt_ref)
    }

    fn reload(
        store: &LifecycleStore,
        engine: &WorkspaceEngine,
    ) -> crate::product::models::WorkspaceSessionRecord {
        store
            .get_workspace_session(&engine.session().session_id)
            .expect("reload durable session record")
    }

    fn teaching_window(
        record: &crate::product::models::WorkspaceSessionRecord,
    ) -> &ScGenerationTeachingWindow {
        record
            .sc_generation_teaching_windows
            .last()
            .expect("teaching window")
    }

    fn attempt_state(
        record: &crate::product::models::WorkspaceSessionRecord,
        attempt_no: u8,
    ) -> &crate::product::work_item_plan_policy::ScTeachingAttemptState {
        teaching_window(record)
            .attempts
            .iter()
            .find(|attempt| attempt.attempt_no == attempt_no)
            .expect("teaching attempt state")
    }

    fn placeholder_entries<'a>(
        record: &'a crate::product::models::WorkspaceSessionRecord,
        attempt_key: &str,
    ) -> Vec<&'a ProviderStartLedgerEntry> {
        record
            .provider_start_ledger
            .iter()
            .filter(|entry| entry.provider_attempt_key.as_deref() == Some(attempt_key))
            .collect()
    }

    /// spawn 身份已确认的 `ProviderSession` 替身（事件/命令通道就绪即可——
    /// 回填只透传会话，不进入事件驱动循环）。
    fn spawn_confirmed_session() -> ProviderSession {
        let (_events_tx, events_rx) = mpsc::channel(8);
        let (commands_tx, _commands_rx) = mpsc::channel(8);
        ProviderSession {
            events: events_rx,
            commands: commands_tx,
            native_session_id: None,
        }
    }

    /// ① claim 后驱动教学重驱 run：spawn 身份确认后的同步回填把 attempt
    /// 置为 ActualStarted，对应 ledger 占位条目原位补全（provider/started_at
    /// 齐备），同键仍只有一条、账本零新增。能测出的缺陷：回填漏接线（attempt
    /// 停在 Claimed）、另插第二条、provider/started_at 缺失（不构成
    /// `is_actual_launch_observation`）。
    #[test]
    fn teaching_attempt_backfilled_after_spawn_identity_confirmed() {
        let (_temp, store, mut engine, attempt_ref) = sc_teaching_launch_fixture();
        let before = reload(&store, &engine);
        let ledger_len_before = before.provider_start_ledger.len();
        let backfilled =
            engine.backfill_sc_teaching_launch(Some(&attempt_ref), Ok(spawn_confirmed_session()));
        assert!(backfilled.is_ok(), "spawn 已确认的回填必须透传会话");
        let after = reload(&store, &engine);
        assert_eq!(
            attempt_state(&after, 2).lifecycle,
            ScTeachingAttemptLifecycle::ActualStarted,
            "回填后 attempt 必须是 ActualStarted"
        );
        let placeholders = placeholder_entries(&after, &attempt_ref.attempt_key);
        assert_eq!(
            placeholders.len(),
            1,
            "同键只允许一条（原位补全不另插第二条）"
        );
        let entry = placeholders[0];
        assert!(entry.started, "占位必须被补全为 started=true");
        assert_eq!(
            entry.provider.as_deref(),
            Some("codex"),
            "provider 必须是会话 author_provider 名（serde snake_case 投影）"
        );
        assert!(
            entry.started_at.is_some(),
            "started_at 必须补全（RFC3339 UTC）"
        );
        assert!(
            entry.is_actual_launch_observation(),
            "补全后的条目必须构成真实启动观察"
        );
        assert_eq!(
            after.provider_start_ledger.len(),
            ledger_len_before,
            "回填不得新增账本条目"
        );
        assert_eq!(
            engine.session().single_candidate_phase,
            Some(crate::product::models::SingleCandidatePhase::Generate),
            "引擎内存镜像必须与 durable 同步（后续收口读同一事实）"
        );
    }

    /// ② 注入回填失败（把 durable 会话文件中的占位条目移除——observe 因
    /// 占位缺失 InvalidRecord，文件仍可写故 mark_unknown 迁移路径可达）⇒
    /// run 转既有失败收口（Err 稳定原因码），attempt 持久化为 LaunchUnknown
    /// （不是「未启动」），claim 与窗口证据保留、预算零消耗、无第二次启动。
    /// 能测出的缺陷：回填失败被吞（继续无账驱动）、attempt 被倒回未 claim、
    /// 失败被当成实际启动补全。
    #[test]
    fn spawn_backfill_failure_stops_process_as_unknown() {
        let (_temp, store, mut engine, attempt_ref) = sc_teaching_launch_fixture();
        let before = reload(&store, &engine);
        assert_eq!(before.provider_start_ledger.len(), 2, "原生成 + 教学占位");
        let session_id = engine.session().session_id.clone();
        let mut corrupted = before.clone();
        corrupted.provider_start_ledger.truncate(1);
        let session_path = store
            .app_paths()
            .issue_lifecycle_root(&corrupted.project_id, &corrupted.issue_id)
            .join("workspace-sessions")
            .join(format!("{session_id}.json"));
        write_json(&session_path, &corrupted).expect("inject missing placeholder");
        let stopped =
            engine.backfill_sc_teaching_launch(Some(&attempt_ref), Ok(spawn_confirmed_session()));
        let Err(error) = stopped else {
            panic!("回填失败必须转 Err 交既有失败收口停 run")
        };
        assert!(
            error.details.contains("sc_teaching_launch_backfill_failed"),
            "稳定原因码必须可判别：{}",
            error.details
        );
        let after = reload(&store, &engine);
        assert_eq!(
            attempt_state(&after, 2).lifecycle,
            ScTeachingAttemptLifecycle::LaunchUnknown,
            "回填失败的 attempt 必须是 LaunchUnknown（不是未启动、不是 ActualStarted）"
        );
        let window = teaching_window(&after);
        assert!(
            matches!(window.status, ScTeachingWindowStatus::Active),
            "claim 与窗口证据保留（不关窗、不删除）"
        );
        assert_eq!(window.attempts.len(), 2);
        assert_eq!(window.max_attempts, 2);
        assert_eq!(after.run_history.repairs_used, 0, "预算零消耗");
        assert_eq!(after.run_history.manual_repairs_used, 0, "预算零消耗");
        assert!(
            placeholder_entries(&after, &attempt_ref.attempt_key).is_empty(),
            "无第二次启动：不残留/不新增本 attempt 的补全条目"
        );
        assert_eq!(after.provider_start_ledger.len(), 1);
    }

    /// ③ 会话尾收口：attempt 处于 Claimed（尚未回填）时，SC run 臂统一出口
    /// （`release_after_session_tail` 注销的调用侧验证点）先把 attempt 持久化
    /// 为 LaunchUnknown；迁移不伪造启动证据（占位不补全）；重复收口幂等零
    /// 写盘。能测出的缺陷：会话尾漏迁移（Claimed 悬置）、迁移顺手倒填启动
    /// 证据、重复迁移二次写盘。
    #[test]
    fn session_tail_migration_to_unknown_when_claimed_unbackfilled() {
        let (_temp, store, mut engine, attempt_ref) = sc_teaching_launch_fixture();
        engine.migrate_claimed_sc_teaching_attempts_to_unknown();
        let after = reload(&store, &engine);
        assert_eq!(
            attempt_state(&after, 2).lifecycle,
            ScTeachingAttemptLifecycle::LaunchUnknown,
            "会话尾收口必须先持久化 LaunchUnknown 再注销"
        );
        let placeholders = placeholder_entries(&after, &attempt_ref.attempt_key);
        assert_eq!(placeholders.len(), 1);
        let entry = placeholders[0];
        assert!(
            !entry.started && entry.provider.is_none() && entry.started_at.is_none(),
            "迁移只改 attempt 状态，不伪造启动证据"
        );
        assert!(!entry.is_actual_launch_observation());
        let settled = reload(&store, &engine);
        engine.migrate_claimed_sc_teaching_attempts_to_unknown();
        let again = reload(&store, &engine);
        assert_eq!(
            settled.updated_at, again.updated_at,
            "重复会话尾收口必须幂等零写盘（Claimed→LaunchUnknown 只迁移一次）"
        );
        assert_eq!(
            attempt_state(&again, 2).lifecycle,
            ScTeachingAttemptLifecycle::LaunchUnknown
        );
    }

    /// ④ full fingerprint 漂移：治理层（gateway resume 指纹校验，
    /// `RESUME_FINGERPRINT_MISMATCH`）在 spawn 之前拒启——驱动入口拿到的是
    /// 启动 Err，接线面原样透传且零 durable 副作用：零 adapter start（无任何
    /// 实际启动观察）、attempt 保持 Claimed（确定未启动，design D2）、
    /// repairs_used/manual_repairs_used 与候选 refs 全不变。能测出的缺陷：
    /// 漂移场景被接线面转成启动成功、或消耗一次预算/多占一个物理键。
    #[test]
    fn fingerprint_drift_zero_adapter_start_and_budget_refs_unchanged() {
        let (_temp, store, mut engine, attempt_ref) = sc_teaching_launch_fixture();
        let before = reload(&store, &engine);
        let drift = ProviderAdapterError::provider_unavailable(
            "resume_fingerprint_mismatch: full fingerprint drift refused before spawn",
        );
        let refused = engine.backfill_sc_teaching_launch(Some(&attempt_ref), Err(drift));
        let error = match refused {
            Err(error) => error,
            Ok(_) => panic!("漂移拒绝必须原样上浮，不得转成启动成功"),
        };
        assert!(
            error.details.contains("resume_fingerprint_mismatch"),
            "透传不得改写治理层拒绝原因：{}",
            error.details
        );
        let after = reload(&store, &engine);
        assert!(
            after
                .provider_start_ledger
                .iter()
                .all(|entry| !entry.is_actual_launch_observation()),
            "零 adapter start：无任何实际启动观察"
        );
        assert_eq!(
            after.provider_start_ledger.len(),
            before.provider_start_ledger.len(),
            "漂移被治理层拦截，接线面零账本副作用"
        );
        assert_eq!(
            attempt_state(&after, 2).lifecycle,
            ScTeachingAttemptLifecycle::Claimed,
            "确定未启动保持 Claimed（design D2），不迁移不回填"
        );
        assert_eq!(
            after.run_history.repairs_used,
            before.run_history.repairs_used
        );
        assert_eq!(
            after.run_history.manual_repairs_used,
            before.run_history.manual_repairs_used
        );
        assert_eq!(
            after.work_item_plan_source_revision_ref,
            before.work_item_plan_source_revision_ref
        );
        assert_eq!(after.plan_candidate_ir_ref, before.plan_candidate_ir_ref);
        assert_eq!(after.mechanical_report_ref, before.mechanical_report_ref);
    }

    /// ⑤ 显式 fresh：fresh 的物理 run/native 是新证据、不是新 SC logical
    /// 授权——fresh 重入（重读 durable 后）再次 establish 命中同一窗口
    /// （Existing），generation_seq/approved/frozen_input_digest/attempts 全部
    /// 不变，无第二个 Active 窗口（未获取新 RepairOnce）；非教学 run（None）
    /// 的接线直通零副作用。能测出的缺陷：fresh 重建窗口/新批 RepairOnce、
    /// 冻结输入被改写、None 路径顺手触碰窗口。
    #[test]
    fn explicit_fresh_keeps_window_and_frozen_inputs() {
        let (_temp, store, mut engine, attempt_ref) = sc_teaching_launch_fixture();
        let before = reload(&store, &engine);
        let window_before = teaching_window(&before).clone();
        let fresh_read = store
            .get_workspace_session(&engine.session().session_id)
            .expect("fresh reload from durable");
        match store
            .establish_sc_teaching_window(&fresh_read, &sc_teaching_request())
            .expect("fresh re-entry establish must complete")
        {
            ScTeachingEstablishOutcome::Existing {
                record,
                window_generation_seq,
            } => {
                assert_eq!(window_generation_seq, 0);
                let window = record.sc_generation_teaching_windows.last().unwrap();
                assert_eq!(window.generation_seq, window_before.generation_seq);
                assert!(window.repair_once_approved);
                assert_eq!(
                    window.repair_once_approved, window_before.repair_once_approved,
                    "批准结论不得因 fresh 变化"
                );
                assert_eq!(
                    window.frozen_input_digest,
                    window_before.frozen_input_digest
                );
                assert_eq!(window.attempts, window_before.attempts);
                assert_eq!(
                    record.sc_generation_teaching_windows.len(),
                    1,
                    "fresh 不得获取第二个窗口（无新 RepairOnce）"
                );
            }
            ScTeachingEstablishOutcome::Established { .. } => {
                panic!("fresh 重入不得重建窗口")
            }
        }
        let passthrough = engine.backfill_sc_teaching_launch(None, Ok(spawn_confirmed_session()));
        assert!(passthrough.is_ok(), "非教学 run（None）零副作用直通");
        let after = reload(&store, &engine);
        assert_eq!(
            teaching_window(&after).clone(),
            window_before,
            "None 直通不得触碰窗口事实"
        );
        let _ = attempt_ref;
    }

    /// Task 3 评审 Minor-1 store 加固：已关窗口上的 observe（即使同证据的
    /// 幂等形态）必须明确拒绝，不静默补全也不静默成功。能测出的缺陷：
    /// 关窗后迟到 observe 在已关窗口上补全启动证据。
    #[test]
    fn observe_on_closed_window_is_refused_not_silently_completed() {
        let (_temp, store, mut engine, attempt_ref) = sc_teaching_launch_fixture();
        assert!(
            engine
                .backfill_sc_teaching_launch(Some(&attempt_ref), Ok(spawn_confirmed_session()))
                .is_ok()
        );
        let observed = reload(&store, &engine);
        let closed = store
            .close_sc_teaching_window(&observed, 0, 2, ScTeachingAttemptOutcome::Delivered)
            .expect("close delivered attempt");
        assert!(matches!(
            teaching_window(&closed).status,
            ScTeachingWindowStatus::Closed { .. }
        ));
        let completed = placeholder_entries(&closed, &attempt_ref.attempt_key)[0];
        let same_provider = completed.provider.clone().expect("completed provider");
        let same_started_at = completed.started_at.clone().expect("completed started_at");
        let again = store.observe_sc_teaching_provider_start(
            &closed,
            0,
            2,
            &same_provider,
            same_started_at,
        );
        assert!(
            again.is_err(),
            "已关窗口的 observe 必须明确拒绝（含同证据幂等形态），不得静默返回成功"
        );
    }

    /// Task 3 评审 Minor-1 store 加固：存在晚于当前 attempt 的在飞
    /// （Claimed/LaunchUnknown）后继 attempt 时不得关窗——防止关窗后后继
    /// 启动证据无处落账。能测出的缺陷：close(gen,1,…) 在 attempt2 仍 Claimed
    /// 时关窗。
    #[test]
    fn close_with_later_in_flight_attempt_is_refused() {
        let (_temp, store, engine, _attempt_ref) = sc_teaching_launch_fixture();
        let claimed = reload(&store, &engine);
        let result = store.close_sc_teaching_window(
            &claimed,
            0,
            1,
            ScTeachingAttemptOutcome::FailedTerminal,
        );
        assert!(
            result.is_err(),
            "attempt2 仍在飞（Claimed）时按 attempt1 关窗必须被拒绝"
        );
        let after = reload(&store, &engine);
        assert!(
            matches!(
                teaching_window(&after).status,
                ScTeachingWindowStatus::Active
            ),
            "被拒的关窗不得落盘"
        );
        assert_eq!(
            attempt_state(&after, 2).lifecycle,
            ScTeachingAttemptLifecycle::Claimed,
            "在飞后继 attempt 原样保留"
        );
    }
}
