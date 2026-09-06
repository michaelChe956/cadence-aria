// part_32 拆分（large_file_guard 1200 行上限）：prompt 滑动窗口与 builder
// 角色策略矩阵测试从 part_32.rs 移入；include! 保持同一模块作用域，
// 测试路径与行为零变化。

#[test]
fn prompt_sliding_window_applies_to_author_revision_and_reviewer_entrypoints() {
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut session = make_session("sess_prompt_sliding_window");
    session.workspace_type = WorkspaceType::Story;
    let mut artifacts = Vec::new();
    session.messages = vec![SessionMessage {
        id: "msg_000".to_string(),
        role: "system".to_string(),
        content: "Workspace 生成任务已准备\n\n[canonical_inputs]\n完整 canonical inputs：不得裁剪。\n\n[constraint_summary]\n历史约束。".to_string(),
        checkpoint_id: None,
        created_at: "2026-08-21T00:00:00Z".to_string(),
    }];
    for round in 1..=4 {
        let markdown = format!(
            "# Story Artifact v{round}\n\n## 功能需求\n- [REQ-{round:03}] {}\n\n## 成功标准\n- [AC-{round:03}] {}\n",
            "早期 artifact 正文 ".repeat(30),
            "验收细节 ".repeat(30),
        );
        session.messages.push(SessionMessage {
            id: format!("msg_user_{round}"),
            role: "user".to_string(),
            content: format!("ROUND-{round}-USER-RAW {}", "需求细节 ".repeat(30)),
            checkpoint_id: None,
            created_at: format!("2026-08-21T00:00:{round:02}Z"),
        });
        session.messages.push(SessionMessage {
            id: format!("msg_author_{round}"),
            role: "assistant".to_string(),
            content: markdown.clone(),
            checkpoint_id: None,
            created_at: format!("2026-08-21T00:01:{round:02}Z"),
        });
        session.messages.push(SessionMessage {
            id: format!("msg_reviewer_{round}"),
            role: "reviewer".to_string(),
            content: format!(
                "[review_summary]\nround {round} verdict\n\n[review_findings]\n1. severity: suggestion\n   message: round {round} suggestion\n   evidence: artifact v{round}\n   required_action: optional"
            ),
            checkpoint_id: None,
            created_at: format!("2026-08-21T00:02:{round:02}Z"),
        });
        artifacts.push(ArtifactVersion {
            version: round,
            payload: ArtifactPayload::Markdown {
                markdown,
                diff: None,
            },
            generated_by: ProviderName::ClaudeCode,
            reviewed_by: Some(ProviderName::Codex),
            review_verdict: Some(ReviewVerdictType::Revise),
            confirmed_by: None,
            is_current: round == 4,
            created_at: format!("2026-08-21T00:03:{round:02}Z"),
            source_node_id: format!("author_round_{round}"),
        });
    }
    session.messages.insert(
        4,
        SessionMessage {
            id: "msg_choice".to_string(),
            role: "system".to_string(),
            content: "结构化交互审计记录（daemon 捕获）\n- choice_id: choice_rollout\n- answers:\n  - question_id: q1\n    selected: gradual = 分批发布\n- impacts: REQ-001, AC-001".to_string(),
            checkpoint_id: None,
            created_at: "2026-08-21T00:00:03Z".to_string(),
        },
    );
    session.artifact = Some(artifacts[3].payload.clone());
    let checkpoint_tmp = TempDir::new().unwrap();
    let mut engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
        event_tx,
        session,
    );
    engine.artifact_versions = artifacts;
    engine.latest_review_verdict = Some(ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "第 1 轮必须修复仍未关闭。".to_string(),
        summary: "第 1 轮追踪关系未关闭".to_string(),
        findings: vec![ReviewFinding {
            severity: ReviewFindingSeverity::MustFix,
            message: "ROUND-1-MUST-FIX-FULL-TEXT：REQ-001 必须追踪到 AC-001。".to_string(),
            evidence: "artifact v1 / REQ-001".to_string(),
            required_action: "补齐 REQ-001 -> AC-001 trace。".to_string(),
            category: None,
            class_hint: None,
            contract_field: None,
        }],
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    });

    let author = engine
        .build_streaming_input("新的 author 请求", AuthorPromptMode::FullConversation)
        .expect("author prompt")
        .prompt;
    let revision = engine.build_revision_full_prompt(
        engine
            .session
            .artifact
            .as_ref()
            .and_then(|artifact| artifact.markdown())
            .expect("current artifact"),
        engine.latest_review_verdict.as_ref().expect("review verdict"),
        &engine.routing_reference_context(),
    );
    let reviewer = engine.build_review_input().expect("review prompt").prompt;

    for (entrypoint, prompt) in [
        ("author", &author),
        ("revision", &revision),
        ("reviewer", &reviewer),
    ] {
        assert!(
            prompt.contains("[历史压缩摘要 round=1]"),
            "{entrypoint} must summarize early rounds: {prompt}"
        );
        assert!(
            !prompt.contains("ROUND-1-USER-RAW"),
            "{entrypoint} must not replay early raw rounds: {prompt}"
        );
        assert!(prompt.contains("ROUND-3-USER-RAW"), "{entrypoint}: {prompt}");
        assert!(prompt.contains("ROUND-4-USER-RAW"), "{entrypoint}: {prompt}");
        assert!(prompt.contains("choice_rollout"), "{entrypoint}: {prompt}");
        assert!(prompt.contains("gradual = 分批发布"), "{entrypoint}: {prompt}");
        assert!(prompt.contains("# Story Artifact v4"), "{entrypoint}: {prompt}");
    }
    assert!(reviewer.contains("完整 canonical inputs：不得裁剪。"));
    assert!(reviewer.contains("ROUND-1-MUST-FIX-FULL-TEXT"));
    assert!(reviewer.contains("artifact v1 -> v2 相邻版本差异摘要"));

    let full_replay_len = engine
        .session
        .messages
        .iter()
        .map(|message| message.role.len() + message.content.len() + 5)
        .sum::<usize>();
    assert!(
        author.len() < full_replay_len + 5_000,
        "prompt character-count decrease is a proxy metric only, not a release gate"
    );
}

