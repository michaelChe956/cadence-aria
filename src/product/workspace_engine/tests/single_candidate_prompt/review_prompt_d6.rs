// D6（REQ-WSC-06「前轮 finding 状态求交按内容身份」）测试从 review_prompt.rs
// 拆入（large_file_guard 1200 行上限）：previous_round 分区/求交/去重测试 +
// 被本轮 D6 扩展的前轮注入契约测试及其辅助夹具；测试逻辑与断言零改动。

use super::*;

/// REQ-TOP-04 场景 6（F-52 L2 前轮注入）：复评轮 prompt 必须携带上一
/// ReviewerRun 持久化 verdict 的非 advisory findings 结构化身份清单
/// （fingerprint/category/contract_field/required_action）+ 当前候选 ref，
/// 并教学「同一 fingerprint 重现须显式标注 repeated」；首轮（无前轮
/// verdict）不注入该段。
#[test]
fn single_candidate_review_prompt_injects_previous_round_finding_identities() {
    let (_tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        super::super::make_work_item_plan_engine_with_draft_candidate("previous_round_injection");
    // D6 扩展：机械报告注入 code=intent_unexecutable ⇒ 前轮 finding（同内容
    // 身份）在本轮机械复算仍存在，应落待办区（required_action 仍须在场）。
    let (ir_ref, report_ref) = super::persist_verification_artifacts_with_findings(
        &lifecycle,
        &plan_id,
        vec![WorkItemSplitFinding {
            severity: crate::product::models::WorkItemSplitFindingSeverity::Warning,
            code: "intent_unexecutable".to_string(),
            message: "mechanical summary: intent target codebase unavailable".to_string(),
            work_item_ids: vec!["wi-a".to_string()],
            kind: None,
        }],
    );
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("load session");
    record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    record.plan_candidate_ir_ref = Some(ir_ref.clone());
    record.mechanical_report_ref = Some(report_ref);
    write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist session refs");
    engine.session = crate::product::workspace_engine::types::WorkspaceSession::from_record(record);
    engine.active_node_id = Some("reviewer-current".to_string());
    engine.timeline_nodes.push(TimelineNode {
        node_id: "reviewer-current".to_string(),
        node_type: TimelineNodeType::ReviewerRun,
        agent: None,
        stage: WsWorkspaceStage::CrossReview,
        round: Some(2),
        status: TimelineNodeStatus::Active,
        title: "reviewer run".to_string(),
        summary: None,
        started_at: "2026-09-25T00:00:00Z".to_string(),
        completed_at: None,
        duration_ms: None,
        artifact_ref: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: crate::product::models::ProviderName::ClaudeCode,
            reviewer: Some(crate::product::models::ProviderName::KimiCode),
            review_rounds: 2,
            permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        },
        retry: None,
    });

    // 首轮形态（无前轮 verdict）：不注入前轮 findings 段。
    let first_round = engine
        .build_work_item_plan_review_input()
        .expect("first-round review input");
    assert!(
        !first_round
            .prompt
            .contains("Previous Review Round Findings"),
        "the initial round must not carry a previous-round section"
    );

    // 上一轮 ReviewerRun 节点 + durable verdict。
    let previous_finding = ReviewFinding {
        severity: ReviewFindingSeverity::MustFix,
        message: "coverage 缺口：上游未提供 capability".to_string(),
        evidence: "edge wi-a -> wi-b".to_string(),
        required_action: "补齐 capability 声明".to_string(),
        category: Some(ReviewFindingCategory::ContractGap),
        class_hint: None,
        contract_field: Some("plan_preflight.intent_unexecutable".to_string()),
        machine_repair: None,
        owner_question: None,
        owner_options: None,
        disposition: None,
        target_work_item_id: None,
        finding_key: None,
    };
    let previous_verdict = ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "previous round revise".to_string(),
        summary: "previous round revise".to_string(),
        findings: vec![previous_finding],
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    };
    let previous_node_id = "reviewer-previous".to_string();
    engine.timeline_nodes.insert(
        0,
        TimelineNode {
            node_id: previous_node_id.clone(),
            node_type: TimelineNodeType::ReviewerRun,
            agent: None,
            stage: WsWorkspaceStage::CrossReview,
            round: Some(1),
            status: TimelineNodeStatus::Completed,
            title: "reviewer run".to_string(),
            summary: None,
            started_at: "2026-09-25T00:00:00Z".to_string(),
            completed_at: Some("2026-09-25T00:01:00Z".to_string()),
            duration_ms: None,
            artifact_ref: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: crate::product::models::ProviderName::ClaudeCode,
                reviewer: Some(crate::product::models::ProviderName::KimiCode),
                review_rounds: 2,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            retry: None,
        },
    );
    lifecycle
        .save_node_detail(
            &engine.session().session_id,
            &previous_node_id.clone(),
            &crate::product::models::NodeDetail {
                node_id: previous_node_id,
                session_id: engine.session().session_id.clone(),
                node_type: TimelineNodeType::ReviewerRun,
                status: TimelineNodeStatus::Completed,
                agent_role: None,
                provider: None,
                prompt: None,
                messages: Vec::new(),
                streaming_content: String::new(),
                execution_events: Vec::new(),
                permission_events: Vec::new(),
                verdict: Some(serde_json::to_value(&previous_verdict).expect("verdict json")),
                artifact_ref: None,
                is_revision: false,
                revision_feedback: None,
                base_artifact_ref: None,
                started_at: "2026-09-25T00:00:00Z".to_string(),
                ended_at: None,
            },
        )
        .expect("persist previous reviewer verdict");

    let replay_round = engine
        .build_work_item_plan_review_input()
        .expect("replay review input");
    let (fingerprint, unstable) = FindingFingerprint::identity_for_finding(
        Some(ReviewFindingCategory::ContractGap),
        FindingClass::Repairable,
        "coverage 缺口：上游未提供 capability",
        Some("plan_preflight.intent_unexecutable"),
    );
    assert!(!unstable, "fixture must sit in the stable identity domain");
    assert!(
        replay_round
            .prompt
            .contains("Previous Review Round Findings")
    );
    assert!(
        replay_round.prompt.contains(fingerprint.0.as_str()),
        "prompt must carry the previous-round finding fingerprint identity"
    );
    assert!(replay_round.prompt.contains("contract_gap"));
    assert!(
        replay_round
            .prompt
            .contains("plan_preflight.intent_unexecutable")
    );
    assert!(replay_round.prompt.contains("补齐 capability 声明"));
    assert!(
        replay_round.prompt.contains(&ir_ref),
        "prompt must name the current plan_candidate_ir_ref under review"
    );
    assert!(
        replay_round.prompt.contains("repeated"),
        "prompt must teach explicit repeated marking for recurring fingerprints"
    );
    // D6：前轮清单按内容身份求交分区注入——同码仍存在 ⇒ 待办区在场。
    assert!(
        replay_round.prompt.contains("unresolved_findings"),
        "partitioned previous-round payload must carry the unresolved zone"
    );
    assert!(
        replay_round.prompt.contains("resolved_findings"),
        "partitioned previous-round payload must carry the resolved zone"
    );
    assert!(
        replay_round
            .prompt
            .contains("previous_round_repetition_discipline"),
        "discipline tag must stay attached to the partitioned payload"
    );
}

