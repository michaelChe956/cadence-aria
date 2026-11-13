//! P22：SC 教学重驱 prompt 的最终装配 facade（纯函数面：零 I/O、零 provider、零 store）。
//!
//! 分层（Plan §2.4）：
//! - 模板事实与 source 块渲染归模板层——compile 臂走 P23 的
//!   `build_work_item_plan_compile_reredrive_prompt`（7 参 + `Result`），
//!   结构臂走 P24 的 `author_structure_repair_prompt`（4 参 + `Result`）；
//! - IR 尾段（指令段 + IDs 段 + 末尾输出纪律）自旧
//!   `single_candidate_compile_preflight::append_ir_repair_instructions` 逐字搬入
//!   （唯一来源，不复制第二份）；
//! - 预算裁决、摘要与字节计数只发生在 `finish_sc_teaching_prompt`：先成全文，
//!   再裁决——尾段之后不再改文本。
//!
//! 超预算**不是**错误：全文逐字保留并返回 `RejectedBudget`（供审计）；`Err`
//! 只来自 source/render 失败（`sc_failure_evidence_invalid`）。预算上限只有一个
//! 来源：`WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES`（本模块不另立数值）。

use super::sc_teaching_source::ScResolvedTeachingSource;
use super::single_candidate_compile_preflight::{PreflightVerdict, TeachingPayload};
use crate::product::planning_failure::PlanningFailureSet;
use crate::product::workspace_engine::single_candidate_structure_failure::author_structure_repair_prompt;
use crate::product::workspace_engine::{
    PlanningCompileFailure, ScTeachingPromptError, ScTeachingSourceMetadata,
    build_work_item_plan_compile_reredrive_prompt,
};
use crate::product::work_item_split_engine::prompts::WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES;

/// 审计用构造器名（**不进 `prompt_ref`**）：取各臂今日的 builder 名，
/// 便于与既有 prompt 证据对照。
const COMPILE_BUILDER_ID: &str = "build_typed_compile_reredrive_prompt";
const IR_BUILDER_ID: &str = "build_preflight_ir_reredrive_prompt";
const STRUCTURE_BUILDER_ID: &str = "author_structure_repair_prompt";

/// 最终教学重驱 prompt 的装配结果（纯数据；超预算也带全文，供审计）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ScPreparedTeachingPrompt {
    /// 最终全文（超预算也逐字保留，不截断）。
    pub text: String,
    /// 审计用构造器名；**不进 `prompt_ref`**。
    pub builder_id: String,
    /// 来源 locator（resolver 解析出的真实 ref）。
    pub source_ref: String,
    /// 来源正文 sha256（小写 hex）。
    pub source_sha256: String,
    /// 来源正文 UTF-8 字节数。
    pub source_bytes: u64,
    /// 最终全文 sha256（小写 hex）。
    pub prompt_digest: String,
    /// 最终全文 UTF-8 字节数（裁决按字节，不按字符）。
    pub prompt_bytes: u64,
    /// 预算裁决（上限单一来源：`WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES`）。
    pub budget_verdict: ScTeachingPromptBudgetVerdict,
}

/// 预算裁决：超限不是错误（全文保留供审计），但不得据此放行教学窗口。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum ScTeachingPromptBudgetVerdict {
    /// 最终全文在预算内。
    Allowed,
    /// 最终全文超预算（`required_bytes` = 最终全文字节数）。
    RejectedBudget { required_bytes: u64, max_bytes: u64 },
}

/// compile 臂：模板事实 + source 块 → 最终全文与预算裁决。
pub(super) fn prepare_compile_teaching_prompt(
    payload: &TeachingPayload,
    autorepair_instructions: &[String],
    registered_requirement_ids: &[String],
    source: &ScResolvedTeachingSource,
) -> Result<ScPreparedTeachingPrompt, ScTeachingPromptError> {
    let metadata = source_metadata(source);
    let text = build_work_item_plan_compile_reredrive_prompt(
        &payload.parse_reasons,
        &payload.lowering_faults,
        &payload.ir_faults,
        autorepair_instructions,
        registered_requirement_ids,
        &source.text,
        &metadata,
    )?;
    Ok(finish_sc_teaching_prompt(
        text,
        COMPILE_BUILDER_ID,
        source,
    ))
}

