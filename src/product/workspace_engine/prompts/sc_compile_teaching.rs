//! SC compile 教学重驱 prompt 构造面，自 prompts.rs 拆出以守 1200 行文件守护
//! （先例：同仓另一模块的 `work_item_split_engine/prompts/sc_author_teaching.rs`、
//! `work_item_split_engine/prompts/sc_revision.rs`）。
//!
//! 对外路径不变——`prompts` 原样 re-export 本模块入口，调用点
//! `crate::product::workspace_engine::build_work_item_plan_compile_reredrive_prompt`
//! 仍可解析。Task 4（MODIFIED REQ-WSC-09）：入口改收 typed 事实四元组
//! （parse 码表回灌 + lowering/IR typed 载荷 + 有界指令行）。因 `prompts.rs`
//! 本体禁写（本 change 不写该文件）而模块私有，
//! [`TeachingReredriveFacts`] 无法在模块外点名 ⇒ 入口边界以四个切片参数
//! 平面展开（字段一一对应，不经过 message 文本），struct 留作模块内的
//! typed 事实载体与单测面。

// 本模块的函数体只用到 `crate::` 全路径，无需父模块的名字；此处保留
// `use super::*;` 仅为让迁入文档注释里的 intra-doc 链接
// [`build_artifact_retry_prompt`] 在本模块内仍可解析（rustc 的
// unused_imports 不把文档链接计为使用，故显式豁免）。
#[allow(unused_imports)]
use super::*;

use crate::product::work_item_plan_compiler::{
    IrFault, IrFaultKind, IrFaultLocator, LoweringFault,
};
use crate::product::workspace_engine::{ScTeachingPromptError, ScTeachingSourceMetadata};

/// A1（REQ-WSC-32）：parse 诊断事实块的有界投影**字节**预算（UTF-8）——
/// 只约束新增的 parse 事实块（多行：首行三段逐字 + field/example 行），
/// 超限整条省略不切半；旧 lowering/IR/registered IDs 与单行呈现原文不在
/// 此列。逐条**条数**另有与契约修复指令单点同源的
/// `MAX_AUTOREPAIR_INSTRUCTION_LINES` 上限（见渲染处），本预算只兜上限内
/// 条目的体积。
const MAX_PARSE_FACT_BLOCK_BYTES: usize = 32_000;

/// Task 4（MODIFIED REQ-WSC-09）：教学重驱 prompt 的 typed 事实载体。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TeachingReredriveFacts<'a> {
    /// parse 语法类（码表 4 项）code:line:message 回灌。
    pub parse_reasons: &'a [String],
    /// lowering typed fault（构造点显式写入、事实齐全）。
    pub lowering_faults: &'a [LoweringFault],
    /// IR typed 载荷（行在场 + 合法集合非空）。
    pub ir_faults: &'a [IrFault],
    /// Task 3 收窄指令的呈现文本（条数已在调用侧/本入口双重有界）。
    pub autorepair_instructions: &'a [String],
    /// 上一轮失败源码全文（P23：逐字入 prompt，不 trim / 不转义 / 不截断）。
    pub previous_source: &'a str,
    /// 上一轮失败源码的来源元数据（ref/hash/bytes/origin），由 P21 renderer
    /// 与正文复算比对；只读借用。
    pub source: ScTeachingSourceMetadata<'a>,
}

/// 四类 IR 诊断身份的稳定码名（与 producer 写入的 diagnostic code 同名；
/// 枚举变体 → 码名的显式映射，非从 message 反解析）。
fn ir_fault_kind_code(kind: &IrFaultKind) -> &'static str {
    match kind {
        IrFaultKind::UnknownDoneWhenRef => "unknown_done_when_ref",
        IrFaultKind::UnknownReviewerCheckRef => "unknown_reviewer_check_ref",
        IrFaultKind::AcceptanceCriterionWithoutReviewerCheck => {
            "acceptance_criterion_without_reviewer_check"
        }
        IrFaultKind::UnknownRequirementRef => "unknown_requirement_ref",
    }
}

