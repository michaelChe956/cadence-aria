// 3.6 弱模型基线加固：SC author prompt 弱模型精度教学契约测试（组 1）与
// 教学↔校验器逐字对齐漂移防线（组 3）。从 prompt_contract.rs 拆出以守住
// large_file_guard 1200 行红线：从 prompt_contract.rs 拆出；模块包装层的
// use super::* 提供共享 fixture（split_prompt_fixture 等）。

const WEAK_MODEL_TEST_LANGUAGE_RULES: &str =
    "## 语言规则\n\n- **必须使用中文** - 所有响应、解释、注释和文档必须使用中文。";

/// 3.6 弱模型基线加固（组 1）：SC author prompt 增补面向 flash 级弱模型的
/// 精度教学两段——AC 纪律（reviewer check 字段成对，含字段形态正例）与引用
/// 纪律（只逐字复制输入 spec 已定义 id，含 REQ-002 反例）。既有教学段逐字保留。
#[test]
fn work_item_plan_markdown_prompt_teaches_weak_model_precision_discipline() {
    let (request, issue, repository) = split_prompt_fixture();
    let design_requirement_ids = vec!["REQ-001".to_string()];
    let prompt =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_prompt(
            &request,
            &issue,
            &repository,
            crate::product::work_item_split_engine::prompts::WorkItemPlanMarkdownAuthorContext {
                story_context: "story_spec_0001: level selection",
                design_context: "design_spec_0001: levels API",
                design_requirement_ids: &design_requirement_ids,
                repository_structure: "src/product/levels; web/src/levels; tests/integration",
                language_rules: WEAK_MODEL_TEST_LANGUAGE_RULES,
                routing_context: &RoutingReferenceContext::Legacy,
            },
        )
        .expect("markdown author prompt");

    for required in [
        "[weak_model_precision]",
        "AC 纪律：每个 `- criterion_id: AC-xxx` 必须在 Handoff Schema 配对一行 `- reviewer_check_refs: AC-xxx`",
        "正例：`- criterion_id: AC-001` 配 `- reviewer_check_refs: AC-001`",
        "漏写 → acceptance_criterion_without_reviewer_check 拒绝",
        "引用纪律：requirement_refs/done_when_refs 只能逐字复制 spec 已定义 id（REQ-*/AC-*/NFR-*）",
        "task 的 requirement_refs 必须逐字取自 [design_requirements] 清单",
        "反例：引用清单没有的 REQ-002 → unknown_requirement_ref 拒绝",
    ] {
        assert!(
            prompt.contains(required),
            "弱模型精度教学必须包含 {required}: {prompt}"
        );
    }
    // 既有教学段不删不改：交叉引用纪律原句仍在，新段紧随其后、grammar 之前。
    for retained in [
        "[cross_reference_discipline]",
        "handoff 的 reviewer_check_refs 必须与全部且仅本 item 的 acceptance criterion ID 集合完全一致（每条 AC 恰好被检查一次）",
        "requirement_refs 仅引用清单；清单外 REQ-* 拒绝。",
        "每个被 tasks 的 requirement_refs 引用的 requirement_id，必须在本 item 的 Traceability section 有对应登记行（requirement_id 逐字相同）；登记值只能来自 [design_requirements] 清单",
    ] {
        assert!(
            prompt.contains(retained),
            "既有引用纪律段必须逐字保留 {retained}: {prompt}"
        );
    }
    let discipline_pos = prompt
        .find("[cross_reference_discipline]")
        .expect("cross reference discipline");
    let weak_pos = prompt
        .find("[weak_model_precision]")
        .expect("weak model precision discipline");
    let grammar_pos = prompt.find("[markdown_grammar]").expect("grammar block");
    assert!(
        discipline_pos < weak_pos && weak_pos < grammar_pos,
        "弱模型精度教学必须紧邻既有引用纪律段之后、grammar 之前"
    );
    assert!(
        prompt.len()
            < crate::product::work_item_split_engine::prompts::WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES,
        "增补教学后仍必须低于质量预算红线（第 9 次提额后 21,000），实测 {} bytes",
        prompt.len()
    );
}