#[test]
fn design_prompt_sliding_window_preserves_decision_audit_and_required_evidence() {
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut session = make_session("sess_design_prompt_sliding_window");
    session.workspace_type = WorkspaceType::Design;
    session.messages = vec![SessionMessage {
        id: "msg_000".to_string(),
        role: "system".to_string(),
        content: "Workspace 生成任务已准备\n\n[canonical_inputs]\n完整 Design canonical inputs：不得裁剪。\n\n[constraint_summary]\nDesign 历史约束。".to_string(),
        checkpoint_id: None,
        created_at: "2026-08-21T00:00:00Z".to_string(),
    }];
    let choice_audit = "结构化交互审计记录（daemon 捕获）\n- audit_kind: provider_choice_response\n- choice_id: author-decision-001\n- source: AskUserQuestion\n- provider_role: author\n- request_prompt: 请选择 Design 兼容策略。\n- answers:\n  - question_id: compatibility\n    question: API 是否保持旧客户端兼容？\n    selected: compatibility_first = 保持旧客户端兼容\n    free_text: 用户确认先保证现有调用方不受影响。\n- impacts: [DEC-002], [CMP-002], [API-002], [REQ-001], [AC-001]\n\n说明：以上记录由 daemon 捕获，是 author-decision/source claims 的可审计来源。";
    let mut artifacts = Vec::new();
    for round in 1..=4 {
        let markdown = complete_design_artifact(
            &format!(
                "[DEC-{round:03}] Design 决策正文 {}",
                "决策细节 ".repeat(30)
            ),
            &format!(
                "[API-{round:03}] API 契约正文 {}",
                "接口细节 ".repeat(30)
            ),
        )
        .replace("[DEC-001]", &format!("[DEC-{round:03}]"))
        .replace("[CMP-001]", &format!("[CMP-{round:03}]"))
        .replace("[API-001]", &format!("[API-{round:03}]"));
        session.messages.push(SessionMessage {
            id: format!("msg_design_user_{round}"),
            role: "user".to_string(),
            content: format!(
                "DESIGN-ROUND-{round}-USER-RAW {}",
                "用户设计细节 ".repeat(30)
            ),
            checkpoint_id: None,
            created_at: format!("2026-08-21T00:00:{round:02}Z"),
        });
        session.messages.push(SessionMessage {
            id: format!("msg_design_author_{round}"),
            role: "assistant".to_string(),
            content: markdown.clone(),
            checkpoint_id: None,
            created_at: format!("2026-08-21T00:01:{round:02}Z"),
        });
        if round == 2 {
            session.messages.push(SessionMessage {
                id: "msg_design_choice".to_string(),
                role: "system".to_string(),
                content: choice_audit.to_string(),
                checkpoint_id: None,
                created_at: "2026-08-21T00:01:30Z".to_string(),
            });
        }
        session.messages.push(SessionMessage {
            id: format!("msg_design_reviewer_{round}"),
            role: "reviewer".to_string(),
            content: format!(
                "[review_summary]\nDesign round {round} verdict\n\n[review_findings]\n1. severity: suggestion\n   message: Design round {round} suggestion\n   evidence: artifact v{round}\n   required_action: optional"
            ),
            checkpoint_id: None,
            created_at: format!("2026-08-21T00:02:{round:02}Z"),
        });
        artifacts.push(ArtifactVersion {
            version: round,
            payload: ArtifactPayload::Markdown {
                markdown,
                diff: None,
            },
            generated_by: ProviderName::ClaudeCode,
            reviewed_by: Some(ProviderName::Codex),
            review_verdict: Some(ReviewVerdictType::Revise),
            confirmed_by: None,
            is_current: round == 4,
            created_at: format!("2026-08-21T00:03:{round:02}Z"),
            source_node_id: format!("design_author_round_{round}"),
        });
    }
    let latest_artifact = artifacts[3]
        .payload
        .markdown()
        .expect("latest Design artifact markdown")
        .to_string();
    session.artifact = Some(artifacts[3].payload.clone());
    let checkpoint_tmp = TempDir::new().unwrap();
    let mut engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
        event_tx,
        session,
    );
    engine.artifact_versions = artifacts;
    engine.latest_review_verdict = Some(ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "Design 第 1 轮必须修复仍未关闭。".to_string(),
        summary: "Design 追踪关系未关闭".to_string(),
        findings: vec![ReviewFinding {
            severity: ReviewFindingSeverity::MustFix,
            message: "DESIGN-R1-MUST-FIX-FULL-TEXT：DEC-001 必须追踪到 API-001。".to_string(),
            evidence: "Design artifact v1 / DEC-001 / API-001".to_string(),
            required_action: "补齐 DEC-001 -> CMP-001 -> API-001 trace。".to_string(),
            category: None,
            class_hint: None,
            contract_field: None,
        }],
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    });

    let author = engine
        .build_streaming_input("新的 Design author 请求", AuthorPromptMode::FullConversation)
        .expect("author prompt")
        .prompt;
    let revision = engine.build_revision_full_prompt(
        engine
            .session
            .artifact
            .as_ref()
            .and_then(|artifact| artifact.markdown())
            .expect("current Design artifact"),
        engine.latest_review_verdict.as_ref().expect("review verdict"),
        &engine.routing_reference_context(),
    );
    let reviewer = engine.build_review_input().expect("review prompt").prompt;

    for (entrypoint, prompt) in [
        ("author", &author),
        ("revision", &revision),
        ("reviewer", &reviewer),
    ] {
        assert!(
            prompt.contains("[历史压缩摘要 round=1]")
                && prompt.contains("[历史压缩摘要 round=2]"),
            "{entrypoint} must summarize early Design rounds: {prompt}"
        );
        for raw in ["DESIGN-ROUND-1-USER-RAW", "DESIGN-ROUND-2-USER-RAW"] {
            assert!(
                !prompt.contains(raw),
                "{entrypoint} must not replay early Design raw round `{raw}`: {prompt}"
            );
        }
        for raw in ["DESIGN-ROUND-3-USER-RAW", "DESIGN-ROUND-4-USER-RAW"] {
            assert!(
                prompt.contains(raw),
                "{entrypoint} must retain recent Design raw round `{raw}`: {prompt}"
            );
        }
        assert!(
            prompt.contains(choice_audit),
            "{entrypoint} must retain the full author-decision choice audit: {prompt}"
        );
        assert!(
            prompt.contains(&latest_artifact),
            "{entrypoint} must retain the full latest Design artifact: {prompt}"
        );
        for required_finding_text in [
            "DESIGN-R1-MUST-FIX-FULL-TEXT：DEC-001 必须追踪到 API-001。",
            "Design artifact v1 / DEC-001 / API-001",
            "补齐 DEC-001 -> CMP-001 -> API-001 trace。",
        ] {
            assert!(
                prompt.contains(required_finding_text),
                "{entrypoint} must retain full open must_fix finding `{required_finding_text}`: {prompt}"
            );
        }
        for token in [
            "DEC-003", "CMP-003", "API-003", "DEC-004", "CMP-004", "API-004",
        ] {
            assert!(
                prompt.contains(token),
                "{entrypoint} must retain recent Design token `{token}`: {prompt}"
            );
        }
    }
    assert!(reviewer.contains("完整 Design canonical inputs：不得裁剪。"));
    assert!(reviewer.contains("artifact v1 -> v2 相邻版本差异摘要"));
}

