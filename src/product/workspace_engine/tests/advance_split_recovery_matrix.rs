//! REQ-MTG-05（WP5 恢复矩阵+增殖审计）：7 checkpoint × target 形态矩阵+审计
//! 追溯族（ter A-WP5 验收口径）。
//!
//! - 矩阵（R4 纪律）：2-target 七 checkpoint Crash 全量（每格：中断点→durable
//!   中间态→同套 attempt 集恢复→审计不重写不漂移→replay Replayed）+3-target
//!   抽样两中断点（WorktreeBound/UnitsMaterialized）。
//! - 半启动/断连（5.2）：`prepare_resumed_attempt_for_runner` per-attempt 独立
//!   等价（已物化补 head/未物化降级 WorktreePrepare，他 attempt 零牵连）；
//!   attach 重启门的集绑定放行由 `coding_ws_handler::socket::resumption` 测试族
//!   钉死。
//! - 审计族（5.1）：创建全字段留痕、检索 per-(plan,target) 消解、幂等首写定档、
//!   无编排（行为面：审计落盘后 attempts 停留初始二元组）。
//! - 单 target 零变化（回归锁）：`advance_handler` 既有矩阵（七 checkpoint）扩
//!   展钉死「单 target 不落任何 split-audit 记录」。

use super::advance_split_targets::{
    SPLIT_ISSUE_ID, SPLIT_PLAN_ID, SPLIT_PROJECT_ID, run_git, split_advance_fixture,
    split_advance_fixture_with_target_count, split_input,
};
use super::*;
use crate::product::advance_store::{AdvanceStatus, AdvanceStore};
use crate::product::coding_attempt_store::{
    CodingAttemptStore, CodingGroupInitializationPhase, SplitAuditRecord, SplitAuditTrigger,
    SplitAuditTriggerKind,
};
use crate::product::coding_workspace_engine::CodingWorkspaceEngine;
use crate::product::git_workspace_service::GitWorkspaceService;
use std::collections::{BTreeMap, BTreeSet};

fn split_audits(store: &CodingAttemptStore) -> Vec<SplitAuditRecord> {
    store
        .get_split_audits_for_plan(SPLIT_PROJECT_ID, SPLIT_ISSUE_ID, SPLIT_PLAN_ID)
        .unwrap()
}

fn audit_stamps(store: &CodingAttemptStore) -> BTreeMap<String, String> {
    split_audits(store)
        .into_iter()
        .map(|audit| (audit.attempt_id.clone(), audit.created_at.clone()))
        .collect()
}

fn journal_attempt_ids(store: &CodingAttemptStore) -> BTreeSet<String> {
    store
        .list_group_initialization_journals_for_plan(
            SPLIT_PROJECT_ID,
            SPLIT_ISSUE_ID,
            SPLIT_PLAN_ID,
        )
        .unwrap()
        .into_iter()
        .map(|journal| journal.attempt.id)
        .collect()
}

fn assert_all_journals_at(
    store: &CodingAttemptStore,
    phase: CodingGroupInitializationPhase,
) -> Vec<CodingGroupInitializationPhase> {
    let journals = store
        .list_group_initialization_journals_for_plan(
            SPLIT_PROJECT_ID,
            SPLIT_ISSUE_ID,
            SPLIT_PLAN_ID,
        )
        .unwrap();
    assert!(
        journals.iter().all(|journal| journal.phase == phase),
        "expected every split journal at {phase:?}, got {:?}",
        journals
            .iter()
            .map(|journal| journal.phase)
            .collect::<Vec<_>>()
    );
    journals.into_iter().map(|journal| journal.phase).collect()
}

fn attempt_count(store: &CodingAttemptStore) -> usize {
    store
        .list_attempts_for_work_item_group(SPLIT_PROJECT_ID, SPLIT_ISSUE_ID, SPLIT_PLAN_ID)
        .unwrap()
        .len()
}

fn coding_ws_engine(store: &CodingAttemptStore) -> CodingWorkspaceEngine {
    let (event_tx, _event_rx) = mpsc::channel(8);
    CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), event_tx)
}