/// IR 臂：模板事实 + source 块（base）→ IR 尾段（指令段 + IDs 段 + 末尾输出
/// 纪律）→ 最终全文与预算裁决。
pub(super) fn prepare_ir_teaching_prompt(
    preflight: &PreflightVerdict,
    registered_requirement_ids: &[String],
    source: &ScResolvedTeachingSource,
) -> Result<ScPreparedTeachingPrompt, ScTeachingPromptError> {
    let metadata = source_metadata(source);
    let base = build_work_item_plan_compile_reredrive_prompt(
        &preflight.failure_reasons,
        &[],
        &preflight.ir_faults,
        &[],
        registered_requirement_ids,
        &source.text,
        &metadata,
    )?;
    let text = append_ir_repair_instructions(base, registered_requirement_ids);
    Ok(finish_sc_teaching_prompt(text, IR_BUILDER_ID, source))
}

/// 结构臂：结构模板（事实 + typed 集 + source 块 + `REVISION_TEACHING`）→
/// 最终全文与预算裁决。
pub(super) fn prepare_structure_teaching_prompt(
    failure: &PlanningCompileFailure,
    structure_failures: &PlanningFailureSet,
    source: &ScResolvedTeachingSource,
) -> Result<ScPreparedTeachingPrompt, ScTeachingPromptError> {
    let metadata = source_metadata(source);
    let text = author_structure_repair_prompt(
        failure,
        structure_failures,
        &source.text,
        &metadata,
    )?;
    Ok(finish_sc_teaching_prompt(
        text,
        STRUCTURE_BUILDER_ID,
        source,
    ))
}

/// 唯一算 digest / bytes / verdict 的地方：先成全文，再裁决预算。
///
/// 超预算不返回错误——全文逐字保留（`RejectedBudget`），供审计与拒绝取证。
pub(super) fn finish_sc_teaching_prompt(
    text: String,
    builder_id: &str,
    source: &ScResolvedTeachingSource,
) -> ScPreparedTeachingPrompt {
    let prompt_bytes = text.len() as u64;
    let max_bytes = WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES as u64;
    let budget_verdict = if prompt_bytes <= max_bytes {
        ScTeachingPromptBudgetVerdict::Allowed
    } else {
        ScTeachingPromptBudgetVerdict::RejectedBudget {
            required_bytes: prompt_bytes,
            max_bytes,
        }
    };
    ScPreparedTeachingPrompt {
        prompt_digest: sha256_hex(&text),
        text,
        builder_id: builder_id.to_string(),
        source_ref: source.failure_source_ref.clone(),
        source_sha256: source.sha256.clone(),
        source_bytes: source.bytes,
        prompt_bytes,
        budget_verdict,
    }
}

/// 来源元数据（只读借用；完整性由 P21 renderer 复算校验）。
fn source_metadata(source: &ScResolvedTeachingSource) -> ScTeachingSourceMetadata<'_> {
    ScTeachingSourceMetadata {
        source_ref: &source.failure_source_ref,
        sha256: &source.sha256,
        bytes: source.bytes,
        origin: source.origin.clone(),
    }
}

/// IR 重驱 prompt 的既有修复指令尾段（自旧
/// `single_candidate_compile_preflight::append_ir_repair_instructions` 逐字搬入；
/// 措辞与换行零改动）。
fn append_ir_repair_instructions(
    mut prompt: String,
    registered_requirement_ids: &[String],
) -> String {
    prompt.push_str(
        "上述为 IR 校验失败（引用或字段不符合契约）。\n\
         修正引用/补齐字段：requirement_refs、done_when_refs、reviewer_check_refs 只能逐字引用本计划已定义 id；\
         每个 criterion_id 必须有配对的 reviewer_check_refs 行。\n",
    );
    // r41 A2/r45 扩 AC:附已登记编号清单(产品侧与 harness 反馈轮对称;
    // story+design 双源 REQ-/NFR-/AC-,与 fresh prompt 的
    // [design_requirements] 同集)。
    if !registered_requirement_ids.is_empty() {
        prompt.push_str(&format!(
            "[design_requirements] {}\n\
             requirement_refs 与 done_when 的引用只能取自上列清单(REQ-*/NFR-*/AC-*),\
             清单外编号一律删除或改为清单内编号。\n",
            registered_requirement_ids.join("、")
        ));
    }
    prompt.push_str("修正后重新输出完整 plan，第一行即 `# Work Item Plan`，不要输出解释。\n");
    prompt
}

