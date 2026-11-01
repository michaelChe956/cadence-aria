//! issue 级跨仓最终评审 prompt（REQ-GFR-CR-04「跨仓评审材料与 provider 选择」，
//! 设计 B §四 B3）。
//!
//! 全新 prompt：不复用 GroupFinalReview prompt 的「WorkItemGroup 全部 units」
//! 假设，也不触碰既有 code review / group review prompt 里写死的 source_stage
//! 约束文本（设计 B §四 B6「单仓一行不改」）。
//!
//! 本评审是**建议性（advisory）**：不得要求返工、不得输出门控指令；findings
//! 的 `required_action` 只作建议。

use super::*;
use crate::product::coding_models::CrossRepoRepositorySection;

/// 输出 schema 名（与既有 `coding_workspace_internal_pr_review_json` 同风格，
/// 设计 B §四 B2）。
pub const ISSUE_CROSS_REPO_REVIEW_OUTPUT_SCHEMA: &str =
    "coding_workspace_issue_cross_repo_review_json";

/// prompt 标题（与 `Coding Workspace GroupFinalReview` 同风格）。
pub(crate) const ISSUE_CROSS_REPO_REVIEW_TITLE: &str = "Coding Workspace IssueCrossRepoFinalReview";

/// 每仓 diff 分段预算（字符）：与 code review 同值（`review_parser.rs`
/// `truncate_prompt_section` 的 30_000，设计 B §四 B4）。
pub(crate) const CROSS_REPO_DIFF_SECTION_MAX_CHARS: usize = 30_000;

/// 跨仓材料全包预算（字节）：超出按仓顺序截断并显式标注被截断的仓，
/// 不静默省略（REQ-GFR-CR-04「材料超预算显式标注」）。
pub(crate) const CROSS_REPO_MATERIAL_MAX_BYTES: usize = 120_000;

