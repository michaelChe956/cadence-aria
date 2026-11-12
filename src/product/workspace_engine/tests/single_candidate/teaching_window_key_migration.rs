// Task 5（序 3，tasks 1.2 迁移面 / 8.3「记 C 后 A 路径键不变」，
// bound-sc-generation-teaching）：隐式键派生迁移与 policy CAS 镜像纪律用例。
// provider 启动键显式派生——新键序号 = next_sc_author_launch_ordinal（前缀
// 最大数字后缀 + 1），A 接力键由 repair_reservation 原键精确消费；记一条 C
// 教学条目不再把 A 路径的键序号顶偏（人门键/C 记账与序号解耦）。
// `PolicyRoutePersist.provider_start_ledger` 必须取自最近一次 store 返回/
// 重读的 record：旧 expected ⇒ Conflict，不得静默覆盖已回填条目。
mod teaching_window_key_migration {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::checkpoint_store::CheckpointStore;
    use crate::product::json_store::ProductStoreError;
    use crate::product::lifecycle_store::{
        CreateWorkspaceSessionInput, PolicyRoutePersist, ScTeachingClaimOutcome,
        ScTeachingEstablishOutcome, WorkItemPlanSessionOptions,
    };
    use crate::product::models::{ProviderName, WorkspaceSessionRecord, WorkspaceType};
    use crate::product::planning_failure::PlanningFailureKind;
    use crate::product::work_item_plan_policy::{
        ProviderStartLedgerEntry, RepairReservationState, ScTeachingAttemptLifecycle,
        ScTeachingWindowRequest,
    };
    use crate::product::workspace_engine::review::routing::routing_scope::provider_start_ledger_for_action;

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

    fn repair_action() -> RoutingAction {
        RoutingAction::TriggerAggregateRepair {
            findings: Vec::new(),
        }
    }

    fn create_sc_session(store: &LifecycleStore) -> WorkspaceSessionRecord {
        store
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "entity_task5".to_string(),
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
            .expect("create SC session")
    }

