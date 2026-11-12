//! SC 生成教学重驱窗口 store CAS 用例（REQ-WSC-28B/28C store 面，Task 3）。
//! 夹具体例沿用 `workspace_single_candidate_tests.rs`：tempdir +
//! `create_workspace_session` + `reserve_single_candidate_provider_start`。

use tempfile::tempdir;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::{
    CreateWorkspaceSessionInput, LifecycleStore, PolicyRoutePersist, ScTeachingClaimOutcome,
    ScTeachingClaimRejection, ScTeachingEstablishOutcome, WorkItemPlanSessionOptions,
};
use crate::product::models::{
    ProviderName, SingleCandidatePhase, WorkspaceSessionRecord, WorkspaceSessionStatus,
    WorkspaceType,
};
use crate::product::planning_failure::{PlanningFailureKind, StructuralAuthorizationKind};
use crate::product::work_item_plan_policy::{
    ProviderStartLedgerEntry, RunPolicy, SC_TEACHING_APPROVAL_RULE_VERSION,
    ScTeachingApprovalRejection, ScTeachingAttemptLifecycle, ScTeachingAttemptOutcome,
    ScTeachingWindowRequest, ScTeachingWindowStatus, WorkItemPlanFlowKind,
    sc_teaching_frozen_input_digest,
};

fn digest64(seed: &str) -> String {
    format!("{seed:0<64}")
}

fn create_sc_session(
    store: &LifecycleStore,
    policy: RunPolicy,
    entity_id: &str,
) -> WorkspaceSessionRecord {
    store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: entity_id.to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Codex,
            reviewer_provider: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: Some(WorkItemPlanSessionOptions {
                flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                run_policy: policy,
                rollout_snapshot: true,
            }),
        })
        .expect("create candidate session")
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

fn reserve_original_launch(
    store: &LifecycleStore,
    session: &WorkspaceSessionRecord,
) -> WorkspaceSessionRecord {
    let key = format!("single_candidate_author:{}:0", session.id);
    let (record, did_start) = store
        .reserve_single_candidate_provider_start(session, &key)
        .expect("reserve original generation launch");
    assert!(did_start);
    record
}

fn establish_approved(
    store: &LifecycleStore,
    generate: &WorkspaceSessionRecord,
) -> WorkspaceSessionRecord {
    let request = sc_teaching_request();
    match store
        .establish_sc_teaching_window(generate, &request)
        .expect("establish teaching window")
    {
        ScTeachingEstablishOutcome::Established {
            record,
            approved,
            rejection,
        } => {
            assert!(approved);
            assert_eq!(rejection, None);
            record
        }
        ScTeachingEstablishOutcome::Existing { .. } => {
            panic!("fresh chain must establish a new window")
        }
    }
}

fn claim_attempt(
    store: &LifecycleStore,
    expected: &WorkspaceSessionRecord,
) -> WorkspaceSessionRecord {
    match store
        .claim_sc_teaching_attempt(expected, 0)
        .expect("claim teaching attempt")
    {
        ScTeachingClaimOutcome::Granted(record) => record,
        ScTeachingClaimOutcome::Replay(_) => panic!("first claim must grant attempt 2"),
        ScTeachingClaimOutcome::Denied(rejection) => panic!("unexpected denial: {rejection:?}"),
    }
}

fn observe_started(
    store: &LifecycleStore,
    expected: &WorkspaceSessionRecord,
) -> WorkspaceSessionRecord {
    store
        .observe_sc_teaching_provider_start(
            expected,
            0,
            2,
            "codex",
            "2026-11-13T02:00:00Z".to_string(),
        )
        .expect("observe teaching provider start")
}

fn session_json_path(
    store: &LifecycleStore,
    record: &WorkspaceSessionRecord,
) -> std::path::PathBuf {
    store
        .app_paths()
        .issue_lifecycle_root(&record.project_id, &record.issue_id)
        .join("workspace-sessions")
        .join(format!("{}.json", record.id))
}

fn route_persist(
    expected: &WorkspaceSessionRecord,
    status: WorkspaceSessionStatus,
    phase: Option<SingleCandidatePhase>,
    ledger: Vec<ProviderStartLedgerEntry>,
) -> PolicyRoutePersist {
    PolicyRoutePersist {
        status,
        single_candidate_phase: phase,
        run_history: expected.run_history.clone(),
        scope: expected.review_invocation_scope.clone(),
        gate: expected.human_gate_snapshot.clone(),
        diagnostics: expected.policy_diagnostics.clone(),
        repair_reservation: None,
        provider_start_ledger: ledger,
        author_rewrite_budget: expected.author_rewrite_budget.clone(),
    }
}

fn non_teaching_entry(session_id: &str, ordinal: usize) -> ProviderStartLedgerEntry {
    ProviderStartLedgerEntry {
        provider_start_idempotency_key: format!("single_candidate_author:{session_id}:{ordinal}"),
        started: true,
        provider: None,
        started_at: None,
        provider_attempt_key: None,
    }
}