/// 构建跨仓最终评审 prompt 主体。
///
/// 结构（设计 B §四 B3）：标题 → 逐仓分段（仓名 + work item 列表 + attempt +
/// 分支 + base..head diff）→ 跨仓一致性检查清单 → 输出 JSON 契约（含
/// `source_stage=issue_cross_repo_review`）→ 既有 reviewer 纪律段 → advisory
/// 红线写死。项目/issue/plan 头由调用方（engine）在标题后注入。
///
/// `sections[i]` 与 `diffs[i]` 一一对应；`diff_truncated` 的分段在材料末尾以
/// `truncated_repositories: [...]` 显式标注。
pub fn build_issue_cross_repo_review_prompt(
    sections: &[CrossRepoRepositorySection],
    diffs: &[String],
) -> String {
    let mut repository_blocks = String::new();
    for (index, section) in sections.iter().enumerate() {
        let diff = diffs.get(index).map(String::as_str).unwrap_or_default();
        repository_blocks.push_str(&format!(
            "\nRepository #{}:\n\
             Repository: {} (logical id: {})\n\
             Attempt: {}\n\
             Branch: {}\n\
             Diff range: {}..{}\n\
             Work Items: {}\n\
             \n完整变更 git diff:\n````diff\n{}\n````\n",
            index + 1,
            section.repository_name,
            section.logical_repository_id,
            section.attempt_id,
            section.branch_name,
            section.base_commit,
            section.head_commit,
            if section.work_item_ids.is_empty() {
                "-".to_string()
            } else {
                section.work_item_ids.join(", ")
            },
            diff,
        ));
    }

    let truncated: Vec<&str> = sections
        .iter()
        .filter(|section| section.diff_truncated)
        .map(|section| section.logical_repository_id.as_str())
        .collect();
    let truncation_note = if truncated.is_empty() {
        String::new()
    } else {
        format!(
            "\ntruncated_repositories: [{}]\n\
             （上述仓的 diff 因跨仓材料全包预算 {} 字节被按仓顺序截断；此为显式标注，\
             不是静默省略。仍必须基于这些仓的分段头信息（仓名 / attempt / 分支 / \
             diff 区间）审查其跨仓一致性，并在结论中说明材料受限。）\n",
            truncated.join(", "),
            CROSS_REPO_MATERIAL_MAX_BYTES,
        )
    };

    format!(
        "{title}\n\
         {runtime_contract}\
         你是 issue 级跨仓最终评审 reviewer（IssueCrossRepoFinalReview）。该 issue 下的\
         全部 work item 已完成开发，你审查的是**多个仓库协同交付**的整体一致性：单仓\
         内部质量已由各仓 code review 把关，你只看跨仓交界面的风险。\n\
         \n跨仓一致性检查清单:\n\
         - 契约一致性：各仓对同一契约（接口签名、数据模型、事件/消息格式、配置项）的\
         定义与使用是否一致；契约变更是否同步到全部消费方。\n\
         - 跨仓调用面：跨仓调用（HTTP/RPC/消息/共享库）的错误处理、超时、兼容性与\
         版本协商是否闭环；是否存在单向依赖断裂或循环依赖。\n\
         - 上下游交付闭环：上游产出与下游消费是否对齐（字段、语义、时序）；交付物\
         是否覆盖全部下游仓的接入需要。\n\
         \n逐仓材料：\n\
         {repository_blocks}\
         {truncation_note}\
         {test_scope_contract}\
         {evidence_boundary_contract}\
         {stack_assumption_contract}\
         \nadvisory 红线（写死，不可覆盖）:\n\
         - 本评审是建议性（advisory）：不得要求返工，不得输出任何门控指令。\n\
         - findings 的 required_action 只作建议，不构成阻塞条件；verdict 为\
         request_changes 或 blocked 也不会阻止交付确认。\n\
         \n输出要求:\n\
         - verdict 只能是 approve、request_changes、blocked 之一。\n\
         - 每条 finding 必须携带 source_stage=issue_cross_repo_review 与 repository\
         （所属仓名）。\n\
         - findings 必须落在跨仓交界面（契约一致性 / 跨仓调用面 / 上下游交付闭环），\
         不重复单仓内部问题。\n\
         \n只输出 JSON：{{\"verdict\":\"approve|request_changes|blocked\",\"summary\":\"...\",\"findings\":[{{\"severity\":\"error|warning|info\",\"file_path\":\"...\",\"line\":1,\"message\":\"...\",\"required_action\":\"...\",\"source_stage\":\"issue_cross_repo_review\",\"repository\":\"<仓名>\"}}]}}\n",
        title = ISSUE_CROSS_REPO_REVIEW_TITLE,
        runtime_contract = provider_runtime_contract("IssueCrossRepoFinalReview"),
        repository_blocks = repository_blocks,
        truncation_note = truncation_note,
        test_scope_contract = reviewer_test_scope_contract(),
        evidence_boundary_contract = reviewer_process_evidence_boundary_contract(),
        stack_assumption_contract = no_default_stack_assumption_contract(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(logical_id: &str, truncated: bool) -> CrossRepoRepositorySection {
        CrossRepoRepositorySection {
            repository_name: format!("repo-{logical_id}"),
            logical_repository_id: logical_id.to_string(),
            attempt_id: format!("attempt_{logical_id}"),
            branch_name: "aria/issues/issue_0001".to_string(),
            base_commit: "base".to_string(),
            head_commit: "head".to_string(),
            work_item_ids: vec!["work_item_0001".to_string()],
            diff_truncated: truncated,
        }
    }

    #[test]
    fn cross_repo_prompt_carries_title_and_per_repository_sections() {
        let sections = vec![section("logical_a", false), section("logical_b", false)];
        let diffs = vec!["diff-a".to_string(), "diff-b".to_string()];
        let prompt = build_issue_cross_repo_review_prompt(&sections, &diffs);
        assert!(prompt.contains("Coding Workspace IssueCrossRepoFinalReview"));
        assert!(prompt.contains("repo-logical_a"));
        assert!(prompt.contains("repo-logical_b"));
        assert!(prompt.contains("diff-a"));
        assert!(prompt.contains("diff-b"));
        assert!(prompt.contains("source_stage=issue_cross_repo_review"));
        assert!(!prompt.contains("truncated_repositories"));
    }

    #[test]
    fn cross_repo_prompt_marks_truncated_repositories_explicitly() {
        let sections = vec![section("logical_a", false), section("logical_b", true)];
        let diffs = vec!["diff-a".to_string(), String::new()];
        let prompt = build_issue_cross_repo_review_prompt(&sections, &diffs);
        assert!(prompt.contains("truncated_repositories: [logical_b]"));
        // 分段头仍保留（不静默省略整仓）。
        assert!(prompt.contains("repo-logical_b"));
    }
}