/// F2-B（Task 4 升级为 typed 载荷渲染）：SC compile 失败教学重驱 prompt
/// （每 candidate 恰一次）。复用 [`build_artifact_retry_prompt`] 的模板形态
/// （「上一轮已结束，但没有输出完整 artifact…不要继续调研，不要只解释…立即
/// 输出完整…」）+ parse 码表错误原文逐条回灌 + lowering/IR typed 载荷
/// （field/行号/期望形态/合法集合）+ 有界契约指令行 + work-item-plan
/// markdown source 的硬性重驱指令（第一行即文档标题）。
///
/// r46 核心修(三角色收敛):并入登记 ID 清单+短规则——compile 教学重驱
/// 此前只有错误原文+B1 白名单,ID 纪律裸奔(现场:r46 首轮漏填
/// requirement_id → 重驱改编号/新增清单外编号,震荡不收敛)。清单与
/// IR 重驱/harness 反馈轮同集(story+design 双源 REQ-/NFR-/AC-)。
///
/// P23（Plan §2.4）：入参加上一轮失败源码全文与来源元数据——本轮 prompt
/// **自带 source**（不依赖「刚才读过的文件」这类外部上下文），source 块由
/// P21 的 `render_sc_previous_source_block` 逐字渲染，位置在失败事实之后、
/// 最终完整输出指令之前。来源完整性不符（sha256/bytes 与正文复算不一致）
/// 时返回 [`ScTeachingPromptError`]，不返回任何半成品文本。
pub(crate) fn build_work_item_plan_compile_reredrive_prompt(
    parse_reasons: &[String],
    lowering_faults: &[LoweringFault],
    ir_faults: &[IrFault],
    autorepair_instructions: &[String],
    registered_requirement_ids: &[String],
    previous_source: &str,
    source: &ScTeachingSourceMetadata<'_>,
) -> Result<String, ScTeachingPromptError> {
    render_teaching_reredrive_prompt(
        &TeachingReredriveFacts {
            parse_reasons,
            lowering_faults,
            ir_faults,
            autorepair_instructions,
            previous_source,
            source: source.clone(),
        },
        registered_requirement_ids,
    )
}