fn teaching_placeholder<'a>(
    record: &'a WorkspaceSessionRecord,
    attempt_key: &str,
) -> &'a ProviderStartLedgerEntry {
    record
        .provider_start_ledger
        .iter()
        .find(|entry| entry.provider_attempt_key.as_deref() == Some(attempt_key))
        .expect("teaching placeholder ledger entry")
}

fn assert_rejected_establish(
    store: &LifecycleStore,
    expected: &WorkspaceSessionRecord,
    request: &ScTeachingWindowRequest,
    want: ScTeachingApprovalRejection,
) -> WorkspaceSessionRecord {
    match store
        .establish_sc_teaching_window(expected, request)
        .expect("establish must complete even when rejected")
    {
        ScTeachingEstablishOutcome::Established {
            record,
            approved,
            rejection,
        } => {
            assert!(!approved, "{want:?} must not be approved");
            assert_eq!(rejection, Some(want));
            let window = record
                .sc_generation_teaching_windows
                .last()
                .expect("established window");
            assert_eq!(window.max_attempts, 1, "{want:?} ⇒ max=1");
            assert!(!window.repair_once_approved);
            let variant_name = format!("{want:?}");
            assert!(
                record.policy_diagnostics.iter().any(|diagnostic| {
                    diagnostic.code == "sc_teaching_window_not_approved"
                        && diagnostic.message.contains(&variant_name)
                }),
                "拒绝必须同事务留痕 durable 诊断（含枚举名 {variant_name}）：{:?}",
                record.policy_diagnostics
            );
            record
        }
        ScTeachingEstablishOutcome::Existing { .. } => {
            panic!("no prior window: must establish (rejected), not Existing")
        }
    }
}

#[test]
fn establish_approves_with_all_four_facts_and_freezes_max_two() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let session = create_sc_session(&store, RunPolicy::Interactive, "entity_0001");
    let generate = reserve_original_launch(&store, &session);

    let request = sc_teaching_request();
    let record = match store
        .establish_sc_teaching_window(&generate, &request)
        .expect("establish teaching window")
    {
        ScTeachingEstablishOutcome::Established {
            record,
            approved,
            rejection,
        } => {
            assert!(approved);
            assert_eq!(rejection, None);
            record
        }
        ScTeachingEstablishOutcome::Existing { .. } => panic!("first establish must create"),
    };
    assert_eq!(record.sc_generation_teaching_windows.len(), 1);
    let window = &record.sc_generation_teaching_windows[0];
    assert_eq!(window.generation_seq, 0);
    assert_eq!(
        window.authorization_kind,
        StructuralAuthorizationKind::GenerationRun
    );
    assert_eq!(window.original_session_id, record.id);
    assert_eq!(window.original_run_phase, SingleCandidatePhase::Generate);
    assert_eq!(
        window.approval_rule_version,
        SC_TEACHING_APPROVAL_RULE_VERSION
    );
    assert_eq!(
        window.approval_checked_against,
        format!("single_candidate_author:{}:0", record.id)
    );
    assert_eq!(window.max_attempts, 2);
    assert!(window.repair_once_approved);
    assert_eq!(window.status, ScTeachingWindowStatus::Active);
    assert_eq!(window.frozen_execution_policy_digest, None);
    assert_eq!(window.chain_non_teaching_base, 0);
    assert_eq!(window.non_teaching_total_at_close, None);
    // 冻结摘要必须经 types.rs 的唯一入口取值（Task 2 评审待办②：防手拼摘要跨 Task 不一致）。
    assert_eq!(
        window.frozen_input_digest,
        sc_teaching_frozen_input_digest(&request)
    );
    // attempt1 = 原调用，建立时固化 Terminal{FailedTerminal}。
    assert_eq!(window.attempts.len(), 1);
    let attempt1 = &window.attempts[0];
    assert_eq!(attempt1.attempt_no, 1);
    assert_eq!(
        attempt1.attempt_key,
        format!("sc_generation:{}:0:attempt:1", record.id)
    );
    assert_eq!(
        attempt1.launch_ledger_key.as_deref(),
        Some(format!("single_candidate_author:{}:0", record.id).as_str())
    );
    assert_eq!(
        attempt1.lifecycle,
        ScTeachingAttemptLifecycle::Terminal {
            outcome: ScTeachingAttemptOutcome::FailedTerminal,
        }
    );
    // 建立不新增 ledger 条目，也不写诊断。
    assert_eq!(record.provider_start_ledger.len(), 1);
    assert!(record.policy_diagnostics.is_empty());
    // 落盘可重读且一致。
    let reloaded = store
        .get_workspace_session(&record.id)
        .expect("reload record");
    assert_eq!(
        reloaded.sc_generation_teaching_windows,
        record.sc_generation_teaching_windows
    );
}

