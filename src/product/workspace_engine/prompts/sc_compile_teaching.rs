//! SC compile 教学重驱 prompt 构造面，自 prompts.rs 拆出以守 1200 行文件守护
//! （先例：同仓另一模块的 `work_item_split_engine/prompts/sc_author_teaching.rs`、
//! `work_item_split_engine/prompts/sc_revision.rs`）。
//!
//! 对外路径不变——`prompts` 原样 re-export 本模块入口，调用点
//! `crate::product::workspace_engine::build_work_item_plan_compile_reredrive_prompt`
//! 仍可解析。

// 本模块的函数体只用到 `crate::` 全路径，无需父模块的名字；此处保留
// `use super::*;` 仅为让迁入文档注释里的 intra-doc 链接
// [`build_artifact_retry_prompt`] 在本模块内仍可解析（rustc 的
// unused_imports 不把文档链接计为使用，故显式豁免）。
#[allow(unused_imports)]
use super::*;

/// F2-B：SC compile 失败教学重驱 prompt（每 candidate 恰一次）。复用
/// [`build_artifact_retry_prompt`] 的模板形态（「上一轮已结束，但没有输出完整
/// artifact…不要继续调研，不要只解释…立即输出完整…」）+ compile 错误原文
/// 逐条回灌 + work-item-plan markdown source 的硬性重驱指令（第一行即文档标题）。
///
/// r46 核心修(三角色收敛):并入登记 ID 清单+短规则——compile 教学重驱
/// 此前只有错误原文+B1 白名单,ID 纪律裸奔(现场:r46 首轮漏填
/// requirement_id → 重驱改编号/新增清单外编号,震荡不收敛)。清单与
/// IR 重驱/harness 反馈轮同集(story+design 双源 REQ-/NFR-/AC-)。
pub(crate) fn build_work_item_plan_compile_reredrive_prompt(
    blocking_reasons: &[String],
    registered_requirement_ids: &[String],
) -> String {
    let mut prompt = String::from(
        "上一轮已结束，但没有输出完整的 work-item-plan markdown source。\n\
         不要继续调研，不要只解释，不要输出任何前言或路由回执。\n\
         请基于已有上下文和刚才读取的文件，立即输出完整 work-item-plan markdown source；第一行即文档标题 `# Work Item Plan`。\n",
    );
    if !blocking_reasons.is_empty() {
        prompt.push_str("\n具体失败原因:\n");
        for reason in blocking_reasons {
            prompt.push_str("- ");
            prompt.push_str(reason);
            prompt.push('\n');
        }
        // r41 B1(诊断增强):unknown_structured_key 类失败附「合法 section/key
        // 白名单 + 具体动作」——错误码单行不足以让 AI 自纠(它不知道合法集)。
        if blocking_reasons
            .iter()
            .any(|reason| reason.contains("unknown_structured_key"))
        {
            let sections =
                crate::product::work_item_plan_compiler::grammar::STRUCTURED_SECTIONS.join("、");
            let free =
                crate::product::work_item_plan_compiler::grammar::FREE_TEXT_SECTIONS.join("、");
            prompt.push_str(&format!(
                "\n结构化 section/key 白名单(结构化段只允许这些 key):\n- 结构化: {sections}\n- 自由文本: {free}\n\
                 动作:删除违例 section,或将其内容并入语义最近的合法 section 后重新输出完整 plan。\n"
            ));
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
    prompt.push('\n');
    prompt
}

#[cfg(test)]
mod reredrive_prompt_tests {
    use super::build_work_item_plan_compile_reredrive_prompt;

    /// r41 B1:unknown_structured_key 类失败的重驱 prompt 必须附 14 合法
    /// section/key 白名单与具体动作(AI 不知道合法集就无法自纠)。
    #[test]
    fn compile_reredrive_prompt_lists_section_whitelist_for_unknown_key() {
        let reasons = vec!["unknown_structured_key:1:未知结构化 key `NotAPermission`".to_string()];
        let prompt = build_work_item_plan_compile_reredrive_prompt(&reasons, &[]);
        assert!(prompt.contains("unknown_structured_key"));
        assert!(
            prompt.contains("结构化: Identity"),
            "白名单必须列出合法 section 集: {prompt}"
        );
        assert!(prompt.contains("Traceability"));
        assert!(
            prompt.contains("删除违例 section"),
            "必须给具体动作: {prompt}"
        );
    }

    /// 对照:非 unknown_structured_key 失败不附白名单(保持既有形态)。
    #[test]
    fn compile_reredrive_prompt_keeps_plain_form_without_unknown_key() {
        let reasons = vec!["missing_section:0:缺 Identity".to_string()];
        let prompt = build_work_item_plan_compile_reredrive_prompt(&reasons, &[]);
        assert!(!prompt.contains("白名单"));
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
        let prompt = build_work_item_plan_compile_reredrive_prompt(&reasons, &registered);
        assert!(
            prompt.contains("[design_requirements] REQ-ENV-01、NFR-PERF-02、AC-001"),
            "真实登记清单(双源同集 REQ/NFR/AC)必须并入: {prompt}"
        );
        assert!(
            prompt.contains("requirement_id 必填且逐字取自上列清单"),
            "必填规则必须在场: {prompt}"
        );
        assert!(
            prompt.contains("不重编号、禁止新增清单外 REQ-*/NFR-*/AC-*/TASK"),
            "修复纪律必须在场: {prompt}"
        );
        assert!(
            prompt.contains("done_when 只指向已定义编号"),
            "done_when 约束必须在场: {prompt}"
        );
    }

    /// r46:清单空时不注入(无 spec 上下文的老会话零变化)。
    #[test]
    fn compile_reredrive_prompt_omits_empty_registered_ids() {
        let reasons = vec!["missing_section:0:缺 Identity".to_string()];
        let prompt = build_work_item_plan_compile_reredrive_prompt(&reasons, &[]);
        assert!(!prompt.contains("[design_requirements]"));
    }
}
