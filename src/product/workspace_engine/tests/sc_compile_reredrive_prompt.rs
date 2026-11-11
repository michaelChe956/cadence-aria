// F2-B（SC compile 失败教学重驱）：missing_section 类 compile 失败不再直接终态——
// 构造「教学重驱 prompt」复用 build_artifact_retry_prompt（prompts.rs 100-133）的
// 模板形态 + compile 错误原文 + 「立即输出完整 work-item-plan markdown source，
// 第一行即标题」。契约测试风格对齐 part_10 的 build_artifact_retry_prompt 测试。

use super::*;

#[test]
fn work_item_plan_compile_reredrive_prompt_carries_errors_and_immediate_output_directive() {
    let reasons = vec![
        "missing_section:4:Work Item 缺少必需 section。".to_string(),
        "missing_section:6:Work Item 缺少必需 section。".to_string(),
    ];

    let prompt = build_work_item_plan_compile_reredrive_prompt(&reasons, &[], &[], &[], &[]);

    for required in [
        // build_artifact_retry_prompt 的模板形态（教学重驱先例措辞）。
        "上一轮已结束，但没有输出完整的 work-item-plan markdown source",
        "不要继续调研，不要只解释",
        // 立即重驱 + 第一行即标题。
        "立即输出完整 work-item-plan markdown source",
        "第一行即文档标题 `# Work Item Plan`",
        // compile 错误原文逐条回灌。
        "具体失败原因:",
        "missing_section:4:Work Item 缺少必需 section。",
        "missing_section:6:Work Item 缺少必需 section。",
    ] {
        assert!(
            prompt.contains(required),
            "compile re-drive prompt must contain {required}: {prompt}"
        );
    }
}

/// 迁移等价性护栏（决策六 Task 1）：调用方视角的路径
/// `crate::product::workspace_engine::build_work_item_plan_compile_reredrive_prompt`
/// 必须始终可解析，且返回的 prompt 含既有开场句——编译教学块从
/// `workspace_engine/prompts.rs` 迁到 `workspace_engine/prompts/sc_compile_teaching.rs`
/// 后，对外路径与文案均不变。
#[test]
fn work_item_plan_compile_reredrive_prompt_is_reachable_from_caller_path() {
    let reasons = vec!["missing_section:4:Work Item 缺少必需 section。".to_string()];

    let prompt = crate::product::workspace_engine::build_work_item_plan_compile_reredrive_prompt(
        &reasons,
        &[],
        &[],
        &[],
        &[],
    );

    assert!(
        prompt.contains("上一轮已结束，但没有输出完整的 work-item-plan markdown source"),
        "compile re-drive prompt must keep the existing opening sentence: {prompt}"
    );
}
