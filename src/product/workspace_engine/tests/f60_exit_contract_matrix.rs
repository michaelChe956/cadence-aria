// F-60 P0（因素三/五 + 用户裁决 2026-09-26 分层合同注入）：契约出口强制注入验收矩阵。
// 设计：cadence/designs/2026-09-25_技术方案_F60一次成功率_v1.0.md。
// 每个最终构造 StreamingProviderInput 的入口（build_streaming_input 的
// FullConversation/DeltaOnly、choice 应答续跑两轮、AuthorConfirm 用户反馈
// 有/无 resume、reviewer delta/full、Codex resume stall fresh、以及
// build_work_item_plan_streaming_input 的显式合同族）末端最后可信合同块：
// - 初次生成（FullConversation／fresh 修订轮／Markdown 直发）＝ 完整装配；
// - 后续轮（DeltaOnly 续跑／resume 增量修订）＝ 一行短引用（全部必需 heading +
//   关键追踪 token + 指向会话开头完整合同），不重复全文。
// 历史旧 marker 不抑制任何形态；JSON 子链不误拼 Markdown 合同。
use super::*;

const ALL_MARKDOWN_WORKSPACE_TYPES: [WorkspaceType; 4] = [
    WorkspaceType::Story,
    WorkspaceType::Design,
    WorkspaceType::WorkItem,
    WorkspaceType::WorkItemPlan,
];

fn exit_contract_engine(session_id: &str, workspace_type: WorkspaceType) -> WorkspaceEngine {
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut session = make_session(session_id);
    session.workspace_type = workspace_type;
    let checkpoint_tmp = TempDir::new().unwrap();
    WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
        event_tx,
        session,
    )
}

fn recorded_author_conversation() -> ProviderConversationRef {
    ProviderConversationRef {
        role: ProviderConversationRole::Author,
        provider: ProviderName::ClaudeCode,
        provider_session_id: "author-session-1".to_string(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        last_node_id: None,
    }
}

fn revise_verdict() -> ReviewVerdict {
    ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "请补全结构。".to_string(),
        summary: "缺少 artifact schema".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    }
}

/// 完整形态断言：prompt 中最后一个 `[artifact_schema_contract]` 之后必须逐字节
/// 是当前类型的同源完整渲染，choice 决策归档、负面清单与骨架同块随行。
fn assert_terminal_full_contract(prompt: &str, workspace_type: &WorkspaceType) {
    let schema = author_artifact_schema_contract_for(workspace_type)
        .expect("Markdown workspace must render an artifact schema contract");
    let expected_schema = schema.trim_start_matches('\n');
    let last_marker = prompt
        .rfind(ARTIFACT_SCHEMA_CONTRACT_MARKER)
        .unwrap_or_else(|| {
            panic!("{workspace_type:?} prompt must carry the artifact schema contract: {prompt}")
        });
    let tail = &prompt[last_marker..];
    assert!(
        tail.starts_with(expected_schema),
        "{workspace_type:?} terminal contract must be the current full render: {prompt}"
    );
    assert!(
        tail.contains("author-decision-*"),
        "{workspace_type:?} terminal contract must carry the choice decision archive clause: {prompt}"
    );
    assert!(
        tail.contains("输出纪律（负面清单）"),
        "{workspace_type:?} terminal contract must carry the negative list: {prompt}"
    );
    assert!(
        tail.contains("最小结构骨架示例"),
        "{workspace_type:?} terminal contract must carry the skeleton example: {prompt}"
    );
    assert!(
        !tail[expected_schema.len()..].contains(ARTIFACT_SCHEMA_CONTRACT_MARKER),
        "{workspace_type:?} no second schema contract may follow the terminal one: {prompt}"
    );
    assert!(
        prompt.contains("原始返回必须使用完整 artifact fenced block"),
        "{workspace_type:?} terminal contract must teach the artifact fence rule: {prompt}"
    );
    assert!(
        prompt.contains(&format!(
            "fence 内第一行必须是 {} 一级标题",
            workspace_type_title(workspace_type)
        )),
        "{workspace_type:?} terminal contract must teach the H1 rule: {prompt}"
    );
    assert!(
        prompt.contains("四反引号 ````artifact"),
        "{workspace_type:?} terminal contract must teach the quadruple-backtick rule: {prompt}"
    );
}

