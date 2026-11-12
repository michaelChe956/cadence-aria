// large_file_guard（>1200 行）拆分：cycle 锚定派生单测从 routing_scope.rs
// 移入本文件（conversational_gate/story_terminate.rs 先例），逻辑零改动。
use super::*;
use crate::product::work_item_plan_policy::ReviewCycleState;

#[test]
fn single_candidate_cycle_is_anchored_on_candidate_revision_identity() {
    let ir_ref = "project/project_0001/issue/issue_0001/plan/plan_0001/plan_candidate_ir/ir-340fe12ab34c5678";
    let cycle_key = "sc:candidate:340fe12ab34c5678".to_string();
    // ① 无 durable scope：按当前候选 source_revision 身份段开新 cycle（Initial）。
    assert_eq!(
        single_candidate_review_cycle(None, Some(ir_ref), &RunHistory::default()),
        Ok((cycle_key.clone(), ReviewPhase::Initial))
    );
    // ② 无 scope 且无候选 ref：fail-closed。
    assert!(single_candidate_review_cycle(None, None, &RunHistory::default()).is_err());

    let anchor_scope = ReviewInvocationScope::initial(ir_ref);
    let repaired_ref = "project/project_0001/issue/issue_0001/plan/plan_0001/plan_candidate_ir/ir-ffffffffffffffff";
    // ③ 初评已计数 → 自动返修后的复评仍是同一 cycle（Verification，锚定初评候选）。
    let anchored = RunHistory {
        review_cycles: std::collections::BTreeMap::from([(
            cycle_key.clone(),
            ReviewCycleState {
                initial_count: 1,
                ..ReviewCycleState::default()
            },
        )]),
        ..RunHistory::default()
    };
    assert_eq!(
        single_candidate_review_cycle(Some(&anchor_scope), Some(repaired_ref), &anchored),
        Ok((cycle_key.clone(), ReviewPhase::Verification))
    );
    // ④ 初评被中断重跑（计数 0；含 reviewer 节点轮换）→ 同 key、Initial。
    assert_eq!(
        single_candidate_review_cycle(Some(&anchor_scope), Some(ir_ref), &RunHistory::default()),
        Ok((cycle_key.clone(), ReviewPhase::Initial))
    );
    // ⑤ Verification scope 显式锚定 chain 头（cycle_anchor_revision_id）。
    let verification_scope = ReviewInvocationScope::verification(
        std::collections::BTreeSet::new(),
        repaired_ref,
        "report-001",
        Some(ir_ref.to_string()),
    );
    assert_eq!(
        single_candidate_review_cycle(
            Some(&verification_scope),
            Some(repaired_ref),
            &RunHistory::default()
        ),
        Ok((cycle_key.clone(), ReviewPhase::Verification))
    );
    // ⑥ 旧 durable Verification（anchor 缺省 None）fail-safe：退化用 repaired 身份段。
    let legacy_verification = ReviewInvocationScope::verification(
        std::collections::BTreeSet::new(),
        repaired_ref,
        "report-001",
        None,
    );
    assert_eq!(
        single_candidate_review_cycle(
            Some(&legacy_verification),
            Some(repaired_ref),
            &RunHistory::default()
        ),
        Ok((
            "sc:candidate:ffffffffffffffff".to_string(),
            ReviewPhase::Verification
        ))
    );
    // ⑦ 旧 review:<node> cycle 记录不迁移：新身份 key 计数为 0 → fresh Initial。
    let legacy_history = RunHistory {
        review_cycles: std::collections::BTreeMap::from([(
            "review:reviewer-node".to_string(),
            ReviewCycleState {
                initial_count: 1,
                verification_count: 1,
                ..ReviewCycleState::default()
            },
        )]),
        ..RunHistory::default()
    };
    assert_eq!(
        single_candidate_review_cycle(Some(&anchor_scope), Some(ir_ref), &legacy_history),
        Ok((cycle_key, ReviewPhase::Initial))
    );
}