/// 5.1 #1：分流创建留审计——每 target-attempt 一条，全字段（plan/target/attempt
/// 身份+bound_plan_revision_id+dependency_graph_revision_id+trigger.command_id）。
#[tokio::test]
async fn split_advance_records_proliferation_audit_for_every_target_attempt() {
    let fixture = split_advance_fixture(false).await;
    let mut engine = fixture.engine();

    let outcome = engine
        .handle_advance(split_input("cmd_audit_fields"))
        .await
        .expect("split advance completes");
    let AdvanceOutcome::Completed { record, .. } = outcome else {
        panic!("expected Completed");
    };

    let coding_store = fixture.coding_store();
    let authoritative = coding_store
        .resolve_authoritative_group_plan_binding(SPLIT_PROJECT_ID, SPLIT_ISSUE_ID, SPLIT_PLAN_ID)
        .unwrap();
    let audits = split_audits(&coding_store);
    assert_eq!(audits.len(), 2, "one audit record per target attempt");

    let mut seen_targets = BTreeSet::new();
    for audit in &audits {
        assert_eq!(audit.id, audit.attempt_id, "audit id mirrors attempt id");
        assert_eq!(audit.project_id, SPLIT_PROJECT_ID);
        assert_eq!(audit.issue_id, SPLIT_ISSUE_ID);
        assert_eq!(audit.plan_id, SPLIT_PLAN_ID);
        assert!(
            seen_targets.insert(audit.target_repository_id.clone()),
            "audit targets are unique per plan: {}",
            audit.target_repository_id
        );
        let target = crate::product::logical_codebase::LogicalRepositoryId(
            audit
                .target_repository_id
                .parse::<uuid::Uuid>()
                .expect("audit target repository id parses"),
        );
        let attempt = coding_store
            .get_attempt_for_work_item_group(
                SPLIT_PROJECT_ID,
                SPLIT_ISSUE_ID,
                SPLIT_PLAN_ID,
                Some(target),
            )
            .unwrap()
            .expect("audit attempt resolves per (plan,target)");
        assert_eq!(audit.attempt_id, attempt.id);
        assert_eq!(
            audit.bound_plan_revision_id, authoritative.plan_revision_id,
            "audit carries the authoritative binding revision"
        );
        assert_eq!(
            audit.dependency_graph_revision_id, authoritative.dependency_graph_revision_id,
            "audit carries the dependency graph revision"
        );
        assert_eq!(audit.trigger.kind, SplitAuditTriggerKind::Advance);
        assert_eq!(
            audit.trigger.command_id.as_deref(),
            Some("cmd_audit_fields"),
            "trigger command id is forwarded from AdvanceInput"
        );
        assert!(!audit.created_at.trim().is_empty());

        // 5.1 #4 行为面（审计不承载编排）：审计落盘后 attempt 仍停留初始二元组。
        assert_eq!(
            attempt.status,
            crate::product::coding_models::CodingAttemptStatus::Created
        );
        assert_eq!(
            attempt.stage,
            crate::product::coding_models::CodingExecutionStage::PrepareContext
        );
    }
    let mut audit_ids = audits
        .iter()
        .map(|audit| audit.attempt_id.clone())
        .collect::<Vec<_>>();
    audit_ids.sort_unstable();
    let mut record_ids = record
        .target_attempts
        .iter()
        .map(|binding| binding.attempt_id.clone())
        .collect::<Vec<_>>();
    record_ids.sort_unstable();
    assert_eq!(
        audit_ids, record_ids,
        "audit set mirrors the bound attempt set"
    );
}