// ===================== D6：前轮 finding 状态复算（内容身份求交 + 去重 + 分区注入） =====================

struct PreviousRoundReplay {
    // TempDir 必须存活到断言结束：lifecycle 全部落盘在它的 .aria 下。
    _tmp: tempfile::TempDir,
    engine: crate::product::workspace_engine::WorkspaceEngine,
    ir_ref: String,
    #[allow(dead_code)]
    report_ref: String,
}

fn mechanical_finding(code: &str, message: &str) -> WorkItemSplitFinding {
    WorkItemSplitFinding {
        severity: crate::product::models::WorkItemSplitFindingSeverity::Warning,
        code: code.to_string(),
        message: message.to_string(),
        work_item_ids: vec!["wi-a".to_string()],
        kind: None,
    }
}

fn previous_finding(
    message: &str,
    contract_field: Option<&str>,
    required_action: &str,
) -> ReviewFinding {
    ReviewFinding {
        severity: ReviewFindingSeverity::MustFix,
        message: message.to_string(),
        evidence: "edge wi-a -> wi-b".to_string(),
        required_action: required_action.to_string(),
        category: Some(ReviewFindingCategory::ContractGap),
        class_hint: None,
        contract_field: contract_field.map(str::to_string),
        machine_repair: None,
        owner_question: None,
        owner_options: None,
        disposition: None,
        target_work_item_id: None,
        finding_key: None,
    }
}

