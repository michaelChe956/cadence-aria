use std::collections::BTreeSet;
use std::path::Path;

use serde_json::json;

use crate::product::lifecycle_store::LifecycleStore;
use crate::product::logical_codebase::member_reality::{
    MEMBER_REALITY_MAX_LAYERED_LINES, MEMBER_REALITY_MAX_TOP_DIRECTORIES,
    summarize_layered_directories,
};
use crate::product::models::{DesignContextCapabilities, IssueRecord, WorkspaceType};
use crate::product::workspace_engine::{
    artifact_constraint_spec_for, heading_matches_rule, normalize_workspace_heading_line,
};
use crate::web::error::{ApiError, ApiResult};
use crate::web::types::GenerateWorkItemsRequest;

/// 多仓目录摘要：读取失败的显式前缀（REQ-PLN-17）。
const REPOSITORY_STRUCTURE_UNAVAILABLE_PREFIX: &str = "repository_structure_unavailable:";

/// 多仓目录摘要：空仓的显式前缀（REQ-PLN-17）。
const REPOSITORY_STRUCTURE_EMPTY_PREFIX: &str = "repository_structure_empty:";

pub fn design_context_capabilities_for_request(
    lifecycle: &LifecycleStore,
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
) -> ApiResult<DesignContextCapabilities> {
    let design_context = collect_design_context(lifecycle, request, issue)?;
    Ok(merge_design_context_capabilities(&design_context))
}

pub fn design_context_gaps(capabilities: &DesignContextCapabilities) -> Vec<String> {
    let mut gaps = Vec::new();
    if !capabilities.has_architecture {
        gaps.push("missing_architecture".to_string());
    }
    if !capabilities.has_module_breakdown {
        gaps.push("missing_module_breakdown".to_string());
    }
    if !capabilities.has_tech_stack {
        gaps.push("missing_tech_stack".to_string());
    }
    if !capabilities.has_test_strategy {
        gaps.push("missing_test_strategy".to_string());
    }
    if !capabilities.has_key_paths {
        gaps.push("missing_key_paths".to_string());
    }
    gaps
}

pub(crate) fn collect_story_context(
    lifecycle: &LifecycleStore,
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
) -> ApiResult<Vec<String>> {
    let project_id = &issue.project_id;
    let issue_id = &issue.id;
    let story_specs = lifecycle
        .list_story_specs(project_id, issue_id)
        .map_err(super::types::product_store_api_error)?;

    request
        .story_spec_ids
        .iter()
        .map(|id| {
            let spec = story_specs.iter().find(|s| &s.id == id).ok_or_else(|| {
                ApiError::runtime("story_spec_not_found", "story spec not found", json!({}))
            })?;
            let markdown = latest_markdown(lifecycle, project_id, issue_id, id)?;
            Ok(format!(
                "Story Spec: {} ({})\n{}",
                spec.title, spec.id, markdown
            ))
        })
        .collect::<ApiResult<Vec<_>>>()
}

pub(crate) fn collect_design_context(
    lifecycle: &LifecycleStore,
    request: &GenerateWorkItemsRequest,
    issue: &IssueRecord,
) -> ApiResult<Vec<String>> {
    let project_id = &issue.project_id;
    let issue_id = &issue.id;
    let design_specs = lifecycle
        .list_design_specs(project_id, issue_id)
        .map_err(super::types::product_store_api_error)?;

    request
        .design_spec_ids
        .iter()
        .map(|id| {
            let spec = design_specs.iter().find(|s| &s.id == id).ok_or_else(|| {
                ApiError::runtime("design_spec_not_found", "design spec not found", json!({}))
            })?;
            let markdown = latest_markdown(lifecycle, project_id, issue_id, id)?;
            Ok(format!(
                "Design Spec: {} ({})\n{}",
                spec.title, spec.id, markdown
            ))
        })
        .collect::<ApiResult<Vec<_>>>()
}