#[test]
fn establish_rejects_each_missing_fact_with_max_one() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let request = sc_teaching_request();

    // ① phase 不为 Generate（保留前缀 launch 事实）⇒ NoProvableAuthorization。
    let session = create_sc_session(&store, RunPolicy::Interactive, "entity_0002a");
    let generate = reserve_original_launch(&store, &session);
    let not_generate = store
        .compare_and_save_policy_route(
            &generate,
            route_persist(
                &generate,
                WorkspaceSessionStatus::Running,
                Some(SingleCandidatePhase::Approval),
                generate.provider_start_ledger.clone(),
            ),
        )
        .expect("persist non-Generate phase");
    assert_rejected_establish(
        &store,
        &not_generate,
        &request,
        ScTeachingApprovalRejection::NoProvableAuthorization,
    );

    // ② provider 路径不可准入 ⇒ ProviderPathNotAdmissible。
    let session = create_sc_session(&store, RunPolicy::Interactive, "entity_0002b");
    let generate = reserve_original_launch(&store, &session);
    let mut inadmissible = request.clone();
    inadmissible.provider_path_admissible = false;
    assert_rejected_establish(
        &store,
        &generate,
        &inadmissible,
        ScTeachingApprovalRejection::ProviderPathNotAdmissible,
    );

    // ③ 前缀已有 2 条非教学条目（base=0 路径）⇒ NoRemainingPhysicalChance。
    let session = create_sc_session(&store, RunPolicy::Interactive, "entity_0002c");
    let generate = reserve_original_launch(&store, &session);
    let mut ledger = generate.provider_start_ledger.clone();
    ledger.push(non_teaching_entry(&generate.id, 1));
    let two_starts = store
        .compare_and_save_policy_route(
            &generate,
            route_persist(
                &generate,
                WorkspaceSessionStatus::Running,
                Some(SingleCandidatePhase::Generate),
                ledger,
            ),
        )
        .expect("persist two non-teaching starts");
    assert_rejected_establish(
        &store,
        &two_starts,
        &request,
        ScTeachingApprovalRejection::NoRemainingPhysicalChance,
    );

    // ④ failure_typed_kinds 为空 ⇒ IncompleteFailureFacts。
    let session = create_sc_session(&store, RunPolicy::Interactive, "entity_0002d");
    let generate = reserve_original_launch(&store, &session);
    let mut no_kinds = request.clone();
    no_kinds.failure_typed_kinds = Vec::new();
    assert_rejected_establish(
        &store,
        &generate,
        &no_kinds,
        ScTeachingApprovalRejection::IncompleteFailureFacts,
    );
}

#[test]
fn establish_is_idempotent_and_ab_relay_does_not_rebuild() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let session = create_sc_session(&store, RunPolicy::Interactive, "entity_0003");
    let generate = reserve_original_launch(&store, &session);
    let record = establish_approved(&store, &generate);

    // 同 expected 重放（模拟重连后原样再入）⇒ Existing，窗口字段零变化。
    let request = sc_teaching_request();
    match store
        .establish_sc_teaching_window(&record, &request)
        .expect("replay establish")
    {
        ScTeachingEstablishOutcome::Existing {
            record: replayed,
            window_generation_seq,
        } => {
            assert_eq!(window_generation_seq, 0);
            assert_eq!(replayed.sc_generation_teaching_windows.len(), 1);
            assert_eq!(
                replayed.sc_generation_teaching_windows[0],
                record.sc_generation_teaching_windows[0]
            );
        }
        ScTeachingEstablishOutcome::Established { .. } => {
            panic!("Active 窗口不得被重建（A/B 接力/重放共用同一窗口）")
        }
    }

    // A/B 接力：重读 record 后携带漂移过的请求材料再入 ⇒ 仍 Existing，不重建不批准新窗口。
    let reloaded = store
        .get_workspace_session(&record.id)
        .expect("reload record");
    let mut drifted = sc_teaching_request();
    drifted.context_digest = digest64("drift");
    match store
        .establish_sc_teaching_window(&reloaded, &drifted)
        .expect("relay establish with drifted facts")
    {
        ScTeachingEstablishOutcome::Existing {
            record: relayed,
            window_generation_seq,
        } => {
            assert_eq!(window_generation_seq, 0);
            assert_eq!(relayed.sc_generation_teaching_windows.len(), 1);
            assert_eq!(
                relayed.sc_generation_teaching_windows[0],
                record.sc_generation_teaching_windows[0]
            );
        }
        ScTeachingEstablishOutcome::Established { .. } => {
            panic!("A/B 接力不得用漂移材料重建窗口")
        }
    }
}