/// 全文摘要（sha256 小写 hex）——与证据库同一口径。
fn sha256_hex(text: &str) -> String {
    use sha2::Digest as _;
    hex::encode(sha2::Sha256::digest(text.as_bytes()))
}
// P22 RED：最终预算装配的 facade 边界（3 个字节边界场景）。
//
// 落位：src/web/workspace_ws_handler/run/sc_teaching_prompt.rs 末尾（inline test module）。
// 消费真实 facade API（prepare_compile_teaching_prompt / prepare_ir_teaching_prompt），
// 不 mock 任何层、不 pin 任何提示词措辞——只钉字节数、预算裁决、摘要与全文在场。
//
// 依赖：P20 的 ScResolvedTeachingSource（run::single_candidate::sc_teaching_source）与
// P23/P24 的模板新签名（previous_source + ScTeachingSourceMetadata）。
// RED 现状：facade 模块尚不存在 ⇒ 编译失败（E0433/E0432），这是设计中的 RED。
#[cfg(test)]
mod tests {
    use super::{
        ScPreparedTeachingPrompt, ScTeachingPromptBudgetVerdict, prepare_compile_teaching_prompt,
        prepare_ir_teaching_prompt,
    };
    use super::super::sc_teaching_source::ScResolvedTeachingSource;
    use super::super::single_candidate_compile_preflight::{PreflightVerdict, TeachingPayload};
    use crate::product::work_item_plan_compiler::{IrFault, IrFaultKind, IrFaultLocator};
    use crate::product::work_item_split_engine::prompts::WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES;
    use crate::web::workspace_ws_types::ScTeachingSourceOrigin;
    use sha2::Digest as _;

    /// 预算上限只有这一个来源（facade 不得另立第二数值）。
    const MAX: usize = WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES;
    const SOURCE_REF: &str = "sc-split-output/v1/session-0001/1";
    /// 校准样本字节数：与目标样本同为 6 位数 ⇒ 元数据头长度不变 ⇒ 差值精确。
    const CALIBRATION_BYTES: usize = 100_000;

    fn sha256_hex(body: &str) -> String {
        hex::encode(sha2::Sha256::digest(body.as_bytes()))
    }

    /// 精确字节数的填充体：CJK 3 字节/字，余数用 ASCII 补齐（保证 chars != bytes）。
    fn filler(bytes: usize) -> String {
        let mut text = String::with_capacity(bytes);
        let mut remaining = bytes;
        while remaining >= 3 {
            text.push('中');
            remaining -= 3;
        }
        while remaining > 0 {
            text.push('x');
            remaining -= 1;
        }
        text
    }

    fn resolved(text: String) -> ScResolvedTeachingSource {
        // 先算摘要/字节，再 move text（否则借用已移动的值 ⇒ E0382）。
        let sha256 = sha256_hex(&text);
        let bytes = text.len() as u64;
        ScResolvedTeachingSource {
            text,
            failure_source_ref: SOURCE_REF.to_string(),
            sha256,
            bytes,
            origin: ScTeachingSourceOrigin::CompileSplitOutput,
        }
    }

    fn compile_payload() -> TeachingPayload {
        TeachingPayload {
            parse_reasons: vec!["missing_section:5:结构化 section 缺少必需字段。\n  缺失字段: Goal\n  正确形状: - Goal: value".to_string()],
            lowering_faults: Vec::new(),
            ir_faults: Vec::new(),
        }
    }

    /// IR 臂的等价事实：preflight 与 compile 模板用的是同一批 typed 事实。
    fn ir_facts() -> (PreflightVerdict, TeachingPayload) {
        let fault = IrFault {
            kind: IrFaultKind::UnknownRequirementRef,
            work_item_id: "WI-001".to_string(),
            locator: IrFaultLocator::Task {
                task_id: "TASK-001".to_string(),
            },
            offending_ref: "REQ-002".to_string(),
            legal_refs: vec!["REQ-001".to_string()],
            source_line: Some(30),
            // 期望形态取 kind 的单一事实源（IrFaultKind::expected_shape），
            // 不手写、不猜；字段本身是 IrFault 的第 7 个真实字段。
            expected_shape: IrFaultKind::UnknownRequirementRef.expected_shape().to_string(),
        };
        let failure_reasons = vec![
            "unknown_requirement_ref: 第 30 行 task TASK-001 的引用 REQ-002 不在合法集合内".to_string(),
        ];
        let preflight = PreflightVerdict {
            ir_faults: vec![fault.clone()],
            operational_failures: Vec::new(),
            failure_reasons: failure_reasons.clone(),
        };
        let payload = TeachingPayload {
            parse_reasons: failure_reasons,
            lowering_faults: Vec::new(),
            ir_faults: vec![fault],
        };
        (preflight, payload)
    }

    /// 组装开销（模板/围栏/元数据头/尾段，不含 source 正文）：两趟差值。
    fn assembly_overhead(payload: &TeachingPayload, ids: &[String]) -> usize {
        let calibration = resolved(filler(CALIBRATION_BYTES));
        let prepared = prepare_compile_teaching_prompt(payload, &[], ids, &calibration)
            .expect("校准 prompt 必须可组装");
        prepared.prompt_bytes as usize - CALIBRATION_BYTES
    }

