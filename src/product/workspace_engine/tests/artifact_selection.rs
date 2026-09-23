//! Task 2（change `artifact-candidate-selection`）：workspace-aware 唯一 gate-passing
//! 候选选择器契约（REQ-ACS-01）。
//!
//! 用例覆盖：唯一通过选中、零通过逐候选保留阻断原因、双通过歧义、三候选（1 示意 +
//! 2 合法）歧义、候选全失败时禁止回落候选外 heading、无任何 marker 时 legacy 单候选
//! fallback，以及 F-46 真实 durable 原文实跑（候选 2 个、`:200-203` 拒、`:270-361`
//! 唯一通过）。
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