/// 复评轮夹具：当前候选机械报告（findings 可注入）+ 上一轮 ReviewerRun
/// durable verdict；沿既有 `single_candidate_review_prompt_injects_previous_
/// round_finding_identities` 的搭建形态。
fn previous_round_replay(
    entity: &str,
    mechanical_findings: Vec<WorkItemSplitFinding>,
    previous_findings: Vec<ReviewFinding>,
) -> PreviousRoundReplay {
    let (tmp, _checkpoint_store, lifecycle, plan_id, mut engine) =
        super::super::make_work_item_plan_engine_with_draft_candidate(entity);
    let (ir_ref, report_ref) = super::persist_verification_artifacts_with_findings(
        &lifecycle,
        &plan_id,
        mechanical_findings,
    );
    let mut record = lifecycle
        .get_workspace_session(&engine.session().session_id)
        .expect("load session");
    record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
    record.plan_candidate_ir_ref = Some(ir_ref.clone());
    record.mechanical_report_ref = Some(report_ref.clone());
    write_json(
        &lifecycle
            .app_paths()
            .issue_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id)),
        &record,
    )
    .expect("persist session refs");
    engine.session = crate::product::workspace_engine::types::WorkspaceSession::from_record(record);
    engine.active_node_id = Some("reviewer-current".to_string());
    engine.timeline_nodes.push(TimelineNode {
        node_id: "reviewer-current".to_string(),
        node_type: TimelineNodeType::ReviewerRun,
        agent: None,
        stage: WsWorkspaceStage::CrossReview,
        round: Some(2),
        status: TimelineNodeStatus::Active,
        title: "reviewer run".to_string(),
        summary: None,
        started_at: "2026-09-25T00:00:00Z".to_string(),
        completed_at: None,
        duration_ms: None,
        artifact_ref: None,
        provider_config_snapshot: ProviderConfigSnapshot {
            author: crate::product::models::ProviderName::ClaudeCode,
            reviewer: Some(crate::product::models::ProviderName::KimiCode),
            review_rounds: 2,
            permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        },
        retry: None,
    });

    let previous_verdict = ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "previous round revise".to_string(),
        summary: "previous round revise".to_string(),
        findings: previous_findings,
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    };
    let previous_node_id = "reviewer-previous".to_string();
    engine.timeline_nodes.insert(
        0,
        TimelineNode {
            node_id: previous_node_id.clone(),
            node_type: TimelineNodeType::ReviewerRun,
            agent: None,
            stage: WsWorkspaceStage::CrossReview,
            round: Some(1),
            status: TimelineNodeStatus::Completed,
            title: "reviewer run".to_string(),
            summary: None,
            started_at: "2026-09-25T00:00:00Z".to_string(),
            completed_at: Some("2026-09-25T00:01:00Z".to_string()),
            duration_ms: None,
            artifact_ref: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: crate::product::models::ProviderName::ClaudeCode,
                reviewer: Some(crate::product::models::ProviderName::KimiCode),
                review_rounds: 2,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            retry: None,
        },
    );
    lifecycle
        .save_node_detail(
            &engine.session().session_id,
            &previous_node_id.clone(),
            &crate::product::models::NodeDetail {
                node_id: previous_node_id,
                session_id: engine.session().session_id.clone(),
                node_type: TimelineNodeType::ReviewerRun,
                status: TimelineNodeStatus::Completed,
                agent_role: None,
                provider: None,
                prompt: None,
                messages: Vec::new(),
                streaming_content: String::new(),
                execution_events: Vec::new(),
                permission_events: Vec::new(),
                verdict: Some(serde_json::to_value(&previous_verdict).expect("verdict json")),
                artifact_ref: None,
                is_revision: false,
                revision_feedback: None,
                base_artifact_ref: None,
                started_at: "2026-09-25T00:00:00Z".to_string(),
                ended_at: None,
            },
        )
        .expect("persist previous reviewer verdict");
    PreviousRoundReplay {
        _tmp: tmp,
        engine,
        ir_ref,
        report_ref,
    }
}

