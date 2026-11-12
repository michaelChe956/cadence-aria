use tempfile::tempdir;

use super::*;
use crate::product::app_paths::ProductAppPaths;
use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, WorkItemPlanSessionOptions};
use crate::product::models::{ProviderName, WorkspaceType};
use crate::product::work_item_plan_policy::{ReviewInvocationScope, RunHistory, RunPolicy};

#[test]
fn single_candidate_provider_start_reservation_is_one_shot_and_durable() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let session = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "entity_0001".to_string(),
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
    let key = "single_candidate_author:session:0";
    let (started, did_start) = store
        .reserve_single_candidate_provider_start(&session, key)
        .expect("reserve provider start");
    assert!(did_start);
    assert_eq!(
        started.single_candidate_phase,
        Some(SingleCandidatePhase::Generate)
    );
    assert_eq!(started.provider_start_ledger.len(), 1);
    let (replayed, did_replay) = store
        .reserve_single_candidate_provider_start(&started, key)
        .expect("replay provider start");
    assert!(!did_replay);
    assert_eq!(replayed, started);
}

#[test]
fn preclaimed_single_candidate_repair_key_starts_provider_once() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let session = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "entity_0001".to_string(),
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
    let first_key = format!("single_candidate_author:{}:0", session.id);
    let (started, did_start) = store
        .reserve_single_candidate_provider_start(&session, &first_key)
        .expect("reserve first provider start");
    assert!(did_start);
    let repair_key = format!("single_candidate_author:{}:1", session.id);
    let reservation = crate::product::work_item_plan_policy::RepairReservation {
        token: format!("single_candidate_author_repair:{}:1", session.id),
        owner_session_id: session.id.clone(),
        owner_run_id: "reviewer-run-1".to_string(),
        provider_start_idempotency_key: repair_key.clone(),
        state: crate::product::work_item_plan_policy::RepairReservationState::Reserved,
        commit_id: None,
    };
    let mut ledger = started.provider_start_ledger.clone();
    ledger.push(
        crate::product::work_item_plan_policy::ProviderStartLedgerEntry {
            provider_start_idempotency_key: repair_key.clone(),
            started: true,
            provider: None,
            started_at: None,
        },
    );
    let preclaimed = store
        .compare_and_save_policy_route(
            &started,
            super::PolicyRoutePersist {
                status: crate::product::models::WorkspaceSessionStatus::Running,
                single_candidate_phase: Some(SingleCandidatePhase::Generate),
                run_history: started.run_history.clone(),
                scope: started.review_invocation_scope.clone(),
                gate: started.human_gate_snapshot.clone(),
                diagnostics: started.policy_diagnostics.clone(),
                repair_reservation: Some(reservation),
                provider_start_ledger: ledger,
                author_rewrite_budget: started.author_rewrite_budget.clone(),
            },
        )
        .expect("preclaim repair provider key with the route");

    let (provider_started, did_start) = store
        .reserve_single_candidate_provider_start(&preclaimed, &repair_key)
        .expect("consume the preclaimed repair provider key");
    assert!(
        did_start,
        "a freshly preclaimed repair key must start its provider"
    );
    assert_eq!(
        provider_started.provider_start_ledger,
        preclaimed.provider_start_ledger
    );
    assert_eq!(
        provider_started
            .repair_reservation
            .as_ref()
            .map(|reservation| reservation.state),
        Some(crate::product::work_item_plan_policy::RepairReservationState::ProviderStarted)
    );

    let (_replayed, replayed) = store
        .reserve_single_candidate_provider_start(&provider_started, &repair_key)
        .expect("replay repair provider start");
    assert!(!replayed, "the consumed repair key remains one-shot");
}

#[test]
fn explicit_start_generation_rearms_failed_single_candidate_but_rejects_completed() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let session = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "entity_0001".to_string(),
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
    let failed = store
        .compare_and_save_single_candidate_phase(
            &session,
            SingleCandidatePhase::Failed,
            crate::product::models::WorkspaceSessionStatus::Failed,
        )
        .expect("persist failed session");
    let rearmed = store
        .rearm_failed_single_candidate_for_start_generation(&failed)
        .expect("explicit start generation must rearm failed session");
    assert_eq!(
        rearmed.single_candidate_phase,
        Some(SingleCandidatePhase::Prepare)
    );
    assert_eq!(
        rearmed.status,
        crate::product::models::WorkspaceSessionStatus::Open
    );

    let completed = store
        .compare_and_save_single_candidate_phase(
            &rearmed,
            SingleCandidatePhase::Completed,
            crate::product::models::WorkspaceSessionStatus::Confirmed,
        )
        .expect("persist completed session");
    assert!(matches!(
        store.rearm_failed_single_candidate_for_start_generation(&completed),
        Err(ProductStoreError::Conflict { .. })
    ));
}

#[test]
fn single_candidate_evaluation_persists_report_without_upgrading_invocation_scope() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let mut session = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "entity_0001".to_string(),
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
        .expect("create single candidate session");
    session.single_candidate_phase = Some(SingleCandidatePhase::Evaluate);
    session.work_item_plan_source_revision_ref = Some(
        "project/project_0001/issue/issue_0001/plan/entity_0001/source_revision/source-001"
            .to_string(),
    );
    session.plan_candidate_ir_ref = Some(
        "project/project_0001/issue/issue_0001/plan/entity_0001/plan_candidate_ir/ir-001"
            .to_string(),
    );
    let initial_scope = ReviewInvocationScope::initial("review:node-a");
    session.review_invocation_scope = Some(initial_scope.clone());
    session.run_history = RunHistory {
        repairs_used: 1,
        ..RunHistory::default()
    };
    write_json(
        &store
            .workspace_sessions_root("project_0001", "issue_0001")
            .join(format!("{}.json", session.id)),
        &session,
    )
    .expect("seed evaluate session");

    let report_ref =
        "project/project_0001/issue/issue_0001/plan/entity_0001/mechanical_report/report-001";
    let expected = store
        .get_workspace_session(&session.id)
        .expect("load evaluate session");
    let saved = store
        .compare_and_save_single_candidate_evaluation(&expected, report_ref)
        .expect("persist mechanical report");

    assert_eq!(saved.mechanical_report_ref.as_deref(), Some(report_ref));
    assert_eq!(saved.review_invocation_scope, Some(initial_scope));
}

