// markdown author prompt 契约测试从 prompt_contract.rs 拆入（large_file_guard
// 1667 行登记上限）：D5（REQ-WSC-06「基线树事实直供」）基线树测试 + 相邻
// 同主题的 cross-reference discipline 契约测试；测试逻辑与断言零改动。
// 本模块自带语言规则夹具（`TEST_SINGLE_CANDIDATE_LANGUAGE_RULES`，同
// prompt_contract.rs）。

const TEST_SINGLE_CANDIDATE_LANGUAGE_RULES: &str =
    "## 语言规则\n\n- **必须使用中文** - 所有响应、解释、注释和文档必须使用中文。";

#[test]
fn work_item_plan_markdown_prompt_keeps_cross_reference_discipline_when_design_list_is_empty() {
    let (request, issue, repository) = split_prompt_fixture();
    let prompt =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_prompt(
            &request,
            &issue,
            &repository,
            crate::product::work_item_split_engine::prompts::WorkItemPlanMarkdownAuthorContext {
                story_context: "story_spec_0001: level selection",
                design_context: "design_spec_0001: levels API",
                design_requirement_ids: &[],
                repository_structure: "src/product/levels; web/src/levels; tests/integration",
                language_rules: TEST_SINGLE_CANDIDATE_LANGUAGE_RULES,
                plan_options: &TEST_ALL_FALSE_PLAN_OPTIONS,
                routing_context: &RoutingReferenceContext::Legacy,
                member_targets: None,
                change_order: &[],
                baseline_tree: None,
            },
        )
        .expect("markdown author prompt");

    for required in [
        "[design_requirements] （无；不编造）",
        "[cross_reference_discipline]",
        "handoff 的 reviewer_check_refs 与全部且仅本 item 的 acceptance criterion ID 集合完全一致（每条 AC 恰好被检查一次）",
        "同一 Work Item 内同一 command 至多声明一次；复合验证合并为一条 check 或改用 manual_instruction",
        "每个被 tasks 的 requirement_refs 引用的 requirement_id，在本 item 的 Traceability section 有逐字相同的登记行；登记值只来自 [design_requirements] 清单",
        "task_id、criterion_id、check_id、contract_id 全文档全局唯一且全局递增（第二个 Work Item 从 TASK-004、AC-004 续起，不在 item 内重新编号）",
    ] {
        assert!(
            prompt.contains(required),
            "markdown prompt must retain cross-reference discipline with an empty design list; missing {required}: {prompt}"
        );
    }

    // Task 7（REQ-CRP-05 计量闸门）：重复 trusted command 的 lowering_error
    // 引文与正反例已退出 prompt——重复引用的判定由编译器终局校验承接，修复由
    // workspace_engine/tests/contract_autorepair_migration.rs::
    // duplicate_trusted_command_both_branches_in_instruction 产的 author 指令承接。
    for retired in [
        "「同一 Work Item 不得重复引用 trusted command。」",
        "反例：同一 item 的 CHECK-001 与 CHECK-002 都写 `- command: npm test` → lowering_error 拒绝。",
        "正例：合并为一条 `- command: npm test` 的 check；或第二条写 `- command: null` 并配 `- manual_instruction: <人工步骤>`。",
    ] {
        assert!(
            !prompt.contains(retired),
            "REQ-CRP-05 计量闸门：已退出 prompt 的机械条款不得回潮：{retired}"
        );
    }
}

// ---------------------------------------------------------------------------
// D5 事前（REQ-WSC-06「基线树事实直供」「基线树清单不得占用固定规范面预算」）：
// 作者轮启动前装载校验器同源的 plan 基线树（plan_preflight::plan_baseline_tree），
// 作为 L1 扩展注入 [confirmed_context] 的 `baseline:` 段（上下文面，32,000 度量）。
// 缺席语义：无仓 issue（Ok(None)）/ 装载失败（Err）/ 空树 ⇒ 该段整段缺席
// （不留 baseline: 空标记），既有 fixture prompt 逐字节不变；装载失败不阻断
// 作者轮（既有 fail-closed 点仍在 Evaluate 阶段）。
// ---------------------------------------------------------------------------