// ============================================================================
// F3 restrict-role-write-tools Task 1.2：D2 角色×策略矩阵表驱动断言。
// ============================================================================

/// D2 矩阵分派 fixture：每个 entry 走真实构造路径，不虚构单一 role-fixture。
/// - workspace author/revision/review：WorkspaceEngine 真实 builder（part_31 式
///   session fixture）。
/// - coding coder/reviewer：`provider_retry.rs`（Coder/CodeReviewer 两锚点）、
///   `internal_pr_review.rs`、`group_review_orchestrator.rs` 的真实生产构造函数
///   （文件内具名构造函数，内部经共享工厂 `streaming_input_from_adapter` 派生
///   tool_policy——绕过工厂即绕过 D2 矩阵，策略断言必须失败）。
/// - 聚合初始化：`coordinator_provider_turn.inc.rs:57-77` 真实构造（gateway-backed
///   driver 的 `streaming_input`）。
/// - Handoff 无真实 builder，不参与本矩阵（由 Task 3.1 守卫测试以合成 input 覆盖）。
fn entry_input(entry: &str) -> StreamingProviderInput {
    use crate::cross_cutting::streaming_provider::ProviderPermissionMode;

    match entry {
        "sc_author" => {
            let (event_tx, _event_rx) = mpsc::channel(8);
            let mut session = make_session("sess_matrix_sc_author");
            session.artifact = Some(artifact_payload(
                "# Story Spec\n\n## 功能需求\n- [REQ-001] Draft.\n",
            ));
            let checkpoint_tmp = TempDir::new().expect("checkpoint tempdir");
            let engine = WorkspaceEngine::new(
                Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
                event_tx,
                session,
            );
            engine
                .build_streaming_input("开始生成", AuthorPromptMode::FullConversation)
                .expect("sc author input")
        }
        "sc_revision" => {
            let (event_tx, _event_rx) = mpsc::channel(8);
            let mut session = make_session("sess_matrix_sc_revision");
            session.artifact = Some(artifact_payload(
                "# Story Spec\n\n## 功能需求\n- [REQ-001] Draft.\n",
            ));
            let checkpoint_tmp = TempDir::new().expect("checkpoint tempdir");
            let mut engine = WorkspaceEngine::new(
                Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
                event_tx,
                session,
            );
            engine.latest_review_verdict = Some(ReviewVerdict {
                verdict: ReviewVerdictType::Revise,
                comments: "补充验收标准".to_string(),
                summary: "需要返修".to_string(),
                findings: Vec::new(),
                review_gate: ReviewGate::RequiresRevision,
                work_item_plan_review: None,
                structured_output_diagnostic: None,
            });
            engine.build_revision_input().expect("sc revision input")
        }
        "wip_author" => {
            let (event_tx, _event_rx) = mpsc::channel(8);
            let session = make_session("sess_matrix_wip_author");
            let checkpoint_tmp = TempDir::new().expect("checkpoint tempdir");
            let engine = WorkspaceEngine::new(
                Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
                event_tx,
                session,
            );
            engine.build_work_item_plan_streaming_input(
                ProviderType::ClaudeCode,
                "work item plan prompt".to_string(),
                checkpoint_tmp.path().to_string_lossy().to_string(),
                ProviderName::ClaudeCode,
            )
        }
        "workspace_reviewer" => {
            let (event_tx, _event_rx) = mpsc::channel(8);
            let mut session = make_session("sess_matrix_workspace_reviewer");
            session.artifact = Some(artifact_payload(
                "# Story Spec\n\n## 功能需求\n- [REQ-001] Draft.\n",
            ));
            let checkpoint_tmp = TempDir::new().expect("checkpoint tempdir");
            let engine = WorkspaceEngine::new(
                Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
                event_tx,
                session,
            );
            engine.build_review_input().expect("workspace reviewer input")
        }
        "coding_coder" => {
            // provider_retry.rs Coder 锚点的真实生产构造函数：Executor，D2 禁带策略。
            let worktree = tempfile::tempdir().expect("coding worktree").keep();
            let (_legacy, input) =
                crate::product::coding_workspace_engine::coder_retry_cycle_streaming_input(
                    &ProviderName::Codex,
                    "coding coder prompt".to_string(),
                    &worktree,
                    "provider_stream_log_dir".to_string(),
                    "attempt_matrix_coder",
                    None,
                    ProviderPermissionMode::Auto,
                );
            input
        }
        "coding_reviewer" => {
            // internal_pr_review.rs 锚点的真实生产构造函数：InternalReviewer=
            // Reviewer，D2 必带。
            let worktree = tempfile::tempdir().expect("coding worktree").keep();
            let (_legacy, input) =
                crate::product::coding_workspace_engine::internal_pr_review_streaming_input(
                    &ProviderName::Codex,
                    "coding reviewer prompt".to_string(),
                    &worktree,
                    "provider_stream_log_dir".to_string(),
                    "attempt_matrix_internal_reviewer",
                    None,
                    ProviderPermissionMode::Auto,
                );
            input
        }
        "coding_code_reviewer" => {
            // provider_retry.rs CodeReviewer 锚点的真实生产构造函数：Reviewer，
            // D2 必带（structured output contract 用生产同款构造器）。
            let worktree = tempfile::tempdir().expect("coding worktree").keep();
            let (_legacy, input) =
                crate::product::coding_workspace_engine::code_reviewer_retry_cycle_streaming_input(
                    &ProviderName::Codex,
                    "coding code reviewer prompt".to_string(),
                    &worktree,
                    "provider_stream_log_dir".to_string(),
                    "attempt_matrix_code_reviewer",
                    None,
                    crate::product::coding_workspace_engine::code_review_structured_output_contract(
                        "nonce1".to_string(),
                    ),
                    ProviderPermissionMode::Auto,
                );
            input
        }
        "coding_group_reviewer" => {
            // group_review_orchestrator.rs 锚点的真实生产构造函数：group
            // review shard/reduction=Reviewer，D2 必带。
            let worktree = tempfile::tempdir().expect("coding worktree").keep();
            let (_legacy, input) =
                crate::product::coding_workspace_engine::group_review_streaming_input(
                    &ProviderName::Codex,
                    "coding group review prompt".to_string(),
                    &worktree,
                    "provider_stream_log_dir".to_string(),
                    "attempt_matrix_group_reviewer",
                    ProviderPermissionMode::Auto,
                );
            input
        }
        "aggregate_turn" => {
            // coordinator_provider_turn.inc.rs:57-77 真实构造：聚合初始化 provider turn
            // = Executor，需写配置文件，D2 禁带策略（None 是正确值）。
            let fixture = review_gateway_fixture();
            let driver =
                crate::product::logical_codebase::aggregate_initialization_coordinator::GatewayBackedAggregateProviderTurnDriver::claude_code(
                    fixture.gateway.clone(),
                    "cap_claude_code_1_4_0",
                );
            driver.streaming_input(
                crate::product::logical_codebase::AggregateInitializationStepKind::RuleAndMcpConfig,
                &fixture.worktree,
            )
        }
        other => panic!("unknown matrix entry: {other}"),
    }
}