/// 从已确认 Design Spec 的 markdown 上下文提取已登记的需求 ID。
///
/// DesignSpecRecord 本身只保存 spec 元数据，需求正文位于其 latest version markdown；
/// 因此这里只接受 markdown 中显式出现的 REQ-/NFR- token，不为 prompt 生成任何 ID。
#[cfg(test)] // 单测断言用薄包装;生产直接消费 extract_registered_requirement_ids。
pub(crate) fn extract_design_requirement_ids(design_context: &[String]) -> Vec<String> {
    extract_registered_requirement_ids(design_context)
}

/// r41 A1(结构根修)/r45 扩 AC:从 story+design 双源上下文提取已登记的
/// 引用 ID(REQ-/NFR- 需求编号 + AC- 验收标准编号)。
///
/// 此前仅扫 design 上下文——design 正文缺 NFR token 时白名单缺员,反而逼
/// AI 编造编号(r33/r37 现场三轮编造的结构诱因);story 正文中的 REQ-/NFR-
/// 同属已登记需求域,双源并扫消除缺员窗口。r45:story schema 强制 AC-*
/// (验收标准),task done_when 引用的 AC 编号同属登记域——第五变体
/// (AC-002/AC-003 编造)证明 AC 是独立命名空间漏口,并入同集提取。
/// pub:矩阵 harness 复用同一实现(消除与 harness 侧提取器的双实现漂移)。
pub fn extract_registered_requirement_ids(contexts: &[String]) -> Vec<String> {
    let mut ids = BTreeSet::new();
    for context in contexts {
        for token in context
            .split(|character: char| !(character.is_ascii_alphanumeric() || character == '-'))
        {
            if (token.starts_with("REQ-") || token.starts_with("NFR-") || token.starts_with("AC-"))
                && token.len() > "AC-".len()
            {
                ids.insert(token.to_string());
            }
        }
    }
    ids.into_iter().collect()
}

fn latest_markdown(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    entity_id: &str,
) -> ApiResult<String> {
    let versions = lifecycle
        .list_versions(project_id, issue_id, entity_id)
        .map_err(super::types::product_store_api_error)?;
    Ok(versions
        .into_iter()
        .max_by_key(|v| v.version)
        .map(|v| v.markdown)
        .unwrap_or_else(|| "(no version)".to_string()))
}

/// issue 是否归属已登记（非 legacy 别名）逻辑代码库——多仓 prompt 形态判据。
pub(crate) fn is_lc_issue(issue: &IssueRecord) -> bool {
    issue.logical_codebase_id
        != crate::product::logical_codebase::legacy_logical_codebase_id(&issue.project_id)
}

/// 目录摘要（单仓 / 多仓分派）。
///
/// - `logical == false`（单仓）：原实现逐字保留（单层 `truncate(30)` +
///   `(empty repository)`），单仓 prompt 字节 MUST 不变。
/// - `logical == true`（多仓，REQ-PLN-17）：分层摘要（顶层条目 + 一级子项，
///   顶层 ≤12 项 / 每目录一级子项 ≤6 / 总行数 ≤40），MUST NOT 使用单层截断
///   30 条；目录读取失败 → `repository_structure_unavailable: <原因>`；
///   空仓 → `repository_structure_empty: <路径>`，MUST NOT 以伪内容冒充空仓。
pub(crate) fn summarize_repository_structure(path: &Path, logical: bool) -> String {
    if logical {
        return summarize_repository_structure_logical(path);
    }
    let mut entries = Vec::new();
    if let Ok(reader) = std::fs::read_dir(path) {
        for entry in reader.flatten() {
            if let Ok(metadata) = entry.metadata() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name == ".git" {
                    continue;
                }
                let kind = if metadata.is_dir() { "dir" } else { "file" };
                entries.push(format!("{kind}: {name}"));
            }
        }
    }
    entries.sort();
    entries.truncate(30);
    if entries.is_empty() {
        "(empty repository)".to_string()
    } else {
        entries.join("\n")
    }
}