/// 入口的渲染本体（typed 事实载体形态；模块内单测直接钉这个面）。
fn render_teaching_reredrive_prompt(
    facts: &TeachingReredriveFacts<'_>,
    registered_requirement_ids: &[String],
) -> Result<String, ScTeachingPromptError> {
    let mut prompt = String::from(
        "上一轮已结束，但没有输出完整的 work-item-plan markdown source。\n\
         不要继续调研，不要只解释，不要输出任何前言或路由回执。\n",
    );
    if !facts.parse_reasons.is_empty() {
        prompt.push_str("\n具体失败原因:\n");
        // A1（REQ-WSC-32）：仅新增的 parse 事实块（多行：首行逐字三段 +
        // field/example 行）受有界投影约束——整条省略、顺序稳定、显式记账；
        // 单行呈现原文（IR 预检 failure_reasons 等旧材料）不截断。双重界：
        // ① 逐条条数与契约修复指令单点同源
        // MAX_AUTOREPAIR_INSTRUCTION_LINES——owner 可见 prompt 不得另写一套
        // 阈值（Important-1 同族缺陷：指令面已按上限截断、计数收口的超限
        // 条目，不得经事实块的 field/行号行再无界透传）；② 字节预算
        // MAX_PARSE_FACT_BLOCK_BYTES 兜住上限内条目的体积。
        let max_fact_entries =
            crate::product::workspace_engine::contract_autorepair::MAX_AUTOREPAIR_INSTRUCTION_LINES;
        let mut fact_block_bytes = 0usize;
        let mut total = 0usize;
        let mut shown = 0usize;
        let mut omitted = 0usize;
        for reason in facts.parse_reasons {
            if !reason.contains('\n') {
                prompt.push_str("- ");
                prompt.push_str(reason);
                prompt.push('\n');
                continue;
            }
            total += 1;
            if shown >= max_fact_entries {
                omitted += 1;
                continue;
            }
            // "- " 前缀 + 块本体 + 换行 = 该块在 prompt 中的 UTF-8 字节数。
            let rendered_bytes = reason.len() + "- ".len() + 1;
            if fact_block_bytes + rendered_bytes > MAX_PARSE_FACT_BLOCK_BYTES {
                omitted += 1;
                continue;
            }
            fact_block_bytes += rendered_bytes;
            shown += 1;
            prompt.push_str("- ");
            prompt.push_str(reason);
            prompt.push('\n');
        }
        if omitted > 0 {
            prompt.push_str(&format!(
                "- parse 事实块有界投影收口:共 {total} 条 / 呈现 {shown} 条 / 省略 {omitted} 条(整条省略,不切半条诊断;逐条上限 {max_fact_entries} 条与契约修复指令同源,字节预算 {MAX_PARSE_FACT_BLOCK_BYTES} B)。\n"
            ));
        }
        // r41 B1(诊断增强):unknown_structured_key 类失败附「合法 section/key
        // 白名单 + 具体动作」——错误码单行不足以让 AI 自纠(它不知道合法集)。
        // A1（REQ-WSC-32）:该分支再附 40 个合法 STRUCTURED_KEYS(仅此分支,
        // 不重发完整结构契约大包)。
        if facts
            .parse_reasons
            .iter()
            .any(|reason| reason.contains("unknown_structured_key"))
        {
            let sections =
                crate::product::work_item_plan_compiler::grammar::STRUCTURED_SECTIONS.join("、");
            let free =
                crate::product::work_item_plan_compiler::grammar::FREE_TEXT_SECTIONS.join("、");
            let keys = crate::product::work_item_plan_compiler::grammar::STRUCTURED_KEYS.join("、");
            prompt.push_str(&format!(
                "\n结构化 section/key 白名单(结构化段只允许这些 key):\n- 结构化: {sections}\n- 自由文本: {free}\n- 合法 key: {keys}\n\
                 动作:删除违例 section,或将其内容并入语义最近的合法 section 后重新输出完整 plan。\n"
            ));
        }
    }
    // Task 4：lowering typed 载荷——field/行号/期望形态/原值（不回灌裸
    // message 堆砌，事实即教学面）。
    if !facts.lowering_faults.is_empty() {
        prompt.push_str("\nlowering typed 事实(逐条按事实修复):\n");
        for fault in facts.lowering_faults {
            match fault {
                LoweringFault::BranchKeyMismatch {
                    kind,
                    expected_field,
                    actual_field,
                    value,
                    line,
                    expected_shape,
                } => {
                    prompt.push_str(&format!(
                        "- 字段 `{expected_field}`(第 {line} 行): `{kind}` 分支的值 `{value}` 现填在 `{actual_field}`;应为 `{expected_shape}` 形态。\n"
                    ));
                }
                LoweringFault::RequiredFieldAbsent {
                    field,
                    expected_shape,
                } => {
                    prompt.push_str(&format!(
                        "- 必填字段 `{field}` 在场但为空;应为 `{expected_shape}` 形态。\n"
                    ));
                }
            }
        }
    }
    // Task 4：IR typed 载荷——引用/合法集合/期望形态（合法集合即唯一可选值）。
    if !facts.ir_faults.is_empty() {
        prompt.push_str("\nIR 引用 typed 事实(逐条按事实修复,合法集合即唯一可选值):\n");
        for fault in facts.ir_faults {
            let locator = match &fault.locator {
                IrFaultLocator::Task { task_id } => format!("task {task_id}"),
                IrFaultLocator::Handoff => "handoff".to_string(),
                IrFaultLocator::Criterion { criterion_id } => format!("criterion {criterion_id}"),
            };
            prompt.push_str(&format!(
                "- 第 {} 行 {}(work item {})触发 `{}`: 引用 `{}` 不在合法集合 [{}] 内;应为 `{}` 形态。\n",
                fault
                    .source_line
                    .map(|line| line.to_string())
                    .unwrap_or_else(|| "?".to_string()),
                locator,
                fault.work_item_id,
                ir_fault_kind_code(&fault.kind),
                fault.offending_ref,
                fault.legal_refs.join("、"),
                fault.expected_shape,
            ));
        }
    }
    // Task 3 评审 Minor-3 × Task 4 评审 Important-1：指令行条数上限——截断
    // 与真总数摘要**单点复用** `contract_autorepair::cap_autorepair_instruction_lines`
    //（入参保持未截断，摘要写真总数；不在此另写一套 take/文案）。
    if !facts.autorepair_instructions.is_empty() {
        prompt.push_str(
            "\n契约缺口修复指令(O1:平台不改写 source,交你在保护闭包内产出新 revision):\n",
        );
        for instruction in
            crate::product::workspace_engine::contract_autorepair::cap_autorepair_instruction_lines(
                facts.autorepair_instructions.to_vec(),
            )
        {
            prompt.push_str("- ");
            prompt.push_str(&instruction);
            prompt.push('\n');
        }
    }
    // r46 核心修:登记 ID 清单 + 短规则(与 IR 重驱同集;措辞与校验器字面量
    // 对齐:三元组缺 requirement_id → lowering_error「traceability 缺少
    // requirement_id」拒绝)。
    if !registered_requirement_ids.is_empty() {
        prompt.push_str(&format!(
            "\n[design_requirements] {}\n\
             每条 Traceability 三元组 requirement_id 必填且逐字取自上列清单;\
             修复引用时不重编号、禁止新增清单外 REQ-*/NFR-*/AC-*/TASK;\
             done_when 只指向已定义编号。\n",
            registered_requirement_ids.join("、")
        ));
    }
    // P23（Plan §2.4）：上一轮失败源码逐字入 prompt（只有 P21 renderer 一个
    // 来源——不自拼围栏、不 trim / 不转义 / 截断）。位置：全部失败事实之后、
    // 最终完整输出指令之前。
    let source_block = crate::product::workspace_engine::render_sc_previous_source_block(
        facts.previous_source,
        &facts.source,
    )?;
    prompt.push('\n');
    prompt.push_str(&source_block);
    prompt.push('\n');
    // 最终输出指令在 source 块之后：source 即修复起点，本轮 prompt 自足。
    prompt.push('\n');
    prompt.push_str(
        "以围栏内的上一轮失败源码为唯一修复起点（本次 prompt 自足，\
         不要重新调研、不要读取其它文件），立即输出完整 work-item-plan markdown source；\
         第一行即文档标题 `# Work Item Plan`。\
         只改动失败事实指出的位置，其余内容逐字保留；不要回显、复述或重复上面围栏内的源码，\
         也不要输出围栏或任何多余 heading（实测：回显源码会导致 heading/section 结构错误）。\n",
    );
    Ok(prompt)
}