/// 截取 `### Previous Review Round Findings` 段的 JSON 载荷（纪律段之前）。
fn previous_round_payload(text: &str) -> serde_json::Value {
    let section = text
        .split("### Previous Review Round Findings")
        .nth(1)
        .expect("text must carry the previous-round findings section");
    let discipline = section
        .find("[previous_round_repetition_discipline]")
        .expect("section must close with the repetition discipline tag");
    serde_json::from_str(section[..discipline].trim())
        .expect("previous-round payload must be valid JSON")
}

fn zone<'a>(payload: &'a serde_json::Value, key: &str) -> &'a Vec<serde_json::Value> {
    payload
        .get(key)
        .unwrap_or_else(|| panic!("payload must carry the `{key}` zone: {payload}"))
        .as_array()
        .unwrap_or_else(|| panic!("`{key}` must be a list: {payload}"))
}

/// D6：本轮机械报告仍存在同内容身份 ⇒ 前轮 finding 落待办区（附
/// required_action 与 contract_field），不落已关闭区。
#[test]
fn previous_round_section_marks_unresolved_finding_as_pending() {
    let replay = previous_round_replay(
        "previous_round_unresolved",
        vec![mechanical_finding(
            "intent_unexecutable",
            "mechanical summary: intent target codebase unavailable",
        )],
        vec![previous_finding(
            "intent 不可执行：目标代码库 ID 未授权",
            Some("plan_preflight.intent_unexecutable"),
            "把 intent_target_codebase 改为已授权的逻辑代码库 ID",
        )],
    );
    let prompt = replay
        .engine
        .build_work_item_plan_review_input()
        .expect("replay review input")
        .prompt;
    let payload = previous_round_payload(&prompt);
    let unresolved = zone(&payload, "unresolved_findings");
    assert_eq!(unresolved.len(), 1, "{payload}");
    assert_eq!(unresolved[0]["content_identity"], "intent_unexecutable");
    assert_eq!(
        unresolved[0]["contract_field"],
        "plan_preflight.intent_unexecutable"
    );
    assert_eq!(
        unresolved[0]["required_action"],
        "把 intent_target_codebase 改为已授权的逻辑代码库 ID"
    );
    assert_eq!(payload["mechanical_report_available"], true, "{payload}");
    assert!(
        zone(&payload, "resolved_findings").is_empty(),
        "仍存在的 finding 不得进已关闭区: {payload}"
    );
}

