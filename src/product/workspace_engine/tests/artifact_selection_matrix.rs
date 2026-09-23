//! Task 6（change `artifact-candidate-selection`）：跨类型表驱动回归矩阵。
//!
//! `Story / Design / WorkItem / WorkItemPlan` × 场景矩阵：同一份候选选择契约必须在四个
//! workspace type 上给出一致结论（REQ-ACS-01 全场景 + `story-pipeline-weak-model-hardening`
//! MODIFIED 的「一次成功」新判据）。
//!
//! | 场景 | 钉住的契约 |
//! |---|---|
//! | 前置示意 block + 唯一有效候选 | `Unique`：选中合规候选，示意块与 `<thinking>` 过程文本零混入 |
//! | 两个各自通过 gate 的候选 | `Ambiguous`：fail-closed，不取末块/最长块 |
//! | 所选正文污染（`<thinking>` / nested artifact fence） | 既有 gate 硬拒绝 → `NoPassing`，零放宽 |
//! | 候选全失败 + 候选外完全合规正文 | `NoPassing`：不回落 heading/tail 猜测产物 |
//! | session reload（`WorkspaceSession::from_record`） | 只取唯一通过候选，不错切成旧「首开—末闭」区间 |
//! | 同长度 fence 歧义（扫描器返回空） | 钉死「退 legacy fallback」的交界行为（AcSel02 移交覆盖项） |
//! | story 一次成功新判据 | 前置示意 block 不触发 retry；零通过/歧义触发 retry |

use super::*;

/// 示意 block：四类型共用「缺全部必需 heading」形态，任何 workspace type 的 gate 都拒绝。
const DRAFT_BLOCK: &str = "# 示意标题\n...\n";

fn fenced(markdown: &str) -> String {
    format!("```artifact\n{markdown}```\n")
}

/// Work Item Plan 的 **legacy Markdown** 合规产物（split JSON 流不走候选选择器）。
fn complete_work_item_plan_artifact(scope: &str) -> String {
    format!(
        "# Work Item Plan\n\n\
         ## 计划范围\n{scope}\n\n\
         ## 任务拆分\n- [TASK-001] 后端。\n\n\
         ## 依赖图\n无。\n\n\
         ## 验证计划\ncargo test --locked。\n\n\
         ## 执行顺序\n先后端。\n\n\
         ## 风险\n无。\n\n\
         ## 追踪关系\nsource ids: Story Spec story_spec_0001, Design Spec design_spec_0001。\n\
         [TASK-001] -> [REQ-001]\n"
    )
}

/// 矩阵行：一个 workspace type 的两份独立合规产物样本。
struct ArtifactMatrixCase {
    label: &'static str,
    workspace_type: WorkspaceType,
    /// 唯一 gate-passing 候选样本。
    compliant: String,
    /// 第二份独立合规候选（用于歧义场景，必须与 `compliant` 内容不同）。
    alternate: String,
}

fn story_case() -> ArtifactMatrixCase {
    ArtifactMatrixCase {
        label: "story",
        workspace_type: WorkspaceType::Story,
        compliant: complete_story_artifact(
            "系统支持 provider 检查。",
            "用户能看到 provider 状态。",
        ),
        alternate: complete_story_artifact("系统支持 provider 重试。", "用户能看到重试结果。"),
    }
}

fn design_case() -> ArtifactMatrixCase {
    ArtifactMatrixCase {
        label: "design",
        workspace_type: WorkspaceType::Design,
        compliant: complete_design_artifact("复用现有组件边界。", "新增 provider 检查接口。"),
        alternate: complete_design_artifact("新增独立组件边界。", "新增 provider 重试接口。"),
    }
}

fn work_item_case() -> ArtifactMatrixCase {
    ArtifactMatrixCase {
        label: "work_item",
        workspace_type: WorkspaceType::WorkItem,
        compliant: complete_work_item_artifact("实现 provider 检查。"),
        alternate: complete_work_item_artifact("实现 provider 重试。"),
    }
}

fn work_item_plan_case() -> ArtifactMatrixCase {
    ArtifactMatrixCase {
        label: "work_item_plan",
        workspace_type: WorkspaceType::WorkItemPlan,
        compliant: complete_work_item_plan_artifact(
            "本计划覆盖 Issue issue_0001 的 provider 检查。",
        ),
        alternate: complete_work_item_plan_artifact(
            "本计划覆盖 Issue issue_0001 的 provider 重试。",
        ),
    }
}