#[cfg(test)]
mod reredrive_prompt_tests {
    use sha2::Digest as _;

    use super::{
        TeachingReredriveFacts, build_work_item_plan_compile_reredrive_prompt,
        render_teaching_reredrive_prompt,
    };
    use crate::product::work_item_plan_compiler::{
        IrFault, IrFaultKind, IrFaultLocator, LoweringFault,
    };
    use crate::product::workspace_engine::{ScTeachingPromptError, ScTeachingSourceMetadata};
    use crate::web::workspace_ws_types::ScTeachingSourceOrigin;

    /// P23 来源 fixture 的 locator（教学来源 locator 形态）。
    const FIXTURE_SOURCE_REF: &str = "sc-teaching-source/v1/session-1/chain-1/0";

    /// 真实字节口径的上一轮源码：自带反引号围栏行 + CJK 多字节正文
    /// （chars ≠ bytes，字节口径断言才有区分力）。
    const FIXTURE_SOURCE: &str =
        "# Work Item Plan\n\n```yaml\nid: WI-PLAN\n```\n中文正文：计划。\n";

    /// 真实字节口径的来源 fixture：sha256/bytes 由正文自身复算（与 P21
    /// renderer 同一口径），不手写魔法值。
    struct SourceFixture {
        sha256: String,
    }

    impl SourceFixture {
        fn of(source: &str) -> Self {
            Self {
                sha256: hex::encode(sha2::Sha256::digest(source.as_bytes())),
            }
        }

        fn metadata<'a>(&'a self, source: &'a str) -> ScTeachingSourceMetadata<'a> {
            ScTeachingSourceMetadata {
                source_ref: FIXTURE_SOURCE_REF,
                sha256: &self.sha256,
                bytes: source.len() as u64,
                origin: ScTeachingSourceOrigin::CompileSplitOutput,
            }
        }
    }

    /// 新签名（7 参 → `Result`）的完整调用面，来源元数据按真实字节口径构造。
    fn build(
        parse_reasons: &[String],
        lowering_faults: &[LoweringFault],
        ir_faults: &[IrFault],
        instructions: &[String],
        registered: &[String],
    ) -> Result<String, ScTeachingPromptError> {
        let fixture = SourceFixture::of(FIXTURE_SOURCE);
        build_work_item_plan_compile_reredrive_prompt(
            parse_reasons,
            lowering_faults,
            ir_faults,
            instructions,
            registered,
            FIXTURE_SOURCE,
            &fixture.metadata(FIXTURE_SOURCE),
        )
    }

    /// 旧形态（仅 parse 码表错误原文）的便捷调用。
    fn plain_prompt(parse_reasons: &[String], registered: &[String]) -> String {
        build(parse_reasons, &[], &[], &[], registered).expect("合法来源元数据必须构造成功")
    }

    /// 按围栏结构在 prompt 内定位上一轮源码正文并断言逐字完整——正则与断言都
    /// 直接作用于 prompt 本身，不经 renderer 重算（避免自证）；返回正文起点
    /// 供顺序断言使用。
    fn assert_lossless_source_block(prompt: &str) -> usize {
        let start = prompt.find(FIXTURE_SOURCE).expect("source 正文必须在场");
        let open_fence = prompt[..start]
            .strip_suffix('\n')
            .and_then(|head| head.rsplit('\n').next())
            .expect("正文前必须是一条围栏行");
        assert!(
            open_fence.len() >= 3 && open_fence.bytes().all(|byte| byte == b'`'),
            "正文前必须是反引号围栏行: {open_fence:?}"
        );
        let after = prompt[start + FIXTURE_SOURCE.len()..]
            .strip_prefix('\n')
            .expect("正文后必须换行");
        assert!(
            after.starts_with(open_fence),
            "闭合围栏必须与开启围栏等长: {after:?}"
        );
        let tail = after[open_fence.len()..]
            .strip_prefix('\n')
            .expect("闭合围栏后必须换行");
        assert!(
            !tail.trim().is_empty(),
            "source 块之后必须仍有内容（source 不是文末）"
        );
        start
    }

