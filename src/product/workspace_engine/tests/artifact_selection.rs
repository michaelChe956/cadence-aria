//! Task 2（change `artifact-candidate-selection`）：workspace-aware 唯一 gate-passing
//! 候选选择器契约（REQ-ACS-01）。
//!
//! 用例覆盖：唯一通过选中、零通过逐候选保留阻断原因、双通过歧义、三候选（1 示意 +
//! 2 合法）歧义、候选全失败时禁止回落候选外 heading、无任何 marker 时 legacy 单候选
//! fallback，以及 F-46 真实 durable 原文实跑（候选 2 个、`:200-203` 拒、`:270-361`
//! 唯一通过）。
use super::provider_drive::artifact_retry::{
    artifact_failure_reasons_with_diagnostic, artifact_selection_diagnostic_event,
};
use super::*;

/// F-46 真实 provider 原文（durable `workspace_session_0007` 的最后一条 assistant
/// `content`，19,921 字符）。`.aria/` 在版本库外，故以 fixture 固化，保证任何 checkout
/// 下都能复核同一输入、同一行号、同一 hash。
const F46_RAW_OUTPUT: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/product/workspace_engine/tests/fixtures/f46_story_artifact_raw_output.txt"
));

/// 被拒候选形态：缺全部二级 heading、`[REQ-*]`、`[AC-*]` 与 source id（示意 block）。
const DRAFT_STORY: &str = "# 示意标题\n...\n";

fn fenced(markdown: &str) -> String {
    format!("```artifact\n{markdown}```\n")
}

#[test]
fn select_unique_passing_candidate_after_rejected_draft_block() {
    let compliant =
        complete_story_artifact("系统支持 provider 检查。", "用户能看到 provider 状态。");
    let output = format!(
        "前言\n{}中间 <thinking>过程</thinking> 文本\n{}尾随说明\n",
        fenced(DRAFT_STORY),
        fenced(&compliant),
    );

    let selection = select_workspace_artifact(&output, WorkspaceType::Story);

    assert_eq!(selection.verdict, SelectionVerdict::Unique);
    assert!(!selection.used_legacy_fallback);
    assert_eq!(selection.candidates.len(), 2);
    assert_eq!(
        (
            selection.candidates[0].opening_line,
            selection.candidates[0].closing_line
        ),
        (2, 5)
    );
    assert!(!selection.candidates[0].passed);
    assert!(
        selection.candidates[0]
            .blocking_reasons
            .iter()
            .any(|reason| reason.contains("缺少 heading"))
    );
    assert!(selection.candidates[1].passed);
    assert!(selection.candidates[1].blocking_reasons.is_empty());
    // 过程文本与示意块都不进入产物：选中正文逐字等于第二候选正文。
    assert_eq!(
        selection.selected_markdown.as_deref(),
        Some(compliant.as_str())
    );
}

#[test]
fn select_zero_passing_keeps_blocking_reasons_per_candidate() {
    let output = format!(
        "{}块间过程说明\n{}",
        fenced(DRAFT_STORY),
        fenced("# 另一个示意标题\n没有必需 heading\n"),
    );

    let selection = select_workspace_artifact(&output, WorkspaceType::Story);

    assert_eq!(selection.verdict, SelectionVerdict::NoPassing);
    assert_eq!(selection.selected_markdown, None);
    assert_eq!(selection.candidates.len(), 2);
    assert_eq!(
        (
            selection.candidates[0].opening_line,
            selection.candidates[0].closing_line
        ),
        (1, 4)
    );
    assert_eq!(
        (
            selection.candidates[1].opening_line,
            selection.candidates[1].closing_line
        ),
        (6, 9)
    );
    for candidate in &selection.candidates {
        assert!(!candidate.passed);
        assert!(!candidate.blocking_reasons.is_empty());
    }
}