#[test]
fn markdown_author_prompt_injects_baseline_tree_section() {
    let (request, issue, repository) = split_prompt_fixture();
    let design_requirement_ids = vec!["REQ-002".to_string()];
    let prompt =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_prompt(
            &request,
            &issue,
            &repository,
            crate::product::work_item_split_engine::prompts::WorkItemPlanMarkdownAuthorContext {
                story_context: "story_spec_0001: level selection",
                design_context: "design_spec_0001: levels API",
                design_requirement_ids: &design_requirement_ids,
                repository_structure: "busi; frontend",
                language_rules: TEST_SINGLE_CANDIDATE_LANGUAGE_RULES,
                plan_options: &TEST_ALL_FALSE_PLAN_OPTIONS,
                routing_context: &RoutingReferenceContext::Legacy,
                member_targets: None,
                change_order: &[],
                baseline_tree: Some("busi: busi/src/main/java\nfrontend: frontend/src/App.vue"),
            },
        )
        .expect("markdown author prompt with baseline tree");
    assert!(
        prompt.contains("baseline:"),
        "基线树在场时 author prompt 必须直供 baseline: 段（事实，而非仅纪律）"
    );
    assert!(
        prompt.contains("frontend/src/App.vue"),
        "baseline: 段必须逐字携带基线树路径"
    );
    assert!(
        prompt.contains("busi: busi/src/main/java"),
        "baseline: 段必须按成员仓分组呈现路径"
    );
    // 注入位置：[confirmed_context] 段内、story_spec_ids 之前（上下文面，
    // 不挤占固定规范面质量预算）。
    let context_pos = prompt
        .find("[confirmed_context]")
        .expect("confirmed context section");
    let baseline_pos = prompt.find("baseline:").expect("baseline section");
    let spec_ids_pos = prompt.find("story_spec_ids:").expect("story spec ids");
    assert!(
        context_pos < baseline_pos && baseline_pos < spec_ids_pos,
        "baseline: 段必须落在 [confirmed_context] 内、story_spec_ids 之前"
    );
}

/// D5 事前边界：story/design/structure **各自占满配额**时，基线树清单
/// MUST NOT 挤破上下文面。
///
/// 钉的是 owner 2026-11-10 裁决的**两面归属**语义：清单计入上下文面
/// 同一配额，从三段压缩正文之后的**真实剩余额度**取；额度为 0 ⇒ 整段
/// 缺席 ⇒ 带清单与不带清单的上下文面**逐字节同长**。
///
/// 口径说明（既有事实，非本 change 引入）：`story:`/`design:`/
/// `structure:`/`story_spec_ids:` 等固定开销**未被**计入 32,000 的
/// 三段配额，故三段占满时上下文面**既有**地超出 32,000 数十字节。本
/// change 不擅自修正该既有溢出（不扩范围），但 MUST NOT 让清单**新增**
/// 溢出 —— 故此处断言「带清单 ≡ 不带清单」。
#[test]
fn markdown_author_prompt_context_face_stays_within_budget_when_all_faces_are_full() {
    let (request, issue, repository) = split_prompt_fixture();
    let design_requirement_ids = vec!["REQ-002".to_string()];
    // 三段各自远超自身配额（story 14,400 / design 14,400 / structure 3,200）。
    let story = "S".repeat(40_000);
    let design = "D".repeat(40_000);
    let structure = "R".repeat(20_000);
    let member_dirs = vec![
        "busi".to_string(),
        "frontend".to_string(),
        "gateway".to_string(),
    ];
    let mut tree = std::collections::BTreeSet::new();
    for idx in 0..500 {
        tree.insert(format!("busi/src/main/java/service/backend/{idx:03}.java"));
        tree.insert(format!("frontend/src/views/notice/page/{idx:03}.vue"));
        tree.insert(format!("gateway/policy/rules/route_rule_{idx:03}.yaml"));
    }
    let baseline =
        crate::product::work_item_split_engine::prompts::render_baseline_tree_section(
            &tree,
            &member_dirs,
        );

    let face_bytes = |prompt: &str| -> usize {
        let start = prompt
            .find("[confirmed_context]")
            .expect("confirmed context section")
            + "[confirmed_context]".len();
        let end = prompt
            .find("[source_boundary]")
            .expect("source boundary after confirmed context");
        prompt[start..end].len()
    };

    let build = |baseline_tree: Option<&str>| {
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_prompt(
            &request,
            &issue,
            &repository,
            crate::product::work_item_split_engine::prompts::WorkItemPlanMarkdownAuthorContext {
                story_context: &story,
                design_context: &design,
                design_requirement_ids: &design_requirement_ids,
                repository_structure: &structure,
                language_rules: TEST_SINGLE_CANDIDATE_LANGUAGE_RULES,
                plan_options: &TEST_ALL_FALSE_PLAN_OPTIONS,
                routing_context: &RoutingReferenceContext::Legacy,
                member_targets: None,
                change_order: &[],
                baseline_tree,
            },
        )
        .expect("markdown author prompt")
    };

    let without = build(None);
    let with = build(Some(&baseline));
    assert_eq!(
        face_bytes(&with),
        face_bytes(&without),
        "三段占满上下文面时，基线树清单 MUST 整段缺席（额度为 0），MUST NOT 新增任何上下文面字节"
    );
    assert!(
        !with.contains("baseline:"),
        "额度为 0 时 MUST NOT 留 baseline: 空标记"
    );
}