/// 分层短引用断言（用户裁决 2026-09-26）：末端为一行简引——同一规则源点名列出
/// 当前类型全部必需二级 heading 与必需稳定 ID/追踪 token（含 source id），指向
/// 会话开头完整合同；不重复全文（无 schema 全文/负面清单/骨架）。
fn assert_terminal_delta_reference(prompt: &str, workspace_type: &WorkspaceType) {
    let spec = artifact_constraint_spec_for(workspace_type);
    let reference_at = prompt.rfind("输出格式契约（本轮简引").unwrap_or_else(|| {
        panic!("{workspace_type:?} delta round must end with the short reference: {prompt}")
    });
    let tail = &prompt[reference_at..];
    assert!(
        tail.contains("[artifact_schema_contract]"),
        "{workspace_type:?} short reference must point at the session-head full contract: {prompt}"
    );
    assert!(
        tail.contains(&format!(
            "fence 内第一行是 {} 一级标题",
            workspace_type_title(workspace_type)
        )),
        "{workspace_type:?} short reference must restate the fence/H1 rule: {prompt}"
    );
    assert!(
        tail.contains("四反引号 ````artifact"),
        "{workspace_type:?} short reference must restate the quadruple-backtick rule: {prompt}"
    );
    for heading in &spec.required_headings {
        assert!(
            tail.contains(&format!("## {}", heading.label)),
            "{workspace_type:?} short reference must name required heading `{}`: {prompt}",
            heading.label
        );
    }
    for rule in &spec.required_id_patterns {
        assert!(
            tail.contains(rule.label),
            "{workspace_type:?} short reference must name required id `{}`: {prompt}",
            rule.label
        );
    }
    for rule in &spec.required_tokens {
        assert!(
            tail.contains(rule.label),
            "{workspace_type:?} short reference must name required token `{}`: {prompt}",
            rule.label
        );
    }
    assert!(
        tail.contains("author-decision-*"),
        "{workspace_type:?} short reference must keep the decision-archive pointer: {prompt}"
    );
    // 不重复全文：完整装配独有的块不得出现。
    assert!(
        !tail.contains("最终 Markdown artifact 必须同时满足以下 parser schema"),
        "{workspace_type:?} short reference must not replay the full schema: {prompt}"
    );
    assert!(
        !tail.contains("输出纪律（负面清单）"),
        "{workspace_type:?} short reference must not replay the negative list: {prompt}"
    );
    assert!(
        !tail.contains("最小结构骨架示例"),
        "{workspace_type:?} short reference must not replay the skeleton: {prompt}"
    );
    // 末端性：短引用之后没有再出现 schema 全文块。
    assert!(
        !tail[tail.find("author-decision-*").expect("pointer")..]
            .contains(ARTIFACT_SCHEMA_CONTRACT_MARKER),
        "{workspace_type:?} nothing may follow the terminal short reference: {prompt}"
    );
}

/// F-46 回归基座：初次 Full = 完整装配；后续轮 Delta = 一行短引用，四个 Markdown 类型。
#[test]
fn exit_contract_covers_full_and_delta_for_all_markdown_workspace_types() {
    for workspace_type in ALL_MARKDOWN_WORKSPACE_TYPES {
        let engine = exit_contract_engine(
            &format!("sess_f60_exit_full_{workspace_type:?}"),
            workspace_type.clone(),
        );
        let full = engine
            .build_streaming_input("开始生成", AuthorPromptMode::FullConversation)
            .expect("full streaming input");
        assert!(
            full.prompt.contains("开始生成"),
            "{workspace_type:?} full prompt must keep the user content: {}",
            full.prompt
        );
        assert_terminal_full_contract(&full.prompt, &workspace_type);

        let engine = exit_contract_engine(
            &format!("sess_f60_exit_delta_{workspace_type:?}"),
            workspace_type.clone(),
        );
        let delta = engine
            .build_streaming_input("请继续输出完整候选产物", AuthorPromptMode::DeltaOnly)
            .expect("delta streaming input");
        assert!(
            delta.prompt.starts_with("请继续输出完整候选产物"),
            "{workspace_type:?} delta content must pass through verbatim at the head: {}",
            delta.prompt
        );
        assert_terminal_delta_reference(&delta.prompt, &workspace_type);
    }
}

