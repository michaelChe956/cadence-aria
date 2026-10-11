//! F5-A：SingleCandidate author 修订轮 findings 回灌 prompt。
//!
//! 根因（run1d）：SC 修订轮 author 重跑的 prompt 与首轮逐字节相同，
//! reviewer findings 从未注入，author 每轮全新生成、不可能修复看不到的问题。
//! 本模块在首轮 prompt 的共享尾部输出指令之前插入 [review_revision] 返修段：
//! comments/summary 拼装对齐 legacy `prompts/revision.rs` 的
//! `build_revision_delta_prompt`（trusted_review_comments + summary），
//! 并附加硬性逐条修复指令。首轮（无 verdict）路径完全不经过本模块，
//! 其 prompt 逐字节不变。

use super::{
    IssueRecord, RepositoryRecord, WORK_ITEM_PLAN_MARKDOWN_OUTPUT_DIRECTIVE,
    WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES, WorkItemPlanMarkdownAuthorContext,
    build_work_item_plan_markdown_prompt,
};
use crate::product::workspace_engine::trusted_review_comments;
use crate::web::types::GenerateWorkItemsRequest;
use crate::web::workspace_ws_types::{ReviewFindingSeverity, ReviewVerdict};

/// 构造 SC author 修订轮 prompt：复用首轮完整 prompt（逐字节），仅在尾部
/// 输出指令之前插入 reviewer findings 回灌段与硬性修复指令。
pub(crate) fn build_work_item_plan_markdown_revision_prompt(
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
    repository: &RepositoryRecord,
    context: WorkItemPlanMarkdownAuthorContext<'_>,
    review: &ReviewVerdict,
) -> Result<String, String> {
    let base = build_work_item_plan_markdown_prompt(request, issue, repository, context)?;
    let Some(output_pos) = base.rfind(WORK_ITEM_PLAN_MARKDOWN_OUTPUT_DIRECTIVE) else {
        return Err(
            "work item plan markdown revision prompt cannot locate the shared output directive"
                .to_string(),
        );
    };
    let revision_block = render_review_revision_block(review);
    let mut prompt = String::with_capacity(base.len() + revision_block.len());
    prompt.push_str(&base[..output_pos]);
    prompt.push_str(&revision_block);
    prompt.push_str(WORK_ITEM_PLAN_MARKDOWN_OUTPUT_DIRECTIVE);
    if prompt.len() > WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES {
        return Err(format!(
            "work item plan markdown revision prompt exceeds hard budget: {} > {} bytes",
            prompt.len(),
            WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES
        ));
    }
    Ok(prompt)
}

/// B2c（Task 5.5，tasks.md 5.5 / 设计 §5.3 sc_author 行）：SC 显式 fresh 的
/// 返修轮 prompt——全新 provider 会话（resume=None，不回捞旧 native）没有
/// 旧会话历史可依赖，返修上下文必须自足：在既有返修段之前插入
/// `[previous_source]` 小节（durable 上一版 candidate source 全文，由调用方
/// 从 source store 按 ref 装载），声明「全新会话、不依赖任何历史会话」。
/// 正常 resume 的修订 prompt（`build_work_item_plan_markdown_revision_prompt`，
/// 依赖旧会话记忆「上一版」）保持零变化；首轮（无 verdict）fresh 不经本函数。
pub(crate) fn build_work_item_plan_markdown_fresh_revision_prompt(
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
    repository: &RepositoryRecord,
    context: WorkItemPlanMarkdownAuthorContext<'_>,
    review: &ReviewVerdict,
    previous_source: &str,
) -> Result<String, String> {
    let base = build_work_item_plan_markdown_prompt(request, issue, repository, context)?;
    let Some(output_pos) = base.rfind(WORK_ITEM_PLAN_MARKDOWN_OUTPUT_DIRECTIVE) else {
        return Err(
            "work item plan markdown fresh revision prompt cannot locate the shared output directive"
                .to_string(),
        );
    };
    let mut previous_block = String::new();
    previous_block.push_str(
        "[previous_source]\n\
         本次是对 work-item-plan source 的显式全新会话返修轮：旧 provider 会话已因漂移废弃，本轮不依赖任何历史会话；\n\
         以下代码围栏内是上一版完整 markdown source（durable 候选原文），本轮必须输出在上一版基础上修复后的完整 markdown source。\n\n",
    );
    previous_block.push_str("```markdown\n");
    previous_block.push_str(previous_source.trim());
    previous_block.push_str("\n```\n\n");
    let revision_block = render_review_revision_block(review);
    let mut prompt =
        String::with_capacity(base.len() + previous_block.len() + revision_block.len());
    prompt.push_str(&base[..output_pos]);
    prompt.push_str(&previous_block);
    prompt.push_str(&revision_block);
    prompt.push_str(WORK_ITEM_PLAN_MARKDOWN_OUTPUT_DIRECTIVE);
    if prompt.len() > WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES {
        return Err(format!(
            "work item plan markdown fresh revision prompt exceeds hard budget: {} > {} bytes",
            prompt.len(),
            WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES
        ));
    }
    Ok(prompt)
}

fn render_review_revision_block(review: &ReviewVerdict) -> String {
    let mut block = String::new();
    block.push_str(
        "[review_revision]\n\
         这是对当前 work-item-plan source 的返修轮：以下 reviewer findings 指向上一版 source 的具体缺陷；本轮必须输出在上一版基础上修复后的完整 markdown source。\n\n",
    );
    block.push_str("Reviewer 审核意见:\n");
    if let Some(comments) = trusted_review_comments(review) {
        block.push_str(comments);
    } else {
        block.push_str("（reviewer 未提供可信意见全文；以下列 findings 为准。）");
    }
    block.push_str("\n\nReviewer 摘要:\n");
    block.push_str(&review.summary);
    block.push_str("\n\n[review_findings]\n");
    if review.findings.is_empty() {
        block.push_str("（本轮无可注入的结构化 findings；按 Reviewer 审核意见逐条修复。）");
    } else {
        for (index, finding) in review.findings.iter().enumerate() {
            block.push_str(&format!(
                "{}. severity: {}\n   message: {}\n   evidence: {}\n   required_action: {}\n",
                index + 1,
                severity_text(&finding.severity),
                finding.message.trim(),
                finding.evidence.trim(),
                finding.required_action.trim(),
            ));
        }
        block.push_str("（severity=suggestion 的条目为建议项；must_fix/blocking 条目全部为本轮必须修复项。）\n");
    }
    block.push_str(
        "\n[revision_directives]\n\
         - 返修=逐字应用：每条 required_action 给出的补丁行/删除操作逐字应用到上一版 source 的对应位置；除该行外，已通过部分逐字节保持不变，禁止重写、重排或润色未涉及的 section。\n\
         - 必须在本轮内逐条修复全部 must_fix/blocking findings，逐条对齐其 required_action；不得只修复其中部分。\n\
         - 不得新增与上述 findings 无关的其他变更；未涉及的 section 保持与上一版一致。\n\
         - 若某条 finding 无法在本轮内修复，不要静默跳过：在对应 Work Item 的 Blockers 中如实登记 blocker 及原因。\n\n",
    );
    block
}

fn severity_text(severity: &ReviewFindingSeverity) -> &'static str {
    match severity {
        ReviewFindingSeverity::Blocking => "blocking",
        ReviewFindingSeverity::MustFix => "must_fix",
        ReviewFindingSeverity::Suggestion => "suggestion",
    }
}