#[test]
fn evaluation_cas_accepts_report_ref_rotation_for_equivalent_content() {
    // P2 回归（session_0003 重放面）：report_id 内容寻址化后，持有旧格式
    // mechanical_report_ref 的会话在重启重放 author Evaluate（既有支撑
    // 路径）时，同内容的新 report 轮换 id → CAS ref 不等。等价内容（剔除
    // id/content_hash 的全量体摘要相等，即 content_addressed_id 相等）
    // 必须接受 ref 轮换保持重入幂等；内容真变或任一 ref 悬空仍 Conflict。
    use crate::product::models::{WorkItemSplitFinding, WorkItemSplitFindingSeverity};
    use crate::product::planning_failure::PlanningFailureKind;
    use crate::product::work_item_plan_compiler::{
        PlanCandidateIr, PlanCandidateMechanicalReport, WORK_ITEM_PLAN_COMPILER_VERSION,
    };
    use crate::product::work_item_plan_source_store::{
        PlanCandidateIrRecord, PlanCandidateMechanicalReportRecord, SourceRevisionRecord,
        WorkItemPlanSourceStore,
    };
    use sha2::{Digest, Sha256};

    fn e2e_finding(kind: Option<PlanningFailureKind>) -> WorkItemSplitFinding {
        WorkItemSplitFinding {
            severity: WorkItemSplitFindingSeverity::Warning,
            code: "integration_or_e2e_skipped_risk".to_string(),
            message: "integration or e2e verification skipped risk".to_string(),
            work_item_ids: Vec::new(),
            kind,
        }
    }

    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let mut session = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "entity_0001".to_string(),
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
        .expect("create single candidate session");
    session.single_candidate_phase = Some(SingleCandidatePhase::Evaluate);

    // 落盘三件套：source/ir + 旧格式 id 的存量报告（kind=None，模拟
    // 内容寻址化之前落盘的历史记录）。
    let source_store = WorkItemPlanSourceStore::new(store.app_paths());
    let source_text = "# Work Item Plan\n";
    let source_hash = hex::encode(Sha256::digest(source_text.as_bytes()));
    let mut source = SourceRevisionRecord {
        id: "source-001".to_string(),
        source: source_text.to_string(),
        source_revision_hash: source_hash.clone(),
        content_hash: String::new(),
    };
    source.content_hash = source.content_hash().unwrap();
    let source_ref = source_store
        .put_source_revision("project_0001", "issue_0001", "entity_0001", &source)
        .unwrap();
    let mut ir = PlanCandidateIrRecord {
        id: "ir-001".to_string(),
        source_revision_id: source.id.clone(),
        ir: PlanCandidateIr {
            source_revision_hash: source_hash.clone(),
            compiler_version: WORK_ITEM_PLAN_COMPILER_VERSION.to_string(),
            items: Vec::new(),
        },
        content_hash: String::new(),
    };
    ir.content_hash = ir.content_hash().unwrap();
    let ir_ref = source_store
        .put_plan_candidate_ir("project_0001", "issue_0001", "entity_0001", &ir)
        .unwrap();
    session.work_item_plan_source_revision_ref = Some(source_ref);
    session.plan_candidate_ir_ref = Some(ir_ref);

    let report_body = |findings| PlanCandidateMechanicalReport {
        source_revision_hash: source_hash.clone(),
        compiler_version: WORK_ITEM_PLAN_COMPILER_VERSION.to_string(),
        findings,
    };
    let legacy_findings = vec![e2e_finding(None)];
    let mut legacy = PlanCandidateMechanicalReportRecord {
        id: format!("report-{}", &source_hash[..16]),
        source_revision_id: source.id.clone(),
        ir_id: ir.id.clone(),
        report: report_body(legacy_findings.clone()),
        // SG3a.0：旧 fixture 形态（无结构报告）。
        structure_report: None,
        content_hash: String::new(),
    };
    legacy.content_hash = legacy.content_hash().unwrap();
    let legacy_ref = source_store
        .put_mechanical_report("project_0001", "issue_0001", "entity_0001", &legacy)
        .unwrap();
    session.mechanical_report_ref = Some(legacy_ref.clone());
    write_json(
        &store
            .workspace_sessions_root("project_0001", "issue_0001")
            .join(format!("{}.json", session.id)),
        &session,
    )
    .expect("seed evaluate session with legacy report ref");

    // 同内容新 report：内容寻址 id（≠ 旧格式 id）→ ref 轮换。
    let mut rotated = PlanCandidateMechanicalReportRecord {
        id: String::new(),
        source_revision_id: source.id.clone(),
        ir_id: ir.id.clone(),
        report: report_body(legacy_findings),
        // SG3a.0：旧 fixture 形态（无结构报告）。
        structure_report: None,
        content_hash: String::new(),
    };
    rotated.id = rotated.content_addressed_id().unwrap();
    rotated.content_hash = rotated.content_hash().unwrap();
    assert_ne!(rotated.id, legacy.id);
    let rotated_ref = source_store
        .put_mechanical_report("project_0001", "issue_0001", "entity_0001", &rotated)
        .unwrap();

    let expected = store
        .get_workspace_session(&session.id)
        .expect("load evaluate session");
    let saved = store
        .compare_and_save_single_candidate_evaluation(&expected, &rotated_ref)
        .expect("equivalent-content report ref rotation must stay idempotent");
    assert_eq!(
        saved.mechanical_report_ref.as_deref(),
        Some(rotated_ref.as_str())
    );

    // 轮换后同 ref 重放仍幂等（相等分支）。
    let replayed = store
        .get_workspace_session(&session.id)
        .expect("reload rotated session");
    let saved_again = store
        .compare_and_save_single_candidate_evaluation(&replayed, &rotated_ref)
        .expect("replay after rotation is idempotent");
    assert_eq!(
        saved_again.mechanical_report_ref.as_deref(),
        Some(rotated_ref.as_str())
    );

    // 内容真变（typed kind）→ 不同内容寻址 id → CAS 仍 Conflict。
    let mut diverged = PlanCandidateMechanicalReportRecord {
        id: String::new(),
        source_revision_id: source.id.clone(),
        ir_id: ir.id.clone(),
        report: report_body(vec![e2e_finding(Some(PlanningFailureKind::Unknown {
            code: "integration_or_e2e_skipped_risk".to_string(),
        }))]),
        // SG3a.0：旧 fixture 形态（无结构报告）。
        structure_report: None,
        content_hash: String::new(),
    };
    diverged.id = diverged.content_addressed_id().unwrap();
    diverged.content_hash = diverged.content_hash().unwrap();
    let diverged_ref = source_store
        .put_mechanical_report("project_0001", "issue_0001", "entity_0001", &diverged)
        .unwrap();
    let current = store
        .get_workspace_session(&session.id)
        .expect("reload current session");
    assert!(matches!(
        store.compare_and_save_single_candidate_evaluation(&current, &diverged_ref),
        Err(ProductStoreError::Conflict { .. })
    ));
}
#[test]
fn single_candidate_compile_id_uses_the_published_test_vector() {
    assert_eq!(
        single_candidate_compile_id(
            "session-001",
            "plan-001",
            "approval-001",
            "2026-08-27T12:34:56Z",
        ),
        "5a16e570210838318554c17b3ebd0c433c3001ce00adb7b8e9726d79aecf788e",
    );
}

#[test]
fn single_candidate_approval_and_reservation_are_cas_bound_to_durable_refs() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let mut session = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "entity_0001".to_string(),
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
        .expect("create single candidate session");
    session.single_candidate_phase = Some(SingleCandidatePhase::Approval);
    session.work_item_plan_source_revision_ref = Some(
        "project/project_0001/issue/issue_0001/plan/entity_0001/source_revision/source-001"
            .to_string(),
    );
    session.plan_candidate_ir_ref = Some(
        "project/project_0001/issue/issue_0001/plan/entity_0001/plan_candidate_ir/ir-001"
            .to_string(),
    );
    session.mechanical_report_ref = Some(
        "project/project_0001/issue/issue_0001/plan/entity_0001/mechanical_report/report-001"
            .to_string(),
    );
    write_json(
        &store
            .workspace_sessions_root("project_0001", "issue_0001")
            .join(format!("{}.json", session.id)),
        &session,
    )
    .expect("seed durable candidate context");
    let approval_id = single_candidate_approval_attempt_id(
        &session.id,
        &session.entity_id,
        session
            .work_item_plan_source_revision_ref
            .as_deref()
            .unwrap(),
        session.plan_candidate_ir_ref.as_deref().unwrap(),
        session.mechanical_report_ref.as_deref().unwrap(),
    );
    let approved = store
        .compare_and_save_single_candidate_approval(&session, &approval_id, "2026-08-27T12:34:56Z")
        .expect("approval CAS");
    assert_eq!(
        approved.approval_attempt_id.as_deref(),
        Some(approval_id.as_str())
    );
    assert_eq!(
        approved.approved_at.as_deref(),
        Some("2026-08-27T12:34:56Z")
    );

    let reservation = SingleCandidateCompileReservation {
        compile_id: single_candidate_compile_id(
            &approved.id,
            &approved.entity_id,
            &approval_id,
            "2026-08-27T12:34:56Z",
        ),
        now: "2026-08-27T12:34:56Z".to_string(),
        publication_provenance_ref: format!(
            "project/{}/issue/{}/plan/{}/publication_provenance/{}",
            approved.project_id,
            approved.issue_id,
            approved.entity_id,
            single_candidate_compile_id(
                &approved.id,
                &approved.entity_id,
                &approval_id,
                "2026-08-27T12:34:56Z",
            )
        ),
    };
    let reserved = store
        .put_compile_reservation_cas(
            "project_0001",
            "issue_0001",
            "entity_0001",
            &approved.id,
            &approved,
            &reservation,
        )
        .expect("reservation CAS");
    assert_eq!(reserved.compile_reservation.as_ref(), Some(&reservation));
    assert!(matches!(
        store.compare_and_save_single_candidate_approval(
            &session,
            &approval_id,
            "2026-08-27T12:34:56Z",
        ),
        Err(ProductStoreError::Conflict { .. })
    ));
}