    /// r41 B1:unknown_structured_key 类失败的重驱 prompt 必须附 14 合法
    /// section/key 白名单与具体动作(AI 不知道合法集就无法自纠)。
    #[test]
    fn compile_reredrive_prompt_lists_section_whitelist_for_unknown_key() {
        let reasons = vec!["unknown_structured_key:1:未知结构化 key `NotAPermission`".to_string()];
        let prompt = plain_prompt(&reasons, &[]);
        assert!(prompt.contains("unknown_structured_key"));
        assert!(
            prompt.contains("结构化: Identity"),
            "白名单必须列出合法 section 集: {prompt}"
        );
        assert!(prompt.contains("Traceability"));
    }

    /// 对照:非 unknown_structured_key 失败不附白名单(保持既有形态)。
    #[test]
    fn compile_reredrive_prompt_keeps_plain_form_without_unknown_key() {
        let reasons = vec!["missing_section:0:缺 Identity".to_string()];
        let prompt = plain_prompt(&reasons, &[]);
        let keys = crate::product::work_item_plan_compiler::grammar::STRUCTURED_KEYS.join("、");
        assert!(
            !prompt.contains(&keys),
            "非 unknown_structured_key 失败不得附带合法 key 集: {prompt}"
        );
    }

    /// r46 核心修 TDD(精确复现 r46 首轮漏填形态):compile 教学重驱 prompt
    /// 必须含真实登记清单+必填规则——此前只有错误原文+B1 白名单,ID 纪律
    /// 裸奔,重驱改编号/新增清单外编号震荡不收敛。
    #[test]
    fn compile_reredrive_prompt_carries_registered_ids_and_discipline() {
        let reasons = vec!["lowering_error:42:traceability 缺少 requirement_id。".to_string()];
        let registered = vec![
            "REQ-ENV-01".to_string(),
            "NFR-PERF-02".to_string(),
            "AC-001".to_string(),
        ];
        let prompt = plain_prompt(&reasons, &registered);
        assert_eq!(
            prompt.matches("[design_requirements] ").count(),
            1,
            "登记清单段必须恰出现一次: {prompt}"
        );
        assert!(
            prompt.contains("[design_requirements] REQ-ENV-01、NFR-PERF-02、AC-001"),
            "真实登记清单(双源同集 REQ/NFR/AC)必须逐字并入: {prompt}"
        );
    }

    /// r46:清单空时不注入(无 spec 上下文的老会话零变化)。
    #[test]
    fn compile_reredrive_prompt_omits_empty_registered_ids() {
        let reasons = vec!["missing_section:0:缺 Identity".to_string()];
        let prompt = plain_prompt(&reasons, &[]);
        assert!(!prompt.contains("[design_requirements]"));
    }

    /// Task 4（MODIFIED REQ-WSC-09）：typed 载荷渲染——field 名、行号、
    /// 期望形态、合法集合与指令行进 prompt；不依赖裸 message 文本。
    #[test]
    fn compile_reredrive_prompt_renders_typed_payloads() {
        let parse_reasons = vec!["invalid_ears:9:statement 必须符合 EARS 模板。".to_string()];
        let lowering_faults = vec![LoweringFault::BranchKeyMismatch {
            kind: "single_repository",
            expected_field: "intent_target_repo",
            actual_field: "intent_target_logical_repo",
            value: "legacy-alias-value".to_string(),
            line: 12,
            expected_shape: "- intent_target_repo: value",
        }];
        let ir_faults = vec![IrFault {
            kind: IrFaultKind::UnknownRequirementRef,
            work_item_id: "WI-001".to_string(),
            locator: IrFaultLocator::Task {
                task_id: "TASK-001".to_string(),
            },
            offending_ref: "REQ-002".to_string(),
            legal_refs: vec!["REQ-001".to_string()],
            source_line: Some(30),
            expected_shape: "- requirement_refs: REQ-001".to_string(),
        }];
        let instructions = vec!["rule-x: 迁往合法字段".to_string()];
        let fixture = SourceFixture::of(FIXTURE_SOURCE);
        let prompt = render_teaching_reredrive_prompt(
            &TeachingReredriveFacts {
                parse_reasons: &parse_reasons,
                lowering_faults: &lowering_faults,
                ir_faults: &ir_faults,
                autorepair_instructions: &instructions,
                previous_source: FIXTURE_SOURCE,
                source: fixture.metadata(FIXTURE_SOURCE),
            },
            &[],
        )
        .expect("合法来源元数据必须构造成功");
        for required in [
            "intent_target_repo",
            "第 12 行",
            "- intent_target_repo: value",
            "legacy-alias-value",
            "第 30 行",
            "合法集合 [REQ-001]",
            "unknown_requirement_ref",
            "迁往合法字段",
        ] {
            assert!(
                prompt.contains(required),
                "typed 载荷渲染必须包含 {required}: {prompt}"
            );
        }
    }