#[test]
fn markdown_author_prompt_omits_baseline_section_when_tree_absent() {
    let (request, issue, repository) = split_prompt_fixture();
    let design_requirement_ids = vec!["REQ-002".to_string()];
    let prompt =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_prompt(
            &request,
            &issue,
            &repository,
            crate::product::work_item_split_engine::prompts::WorkItemPlanMarkdownAuthorContext {
                story_context: "story_spec_0001: level selection",
                design_context: "design_spec_0001: levels API",
                design_requirement_ids: &design_requirement_ids,
                repository_structure: "busi; frontend",
                language_rules: TEST_SINGLE_CANDIDATE_LANGUAGE_RULES,
                plan_options: &TEST_ALL_FALSE_PLAN_OPTIONS,
                routing_context: &RoutingReferenceContext::Legacy,
                member_targets: None,
                change_order: &[],
                baseline_tree: None,
            },
        )
        .expect("markdown author prompt without baseline tree");
    assert!(
        !prompt.contains("baseline:"),
        "基线树不可得（无仓 issue）时 baseline: 段必须整段缺席，不留空标记"
    );
}

#[test]
fn markdown_author_prompt_with_baseline_tree_stays_within_context_budget() {
    let (request, issue, repository) = split_prompt_fixture();
    // 大仓压力形态：3 成员仓 × 500 路径（1,500 条）远超 8,000 B 渲染上限，
    // 必须按成员仓轮转保底截断。
    let member_dirs = vec![
        "busi".to_string(),
        "frontend".to_string(),
        "gateway".to_string(),
    ];
    let mut tree = std::collections::BTreeSet::new();
    for idx in 0..500 {
        tree.insert(format!("busi/src/main/java/service/backend/{idx:03}.java"));
        tree.insert(format!("frontend/src/views/notice/page/{idx:03}.vue"));
        tree.insert(format!("gateway/policy/rules/route_rule_{idx:03}.yaml"));
    }
    let baseline =
        crate::product::work_item_split_engine::prompts::render_baseline_tree_section(
            &tree,
            &member_dirs,
        );
    assert!(
        baseline.len() <= crate::product::work_item_split_engine::prompts::WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES / 4,
        "大仓基线树渲染必须受上下文配额 1/4（8,000 B）约束，实测 {} B",
        baseline.len()
    );
    for member in [&member_dirs[0], &member_dirs[1], &member_dirs[2]] {
        assert!(
            baseline.contains(&format!("{member}: ")),
            "轮转保底截断不得把成员仓 {member} 整体挤掉"
        );
    }
    let design_requirement_ids = vec!["REQ-002".to_string()];
    let prompt =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_prompt(
            &request,
            &issue,
            &repository,
            crate::product::work_item_split_engine::prompts::WorkItemPlanMarkdownAuthorContext {
                story_context: "story_spec_0001: level selection",
                design_context: "design_spec_0001: levels API",
                design_requirement_ids: &design_requirement_ids,
                repository_structure: "busi; frontend; gateway",
                language_rules: TEST_SINGLE_CANDIDATE_LANGUAGE_RULES,
                plan_options: &TEST_ALL_FALSE_PLAN_OPTIONS,
                routing_context: &RoutingReferenceContext::Legacy,
                member_targets: None,
                change_order: &[],
                baseline_tree: Some(&baseline),
            },
        )
        .expect("markdown author prompt with truncated baseline tree");
    // 上下文面度量：[confirmed_context] 段（到 [source_boundary] 为止）的
    // 字节数必须仍在 32,000 内——清单走上下文面，不因直供顶破预算。
    let context_start = prompt
        .find("[confirmed_context]")
        .expect("confirmed context section")
        + "[confirmed_context]".len();
    let context_end = prompt
        .find("[source_boundary]")
        .expect("source boundary after confirmed context");
    let context_face_bytes = prompt[context_start..context_end].len();
    assert!(
        context_face_bytes
            <= crate::product::work_item_split_engine::prompts::WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES,
        "基线树清单走上下文面：[confirmed_context] 段 {context_face_bytes} B 超出 32,000 B 预算"
    );
    for member in [&member_dirs[0], &member_dirs[1], &member_dirs[2]] {
        assert!(
            prompt[context_start..context_end].contains(&format!("{member}: ")),
            "截断后成员仓 {member} 的路径必须仍在场（不得整体挤掉）"
        );
    }
}