/// F-54 B1（诊断 §4.1）：修复轮经本 CAS 轮换 refs 时必须同时失效旧 Approval
/// 元组（approval_attempt_id / approved_at / compile_reservation）。0009 事故：
/// 旧元组与新 refs 失配 → 下次 approve 命中 compile 冲突臂 → durable Failed
/// 污染门相位。清空后下次 approve 走 (None, None) 全新元组分支。
#[test]
fn repair_generation_rotation_invalidates_stale_approval_tuple() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let mut session = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "entity_0001".to_string(),
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
        .expect("create single candidate session");
    let source_ref_v1 =
        "project/project_0001/issue/issue_0001/plan/entity_0001/source_revision/source-001";
    let ir_ref_v1 =
        "project/project_0001/issue/issue_0001/plan/entity_0001/plan_candidate_ir/ir-001";
    let report_ref_v1 =
        "project/project_0001/issue/issue_0001/plan/entity_0001/mechanical_report/report-001";
    // 门内反馈修订轮启动：phase=Generate + v1 refs/report + 首次 approve 落的旧元组。
    session.single_candidate_phase = Some(SingleCandidatePhase::Generate);
    session.work_item_plan_source_revision_ref = Some(source_ref_v1.to_string());
    session.plan_candidate_ir_ref = Some(ir_ref_v1.to_string());
    session.mechanical_report_ref = Some(report_ref_v1.to_string());
    session.approval_attempt_id = Some("affac4fc084a_stale_attempt_for_v1_refs".to_string());
    session.approved_at = Some("2026-09-24T10:09:16.606292797+00:00".to_string());
    session.compile_reservation = Some(SingleCandidateCompileReservation {
        compile_id: "1f94b833_stale_compile".to_string(),
        now: "2026-09-24T10:09:16.606292797+00:00".to_string(),
        publication_provenance_ref: "project/project_0001/issue/issue_0001/plan/entity_0001/publication_provenance/1f94b833_stale_compile".to_string(),
    });
    write_json(
        &store
            .workspace_sessions_root("project_0001", "issue_0001")
            .join(format!("{}.json", session.id)),
        &session,
    )
    .expect("seed repairing session with stale approval tuple");
    let expected = store
        .get_workspace_session(&session.id)
        .expect("reload repairing session");

    let source_ref_v2 =
        "project/project_0001/issue/issue_0001/plan/entity_0001/source_revision/source-002";
    let ir_ref_v2 =
        "project/project_0001/issue/issue_0001/plan/entity_0001/plan_candidate_ir/ir-002";
    let rotated = store
        .compare_and_save_single_candidate_generation(&expected, source_ref_v2, ir_ref_v2)
        .expect("persist repaired refs");
    assert_eq!(rotated.mechanical_report_ref, None);
    assert_eq!(
        rotated.single_candidate_phase,
        Some(SingleCandidatePhase::Evaluate)
    );
    assert_eq!(
        rotated.approval_attempt_id, None,
        "F-54 B1: refs 轮换必须失效旧 Approval 元组"
    );
    assert_eq!(rotated.approved_at, None, "F-54 B1: approved_at 随元组失效");
    assert_eq!(
        rotated.compile_reservation, None,
        "F-54 B1: compile_reservation 随元组失效"
    );

    // 修复轮后的下一次 approve 不得再冲突：evaluation 落新报告 → approval CAS
    // 以新 refs 计算 attempt id 落全新元组（旧代码在此处 Conflict）。
    let report_ref_v2 =
        "project/project_0001/issue/issue_0001/plan/entity_0001/mechanical_report/report-002";
    let evaluated = store
        .compare_and_save_single_candidate_evaluation(&rotated, report_ref_v2)
        .expect("persist repaired mechanical report");
    // 评审路由开门（EnterHumanGate）把相位从 Evaluate 提升到 Approval（M1/D6）。
    let mut gated = evaluated;
    gated.single_candidate_phase = Some(SingleCandidatePhase::Approval);
    write_json(
        &store
            .workspace_sessions_root("project_0001", "issue_0001")
            .join(format!("{}.json", gated.id)),
        &gated,
    )
    .expect("seed Approval gate after repaired evaluation");
    let fresh_attempt = single_candidate_approval_attempt_id(
        &gated.id,
        &gated.entity_id,
        source_ref_v2,
        ir_ref_v2,
        report_ref_v2,
    );
    let approved = store
        .compare_and_save_single_candidate_approval(&gated, &fresh_attempt, "2026-09-24T13:29:03Z")
        .expect("approve after repair rotation must not conflict with the stale tuple");
    assert_eq!(
        approved.approval_attempt_id.as_deref(),
        Some(fresh_attempt.as_str())
    );
}

/// SC 门内人工修订经 `complete_human_gate_revision` 轮换 refs 时必须同时
/// 失效旧 Approval 绑定（approval_attempt_id / approved_at /
/// compile_reservation / publication_provenance_ref）。缺口形态（诊断冻结）：
/// 首轮 confirm 在 Approval/reservation/provenance 持久化后中断 → 门重开 →
/// 门内修订换 refs → 复评通过再 confirm 时 compile 以新 refs 重算 attempt
/// id ≠ 旧元组 → `single_candidate_approval_conflict`，门永久卡死。refs 未
/// 真正轮换（同 source 重放：source-only 内容寻址下 refs 不变）时旧元组对
/// 当前 refs 仍有效，必须原样保留——同 refs 无新代次，不得静默覆盖。
#[test]
fn human_gate_revision_rotation_invalidates_stale_approval_tuple() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let mut session = store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "entity_0001".to_string(),
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
        .expect("create single candidate session");
    let source_ref_v1 =
        "project/project_0001/issue/issue_0001/plan/entity_0001/source_revision/source-001";
    let ir_ref_v1 =
        "project/project_0001/issue/issue_0001/plan/entity_0001/plan_candidate_ir/ir-001";
    let report_ref_v1 =
        "project/project_0001/issue/issue_0001/plan/entity_0001/mechanical_report/report-001";
    let stale_attempt = single_candidate_approval_attempt_id(
        &session.id,
        &session.entity_id,
        source_ref_v1,
        ir_ref_v1,
        report_ref_v1,
    );
    let stale_provenance_ref = "project/project_0001/issue/issue_0001/plan/entity_0001/publication_provenance/1f94b833_stale_compile";
    // 首轮 confirm 中断后的 durable 形态：Approval 相位 + v1 refs + 与 v1 绑定
    // 的旧 Approval 元组 + compile reservation + 旧 provenance 引用。
    session.single_candidate_phase = Some(SingleCandidatePhase::Approval);
    session.work_item_plan_source_revision_ref = Some(source_ref_v1.to_string());
    session.plan_candidate_ir_ref = Some(ir_ref_v1.to_string());
    session.mechanical_report_ref = Some(report_ref_v1.to_string());
    session.approval_attempt_id = Some(stale_attempt.clone());
    session.approved_at = Some("2026-10-12T08:00:00Z".to_string());
    session.compile_reservation = Some(SingleCandidateCompileReservation {
        compile_id: single_candidate_compile_id(
            &session.id,
            &session.entity_id,
            &stale_attempt,
            "2026-10-12T08:00:00Z",
        ),
        now: "2026-10-12T08:00:00Z".to_string(),
        publication_provenance_ref: stale_provenance_ref.to_string(),
    });
    session.publication_provenance_ref = Some(stale_provenance_ref.to_string());
    write_json(
        &store
            .workspace_sessions_root("project_0001", "issue_0001")
            .join(format!("{}.json", session.id)),
        &session,
    )
    .expect("seed approval gate with stale tuple");
    let expected = store
        .get_workspace_session(&session.id)
        .expect("reload approval gate session");

    let running_turn = crate::product::models::HumanGateTurn {
        turn_id: "human_gate_turn_rotation_0001".to_string(),
        session_id: expected.id.clone(),
        command_id: "cmd_rotation_0001".to_string(),
        feedback_text: "修订当前候选".to_string(),
        status: crate::product::models::HumanGateTurnStatus::Running,
        attempt_no: 1,
        budget_reserved: 1,
        source_hash: "a1b2c3d4".repeat(8),
        result_artifact_ref: None,
        failure_class: None,
        created_at: "2026-10-12T09:00:00Z".to_string(),
        updated_at: "2026-10-12T09:00:01Z".to_string(),
    };
    write_json(
        &store
            .human_gate_turn_path(&expected, &running_turn.turn_id)
            .expect("revision turn path"),
        &running_turn,
    )
    .expect("seed running revision turn");

    let source_ref_v2 =
        "project/project_0001/issue/issue_0001/plan/entity_0001/source_revision/source-002";
    let ir_ref_v2 =
        "project/project_0001/issue/issue_0001/plan/entity_0001/plan_candidate_ir/ir-002";
    let report_ref_v2 =
        "project/project_0001/issue/issue_0001/plan/entity_0001/mechanical_report/report-002";
    let mut completed = running_turn.clone();
    completed.status = crate::product::models::HumanGateTurnStatus::Completed;
    let rotated = store
        .complete_human_gate_revision(
            &expected,
            completed,
            source_ref_v2,
            ir_ref_v2,
            report_ref_v2,
            &[],
            "artifact_version_001",
        )
        .expect("persist in-gate revision refs");
    assert_eq!(
        rotated.work_item_plan_source_revision_ref.as_deref(),
        Some(source_ref_v2)
    );
    assert_eq!(rotated.plan_candidate_ir_ref.as_deref(), Some(ir_ref_v2));
    assert_eq!(
        rotated.mechanical_report_ref.as_deref(),
        Some(report_ref_v2)
    );
    assert_eq!(
        rotated.approval_attempt_id, None,
        "门内修订轮换 refs 必须失效旧 Approval 元组"
    );
    assert_eq!(rotated.approved_at, None, "approved_at 随元组失效");
    assert_eq!(
        rotated.compile_reservation, None,
        "compile_reservation 随元组失效"
    );
    assert_eq!(
        rotated.publication_provenance_ref, None,
        "旧 provenance 引用随旧编译代次一并失效"
    );

    // 复评通过后的下一次 approve 不得再冲突：清空后走 (None, None) 全新元组
    // 分支，新 attempt id 按新 refs 计算（旧代码在此命中 `_` 冲突臂）。
    let fresh_attempt = single_candidate_approval_attempt_id(
        &rotated.id,
        &rotated.entity_id,
        source_ref_v2,
        ir_ref_v2,
        report_ref_v2,
    );
    let approved = store
        .compare_and_save_single_candidate_approval(
            &rotated,
            &fresh_attempt,
            "2026-10-12T10:00:00Z",
        )
        .expect("approve after in-gate revision must not conflict with the stale tuple");
    assert_eq!(
        approved.approval_attempt_id.as_deref(),
        Some(fresh_attempt.as_str())
    );

    // refs 未真正轮换（同 source 重放）时不得误伤仍有效的新元组：source-only
    // 内容寻址使同 source 产出同 refs，现元组按 refs 计算仍精确匹配。
    let replay_expected = store
        .get_workspace_session(&approved.id)
        .expect("reload session after fresh approval");
    let replay_turn = crate::product::models::HumanGateTurn {
        turn_id: "human_gate_turn_rotation_0002".to_string(),
        session_id: replay_expected.id.clone(),
        command_id: "cmd_rotation_0002".to_string(),
        feedback_text: "按同一 source 重放修订".to_string(),
        status: crate::product::models::HumanGateTurnStatus::Running,
        attempt_no: 1,
        budget_reserved: 1,
        source_hash: "a1b2c3d4".repeat(8),
        result_artifact_ref: None,
        failure_class: None,
        created_at: "2026-10-12T11:00:00Z".to_string(),
        updated_at: "2026-10-12T11:00:01Z".to_string(),
    };
    write_json(
        &store
            .human_gate_turn_path(&replay_expected, &replay_turn.turn_id)
            .expect("replay turn path"),
        &replay_turn,
    )
    .expect("seed running replay turn");
    let mut replay_completed = replay_turn.clone();
    replay_completed.status = crate::product::models::HumanGateTurnStatus::Completed;
    let replayed = store
        .complete_human_gate_revision(
            &replay_expected,
            replay_completed,
            source_ref_v2,
            ir_ref_v2,
            report_ref_v2,
            &[],
            "artifact_version_002",
        )
        .expect("same-refs revision replay must persist");
    assert_eq!(
        replayed.approval_attempt_id.as_deref(),
        Some(fresh_attempt.as_str()),
        "refs 未轮换时有效元组必须原样保留"
    );
    assert_eq!(
        replayed.approved_at.as_deref(),
        Some("2026-10-12T10:00:00Z"),
        "refs 未轮换时 approved_at 不得被重置"
    );
}