/// 3.6 弱模型基线加固（组 3）：教学示例的字段名、错误码与消息语义必须与
/// `work_item_contract::validation.rs` 的 IR 校验判定逐字对齐（防教学与校验器
/// 漂移）：(1) 教学点名的字段/错误码/消息片段逐字存在于 validation.rs 判定源码；
/// (2) 教学的字段行形态与 grammar 白名单 key 一致；(3) 行为级对齐——按教学反例
/// 构造的最小 plan 必须被同一套校验器以教学点名的错误码拒绝。
#[test]
fn weak_model_precision_teaching_matches_contract_validator_judgement() {
    let validator_source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/product/work_item_contract/validation.rs"
    ));
    for aligned in [
        "reviewer_check_refs",
        "criterion_id",
        "requirement_refs",
        "acceptance_criterion_without_reviewer_check",
        "unknown_requirement_ref",
        "has no handoff reviewer check",
        "references unknown design requirement",
    ] {
        assert!(
            validator_source.contains(aligned),
            "validation.rs 判定源码必须包含教学对齐片段 {aligned}，否则教学已与校验器漂移"
        );
    }
    for structured_key in ["criterion_id", "reviewer_check_refs", "requirement_refs"] {
        assert!(
            crate::product::work_item_plan_compiler::grammar::STRUCTURED_KEYS
                .contains(&structured_key),
            "教学字段 {structured_key} 必须与 grammar 白名单 key 逐字一致"
        );
    }

    // 教学侧闭环：prompt 中的弱模型精度教学必须逐字携带上述错误码与字段形态，
    // 与 validation.rs 判定源码两侧同时断言才构成漂移防线。
    let (request, issue, repository) = split_prompt_fixture();
    let design_requirement_ids = vec!["REQ-001".to_string()];
    let prompt =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_prompt(
            &request,
            &issue,
            &repository,
            crate::product::work_item_split_engine::prompts::WorkItemPlanMarkdownAuthorContext {
                story_context: "story_spec_0001",
                design_context: "design_spec_0001",
                design_requirement_ids: &design_requirement_ids,
                repository_structure: "src/product",
                language_rules: WEAK_MODEL_TEST_LANGUAGE_RULES,
                routing_context: &RoutingReferenceContext::Legacy,
            },
        )
        .expect("markdown author prompt");
    for taught in [
        "acceptance_criterion_without_reviewer_check",
        "unknown_requirement_ref",
        "`- criterion_id: AC-xxx`",
        "`- reviewer_check_refs: AC-xxx`",
        "REQ-*/AC-*/NFR-*",
    ] {
        assert!(
            prompt.contains(taught),
            "弱模型教学必须逐字携带与校验器对齐的片段 {taught}: {prompt}"
        );
    }

    // 行为级对齐：教学正反例对应的 markdown 变体必须触发教学点名的错误码与消息。
    let minimum_source = prompt
        .split_once("[minimum_legal_source] 仅示语法形状；按当前上下文替换，勿照抄。\n")
        .and_then(|( _, rest)| rest.split_once("[real_finding_few_shot]"))
        .map(|(source, _)| source.to_string())
        .expect("prompt must inline a minimum legal source");
    // 反例变体 1：task 引用 Traceability 未登记的 REQ-002（教学反例本体）；
    // Traceability 仍登记 REQ-001，其余占位 id 一并落为 REQ-001。
    let unknown_requirement_source = minimum_source
        .replacen(
            "- requirement_refs: design_requirement_placeholder",
            "- requirement_refs: REQ-002",
            1,
        )
        .replace("design_requirement_placeholder", "REQ-001");
    // 反例变体 2：AC-001 失去配对的 reviewer_check_refs 行。
    let missing_reviewer_check_source = minimum_source
        .replace("design_requirement_placeholder", "REQ-001")
        .replacen("reviewer_check_refs: AC-001", "reviewer_check_refs: AC-002", 1);

    let compile = |source: String| {
        crate::product::work_item_plan_compiler::compile_work_item_plan(
            &source,
            &crate::product::work_item_plan_compiler::WorkItemPlanSourceContext {
                target_repository_id: "repo_0001".to_string(),
            },
        )
    };
    let validate = |ir: &crate::product::work_item_plan_compiler::PlanCandidateIr| {
        let spec_ids = vec!["story_spec_0001".to_string()];
        let design_ids = vec!["design_spec_0001".to_string()];
        let now = chrono::Utc::now().to_rfc3339();
        crate::product::work_item_plan_compiler::validate_plan_candidate_ir(
            ir,
            &crate::product::work_item_plan_compiler::PlanCandidateValidationContext {
                project_id: "project_0001",
                issue_id: "issue_0001",
                plan_id: "plan_0001",
                source_story_spec_ids: &spec_ids,
                source_design_spec_ids: &design_ids,
                repository_profile: None,
                now: &now,
            },
        )
    };

    let base_ir = compile(minimum_source.replace("design_requirement_placeholder", "REQ-001"))
        .expect("baseline minimum source must compile");
    assert!(
        validate(&base_ir).is_ok(),
        "基线（REQ-001 登记齐全、reviewer check 成对）必须通过 IR 校验，否则教学基线漂移"
    );

    let unknown_ir = compile(unknown_requirement_source)
        .expect("unknown requirement variant must still compile (error is IR-level)");
    let unknown_diagnostics = validate(&unknown_ir)
        .expect_err("引用不存在的 REQ-002 必须被 IR 校验拒绝");
    assert!(
        unknown_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "unknown_requirement_ref"
                && diagnostic.message.contains("references unknown design requirement REQ-002")),
        "教学反例必须与校验器实际判定逐字一致：{:?}",
        unknown_diagnostics
    );

    let reviewer_ir = compile(missing_reviewer_check_source)
        .expect("missing reviewer check variant must still compile");
    let reviewer_diagnostics = validate(&reviewer_ir)
        .expect_err("失去 reviewer check 的 AC 必须被 IR 校验拒绝");
    assert!(
        reviewer_diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "acceptance_criterion_without_reviewer_check"
                && diagnostic.message.contains("AC-001 has no handoff reviewer check")),
        "教学 AC 纪律必须与校验器实际判定逐字一致：{:?}",
        reviewer_diagnostics
    );
}