    /// Task 3 评审 Minor-3 × Task 4 评审 Important-1/Minor-1：指令条数上限
    /// ——截断与「共 N 条契约修复指令」摘要单点复用
    /// `cap_autorepair_instruction_lines`（入参未截断，摘要写真总数）。
    #[test]
    fn compile_reredrive_prompt_bounds_instruction_lines() {
        let instructions: Vec<String> = (0..10)
            .map(|index| format!("rule-{index}: 修复动作"))
            .collect();
        let prompt = build(&[], &[], &[], &instructions, &[]).expect("合法来源元数据必须构造成功");
        assert!(
            prompt.contains("rule-3: 修复动作"),
            "上限内的指令必须在场: {prompt}"
        );
        assert!(
            !prompt.contains("rule-4: 修复动作"),
            "超上限的指令不得逐条透传: {prompt}"
        );
        assert!(
            prompt.contains("共 10 条"),
            "截断必须以真总数计数摘要显式收口: {prompt}"
        );
        assert!(
            !prompt.contains("共 5 条"),
            "摘要不得报告截断后长度: {prompt}"
        );
    }
    /// A1（REQ-WSC-32）：parse 事实块（首行逐字三段 + field/example 行）逐条
    /// 渲染——同 message 不同 field 的条目在最终 prompt 逐条可区分；预算内
    /// 不出现省略记账。
    #[test]
    fn parse_fact_blocks_carry_field_and_example_per_entry() {
        let parse_reasons = vec![
            "missing_section:5:结构化 section 缺少必需字段。\n  缺失字段: Goal\n  正确形状: - Goal: value".to_string(),
            "missing_section:9:结构化 section 缺少必需字段。\n  缺失字段: role\n  正确形状: - role: value".to_string(),
        ];
        let prompt = plain_prompt(&parse_reasons, &[]);
        for (line, field) in [(5usize, "Goal"), (9, "role")] {
            assert!(
                prompt.contains(&format!(
                    "- missing_section:{line}:结构化 section 缺少必需字段。\n  缺失字段: {field}\n  正确形状: - {field}: value\n"
                )),
                "parse 事实块必须逐条带齐三段+精确 field+正确形状: {prompt}"
            );
        }
        assert!(
            !prompt.contains("共 2 条"),
            "预算内不得出现省略记账: {prompt}"
        );
    }

    /// A1：事实块超**条数上限**（与契约修复指令单点同源
    /// `MAX_AUTOREPAIR_INSTRUCTION_LINES`）⇒ 超限条目整条省略 + 显式记账
    ///（总数/呈现数/省略数），顺序稳定，不切半条（被省略条目的任何片段
    /// 都不得出现）。Important-1 同族护栏：指令面已按上限收口的超限条目，
    /// 不得经事实块的 field/首行再透传进同一 owner 可见 prompt。
    #[test]
    fn parse_fact_block_over_entry_cap_omits_whole_entries_and_reports_counts() {
        use crate::product::workspace_engine::contract_autorepair::MAX_AUTOREPAIR_INSTRUCTION_LINES;
        // 6 条小事实块（字节预算不介入）：条数上限 ⇒ 呈现前 4、省略后 2。
        let parse_reasons: Vec<String> = (0..6)
            .map(|index| {
                format!(
                    "missing_section:{index:03}:m\n  缺失字段: f{index:03}\n  正确形状: - f{index:03}: value"
                )
            })
            .collect();
        let prompt = plain_prompt(&parse_reasons, &[]);
        assert!(
            prompt.contains(&format!(
                "共 6 条 / 呈现 {MAX_AUTOREPAIR_INSTRUCTION_LINES} 条 / 省略 {} 条",
                6 - MAX_AUTOREPAIR_INSTRUCTION_LINES
            )),
            "超条数上限必须显式记账总数/呈现数/省略数: {prompt}"
        );
        assert!(
            prompt.contains("缺失字段: f003") && prompt.contains("missing_section:003:"),
            "上限内末条完整在场: {prompt}"
        );
        // 整条省略：第 5 条（0-based 04）的任何片段（首行/field/形状）不得出现。
        assert!(
            !prompt.contains("f004"),
            "被省略条目不得残留半条片段: {prompt}"
        );
        assert!(
            !prompt.contains("missing_section:004:"),
            "被省略条目的首行一并省略: {prompt}"
        );
        // 呈现顺序稳定。
        let first = prompt.find("f000").expect("首条必须在场");
        let last = prompt.find("f003").expect("上限内末条必须在场");
        assert!(first < last, "呈现顺序必须稳定");
    }