/// 四类型样本表（矩阵的行）。
fn matrix_cases() -> [ArtifactMatrixCase; 4] {
    [
        story_case(),
        design_case(),
        work_item_case(),
        work_item_plan_case(),
    ]
}

/// 前置示意 block + `<thinking>` 过程文本 + 唯一合规候选（F-46 形态）。
fn draft_then_candidate(compliant: &str) -> String {
    format!(
        "前言\n{}中间 <thinking>过程</thinking> 文本\n{}尾随说明\n",
        fenced(DRAFT_BLOCK),
        fenced(compliant),
    )
}

/// 同长度 fence 歧义输入：外层 ```artifact 正文内的 ```bash 未与外层配对，边界不可判定。
fn same_length_fence_ambiguity(body: &str) -> String {
    format!("前言\n```artifact\n{body}```bash\necho hi\n```\n尾随\n")
}

/// 场景 1：前置示意 block + 唯一有效候选 → 恢复（REQ-ACS-01 首场景，四类型一致）。
#[test]
fn matrix_unique_candidate_recovers_after_draft_block() {
    for case in matrix_cases() {
        let output = draft_then_candidate(&case.compliant);
        let selection = select_workspace_artifact(&output, case.workspace_type.clone());

        assert_eq!(
            selection.verdict,
            SelectionVerdict::Unique,
            "{}",
            case.label
        );
        assert!(!selection.used_legacy_fallback, "{}", case.label);
        assert_eq!(selection.candidates.len(), 2, "{}", case.label);
        assert!(!selection.candidates[0].passed, "{}", case.label);
        assert!(
            selection.candidates[0]
                .blocking_reasons
                .iter()
                .any(|reason| reason.contains("缺少 heading")),
            "{}: 示意 block 必须带必需 heading 缺失原因：{:?}",
            case.label,
            selection.candidates[0].blocking_reasons,
        );
        assert!(selection.candidates[1].passed, "{}", case.label);
        assert!(
            selection.candidates[1].blocking_reasons.is_empty(),
            "{}",
            case.label
        );
        assert_eq!(
            selected_artifact_markdown(&selection).as_deref(),
            Some(case.compliant.trim()),
            "{}: 过程文本与示意 block 不得进入产物",
            case.label,
        );
    }
}

/// 场景 2：两个各自通过 gate 的候选 → 歧义失败关闭（不做多数决/最长决）。
#[test]
fn matrix_two_passing_candidates_are_ambiguous() {
    for case in matrix_cases() {
        let output = format!("{}{}", fenced(&case.compliant), fenced(&case.alternate));
        let selection = select_workspace_artifact(&output, case.workspace_type.clone());

        assert_eq!(
            selection.verdict,
            SelectionVerdict::Ambiguous,
            "{}",
            case.label
        );
        assert_eq!(selection.selected_markdown, None, "{}", case.label);
        assert_eq!(
            selected_artifact_markdown(&selection),
            None,
            "{}: 歧义不得产出任何正文",
            case.label,
        );
        assert_eq!(selection.candidates.len(), 2, "{}", case.label);
        assert!(
            selection
                .candidates
                .iter()
                .all(|candidate| candidate.passed),
            "{}: 夹具前提是两份候选都通过 gate：{:?}",
            case.label,
            selection.candidates,
        );
    }
}

/// 场景 3：所选正文污染仍硬拒绝（REQ-ACS-01 末场景：提取边界修复不构成放行）。
#[test]
fn matrix_polluted_unique_candidate_is_still_rejected() {
    for case in matrix_cases() {
        for (pollution, polluted, expected_reason) in [
            (
                "thinking 标签",
                format!("{}过程 <thinking>思考</thinking>\n", case.compliant),
                "禁止内容: <thinking> 出现在 artifact 内",
            ),
            (
                "nested artifact fence",
                format!("{}```artifact\n示意内层\n```\n", case.compliant),
                "禁止内容: artifact fence 出现在已抽取 artifact 内",
            ),
        ] {
            let output = fenced(&polluted);
            let selection = select_workspace_artifact(&output, case.workspace_type.clone());

            assert_eq!(
                selection.verdict,
                SelectionVerdict::NoPassing,
                "{} / {pollution}",
                case.label,
            );
            assert_eq!(
                selection.selected_markdown, None,
                "{} / {pollution}",
                case.label
            );
            assert_eq!(
                selection.candidates.len(),
                1,
                "{} / {pollution}",
                case.label
            );
            assert!(
                selection.candidates[0]
                    .blocking_reasons
                    .iter()
                    .any(|reason| reason == expected_reason),
                "{} / {pollution}: 阻断原因须逐字保留：{:?}",
                case.label,
                selection.candidates[0].blocking_reasons,
            );
        }
    }
}