/// D6：本轮机械报告已无该内容身份 ⇒ 前轮 finding 落已关闭区（附
/// resolved_by），且该区不得携带 required_action（不得要求作者再改）。
#[test]
fn previous_round_section_marks_resolved_finding_as_closed() {
    let replay = previous_round_replay(
        "previous_round_resolved",
        Vec::new(),
        vec![previous_finding(
            "intent 不可执行：目标代码库 ID 未授权",
            Some("plan_preflight.intent_unexecutable"),
            "把 intent_target_codebase 改为已授权的逻辑代码库 ID",
        )],
    );
    let prompt = replay
        .engine
        .build_work_item_plan_review_input()
        .expect("replay review input")
        .prompt;
    let payload = previous_round_payload(&prompt);
    let resolved = zone(&payload, "resolved_findings");
    assert_eq!(resolved.len(), 1, "{payload}");
    assert_eq!(resolved[0]["content_identity"], "intent_unexecutable");
    assert_eq!(
        resolved[0]["resolved_by"].as_str(),
        Some(replay.ir_ref.as_str()),
        "已关闭依据必须是当前候选 revision 标识: {payload}"
    );
    assert!(
        resolved[0].get("required_action").is_none(),
        "已关闭区条目 MUST NOT 携带可执行修改要求: {payload}"
    );
    assert!(
        zone(&payload, "unresolved_findings").is_empty(),
        "已消失的 finding 不得进待办区: {payload}"
    );
    assert!(
        !prompt.contains("把 intent_target_codebase 改为已授权的逻辑代码库 ID"),
        "已关闭 finding 的 required_action 不得以任何形式回灌 prompt"
    );
}

/// D6：前轮多条同内容身份（同一 preflight 码对多个 WI 各登记一次）⇒ 只保留
/// 一条，重复条目折叠为计数。
#[test]
fn previous_round_section_deduplicates_same_content_identity() {
    let previous = (1..=4)
        .map(|index| {
            previous_finding(
                &format!("WI-00{index} intent 不可执行：目标代码库 ID 未授权"),
                Some("plan_preflight.intent_unexecutable"),
                "把 intent_target_codebase 改为已授权的逻辑代码库 ID",
            )
        })
        .collect();
    let replay = previous_round_replay(
        "previous_round_dedup",
        vec![mechanical_finding(
            "intent_unexecutable",
            "mechanical summary",
        )],
        previous,
    );
    let prompt = replay
        .engine
        .build_work_item_plan_review_input()
        .expect("replay review input")
        .prompt;
    let payload = previous_round_payload(&prompt);
    let unresolved = zone(&payload, "unresolved_findings");
    assert_eq!(
        unresolved.len(),
        1,
        "同一内容身份只保留首条，不得以多条重复条目呈现: {payload}"
    );
    assert_eq!(
        payload["duplicate_count"].as_u64(),
        Some(3),
        "4 条同身份前轮 finding ⇒ 1 条 + 折叠计数 3: {payload}"
    );
}

/// D6：求交键是内容身份（contract_field vs 机械报告 code），不是含 message
/// 的指纹——机械报告 message 与评审 message 天然不同。A：评审 message 带
/// `[code]` 前缀（≠ 机械 message）但同码 ⇒ 仍判待办；B：评审 message 与
/// 某机械 finding message 逐字相同但不同码 ⇒ 仍判已关闭。按 message/指纹
/// 求交的实现两条都会判错。
#[test]
fn previous_round_section_resolves_by_content_identity_not_fingerprint() {
    let replay = previous_round_replay(
        "previous_round_identity_not_fingerprint",
        vec![
            mechanical_finding("intent_unexecutable", "xxx"),
            mechanical_finding("unrelated_code", "yyy"),
        ],
        vec![
            previous_finding(
                "[intent_unexecutable] xxx",
                Some("plan_preflight.intent_unexecutable"),
                "修复 A",
            ),
            previous_finding("yyy", Some("plan_preflight.other_code"), "修复 B"),
        ],
    );
    let prompt = replay
        .engine
        .build_work_item_plan_review_input()
        .expect("replay review input")
        .prompt;
    let payload = previous_round_payload(&prompt);
    let unresolved = zone(&payload, "unresolved_findings");
    let resolved = zone(&payload, "resolved_findings");
    assert_eq!(
        unresolved.len(),
        1,
        "message 不同但内容身份相同 ⇒ 仍须判待办: {payload}"
    );
    assert_eq!(unresolved[0]["content_identity"], "intent_unexecutable");
    assert_eq!(
        resolved.len(),
        1,
        "message 逐字相同但内容身份不同 ⇒ 仍须判已关闭: {payload}"
    );
    assert_eq!(resolved[0]["content_identity"], "other_code");
}