/// 5.1 #3：检索 per-(plan,target) 消解——N targets N 条、每条 target 唯一；
/// 异 plan 记录不串扰。
#[tokio::test]
async fn split_audit_retrieval_resolves_per_plan_target_and_filters_foreign_plans() {
    let fixture = split_advance_fixture(false).await;
    let mut engine = fixture.engine();
    engine
        .handle_advance(split_input("cmd_audit_retrieval"))
        .await
        .expect("split advance completes");

    let coding_store = fixture.coding_store();
    // 异 plan 审计（同 issue 目录下的旁路记录）不得进入本 plan 检索结果。
    coding_store
        .record_split_audit(&SplitAuditRecord {
            id: "coding_attempt_foreign_plan".to_string(),
            project_id: SPLIT_PROJECT_ID.to_string(),
            issue_id: SPLIT_ISSUE_ID.to_string(),
            plan_id: "work_item_plan_9999".to_string(),
            target_repository_id: "logical_repo_foreign".to_string(),
            attempt_id: "coding_attempt_foreign_plan".to_string(),
            bound_plan_revision_id: "work_item_revision_foreign".to_string(),
            dependency_graph_revision_id: "dependency_graph_foreign".to_string(),
            trigger: SplitAuditTrigger {
                kind: SplitAuditTriggerKind::GroupCreation,
                command_id: None,
            },
            created_at: chrono::Utc::now().to_rfc3339(),
        })
        .unwrap();

    let audits = split_audits(&coding_store);
    assert_eq!(audits.len(), 2, "foreign-plan audit is filtered out");
    let targets: BTreeSet<_> = audits
        .iter()
        .map(|audit| audit.target_repository_id.clone())
        .collect();
    assert_eq!(targets.len(), 2, "no duplicate target keys");
    let foreign = coding_store
        .get_split_audits_for_plan(SPLIT_PROJECT_ID, SPLIT_ISSUE_ID, "work_item_plan_9999")
        .unwrap();
    assert_eq!(foreign.len(), 1);
    assert_eq!(
        foreign[0].trigger.kind,
        SplitAuditTriggerKind::GroupCreation
    );

    // 无审计的 plan 检索为空集（不误报、不 panic）。
    assert!(
        coding_store
            .get_split_audits_for_plan(SPLIT_PROJECT_ID, SPLIT_ISSUE_ID, "work_item_plan_8888")
            .unwrap()
            .is_empty()
    );
}

/// 5.1 #2（store 面）：幂等首写定档——身份一致命中不重写（created_at 与首写
/// command_id 保留）；身份冲突（同 attempt 异 target）fail-closed。
#[test]
fn split_audit_store_write_is_idempotent_first_writer_wins() {
    let root = tempfile::tempdir().unwrap();
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
    let record = |command_id: &str, created_at: &str| SplitAuditRecord {
        id: "coding_attempt_audit_idem".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_plan_0001".to_string(),
        plan_id: "work_item_plan_0001".to_string(),
        target_repository_id: "logical_repo_api".to_string(),
        attempt_id: "coding_attempt_audit_idem".to_string(),
        bound_plan_revision_id: "work_item_revision_0001".to_string(),
        dependency_graph_revision_id: "dependency_graph_0001".to_string(),
        trigger: SplitAuditTrigger {
            kind: SplitAuditTriggerKind::Advance,
            command_id: Some(command_id.to_string()),
        },
        created_at: created_at.to_string(),
    };

    store
        .record_split_audit(&record("command_first", "2026-09-19T00:00:00Z"))
        .unwrap();
    // 同身份异 command/created_at：首写定档，不重写。
    store
        .record_split_audit(&record("command_second", "2026-09-19T12:00:00Z"))
        .unwrap();
    let persisted = store
        .get_split_audits_for_plan("project_0001", "issue_plan_0001", "work_item_plan_0001")
        .unwrap();
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].created_at, "2026-09-19T00:00:00Z");
    assert_eq!(
        persisted[0].trigger.command_id.as_deref(),
        Some("command_first")
    );

    // 同 attempt 异 target：身份冲突 fail-closed。
    let mut conflicting = record("command_first", "2026-09-19T00:00:00Z");
    conflicting.target_repository_id = "logical_repo_web".to_string();
    assert!(
        store.record_split_audit(&conflicting).is_err(),
        "identity conflict must fail closed"
    );
}