#[test]
fn approval_survives_reload_and_explicit_fresh() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let session = create_sc_session(&store, RunPolicy::Interactive, "entity_0004");
    let generate = reserve_original_launch(&store, &session);
    let record = establish_approved(&store, &generate);
    let window = &record.sc_generation_teaching_windows[0];

    // 模拟重连/恢复后的重读：approved 与 rule_version 不变。
    let reloaded = store
        .get_workspace_session(&record.id)
        .expect("reload record");
    let reloaded_window = &reloaded.sc_generation_teaching_windows[0];
    assert!(reloaded_window.repair_once_approved);
    assert_eq!(
        reloaded_window.approval_rule_version,
        SC_TEACHING_APPROVAL_RULE_VERSION
    );
    assert_eq!(
        reloaded_window.frozen_input_digest,
        window.frozen_input_digest
    );

    // 模拟显式 fresh：新 store 句柄（新进程视角）读取同一 durable 事实。
    let fresh_store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let fresh = fresh_store
        .get_workspace_session(&record.id)
        .expect("fresh store read");
    assert_eq!(
        fresh.sc_generation_teaching_windows,
        reloaded.sc_generation_teaching_windows
    );
    assert_eq!(
        fresh.sc_generation_teaching_windows[0].approval_rule_version,
        SC_TEACHING_APPROVAL_RULE_VERSION
    );
}

#[test]
fn concurrent_claim_same_key_single_winner() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let session = create_sc_session(&store, RunPolicy::Interactive, "entity_0005");
    let generate = reserve_original_launch(&store, &session);
    let established = establish_approved(&store, &generate);

    // 第一次 claim ⇒ Granted（push attempt2 + ledger 占位同事务）。
    let granted = claim_attempt(&store, &established);
    // 落盘即所见：重读与返回一致（原子性旁证）。
    let on_disk = store
        .get_workspace_session(&established.id)
        .expect("reload");
    assert_eq!(on_disk, granted);

    // 同一 expected 并发再 claim ⇒ 整记录 CAS 冲突（单一赢家）。
    assert!(matches!(
        store.claim_sc_teaching_attempt(&established, 0),
        Err(ProductStoreError::Conflict { .. })
    ));

    // 用第一次返回的 record 再 claim ⇒ Replay（同 attempt 幂等，不追加新 attempt/条目）。
    let outcome = store
        .claim_sc_teaching_attempt(&granted, 0)
        .expect("replay claim");
    assert_eq!(outcome, ScTeachingClaimOutcome::Replay(granted.clone()));

    // ledger 中该 attempt 的占位条目只有一条（attempt 身份键 = attempt_key）。
    let attempt2_key = format!("sc_generation:{}:0:attempt:2", established.id);
    assert_eq!(
        granted
            .provider_start_ledger
            .iter()
            .filter(|entry| entry.provider_attempt_key.as_deref() == Some(attempt2_key.as_str()))
            .count(),
        1
    );
}

#[test]
fn claim_rejected_over_limit_with_decidable_reason() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let session = create_sc_session(&store, RunPolicy::Interactive, "entity_0006");
    let generate = reserve_original_launch(&store, &session);
    let established = establish_approved(&store, &generate);
    let granted = claim_attempt(&store, &established);
    let observed = observe_started(&store, &granted);

    // attempts 已达 2（原调用 Terminal + 教学 ActualStarted）且末态非 Claimed/
    // LaunchUnknown ⇒ 再要新 attempt ⇒ 可判定拒绝 Exhausted，不写任何字段。
    let outcome = store
        .claim_sc_teaching_attempt(&observed, 0)
        .expect("claim over limit resolves determinably");
    assert_eq!(
        outcome,
        ScTeachingClaimOutcome::Denied(ScTeachingClaimRejection::Exhausted {
            max_attempts: 2,
            consumed: 2,
        })
    );
    // 无新增 ledger 条目、整记录零变化（Denied 不落盘）。
    let after = store.get_workspace_session(&observed.id).expect("reload");
    assert_eq!(after, observed);
}

#[test]
fn claim_pushes_started_false_placeholder_with_attempt_association() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let session = create_sc_session(&store, RunPolicy::Interactive, "entity_0007");
    let generate = reserve_original_launch(&store, &session);
    let established = establish_approved(&store, &generate);
    let granted = claim_attempt(&store, &established);

    let attempt2_key = format!("sc_generation:{}:0:attempt:2", established.id);
    let entry = teaching_placeholder(&granted, &attempt2_key);
    // 占位条目：started=false、provider/started_at 缺省、关联教学 attempt 键。
    assert!(!entry.started);
    assert_eq!(entry.provider, None);
    assert_eq!(entry.started_at, None);
    assert!(!entry.is_actual_launch_observation());
    // 键序单调：next_sc_author_launch_ordinal（:0 之后 ⇒ :1），与 C 记账解耦。
    assert_eq!(
        entry.provider_start_idempotency_key,
        format!("single_candidate_author:{}:1", established.id)
    );

    // 窗口侧 attempt2：Claimed 且 launch_ledger_key 指向该占位条目。
    let window = &granted.sc_generation_teaching_windows[0];
    assert_eq!(window.attempts.len(), 2);
    let attempt2 = &window.attempts[1];
    assert_eq!(attempt2.attempt_no, 2);
    assert_eq!(attempt2.attempt_key, attempt2_key);
    assert_eq!(attempt2.lifecycle, ScTeachingAttemptLifecycle::Claimed);
    assert_eq!(
        attempt2.launch_ledger_key.as_deref(),
        Some(entry.provider_start_idempotency_key.as_str())
    );
}