/// D6 降级分支一：`contract_field` 缺失或无 `plan_preflight.` 前缀 ⇒ 不可机械
/// 判定——既不进待办也不进已关闭（不得当作已解决）。
#[test]
fn previous_round_section_skips_non_mechanically_judgeable_findings() {
    let replay = previous_round_replay(
        "previous_round_non_mechanical",
        vec![mechanical_finding(
            "intent_unexecutable",
            "mechanical summary",
        )],
        vec![
            previous_finding(
                "coverage 缺口：上游未提供 capability",
                Some("WI-001.output_contracts[CT-001].capabilities"),
                "补齐 capability 声明",
            ),
            previous_finding("无 contract_field 的 finding", None, "修复缺字段项"),
            previous_finding(
                "intent 不可执行：目标代码库 ID 未授权",
                Some("plan_preflight.intent_unexecutable"),
                "把 intent_target_codebase 改为已授权的逻辑代码库 ID",
            ),
        ],
    );
    let prompt = replay
        .engine
        .build_work_item_plan_review_input()
        .expect("replay review input")
        .prompt;
    let payload = previous_round_payload(&prompt);
    let unresolved = zone(&payload, "unresolved_findings");
    let resolved = zone(&payload, "resolved_findings");
    assert_eq!(
        unresolved.len(),
        1,
        "只有带 plan_preflight. 前缀的可机械判定项能进待办: {payload}"
    );
    assert_eq!(unresolved[0]["content_identity"], "intent_unexecutable");
    assert!(
        resolved.is_empty(),
        "不可机械判定的 finding 不得被当作已解决: {payload}"
    );
}

/// D6 降级分支二：机械报告 ref 为 None 或装载失败 ⇒ 如实降级——
/// mechanical_report_available=false，前轮 finding 既不进待办也不进已关闭；
/// 不得 panic、不得当作「全部已解决」。
#[test]
fn previous_round_section_degrades_when_mechanical_report_unavailable() {
    let mut replay = previous_round_replay(
        "previous_round_report_unavailable",
        Vec::new(),
        vec![previous_finding(
            "intent 不可执行：目标代码库 ID 未授权",
            Some("plan_preflight.intent_unexecutable"),
            "把 intent_target_codebase 改为已授权的逻辑代码库 ID",
        )],
    );
    let section_for = |engine: &crate::product::workspace_engine::WorkspaceEngine| {
        crate::product::workspace_engine::prompts::single_candidate_previous_round_findings_section(
            engine,
            &replay.ir_ref,
        )
        .expect("降级必须如实注入清单段（不 panic、不清空清单）")
    };
    for unavailable_ref in [None, Some("mechanical_report/does-not-resolve".to_string())] {
        replay.engine.session.mechanical_report_ref = unavailable_ref;
        let section = section_for(&replay.engine);
        let payload = previous_round_payload(&section);
        assert_eq!(
            payload["mechanical_report_available"], false,
            "机械报告不可得必须显式标注: {payload}"
        );
        assert!(
            zone(&payload, "unresolved_findings").is_empty(),
            "机械报告不可得 ⇒ 不得把前轮 finding 判为待办: {payload}"
        );
        assert!(
            zone(&payload, "resolved_findings").is_empty(),
            "机械报告不可得 ⇒ 不得静默当作「全部已解决」: {payload}"
        );
    }
}