    fn session_json_path(
        store: &LifecycleStore,
        record: &WorkspaceSessionRecord,
    ) -> std::path::PathBuf {
        store
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id))
    }

    fn engine_for(store: &LifecycleStore, record: &WorkspaceSessionRecord) -> WorkspaceEngine {
        let (event_tx, _event_rx) = mpsc::channel(64);
        WorkspaceEngine::new_persistent(
            std::sync::Arc::new(CheckpointStore::new(store.app_paths().root().to_path_buf())),
            store.clone(),
            event_tx,
            WorkspaceSession::from_record(record.clone()),
        )
    }

    /// A 预领落盘：`provider_start_ledger_for_action` 走 `TriggerAggregateRepair`
    /// 的产物经 `compare_and_save_policy_route` 原子写入 durable 记录。
    fn persist_a_preclaim(
        store: &LifecycleStore,
        expected: &WorkspaceSessionRecord,
        reservation: crate::product::work_item_plan_policy::RepairReservation,
        ledger: Vec<ProviderStartLedgerEntry>,
        budget: crate::product::models::author_rewrite_budget::AuthorRewriteBudgetState,
    ) -> WorkspaceSessionRecord {
        store
            .compare_and_save_policy_route(
                expected,
                PolicyRoutePersist {
                    status: expected.status.clone(),
                    single_candidate_phase: expected.single_candidate_phase.clone(),
                    run_history: expected.run_history.clone(),
                    scope: expected.review_invocation_scope.clone(),
                    gate: expected.human_gate_snapshot.clone(),
                    diagnostics: expected.policy_diagnostics.clone(),
                    repair_reservation: Some(reservation),
                    provider_start_ledger: ledger,
                    author_rewrite_budget: Some(budget),
                },
            )
            .expect("persist A preclaim via policy route CAS")
    }

    /// 原 C 教学链种子：原生成 reserve（前缀 :0，phase → Generate）→ 窗口
    /// establish（approved，max=2）→ claim attempt2（真实 claim 落账：占位键
    /// = next_sc_author_launch_ordinal = :1，provider_attempt_key 关联 attempt）。
    fn teaching_claimed_record(
        store: &LifecycleStore,
        session: &WorkspaceSessionRecord,
    ) -> WorkspaceSessionRecord {
        let original_key = format!("single_candidate_author:{}:0", session.id);
        let (reserved, did_start) = store
            .reserve_single_candidate_provider_start(session, &original_key)
            .expect("reserve original generation launch");
        assert!(did_start);
        let established = match store
            .establish_sc_teaching_window(&reserved, &sc_teaching_request())
            .expect("establish teaching window")
        {
            ScTeachingEstablishOutcome::Established {
                record, approved, ..
            } => {
                assert!(approved, "四项判据齐备必须批准（本链非教学计数 = 1）");
                record
            }
            ScTeachingEstablishOutcome::Existing { .. } => {
                panic!("fresh chain must establish a new window")
            }
        };
        match store
            .claim_sc_teaching_attempt(&established, 0)
            .expect("claim teaching attempt")
        {
            ScTeachingClaimOutcome::Granted(record) => record,
            other => panic!("first claim must grant attempt 2, got {other:?}"),
        }
    }

    /// 8.3「记一次 C 后 A 路径键不变」：C 教学占位条目（claim 落账）之后——
    /// ① A 预领键 K 落在 C 之后的下一个显式序号（不与人门键/C 记账耦合）；
    /// ② K 仍按**原键**精确消费成功（不误接 C 占位、不让位），且 C 条目
    ///    原样保留；已消费（ProviderStarted）后引擎侧同键幂等让位（k3 单飞，
    ///    不派生新键、不追加 ledger）；
    /// ③ 此后新 A 预领键 ≠ 任何既有键且序号严格单调（= 1 + 最大数字后缀）。
    /// 既有缺陷形态：`ledger.len()` 派生把人门键计入 ⇒ 预领键跳号漂移；
    /// Generate `len-1` 启发式把 C 占位/人门键误当当前键 ⇒ 接力误让位或
    /// 追加第二把键。
    #[test]
    fn teaching_record_does_not_shift_a_path_keys() {
        let temp = tempfile::tempdir().expect("fixture tempdir");
        let store = LifecycleStore::new(ProductAppPaths::new(temp.path().to_path_buf()));
        let session = create_sc_session(&store);
        let claimed = teaching_claimed_record(&store, &session);

        // A 预领：种子链（[:0 原调用, :1 C 占位]）上的下一把显式序号 = :2。
        let (reservation, ledger, budget) =
            provider_start_ledger_for_action(&claimed, &repair_action(), None, 0);
        let reservation = reservation.expect("A 预领必须产出 repair reservation");
        let budget = budget.expect("A 预领必须产出 author 重写预算域");
        let preclaimed_key = reservation.provider_start_idempotency_key.clone();
        assert_eq!(
            preclaimed_key,
            format!("single_candidate_author:{}:2", session.id),
            "A 预领键 = next_sc_author_launch_ordinal（记 C 后接续，不顶偏）"
        );
        let preclaimed = persist_a_preclaim(&store, &claimed, reservation, ledger, budget);

        // 混排一条人门家族键（不同前缀）：len 派生与序号派生在此分叉。
        let teaching_key = format!("single_candidate_author:{}:1", session.id);
        let teaching_entry_before = preclaimed
            .provider_start_ledger
            .iter()
            .find(|entry| entry.provider_start_idempotency_key == teaching_key)
            .expect("C 教学占位条目必须在 ledger")
            .clone();
        let mut mixed = preclaimed.clone();
        mixed.provider_start_ledger.push(ProviderStartLedgerEntry {
            provider_start_idempotency_key: "human_gate:turn_0001:attempt:1".to_string(),
            started: false,
            provider: None,
            started_at: None,
            provider_attempt_key: None,
        });
        write_json(&session_json_path(&store, &mixed), &mixed).expect("rewrite mixed record");

        // ② A 预领键 K 仍按原键精确消费成功（Reserved → ProviderStarted）。
        let (consumed, should_start) = store
            .reserve_single_candidate_provider_start(&mixed, &preclaimed_key)
            .expect("A 接力必须按预领原键精确消费（不误接）");
        assert!(should_start, "Reserved 预领键消费必须放行真启动");
        assert_eq!(
            consumed
                .repair_reservation
                .as_ref()
                .map(|reservation| reservation.state),
            Some(RepairReservationState::ProviderStarted),
            "预领键消费必须推进 reservation 状态"
        );
        // C 条目原样保留（未被消费改写、未另插第二条）。
        let teaching_entries: Vec<_> = consumed
            .provider_start_ledger
            .iter()
            .filter(|entry| entry.provider_start_idempotency_key == teaching_key)
            .collect();
        assert_eq!(teaching_entries.len(), 1, "C 占位条目不得被消费路径复制");
        assert_eq!(teaching_entries[0], &teaching_entry_before);

        // ②（续）已消费后引擎侧同键幂等让位：不派生新键、不追加条目。
        let mut engine = engine_for(&store, &consumed);
        let engine_should_start = engine
            .reserve_single_candidate_author_start()
            .expect("engine 接力 reserve（同键幂等让位）");
        assert!(
            !engine_should_start,
            "已消费的预领键不得再次放行启动（k3 单飞）"
        );
        let after_relay = store
            .get_workspace_session(&session.id)
            .expect("relay durable record");
        assert_eq!(
            after_relay.provider_start_ledger.len(),
            mixed.provider_start_ledger.len(),
            "引擎侧让位不得追加 ledger 条目（按原键消费，不派生 :3+ 新键）"
        );

        // ③ 新 A 预领键 ≠ 任何既有键且序号严格单调（= 1 + 最大数字后缀 = 3，
        // 与人门键、C 记账解耦——len 派生在此给出跳号 :4）。
        let (reservation2, ledger2, budget2) =
            provider_start_ledger_for_action(&after_relay, &repair_action(), None, 0);
        let reservation2 = reservation2.expect("第二次 A 预领必须产出 reservation");
        let new_key = reservation2.provider_start_idempotency_key.clone();
        let existing_keys: Vec<&str> = after_relay
            .provider_start_ledger
            .iter()
            .map(|entry| entry.provider_start_idempotency_key.as_str())
            .collect();
        assert!(
            !existing_keys.contains(&new_key.as_str()),
            "新预领键不得与任何既有键冲突（含人门键/C 占位）"
        );
        let new_ordinal: usize = new_key
            .rsplit(':')
            .next()
            .and_then(|suffix| suffix.parse().ok())
            .expect("新预领键必须携带数字后缀");
        assert_eq!(
            new_ordinal, 3,
            "新键序号 = 1 + 最大数字后缀（严格单调、不跳号）"
        );
        let persisted2 = persist_a_preclaim(
            &store,
            &after_relay,
            reservation2,
            ledger2,
            budget2.expect("第二次 A 预领必须产出预算域"),
        );
        assert_eq!(persisted2.provider_start_ledger.len(), 5);
        assert!(
            persisted2.provider_start_ledger.iter().any(|entry| {
                entry.provider_start_idempotency_key == new_key
                    && entry.provider_attempt_key.is_none()
            }),
            "新预领条目必须随第二次 CAS 落账（键 = 显式序号）"
        );
    }

    /// ① 的正面钉子：ledger 已含 C 关联条目（与人门键混排）后，新业务启动
    /// （Prepare 相位，rearm 后形态）的新键后缀 = 1 + 最大数字后缀——不复用
    /// C 的键、不跳号、不撞人门键。既有缺陷：len 派生把人门键计入 ⇒ 跳号。
    #[test]
    fn reserve_author_start_uses_explicit_ordinal_after_teaching_entry() {
        let temp = tempfile::tempdir().expect("fixture tempdir");
        let store = LifecycleStore::new(ProductAppPaths::new(temp.path().to_path_buf()));
        let session = create_sc_session(&store);
        let claimed = teaching_claimed_record(&store, &session);

        // 人门键混排 + 显式重臂形态（Failed → rearm ⇒ Prepare/Open）。
        let mut seeded = claimed.clone();
        seeded.provider_start_ledger.push(ProviderStartLedgerEntry {
            provider_start_idempotency_key: "human_gate:turn_0002:attempt:1".to_string(),
            started: false,
            provider: None,
            started_at: None,
            provider_attempt_key: None,
        });
        seeded.single_candidate_phase = Some(SingleCandidatePhase::Prepare);
        seeded.status = WorkspaceSessionStatus::Open;
        write_json(&session_json_path(&store, &seeded), &seeded).expect("rewrite seeded record");

        let mut engine = engine_for(&store, &seeded);
        assert!(
            engine
                .reserve_single_candidate_author_start()
                .expect("新业务启动 reserve"),
            "新业务启动必须放行（新键）"
        );
        let after = store
            .get_workspace_session(&session.id)
            .expect("durable after reserve");

        // 新键 = 1 + 最大数字后缀（:1 为 C 条目 ⇒ :2）；len 派生会跳到 :3。
        let new_key = format!("single_candidate_author:{}:2", session.id);
        let new_entry = after
            .provider_start_ledger
            .iter()
            .find(|entry| entry.provider_start_idempotency_key == new_key)
            .expect("新业务启动键 = 显式序号 1+最大后缀（不跳号）");
        assert!(new_entry.provider_attempt_key.is_none());

        // 不复用：C 的 :1 仍只有一条且原样；人门键原样；无 :3+ 新条目。
        let teaching_key = format!("single_candidate_author:{}:1", session.id);
        assert_eq!(
            after
                .provider_start_ledger
                .iter()
                .filter(|entry| entry.provider_start_idempotency_key == teaching_key)
                .count(),
            1,
            "不得复用/复制 C 教学条目键"
        );
        let prefix = format!("single_candidate_author:{}:", session.id);
        let max_suffix = after
            .provider_start_ledger
            .iter()
            .filter_map(|entry| {
                entry
                    .provider_start_idempotency_key
                    .strip_prefix(&prefix)
                    .and_then(|suffix| suffix.parse::<usize>().ok())
            })
            .max()
            .expect("ledger 必须含前缀条目");
        assert_eq!(max_suffix, 2, "落账后最大后缀 = 新键（序号单调不跳）");
    }

    /// runtime 镜像纪律：`WorkspaceSession::from_record` 后教学窗口字段与
    /// durable record 相等（单源 = record；镜像口径同 provider_start_ledger）。
    #[test]
    fn runtime_mirror_carries_teaching_windows() {
        let temp = tempfile::tempdir().expect("fixture tempdir");
        let store = LifecycleStore::new(ProductAppPaths::new(temp.path().to_path_buf()));
        let session = create_sc_session(&store);
        let claimed = teaching_claimed_record(&store, &session);

        let mirrored = WorkspaceSession::from_record(claimed.clone());
        assert_eq!(
            serde_json::to_vec(&mirrored.sc_generation_teaching_windows).expect("serialize mirror"),
            serde_json::to_vec(&claimed.sc_generation_teaching_windows).expect("serialize record"),
            "镜像窗口字段必须与 record 逐字段相等"
        );
        let window = &mirrored.sc_generation_teaching_windows[0];
        assert_eq!(window.generation_seq, 0);
        assert_eq!(
            window.attempts.len(),
            2,
            "attempt1 固化 + claim 的 attempt2"
        );
        assert_eq!(
            window.attempts[1].launch_ledger_key.as_deref(),
            Some(format!("single_candidate_author:{}:1", session.id).as_str()),
            "镜像保留 claim 落账的 launch 键关联"
        );
        assert!(matches!(
            window.attempts[1].lifecycle,
            ScTeachingAttemptLifecycle::Claimed
        ));
    }

    /// ④ policy CAS 镜像纪律：observe 返回的 saved 是最近一次 store 事实——
    /// 以它为 expected 的 `PolicyRoutePersist` CAS 成功且回填条目完好；用
    /// observe 前的旧 expected ⇒ `Err(Conflict)` 且 stored 回填条目无损
    /// （expected 全量比对挡下陈旧覆盖，不静默丢条目）。
    #[test]
    fn policy_cas_after_observe_keeps_backfilled_entry() {
        let temp = tempfile::tempdir().expect("fixture tempdir");
        let store = LifecycleStore::new(ProductAppPaths::new(temp.path().to_path_buf()));
        let session = create_sc_session(&store);
        let claimed = teaching_claimed_record(&store, &session);
        let teaching_key = format!("single_candidate_author:{}:1", session.id);

        let observed = store
            .observe_sc_teaching_provider_start(
                &claimed,
                0,
                2,
                "claude_code",
                "2026-11-13T00:00:00Z".to_string(),
            )
            .expect("observe teaching provider start");
        let backfilled = observed
            .provider_start_ledger
            .iter()
            .find(|entry| entry.provider_start_idempotency_key == teaching_key)
            .expect("回填条目必须在 ledger");
        assert!(
            backfilled.started
                && backfilled.provider.as_deref() == Some("claude_code")
                && backfilled.started_at.is_some(),
            "observe 必须原位补全 provider/started_at"
        );

        // (a) 最近事实（saved）作 expected ⇒ CAS 成功且回填条目完好。
        let persist_from = |record: &WorkspaceSessionRecord| PolicyRoutePersist {
            status: record.status.clone(),
            single_candidate_phase: record.single_candidate_phase.clone(),
            run_history: record.run_history.clone(),
            scope: record.review_invocation_scope.clone(),
            gate: record.human_gate_snapshot.clone(),
            diagnostics: record.policy_diagnostics.clone(),
            repair_reservation: record.repair_reservation.clone(),
            provider_start_ledger: record.provider_start_ledger.clone(),
            author_rewrite_budget: record.author_rewrite_budget.clone(),
        };
        let persisted = store
            .compare_and_save_policy_route(&observed, persist_from(&observed))
            .expect("以最近 store 事实为 expected 的 CAS 必须成功");
        assert!(
            persisted.provider_start_ledger.iter().any(|entry| entry
                .provider_start_idempotency_key
                == teaching_key
                && entry.started
                && entry.provider.as_deref() == Some("claude_code")),
            "CAS 后回填条目必须完好（镜像纪律）"
        );

        // (b) observe 前旧 expected ⇒ Conflict，stored 回填条目无损。
        let stale = store.compare_and_save_policy_route(&claimed, persist_from(&claimed));
        assert!(
            matches!(stale, Err(ProductStoreError::Conflict { .. })),
            "旧 expected 的 CAS 必须被全量比对挡下（不得覆盖已更新 ledger）"
        );
        let after = store
            .get_workspace_session(&session.id)
            .expect("re-read after stale CAS");
        let kept = after
            .provider_start_ledger
            .iter()
            .find(|entry| entry.provider_start_idempotency_key == teaching_key)
            .expect("stored 回填条目必须在");
        assert!(
            kept.started && kept.provider.is_some() && kept.started_at.is_some(),
            "旧 expected 的 CAS 不得静默丢掉已回填条目"
        );
    }
}