#[test]
fn observe_completes_in_place_idempotent_no_second_record() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let session = create_sc_session(&store, RunPolicy::Interactive, "entity_0008");
    let generate = reserve_original_launch(&store, &session);
    let established = establish_approved(&store, &generate);
    let granted = claim_attempt(&store, &established);
    let ledger_len = granted.provider_start_ledger.len();

    let observed = observe_started(&store, &granted);
    // 原位补全：不另插第二条（条目数不变），字段齐备构成实际启动观察。
    assert_eq!(observed.provider_start_ledger.len(), ledger_len);
    let attempt2_key = format!("sc_generation:{}:0:attempt:2", established.id);
    let entry = teaching_placeholder(&observed, &attempt2_key);
    assert!(entry.started);
    assert_eq!(entry.provider.as_deref(), Some("codex"));
    assert_eq!(entry.started_at.as_deref(), Some("2026-11-13T02:00:00Z"));
    assert!(entry.is_actual_launch_observation());
    assert_eq!(
        observed.sc_generation_teaching_windows[0].attempts[1].lifecycle,
        ScTeachingAttemptLifecycle::ActualStarted
    );

    // 同参再 observe ⇒ 幂等返回（整记录相等，条目数仍不变）。
    let replayed = store
        .observe_sc_teaching_provider_start(
            &observed,
            0,
            2,
            "codex",
            "2026-11-13T02:00:00Z".to_string(),
        )
        .expect("idempotent replay observe");
    assert_eq!(replayed, observed);
    assert_eq!(replayed.provider_start_ledger.len(), ledger_len);

    // 证据不同（provider 漂移）⇒ Conflict（不接受自报覆盖）。
    assert!(matches!(
        store.observe_sc_teaching_provider_start(
            &observed,
            0,
            2,
            "pi",
            "2026-11-13T09:00:00Z".to_string(),
        ),
        Err(ProductStoreError::Conflict { .. })
    ));
}

#[test]
fn observe_rejected_when_over_limit_or_not_claimed() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let session = create_sc_session(&store, RunPolicy::Interactive, "entity_0009");
    let generate = reserve_original_launch(&store, &session);
    let established = establish_approved(&store, &generate);

    // 未 claim 的 attempt_no（不存在）⇒ 可判定拒绝，不接受自报。
    assert!(matches!(
        store.observe_sc_teaching_provider_start(
            &established,
            0,
            2,
            "codex",
            "2026-11-13T02:00:00Z".to_string(),
        ),
        Err(ProductStoreError::InvalidRecord { .. })
    ));
    // 超出窗口上限的 attempt_no ⇒ 同样可判定拒绝。
    assert!(matches!(
        store.observe_sc_teaching_provider_start(
            &established,
            0,
            3,
            "codex",
            "2026-11-13T02:00:00Z".to_string(),
        ),
        Err(ProductStoreError::InvalidRecord { .. })
    ));
    // Terminal 态 attempt（attempt1 建立即固化）⇒ 不可回填。
    assert!(matches!(
        store.observe_sc_teaching_provider_start(
            &established,
            0,
            1,
            "codex",
            "2026-11-13T02:00:00Z".to_string(),
        ),
        Err(ProductStoreError::InvalidRecord { .. })
    ));

    // claim 后关闭（attempt2 → Terminal）再 observe ⇒ 仍拒绝（终态不可回填）。
    let granted = claim_attempt(&store, &established);
    let observed = observe_started(&store, &granted);
    let closed = store
        .close_sc_teaching_window(&observed, 0, 2, ScTeachingAttemptOutcome::FailedTerminal)
        .expect("close teaching window");
    assert!(matches!(
        store.observe_sc_teaching_provider_start(
            &closed,
            0,
            2,
            "codex",
            "2026-11-13T02:00:00Z".to_string(),
        ),
        Err(ProductStoreError::InvalidRecord { .. })
    ));
}

#[test]
fn old_started_placeholder_not_actual_launch() {
    // 旧格式条目：started=true 但无 provider/started_at ⇒ 不构成实际启动证据。
    let legacy = ProviderStartLedgerEntry {
        provider_start_idempotency_key: "single_candidate_author:s9:0".to_string(),
        started: true,
        provider: None,
        started_at: None,
        provider_attempt_key: None,
    };
    assert!(!legacy.is_actual_launch_observation());

    // 新占位（claim 产物 started=false）⇒ 同断言。
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let session = create_sc_session(&store, RunPolicy::Interactive, "entity_0010");
    let generate = reserve_original_launch(&store, &session);
    let established = establish_approved(&store, &generate);
    let granted = claim_attempt(&store, &established);
    let attempt2_key = format!("sc_generation:{}:0:attempt:2", established.id);
    let entry = teaching_placeholder(&granted, &attempt2_key);
    assert!(!entry.started);
    assert!(!entry.is_actual_launch_observation());
}