#[test]
fn select_two_passing_candidates_is_ambiguous() {
    let first = complete_story_artifact("第一个候选需求。", "第一个候选可验收。");
    let second = complete_story_artifact("第二个候选需求。", "第二个候选可验收。");
    let output = format!("{}{}", fenced(&first), fenced(&second));

    let selection = select_workspace_artifact(&output, WorkspaceType::Story);

    assert_eq!(selection.verdict, SelectionVerdict::Ambiguous);
    assert_eq!(selection.selected_markdown, None);
    assert_eq!(selection.candidates.len(), 2);
    assert!(
        selection
            .candidates
            .iter()
            .all(|candidate| candidate.passed)
    );
}

#[test]
fn select_three_candidates_with_two_passing_is_ambiguous() {
    let first = complete_story_artifact("第一个候选需求。", "第一个候选可验收。");
    let second = complete_story_artifact("第二个候选需求。", "第二个候选可验收。");
    let output = format!(
        "{}{}{}",
        fenced(DRAFT_STORY),
        fenced(&first),
        fenced(&second),
    );

    let selection = select_workspace_artifact(&output, WorkspaceType::Story);

    assert_eq!(selection.verdict, SelectionVerdict::Ambiguous);
    assert_eq!(selection.selected_markdown, None);
    assert_eq!(selection.candidates.len(), 3);
    assert_eq!(
        selection
            .candidates
            .iter()
            .filter(|candidate| candidate.passed)
            .count(),
        2
    );
}

#[test]
fn select_zero_passing_does_not_fall_back_to_heading_outside_candidates() {
    // 候选全失败，但候选外另有一段完全合规的 Story 正文（无 fence）：禁止把它当产物。
    let compliant_outside = complete_story_artifact("候选外需求。", "候选外可验收。");
    let output = format!("{}\n{compliant_outside}", fenced(DRAFT_STORY));

    let selection = select_workspace_artifact(&output, WorkspaceType::Story);

    assert_eq!(selection.verdict, SelectionVerdict::NoPassing);
    assert_eq!(selection.selected_markdown, None);
    assert!(!selection.used_legacy_fallback);
    assert_eq!(selection.candidates.len(), 1);
    assert!(!selection.candidates[0].passed);
    assert!(!selection.candidates[0].blocking_reasons.is_empty());
}

#[test]
fn select_without_any_marker_uses_legacy_single_candidate_fallback() {
    let compliant = complete_story_artifact("无 marker 需求。", "无 marker 可验收。");
    let output = format!("分析过程。\n{compliant}");

    let selection = select_workspace_artifact(&output, WorkspaceType::Story);

    assert!(selection.used_legacy_fallback);
    assert_eq!(selection.verdict, SelectionVerdict::Unique);
    assert_eq!(selection.candidates.len(), 1);
    assert!(selection.candidates[0].passed);
    // 行号描述 legacy 抽取区间：heading 在第 2 行，正文末行即输出末行。
    assert_eq!(selection.candidates[0].opening_line, 2);
    assert_eq!(selection.candidates[0].closing_line, output.lines().count());
    assert_eq!(
        selection.selected_markdown.as_deref(),
        Some(compliant.trim_end())
    );
}

#[test]
fn select_f46_real_durable_output_picks_unique_gate_passing_candidate() {
    let lines = F46_RAW_OUTPUT.split_inclusive('\n').collect::<Vec<_>>();

    let selection = select_workspace_artifact(F46_RAW_OUTPUT, WorkspaceType::Story);

    assert!(!selection.used_legacy_fallback);
    assert_eq!(selection.raw_output_chars, 19_921);
    assert_eq!(
        selection.raw_output_sha256,
        "a67932c7bbeca856f9f452f7ccc044c9579424b64c2aa10dc3cfcc68c4343804"
    );
    assert_eq!(selection.candidates.len(), 2);

    let rejected = &selection.candidates[0];
    assert_eq!((rejected.opening_line, rejected.closing_line), (200, 203));
    assert!(!rejected.passed);
    assert_eq!(
        rejected.sha256,
        "b10b881edc61dc6d3c8e2a955b398f3a959d2603a7772457c3d6b9171b633519"
    );
    assert!(
        rejected
            .blocking_reasons
            .iter()
            .any(|reason| reason.contains("缺少 heading"))
    );
    // 逐候选阻断原因必须完整保留（示意块缺 6 个必需 heading + [REQ-*] + [AC-*] +
    // source id），不得截断或合并。
    assert_eq!(rejected.blocking_reasons.len(), 9);

    let passing = &selection.candidates[1];
    assert_eq!((passing.opening_line, passing.closing_line), (270, 361));
    assert!(passing.passed);
    assert!(passing.blocking_reasons.is_empty());
    assert_eq!(
        passing.sha256,
        "56e7686f51539e0a6a026897e1151371dbb40782634b68c40d69cace7e6ab774"
    );

    assert_eq!(selection.verdict, SelectionVerdict::Unique);
    // 选中正文逐字等于真实候选正文（content `:271-360`，90 行，含正文末行换行）。
    let expected = lines[270..360].concat();
    assert_eq!(
        selection.selected_markdown.as_deref(),
        Some(expected.as_str())
    );
}