#[test]
fn single_candidate_terminal_actions_close_the_review_cycle() {
    let snapshot = crate::product::work_item_plan_policy::HumanGateSnapshot {
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 0,
        trigger: crate::product::work_item_plan_policy::HumanReason::NativeHumanRequired,
        resumable: true,
        accepted_feedback_turns: None,
    };
    for action in [
        RoutingAction::ContinueToCompleted,
        RoutingAction::EnterHumanGate {
            snapshot: snapshot.clone(),
        },
        RoutingAction::StopNeedsHuman { snapshot },
        RoutingAction::AbortFatal {
            reason: FatalReason::ProtocolViolation,
            diagnostics: Vec::new(),
        },
    ] {
        assert!(
            closes_single_candidate_cycle(&action),
            "terminal action {action:?} must close the review cycle"
        );
    }
    assert!(!closes_single_candidate_cycle(
        &RoutingAction::TriggerAggregateRepair {
            findings: Vec::new(),
        }
    ));
}

#[test]
fn single_candidate_scope_rejects_initial_scope_during_verification() {
    let result = validate_single_candidate_scope(
        ReviewInvocationScope::initial("revision-001"),
        ReviewPhase::Verification,
    );
    assert!(matches!(
        result,
        Err(action)
            if matches!(
                *action,
                RoutingAction::AbortFatal {
                    reason: FatalReason::ProtocolViolation,
                    ..
                }
            )
    ));
}

#[test]
fn single_candidate_scope_preserves_existing_scope_on_protocol_fatal() {
    let durable_scope = ReviewInvocationScope::initial("durable-revision");
    let replacement_scope = ReviewInvocationScope::initial("replacement-revision");
    let action = RoutingAction::AbortFatal {
        reason: FatalReason::ProtocolViolation,
        diagnostics: Vec::new(),
    };
    assert_eq!(
        scope_for_action(Some(&durable_scope), &replacement_scope, &action),
        Some(durable_scope)
    );
    assert_eq!(scope_for_action(None, &replacement_scope, &action), None);
}

#[test]
fn single_candidate_scope_rejects_verification_scope_during_initial() {
    let result = validate_single_candidate_scope(
        ReviewInvocationScope::verification(
            std::collections::BTreeSet::new(),
            "revision-001",
            "report-001",
            None,
        ),
        ReviewPhase::Initial,
    );
    assert!(matches!(
        result,
        Err(action)
            if matches!(
                *action,
                RoutingAction::AbortFatal {
                    reason: FatalReason::ProtocolViolation,
                    ..
                }
            )
    ));
}
#[test]
fn single_candidate_cycle_none_scope_demotes_to_verification_when_initial_consumed() {
    // r37 现场(产品计费 bug 锚点):终态 action(EnterHumanGate)关闭
    // durable scope 后,同一候选(人工修订 AI 原样复写→源内容哈希不变→
    // 同 cycle key)再进评审——None 臂此前无条件判 Initial,对 initial=1
    // 的同 key cycle 再 +1 initial,merge_into 按 initial≤1 拒收,呈形
    // StateCorruption「run history delta cannot be merged within policy
    // limits」(r37 消耗账:transitions 2/12,manual 0/3,repairs 0/1,
    // cycles=[sc:candidate:af6aa517…[initial=1,verification=0,repairs=0]]
    // ——全在预算内却拒=本缺陷)。修:None 臂与 Some(Initial{R}) 臂同规
    // ——同 key cycle 初评已计数 → 判 Verification(复评),不再二次 initial。
    let ir_ref = "project/project_0001/issue/issue_0001/plan/plan_0001/plan_candidate_ir/ir-340fe12ab34c5678";
    let cycle_key = "sc:candidate:340fe12ab34c5678".to_string();
    let history = RunHistory {
        review_cycles: std::collections::BTreeMap::from([(
            cycle_key.clone(),
            ReviewCycleState {
                initial_count: 1,
                verification_count: 0,
                repairs_used: 0,
                ..ReviewCycleState::default()
            },
        )]),
        ..RunHistory::default()
    };
    let (key, phase) = single_candidate_review_cycle(None, Some(ir_ref), &history)
        .expect("same-candidate re-review must resolve, not fail");
    assert_eq!(key, cycle_key);
    assert_eq!(
        phase,
        ReviewPhase::Verification,
        "初评已计数的同 key cycle:同候选再进评审必须判 Verification(复评),不得二次 Initial"
    );
    // 对照:计数 0(真新候选)仍判 Initial。
    let fresh = single_candidate_review_cycle(None, Some(ir_ref), &RunHistory::default())
        .expect("fresh candidate resolves");
    assert_eq!(fresh.1, ReviewPhase::Initial);
}