#[test]
fn historical_record_without_window_yields_no_authorization_and_no_backfill() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let session = create_sc_session(&store, RunPolicy::Interactive, "entity_0011");

    // 旧格式 JSON：文件不含 sc_generation_teaching_windows 键（空窗口本就不写键）。
    let path = session_json_path(&store, &session);
    let bytes_before = std::fs::read_to_string(&path).expect("read session json");
    assert!(
        !bytes_before.contains("sc_generation_teaching_windows"),
        "旧格式记录不得含窗口键：{bytes_before}"
    );

    // 读入 ⇒ windows 为空；claim ⇒ Denied(NoWindow)（零授权，不从缺省推断机会）。
    let record = store
        .get_workspace_session(&session.id)
        .expect("load record");
    assert!(record.sc_generation_teaching_windows.is_empty());
    assert_eq!(
        store
            .claim_sc_teaching_attempt(&record, 0)
            .expect("claim resolves"),
        ScTeachingClaimOutcome::Denied(ScTeachingClaimRejection::NoWindow)
    );

    // Denied 不写盘：文件字节原样（不倒填 grant / actual-start / 空窗口数组）。
    let bytes_after = std::fs::read_to_string(&path).expect("re-read session json");
    assert_eq!(bytes_after, bytes_before);

    // 重新序列化落盘 ⇒ 字段仍缺省（skip_serializing_if 空即不写键）。
    let value = serde_json::to_value(&record).expect("serialize record");
    assert!(value.get("sc_generation_teaching_windows").is_none());
}

#[test]
fn mark_unknown_transitions_claimed_only_and_is_idempotent() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let session = create_sc_session(&store, RunPolicy::Interactive, "entity_0014");
    let generate = reserve_original_launch(&store, &session);
    let established = establish_approved(&store, &generate);
    let granted = claim_attempt(&store, &established);

    // Claimed → LaunchUnknown（崩溃/回填失败收口）。
    let unknown = store
        .mark_sc_teaching_attempt_unknown(&granted, 0, 2)
        .expect("mark claimed attempt unknown");
    assert_eq!(
        unknown.sc_generation_teaching_windows[0].attempts[1].lifecycle,
        ScTeachingAttemptLifecycle::LaunchUnknown
    );
    // 已 LaunchUnknown ⇒ 幂等返回（整记录相等，不写盘）。
    let replayed = store
        .mark_sc_teaching_attempt_unknown(&unknown, 0, 2)
        .expect("idempotent mark unknown");
    assert_eq!(replayed, unknown);
    // ActualStarted / Terminal 态 ⇒ 可判定拒绝（不得回退到 Unknown）。
    let observed = observe_started(&store, &unknown);
    assert!(matches!(
        store.mark_sc_teaching_attempt_unknown(&observed, 0, 2),
        Err(ProductStoreError::InvalidRecord { .. })
    ));
    assert!(matches!(
        store.mark_sc_teaching_attempt_unknown(&observed, 0, 1),
        Err(ProductStoreError::InvalidRecord { .. })
    ));
}

#[test]
fn takeover_child_gets_no_window_parent_facts_retained() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let session = create_sc_session(&store, RunPolicy::AutoIfValid, "entity_0012");
    let generate = reserve_original_launch(&store, &session);
    let established = establish_approved(&store, &generate);
    let granted = claim_attempt(&store, &established);
    let observed = observe_started(&store, &granted);
    let closed = store
        .close_sc_teaching_window(&observed, 0, 2, ScTeachingAttemptOutcome::FailedTerminal)
        .expect("close teaching window");

    // 父会话转 StoppedNeedsHuman + 可恢复人工门（窗口字段经锁内 stored 保留）。
    let parent = store
        .compare_and_save_policy_route(
            &closed,
            PolicyRoutePersist {
                status: WorkspaceSessionStatus::StoppedNeedsHuman,
                single_candidate_phase: None,
                run_history: closed.run_history.clone(),
                scope: closed.review_invocation_scope.clone(),
                gate: Some(resumable_gate()),
                diagnostics: closed.policy_diagnostics.clone(),
                repair_reservation: None,
                provider_start_ledger: closed.provider_start_ledger.clone(),
                author_rewrite_budget: closed.author_rewrite_budget.clone(),
            },
        )
        .expect("persist stopped parent");
    assert_eq!(
        parent.sc_generation_teaching_windows,
        closed.sc_generation_teaching_windows
    );

    let child = store
        .takeover_stopped_needs_human(&parent.id)
        .expect("takeover stopped run");
    // 子会话不继承、不铸造教学窗口（不新增机会）。
    assert!(
        child.sc_generation_teaching_windows.is_empty(),
        "接管子会话不得获得教学窗口"
    );
    // 父记录窗口原样保留（供审计/接管引用）。
    let parent_after = store
        .get_workspace_session(&parent.id)
        .expect("reload parent");
    assert_eq!(parent_after.sc_generation_teaching_windows.len(), 1);
    assert_eq!(
        parent_after.sc_generation_teaching_windows[0],
        closed.sc_generation_teaching_windows[0]
    );
    assert!(matches!(
        parent_after.sc_generation_teaching_windows[0].status,
        ScTeachingWindowStatus::Closed { .. }
    ));
}

