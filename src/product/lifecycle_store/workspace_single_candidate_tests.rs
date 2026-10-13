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