    /// ① 最终全文恰好等于预算上限（262144 B）⇒ Allowed。
    /// source 含中文 ⇒ chars != bytes（证明裁决口径是字节不是字符）。
    #[test]
    fn final_prompt_at_budget_ceiling_is_allowed_with_cjk_source() {
        let payload = compile_payload();
        let ids = vec!["REQ-ENV-01".to_string(), "AC-001".to_string()];
        let overhead = assembly_overhead(&payload, &ids);
        let source = resolved(filler(MAX.saturating_sub(overhead)));
        let prepared: ScPreparedTeachingPrompt =
            prepare_compile_teaching_prompt(&payload, &[], &ids, &source)
                .expect("预算内 prompt 必须返回");
        assert_eq!(
            prepared.prompt_bytes as usize, MAX,
            "最终全文必须恰好等于预算上限"
        );
        assert_eq!(prepared.budget_verdict, ScTeachingPromptBudgetVerdict::Allowed);
        assert!(
            prepared.text.contains(source.text.as_str()),
            "source 必须逐字完整在场"
        );
        assert!(
            prepared.text.chars().count() < prepared.prompt_bytes as usize,
            "中文 source ⇒ 字符数 != 字节数（裁决按字节）"
        );
        assert_eq!(prepared.prompt_digest, sha256_hex(&prepared.text));
        assert_eq!(prepared.source_ref, source.failure_source_ref);
        assert_eq!(prepared.source_sha256, source.sha256);
        assert_eq!(prepared.source_bytes, source.bytes);
        assert!(!prepared.builder_id.is_empty(), "builder_id 必须可审计");
    }

    /// ② 超出上限 1 字节（262145 B）⇒ RejectedBudget，但**全文逐字保留**
    /// （不得 Err 丢正文：超限也要可审计）。
    #[test]
    fn one_byte_over_budget_keeps_full_text_and_reports_rejected_budget() {
        let payload = compile_payload();
        let ids = vec!["REQ-ENV-01".to_string()];
        let overhead = assembly_overhead(&payload, &ids);
        let source = resolved(filler((MAX + 1).saturating_sub(overhead)));
        let prepared = prepare_compile_teaching_prompt(&payload, &[], &ids, &source)
            .expect("超预算必须返回完整正文，不得 Err 丢全文");
        assert_eq!(prepared.prompt_bytes as usize, MAX + 1);
        assert_eq!(
            prepared.text.len(),
            MAX + 1,
            "超预算时正文必须逐字保留供审计"
        );
        assert!(
            prepared.text.contains(source.text.as_str()),
            "source 必须逐字完整在场"
        );
        assert_eq!(
            prepared.budget_verdict,
            ScTeachingPromptBudgetVerdict::RejectedBudget {
                required_bytes: (MAX + 1) as u64,
                max_bytes: MAX as u64,
            }
        );
        assert_eq!(prepared.prompt_digest, sha256_hex(&prepared.text));
    }

    /// ③ IR 臂：base（IR 尾段之前的组装面）仍在预算内，IR 尾段 + IDs 把最终
    /// 全文顶出预算 ⇒ required_bytes 必须是**最终全文长度**，不是 base 长度。
    #[test]
    fn ir_tail_over_budget_reports_final_length_not_base_length() {
        let (preflight, payload) = ir_facts();
        let ids = vec!["REQ-ENV-01".to_string()];
        let overhead = assembly_overhead(&payload, &ids);
        let margin = 64usize;
        let source = resolved(filler(MAX.saturating_sub(margin + overhead)));
        // 可观测的 base 代理：IR base 与 compile 模板同源同事实（见 Plan §2.4）。
        let base = prepare_compile_teaching_prompt(&payload, &[], &ids, &source)
            .expect("base 组装必须可完成");
        assert!(
            base.prompt_bytes as usize <= MAX,
            "IR 尾段之前的 base 必须仍在预算内"
        );
        let prepared = prepare_ir_teaching_prompt(&preflight, &ids, &source)
            .expect("IR 超预算必须返回完整正文");
        assert!(
            prepared.prompt_bytes as usize > MAX,
            "IR 尾段 + IDs 必须把最终全文顶出预算"
        );
        assert_eq!(
            prepared.budget_verdict,
            ScTeachingPromptBudgetVerdict::RejectedBudget {
                required_bytes: prepared.prompt_bytes,
                max_bytes: MAX as u64,
            },
            "required_bytes 必须是最终全文长度（不是 base 长度）"
        );
        assert_eq!(prepared.text.len() as u64, prepared.prompt_bytes);
        assert!(
            prepared.text.contains(source.text.as_str()),
            "source 必须逐字完整在场"
        );
    }
}