/// k3 fix round 1（P2，审计与 attempt 同生命周期删除）：`delete_attempt` 连带
/// 清除 `split-audit/{attempt_id}.json`——中断后删除+同 command_id 重试不得在
/// 同 (plan,target) 留下双审计（per-(plan,target) 唯一不变式+检索无歧义，
/// REQ-MTG-05；增殖证据看在职 attempt）。
#[tokio::test]
async fn split_audit_is_deleted_with_attempt_and_retry_stays_unique_per_target() {
    let fixture = split_advance_fixture(false).await;
    let coding_store = fixture.coding_store();

    // WorktreeBound Crash：两 target-attempts+journals+audits 已 durable，
    // worktree 三件套已登记（删除面不受影响）。
    let request = split_input("cmd_audit_delete");
    let failpoint = register_advance_initialization_failpoint(
        &request,
        AdvanceInitializationFailpoint::WorktreeBound,
        AdvanceInitializationFailpointMode::Crash,
    );
    let mut engine = fixture.engine();
    let crashed_request = request.clone();
    let crashed = tokio::spawn(async move { engine.handle_advance(crashed_request).await });
    assert!(
        crashed.await.is_err(),
        "failpoint must crash the split advance"
    );
    drop(failpoint);

    let pre_delete = split_audits(&coding_store);
    assert_eq!(pre_delete.len(), 2);

    // 删除全部 target-attempts（含 per-target journal）——审计必须同生命周期清场。
    for audit in &pre_delete {
        coding_store
            .delete_attempt(SPLIT_PROJECT_ID, SPLIT_ISSUE_ID, &audit.attempt_id)
            .unwrap();
    }
    assert!(
        split_audits(&coding_store).is_empty(),
        "audit records must be deleted with their attempts (k3 P2)"
    );

    // 同 command_id 重试：per-target journal 全部重建（全新 attempt UUID），
    // 外层 journal 集绑定按既有语义对失配集 fail-closed——但审计面仍须
    // per-(plan,target) 唯一（旧记录已随删除清场，重试只落新身份一条）。
    let mut engine = fixture.engine();
    let _ = engine.handle_advance(request).await;
    let live_ids: BTreeSet<String> = coding_store
        .list_group_initialization_journals_for_plan(
            SPLIT_PROJECT_ID,
            SPLIT_ISSUE_ID,
            SPLIT_PLAN_ID,
        )
        .unwrap()
        .into_iter()
        .map(|journal| journal.attempt.id)
        .collect();
    assert_eq!(live_ids.len(), 2, "retry rebuilds a fresh attempt set");
    let audits_after = split_audits(&coding_store);
    assert_eq!(
        audits_after.len(),
        2,
        "exactly one audit per (plan,target) after the retry"
    );
    let mut targets: Vec<_> = audits_after
        .iter()
        .map(|audit| audit.target_repository_id.clone())
        .collect();
    targets.sort();
    targets.dedup();
    assert_eq!(targets.len(), 2, "no duplicate target keys");
    for audit in &audits_after {
        assert!(
            live_ids.contains(&audit.attempt_id),
            "audit mirrors the live attempt set, not deleted residue"
        );
    }
}