/// 多仓目录摘要（分层 + 失败显式）。
///
/// 分层摘要复用逐仓实况（`member_reality`）的分层实现——顶层目录 ≤12 项、
/// 每个顶层目录一级子项 ≤6、总行数 ≤40；顶层文件条目一并列出（≤12 顶层项）。
/// 读取失败与空仓 MUST 显式，MUST NOT 静默降级为伪内容。
fn summarize_repository_structure_logical(path: &Path) -> String {
    let reader = match std::fs::read_dir(path) {
        Ok(reader) => reader,
        Err(error) => {
            return format!("{REPOSITORY_STRUCTURE_UNAVAILABLE_PREFIX} {error}");
        }
    };
    let mut top_files: Vec<String> = Vec::new();
    for entry in reader.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == ".git" {
            continue;
        }
        match entry.file_type() {
            Ok(file_type) if file_type.is_dir() => {}
            Ok(_) => top_files.push(name),
            Err(_) => {}
        }
    }
    top_files.sort();
    let directories = group_layered_directories(summarize_layered_directories(path));

    // 顶层条目（文件 + 目录）按名称排序，总数 ≤12。
    let mut top_level: Vec<(String, bool)> =
        Vec::with_capacity(top_files.len() + directories.len());
    for file in top_files {
        top_level.push((file, false));
    }
    for (directory, _) in &directories {
        top_level.push((directory.clone(), true));
    }
    top_level.sort();
    top_level.truncate(MEMBER_REALITY_MAX_TOP_DIRECTORIES);

    let mut lines: Vec<String> = Vec::new();
    for (name, is_directory) in top_level {
        if is_directory {
            lines.push(format!("{name}/"));
            if let Some((_, children)) = directories.iter().find(|(dir, _)| dir == &name) {
                lines.extend(children.iter().cloned());
            }
        } else {
            lines.push(name);
        }
    }
    lines.truncate(MEMBER_REALITY_MAX_LAYERED_LINES);

    if lines.is_empty() {
        format!("{REPOSITORY_STRUCTURE_EMPTY_PREFIX} {}", path.display())
    } else {
        lines.join("\n")
    }
}

/// 把逐仓实况的分层目录摘要（`dir/` + `dir/child/`）按顶层目录归组。
fn group_layered_directories(layered: Vec<String>) -> Vec<(String, Vec<String>)> {
    let mut grouped: Vec<(String, Vec<String>)> = Vec::new();
    for line in layered {
        let trimmed = line.trim_end_matches('/');
        match trimmed.split_once('/') {
            Some((parent, _child)) => {
                if let Some(entry) = grouped.iter_mut().find(|(dir, _)| dir == parent) {
                    entry.1.push(line);
                }
            }
            None => grouped.push((trimmed.to_string(), Vec::new())),
        }
    }
    grouped
}

/// 规范 Design Spec 结构判定复用 parser 契约作为单一事实来源：
/// 强制标题与别名直接取自 artifact_constraints.rs 的
/// `artifact_constraint_spec_for(&WorkspaceType::Design).required_headings`，
/// 标题归一化（容忍 "## 1. 设计范围" 等编号前缀）与匹配逻辑复用 parser 的
/// `normalize_workspace_heading_line` / `heading_matches_rule`。
/// 同步义务：此处不得另行复制标题/别名表；parser 契约变更时本判定自动跟随。
/// 判定口径仅限 parser 的 `required_headings` 存在性（非完整 artifact 完整性门禁），
/// 避免真实缺失被误判齐备。
fn has_canonical_design_spec_structure(markdown: &str) -> bool {
    let spec = artifact_constraint_spec_for(&WorkspaceType::Design);
    let headings: Vec<String> = markdown
        .lines()
        .filter_map(normalize_workspace_heading_line)
        .collect();
    spec.required_headings.iter().all(|rule| {
        headings
            .iter()
            .any(|heading| heading_matches_rule(heading, rule))
    })
}