#[test]
fn builder_factory_applies_role_policy_matrix_per_entry() {
    use crate::cross_cutting::streaming_provider::{
        ProviderToolPolicy, ToolPolicyIntent as PolicyIntent,
    };

    for (entry, role, denied) in [
        ("sc_author", AdapterRole::Orchestrator, true),
        ("sc_revision", AdapterRole::Orchestrator, true),
        ("wip_author", AdapterRole::WorkItemSplitter, true),
        ("workspace_reviewer", AdapterRole::Reviewer, true),
        ("coding_coder", AdapterRole::Executor, false),
        ("coding_reviewer", AdapterRole::Reviewer, true),
        ("coding_code_reviewer", AdapterRole::Reviewer, true),
        ("coding_group_reviewer", AdapterRole::Reviewer, true),
        ("aggregate_turn", AdapterRole::Executor, false),
    ] {
        let input = entry_input(entry);
        assert_eq!(input.role, role, "{entry}");
        assert_eq!(input.tool_policy.is_some(), denied, "{entry}");
        if denied {
            // 必带的语义即唯一合法意图 DenyFileWriteBuiltins（黑名单，非 allowlist）。
            assert!(
                matches!(
                    input.tool_policy,
                    Some(ProviderToolPolicy {
                        intent: PolicyIntent::DenyFileWriteBuiltins
                    })
                ),
                "{entry} must carry exactly DenyFileWriteBuiltins"
            );
        }
    }
}