// === SG4b（plan v1.1 Task SG4b）：合法交付/checkpoint/整组历史恢复 ===
// REQ-CG-02/05、REQ-SG-04/04A：合法 refs 提交 settle 一次、turn 仍
// Running；checkpoint 三 refs/version/hash 完整；纯 restore 0 author 且
// 审批整组失效；新反馈未满足/旧 context/缺 IR/report/hash 错禁止回退；
// review 无法 join 不进 Reviewing、已 review 同 refs/context 才能保；
// 旧 receipt 不能新计。

use crate::product::models::{
    HumanGateReservation, HumanGateTurn, HumanGateTurnFailureClass, HumanGateTurnStatus,
    StructuralCheckpoint, StructuralCheckpointStatus, StructuralGovernanceState,
    StructuralGrantPhase, StructuralRevisionGrant, UserBudgetReceipt, UserBudgetReceiptKind,
    structural_checkpoint_id,
};
use crate::product::planning_failure::PersistedPlanningStructureReport;
use crate::product::work_item_plan_compiler::{
    PlanCandidateIr, PlanCandidateMechanicalReport, WorkItemPlanSourceContext,
    compile_work_item_plan,
};
use crate::product::work_item_plan_policy::{HumanGateSnapshot, HumanReason};
use crate::product::work_item_plan_source_store::{
    PlanCandidateIrRecord, PlanCandidateMechanicalReportRecord, SourceRevisionRecord,
    WorkItemPlanSourceStore,
};
use crate::product::workspace_engine::draft_batch::compile_support::{
    PlanningStructureContext, PlanningStructureItem, PlanningStructureReport,
    project_effective_planning_graph,
};
use crate::product::workspace_engine::{
    PLANNING_STRUCTURE_RULE_VERSION, compute_planning_context_digest,
    freeze_persisted_structure_report, planning_structure_items_from_ir,
};
use crate::web::workspace_ws_types::ArtifactPayload;

/// 真实候选 fixture（rep4）——经真实 parse/lower 产出 IR，结构投影与
/// payload 冻结全走生产单源，不造第二套图算法。
const SG_04B_SOURCE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/product/work_item_plan_compiler/fixtures/work-item-plan-rep4.md"
));

struct Sg04bGeneration {
    source_ref: String,
    ir_ref: String,
    report_ref: String,
    source_hash: String,
    payload: PersistedPlanningStructureReport,
    report: PlanningStructureReport,
    items: Vec<PlanningStructureItem>,
    context: PlanningStructureContext,
}

fn sg_04b_write_session(store: &LifecycleStore, session: &WorkspaceSessionRecord) {
    write_json(
        &store
            .workspace_sessions_root(&session.project_id, &session.issue_id)
            .join(format!("{}.json", session.id)),
        session,
    )
    .expect("write session fixture");
}

fn sg_04b_session(store: &LifecycleStore, entity_id: &str) -> WorkspaceSessionRecord {
    let mut session = store
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
                run_policy: RunPolicy::Interactive,
                rollout_snapshot: true,
            }),
        })
        .expect("create single candidate session");
    session.human_gate_snapshot = Some(HumanGateSnapshot {
        candidate_recovery: None,
        findings: Vec::new(),
        repeated_fingerprints: Vec::new(),
        attempts_used: 0,
        manual_repairs_remaining: 3,
        trigger: HumanReason::NativeHumanRequired,
        resumable: true,
        accepted_feedback_turns: Some(0),
    });
    sg_04b_write_session(store, &session);
    session
}

fn sg_04b_seed_grant(session: &mut WorkspaceSessionRecord, turn_id: &str, command_id: &str) {
    let state = StructuralGovernanceState {
        current_grant: Some(StructuralRevisionGrant {
            grant_id: format!("sgr_{turn_id}"),
            authorization_hash: "f".repeat(64),
            turn_id: Some(turn_id.to_string()),
            command_id: Some(command_id.to_string()),
            candidate_source_revision_ref: None,
            plan_candidate_ir_ref: None,
            mechanical_report_ref: None,
            phase: Some(StructuralGrantPhase::AuthorRunning),
            attempt_no: 1,
            prompt_ref: Some(format!("prompt://{turn_id}/rev1")),
            prompt_digest: Some("a".repeat(64)),
            instruction_digest: Some("b".repeat(64)),
            failing_source_hash: Some("c".repeat(64)),
            review_identity: None,
            granted_at: "2026-10-13T00:00:00Z".to_string(),
        }),
        ..StructuralGovernanceState::default()
    };
    session.structural_governance_state = Some(state);
}

/// 真实链路建立 in-flight turn：reserve（hold+reservation+ledger 原子）
/// → Running。返回最新 session 与 Running turn。
fn sg_04b_reserve_running_turn(
    store: &LifecycleStore,
    expected: &WorkspaceSessionRecord,
    turn_id: &str,
    command_id: &str,
) -> (WorkspaceSessionRecord, HumanGateTurn) {
    let turn = sg_04b_make_turn(expected, turn_id, command_id, HumanGateTurnStatus::Reserved);
    let reservation = HumanGateReservation {
        command_id: command_id.to_string(),
        turn_id: turn_id.to_string(),
        provider_start_idempotency_key: format!("human_gate:{turn_id}:attempt:1"),
        reserved_at: "2026-10-13T00:00:00Z".to_string(),
    };
    let (reserved_session, _) = store
        .compare_and_reserve_human_gate_turn(expected, turn.clone(), reservation)
        .expect("reserve human gate turn");
    let running = HumanGateTurn {
        status: HumanGateTurnStatus::Running,
        ..turn
    };
    let started = store
        .update_human_gate_turn(&reserved_session, running.clone())
        .expect("start turn");
    (started, running)
}

fn sg_04b_make_turn(
    session: &WorkspaceSessionRecord,
    turn_id: &str,
    command_id: &str,
    status: HumanGateTurnStatus,
) -> HumanGateTurn {
    HumanGateTurn {
        turn_id: turn_id.to_string(),
        session_id: session.id.clone(),
        command_id: command_id.to_string(),
        feedback_text: "revise the structure".to_string(),
        status,
        attempt_no: 1,
        budget_reserved: 1,
        source_hash: String::new(),
        result_artifact_ref: None,
        failure_class: None,
        created_at: "2026-10-13T00:00:00Z".to_string(),
        updated_at: "2026-10-13T00:00:00Z".to_string(),
    }
}

fn sg_04b_artifact_versions(session: &WorkspaceSessionRecord) -> Vec<ArtifactVersion> {
    vec![ArtifactVersion {
        version: 1,
        payload: ArtifactPayload::Markdown {
            markdown: SG_04B_SOURCE.to_string(),
            diff: None,
        },
        generated_by: session.author_provider.clone(),
        reviewed_by: None,
        review_verdict: None,
        confirmed_by: None,
        is_current: true,
        created_at: "2026-10-13T02:00:00Z".to_string(),
        source_node_id: "timeline_node_sg4b".to_string(),
    }]
}