/// 方案因素三：不能因为历史消息或用户输入中出现 marker 就省略末端合同（两形态同理）。
#[test]
fn exit_contract_survives_stale_marker_in_session_history_and_delta_content() {
    let mut engine = exit_contract_engine("sess_f60_stale_marker", WorkspaceType::Story);
    engine.session.messages.push(SessionMessage {
        id: "msg_generation_context".to_string(),
        role: "system".to_string(),
        content: "[artifact_schema_contract]\n来自旧 generation brief 的过期条款（不携带当前 gate 全条款）。"
            .to_string(),
        checkpoint_id: None,
        created_at: "2026-07-23T00:00:00Z".to_string(),
    });

    let full = engine
        .build_streaming_input("开始生成", AuthorPromptMode::FullConversation)
        .expect("full input");
    assert!(
        full.prompt.matches(ARTIFACT_SCHEMA_CONTRACT_MARKER).count() >= 2,
        "history marker must not suppress the terminal contract: {}",
        full.prompt
    );
    assert_terminal_full_contract(&full.prompt, &WorkspaceType::Story);

    let delta = engine
        .build_streaming_input(
            "继续生成（历史 delta 原文中带 [artifact_schema_contract] 旧标记）",
            AuthorPromptMode::DeltaOnly,
        )
        .expect("delta input");
    assert_terminal_delta_reference(&delta.prompt, &WorkspaceType::Story);
}

/// choice 应答续跑：连续两轮，每轮 delta 出口都以短引用收尾，choice 点本身只产
/// 问答内容（不再各业务分支自行拼契约）。
#[tokio::test]
async fn choice_followup_two_rounds_each_carry_terminal_contract() {
    for (round, workspace_type) in [(1, WorkspaceType::Story), (2, WorkspaceType::Design)] {
        let mut engine = exit_contract_engine(
            &format!("sess_f60_choice_round_{round}"),
            workspace_type.clone(),
        );
        engine.pending_author_choice = Some(PendingAuthorChoice {
            id: format!("author_choice_round_{round}"),
            prompt: format!("第 {round} 轮问题：范围如何取舍？"),
            options: vec![ChoiceOptionData {
                id: "a".to_string(),
                label: "按当前范围".to_string(),
                description: None,
            }],
            source_node_id: None,
        });

        let followup = engine
            .take_pending_author_choice_prompt(
                &format!("author_choice_round_{round}"),
                vec!["a".to_string()],
                Some("保持可读性".to_string()),
            )
            .await
            .expect("choice followup content");
        assert!(
            !followup.contains(ARTIFACT_SCHEMA_CONTRACT_MARKER),
            "choice point must stay pure Q&A; the exit owns the contract: {followup}"
        );

        let input = engine
            .build_streaming_input(followup.trim(), AuthorPromptMode::DeltaOnly)
            .expect("delta input");
        assert!(
            input.prompt.starts_with("用户回答了 author 的确认问题："),
            "delta content must pass through verbatim at the head: {}",
            input.prompt
        );
        assert!(
            input
                .prompt
                .contains(&format!("第 {round} 轮问题：范围如何取舍？")),
            "{workspace_type:?} round {round} answer content must survive: {}",
            input.prompt
        );
        assert_terminal_delta_reference(&input.prompt, &workspace_type);
    }
}