/// Task 3：合成 durable session record（与磁盘 JSON 同构），用于 reload 路径断言。
fn session_record_with_assistant_output(
    workspace_type: WorkspaceType,
    content: String,
) -> WorkspaceSessionRecord {
    serde_json::from_value(serde_json::json!({
        "id": "workspace_session_reload",
        "project_id": "project_0001",
        "issue_id": "issue_0001",
        "entity_id": "entity_0001",
        "workspace_type": workspace_type,
        "status": "confirmed",
        "author_provider": "codex",
        "reviewer_provider": "claude_code",
        "review_rounds": 1,
        "superpowers_enabled": true,
        "openspec_enabled": true,
        "messages": [{
            "role": "assistant",
            "content": content,
            "created_at": "2026-09-23T00:00:00Z",
        }],
        "created_at": "2026-09-23T00:00:00Z",
        "updated_at": "2026-09-23T00:00:00Z",
    }))
    .expect("workspace session record fixture")
}

/// 双 block 原文：示意 block（过不了 gate）+ `<thinking>` 过程文本 + 唯一合规 block。
fn dual_block_output(compliant: &str) -> String {
    format!(
        "前言\n{}中间 <thinking>过程</thinking> 文本\n{}尾随说明\n",
        fenced(DRAFT_STORY),
        fenced(compliant),
    )
}

/// Task 3（REQ-ACS-01）：reload 与 coding fallback 走同一 selector——`from_record`
/// 只取唯一 gate-passing 候选，不再回退旧「首开—末闭」区间（那样会把示意块、过程
/// 文本与最终块混成一个产物）。
#[test]
fn reload_and_coding_fallback_share_unique_candidate_selection() {
    let compliant_story =
        complete_story_artifact("系统支持 provider 检查。", "用户能看到 provider 状态。");
    let session = WorkspaceSession::from_record(session_record_with_assistant_output(
        WorkspaceType::Story,
        dual_block_output(&compliant_story),
    ));

    assert_eq!(
        session
            .artifact
            .as_ref()
            .map(ArtifactPayload::markdown_or_empty),
        Some(compliant_story.trim())
    );

    // coding fallback（无持久化版本 markdown 时回退到最后一条 assistant 产物）同源。
    let compliant_work_item = complete_work_item_artifact("实现 provider 检查。");
    let work_item_record = session_record_with_assistant_output(
        WorkspaceType::WorkItem,
        dual_block_output(&compliant_work_item),
    );

    assert_eq!(
        crate::product::coding_work_item_context::select_work_item_markdown(
            None,
            &work_item_record
        )
        .as_deref(),
        Some(compliant_work_item.trim())
    );
}

// ===== Task 4（change `artifact-candidate-selection`）：候选选择诊断事件（REQ-ACS-02）=====
//
// 诊断走 `NodeDetail.execution_events` 的既有 upsert 通道，但只 durable——不广播
// `EngineEvent`，因此不新增 WebSocket/UI 公开语义（全局边界）。event id 稳定为
// `artifact_diag_{node_id 前 8}_{raw_output_sha256 前 8}`，原文零复制（output=None、
// detail 只带行号/字节/hash 与截断后的阻断原因）。