/// 5.2 矩阵主格：2-target × 七 checkpoint Crash 全量——每格断言 durable 中间态
/// （中断点语义）→ 同套 attempt 集恢复至 Completed → 审计不重写不漂移 →
/// replay Replayed 且不新增审计。
#[tokio::test]
async fn split_recovery_matrix_all_checkpoints_resume_same_attempt_set() {
    let checkpoints = [
        AdvanceInitializationFailpoint::RecordPersisted,
        AdvanceInitializationFailpoint::JournalPrepared,
        AdvanceInitializationFailpoint::GroupAttemptPersisted,
        AdvanceInitializationFailpoint::AttemptPersisted,
        AdvanceInitializationFailpoint::WorktreeBound,
        AdvanceInitializationFailpoint::PlanBindingSaved,
        AdvanceInitializationFailpoint::UnitsMaterialized,
    ];
    for checkpoint in checkpoints {
        let fixture = split_advance_fixture(false).await;
        let coding_store = fixture.coding_store();
        let advance_store = AdvanceStore::new(fixture.paths.clone());

        let command_id = format!("cmd_matrix_{checkpoint:?}");
        let request = split_input(&command_id);
        let failpoint = register_advance_initialization_failpoint(
            &request,
            checkpoint,
            AdvanceInitializationFailpointMode::Crash,
        );
        let mut engine = fixture.engine();
        let crashed_request = request.clone();
        let crashed = tokio::spawn(async move { engine.handle_advance(crashed_request).await });
        assert!(
            crashed.await.is_err(),
            "{checkpoint:?} must interrupt the split advance"
        );
        drop(failpoint);

        // durable 中间态：record 停留 Initializing（Crash=进程中断语义，无失败标记）。
        let crashed_record = advance_store
            .get_advance_by_command_id(SPLIT_PROJECT_ID, SPLIT_ISSUE_ID, &command_id)
            .unwrap()
            .expect("record durable before every checkpoint");
        assert_eq!(crashed_record.status, AdvanceStatus::Initializing);
        let attempts_before = attempt_count(&coding_store);
        let audits_before = split_audits(&coding_store);
        match checkpoint {
            AdvanceInitializationFailpoint::RecordPersisted => {
                assert!(journal_attempt_ids(&coding_store).is_empty());
                assert_eq!(attempts_before, 0);
                assert!(audits_before.is_empty());
            }
            AdvanceInitializationFailpoint::JournalPrepared => {
                assert_all_journals_at(&coding_store, CodingGroupInitializationPhase::Prepared);
                assert_eq!(attempts_before, 0);
                assert_eq!(audits_before.len(), 2);
            }
            AdvanceInitializationFailpoint::GroupAttemptPersisted => {
                assert_all_journals_at(&coding_store, CodingGroupInitializationPhase::Prepared);
                assert_eq!(attempts_before, 1);
                assert_eq!(audits_before.len(), 2);
            }
            AdvanceInitializationFailpoint::AttemptPersisted => {
                let journals = coding_store
                    .list_group_initialization_journals_for_plan(
                        SPLIT_PROJECT_ID,
                        SPLIT_ISSUE_ID,
                        SPLIT_PLAN_ID,
                    )
                    .unwrap();
                assert_eq!(journals.len(), 2);
                assert_eq!(
                    journals
                        .iter()
                        .filter(|journal| journal.phase
                            == CodingGroupInitializationPhase::AttemptPersisted)
                        .count(),
                    1
                );
                assert_eq!(
                    journals
                        .iter()
                        .filter(|journal| journal.phase == CodingGroupInitializationPhase::Prepared)
                        .count(),
                    1
                );
                assert_eq!(attempts_before, 1);
                assert_eq!(audits_before.len(), 2);
            }
            AdvanceInitializationFailpoint::WorktreeBound => {
                assert_all_journals_at(
                    &coding_store,
                    CodingGroupInitializationPhase::WorktreeBound,
                );
                assert_eq!(attempts_before, 2);
                assert_eq!(audits_before.len(), 2);
            }
            AdvanceInitializationFailpoint::PlanBindingSaved => {
                assert_all_journals_at(
                    &coding_store,
                    CodingGroupInitializationPhase::PlanBindingSaved,
                );
                assert_eq!(attempts_before, 2);
                assert_eq!(audits_before.len(), 2);
            }
            AdvanceInitializationFailpoint::UnitsMaterialized => {
                assert_all_journals_at(
                    &coding_store,
                    CodingGroupInitializationPhase::UnitsMaterialized,
                );
                assert_eq!(attempts_before, 2);
                assert_eq!(audits_before.len(), 2);
            }
        }
        let pre_crash_attempts = journal_attempt_ids(&coding_store);
        let pre_crash_stamps = audit_stamps(&coding_store);

        // 恢复（同 command_id 重放走 checkpoint 续走）：同套 attempt 集 → Completed。
        let mut engine = fixture.engine();
        let outcome = engine
            .handle_advance(request)
            .await
            .expect("recovery must complete the split advance");
        let AdvanceOutcome::Completed {
            record,
            target_attempts,
            ..
        } = outcome
        else {
            panic!("expected Completed after {checkpoint:?} recovery");
        };
        assert_eq!(record.status, AdvanceStatus::Ready);
        assert_eq!(target_attempts.len(), 2);

        assert_all_journals_at(&coding_store, CodingGroupInitializationPhase::Completed);
        let final_attempts = journal_attempt_ids(&coding_store);
        assert_eq!(
            final_attempts.len(),
            2,
            "no attempt proliferation on recovery"
        );
        if !pre_crash_attempts.is_empty() {
            assert_eq!(
                final_attempts, pre_crash_attempts,
                "{checkpoint:?} recovery must resume the same attempt set"
            );
        }
        let mut bound_ids = record
            .target_attempts
            .iter()
            .map(|binding| binding.attempt_id.clone())
            .collect::<BTreeSet<_>>();
        assert_eq!(bound_ids, final_attempts);

        // 审计：恢复不重写、不漂移；身份=最终 attempt 集。
        let audits_after = split_audits(&coding_store);
        assert_eq!(audits_after.len(), 2);
        for audit in &audits_after {
            assert!(final_attempts.contains(&audit.attempt_id));
            if let Some(stamp) = pre_crash_stamps.get(&audit.attempt_id) {
                assert_eq!(
                    &audit.created_at, stamp,
                    "{checkpoint:?} recovery must not rewrite the audit record"
                );
            }
            assert_eq!(audit.trigger.kind, SplitAuditTriggerKind::Advance);
            assert_eq!(
                audit.trigger.command_id.as_deref(),
                Some(command_id.as_str())
            );
        }

        // replay（新 command_id）：Replayed 且不新增审计。
        let replay = engine
            .handle_advance(split_input(&format!("{command_id}_replay")))
            .await
            .expect("completed advance replay");
        assert!(matches!(
            replay,
            AdvanceOutcome::Replayed {
                record: replayed
            } if replayed.status == AdvanceStatus::Ready
        ));
        assert_eq!(
            split_audits(&coding_store).len(),
            2,
            "replay writes no audit"
        );
        bound_ids.clear();
    }
}