/// AuthorConfirm 用户反馈修订（F-60 确认缺口 2）：fresh（无 resume）= 完整装配；
/// resume 增量轮 = 短引用；四类型全覆盖。
#[test]
fn author_feedback_revision_tiers_by_resume_and_covers_all_markdown_types() {
    for workspace_type in ALL_MARKDOWN_WORKSPACE_TYPES {
        let mut engine = exit_contract_engine(
            &format!("sess_f60_feedback_fresh_{workspace_type:?}"),
            workspace_type.clone(),
        );
        engine.pending_revision_context = Some("把成功标准改为可量化的验收。".to_string());

        let fresh = engine
            .build_revision_input_with_resume(false)
            .expect("feedback fresh input");
        assert!(
            fresh
                .prompt
                .contains("请作为 author 基于用户反馈对当前 Workspace 产物做增量修订"),
            "{}",
            fresh.prompt
        );
        assert_eq!(fresh.resume_provider_session_id, None);
        // prompt 内嵌旧稿 → 完整装配使用「上一版 Artifact 已被剥离」变体教学。
        assert!(
            fresh
                .prompt
                .contains("上一版 Artifact 是 daemon 已提取的 markdown"),
            "{}",
            fresh.prompt
        );
        assert_terminal_full_contract(&fresh.prompt, &workspace_type);

        let mut engine = exit_contract_engine(
            &format!("sess_f60_feedback_resume_{workspace_type:?}"),
            workspace_type.clone(),
        );
        engine.pending_revision_context = Some("把成功标准改为可量化的验收。".to_string());
        engine.session.provider_conversations = vec![recorded_author_conversation()];
        let resumed = engine
            .build_revision_input_with_resume(true)
            .expect("feedback resume input");
        assert_eq!(
            resumed.resume_provider_session_id.as_deref(),
            Some("author-session-1")
        );
        assert_terminal_delta_reference(&resumed.prompt, &workspace_type);
    }
}

/// reviewer 返修三变体分层：resume delta = 短引用；fresh full 与 Codex
/// resume-stall fresh（provider 全新会话，prompt 须自足）= 完整装配。
#[test]
fn reviewer_revision_delta_full_and_codex_fresh_carry_terminal_contract() {
    for workspace_type in ALL_MARKDOWN_WORKSPACE_TYPES {
        let mut delta_engine = exit_contract_engine(
            &format!("sess_f60_reviewer_delta_{workspace_type:?}"),
            workspace_type.clone(),
        );
        delta_engine.latest_review_verdict = Some(revise_verdict());
        delta_engine.session.provider_conversations = vec![recorded_author_conversation()];
        let delta = delta_engine
            .build_revision_input()
            .expect("reviewer delta input");
        assert!(
            delta
                .prompt
                .contains("这是对当前 provider 会话的增量返修指令"),
            "{}",
            delta.prompt
        );
        assert_eq!(
            delta.resume_provider_session_id.as_deref(),
            Some("author-session-1")
        );
        assert_terminal_delta_reference(&delta.prompt, &workspace_type);

        let mut full_engine = exit_contract_engine(
            &format!("sess_f60_reviewer_full_{workspace_type:?}"),
            workspace_type.clone(),
        );
        full_engine.latest_review_verdict = Some(revise_verdict());
        let full = full_engine
            .build_revision_input()
            .expect("reviewer full input");
        assert!(
            full.prompt
                .contains("会话上下文（滑动窗口压缩；最近 2 轮保留原文）"),
            "{}",
            full.prompt
        );
        assert!(
            full.prompt
                .contains("上一版 Artifact 是 daemon 已提取的 markdown"),
            "{}",
            full.prompt
        );
        assert_terminal_full_contract(&full.prompt, &workspace_type);

        // Codex resume stall fresh：存在旧会话但禁用 resume → fresh 全量 + 完整装配。
        let mut codex_engine = exit_contract_engine(
            &format!("sess_f60_codex_fresh_{workspace_type:?}"),
            workspace_type.clone(),
        );
        codex_engine.latest_review_verdict = Some(revise_verdict());
        codex_engine.session.provider_conversations = vec![recorded_author_conversation()];
        let codex_fresh = codex_engine
            .build_revision_input_without_resume()
            .expect("codex fresh input");
        assert_eq!(codex_fresh.resume_provider_session_id, None);
        assert!(
            codex_fresh
                .prompt
                .contains("会话上下文（滑动窗口压缩；最近 2 轮保留原文）"),
            "{}",
            codex_fresh.prompt
        );
        assert_terminal_full_contract(&codex_fresh.prompt, &workspace_type);
    }
}