pub fn extract_design_context_capabilities(markdown: &str) -> DesignContextCapabilities {
    // Aria 规范 Design Spec 使用 设计范围/设计决策/公共组件/API 契约/数据模型/风险/追踪关系
    // 等 parser 强制标题，永远不会命中下方启发式标题，会被误判为 5 项上下文全缺失；
    // 检测到规范结构时直接视为 5 项能力均具备。
    if has_canonical_design_spec_structure(markdown) {
        return DesignContextCapabilities {
            has_architecture: true,
            has_module_breakdown: true,
            has_tech_stack: true,
            has_test_strategy: true,
            has_key_paths: true,
        };
    }
    let normalized = markdown_headings(markdown).join("\n").to_lowercase();
    DesignContextCapabilities {
        has_architecture: contains_any(&normalized, &["架构概览", "系统架构", "architecture"]),
        has_module_breakdown: contains_any(
            &normalized,
            &["模块划分", "模块拆分", "modules", "module breakdown"],
        ),
        has_tech_stack: contains_any(
            &normalized,
            &["技术选型", "技术栈", "tech stack", "technology"],
        ),
        has_test_strategy: contains_any(
            &normalized,
            &["测试框架", "测试策略", "test strategy", "testing strategy"],
        ),
        has_key_paths: contains_any(
            &normalized,
            &[
                "关键目录结构",
                "关键路径",
                "key paths",
                "directory structure",
            ],
        ),
    }
}