/// 诊断 JSON 顶层字段（计划 Task 4 schema + spec 要求的逐候选字节范围 +
/// 截断标记 `candidates_truncated`）。字段名集合是调查者脚本的契约，改动即破坏。
const ARTIFACT_DIAGNOSTIC_KEYS: [&str; 10] = [
    "candidate_count",
    "candidates",
    "candidates_truncated",
    "diagnostic_version",
    "passing_count",
    "raw_output_chars",
    "raw_output_sha256",
    "selection",
    "used_legacy_fallback",
    "workspace_type",
];

/// 逐候选字段：行号 + 字节范围（spec 明列）+ 正文 hash + 通过性 + 阻断原因。
const ARTIFACT_DIAGNOSTIC_CANDIDATE_KEYS: [&str; 7] = [
    "blocking_reasons",
    "closing_byte_end",
    "closing_line",
    "opening_byte",
    "opening_line",
    "passed",
    "sha256",
];

/// 零通过 selection（示意块 + 只有一级标题的块）：REQ-ACS-02 失败样本。
fn zero_passing_selection() -> CandidateSelection {
    workspace_artifact_selection(
        &format!("{}{}", fenced(DRAFT_STORY), fenced("# Story Spec\n")),
        &WorkspaceType::Story,
    )
}

/// 合成候选：诊断截断/上界用例需要确定性的超长阻断原因。
fn synthetic_candidate(index: usize, blocking_reasons: Vec<String>) -> CandidateGateResult {
    CandidateGateResult {
        opening_line: index * 10 + 1,
        closing_line: index * 10 + 3,
        opening_byte: index * 100,
        closing_byte_end: index * 100 + 50,
        sha256: format!("{index:064x}"),
        passed: false,
        blocking_reasons,
    }
}

fn synthetic_selection(
    candidates: Vec<CandidateGateResult>,
    raw_output_chars: usize,
) -> CandidateSelection {
    CandidateSelection {
        verdict: SelectionVerdict::NoPassing,
        selected_markdown: None,
        candidates,
        raw_output_chars,
        raw_output_sha256: "a".repeat(64),
        used_legacy_fallback: false,
    }
}

fn persisted_artifact_diagnostic(
    lifecycle_store: &LifecycleStore,
    session_id: &str,
    node_id: &str,
) -> serde_json::Value {
    let detail = lifecycle_store
        .load_node_detail(session_id, node_id)
        .expect("node detail");
    let artifact_events = detail
        .execution_events
        .iter()
        .filter(|event| event["kind"] == "artifact")
        .collect::<Vec<_>>();
    assert_eq!(
        artifact_events.len(),
        1,
        "expected exactly one artifact diagnostic on {node_id}: {detail:?}"
    );
    let event = artifact_events[0];
    assert!(
        event["output"].is_null(),
        "原文零复制：诊断 output 必须为空：{event:?}"
    );
    let diagnostic: serde_json::Value = serde_json::from_str(
        event["detail"]
            .as_str()
            .unwrap_or_else(|| panic!("diagnostic detail must be a JSON string: {event:?}")),
    )
    .expect("diagnostic detail parses");
    diagnostic
}