#[test]
fn single_candidate_policy_invocation_rebuilds_verification_scope_from_durable_refs() {
    // r42 现场:同候选(人工修订原样复写→同 cycle key)再评审,r37 修复使
    // None 臂判 Verification;但 single_candidate_policy_invocation 的恢复
    // 臂只构造 Initial——Verification+scope=None 撞 AbortFatal「scope is
    // not durable」(fresh 终态已清 durable scope,cycle initial=1)。
    // 修:None 臂对 Verification 从 durable refs(plan_candidate_ir_ref/
    // mechanical_report_ref/cycle 指纹)重建 Verification scope,与
    // ensure_review_invocation_scope 同构。
    use crate::product::work_item_plan_policy::RunHistory;
    use crate::product::workspace_engine::types::WorkspaceSession;
    use std::collections::BTreeMap;

    let ir_ref = "project/project_0001/issue/issue_0001/plan/plan_0001/plan_candidate_ir/ir-340fe12ab34c5678";
    let cycle_key = "sc:candidate:340fe12ab34c5678".to_string();
    let (event_tx, _rx) = tokio::sync::mpsc::channel(8);
    let mut session = WorkspaceSession {
        session_id: "sess_scope_rebuild".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        entity_id: "work_item_plan_0001".to_string(),
        workspace_type: crate::product::models::WorkspaceType::WorkItemPlan,
        stage: WorkspaceStage::CrossReview,
        messages: Vec::new(),
        sc_generation_teaching_windows: Vec::new(),
        artifact: None,
        author_provider: crate::product::models::ProviderName::ClaudeCode,
        reviewer_provider: Some(crate::product::models::ProviderName::Codex),
        review_rounds: 1,
        permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        provisional_reviewer_provider: None,
        reviewer_enabled_at_start: None,
        superpowers_enabled: false,
        openspec_enabled: false,
        session_status: crate::product::models::WorkspaceSessionStatus::Running,
        flow_kind: crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate,
        run_policy: crate::product::work_item_plan_policy::RunPolicy::Interactive,
        run_history: RunHistory::default(),
        review_invocation_scope: None,
        human_gate_snapshot: None,
        repair_reservation: None,
        policy_diagnostics: Vec::new(),
        provider_start_ledger: Vec::new(),
        author_rewrite_budget: None,
        single_candidate_phase: None,
        work_item_plan_source_revision_ref: None,
        plan_candidate_ir_ref: None,
        mechanical_report_ref: None,
        publication_provenance_ref: None,
        approval_attempt_id: None,
        approved_at: None,
        compile_reservation: None,
        provider_conversations: Vec::new(),
        logical_launch_fingerprints: Vec::new(),
        superseded_native_sessions: Vec::new(),
        lc_resume_recovery: None,
        structural_governance_state: None,
        review_identity_id: String::new(),
        repository_path: None,
    };
    session.workspace_type = crate::product::models::WorkspaceType::WorkItemPlan;
    session.flow_kind =
        crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate;
    session.stage = crate::product::workspace_engine::WorkspaceStage::CrossReview;
    session.review_invocation_scope = None;
    session.plan_candidate_ir_ref = Some(ir_ref.to_string());
    session.mechanical_report_ref = Some("report-001".to_string());
    session.run_history = RunHistory {
        review_cycles: BTreeMap::from([(
            cycle_key.clone(),
            ReviewCycleState {
                initial_count: 1,
                verification_count: 0,
                repairs_used: 0,
                ..ReviewCycleState::default()
            },
        )]),
        ..RunHistory::default()
    };
    let checkpoint_root = tempfile::tempdir().expect("root");
    let engine = WorkspaceEngine::new(
        std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
            checkpoint_root.path().to_path_buf(),
        )),
        event_tx,
        session,
    );
    let scope = engine
        .policy_invocation(ReviewPhase::Verification, &cycle_key)
        .expect("同 key 已初评的 Verification 必须从 durable refs 重建,不得 AbortFatal");
    assert!(matches!(scope, ReviewInvocationScope::Verification { .. }));
}