/// 3.6 矩阵 codex×重 三连败根因教学（issue_0166/0167/0168：上游 Outputs capabilities
/// 未逐字覆盖下游 Inputs require_all 消费 → required_capability_missing approval
/// 编译拒）：SC author prompt 增补 [weak_model_precision] 第三条「输出契约纪律」。
/// 既有两条教学逐字保留；本测试逐句钉住新教学，并重申段序与预算红线。
#[test]
fn work_item_plan_markdown_prompt_teaches_output_contract_capability_verbatim_coverage() {
    let (request, issue, repository) = split_prompt_fixture();
    let design_requirement_ids = vec!["REQ-001".to_string()];
    let prompt =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_prompt(
            &request,
            &issue,
            &repository,
            crate::product::work_item_split_engine::prompts::WorkItemPlanMarkdownAuthorContext {
                story_context: "story_spec_0001: level selection",
                design_context: "design_spec_0001: levels API",
                design_requirement_ids: &design_requirement_ids,
                repository_structure: "src/product/levels; web/src/levels; tests/integration",
                language_rules: WEAK_MODEL_TEST_LANGUAGE_RULES,
                routing_context: &RoutingReferenceContext::Legacy,
            },
        )
        .expect("markdown author prompt");

    for required in [
        "输出契约纪律：SC 计划由你在同一文档先后写出",
        "下游 Inputs required_capabilities 写什么，被 (provider_logical_work_item_id, contract_id) 指向的上游 Outputs capabilities 就逐字复制什么，require_all 逐项覆盖",
        "同一契约全计划只声明一次：多个下游消费同一契约时把全部能力串合并进这唯一一份 capabilities，禁止按下游重复声明契约",
        "两个下游消费同一 CT-001 → 一份 CT-001 含两者能力串",
        "正例：下游 `- required_capabilities: [数据错误返回 500 且 code=LEVEL_DATA_UNAVAILABLE]` → 上游 `capabilities:` 逐字同串",
        "反例①改写/概括/漏一项 → required_capability_missing 拒绝",
        "反例②按下游重复声明契约 → duplicate_contract_id 拒绝",
    ] {
        assert!(
            prompt.contains(required),
            "输出契约纪律教学必须包含 {required}: {prompt}"
        );
    }
    // 段序不破：新教学条仍在 [weak_model_precision] 段内，位于交叉引用纪律之后、grammar 之前。
    let discipline_pos = prompt
        .find("[cross_reference_discipline]")
        .expect("cross reference discipline");
    let weak_pos = prompt
        .find("[weak_model_precision]")
        .expect("weak model precision discipline");
    let grammar_pos = prompt.find("[markdown_grammar]").expect("grammar block");
    assert!(
        discipline_pos < weak_pos && weak_pos < grammar_pos,
        "输出契约教学追加后段序必须保持：交叉引用纪律 → 弱模型精度 → grammar"
    );
    assert!(
        prompt.len()
            < crate::product::work_item_split_engine::prompts::WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES,
        "第三条教学追加后仍必须低于质量预算红线，实测 {} bytes",
        prompt.len()
    );
    eprintln!(
        "codex×重 SC author prompt bytes={} margin={}",
        prompt.len(),
        crate::product::work_item_split_engine::prompts::WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES
            - prompt.len()
    );
}