/// Task 4 用例①（真实路径）：`complete_assistant_message` 的 gate 失败分支必须落一条
/// 字段齐全的 kind=artifact 诊断，且 event id/status/原文零复制符合契约。
#[tokio::test]
async fn complete_assistant_message_gate_failure_records_full_diagnostic_event() {
    let (_tmp, lifecycle_store, mut engine) = persistent_test_engine();
    let node_id = create_author_run_node(&mut engine).await;
    let failing_output = format!("前言\n{}尾随说明\n", fenced(DRAFT_STORY));
    let expected = workspace_artifact_selection(&failing_output, &WorkspaceType::Story);
    assert_eq!(expected.verdict, SelectionVerdict::NoPassing);

    engine
        .complete_assistant_message(
            "assistant_msg_acs04".to_string(),
            failing_output.clone(),
            false,
        )
        .await;

    let detail = lifecycle_store
        .load_node_detail(&engine.session().session_id, &node_id)
        .expect("node detail");
    let event = detail
        .execution_events
        .iter()
        .find(|event| event["kind"] == "artifact")
        .unwrap_or_else(|| panic!("gate 失败必须落诊断事件：{detail:?}"));

    // 稳定 event id：artifact_diag_{node_id 前 8}_{raw_output_sha256 前 8}。
    assert_eq!(
        event["event_id"].as_str().expect("event id"),
        format!(
            "artifact_diag_{}_{}",
            &node_id[..8],
            &expected.raw_output_sha256[..8]
        ),
    );
    assert!(event["output"].is_null(), "原文零复制：{event:?}");
    assert_eq!(
        event["status"], "failed",
        "零通过诊断应为 failed：{event:?}"
    );

    let diagnostic: serde_json::Value =
        serde_json::from_str(event["detail"].as_str().expect("detail JSON string"))
            .expect("detail parses");
    let mut keys = diagnostic
        .as_object()
        .expect("diagnostic object")
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(keys, ARTIFACT_DIAGNOSTIC_KEYS, "{diagnostic}");

    assert_eq!(diagnostic["diagnostic_version"], 1);
    assert_eq!(diagnostic["workspace_type"], "story");
    assert_eq!(
        diagnostic["raw_output_chars"],
        expected.raw_output_chars as u64
    );
    assert_eq!(diagnostic["raw_output_sha256"], expected.raw_output_sha256);
    assert_eq!(diagnostic["used_legacy_fallback"], false);
    assert_eq!(diagnostic["candidate_count"], 1);
    assert_eq!(diagnostic["passing_count"], 0);
    assert_eq!(diagnostic["selection"], "none");
    assert_eq!(diagnostic["candidates_truncated"], false);

    let candidates = diagnostic["candidates"]
        .as_array()
        .expect("candidates array");
    assert_eq!(candidates.len(), 1);
    let mut candidate_keys = candidates[0]
        .as_object()
        .expect("candidate object")
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    candidate_keys.sort_unstable();
    assert_eq!(
        candidate_keys, ARTIFACT_DIAGNOSTIC_CANDIDATE_KEYS,
        "{diagnostic}"
    );
    assert_eq!(candidates[0]["opening_line"], 2);
    assert_eq!(candidates[0]["closing_line"], 5);
    assert_eq!(
        candidates[0]["opening_byte"],
        expected.candidates[0].opening_byte as u64
    );
    assert_eq!(
        candidates[0]["closing_byte_end"],
        expected.candidates[0].closing_byte_end as u64
    );
    assert_eq!(candidates[0]["sha256"], expected.candidates[0].sha256);
    assert_eq!(candidates[0]["passed"], false);
    assert!(
        !candidates[0]["blocking_reasons"]
            .as_array()
            .expect("blocking reasons")
            .is_empty()
    );

    // gate 结论不变：仍失败关闭、仍无产物。
    assert_eq!(detail.status, TimelineNodeStatus::Failed);
    assert!(engine.session().artifact.is_none());
}

/// Task 4 用例②：同一 node + 同一原文二次落诊断必须 upsert（event id 稳定），
/// 不产生重复条目。
#[tokio::test]
async fn artifact_diagnostic_upsert_is_idempotent_for_same_event_id() {
    let (_tmp, lifecycle_store, mut engine) = persistent_test_engine();
    let node_id = create_author_run_node(&mut engine).await;
    let selection = zero_passing_selection();

    for _ in 0..2 {
        engine
            .record_artifact_selection_diagnostic(Some(node_id.as_str()), &selection)
            .await
            .expect("diagnostic persists");
    }

    let detail = lifecycle_store
        .load_node_detail(&engine.session().session_id, &node_id)
        .expect("node detail");
    assert_eq!(
        detail
            .execution_events
            .iter()
            .filter(|event| event["kind"] == "artifact")
            .count(),
        1,
        "同 event_id 二次写入必须 upsert 而非追加：{detail:?}"
    );
    // 极端重复（10 次）也不放大 durable 体积。
    for _ in 0..8 {
        engine
            .record_artifact_selection_diagnostic(Some(node_id.as_str()), &selection)
            .await
            .expect("diagnostic persists");
    }
    let detail = lifecycle_store
        .load_node_detail(&engine.session().session_id, &node_id)
        .expect("node detail");
    assert_eq!(detail.execution_events.len(), 1, "{detail:?}");
}