/// 5.2 抽样（R4 组合爆炸缓解）：3-target × {WorktreeBound, UnitsMaterialized}。
#[tokio::test]
async fn split_recovery_matrix_three_targets_samples_two_checkpoints() {
    for checkpoint in [
        AdvanceInitializationFailpoint::WorktreeBound,
        AdvanceInitializationFailpoint::UnitsMaterialized,
    ] {
        let fixture = split_advance_fixture_with_target_count(3, false).await;
        let coding_store = fixture.coding_store();

        let command_id = format!("cmd_matrix3_{checkpoint:?}");
        let request = split_input(&command_id);
        let failpoint = register_advance_initialization_failpoint(
            &request,
            checkpoint,
            AdvanceInitializationFailpointMode::Crash,
        );
        let mut engine = fixture.engine();
        let crashed_request = request.clone();
        let crashed = tokio::spawn(async move { engine.handle_advance(crashed_request).await });
        let joined = crashed.await;
        match joined {
            Ok(Ok(outcome)) => panic!("3-target split completed without crash: {outcome:?}"),
            Ok(Err(error)) => panic!("3-target split errored without crash: {error}"),
            Err(_) => {}
        }
        drop(failpoint);

        let pre_crash_attempts = journal_attempt_ids(&coding_store);
        assert_eq!(pre_crash_attempts.len(), 3);

        let mut engine = fixture.engine();
        let outcome = engine
            .handle_advance(request)
            .await
            .expect("3-target recovery completes");
        let AdvanceOutcome::Completed { record, .. } = outcome else {
            panic!("expected Completed");
        };
        assert_all_journals_at(&coding_store, CodingGroupInitializationPhase::Completed);
        let final_attempts = journal_attempt_ids(&coding_store);
        assert_eq!(final_attempts.len(), 3);
        assert_eq!(
            final_attempts, pre_crash_attempts,
            "same attempt set across recovery"
        );
        let audits = split_audits(&coding_store);
        assert_eq!(audits.len(), 3, "one audit per target attempt");
        let audit_targets: BTreeSet<_> = audits
            .iter()
            .map(|audit| audit.target_repository_id.clone())
            .collect();
        assert_eq!(audit_targets.len(), 3);
        assert!(
            audit_targets.contains(&fixture.extra.expect("third target").0.to_string()),
            "the sampled third target carries its own audit record"
        );
        let bound: BTreeSet<_> = record
            .target_attempts
            .iter()
            .map(|binding| binding.attempt_id.clone())
            .collect();
        assert_eq!(bound, final_attempts);

        let replay = engine
            .handle_advance(split_input(&format!("{command_id}_replay")))
            .await
            .expect("replay");
        assert!(matches!(replay, AdvanceOutcome::Replayed { .. }));
    }
}

