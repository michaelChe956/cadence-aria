use std::collections::BTreeSet;

use super::{
    FindingClass, FindingFingerprint, ReviewCycleState, ReviewInvocationScope, RunBudgets,
    RunHistory,
};

#[test]
fn run_history_defaults_missing_fields_and_rejects_unknown_fields() {
    let recovered = serde_json::from_value::<RunHistory>(serde_json::json!({})).unwrap();
    assert_eq!(recovered, RunHistory::default());

    let partial = serde_json::from_value::<RunHistory>(serde_json::json!({
        "repairs_used": 1,
        "seen_fingerprints": [],
    }))
    .unwrap();
    assert_eq!(partial.repairs_used, 1);
    assert_eq!(partial.manual_repairs_used, 0);
    assert_eq!(partial.transitions_used, 0);
    assert_eq!(partial.initial_review_count, 0);
    assert_eq!(partial.verification_review_count, 0);
    assert!(partial.review_cycles.is_empty());

    let cycles = serde_json::from_value::<RunHistory>(serde_json::json!({
        "review_cycles": {
            "draft:outline_001": {
                "initial_count": 1,
                "verification_count": 1
            }
        }
    }))
    .unwrap();
    assert_eq!(
        cycles.review_cycles["draft:outline_001"],
        ReviewCycleState {
            repairs_used: 0,
            initial_count: 1,
            verification_count: 1,
            original_fingerprints: std::collections::BTreeSet::new(),
        },
        "old cycle JSON without repairs_used must recover a zero cycle budget"
    );

    assert!(
        serde_json::from_value::<RunHistory>(serde_json::json!({
            "unexpected": true,
        }))
        .is_err()
    );
}

#[test]
fn run_budgets_default_to_fixed_policy_limits() {
    assert_eq!(
        RunBudgets::default(),
        RunBudgets {
            max_repairs: 1,
            max_transitions: 12,
            max_manual_repairs: 3,
        }
    );
}

#[test]
fn review_invocation_scope_digest_is_canonical_and_validated() {
    let first = ReviewInvocationScope::initial("revision-001");
    let second = ReviewInvocationScope::initial("revision-001");
    assert_eq!(first, second);
    first.validate_digest().unwrap();

    let original_fingerprints = BTreeSet::from([
        FindingFingerprint::for_finding(None, FindingClass::Repairable, "first", Some("field_one")),
        FindingFingerprint::for_finding(
            None,
            FindingClass::Repairable,
            "second",
            Some("field_two"),
        ),
    ]);
    let verification = ReviewInvocationScope::verification(
        original_fingerprints,
        "revision-002",
        "mechanical-report-001",
        None,
    );
    verification.validate_digest().unwrap();

    let empty_verification = ReviewInvocationScope::verification(
        BTreeSet::new(),
        "revision-002",
        "mechanical-report-001",
        None,
    );
    assert_eq!(
        serde_json::to_value(&empty_verification).unwrap()["phase"],
        "verification"
    );
    empty_verification.validate_digest().unwrap();

    let serialized = serde_json::to_value(&verification).unwrap();
    assert_eq!(serialized["phase"], "verification");
    assert!(serialized.get("initial_revision_id").is_none());
    assert!(
        serde_json::from_value::<ReviewInvocationScope>(serialized)
            .unwrap()
            .validate_digest()
            .is_ok()
    );
}

#[test]
fn review_invocation_scope_rejects_invalid_digest_shape_or_recalculation() {
    let scope = ReviewInvocationScope::initial("revision-001");

    let mut invalid_initial_shape = serde_json::to_value(&scope).unwrap();
    invalid_initial_shape["mechanical_report_ref"] = serde_json::json!("not applicable");
    assert!(serde_json::from_value::<ReviewInvocationScope>(invalid_initial_shape).is_err());

    for digest in [
        "not_a_scope_digest",
        "review_scope_v1:too_short",
        "review_scope_v1:A0d52e876560346f9c9fd0777ad620c166699c1d43eb6f42b0134e7506d4b2f8",
        "review_scope_v1:g0d52e876560346f9c9fd0777ad620c166699c1d43eb6f42b0134e7506d4b2f8",
        "review_scope_v1:0000000000000000000000000000000000000000000000000000000000000000",
    ] {
        let mut invalid = serde_json::to_value(&scope).unwrap();
        invalid["scope_digest"] = serde_json::json!(digest);
        assert!(
            serde_json::from_value::<ReviewInvocationScope>(invalid).is_err(),
            "scope digest {digest:?} should be rejected"
        );
    }
}