/// Task 2 夹具：SingleCandidate 会话 + 落盘 store（预领 CAS 用）。
fn sc_preclaim_session_fixture() -> (
    tempfile::TempDir,
    crate::product::lifecycle_store::LifecycleStore,
    WorkspaceSessionRecord,
) {
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::lifecycle_store::{
        CreateWorkspaceSessionInput, LifecycleStore, WorkItemPlanSessionOptions,
    };
    use crate::product::models::{ProviderName, WorkspaceType};
    use crate::product::work_item_plan_policy::{RunPolicy, WorkItemPlanFlowKind};

    let temp = tempfile::tempdir().expect("root");
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path().join(".aria")));
    let session = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "work_item_plan_0001".to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Codex,
            reviewer_provider: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: Some(WorkItemPlanSessionOptions {
                flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                run_policy: RunPolicy::Interactive,
                rollout_snapshot: false,
            }),
        })
        .expect("create SC session");
    (temp, store, session)
}

/// Task 2（工作包 3.1/3.2/1.3/5.3）：平台自动重写扣减与 author 重跑预领
/// **同 session CAS** 落盘——预领键与扣减键同键，CAS 覆写后 durable 记录里
/// 同时含预领 ledger 条目与对应的一条预算扣减（同盘原子）。
#[test]
fn platform_preclaim_charges_author_budget_in_same_cas() {
    let (_temp, store, session) = sc_preclaim_session_fixture();
    let action = RoutingAction::TriggerAggregateRepair {
        findings: Vec::new(),
    };
    let (reservation, ledger, budget) =
        provider_start_ledger_for_action(&session, &action, None, 0);
    let reservation = reservation.expect("预领必须产出 repair reservation");
    let budget = budget.expect("预领必须产出 author 重写预算域状态");
    let preclaim_key = reservation.provider_start_idempotency_key.clone();
    assert_eq!(
        preclaim_key,
        format!("single_candidate_author:{}:0", session.id),
        "预领键形必须是 single_candidate_author:<session>:<n>"
    );
    assert_eq!(budget.auto_rewrite_charges.len(), 1, "预领一次只记一条扣减");
    assert_eq!(
        budget.auto_rewrite_charges[0].provider_start_idempotency_key, preclaim_key,
        "扣减键必须等于预领键（幂等键去重的唯一事实）"
    );
    assert_eq!(budget.total_auto_rewrite_charges_ever, 1);

    let stored = store
        .compare_and_save_policy_route(
            &session,
            PolicyRoutePersist {
                status: session.status.clone(),
                single_candidate_phase: session.single_candidate_phase.clone(),
                run_history: session.run_history.clone(),
                scope: session.review_invocation_scope.clone(),
                gate: session.human_gate_snapshot.clone(),
                diagnostics: session.policy_diagnostics.clone(),
                repair_reservation: Some(reservation),
                provider_start_ledger: ledger,
                author_rewrite_budget: Some(budget),
            },
        )
        .expect("policy route CAS 落盘");
    let persisted = stored
        .author_rewrite_budget
        .expect("预算域必须随同 CAS 落盘");
    assert_eq!(persisted.auto_rewrite_charges.len(), 1);
    assert_eq!(
        persisted.auto_rewrite_charges[0].provider_start_idempotency_key,
        preclaim_key
    );
    assert_eq!(persisted.total_auto_rewrite_charges_ever, 1);
    assert!(
        stored
            .provider_start_ledger
            .iter()
            .any(|entry| entry.provider_start_idempotency_key == preclaim_key),
        "预领 ledger 与预算扣减必须同盘（一次 CAS）"
    );
}