/// build_work_item_plan_streaming_input 合同族显式化：JSON 子链（Structured）
/// 不误拼 Markdown 合同；Markdown author invocation（MarkdownArtifact）出口
/// 注入同一装配（WorkItemPlan gate 全条款，初次直发＝完整形态）。
#[test]
fn plan_streaming_input_contract_family_is_explicit_and_json_chain_stays_clean() {
    let engine = exit_contract_engine("sess_f60_plan_family", WorkspaceType::WorkItemPlan);

    let structured = engine
        .build_work_item_plan_streaming_input(
            ProviderType::Fake,
            "split prompt（JSON Outline/Split/Draft 子链）".to_string(),
            "/tmp/worktree".to_string(),
            ProviderName::Fake,
            PlanAuthorOutputContract::Structured,
        )
        .expect("structured plan input");
    assert!(
        structured
            .prompt
            .starts_with("split prompt（JSON Outline/Split/Draft 子链）"),
        "{}",
        structured.prompt
    );
    assert!(
        !structured.prompt.contains(ARTIFACT_SCHEMA_CONTRACT_MARKER),
        "JSON sub-chain must not receive the Markdown artifact contract: {}",
        structured.prompt
    );
    assert!(
        !structured.prompt.contains("输出格式契约："),
        "JSON sub-chain must stay free of the Markdown output-contract preamble: {}",
        structured.prompt
    );

    let markdown = engine
        .build_work_item_plan_streaming_input(
            ProviderType::Fake,
            "markdown plan author prompt".to_string(),
            "/tmp/worktree".to_string(),
            ProviderName::Fake,
            PlanAuthorOutputContract::MarkdownArtifact,
        )
        .expect("markdown plan input");
    assert!(
        markdown.prompt.starts_with("markdown plan author prompt"),
        "{}",
        markdown.prompt
    );
    assert_terminal_full_contract(&markdown.prompt, &WorkspaceType::WorkItemPlan);
}

/// F-60 P0（用户裁决 2026-09-26）：story/design markdown author 族 prompt 质量
/// 预算——初次完整装配后的无历史 prompt 不超预算；后续轮短引用不超短引用预算
///（实测字节见常量注释回填）。
#[test]
fn markdown_author_family_prompt_quality_budget() {
    for workspace_type in [WorkspaceType::Story, WorkspaceType::Design] {
        let engine = exit_contract_engine(
            &format!("sess_f60_budget_{workspace_type:?}"),
            workspace_type.clone(),
        );
        let input = engine
            .build_streaming_input("开始生成", AuthorPromptMode::FullConversation)
            .expect("full input");
        assert!(
            input.prompt.len() <= MARKDOWN_AUTHOR_PROMPT_MAX_BYTES,
            "{workspace_type:?} first-round prompt {} bytes exceeds budget {MARKDOWN_AUTHOR_PROMPT_MAX_BYTES}",
            input.prompt.len()
        );
    }
    for workspace_type in ALL_MARKDOWN_WORKSPACE_TYPES {
        let reference = markdown_author_output_contract_short_reference(&workspace_type);
        assert!(
            reference.len() <= MARKDOWN_AUTHOR_DELTA_REFERENCE_MAX_BYTES,
            "{workspace_type:?} delta short reference {} bytes exceeds budget {MARKDOWN_AUTHOR_DELTA_REFERENCE_MAX_BYTES}",
            reference.len()
        );
    }
}