/// 真实 sealed generation：真实编译 IR + SG1 投影 + SG3a.1 冻结 payload
/// + store 完整校验链落盘（source/IR/report 三 refs 同代绑定）。
fn sg_04b_seal_generation(
    store: &LifecycleStore,
    session: &WorkspaceSessionRecord,
) -> Sg04bGeneration {
    let scope = SourceStoreScope {
        project_id: session.project_id.clone(),
        issue_id: session.issue_id.clone(),
        plan_id: session.entity_id.clone(),
    };
    let source_store = WorkItemPlanSourceStore::new(store.app_paths());
    let ir: PlanCandidateIr = compile_work_item_plan(
        SG_04B_SOURCE,
        &WorkItemPlanSourceContext {
            target_repository_id: "repo-sg4b".to_string(),
            member_targets: None,
        },
    )
    .expect("rep4 fixture must compile through the real compiler");
    let source_hash = ir.source_revision_hash.clone();
    let source_id = format!("source-{}", &source_hash[..16]);
    let ir_id = format!("ir-{}", &source_hash[..16]);
    let mut source_record = SourceRevisionRecord {
        id: source_id.clone(),
        source: SG_04B_SOURCE.to_string(),
        source_revision_hash: source_hash.clone(),
        content_hash: String::new(),
    };
    source_record.content_hash = source_record.content_hash().unwrap();
    let source_ref = source_store
        .put_source_revision(
            &scope.project_id,
            &scope.issue_id,
            &scope.plan_id,
            &source_record,
        )
        .expect("persist source revision");
    let mut ir_record = PlanCandidateIrRecord {
        id: ir_id.clone(),
        source_revision_id: source_id.clone(),
        ir,
        content_hash: String::new(),
    };
    ir_record.content_hash = ir_record.content_hash().unwrap();
    let ir_ref = source_store
        .put_plan_candidate_ir(
            &scope.project_id,
            &scope.issue_id,
            &scope.plan_id,
            &ir_record,
        )
        .expect("persist candidate IR");

    let mut context = PlanningStructureContext {
        source_revision_ref: source_ref.clone(),
        plan_candidate_ir_ref: ir_ref.clone(),
        source_hash: source_hash.clone(),
        compiler_version: ir_record.ir.compiler_version.clone(),
        rule_version: PLANNING_STRUCTURE_RULE_VERSION.to_string(),
        context_digest: String::new(),
        confirmed_design_id: None,
        confirmed_design_digest: None,
        member_table_digest: None,
        selection_digest: None,
        physical_by_logical: std::collections::BTreeMap::new(),
        change_order: Vec::new(),
        options_digest: sha2_hex(b"{}"),
        baseline_tree_digest: None,
        intent_digest: None,
    };
    context.context_digest = compute_planning_context_digest(&context);
    let mut items = planning_structure_items_from_ir(&ir_record.ir);
    // legacy 单仓归一化（与生产门 assess_single_candidate_structure 同
    // 口径）：无授权成员表时 Plan Intent 归属不参与物理 target 解析。
    if context.physical_by_logical.is_empty() {
        for item in &mut items {
            item.declared_member = None;
            item.logical_target = None;
        }
    }
    let report = project_effective_planning_graph(&items, &context);
    assert!(
        report.structurally_valid,
        "rep4 fixture must project to a legal DAG"
    );
    let payload =
        freeze_persisted_structure_report(&report, &source_id, &ir_id).expect("freeze payload");

    let mut report_record = PlanCandidateMechanicalReportRecord {
        id: String::new(),
        source_revision_id: source_id.clone(),
        ir_id: ir_id.clone(),
        report: PlanCandidateMechanicalReport {
            source_revision_hash: source_hash.clone(),
            compiler_version: ir_record.ir.compiler_version.clone(),
            findings: Vec::new(),
        },
        structure_report: Some(payload.clone()),
        content_hash: String::new(),
    };
    report_record.id = report_record.content_addressed_id().unwrap();
    report_record.content_hash = report_record.content_hash().unwrap();
    let report_ref = source_store
        .put_mechanical_report(
            &scope.project_id,
            &scope.issue_id,
            &scope.plan_id,
            &report_record,
        )
        .expect("persist sealed mechanical report");
    Sg04bGeneration {
        source_ref,
        ir_ref,
        report_ref,
        source_hash,
        payload,
        report,
        items,
        context,
    }
}

fn sha2_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(bytes))
}

/// 已提交 checkpoint 的通用前置：session + sealed generation + 真实
/// reserve/Running turn + commit。
fn sg_04b_committed_generation(
    store: &LifecycleStore,
    entity_id: &str,
) -> (
    WorkspaceSessionRecord,
    HumanGateTurn,
    Sg04bGeneration,
    StructuralCheckpoint,
) {
    let mut session = sg_04b_session(store, entity_id);
    let generation = sg_04b_seal_generation(store, &session);
    sg_04b_seed_grant(&mut session, "turn_sg4b_0001", "cmd_sg4b_0001");
    sg_04b_write_session(store, &session);
    let (expected, turn) =
        sg_04b_reserve_running_turn(store, &session, "turn_sg4b_0001", "cmd_sg4b_0001");
    let artifact_versions = sg_04b_artifact_versions(&expected);
    let committed = store
        .commit_structurally_valid_revision(
            &expected,
            turn.clone(),
            &generation.source_ref,
            &generation.ir_ref,
            &generation.report_ref,
            &artifact_versions,
            "artifact_version_001",
            &generation.report,
        )
        .expect("commit structurally valid revision");
    let checkpoint = committed
        .structural_governance_state
        .as_ref()
        .expect("governance state after commit")
        .structure_checkpoints[0]
        .clone();
    (committed, turn, generation, checkpoint)
}