/// Task 4 用例③：detail 有界——候选条数与单条阻断原因都有上界，且长度不随原文规模
/// 增长（原文零复制）；内容派生的阻断原因截断到上界内。
#[test]
fn artifact_diagnostic_detail_is_bounded_and_truncates_content_derived_reasons() {
    let huge_reason = format!(
        "待确认项未通过 AskUserQuestion 交互解决: {}",
        "疑".repeat(50_000)
    );
    let candidates = (0..40)
        .map(|index| synthetic_candidate(index, vec![huge_reason.clone()]))
        .collect::<Vec<_>>();
    let selection = synthetic_selection(candidates, 400_000);

    let event =
        artifact_selection_diagnostic_event("timeline_node_001", &selection, &WorkspaceType::Story);
    let detail = event.detail.as_deref().expect("detail");
    let diagnostic: serde_json::Value = serde_json::from_str(detail).expect("detail parses");

    // 总数保真、明细截断并显式标记。
    assert_eq!(diagnostic["candidate_count"], 40);
    assert_eq!(diagnostic["passing_count"], 0);
    assert_eq!(diagnostic["candidates_truncated"], true);
    let listed = diagnostic["candidates"].as_array().expect("candidates");
    assert_eq!(listed.len(), 16);

    // 内容派生的阻断原因被截断（远小于 50k 的正文片段）。
    for candidate in listed {
        for reason in candidate["blocking_reasons"].as_array().expect("reasons") {
            let reason = reason.as_str().expect("reason string");
            assert!(
                reason.chars().count() <= 257,
                "单条阻断原因必须截断到上界内，实际 {} 字符",
                reason.chars().count()
            );
            assert!(!reason.contains(&"疑".repeat(1000)), "{reason}");
        }
    }
    assert!(
        detail.chars().count() < 16 * 1024,
        "detail 必须与原文字符数无关（有界），实际 {} 字符",
        detail.chars().count()
    );
    // 原文（40 万字符）零复制：detail 里没有任何长文段。
    assert!(!detail.contains(&"疑".repeat(1000)));
}

/// Task 4 用例③（真实枚举）：候选条数由扫描器真实产出时同样受上界约束。
#[test]
fn artifact_diagnostic_caps_candidates_enumerated_from_real_output() {
    let raw = format!("前缀\n{}尾随说明\n", fenced("# 示意标题\n").repeat(200));
    let selection = workspace_artifact_selection(&raw, &WorkspaceType::Story);
    assert_eq!(selection.verdict, SelectionVerdict::NoPassing);
    assert_eq!(selection.candidates.len(), 200);

    let event =
        artifact_selection_diagnostic_event("timeline_node_002", &selection, &WorkspaceType::Story);
    let detail = event.detail.as_deref().expect("detail");
    let diagnostic: serde_json::Value = serde_json::from_str(detail).expect("detail parses");

    assert_eq!(diagnostic["candidate_count"], 200);
    assert_eq!(
        diagnostic["candidates"]
            .as_array()
            .expect("candidates")
            .len(),
        16
    );
    assert_eq!(diagnostic["candidates_truncated"], true);
    assert!(
        detail.chars().count() < 16 * 1024,
        "{}",
        detail.chars().count()
    );
}