// ---- SC 生成教学重驱耐久窗口模型（REQ-WSC-28B 模型面，Task 2）----

use std::collections::BTreeSet as WindowKeySet;

use super::{
    ProviderStartLedgerEntry, RunPolicy, SC_TEACHING_APPROVAL_RULE_VERSION,
    ScGenerationTeachingWindow, ScTeachingAttemptLifecycle, ScTeachingAttemptOutcome,
    ScTeachingAttemptState, ScTeachingWindowCloseReason, ScTeachingWindowRequest,
    ScTeachingWindowStatus, WorkItemPlanFlowKind, next_sc_author_launch_ordinal,
    non_teaching_provider_start_count, sc_teaching_frozen_input_digest,
};
use crate::product::models::{
    ProviderName, SingleCandidatePhase, WorkspaceSessionRecord, WorkspaceSessionStatus,
    WorkspaceType,
};
use crate::product::planning_failure::{PlanningFailureKind, StructuralAuthorizationKind};

fn digest64(seed: &str) -> String {
    format!("{seed:0<64}")
}

fn full_sc_teaching_request() -> ScTeachingWindowRequest {
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

fn full_sc_teaching_window() -> ScGenerationTeachingWindow {
    let request = full_sc_teaching_request();
    ScGenerationTeachingWindow {
        generation_seq: 0,
        authorization_kind: StructuralAuthorizationKind::GenerationRun,
        original_session_id: "s1".to_string(),
        original_run_phase: SingleCandidatePhase::Generate,
        established_at: "2026-11-13T00:00:00Z".to_string(),
        frozen_input_digest: sc_teaching_frozen_input_digest(&request),
        frozen_execution_policy_digest: Some(digest64("8b")),
        repair_once_approved: true,
        approval_rule_version: SC_TEACHING_APPROVAL_RULE_VERSION.to_string(),
        approval_checked_against: "single_candidate_author:s1:0".to_string(),
        max_attempts: 2,
        attempts: vec![
            ScTeachingAttemptState {
                attempt_no: 1,
                attempt_key: ScGenerationTeachingWindow::attempt_key("s1", 0, 1),
                launch_ledger_key: Some("single_candidate_author:s1:0".to_string()),
                lifecycle: ScTeachingAttemptLifecycle::Terminal {
                    outcome: ScTeachingAttemptOutcome::FailedTerminal,
                },
            },
            ScTeachingAttemptState {
                attempt_no: 2,
                attempt_key: ScGenerationTeachingWindow::attempt_key("s1", 0, 2),
                launch_ledger_key: Some("single_candidate_author:s1:2".to_string()),
                lifecycle: ScTeachingAttemptLifecycle::ActualStarted,
            },
        ],
        status: ScTeachingWindowStatus::Closed {
            reason: ScTeachingWindowCloseReason::Delivered,
        },
        chain_non_teaching_base: 1,
        non_teaching_total_at_close: Some(2),
        request,
    }
}

fn teaching_ledger_entry(key: &str, attempt_key: Option<&str>) -> ProviderStartLedgerEntry {
    ProviderStartLedgerEntry {
        provider_start_idempotency_key: key.to_string(),
        started: true,
        provider: None,
        started_at: None,
        provider_attempt_key: attempt_key.map(ToString::to_string),
    }
}

fn sc_teaching_test_record(
    ledger: Vec<ProviderStartLedgerEntry>,
    windows: Vec<ScGenerationTeachingWindow>,
) -> WorkspaceSessionRecord {
    WorkspaceSessionRecord {
        id: "s1".to_string(),
        project_id: "p1".to_string(),
        issue_id: "i1".to_string(),
        entity_id: "e1".to_string(),
        workspace_type: WorkspaceType::WorkItemPlan,
        status: WorkspaceSessionStatus::Open,
        author_provider: ProviderName::Codex,
        reviewer_provider: None,
        review_rounds: 0,
        permission_modes: Default::default(),
        provisional_reviewer_provider: None,
        reviewer_enabled_at_start: None,
        superpowers_enabled: false,
        openspec_enabled: false,
        flow_kind: WorkItemPlanFlowKind::SingleCandidate,
        run_policy: RunPolicy::Interactive,
        run_history: Default::default(),
        review_invocation_scope: None,
        human_gate_snapshot: None,
        repair_reservation: None,
        human_gate_reservation: None,
        author_rewrite_budget: None,
        policy_diagnostics: Vec::new(),
        provider_start_ledger: ledger,
        single_candidate_phase: Some(SingleCandidatePhase::Generate),
        work_item_plan_source_revision_ref: None,
        plan_candidate_ir_ref: None,
        mechanical_report_ref: None,
        publication_provenance_ref: None,
        approval_attempt_id: None,
        approved_at: None,
        compile_reservation: None,
        sc_recovery_claim: None,
        work_item_runtime_binding: None,
        work_item_child_binding: None,
        provider_conversations: Vec::new(),
        logical_launch_fingerprints: Vec::new(),
        superseded_native_sessions: Vec::new(),
        lc_resume_recovery: None,
        structural_governance_state: None,
        review_identity_id: String::new(),
        messages: Vec::new(),
        created_at: "2026-11-13T00:00:00Z".to_string(),
        updated_at: "2026-11-13T00:00:00Z".to_string(),
        sc_generation_teaching_windows: windows,
    }
}

#[test]
fn attempt_key_format_is_stable() {
    // 键式 sc_generation:{session}:{gen}:attempt:{n} 是后续所有 Task 的
    // 契约（durable 身份键），格式一旦漂移会破坏窗口定位与账本关联。
    assert_eq!(
        ScGenerationTeachingWindow::attempt_key("s1", 0, 1),
        "sc_generation:s1:0:attempt:1"
    );
    assert_eq!(
        ScGenerationTeachingWindow::attempt_key("s1", 0, 2),
        "sc_generation:s1:0:attempt:2"
    );
    assert_eq!(
        ScGenerationTeachingWindow::attempt_key("s1", 1, 1),
        "sc_generation:s1:1:attempt:1"
    );
}

#[test]
fn window_roundtrip_restores_all_claim_binding_facts() {
    let window = full_sc_teaching_window();
    let value = serde_json::to_value(&window).unwrap();

    // 序列化键集合与字段清单一一对应（tasks 1.1 验证口径）：新增/删除/
    // 改名字段都会被键集合断言捕获。
    let window_keys: WindowKeySet<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        window_keys,
        WindowKeySet::from([
            "generation_seq",
            "authorization_kind",
            "original_session_id",
            "original_run_phase",
            "established_at",
            "request",
            "frozen_input_digest",
            "frozen_execution_policy_digest",
            "repair_once_approved",
            "approval_rule_version",
            "approval_checked_against",
            "max_attempts",
            "attempts",
            "status",
            "chain_non_teaching_base",
            "non_teaching_total_at_close",
        ])
    );
    let request_keys: WindowKeySet<&str> = value["request"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        request_keys,
        WindowKeySet::from([
            "provider_path_admissible",
            "failure_source_ref",
            "failure_source_sha256",
            "failure_typed_kinds",
            "business_request_digest",
            "material_digests",
            "context_digest",
            "allowed_edit_closure",
            "instruction_digest",
            "prompt_ref",
            "prompt_digest",
        ])
    );

    // 往返后逐字段相等（含两 attempt 的 claim 绑定事实全量恢复）。
    let restored: ScGenerationTeachingWindow = serde_json::from_value(value).unwrap();
    assert_eq!(restored, window);
}