/// 5.2 半启动（Coding+已物化）：per-attempt 独立等价——被恢复 attempt 从自身
/// worktree 补 head；他 attempt（Created）零牵连。
#[tokio::test]
async fn split_semi_started_resume_backfills_head_per_attempt_only() {
    let fixture = split_advance_fixture(false).await;
    let mut engine = fixture.engine();
    let AdvanceOutcome::Completed { record, .. } = engine
        .handle_advance(split_input("cmd_semi_materialized"))
        .await
        .expect("split advance completes")
    else {
        panic!("expected Completed");
    };

    let coding_store = fixture.coding_store();
    let first_id = record.attempt_id.expect("record first attempt");
    let first = coding_store
        .get_attempt(SPLIT_PROJECT_ID, SPLIT_ISSUE_ID, &first_id)
        .unwrap();
    let other_id = record
        .target_attempts
        .iter()
        .map(|binding| binding.attempt_id.as_str())
        .find(|id| *id != first_id.as_str())
        .expect("other target attempt")
        .to_string();

    // 物化 first 的 worktree（split 只登记意图，物化归 runner 管道——此处按
    // execute_worktree_prepare 的产物形态直接建真实 git worktree）。
    let repository = crate::product::repository_store::RepositoryStore::new(fixture.paths.clone())
        .resolve_logical_repository_for_issue_codebase(SPLIT_PROJECT_ID, None, fixture.api)
        .map(|(_, _, repository)| repository)
        .unwrap();
    let worktree_path = first.worktree_path.clone().expect("split worktree path");
    run_git(
        &repository.path,
        &[
            "worktree",
            "add",
            "--quiet",
            worktree_path.to_str().unwrap(),
            "-b",
            &first.branch_name,
        ],
    );
    let expected_head = {
        let output = std::process::Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(&worktree_path)
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };

    // 半启动：Running+Coding+head 缺失。
    let mut semi_started = first.clone();
    semi_started.status = crate::product::coding_models::CodingAttemptStatus::Running;
    semi_started.stage = crate::product::coding_models::CodingExecutionStage::Coding;
    semi_started.head_commit = None;
    coding_store
        .write_coding_attempt_for_test(&semi_started)
        .unwrap();

    let ws_engine = coding_ws_engine(&coding_store);
    let prepared = ws_engine
        .prepare_resumed_attempt_for_runner(&semi_started)
        .await
        .unwrap();
    assert_eq!(
        prepared.stage,
        crate::product::coding_models::CodingExecutionStage::Coding
    );
    assert_eq!(
        prepared.head_commit.as_deref(),
        Some(expected_head.as_str()),
        "head is backfilled from this attempt's own worktree"
    );

    // 他 attempt 独立等价：仍 Created/PrepareContext，resume 为恒等。
    let other = coding_store
        .get_attempt(SPLIT_PROJECT_ID, SPLIT_ISSUE_ID, &other_id)
        .unwrap();
    assert_eq!(
        other.status,
        crate::product::coding_models::CodingAttemptStatus::Created
    );
    assert_eq!(
        other.stage,
        crate::product::coding_models::CodingExecutionStage::PrepareContext
    );
    let other_prepared = ws_engine
        .prepare_resumed_attempt_for_runner(&other)
        .await
        .unwrap();
    assert_eq!(
        other_prepared, other,
        "un-started sibling attempt is untouched"
    );
}

/// 5.2 半启动（Coding+未物化）：回落 WorktreePrepare 由 runner 管道重新物化；
/// 他 attempt 零牵连。
#[tokio::test]
async fn split_semi_started_unmaterialized_demotes_per_attempt_only() {
    let fixture = split_advance_fixture(false).await;
    let mut engine = fixture.engine();
    let AdvanceOutcome::Completed { record, .. } = engine
        .handle_advance(split_input("cmd_semi_unmaterialized"))
        .await
        .expect("split advance completes")
    else {
        panic!("expected Completed");
    };

    let coding_store = fixture.coding_store();
    let first_id = record.attempt_id.expect("record first attempt");
    let first = coding_store
        .get_attempt(SPLIT_PROJECT_ID, SPLIT_ISSUE_ID, &first_id)
        .unwrap();
    // split 登记的 worktree 路径尚未物化（目录不存在）。
    let worktree_path = first.worktree_path.clone().expect("split worktree path");
    assert!(!worktree_path.exists());

    let mut semi_started = first.clone();
    semi_started.status = crate::product::coding_models::CodingAttemptStatus::Running;
    semi_started.stage = crate::product::coding_models::CodingExecutionStage::Coding;
    semi_started.head_commit = None;
    coding_store
        .write_coding_attempt_for_test(&semi_started)
        .unwrap();

    let ws_engine = coding_ws_engine(&coding_store);
    let prepared = ws_engine
        .prepare_resumed_attempt_for_runner(&semi_started)
        .await
        .unwrap();
    assert_eq!(
        prepared.stage,
        crate::product::coding_models::CodingExecutionStage::WorktreePrepare
    );
    assert_eq!(
        prepared.status,
        crate::product::coding_models::CodingAttemptStatus::Running
    );
    assert!(!worktree_path.exists(), "demotion only stages the phase");

    let other_id = record
        .target_attempts
        .iter()
        .map(|binding| binding.attempt_id.as_str())
        .find(|id| *id != first_id.as_str())
        .expect("other target attempt")
        .to_string();
    let other = coding_store
        .get_attempt(SPLIT_PROJECT_ID, SPLIT_ISSUE_ID, &other_id)
        .unwrap();
    assert_eq!(
        other.status,
        crate::product::coding_models::CodingAttemptStatus::Created
    );
}