/// 教学↔dependency 校验器逐字对齐（codex×重）：输出契约纪律点名的错误码、消息片段
/// 与匹配机制必须逐字存在于 `work_item_contract::dependency.rs` 判定源码；教学字段必须
/// 与 grammar 白名单 key 一致。行为级复用 work_item_contract/tests/dependency.rs 的
/// canonical 契约 fixture 场景：按教学反例（改写/概括/漏一项）构造的最小 plan 必须被
/// canonical 契约校验以 required_capability_missing 拒绝，逐字回填则通过。
#[test]
fn output_contract_capability_teaching_matches_dependency_validator_judgement() {
    let dependency_source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/product/work_item_contract/dependency.rs"
    ));
    for aligned in [
        "required_capability_missing",
        "lacks capability",
        "missing_capabilities_for_policy",
        "provider_logical_work_item_id",
        "required_capabilities",
    ] {
        assert!(
            dependency_source.contains(aligned),
            "dependency.rs 判定源码必须包含教学对齐片段 {aligned}，否则教学已与校验器漂移"
        );
    }
    let validation_source = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/src/product/work_item_contract/validation.rs"
    ));
    assert!(
        validation_source.contains("duplicate_contract_id"),
        "validation.rs 判定源码必须包含教学对齐片段 duplicate_contract_id，否则教学已与校验器漂移"
    );
    for structured_key in [
        "capabilities",
        "required_capabilities",
        "provider_logical_work_item_id",
        "contract_id",
    ] {
        assert!(
            crate::product::work_item_plan_compiler::grammar::STRUCTURED_KEYS
                .contains(&structured_key),
            "教学字段 {structured_key} 必须与 grammar 白名单 key 逐字一致"
        );
    }
    assert!(
        crate::product::work_item_plan_compiler::grammar::ALLOWED_COMPATIBILITY_POLICIES
            .contains(&"require_all"),
        "教学点名的 require_all 必须是 grammar 允许的 compatibility_policy 值"
    );

    // 教学侧闭环：prompt 中的输出契约教学必须逐字携带与 dependency.rs 对齐的片段，
    // 与判定源码两侧同时断言才构成漂移防线。
    let (request, issue, repository) = split_prompt_fixture();
    let design_requirement_ids = vec!["REQ-001".to_string()];
    let prompt =
        crate::product::work_item_split_engine::prompts::build_work_item_plan_markdown_prompt(
            &request,
            &issue,
            &repository,
            crate::product::work_item_split_engine::prompts::WorkItemPlanMarkdownAuthorContext {
                story_context: "story_spec_0001: level selection",
                design_context: "design_spec_0001: levels API",
                design_requirement_ids: &design_requirement_ids,
                repository_structure: "src/product/levels; web/src/levels; tests/integration",
                language_rules: WEAK_MODEL_TEST_LANGUAGE_RULES,
                routing_context: &RoutingReferenceContext::Legacy,
            },
        )
        .expect("markdown author prompt");
    for taught in [
        "required_capability_missing",
        "(provider_logical_work_item_id, contract_id)",
        "require_all",
        "数据错误返回 500 且 code=LEVEL_DATA_UNAVAILABLE",
    ] {
        assert!(
            prompt.contains(taught),
            "输出契约教学必须逐字携带与校验器对齐的片段 {taught}: {prompt}"
        );
    }

    // 行为级对齐：教学正反例对应的最小 canonical plan 必须触发教学点名的错误码与消息。
    let capability = "数据错误返回 500 且 code=LEVEL_DATA_UNAVAILABLE";
    let second_capability = "GET /api/levels 返回 200 与五条关卡记录";
    let provider_with = |capabilities: &[&str]| {
        let mut provider =
            crate::product::work_item_contract::canonical_contract_fixture("WI-001");
        provider.input_contracts.clear();
        provider.output_contracts =
            vec![crate::product::work_item_contract::PromisedOutputContract {
                contract_id: "CT-LEVELS-API-V1".to_string(),
                capabilities: capabilities.iter().map(|c| (*c).to_string()).collect(),
            }];
        provider.handoff_contract.provided_contract_refs =
            vec!["CT-LEVELS-API-V1".to_string()];
        provider
    };
    let consumer_with = |required: &[&str]| {
        let mut consumer =
            crate::product::work_item_contract::canonical_contract_fixture("WI-002");
        consumer.input_contracts =
            vec![crate::product::work_item_contract::RequiredInputContract {
                contract_id: "CT-LEVELS-API-V1".to_string(),
                provider_logical_work_item_id: "WI-001".to_string(),
                required_capabilities: required.iter().map(|c| (*c).to_string()).collect(),
                compatibility_policy:
                    crate::product::work_item_contract::ContractCompatibilityPolicy::RequireAll,
            }];
        consumer.output_contracts.clear();
        consumer.handoff_contract.provided_contract_refs.clear();
        consumer
    };
    let validate = |provider, consumer| {
        let graph = crate::product::work_item_contract::build_dependency_contract_graph(&[
            provider, consumer,
        ])
        .expect("dependency graph");
        crate::product::work_item_contract::validate_dependency_contract_graph(&graph)
    };

    // 反例 1（改写/概括）：上游 capabilities 语义概括下游 required 能力串。
    let paraphrase_report = validate(
        provider_with(&["数据错误统一返回结构化错误体"]),
        consumer_with(&[capability]),
    );
    assert!(
        paraphrase_report.findings.iter().any(|finding| {
            finding.code == "required_capability_missing"
                && finding.logical_work_item_id.as_deref() == Some("WI-002")
                && finding.contract_ref.as_deref() == Some("CT-LEVELS-API-V1")
                && finding.capability_ref.as_deref() == Some(capability)
                && finding
                    .message
                    .contains(&format!("lacks capability {capability} required by WI-002"))
        }),
        "教学反例（改写/概括）必须与 dependency 校验器实际判定逐字一致：{:?}",
        paraphrase_report.findings
    );

    // 反例 2（漏一项）：require_all 下缺第二项能力同样逐项拒绝。
    let partial_report = validate(
        provider_with(&[capability]),
        consumer_with(&[capability, second_capability]),
    );
    assert!(
        partial_report.findings.iter().any(|finding| {
            finding.code == "required_capability_missing"
                && finding.capability_ref.as_deref() == Some(second_capability)
        }),
        "require_all 必须每项逐一覆盖（漏一项拒绝）：{:?}",
        partial_report.findings
    );

    // 正例：上游逐字回填全部 required 能力串 → 无 required_capability_missing finding。
    let verbatim_report = validate(
        provider_with(&[capability, second_capability]),
        consumer_with(&[capability, second_capability]),
    );
    assert!(
        verbatim_report
            .findings
            .iter()
            .all(|finding| finding.code != "required_capability_missing"),
        "逐字回填后不得再出现 required_capability_missing：{:?}",
        verbatim_report.findings
    );
}