/// Task 2（工作包 5.3）：同键重放不得双扣——**两层防线分别钉住**：
/// ① 生产路径重放（CAS 冲突后用同一份未变的 expected 重算）⇒ 领域从同一 base
/// 重建，扣减条数与累计数都不得翻倍；
/// ② L1 纯函数层键命中（base 已含该键）⇒ 原样返回，产出与首次**逐字一致**；
/// ③ L2 durable 层：重放 CAS 被 expected 全量比对挡下（Conflict），
/// durable 记录仍只有一条扣减与一条预领 ledger 条目。
#[test]
fn replayed_preclaim_does_not_double_charge() {
    let (_temp, store, session) = sc_preclaim_session_fixture();
    let action = RoutingAction::TriggerAggregateRepair {
        findings: Vec::new(),
    };
    let (reservation, ledger, first) = provider_start_ledger_for_action(&session, &action, None, 0);
    let reservation = reservation.expect("预领必须产出 repair reservation");
    let first = first.expect("预领必须产出 author 重写预算域状态");
    let preclaim_key = reservation.provider_start_idempotency_key.clone();
    // 生产路径的重放（CAS 冲突后引擎用**同一份未变的 expected** 重算）：
    // 领域从同一 base 重建 ⇒ 仍只记一条扣减、累计不翻倍。
    let (retry_reservation, _retry_ledger, retry) =
        provider_start_ledger_for_action(&session, &action, None, 0);
    let retry_reservation = retry_reservation.expect("重算必须产出 repair reservation");
    let retry = retry.expect("重算必须产出 author 重写预算域状态");
    assert_eq!(
        retry_reservation.provider_start_idempotency_key, preclaim_key,
        "重算必须落在同一预领键上"
    );
    assert_eq!(retry.auto_rewrite_charges.len(), 1, "重算只允许一条扣减");
    assert_eq!(
        retry.total_auto_rewrite_charges_ever, 1,
        "重算不得让累计扣减翻倍"
    );
    // L1 纯函数层：base 已含该键 ⇒ 键命中 ⇒ 原样返回（**连 charged_at 都不变**）。
    let mut replay_base = session.clone();
    replay_base.author_rewrite_budget = Some(first.clone());
    let (replayed_reservation, _replayed_ledger, replayed) =
        provider_start_ledger_for_action(&replay_base, &action, None, 0);
    let replayed_reservation = replayed_reservation.expect("重放必须产出 repair reservation");
    let replayed = replayed.expect("重放必须产出 author 重写预算域状态");
    assert_eq!(
        replayed_reservation.provider_start_idempotency_key, preclaim_key,
        "重放必须落在同一预领键上"
    );
    assert_eq!(
        replayed.auto_rewrite_charges.len(),
        1,
        "同键重放只允许一条扣减"
    );
    assert_eq!(
        replayed.total_auto_rewrite_charges_ever, 1,
        "同键重放不得让累计扣减翻倍"
    );
    assert_eq!(
        replayed, first,
        "键命中的重放产出必须与首次逐字一致（幂等写）"
    );

    let _stored = store
        .compare_and_save_policy_route(
            &session,
            PolicyRoutePersist {
                status: session.status.clone(),
                single_candidate_phase: session.single_candidate_phase.clone(),
                run_history: session.run_history.clone(),
                scope: session.review_invocation_scope.clone(),
                gate: session.human_gate_snapshot.clone(),
                diagnostics: session.policy_diagnostics.clone(),
                repair_reservation: Some(reservation.clone()),
                provider_start_ledger: ledger.clone(),
                author_rewrite_budget: Some(first),
            },
        )
        .expect("首次预领 CAS 落盘");
    // 重放同一 CAS（stale expected）⇒ expected 比对冲突，绝不二次落盘。
    let replay_store = store
        .compare_and_save_policy_route(
            &session,
            PolicyRoutePersist {
                status: session.status.clone(),
                single_candidate_phase: session.single_candidate_phase.clone(),
                run_history: session.run_history.clone(),
                scope: session.review_invocation_scope.clone(),
                gate: session.human_gate_snapshot.clone(),
                diagnostics: session.policy_diagnostics.clone(),
                repair_reservation: Some(reservation),
                provider_start_ledger: ledger,
                author_rewrite_budget: Some(replayed),
            },
        )
        .expect_err("重放 CAS 必须被 expected 比对挡下");
    assert!(
        matches!(
            replay_store,
            crate::product::json_store::ProductStoreError::Conflict { .. }
        ),
        "重放必须是冲突而非静默二次落盘"
    );
    let reloaded = store
        .get_workspace_session(&session.id)
        .expect("reload session");
    let persisted = reloaded.author_rewrite_budget.expect("预算域已落盘");
    assert_eq!(
        persisted.auto_rewrite_charges.len(),
        1,
        "重放后 durable 仍只有一条扣减"
    );
    assert_eq!(persisted.total_auto_rewrite_charges_ever, 1);
    assert_eq!(
        reloaded
            .provider_start_ledger
            .iter()
            .filter(|entry| entry.provider_start_idempotency_key == preclaim_key)
            .count(),
        1,
        "重放后 durable ledger 的预领键仍只登记一次"
    );
}