/// 场景 4：候选全失败但候选外另有完全合规正文（无 fence）→ 禁止回落猜测产物。
#[test]
fn matrix_zero_passing_does_not_fall_back_outside_candidates() {
    for case in matrix_cases() {
        let output = format!("{}\n{}", fenced(DRAFT_BLOCK), case.compliant);
        let selection = select_workspace_artifact(&output, case.workspace_type.clone());

        assert_eq!(
            selection.verdict,
            SelectionVerdict::NoPassing,
            "{}",
            case.label
        );
        assert!(!selection.used_legacy_fallback, "{}", case.label);
        assert_eq!(selection.candidates.len(), 1, "{}", case.label);
        assert!(!selection.candidates[0].passed, "{}", case.label);
        assert_eq!(
            selected_artifact_markdown(&selection),
            None,
            "{}: 候选外正文（旧 heading/tail 抽取目标）不得成为产物",
            case.label,
        );
    }
}

/// 场景 5：session reload 走同一选择器——只取唯一通过候选，不错切成旧「首开—末闭」区间
/// （那样会把示意 block、`<thinking>` 过程文本与最终候选混成一个产物）。
#[test]
fn matrix_reload_selects_unique_candidate_per_workspace_type() {
    for case in matrix_cases() {
        let record = matrix_session_record(
            case.workspace_type.clone(),
            draft_then_candidate(&case.compliant),
        );
        let session = WorkspaceSession::from_record(record);
        let artifact = session
            .artifact
            .as_ref()
            .map(ArtifactPayload::markdown_or_empty);

        assert_eq!(artifact, Some(case.compliant.trim()), "{}", case.label);
        let artifact = artifact.expect("reload artifact");
        assert!(
            !artifact.contains("示意标题"),
            "{}: 示意 block 不得混入 reload 产物",
            case.label,
        );
        assert!(
            !artifact.contains("<thinking>"),
            "{}: 过程文本不得混入 reload 产物",
            case.label,
        );
    }
}

/// 场景 6（AcSel02 移交的显式覆盖项）：同长度 fence 歧义 → 扫描器返回空 → **走 legacy
/// fallback** 的交界行为。
///
/// 候选层 fail-closed（不产出候选、不猜边界）成立；交界在于计划 Step 2.3 规定「fallback
/// 仅在 `scan` 返回空时启用」，而歧义输入恰好让 `scan` 返回空，于是产品级结论交由 legacy
/// 抽取正文的既有 gate 决定。本行把该交界钉死，避免后续把它误读为「歧义一律硬失败」或
/// 反之「歧义无候选即可放行任意 legacy 正文」。
#[test]
fn matrix_same_length_fence_ambiguity_falls_back_to_legacy_gate() {
    for case in matrix_cases() {
        // (a) 歧义输入：候选层零产出，legacy 抽取正文过不了 gate → 产品级仍 fail-closed。
        let non_compliant = same_length_fence_ambiguity(DRAFT_BLOCK);
        assert!(
            crate::product::artifact_extraction::scan_top_level_fenced_candidates(&non_compliant)
                .is_empty(),
            "{}: 同长度 fence 歧义不得产出候选",
            case.label,
        );
        let selection = select_workspace_artifact(&non_compliant, case.workspace_type.clone());
        assert!(selection.used_legacy_fallback, "{}", case.label);
        assert_eq!(
            selection.verdict,
            SelectionVerdict::NoPassing,
            "{}",
            case.label
        );
        assert_eq!(selection.selected_markdown, None, "{}", case.label);

        // (b) 交界：legacy 抽取正文若通过 gate，产品级仍产出——歧义不构成独立的硬失败
        // 判据（钉死现状；legacy 正文含未闭合的 ```bash fence）。
        let recoverable = same_length_fence_ambiguity(&case.compliant);
        assert!(
            crate::product::artifact_extraction::scan_top_level_fenced_candidates(&recoverable)
                .is_empty(),
            "{}: 同长度 fence 歧义不得产出候选",
            case.label,
        );
        let selection = select_workspace_artifact(&recoverable, case.workspace_type.clone());
        assert!(selection.used_legacy_fallback, "{}", case.label);
        assert_eq!(
            selection.verdict,
            SelectionVerdict::Unique,
            "{}",
            case.label
        );
        let legacy_body = format!("{}```bash\necho hi\n", case.compliant);
        assert_eq!(
            selected_artifact_markdown(&selection).as_deref(),
            Some(legacy_body.trim()),
            "{}: legacy fallback 正文即抽取区间",
            case.label,
        );
    }
}