#[test]
fn author_baseline_tree_section_failure_semantics_keep_author_round_startable() {
    use crate::product::work_item_split_engine::prompts::author_baseline_tree_section;
    // 装载失败（Err，如基线分支被删）⇒ 仅记诊断、该段缺席，作者轮不被阻断；
    // 既有 fail-closed 点仍在 Evaluate 阶段。
    assert!(author_baseline_tree_section(
        Err("git ls-tree refs/heads/feat-x failed: fatal: bad object".to_string()),
        &["busi".to_string()],
    )
    .is_none());
    // 无仓 issue（Ok(None)）⇒ 缺席，零回归。
    assert!(
        author_baseline_tree_section(Ok(None), &["busi".to_string()]).is_none(),
        "无仓 issue 时 baseline 段必须缺席"
    );
    // 空树 ⇒ 同样缺席，不注入空 baseline: 标记。
    assert!(
        author_baseline_tree_section(Ok(Some(std::collections::BTreeSet::new())), &["busi".to_string()])
            .is_none(),
        "空基线树不得注入空 baseline: 标记"
    );
    // 在场 ⇒ 按成员仓分组渲染；**未命中成员仓**的路径按**自身首段**分组
    // （根文件归根文件组）。不产出误导性的「未归属」归并组：当前
    // `plan_baseline_tree` 返回的是单仓**成员内相对路径**，与聚合根目录名
    // 形态的 member_dirs 不同域，硬套会掩盖「这份清单只覆盖一个仓」这一事实
    // （见 design.md 登记的多仓基线树覆盖缺陷）。
    let mut tree = std::collections::BTreeSet::new();
    tree.insert("busi/src/a.java".to_string());
    tree.insert("frontend/src/b.vue".to_string());
    tree.insert("README.md".to_string());
    let section = author_baseline_tree_section(Ok(Some(tree)), &["busi".to_string()])
        .expect("baseline tree section");
    assert!(
        section.contains("busi: busi/src/a.java"),
        "命中成员仓的路径归该成员仓组：{section}"
    );
    assert!(
        section.contains("frontend: frontend/src/b.vue"),
        "未命中成员仓的路径按自身首段分组：{section}"
    );
    assert!(
        section.contains("(根文件): README.md"),
        "根级文件归根文件组：{section}"
    );
    assert!(
        !section.contains("未归属"),
        "不得产出误导性的未归属归并组：{section}"
    );
}

#[test]
fn sc_revision_prompt_carries_baseline_tree_section() {
    use crate::web::workspace_ws_types::{
        ReviewFinding, ReviewFindingSeverity, ReviewGate, ReviewVerdict, ReviewVerdictType,
    };
    let (request, issue, repository) = split_prompt_fixture();
    let design_requirement_ids = vec!["REQ-002".to_string()];
    let verdict = ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "契约缺口需返修".to_string(),
        summary: "返修摘要".to_string(),
        findings: vec![ReviewFinding {
            severity: ReviewFindingSeverity::MustFix,
            message: "WI-002 required_capabilities 缺 capability.field-constraints".to_string(),
            evidence: "work-item-plan.md WI-002 Inputs".to_string(),
            required_action: "在 CT-001 显式声明 capability.field-constraints".to_string(),
            category: None,
            class_hint: None,
            contract_field: None,
            machine_repair: None,
            owner_question: None,
            owner_options: None,
            disposition: None,
            target_work_item_id: None,
            finding_key: None,
        }],
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    };
    let revision =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_revision_prompt(
            &request,
            &issue,
            &repository,
            crate::product::work_item_split_engine::prompts::WorkItemPlanMarkdownAuthorContext {
                story_context: "story_spec_0001: level selection",
                design_context: "design_spec_0001: levels API",
                design_requirement_ids: &design_requirement_ids,
                repository_structure: "busi; frontend",
                language_rules: TEST_SINGLE_CANDIDATE_LANGUAGE_RULES,
                plan_options: &TEST_ALL_FALSE_PLAN_OPTIONS,
                routing_context: &RoutingReferenceContext::Legacy,
                member_targets: None,
                change_order: &[],
                baseline_tree: Some("busi: busi/src/main/java\nfrontend: frontend/src/App.vue"),
            },
            &verdict,
        )
        .expect("revision markdown author prompt with baseline tree");
    assert!(
        revision.contains("baseline:"),
        "返修轮必须携带同一份基线树清单（不因返修而丢失该事实）"
    );
    assert!(revision.contains("frontend/src/App.vue"));
}