#[test]
fn sg_04b_checkpoint_current_context_business_and_review_join() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let mut session = sg_04b_session(&store, "entity_sg4b_a");
    let generation = sg_04b_seal_generation(&store, &session);

    // 预置旧审批代次，观察 commit 的同 CAS 整组失效。
    session.single_candidate_phase = Some(SingleCandidatePhase::Approval);
    session.approval_attempt_id = Some("legacy-approval".to_string());
    session.approved_at = Some("2026-10-12T00:00:00Z".to_string());
    session.compile_reservation = Some(SingleCandidateCompileReservation {
        compile_id: "legacy-compile".to_string(),
        now: "2026-10-12T00:00:00Z".to_string(),
        publication_provenance_ref: "project/project_0001/issue/issue_0001/plan/entity_sg4b_a/publication_provenance/legacy-compile".to_string(),
    });
    session.publication_provenance_ref =
        Some("project/project_0001/issue/issue_0001/plan/entity_sg4b_a/publication_provenance/legacy-compile".to_string());
    sg_04b_seed_grant(&mut session, "turn_sg4b_0001", "cmd_sg4b_0001");
    sg_04b_write_session(&store, &session);

    let (expected, turn) =
        sg_04b_reserve_running_turn(&store, &session, "turn_sg4b_0001", "cmd_sg4b_0001");
    let ledger_len_after_start = expected.provider_start_ledger.len();
    let artifact_versions = sg_04b_artifact_versions(&expected);

    // —— 合法 refs 提交：settle 一次、turn 仍 Running、checkpoint 完整 ——
    let committed = store
        .commit_structurally_valid_revision(
            &expected,
            turn.clone(),
            &generation.source_ref,
            &generation.ir_ref,
            &generation.report_ref,
            &artifact_versions,
            "artifact_version_001",
            &generation.report,
        )
        .expect("commit structurally valid revision");

    assert_eq!(
        committed.work_item_plan_source_revision_ref.as_deref(),
        Some(generation.source_ref.as_str())
    );
    assert_eq!(
        committed.plan_candidate_ir_ref.as_deref(),
        Some(generation.ir_ref.as_str())
    );
    assert_eq!(
        committed.mechanical_report_ref.as_deref(),
        Some(generation.report_ref.as_str())
    );
    // 审批代次整组失效（同 CAS）。
    assert_eq!(committed.approval_attempt_id, None);
    assert_eq!(committed.approved_at, None);
    assert!(committed.compile_reservation.is_none());
    assert_eq!(committed.publication_provenance_ref, None);
    // 实际 start：commit 不写 provider ledger（claim/start/commit 分账）。
    assert_eq!(
        committed.provider_start_ledger.len(),
        ledger_len_after_start
    );

    let state = committed
        .structural_governance_state
        .as_ref()
        .expect("governance state");
    // deliver settle 恰一次：单条 DeliveredCharge receipt 绑定同代 refs/context。
    assert_eq!(state.user_receipts.len(), 1);
    let receipt = &state.user_receipts[0];
    assert_eq!(receipt.kind, UserBudgetReceiptKind::DeliveredCharge);
    assert_eq!(receipt.turn_id, "turn_sg4b_0001");
    assert_eq!(receipt.command_id, "cmd_sg4b_0001");
    assert_eq!(
        receipt.source_revision_ref.as_deref(),
        Some(generation.source_ref.as_str())
    );
    assert_eq!(
        receipt.plan_candidate_ir_ref.as_deref(),
        Some(generation.ir_ref.as_str())
    );
    assert_eq!(
        receipt.structure_report_ref.as_deref(),
        Some(generation.report_ref.as_str())
    );
    assert_eq!(
        receipt.context_digest.as_deref(),
        Some(generation.payload.context_digest.as_str())
    );
    assert!(state.user_holds.is_empty(), "deliver 后活动 hold 结清");
    assert_eq!(state.available_user_budget(), 2);
    // grant：phase→Reviewing 在 commit 侧推进，refs 同代绑定。
    let grant = state.current_grant.as_ref().expect("grant");
    assert_eq!(grant.phase, Some(StructuralGrantPhase::Reviewing));
    assert_eq!(
        grant.candidate_source_revision_ref.as_deref(),
        Some(generation.source_ref.as_str())
    );
    assert_eq!(
        grant.mechanical_report_ref.as_deref(),
        Some(generation.report_ref.as_str())
    );
    // checkpoint：三 refs/version/hash 完整。
    assert_eq!(state.structure_checkpoints.len(), 1);
    let checkpoint = &state.structure_checkpoints[0];
    assert_eq!(
        checkpoint.checkpoint_id,
        structural_checkpoint_id(
            "turn_sg4b_0001",
            &generation.source_ref,
            &generation.ir_ref,
            &generation.report_ref
        )
    );
    assert_eq!(
        checkpoint.source_revision_ref.as_deref(),
        Some(generation.source_ref.as_str())
    );
    assert_eq!(
        checkpoint.plan_candidate_ir_ref.as_deref(),
        Some(generation.ir_ref.as_str())
    );
    assert_eq!(
        checkpoint.structure_report_ref.as_deref(),
        Some(generation.report_ref.as_str())
    );
    assert_eq!(checkpoint.artifact_version, Some(1));
    assert_eq!(
        checkpoint.source_hash.as_deref(),
        Some(generation.source_hash.as_str())
    );
    assert_eq!(
        checkpoint.context_digest.as_deref(),
        Some(generation.payload.context_digest.as_str())
    );
    assert_eq!(
        checkpoint.authorization_hash.as_deref(),
        Some("f".repeat(64).as_str())
    );
    assert_eq!(
        checkpoint.receipt_ref.as_deref(),
        Some("user_budget_receipt:turn_sg4b_0001")
    );
    assert_eq!(checkpoint.status, StructuralCheckpointStatus::Active);
    assert_eq!(
        state.current_structure_report_ref.as_deref(),
        Some(generation.report_ref.as_str())
    );
    assert_eq!(
        state.current_context_digest.as_deref(),
        Some(generation.payload.context_digest.as_str())
    );

    // turn 仍 Running（与 complete 分离：不提前 Completed），结果 ref 已绑。
    let durable_turn = store
        .get_human_gate_turn(&committed.id, "turn_sg4b_0001")
        .expect("durable turn");
    assert_eq!(durable_turn.status, HumanGateTurnStatus::Running);
    assert_eq!(
        durable_turn.result_artifact_ref.as_deref(),
        Some("artifact_version_001")
    );
    // durable session 与返回值一致（账本落盘事实）。
    assert_eq!(
        store.get_workspace_session(&committed.id).unwrap(),
        committed
    );

    // settle 恰一次：过期 expected 重放被 CAS 拒绝，receipt 不双计。
    assert!(matches!(
        store.commit_structurally_valid_revision(
            &expected,
            turn.clone(),
            &generation.source_ref,
            &generation.ir_ref,
            &generation.report_ref,
            &artifact_versions,
            "artifact_version_001",
            &generation.report,
        ),
        Err(ProductStoreError::Conflict { .. })
    ));
    assert_eq!(
        store
            .get_workspace_session(&committed.id)
            .unwrap()
            .structural_governance_state
            .as_ref()
            .unwrap()
            .user_receipts
            .len(),
        1
    );

    // —— 纯 restore：0 author、审批失效、旧 receipt 不新计 ——
    // 模拟后续代次漂移：当前 refs 指向 gen2（新候选）且带旧审批元组。
    let mut drifted = committed.clone();
    drifted.work_item_plan_source_revision_ref = Some(
        "project/project_0001/issue/issue_0001/plan/entity_sg4b_a/source_revision/source-gen2"
            .to_string(),
    );
    drifted.plan_candidate_ir_ref = Some(
        "project/project_0001/issue/issue_0001/plan/entity_sg4b_a/plan_candidate_ir/ir-gen2"
            .to_string(),
    );
    drifted.mechanical_report_ref = Some(
        "project/project_0001/issue/issue_0001/plan/entity_sg4b_a/mechanical_report/report-gen2"
            .to_string(),
    );
    drifted.approval_attempt_id = Some("gen2-approval".to_string());
    drifted.approved_at = Some("2026-10-13T03:00:00Z".to_string());
    drifted.compile_reservation = Some(SingleCandidateCompileReservation {
        compile_id: "gen2-compile".to_string(),
        now: "2026-10-13T03:00:00Z".to_string(),
        publication_provenance_ref: "project/project_0001/issue/issue_0001/plan/entity_sg4b_a/publication_provenance/gen2-compile".to_string(),
    });
    drifted.publication_provenance_ref = Some(
        "project/project_0001/issue/issue_0001/plan/entity_sg4b_a/publication_provenance/gen2-compile"
            .to_string(),
    );
    sg_04b_write_session(&store, &drifted);

    // 评审已绑定同 refs/context（finalize 链产物）：review join 可保 Reviewing。
    {
        let mut reviewed = drifted.clone();
        let grant = reviewed
            .structural_governance_state
            .as_mut()
            .unwrap()
            .current_grant
            .as_mut()
            .unwrap();
        grant.review_identity = Some("review_node_sg4b_0001".to_string());
        sg_04b_write_session(&store, &reviewed);
        let restored = store
            .restore_structural_checkpoint(&reviewed, checkpoint, &generation.report)
            .expect("restore with joined review");
        assert_eq!(
            restored.work_item_plan_source_revision_ref.as_deref(),
            Some(generation.source_ref.as_str())
        );
        assert_eq!(restored.approval_attempt_id, None);
        assert_eq!(restored.approved_at, None);
        assert!(restored.compile_reservation.is_none());
        assert_eq!(restored.publication_provenance_ref, None);
        let state = restored.structural_governance_state.as_ref().unwrap();
        assert_eq!(
            state.current_grant.as_ref().unwrap().phase,
            Some(StructuralGrantPhase::Reviewing),
            "已 review 同 refs/context 才能保 Reviewing"
        );
        assert_eq!(
            restored.provider_start_ledger.len(),
            ledger_len_after_start,
            "纯回绑 0 author"
        );
        assert_eq!(state.user_receipts.len(), 1, "旧 receipt 不新计");
    }

    // review 无法 join（无 review_identity）：restore 后复评，不冒充已评审。
    // （重置回 drifted 前置态——上一臂的 restore 已把 durable 状态回绑。）
    sg_04b_write_session(&store, &drifted);
    {
        let restored = store
            .restore_structural_checkpoint(&drifted, checkpoint, &generation.report)
            .expect("pure restore without review join");
        assert_eq!(
            restored.work_item_plan_source_revision_ref.as_deref(),
            Some(generation.source_ref.as_str())
        );
        assert_eq!(
            restored.mechanical_report_ref.as_deref(),
            Some(generation.report_ref.as_str())
        );
        assert_eq!(restored.approval_attempt_id, None);
        assert_eq!(restored.approved_at, None);
        assert!(restored.compile_reservation.is_none());
        assert_eq!(restored.publication_provenance_ref, None);
        let state = restored.structural_governance_state.as_ref().unwrap();
        assert_eq!(
            state.current_grant.as_ref().unwrap().phase,
            Some(StructuralGrantPhase::Assessing),
            "review 无法 join 不得进 Reviewing → 复评"
        );
        assert_eq!(
            restored.provider_start_ledger.len(),
            ledger_len_after_start,
            "纯回绑 0 author（零启动、零 attempt 变化）"
        );
        assert_eq!(state.current_grant.as_ref().unwrap().attempt_no, 1);
        assert!(state.user_holds.is_empty());
        assert_eq!(state.user_receipts.len(), 1, "旧 receipt 不能新计");
        assert_eq!(state.available_user_budget(), 2);
        assert_eq!(
            state.structure_checkpoints[0].status,
            StructuralCheckpointStatus::Active
        );
        assert_eq!(
            state.current_structure_report_ref.as_deref(),
            Some(generation.report_ref.as_str())
        );
    }
}

#[test]
fn sg_04b_restore_rejects_missing_refs_hash_and_context_drift() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let (committed, turn, generation, checkpoint) =
        sg_04b_committed_generation(&store, "entity_sg4b_b");

    // 缺 IR（历史版本只有 Markdown/缺整组证据）禁止回退。
    let mut missing_ir = checkpoint.clone();
    missing_ir.plan_candidate_ir_ref = None;
    assert!(matches!(
        store.restore_structural_checkpoint(&committed, &missing_ir, &generation.report),
        Err(ProductStoreError::InvalidRecord { .. })
    ));
    // 缺结构报告 ref 禁止回退。
    let mut missing_report = checkpoint.clone();
    missing_report.structure_report_ref = None;
    assert!(matches!(
        store.restore_structural_checkpoint(&committed, &missing_report, &generation.report),
        Err(ProductStoreError::InvalidRecord { .. })
    ));
    // checkpoint hash 与耐久代失配禁止回退。
    let mut tampered_hash = checkpoint.clone();
    tampered_hash.source_hash = Some("0".repeat(64));
    assert!(matches!(
        store.restore_structural_checkpoint(&committed, &tampered_hash, &generation.report),
        Err(ProductStoreError::InvalidRecord { .. })
    ));
    // 旧 context（当前 Design/member/options/baseline 漂移）禁止回退：
    // 以当前上下文重派生的 SG1 报告 digest 与冻结 payload 失配。
    let mut drifted_context = generation.context.clone();
    drifted_context.options_digest = sha2_hex(b"{\"changed\":true}");
    drifted_context.context_digest = compute_planning_context_digest(&drifted_context);
    let drifted_report = project_effective_planning_graph(&generation.items, &drifted_context);
    assert!(drifted_report.structurally_valid);
    assert_ne!(
        drifted_report.context_digest,
        generation.payload.context_digest
    );
    assert!(matches!(
        store.restore_structural_checkpoint(&committed, &checkpoint, &drifted_report),
        Err(ProductStoreError::InvalidRecord { .. })
    ));
    // 拒收后 durable 状态零变化。
    assert_eq!(
        store.get_workspace_session(&committed.id).unwrap(),
        committed
    );

    // 新反馈未满足（turn2 活动 hold）禁止整组回退。
    let completed_turn = HumanGateTurn {
        status: HumanGateTurnStatus::Completed,
        ..turn.clone()
    };
    let completed_session = store
        .update_human_gate_turn(&committed, completed_turn)
        .expect("terminalize first turn");
    let turn2 = sg_04b_make_turn(
        &completed_session,
        "turn_sg4b_0002",
        "cmd_sg4b_0002",
        HumanGateTurnStatus::Reserved,
    );
    let reservation2 = HumanGateReservation {
        command_id: "cmd_sg4b_0002".to_string(),
        turn_id: "turn_sg4b_0002".to_string(),
        provider_start_idempotency_key: "human_gate:turn_sg4b_0002:attempt:1".to_string(),
        reserved_at: "2026-10-13T04:00:00Z".to_string(),
    };
    let (held_session, _) = store
        .compare_and_reserve_human_gate_turn(&completed_session, turn2, reservation2)
        .expect("reserve second feedback turn");
    assert!(matches!(
        store.restore_structural_checkpoint(&held_session, &checkpoint, &generation.report),
        Err(ProductStoreError::Conflict { .. })
    ));
    assert_eq!(
        store.get_workspace_session(&held_session.id).unwrap(),
        held_session,
        "新反馈未满足时拒绝回退且零写"
    );
}