    /// A1：事实块超**字节预算**（上限内条目体积超 32,000 B）⇒ 同样整条
    /// 省略 + 显式记账；条数上限不介入时计数由字节预算决定。
    #[test]
    fn parse_fact_block_over_byte_budget_omits_whole_entries_and_reports_counts() {
        // 每块固定 12,000 B ⇒ 渲染 12,003 B/条；32,000 B 预算内恰容 2 条
        //（2×12,003=24,006；3×12,003=36,009 超出），条数上限（4）不介入。
        let parse_reasons: Vec<String> = (0..3)
            .map(|index| {
                let mut block = format!(
                    "missing_section:{index:03}:m\n  缺失字段: f{index:03}\n  正确形状: - f{index:03}: value"
                );
                block.push_str(&"x".repeat(12_000 - block.len()));
                block
            })
            .collect();
        let prompt = plain_prompt(&parse_reasons, &[]);
        assert!(
            prompt.contains("共 3 条 / 呈现 2 条 / 省略 1 条"),
            "超字节预算必须显式记账总数/呈现数/省略数: {prompt}"
        );
        assert!(
            !prompt.contains("f002"),
            "被省略条目不得残留半条片段: {prompt}"
        );
        assert!(
            prompt.contains("缺失字段: f000") && prompt.contains("缺失字段: f001"),
            "预算内条目完整在场: {prompt}"
        );
    }

    /// A1：单条超大（自身即超 32,000 B）也整条省略，不得切半；其余小条
    /// 不受牵连。
    #[test]
    fn single_oversized_parse_fact_block_omitted_whole() {
        let oversized = format!(
            "missing_section:1:m\n  缺失字段: big\n  正确形状: {}",
            "v".repeat(40_000)
        );
        let parse_reasons = vec![
            oversized,
            "missing_section:2:m\n  缺失字段: ok\n  正确形状: - ok: value".to_string(),
        ];
        let prompt = plain_prompt(&parse_reasons, &[]);
        assert!(
            prompt.contains("共 2 条 / 呈现 1 条 / 省略 1 条"),
            "单条超大必须整条省略并记账: {prompt}"
        );
        assert!(prompt.contains("缺失字段: ok"), "小条完整在场: {prompt}");
        assert!(
            !prompt.contains("缺失字段: big"),
            "超大条不得切半残留: {prompt}"
        );
        assert!(
            !prompt.contains("missing_section:1:"),
            "超大条首行一并省略: {prompt}"
        );
    }

    /// A1 边界：单行呈现原文（IR 预检 failure_reasons 等旧材料）不受新预算
    /// 约束——总量超 32,000 B 仍逐条全量在场，无省略记账。
    #[test]
    fn plain_single_line_reasons_are_not_budget_truncated() {
        let parse_reasons: Vec<String> = (0..200)
            .map(|index| format!("unknown_requirement_ref:{index}:{}", "e".repeat(380)))
            .collect();
        let prompt = plain_prompt(&parse_reasons, &[]);
        assert!(
            prompt.contains("unknown_requirement_ref:0:"),
            "首条在场: {prompt}"
        );
        assert!(
            prompt.contains("unknown_requirement_ref:199:"),
            "末条在场（旧材料不截断）: {prompt}"
        );
        assert!(!prompt.contains("共 200 条"), "旧材料无省略记账: {prompt}");
    }

    /// A1（REQ-WSC-32）：unknown_structured_key 类失败附 40 个合法
    /// STRUCTURED_KEYS（仅此分支）；其它失败码不附键集。
    #[test]
    fn unknown_structured_key_failure_lists_legal_key_set() {
        let keys = crate::product::work_item_plan_compiler::grammar::STRUCTURED_KEYS.join("、");
        let prompt = plain_prompt(
            &["unknown_structured_key:7:未知结构化 key 必须拒绝。".to_string()],
            &[],
        );
        assert!(
            prompt.contains(&keys),
            "unknown_structured_key 必须附全部合法 key: {prompt}"
        );
        let plain = plain_prompt(&["missing_section:7:缺 Identity".to_string()], &[]);
        assert!(
            !plain.contains(&keys),
            "非 unknown_structured_key 失败不得附带键集: {plain}"
        );
    }