pub(crate) fn merge_design_context_capabilities(
    design_context: &[String],
) -> DesignContextCapabilities {
    design_context.iter().fold(
        DesignContextCapabilities {
            has_architecture: false,
            has_module_breakdown: false,
            has_tech_stack: false,
            has_test_strategy: false,
            has_key_paths: false,
        },
        |mut merged, markdown| {
            let current = extract_design_context_capabilities(markdown);
            merged.has_architecture |= current.has_architecture;
            merged.has_module_breakdown |= current.has_module_breakdown;
            merged.has_tech_stack |= current.has_tech_stack;
            merged.has_test_strategy |= current.has_test_strategy;
            merged.has_key_paths |= current.has_key_paths;
            merged
        },
    )
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn markdown_headings(markdown: &str) -> Vec<String> {
    markdown
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with('#') {
                Some(trimmed.trim_matches('#').trim().to_string())
            } else {
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn logical_repository_structure_is_layered() {
        // REQ-PNL-17：多仓摘要 MUST 分层呈现（顶层条目 + 一级子项），
        // MUST NOT 使用单层截断 30 条。
        let dir = TempDir::new().expect("temp repo");
        std::fs::create_dir_all(dir.path().join("src/core")).expect("mkdir src/core");
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").expect("write Cargo.toml");

        let summary = summarize_repository_structure(dir.path(), true);

        assert!(
            summary.contains("src/"),
            "多仓摘要必须含顶层目录条目：\n{summary}"
        );
        assert!(
            summary.contains("src/core/"),
            "多仓摘要必须含一级子项：\n{summary}"
        );
        assert!(
            summary.contains("Cargo.toml"),
            "多仓摘要必须含顶层文件条目：\n{summary}"
        );
    }

    #[test]
    fn logical_repository_structure_reports_unavailable_on_read_dir_failure() {
        // 读取失败 MUST 显式报错，MUST NOT 以空仓伪内容冒充。
        let dir = TempDir::new().expect("temp repo");
        let missing = dir.path().join("missing-repository");

        let summary = summarize_repository_structure(&missing, true);

        assert!(
            summary.starts_with("repository_structure_unavailable:"),
            "读取失败必须显式报错：\n{summary}"
        );
        assert!(
            !summary.contains("(empty repository)"),
            "读取失败 MUST NOT 冒充空仓伪内容：\n{summary}"
        );
    }

    #[test]
    fn logical_empty_repository_reports_empty() {
        // 空仓 MUST 以显式路径呈现，MUST NOT 以伪内容冒充。
        let dir = TempDir::new().expect("temp repo");

        let summary = summarize_repository_structure(dir.path(), true);

        assert!(
            summary.starts_with("repository_structure_empty:"),
            "空仓必须显式呈现：\n{summary}"
        );
        assert!(
            summary.contains(&dir.path().display().to_string()),
            "空仓必须带路径：\n{summary}"
        );
        assert!(
            !summary.contains("(empty repository)"),
            "空仓 MUST NOT 使用伪内容：\n{summary}"
        );
    }

    #[test]
    fn single_repo_repository_structure_unchanged() {
        // 单仓摘要 MUST 与改造前逐字一致（快照钉死）。
        let dir = TempDir::new().expect("temp repo");
        std::fs::create_dir_all(dir.path().join(".git")).expect("mkdir .git");
        std::fs::create_dir_all(dir.path().join("src")).expect("mkdir src");
        std::fs::write(dir.path().join("README.md"), "readme\n").expect("write README.md");

        const SINGLE_REPO_SNAPSHOT: &str = "dir: src\nfile: README.md";
        assert_eq!(
            summarize_repository_structure(dir.path(), false),
            SINGLE_REPO_SNAPSHOT
        );

        let empty = TempDir::new().expect("temp repo");
        assert_eq!(
            summarize_repository_structure(empty.path(), false),
            "(empty repository)"
        );

        // 既有 30 条单层截断 MUST 保留。
        let many = TempDir::new().expect("temp repo");
        for index in 0..35 {
            std::fs::write(many.path().join(format!("f{index:02}.txt")), "x").expect("write file");
        }
        let truncated = summarize_repository_structure(many.path(), false);
        let lines: Vec<&str> = truncated.lines().collect();
        assert_eq!(lines.len(), 30, "单仓 MUST 保留 30 条截断：\n{truncated}");
        assert_eq!(lines.first(), Some(&"file: f00.txt"));
        assert_eq!(lines.last(), Some(&"file: f29.txt"));
    }

    /// 主记录缺失语义回归（REQ-DRG-04）：请求一个已被删除的 story_spec 时，既有
    /// 错误码 `story_spec_not_found` 与既有文案 MUST 逐字不变——MUST NOT 被悬空引用
    /// 治理的容错降级为「跳过并告警」或静默返回空。
    #[test]
    fn missing_story_spec_keeps_existing_error_code_and_wording() {
        use crate::product::models::{IssuePhase, IssueStatus};

        let temp = TempDir::new().expect("temp store");
        let lifecycle =
            LifecycleStore::new(crate::product::app_paths::ProductAppPaths::new(temp.path()));
        let issue = IssueRecord {
            id: "issue_0001".to_string(),
            project_id: "project_0001".to_string(),
            repo_id: None,
            base_branch: None,
            logical_codebase_id: crate::product::logical_codebase::legacy_logical_codebase_id(
                "project_0001",
            ),
            title: "Test Issue".to_string(),
            description: None,
            change_id: "change_0001".to_string(),
            phase: IssuePhase::Clarification,
            status: IssueStatus::Draft,
            active_binding_id: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let request = GenerateWorkItemsRequest {
            title: "test plan".to_string(),
            story_spec_ids: vec!["story_spec_gone".to_string()],
            design_spec_ids: Vec::new(),
            include_integration_tests: None,
            include_e2e_tests: None,
            force_frontend_backend_split: None,
            require_execution_plan_confirm: None,
            author_provider: None,
            reviewer_provider: None,
            review_rounds: None,
            superpowers_enabled: None,
            openspec_enabled: None,
            revision_feedback: None,
        };

        let error = collect_story_context(&lifecycle, &request, &issue)
            .expect_err("缺失 story spec MUST 仍报错，不得降级为跳过或空结果");

        assert_eq!(
            error.code, "story_spec_not_found",
            "错误码 MUST 逐字不变（REQ-DRG-04）"
        );
        assert_eq!(
            error.message, "story spec not found",
            "错误文案 MUST 逐字不变（REQ-DRG-04）"
        );
        assert_eq!(error.details, json!({}), "错误 details MUST 保持既有空对象");
    }
}