#[test]
fn sg_04b_commit_rejects_unsealed_generation_and_non_running_turn() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let mut session = sg_04b_session(&store, "entity_sg4b_c");
    let generation = sg_04b_seal_generation(&store, &session);
    sg_04b_seed_grant(&mut session, "turn_sg4b_0001", "cmd_sg4b_0001");
    sg_04b_write_session(&store, &session);
    let (expected, turn) =
        sg_04b_reserve_running_turn(&store, &session, "turn_sg4b_0001", "cmd_sg4b_0001");
    let artifact_versions = sg_04b_artifact_versions(&expected);

    // 旧代 record（无 Some 结构 payload）不得作为合法交付 refs。
    let scope = SourceStoreScope {
        project_id: session.project_id.clone(),
        issue_id: session.issue_id.clone(),
        plan_id: session.entity_id.clone(),
    };
    let source_store = WorkItemPlanSourceStore::new(store.app_paths());
    let source_id = format!("source-{}", &generation.source_hash[..16]);
    let ir_id = format!("ir-{}", &generation.source_hash[..16]);
    let mut unsealed = PlanCandidateMechanicalReportRecord {
        id: String::new(),
        source_revision_id: source_id,
        ir_id,
        report: PlanCandidateMechanicalReport {
            source_revision_hash: generation.source_hash.clone(),
            compiler_version: generation.report.compiler_version.clone(),
            findings: Vec::new(),
        },
        structure_report: None,
        content_hash: String::new(),
    };
    unsealed.id = unsealed.content_addressed_id().unwrap();
    unsealed.content_hash = unsealed.content_hash().unwrap();
    let unsealed_ref = source_store
        .put_mechanical_report(
            &scope.project_id,
            &scope.issue_id,
            &scope.plan_id,
            &unsealed,
        )
        .expect("persist unsealed legacy report");
    assert!(matches!(
        store.commit_structurally_valid_revision(
            &expected,
            turn.clone(),
            &generation.source_ref,
            &generation.ir_ref,
            &unsealed_ref,
            &artifact_versions,
            "artifact_version_001",
            &generation.report,
        ),
        Err(ProductStoreError::InvalidRecord { .. })
    ));

    // 传入 SG1 报告与冻结 payload 不同代（context 漂移）不得提交。
    let mut drifted_context = generation.context.clone();
    drifted_context.options_digest = sha2_hex(b"{\"changed\":true}");
    drifted_context.context_digest = compute_planning_context_digest(&drifted_context);
    let drifted_report = project_effective_planning_graph(&generation.items, &drifted_context);
    assert!(matches!(
        store.commit_structurally_valid_revision(
            &expected,
            turn.clone(),
            &generation.source_ref,
            &generation.ir_ref,
            &generation.report_ref,
            &artifact_versions,
            "artifact_version_001",
            &drifted_report,
        ),
        Err(ProductStoreError::InvalidRecord { .. })
    ));

    // 非 Running turn（Reserved）不得提交。
    let mut reserved_session = sg_04b_session(&store, "entity_sg4b_d");
    sg_04b_seed_grant(&mut reserved_session, "turn_sg4b_r", "cmd_sg4b_r");
    sg_04b_write_session(&store, &reserved_session);
    let reserved_turn = sg_04b_make_turn(
        &reserved_session,
        "turn_sg4b_r",
        "cmd_sg4b_r",
        HumanGateTurnStatus::Reserved,
    );
    let reservation = HumanGateReservation {
        command_id: "cmd_sg4b_r".to_string(),
        turn_id: "turn_sg4b_r".to_string(),
        provider_start_idempotency_key: "human_gate:turn_sg4b_r:attempt:1".to_string(),
        reserved_at: "2026-10-13T00:00:00Z".to_string(),
    };
    let (reserved_expected, reserved_running) = store
        .compare_and_reserve_human_gate_turn(&reserved_session, reserved_turn, reservation)
        .expect("reserve turn");
    assert!(matches!(
        store.commit_structurally_valid_revision(
            &reserved_expected,
            reserved_running,
            &generation.source_ref,
            &generation.ir_ref,
            &generation.report_ref,
            &artifact_versions,
            "artifact_version_001",
            &generation.report,
        ),
        Err(ProductStoreError::InvalidRecord { .. })
    ));

    let grantless_session = sg_04b_session(&store, "entity_sg4b_c2");
    let (grantless_expected, grantless_turn) = sg_04b_reserve_running_turn(
        &store,
        &grantless_session,
        "turn_sg4b_nogr",
        "cmd_sg4b_nogr",
    );
    // 无活动 grant（reserve 物化状态但未建授权窗口）不得提交。
    assert!(matches!(
        store.commit_structurally_valid_revision(
            &grantless_expected,
            grantless_turn,
            &generation.source_ref,
            &generation.ir_ref,
            &generation.report_ref,
            &artifact_versions,
            "artifact_version_001",
            &generation.report,
        ),
        Err(ProductStoreError::InvalidRecord { .. })
    ));

    // 无活动 hold（turn 未经 reserve 预约）不得结算交付。
    let mut no_hold_session = sg_04b_session(&store, "entity_sg4b_e");
    sg_04b_seed_grant(&mut no_hold_session, "turn_sg4b_h", "cmd_sg4b_h");
    sg_04b_write_session(&store, &no_hold_session);
    let no_hold_turn = sg_04b_make_turn(
        &no_hold_session,
        "turn_sg4b_h",
        "cmd_sg4b_h",
        HumanGateTurnStatus::Running,
    );
    write_json(
        &store
            .human_gate_turn_path(&no_hold_session, "turn_sg4b_h")
            .unwrap(),
        &no_hold_turn,
    )
    .unwrap();
    assert!(matches!(
        store.commit_structurally_valid_revision(
            &no_hold_session,
            no_hold_turn,
            &generation.source_ref,
            &generation.ir_ref,
            &generation.report_ref,
            &artifact_versions,
            "artifact_version_001",
            &generation.report,
        ),
        Err(ProductStoreError::InvalidRecord { .. })
    ));
}