#[test]
fn window_attempt_count_decoupled_from_ledger_length() {
    let mut window = full_sc_teaching_window();
    window.status = ScTeachingWindowStatus::Active;
    window.attempts.truncate(1);

    // 5 条 ledger（含非前缀与教学关联条目）不影响窗口 attempt 语义：
    // claimable 与 attempts 只由窗口自身字段决定（tasks 1.2 验证口径）。
    let record = sc_teaching_test_record(
        vec![
            teaching_ledger_entry("human_gate:turn_0001:attempt:1", None),
            teaching_ledger_entry("single_candidate_author:s1:0", None),
            teaching_ledger_entry("single_candidate_author:s1:1", None),
            teaching_ledger_entry(
                "single_candidate_author:s1:2",
                Some("sc_generation:s1:0:attempt:2"),
            ),
            teaching_ledger_entry("compile:child:0001", None),
        ],
        vec![window.clone()],
    );
    assert_eq!(record.provider_start_ledger.len(), 5);
    assert_eq!(
        record.sc_generation_teaching_windows[0].claimable_attempt_no(),
        Some(2)
    );
    assert_eq!(record.sc_generation_teaching_windows[0].attempts.len(), 1);

    // 键派生同样与 ledger 长度解耦：非教学计数只认前缀且无 attempt 关联
    // 的条目；下一序号取前缀最大数字后缀 +1，而非 ledger.len()。
    assert_eq!(non_teaching_provider_start_count(&record), 2);
    assert_eq!(next_sc_author_launch_ordinal(&record), 3);
}