/// F3 Task 4.1 零变化回归：Coder 与聚合初始化 provider turn 保持既有全工具
/// 启动档（无 denylist、无沙箱改写）。聚合 turn 固定 Claude Code（Codex 在
/// danger-full-access 下被 gateway 路由级阻断），与 Codex 沙箱参数无关——
/// 聚合侧按真实 input 断言 launch 档字段（provider_type/role/tool_policy/
/// permission_mode），codex sandbox 参数仅在真实 Codex input（coder）上经
/// `codex_launch_params` 单一来源断言 `danger-full-access`；若 Coder 被误注入
/// denylist 或沙箱被改写，本测试必须失败。
#[test]
fn coder_and_aggregate_executor_keep_existing_full_tool_launch() {
    use crate::cross_cutting::codex_provider::session::codex_launch_params;

    let coder = entry_input("coding_coder");
    assert_eq!(coder.role, AdapterRole::Executor);
    assert_eq!(coder.tool_policy, None);
    assert_eq!(coder.provider_type, ProviderType::Codex);
    assert_eq!(coder.permission_mode, ProviderPermissionMode::Auto);
    assert_eq!(
        codex_launch_params(&coder)["sandbox"],
        "danger-full-access",
        "Coder must keep the existing full-tool sandbox"
    );

    // 聚合初始化同 Executor 档：经 coordinator_provider_turn.inc.rs:57-77 真实
    // 构造路径断言（同 Task 1.2 builder 全集断言），同样禁带策略、维持全工具档；
    // 该 input 是 Claude Code 档，不喂 codex_launch_params（类型不匹配的弱断言）。
    let aggregate = entry_input("aggregate_turn");
    assert_eq!(aggregate.provider_type, ProviderType::ClaudeCode);
    assert_eq!(aggregate.role, AdapterRole::Executor);
    assert_eq!(aggregate.tool_policy, None);
    assert_eq!(aggregate.permission_mode, ProviderPermissionMode::Auto);
}