#[test]
fn sg_04b_finalize_grant_requires_typed_evidence() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let (committed, turn, _generation, checkpoint) =
        sg_04b_committed_generation(&store, "entity_sg4b_f");
    let grant_id = format!("sgr_{}", turn.turn_id);

    // Running turn：不提前 Complete，终态化证据缺失即拒。
    assert!(matches!(
        store.finalize_structural_revision_grant(
            &committed,
            turn.clone(),
            &grant_id,
            Some("review_node_sg4b_0001"),
            Some("scope_digest_sg4b_0001"),
        ),
        Err(ProductStoreError::InvalidRecord { .. })
    ));

    // Completed 但无 review join 证据：不得 Resolved。
    let completed_turn = HumanGateTurn {
        status: HumanGateTurnStatus::Completed,
        ..turn.clone()
    };
    let completed_session = store
        .update_human_gate_turn(&committed, completed_turn.clone())
        .expect("complete turn");
    assert!(matches!(
        store.finalize_structural_revision_grant(
            &completed_session,
            completed_turn.clone(),
            &grant_id,
            None,
            None,
        ),
        Err(ProductStoreError::InvalidRecord { .. })
    ));

    // Completed + review typed 证据：grant Resolved、checkpoint.review_scope 回填。
    let finalized = store
        .finalize_structural_revision_grant(
            &completed_session,
            completed_turn.clone(),
            &grant_id,
            Some("review_node_sg4b_0001"),
            Some("scope_digest_sg4b_0001"),
        )
        .expect("finalize with review evidence");
    let state = finalized.structural_governance_state.as_ref().unwrap();
    let grant = state.current_grant.as_ref().unwrap();
    assert_eq!(grant.phase, Some(StructuralGrantPhase::Resolved));
    assert_eq!(
        grant.review_identity.as_deref(),
        Some("review_node_sg4b_0001")
    );
    assert_eq!(
        state.structure_checkpoints[0].review_scope.as_deref(),
        Some("scope_digest_sg4b_0001")
    );
    assert_eq!(state.user_receipts.len(), 1, "finalize 不改账本");
    // grant 结束后 checkpoint 仍保留（REQ-SG-04）。
    assert_eq!(state.structure_checkpoints.len(), 1);
    assert_eq!(
        state.structure_checkpoints[0].checkpoint_id,
        checkpoint.checkpoint_id
    );

    // 非 target grant：拒绝（不碰他人授权窗口）。
    assert!(matches!(
        store.finalize_structural_revision_grant(
            &finalized,
            completed_turn,
            "sgr_some_other_grant",
            Some("review_node_sg4b_0001"),
            Some("scope_digest_sg4b_0001"),
        ),
        Err(ProductStoreError::Conflict { .. })
    ));

    // Failed(BudgetExhausted)：released receipt 先落（SG4a 真实入口）才可
    // 终态化为 SystemFault；未结算先拒。
    let mut exhausted_session = sg_04b_session(&store, "entity_sg4b_g");
    sg_04b_seed_grant(&mut exhausted_session, "turn_sg4b_x", "cmd_sg4b_x");
    sg_04b_write_session(&store, &exhausted_session);
    let exhausted_turn = sg_04b_make_turn(
        &exhausted_session,
        "turn_sg4b_x",
        "cmd_sg4b_x",
        HumanGateTurnStatus::Reserved,
    );
    let reservation = HumanGateReservation {
        command_id: "cmd_sg4b_x".to_string(),
        turn_id: "turn_sg4b_x".to_string(),
        provider_start_idempotency_key: "human_gate:turn_sg4b_x:attempt:1".to_string(),
        reserved_at: "2026-10-13T00:00:00Z".to_string(),
    };
    let (held, _) = store
        .compare_and_reserve_human_gate_turn(&exhausted_session, exhausted_turn, reservation)
        .expect("reserve exhausted turn");
    let failed_turn = HumanGateTurn {
        status: HumanGateTurnStatus::Failed,
        failure_class: Some(HumanGateTurnFailureClass::BudgetExhausted),
        ..sg_04b_make_turn(
            &held,
            "turn_sg4b_x",
            "cmd_sg4b_x",
            HumanGateTurnStatus::Failed,
        )
    };
    let failed_session = store
        .update_human_gate_turn(&held, failed_turn.clone())
        .expect("fail turn on exhaustion");
    assert!(matches!(
        store.finalize_structural_revision_grant(
            &failed_session,
            failed_turn.clone(),
            "sgr_turn_sg4b_x",
            None,
            None
        ),
        Err(ProductStoreError::InvalidRecord { .. })
    ));
    let release = UserBudgetReceipt {
        kind: UserBudgetReceiptKind::ReleasedStructural,
        turn_id: "turn_sg4b_x".to_string(),
        command_id: "cmd_sg4b_x".to_string(),
        attempt_reason: None,
        source_revision_ref: None,
        plan_candidate_ir_ref: None,
        structure_report_ref: None,
        context_digest: None,
        reason: Some("structural_exhausted_before_delivery".to_string()),
        settled_at: "2026-10-13T05:00:00Z".to_string(),
    };
    let released_session = store
        .settle_structural_user_budget(&failed_session, "turn_sg4b_x", release)
        .expect("release unexhausted hold");
    let finalized_fault = store
        .finalize_structural_revision_grant(
            &released_session,
            failed_turn,
            "sgr_turn_sg4b_x",
            None,
            None,
        )
        .expect("finalize exhaustion with typed evidence");
    assert_eq!(
        finalized_fault
            .structural_governance_state
            .as_ref()
            .unwrap()
            .current_grant
            .as_ref()
            .unwrap()
            .phase,
        Some(StructuralGrantPhase::SystemFault)
    );
}

/// SG4b fix round F1(P2):`record_structure_checkpoint` 同 id 回绑分支必须
/// 把其它 Active 统一降 Superseded——同一时刻至多一个 Active
/// (finalize 按 Active 定位当前交付代次)。复现链:commit G1→(新 turn)
/// commit G2(旧 Active 降级)→restore G1→双 Active 缺陷。
#[test]
fn sg_04b_rebind_restores_single_active_checkpoint() {
    let temp = tempdir().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
    let (committed, turn, generation, checkpoint_a) =
        sg_04b_committed_generation(&store, "entity_sg4b_h");

    // turn1 终态化后,真实链路预约第二反馈 turn,携同代 refs 再交付一代。
    let completed_turn = HumanGateTurn {
        status: HumanGateTurnStatus::Completed,
        ..turn.clone()
    };
    let completed = store
        .update_human_gate_turn(&committed, completed_turn)
        .expect("terminalize first turn");
    let (g2_expected, g2_turn) =
        sg_04b_reserve_running_turn(&store, &completed, "turn_sg4b_0002", "cmd_sg4b_0002");
    // grant 轮换到 turn2(SG6 producer 空缺,fixture 直写——SG4a 先例)。
    let mut g2_expected = g2_expected;
    {
        let state = g2_expected.structural_governance_state.as_mut().unwrap();
        let grant = state.current_grant.as_mut().unwrap();
        grant.grant_id = "sgr_turn_sg4b_0002".to_string();
        grant.turn_id = Some("turn_sg4b_0002".to_string());
        grant.command_id = Some("cmd_sg4b_0002".to_string());
        grant.phase = Some(StructuralGrantPhase::AuthorRunning);
        grant.attempt_no = 1;
        grant.review_identity = None;
    }
    sg_04b_write_session(&store, &g2_expected);
    let mut artifact_v2 = sg_04b_artifact_versions(&g2_expected);
    artifact_v2[0].version = 2;
    let committed_g2 = store
        .commit_structurally_valid_revision(
            &g2_expected,
            g2_turn.clone(),
            &generation.source_ref,
            &generation.ir_ref,
            &generation.report_ref,
            &artifact_v2,
            "artifact_version_002",
            &generation.report,
        )
        .expect("commit second delivery turn");
    {
        let state = committed_g2.structural_governance_state.as_ref().unwrap();
        assert_eq!(state.structure_checkpoints.len(), 2);
        let active_ids: Vec<&str> = state
            .structure_checkpoints
            .iter()
            .filter(|checkpoint| checkpoint.status == StructuralCheckpointStatus::Active)
            .map(|checkpoint| checkpoint.checkpoint_id.as_str())
            .collect();
        assert_eq!(active_ids.len(), 1, "G2 落位时旧 Active 降级(既有行为)");
    }

    // restore 回绑第一代:唯一 Active 必须回到 G1,第二代降 Superseded。
    let restored = store
        .restore_structural_checkpoint(&committed_g2, &checkpoint_a, &generation.report)
        .expect("restore first generation");
    let active_checkpoints: Vec<StructuralCheckpoint> = restored
        .structural_governance_state
        .as_ref()
        .unwrap()
        .structure_checkpoints
        .iter()
        .filter(|checkpoint| checkpoint.status == StructuralCheckpointStatus::Active)
        .cloned()
        .collect();
    assert_eq!(
        active_checkpoints.len(),
        1,
        "回绑后同一时刻至多一个 Active checkpoint"
    );
    assert_eq!(
        active_checkpoints[0].checkpoint_id, checkpoint_a.checkpoint_id,
        "回绑代次即当前 Active"
    );

    // Active 定位后果:finalize 的 review 证据必须写入回绑代次(Active)。
    let g2_completed_turn = HumanGateTurn {
        status: HumanGateTurnStatus::Completed,
        ..g2_turn.clone()
    };
    let g2_completed = store
        .update_human_gate_turn(&restored, g2_completed_turn.clone())
        .expect("terminalize second turn");
    let finalized = store
        .finalize_structural_revision_grant(
            &g2_completed,
            g2_completed_turn,
            "sgr_turn_sg4b_0002",
            Some("review_node_sg4b_0002"),
            Some("scope_sg4b_0002"),
        )
        .expect("finalize targets the rebound active generation");
    let state = finalized.structural_governance_state.as_ref().unwrap();
    let checkpoint_a_after = state
        .structure_checkpoints
        .iter()
        .find(|checkpoint| checkpoint.checkpoint_id == checkpoint_a.checkpoint_id)
        .unwrap();
    assert_eq!(
        checkpoint_a_after.status,
        StructuralCheckpointStatus::Active
    );
    assert_eq!(
        checkpoint_a_after.review_scope.as_deref(),
        Some("scope_sg4b_0002"),
        "review 证据落在 Active(回绑代次)"
    );
    let checkpoint_b_after = state
        .structure_checkpoints
        .iter()
        .find(|checkpoint| checkpoint.checkpoint_id != checkpoint_a.checkpoint_id)
        .unwrap();
    assert_eq!(
        checkpoint_b_after.status,
        StructuralCheckpointStatus::Superseded
    );
    assert_eq!(checkpoint_b_after.review_scope, None);
}