#[test]
fn default_record_has_no_window_and_zero_extra() {
    let record = sc_teaching_test_record(Vec::new(), Vec::new());

    // 无窗口 ⇒ 无从 claim、无任何批准（不从计数为零推断存在机会）。
    assert!(record.sc_generation_teaching_windows.is_empty());
    assert!(record.sc_generation_teaching_windows.last().is_none());
    assert!(
        record.sc_generation_teaching_windows.iter().all(|window| {
            window.claimable_attempt_no().is_none() && !window.repair_once_approved
        })
    );
    assert_eq!(non_teaching_provider_start_count(&record), 0);
    assert_eq!(next_sc_author_launch_ordinal(&record), 0);

    // 旧 JSON（无该字段）读入 ⇒ 空；缺省序列化不写键（旧 wire 零变化、
    // 不倒填授权或启动证据）。
    let wire = serde_json::to_value(&record).unwrap();
    assert!(wire.get("sc_generation_teaching_windows").is_none());
    let reloaded: WorkspaceSessionRecord = serde_json::from_value(wire).unwrap();
    assert!(reloaded.sc_generation_teaching_windows.is_empty());
}

#[test]
fn frozen_input_drift_detected() {
    let request = full_sc_teaching_request();
    let window = full_sc_teaching_window();
    window.verify_frozen_inputs(&request).unwrap();

    let drifted_inputs = [
        ("business_request_digest", digest64("bb")),
        ("context_digest", digest64("cc")),
        ("failure_source_sha256", digest64("dd")),
        ("prompt_ref", "other_prompt_builder".to_string()),
        ("prompt_digest", digest64("ee")),
    ];
    for (field, replacement) in drifted_inputs {
        let mut drifted = request.clone();
        match field {
            "business_request_digest" => drifted.business_request_digest = replacement,
            "context_digest" => drifted.context_digest = replacement,
            "failure_source_sha256" => drifted.failure_source_sha256 = replacement,
            "prompt_ref" => drifted.prompt_ref = replacement,
            "prompt_digest" => drifted.prompt_digest = replacement,
            _ => unreachable!(),
        }
        assert!(
            window.verify_frozen_inputs(&drifted).is_err(),
            "{field} 漂移必须被检测到"
        );
    }

    // 材料摘要集合漂移（增删任一材料）同样触发冻结输入校验失败。
    let mut drifted_materials = request.clone();
    drifted_materials.material_digests = vec![digest64("3c")];
    assert!(window.verify_frozen_inputs(&drifted_materials).is_err());
}

#[test]
fn legacy_ledger_entry_json_still_parses() {
    // 旧 durable JSON（无 provider_attempt_key 键）必须原样可解析为 None。
    let legacy: ProviderStartLedgerEntry = serde_json::from_str(
        r#"{"provider_start_idempotency_key":"single_candidate_author:s1:0","started":true}"#,
    )
    .unwrap();
    assert_eq!(legacy.provider_attempt_key, None);

    // 含键的新格式解析为 Some（教学关联可见）。
    let teaching: ProviderStartLedgerEntry = serde_json::from_str(
        r#"{"provider_start_idempotency_key":"single_candidate_author:s1:1","started":false,"provider_attempt_key":"sc_generation:s1:0:attempt:2"}"#,
    )
    .unwrap();
    assert_eq!(
        teaching.provider_attempt_key.as_deref(),
        Some("sc_generation:s1:0:attempt:2")
    );

    // None 序列化不写键：旧 wire 输出零变化。
    let wire = serde_json::to_value(&legacy).unwrap();
    assert!(wire.get("provider_attempt_key").is_none());
    assert_eq!(
        wire["provider_start_idempotency_key"],
        "single_candidate_author:s1:0"
    );
    assert_eq!(wire["started"], true);
}