/// 可恢复人工门夹具（体例对照 workspace_engine/tests/recovery_task7.rs 的
/// `recovery_gate(true)`：一条 HumanRequired finding + resumable）。
fn resumable_gate() -> crate::product::work_item_plan_policy::HumanGateSnapshot {
    use crate::product::work_item_plan_policy as policy;
    let class = policy::FindingClass::HumanRequired;
    policy::HumanGateSnapshot {
        candidate_recovery: None,
        findings: vec![policy::ClassifiedFinding {
            class,
            fingerprint: policy::FindingFingerprint::for_finding(
                None,
                class,
                "需要人工决定范围",
                Some("scope"),
            ),
            category: Some(policy::ReviewFindingCategory::ScopeConflict),
            severity: "blocking".to_string(),
            message: "需要人工决定范围".to_string(),
            evidence: Some("冲突证据".to_string()),
            required_action: Some("确认范围".to_string()),
            contract_field: Some("scope".to_string()),
            identity_unstable: false,
        }],
        repeated_fingerprints: Vec::new(),
        attempts_used: 2,
        manual_repairs_remaining: 1,
        trigger: policy::HumanReason::NativeHumanRequired,
        resumable: true,
        accepted_feedback_turns: None,
    }
}

/// Task 2 评审待办①：判据③ 的 base > 0 减法分支（上一 Closed 窗口快照参与
/// 扣减）与快照腐坏态（缺写按 0 判宽 / 写高饱和到 0）的唯一钉名覆盖。
#[test]
fn establish_subtracts_previous_closed_window_snapshot_when_judging_chance() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let request = sc_teaching_request();

    // 第一条链：:0 非教学 + :1 教学（claim 占位）⇒ 关闭快照 Some(1)。
    let session = create_sc_session(&store, RunPolicy::Interactive, "entity_0013a");
    let generate = reserve_original_launch(&store, &session);
    let established = establish_approved(&store, &generate);
    let granted = claim_attempt(&store, &established);
    let observed = observe_started(&store, &granted);
    let closed = store
        .close_sc_teaching_window(&observed, 0, 2, ScTeachingAttemptOutcome::FailedTerminal)
        .expect("close first chain");
    assert_eq!(
        closed.sc_generation_teaching_windows[0].non_teaching_total_at_close,
        Some(1),
        "教学启动不占非教学计数：快照只含 :0"
    );

    // 第二条链（rearm）：追加一条新非教学启动 :2 ⇒ 总非教学 = 2。
    let mut ledger = closed.provider_start_ledger.clone();
    ledger.push(non_teaching_entry(&closed.id, 2));
    let rearmed = store
        .compare_and_save_policy_route(
            &closed,
            route_persist(
                &closed,
                WorkspaceSessionStatus::Running,
                Some(SingleCandidatePhase::Generate),
                ledger,
            ),
        )
        .expect("persist rearm chain ledger");

    // base > 0 分支：2 − 1 = 1 ≤ 1 ⇒ 批准（base=0 路径会误判 2 > 1 拒绝）。
    match store
        .establish_sc_teaching_window(&rearmed, &request)
        .expect("establish second chain")
    {
        ScTeachingEstablishOutcome::Established {
            record,
            approved,
            rejection,
        } => {
            assert!(approved);
            assert_eq!(rejection, None);
            assert_eq!(record.sc_generation_teaching_windows.len(), 2);
            let second = &record.sc_generation_teaching_windows[1];
            assert_eq!(second.generation_seq, 1);
            assert_eq!(second.chain_non_teaching_base, 1, "base = 上一 Closed 快照");
            assert_eq!(second.max_attempts, 2);
            assert_eq!(
                second.attempts[0].attempt_key,
                format!("sc_generation:{}:1:attempt:1", record.id)
            );
        }
        ScTeachingEstablishOutcome::Existing { .. } => panic!("Closed 之后必须能追加新窗口"),
    }

    // 对照（base = 0）：同规模 ledger、无上一 Closed 窗口 ⇒ 2 > 1 ⇒ 拒绝。
    let fresh = create_sc_session(&store, RunPolicy::Interactive, "entity_0013b");
    let fresh_generate = reserve_original_launch(&store, &fresh);
    let mut fresh_ledger = fresh_generate.provider_start_ledger.clone();
    fresh_ledger.push(non_teaching_entry(&fresh_generate.id, 1));
    let fresh_two = store
        .compare_and_save_policy_route(
            &fresh_generate,
            route_persist(
                &fresh_generate,
                WorkspaceSessionStatus::Running,
                Some(SingleCandidatePhase::Generate),
                fresh_ledger,
            ),
        )
        .expect("persist two non-teaching starts without prior window");
    assert_rejected_establish(
        &store,
        &fresh_two,
        &request,
        ScTeachingApprovalRejection::NoRemainingPhysicalChance,
    );

    // 腐坏态①：上一 Closed 窗口快照缺写（None ⇒ 按 0 参与扣减）⇒ 饱和判宽
    //（消费数按全量计 ⇒ 2 > 1 ⇒ 拒绝，fail-closed 保守方向）。
    let session = create_sc_session(&store, RunPolicy::Interactive, "entity_0013c");
    let generate = reserve_original_launch(&store, &session);
    let established = establish_approved(&store, &generate);
    let granted = claim_attempt(&store, &established);
    let observed = observe_started(&store, &granted);
    let closed = store
        .close_sc_teaching_window(&observed, 0, 2, ScTeachingAttemptOutcome::FailedTerminal)
        .expect("close chain for corrupt-snapshot cases");
    patch_first_window_snapshot(&session_json_path(&store, &closed), serde_json::Value::Null);
    let nulled = store
        .get_workspace_session(&closed.id)
        .expect("reload nulled");
    assert_eq!(
        nulled.sc_generation_teaching_windows[0].non_teaching_total_at_close,
        None
    );
    let mut nulled_ledger = nulled.provider_start_ledger.clone();
    nulled_ledger.push(non_teaching_entry(&nulled.id, 2));
    let nulled_two = store
        .compare_and_save_policy_route(
            &nulled,
            route_persist(
                &nulled,
                WorkspaceSessionStatus::Running,
                Some(SingleCandidatePhase::Generate),
                nulled_ledger,
            ),
        )
        .expect("persist rearm after null snapshot");
    assert_rejected_establish(
        &store,
        &nulled_two,
        &request,
        ScTeachingApprovalRejection::NoRemainingPhysicalChance,
    );

    // 腐坏态②：快照写高（超出当前总数）⇒ saturating 减到 0 ⇒ 批准。
    // 这是当前 saturating 语义的留痕断言（store 写出的快照单调有上界，仅
    // 手工腐坏能到达该分支；评审口径 = 至少断言当前行为）。
    let session = create_sc_session(&store, RunPolicy::Interactive, "entity_0013d");
    let generate = reserve_original_launch(&store, &session);
    let established = establish_approved(&store, &generate);
    let granted = claim_attempt(&store, &established);
    let observed = observe_started(&store, &granted);
    let closed = store
        .close_sc_teaching_window(&observed, 0, 2, ScTeachingAttemptOutcome::FailedTerminal)
        .expect("close chain for high-snapshot case");
    patch_first_window_snapshot(
        &session_json_path(&store, &closed),
        serde_json::Value::from(999_999usize),
    );
    let inflated = store
        .get_workspace_session(&closed.id)
        .expect("reload inflated");
    assert_eq!(
        inflated.sc_generation_teaching_windows[0].non_teaching_total_at_close,
        Some(999_999)
    );
    let mut inflated_ledger = inflated.provider_start_ledger.clone();
    inflated_ledger.push(non_teaching_entry(&inflated.id, 2));
    let inflated_two = store
        .compare_and_save_policy_route(
            &inflated,
            route_persist(
                &inflated,
                WorkspaceSessionStatus::Running,
                Some(SingleCandidatePhase::Generate),
                inflated_ledger,
            ),
        )
        .expect("persist rearm after inflated snapshot");
    match store
        .establish_sc_teaching_window(&inflated_two, &request)
        .expect("establish under inflated snapshot")
    {
        ScTeachingEstablishOutcome::Established { approved, .. } => {
            assert!(approved, "saturating 减到 0 ⇒ 当前行为为批准（留痕断言）");
        }
        ScTeachingEstablishOutcome::Existing { .. } => panic!("无 Active 窗口必须走建立分支"),
    }
}

/// 测试夹具手术：改写首窗口的 non_teaching_total_at_close（腐坏态构造）。
fn patch_first_window_snapshot(path: &std::path::Path, value: serde_json::Value) {
    let raw = std::fs::read_to_string(path).expect("read session json");
    let mut document: serde_json::Value = serde_json::from_str(&raw).expect("parse session json");
    document["sc_generation_teaching_windows"][0]["non_teaching_total_at_close"] = value;
    std::fs::write(
        path,
        serde_json::to_string(&document).expect("serialize session json"),
    )
    .expect("write session json");
}