    /// P23（Plan §2.4）：上一轮失败源码逐字入 prompt——真实 source（自带反引号
    /// 围栏行 + CJK 多字节）按字节完整、围栏自洽；位置在失败事实之后、文末
    /// 指令之前（顺序断言，不 pin 句子）。
    #[test]
    fn previous_source_block_preserves_source_bytes_after_facts() {
        let parse_reasons = vec![
            "missing_section:5:结构化 section 缺少必需字段。\n  缺失字段: Goal\n  正确形状: - Goal: value"
                .to_string(),
        ];
        let prompt = plain_prompt(&parse_reasons, &[]);
        assert_eq!(
            prompt.matches(FIXTURE_SOURCE).count(),
            1,
            "上一轮源码必须逐字出现且仅一次: {prompt}"
        );
        let source_start = assert_lossless_source_block(&prompt);
        let fact = prompt.find("缺失字段: Goal").expect("parse 事实必须在场");
        assert!(fact < source_start, "source 块必须在失败事实之后");
        assert!(
            FIXTURE_SOURCE.chars().count() < FIXTURE_SOURCE.len(),
            "fixture 必须含 CJK 多字节正文，字节断言才有区分力"
        );
    }

    /// P23 × A1：parse 事实块超 32,000 B 整条省略语义不变（计数记账、不切半），
    /// 且 source 块仍逐字完整——省略不得波及本轮自带的上一轮源码。
    #[test]
    fn oversized_parse_fact_block_omitted_whole_keeps_source_verbatim() {
        let oversized = format!(
            "missing_section:1:m\n  缺失字段: big\n  正确形状: {}",
            "v".repeat(40_000)
        );
        let parse_reasons = vec![
            oversized,
            "missing_section:2:m\n  缺失字段: ok\n  正确形状: - ok: value".to_string(),
        ];
        let prompt = plain_prompt(&parse_reasons, &[]);
        assert!(
            prompt.contains("共 2 条 / 呈现 1 条 / 省略 1 条"),
            "超字节预算必须显式记账总数/呈现数/省略数: {prompt}"
        );
        assert!(prompt.contains("缺失字段: ok"), "小条完整在场: {prompt}");
        assert!(
            !prompt.contains("缺失字段: big"),
            "超大条不得切半残留: {prompt}"
        );
        assert!(
            !prompt.contains("missing_section:1:"),
            "超大条首行一并省略: {prompt}"
        );
        assert_eq!(
            prompt.matches(FIXTURE_SOURCE).count(),
            1,
            "source 正文必须仍逐字完整在场: {prompt}"
        );
        assert_lossless_source_block(&prompt);
    }

    /// P23：来源元数据与正文复算不符 ⇒ `Err`（错误只来自 source/render 失败），
    /// 不返回任何半成品 prompt。
    #[test]
    fn tampered_source_metadata_returns_error_instead_of_prompt() {
        let fixture = SourceFixture::of(FIXTURE_SOURCE);
        let mut metadata = fixture.metadata(FIXTURE_SOURCE);
        metadata.bytes += 1;
        let err = build_work_item_plan_compile_reredrive_prompt(
            &[],
            &[],
            &[],
            &[],
            &[],
            FIXTURE_SOURCE,
            &metadata,
        )
        .expect_err("bytes 篡改必须报错");
        assert_eq!(err.code, "sc_failure_evidence_invalid");
    }

    /// A1（REQ-WSC-32，自 `single_candidate_compile_preflight` 的 typed 重驱用例
    /// 迁移）：同 message、不同 field 的 parse 诊断块经重驱入口后，最终 prompt
    /// 逐条含原三段（逐字）+ 精确 field/形状，两条互不吞并。
    /// （资格判定侧 `teaching_verdict` 为 `run` 模块内的 `pub(super)` 面，其
    /// diagnostics→parse_reasons 投影由 `run/` 自己的测试覆盖。）
    #[test]
    fn typed_reredrive_prompt_keeps_parse_field_and_example_facts() {
        let fields = ["Goal", "role"];
        let parse_reasons: Vec<String> = fields
            .iter()
            .map(|field| {
                format!(
                    "missing_section:5:结构化 section 缺少必需字段。\n  缺失字段: {field}\n  正确形状: - {field}: value"
                )
            })
            .collect();
        let prompt = build(&parse_reasons, &[], &[], &[], &[]).expect("合法来源元数据必须构造成功");
        for field in fields {
            let block = format!(
                "missing_section:5:结构化 section 缺少必需字段。\n  缺失字段: {field}\n  正确形状: - {field}: value"
            );
            assert!(prompt.contains(&block), "{field} 逐条可区分: {prompt}");
            assert_eq!(
                prompt.matches(&block).count(),
                1,
                "{field} 块必须恰出现一次（不得互相吞并）: {prompt}"
            );
        }
    }
}