/// 断言 role 与 tool_policy 成对一致（D2：作者/评审必带 DenyFileWriteBuiltins，
/// Executor/Coder/聚合初始化禁带）。
fn assert_role_policy_pair(
    input: &StreamingProviderInput,
    entry: &str,
    role: AdapterRole,
    denied: bool,
) {
    assert_eq!(input.role, role, "{entry}");
    assert_eq!(input.tool_policy.is_some(), denied, "{entry}");
    if denied {
        use crate::cross_cutting::streaming_provider::ToolPolicyIntent;
        assert!(
            matches!(
                input.tool_policy,
                Some(crate::cross_cutting::streaming_provider::ProviderToolPolicy {
                    intent: ToolPolicyIntent::DenyFileWriteBuiltins
                })
            ),
            "{entry} must carry exactly DenyFileWriteBuiltins"
        );
    }
}

/// builder 全集断言：逐一调用 D2 全表真实 builder（含 review 家族各入口、
/// revision 三变体、WorkItemPlan author 普通/fresh/with-session、review repair、
/// coding 工厂全 AdapterRole 矩阵与聚合初始化）。
#[tokio::test]
async fn workspace_builder_family_pairs_role_with_tool_policy() {
    let mut covered: Vec<(String, StreamingProviderInput)> = Vec::new();

    // —— SC author / revision 家族（三链共用 builder）——
    {
        let (event_tx, _event_rx) = mpsc::channel(8);
        let mut session = make_session("sess_family_sc_author");
        session.artifact = Some(artifact_payload(
            "# Story Spec\n\n## 功能需求\n- [REQ-001] Draft.\n",
        ));
        let checkpoint_tmp = TempDir::new().expect("checkpoint tempdir");
        let engine = WorkspaceEngine::new(
            Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
            event_tx,
            session,
        );
        covered.push((
            "sc_author".to_string(),
            engine
                .build_streaming_input("开始生成", AuthorPromptMode::FullConversation)
                .expect("author input"),
        ));
    }
    {
        let (event_tx, _event_rx) = mpsc::channel(8);
        let mut session = make_session("sess_family_sc_revision");
        session.artifact = Some(artifact_payload(
            "# Story Spec\n\n## 功能需求\n- [REQ-001] Draft.\n",
        ));
        let checkpoint_tmp = TempDir::new().expect("checkpoint tempdir");
        let mut engine = WorkspaceEngine::new(
            Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
            event_tx,
            session,
        );
        engine.latest_review_verdict = Some(ReviewVerdict {
            verdict: ReviewVerdictType::Revise,
            comments: "补充验收标准".to_string(),
            summary: "需要返修".to_string(),
            findings: Vec::new(),
            review_gate: ReviewGate::RequiresRevision,
            work_item_plan_review: None,
            structured_output_diagnostic: None,
        });
        covered.push((
            "sc_revision_resume".to_string(),
            engine.build_revision_input().expect("revision input"),
        ));
        covered.push((
            "sc_revision_without_resume".to_string(),
            engine
                .build_revision_input_without_resume()
                .expect("revision input without resume"),
        ));
        covered.push((
            "sc_revision_with_resume_false".to_string(),
            engine
                .build_revision_input_with_resume(false)
                .expect("revision input with resume false"),
        ));
        // SC reviewer（三链共用 review builder）。
        covered.push((
            "workspace_reviewer".to_string(),
            engine.build_review_input().expect("review input"),
        ));
        // review repair（结构化输出修复，三链共用）。
        let base_input = engine.build_review_input().expect("review input");
        let mut completion = ProviderCompletion::plain(
            "<ARIA_STRUCTURED_OUTPUT nonce=\"stale_nonce_0001\">partial structured review",
            None,
        );
        completion.readable_output = "partial structured review".to_string();
        let parse_error =
            crate::product::workspace_engine::review::ReviewCompletionError::Syntax(
                crate::cross_cutting::structured_output::StructuredOutputError {
                    code: crate::cross_cutting::structured_output::StructuredOutputErrorCode::MissingEndTag,
                    message: "missing end tag".to_string(),
                    expected_nonce: Some("nonce_0001".to_string()),
                    observed_nonce: None,
                    recoverable_value: Some(serde_json::json!({
                        "verdict": "pass",
                        "summary": "unchanged",
                        "findings": []
                    })),
                },
            );
        covered.push((
            "review_repair".to_string(),
            engine
                .build_review_repair_input(&base_input, &completion, &parse_error, None)
                .expect("repair input"),
        ));
    }

    // —— WorkItemPlan author 家族：普通 / fresh（StaleContext 重建）/ with-session。
    // serial/batch draft（draft_batch/runs.rs）与普通 author 共用同一 builder 链。
    {
        let (event_tx, _event_rx) = mpsc::channel(8);
        let session = make_session("sess_family_wip_author");
        let checkpoint_tmp = TempDir::new().expect("checkpoint tempdir");
        let worktree = checkpoint_tmp.path().to_string_lossy().to_string();
        let engine = WorkspaceEngine::new(
            Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
            event_tx,
            session,
        );
        covered.push((
            "wip_author_normal".to_string(),
            engine.build_work_item_plan_streaming_input(
                ProviderType::ClaudeCode,
                "plan prompt".to_string(),
                worktree.clone(),
                ProviderName::ClaudeCode,
            ),
        ));
        covered.push((
            "wip_author_fresh".to_string(),
            engine.build_work_item_plan_streaming_input_fresh(
                ProviderType::ClaudeCode,
                "plan prompt".to_string(),
                worktree.clone(),
                ProviderName::ClaudeCode,
            ),
        ));
        covered.push((
            "wip_author_with_session".to_string(),
            engine.build_work_item_plan_streaming_input_with_session(
                ProviderType::ClaudeCode,
                "plan prompt".to_string(),
                worktree,
                ProviderName::ClaudeCode,
                None,
            ),
        ));
    }

    // —— WorkItemPlan review 家族：dispatcher（candidate 分支）/ outline / batch /
    // draft（single-candidate 与 projection 分派分支在各自既有测试中断言）。
    {
        let (_tmp, _checkpoint_store, _lifecycle, plan_id, mut engine) =
            make_work_item_plan_engine_with_draft_candidate("sess_family_wip_review");
        covered.push((
            "wip_plan_review_candidate".to_string(),
            engine
                .build_work_item_plan_review_input()
                .expect("plan review input"),
        ));
        prepare_work_item_plan_outline_artifact(&mut engine).await;
        let outline_payload = work_item_plan_outline_artifact();
        let ArtifactPayload::WorkItemPlanOutlineCandidate { outline_candidate } = outline_payload
        else {
            panic!("expected outline candidate artifact");
        };
        covered.push((
            "wip_outline_review".to_string(),
            engine
                .build_work_item_plan_outline_review_input(&outline_candidate)
                .expect("outline review input"),
        ));
        save_batch_work_item_plan_index_with_accepted_drafts(&engine, &plan_id);
        covered.push((
            "wip_batch_review".to_string(),
            engine
                .build_work_item_batch_review_input()
                .expect("batch review input"),
        ));
        save_serial_work_item_plan_index(&engine, &plan_id, "outline_a");
        let draft_payload = work_item_draft_artifact_payload(
            &plan_id,
            "outline_a",
            "draft_a",
            WorkItemDraftStatus::Draft,
        );
        let ArtifactPayload::WorkItemDraftCandidate { draft_candidate } = draft_payload else {
            panic!("expected draft candidate artifact");
        };
        covered.push((
            "wip_draft_review".to_string(),
            engine
                .build_work_item_draft_review_input(&draft_candidate)
                .expect("draft review input"),
        ));
    }

    // —— coding 四锚点的真实生产构造函数（provider_retry Coder/CodeReviewer、
    // internal_pr_review、group_review；均经共享工厂派生 tool_policy，绕过工厂
    // 即绕过 D2 矩阵）——
    {
        let worktree = tempfile::tempdir().expect("coding worktree").keep();
        let (_legacy, input) =
            crate::product::coding_workspace_engine::coder_retry_cycle_streaming_input(
                &ProviderName::Codex,
                "coding coder prompt".to_string(),
                &worktree,
                "provider_stream_log_dir".to_string(),
                "attempt_family_coder",
                None,
                crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
            );
        covered.push(("coding_coder_retry".to_string(), input));
    }
    {
        let worktree = tempfile::tempdir().expect("coding worktree").keep();
        let (_legacy, input) =
            crate::product::coding_workspace_engine::code_reviewer_retry_cycle_streaming_input(
                &ProviderName::Codex,
                "coding code reviewer prompt".to_string(),
                &worktree,
                "provider_stream_log_dir".to_string(),
                "attempt_family_code_reviewer",
                None,
                crate::product::coding_workspace_engine::code_review_structured_output_contract(
                    "nonce1".to_string(),
                ),
                crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
            );
        covered.push(("coding_code_reviewer_retry".to_string(), input));
    }
    {
        let worktree = tempfile::tempdir().expect("coding worktree").keep();
        let (_legacy, input) =
            crate::product::coding_workspace_engine::internal_pr_review_streaming_input(
                &ProviderName::Codex,
                "coding reviewer prompt".to_string(),
                &worktree,
                "provider_stream_log_dir".to_string(),
                "attempt_family_internal_reviewer",
                None,
                crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
            );
        covered.push(("coding_internal_pr_review".to_string(), input));
    }
    {
        let worktree = tempfile::tempdir().expect("coding worktree").keep();
        let (_legacy, input) =
            crate::product::coding_workspace_engine::group_review_streaming_input(
                &ProviderName::Codex,
                "coding group review prompt".to_string(),
                &worktree,
                "provider_stream_log_dir".to_string(),
                "attempt_family_group_reviewer",
                crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
            );
        covered.push(("coding_group_review".to_string(), input));
    }

    // —— 共享工厂全 AdapterRole 派生矩阵（工厂级锁定：Orchestrator/
    // WorkItemSplitter/Handoff 在 coding 无真实锚点，由工厂派生覆盖）——
    for role in [
        AdapterRole::Executor,
        AdapterRole::Reviewer,
        AdapterRole::Orchestrator,
        AdapterRole::WorkItemSplitter,
        AdapterRole::Handoff,
    ] {
        let worktree = tempfile::tempdir().expect("coding worktree").keep();
        let legacy_input = AdapterInput {
            provider_type: ProviderType::Codex,
            role: role.clone(),
            worktree_path: Some(worktree.to_string_lossy().to_string()),
            provider_stream_log_dir: None,
            prompt: "coding factory role matrix".to_string(),
            context_files: Vec::new(),
            output_schema: "coding_workspace_markdown".to_string(),
            timeout: 30,
            max_retries: 0,
        };
        covered.push((
            format!("coding_factory_derives_{role:?}"),
            crate::product::coding_workspace_engine::streaming_input_from_adapter(
                &legacy_input,
                worktree,
                crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
            ),
        ));
    }

    // —— 聚合初始化 provider turn（gateway validated，需写配置文件，禁带）——
    covered.push(("aggregate_turn".to_string(), entry_input("aggregate_turn")));

    let expected = [
        ("sc_author", AdapterRole::Orchestrator, true),
        ("sc_revision_resume", AdapterRole::Orchestrator, true),
        ("sc_revision_without_resume", AdapterRole::Orchestrator, true),
        (
            "sc_revision_with_resume_false",
            AdapterRole::Orchestrator,
            true,
        ),
        ("workspace_reviewer", AdapterRole::Reviewer, true),
        ("review_repair", AdapterRole::Reviewer, true),
        ("wip_author_normal", AdapterRole::WorkItemSplitter, true),
        ("wip_author_fresh", AdapterRole::WorkItemSplitter, true),
        (
            "wip_author_with_session",
            AdapterRole::WorkItemSplitter,
            true,
        ),
        ("wip_plan_review_candidate", AdapterRole::Reviewer, true),
        ("wip_outline_review", AdapterRole::Reviewer, true),
        ("wip_batch_review", AdapterRole::Reviewer, true),
        ("wip_draft_review", AdapterRole::Reviewer, true),
        ("coding_coder_retry", AdapterRole::Executor, false),
        ("coding_code_reviewer_retry", AdapterRole::Reviewer, true),
        ("coding_internal_pr_review", AdapterRole::Reviewer, true),
        ("coding_group_review", AdapterRole::Reviewer, true),
        ("coding_factory_derives_Executor", AdapterRole::Executor, false),
        (
            "coding_factory_derives_Reviewer",
            AdapterRole::Reviewer,
            true,
        ),
        (
            "coding_factory_derives_Orchestrator",
            AdapterRole::Orchestrator,
            true,
        ),
        (
            "coding_factory_derives_WorkItemSplitter",
            AdapterRole::WorkItemSplitter,
            true,
        ),
        (
            "coding_factory_derives_Handoff",
            AdapterRole::Handoff,
            false,
        ),
        ("aggregate_turn", AdapterRole::Executor, false),
    ];
    assert_eq!(
        covered.len(),
        expected.len(),
        "family coverage must enumerate every D2 builder entry"
    );
    for ((entry, input), (expected_entry, role, denied)) in covered.iter().zip(expected.iter()) {
        assert_eq!(entry, expected_entry, "builder family order drifted");
        assert_role_policy_pair(input, entry, role.clone(), *denied);
    }
}