/// 场景 7（`story-pipeline-weak-model-hardening` MODIFIED）：story author 的「一次成功」判据
/// = 首次输出存在**唯一一个通过 gate 的完整候选**；前置未通过候选（示意 block/自检推演）
/// 不单独构成失败，两个及以上通过 gate 的候选仍构成歧义失败。
#[test]
fn story_once_success_criterion_follows_unique_candidate_selection() {
    let (_tmp, store) = setup();
    let (tx, _event_rx) = mpsc::channel(8);
    let engine = WorkspaceEngine::new(store, tx, make_session("sess_story_once_success"));
    let case = story_case();

    for (label, output, expect_retry) in [
        (
            "唯一有效候选（无前置 block）",
            fenced(&case.compliant),
            false,
        ),
        (
            "前置示意 block + 唯一有效候选（新判据：仍一次成功）",
            draft_then_candidate(&case.compliant),
            false,
        ),
        (
            "两个有效候选（歧义 → 不计一次成功）",
            format!("{}{}", fenced(&case.compliant), fenced(&case.alternate)),
            true,
        ),
        ("只有示意 block（零通过）", fenced(DRAFT_BLOCK), true),
    ] {
        assert_eq!(
            engine.should_retry_missing_workspace_artifact(&output),
            expect_retry,
            "story「一次成功」判据：{label}",
        );
    }
}

/// 场景 7（引擎级）：前置示意 block 不破坏一次成功——provider 只被调用一次（无 retry）、
/// 节点停在 author_confirm、产物为唯一通过候选且过程文本/示意 block 零混入。
#[tokio::test]
async fn story_author_once_success_with_preceding_draft_block_does_not_retry() {
    let (_tmp, _lifecycle_store, mut engine) = persistent_test_engine();
    engine.session.reviewer_provider = None;
    let case = story_case();
    let inputs = Arc::new(Mutex::new(Vec::new()));
    let provider = Arc::new(ImmediateOutputRecordingProvider {
        inputs: inputs.clone(),
        output: draft_then_candidate(&case.compliant),
    });

    engine
        .handle_user_message(
            "开始生成 Story Spec".to_string(),
            provider,
            empty_provider_commands(),
        )
        .await;

    assert_eq!(
        inputs.lock().unwrap().len(),
        1,
        "唯一通过候选 + 前置示意 block 必须一次成功，不得启动 artifact retry",
    );
    assert_eq!(engine.session().stage, WorkspaceStage::AuthorConfirm);
    assert_eq!(
        engine
            .session()
            .artifact
            .as_ref()
            .map(ArtifactPayload::markdown_or_empty),
        Some(case.compliant.trim()),
    );
    let author_nodes = engine
        .timeline_nodes
        .iter()
        .filter(|node| node.node_type == TimelineNodeType::AuthorRun)
        .collect::<Vec<_>>();
    assert_eq!(
        author_nodes.len(),
        1,
        "一次成功不得为 retry 另起节点：{author_nodes:?}",
    );
    assert_eq!(author_nodes[0].status, TimelineNodeStatus::Completed);
}

/// durable session record 夹具（与磁盘 JSON 同构），用于 reload 场景。
fn matrix_session_record(workspace_type: WorkspaceType, content: String) -> WorkspaceSessionRecord {
    serde_json::from_value(serde_json::json!({
        "id": "workspace_session_matrix",
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