/// Task 4 用例④：诊断持久化失败必须可见（失败摘要附有界提示）且不放宽 gate。
#[tokio::test]
async fn artifact_diagnostic_persistence_failure_stays_visible_without_widening_gate() {
    let (_tmp, _lifecycle_store, mut engine) = persistent_test_engine();
    let selection = zero_passing_selection();

    // 持久化失败臂：node 既不在 timeline_nodes 也不在 store。
    let error = engine
        .record_artifact_selection_diagnostic(Some("timeline_node_missing"), &selection)
        .await
        .expect_err("missing node must surface a persistence error");

    let reasons = artifact_failure_reasons_with_diagnostic(&selection, Some(&error));
    let summary = reasons.join("；");
    assert!(
        summary.contains("artifact diagnostic persistence failed"),
        "失败摘要必须携带 spec 逐字钉死的有界提示：{summary}"
    );
    // 提示有界：不把 store 错误全文（含路径）无限并入摘要。
    assert!(
        reasons.iter().all(|reason| reason.chars().count() <= 512),
        "{reasons:?}"
    );
    // gate 结论不变：仍零通过、仍不产出正文；失败原因一条不少。
    assert!(summary.contains("selection=none"), "{summary}");
    assert_eq!(selected_artifact_markdown(&selection), None);
    assert_eq!(
        reasons.len(),
        selection_failure_reasons(&selection).len() + 1
    );
    // 诊断成功时摘要与既有失败原因逐字一致（不引入额外文本）。
    assert_eq!(
        artifact_failure_reasons_with_diagnostic(&selection, None),
        selection_failure_reasons(&selection),
    );
}

/// Task 4 用例⑤：node detail 尚不存在（极早期失败）时 upsert 必须安全创建。
#[tokio::test]
async fn artifact_diagnostic_upserts_into_empty_node_detail() {
    let (_tmp, lifecycle_store, mut engine) = persistent_test_engine();
    let node_id = create_author_run_node(&mut engine).await;
    // `create_timeline_node` 只写 timeline 列表，此刻该 node 还没有 detail 文件。
    assert!(
        lifecycle_store
            .load_node_detail(&engine.session().session_id, &node_id)
            .is_err(),
        "fixture precondition: node detail must not exist yet"
    );

    engine
        .record_artifact_selection_diagnostic(Some(node_id.as_str()), &zero_passing_selection())
        .await
        .expect("empty node detail must be created safely");

    let diagnostic =
        persisted_artifact_diagnostic(&lifecycle_store, &engine.session().session_id, &node_id);
    assert_eq!(diagnostic["selection"], "none");
    assert_eq!(diagnostic["diagnostic_version"], 1);
}

/// Task 4 附加：唯一通过（成功路径）同样落诊断，且 status=completed —— 诊断覆盖
/// 「候选选择发生」而不仅是失败；成功路径的 detail 同样不含原文。
#[tokio::test]
async fn artifact_diagnostic_records_unique_selection_as_completed() {
    let (_tmp, lifecycle_store, mut engine) = persistent_test_engine();
    let node_id = create_author_run_node(&mut engine).await;
    let compliant =
        complete_story_artifact("系统支持 provider 检查。", "用户能看到 provider 状态。");
    let selection =
        workspace_artifact_selection(&dual_block_output(&compliant), &WorkspaceType::Story);
    assert_eq!(selection.verdict, SelectionVerdict::Unique);

    engine
        .record_artifact_selection_diagnostic(Some(node_id.as_str()), &selection)
        .await
        .expect("diagnostic persists");

    let detail = lifecycle_store
        .load_node_detail(&engine.session().session_id, &node_id)
        .expect("node detail");
    let event = detail
        .execution_events
        .iter()
        .find(|event| event["kind"] == "artifact")
        .expect("artifact diagnostic");
    assert_eq!(event["status"], "completed", "{event:?}");
    let diagnostic =
        persisted_artifact_diagnostic(&lifecycle_store, &engine.session().session_id, &node_id);
    assert_eq!(diagnostic["selection"], "unique");
    assert_eq!(diagnostic["candidate_count"], 2);
    assert_eq!(diagnostic["passing_count"], 1);
    assert_eq!(diagnostic["candidates_truncated"], false);
}
