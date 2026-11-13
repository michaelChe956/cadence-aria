use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::json;

use crate::product::lifecycle_store::LifecycleStore;
use crate::product::logical_codebase::member_reality::{
    MEMBER_REALITY_MAX_LAYERED_LINES, MEMBER_REALITY_MAX_TOP_DIRECTORIES, detect_entrypoints,
    summarize_layered_directories,
};
use crate::product::models::{DesignContextCapabilities, IssueRecord, WorkspaceType};
use crate::product::work_item_plan_compiler::PlanMemberTargets;
use crate::product::work_item_split_engine::prompts::WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES;
use crate::product::workspace_engine::{
    artifact_constraint_spec_for, heading_matches_rule, normalize_workspace_heading_line,
};
use crate::web::error::{ApiError, ApiResult};
use crate::web::types::GenerateWorkItemsRequest;

/// 多仓目录摘要：读取失败的显式前缀（REQ-PLN-17）。
const REPOSITORY_STRUCTURE_UNAVAILABLE_PREFIX: &str = "repository_structure_unavailable:";

/// 多仓目录摘要：空仓的显式前缀（REQ-PLN-17）。
const REPOSITORY_STRUCTURE_EMPTY_PREFIX: &str = "repository_structure_empty:";

/// SC（单候选）首轮 `structure:` 段预算：与 `budget_markdown_context` 的
/// structure 配额同源（上下文面 32,000 的 1/10，现值 3,200）。同步义务：
/// 上游配额表达式调整时本值随之同值——单一事实来源，不复制字面量。
const SC_MEMBER_STRUCTURE_BUDGET_BYTES: usize = WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES / 10;

/// SC 成员感知 structure：成员子项轮转截断说明（计入段预算；身份头行
/// 永不截断，缺子项不冒充全量）。
const MEMBER_STRUCTURE_TRUNCATION_NOTE: &str =
    "（成员子项超限，已按成员仓轮转保底截断；各成员仓身份头完整保留，未列出子项仍属对应成员仓）";

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

/// 单候选（SC）专用：成员感知的 `structure:` 段装配（REQ-WSC-06「成员仓
/// 身份头在预算内全部在场」「成员事实超预算时显式失败而非假装在场」）。
///
/// 与旧 `summarize_repository_structure` 的关系：旧函数及其四个 legacy 调用
/// （split / outline / revision）逐字不变；本接口只服务 SC 首轮装配。
/// - `member_targets` 缺席或成员表为空 ⇒ 原样退回旧摘要（逐字节同旧路径，
///   上层 `budget_markdown_context` 照旧生效）。
/// - 成员在场 ⇒ 所有成员身份头（`member <物理仓ID>: root <前缀|.>`，与
///   `[confirmed_member_order]` 教学的 physical_repository 同汇）先全部保留；
///   12 项 / 40 行只约束可轮转子项。
/// - 整段按完整 UTF-8 行装配在 `SC_MEMBER_STRUCTURE_BUDGET_BYTES` 内（含
///   截断说明），上层字节截断对本输出恒为 no-op，不可能切半身份头。
/// - 身份头（含存在子项时的强制截断说明）自身超限 ⇒ 显式
///   `member_inventory_budget_exceeded` + 成员总数与所需字节：不假称全员
///   在场、不让作者轮失败、不新增模型调用。
/// - 前缀表与 `plan_member_checkout_prefixes` 同键（物理成员仓 ID → 聚合根
///   相对前缀；alias 不能当作已验证 checkout）；缺权威映射 ⇒ 既有错误面
///   显式失败，绝不猜目录。
pub(crate) fn summarize_repository_structure_sc_members(
    aggregate_root: &Path,
    logical: bool,
    member_targets: Option<&PlanMemberTargets>,
    checkout_prefixes: Option<&BTreeMap<String, String>>,
) -> String {
    let Some(targets) = member_targets else {
        return summarize_repository_structure(aggregate_root, logical);
    };
    // 本轮已确认成员：physical_by_logical 值域 + 无归属 WI 的回落仓，按物理
    // ID 去重；顺序确定性 = BTreeMap 迭代序（logical ID）+ 回落仓缀后。
    let mut member_ids: Vec<String> = Vec::new();
    for physical in targets.physical_by_logical.values() {
        if !member_ids.contains(physical) {
            member_ids.push(physical.clone());
        }
    }
    if let Some(fallback) = targets.fallback_physical_id.as_deref()
        && !member_ids.iter().any(|id| id == fallback)
    {
        member_ids.push(fallback.to_string());
    }
    if member_ids.is_empty() {
        return summarize_repository_structure(aggregate_root, logical);
    }
    // 同源前缀交集：缺权威映射 ⇒ 既有错误面显式失败（不猜目录、不静默降级、
    // 不输出部分成员视图）。
    let Some(prefixes) = checkout_prefixes else {
        return format!(
            "{REPOSITORY_STRUCTURE_UNAVAILABLE_PREFIX} member checkout prefix missing for physical repositories {member_ids:?}"
        );
    };
    let mut members: Vec<(&str, &str)> = Vec::with_capacity(member_ids.len());
    for id in &member_ids {
        let Some(prefix) = prefixes.get(id).map(String::as_str) else {
            return format!(
                "{REPOSITORY_STRUCTURE_UNAVAILABLE_PREFIX} member checkout prefix missing for physical repository {id}"
            );
        };
        members.push((id.as_str(), prefix));
    }

    let headers: Vec<String> = members
        .iter()
        .map(|(id, prefix)| {
            let root = if prefix.is_empty() { "." } else { *prefix };
            format!("member {id}: root {root}")
        })
        .collect();
    let headers_bytes: usize = headers.iter().map(|line| line.len() + 1).sum();
    if headers_bytes > SC_MEMBER_STRUCTURE_BUDGET_BYTES {
        return format!(
            "member_inventory_budget_exceeded: members={member_count} required={required_bytes}B budget={SC_MEMBER_STRUCTURE_BUDGET_BYTES}B",
            member_count = members.len(),
            required_bytes = headers_bytes,
        );
    }
    let details: Vec<Vec<String>> = members
        .iter()
        .map(|(_, prefix)| member_rotatable_children(aggregate_root, prefix))
        .collect();
    // 保底集合 = 全部身份头 + （存在任何子项时的）强制截断说明。
    let note_bytes = if details.iter().any(|items| !items.is_empty()) {
        MEMBER_STRUCTURE_TRUNCATION_NOTE.len() + 1
    } else {
        0
    };
    if headers_bytes + note_bytes > SC_MEMBER_STRUCTURE_BUDGET_BYTES {
        return format!(
            "member_inventory_budget_exceeded: members={member_count} required={required_bytes}B budget={SC_MEMBER_STRUCTURE_BUDGET_BYTES}B",
            member_count = members.len(),
            required_bytes = headers_bytes + note_bytes,
        );
    }

    // 轮转子项：每轮每成员仓各取 1 条，总量 ≤40 行（上限只约束子项）。
    let mut rotated: Vec<String> = Vec::new();
    let mut round = 0;
    while rotated.len() < MEMBER_REALITY_MAX_LAYERED_LINES {
        let mut advanced = false;
        for items in &details {
            if rotated.len() >= MEMBER_REALITY_MAX_LAYERED_LINES {
                break;
            }
            if let Some(line) = items.get(round) {
                rotated.push(line.clone());
                advanced = true;
            }
        }
        if !advanced {
            break;
        }
        round += 1;
    }
    // 按完整行装入剩余字节额度（预留截断说明位）——永不出半行。
    let total_items: usize = details.iter().map(Vec::len).sum();
    let mut used = headers_bytes;
    let mut kept: Vec<&str> = Vec::with_capacity(rotated.len());
    for line in &rotated {
        if used + line.len() + 1 + note_bytes > SC_MEMBER_STRUCTURE_BUDGET_BYTES {
            break;
        }
        used += line.len() + 1;
        kept.push(line.as_str());
    }
    let dropped = total_items.saturating_sub(kept.len());

    let mut lines: Vec<&str> = headers.iter().map(String::as_str).collect();
    lines.extend(kept);
    if dropped > 0 {
        lines.push(MEMBER_STRUCTURE_TRUNCATION_NOTE);
    }
    lines.join("\n")
}

/// 单成员仓的可轮转子项：一级目录（`/` 结尾，跳过 `.git`，字典序）+
/// `member_reality::detect_entrypoints` 探到的根下清单文件存在性（≤6 项），
/// 相对聚合根呈现，合计 ≤12 项（`MEMBER_REALITY_MAX_TOP_DIRECTORIES` 同源
/// 上限，只约束子项）。成员目录不可读 ⇒ 单行显式标记（身份头仍在场，不以
/// 伪内容冒充、不让非成员条目顶替）。事实边界：不采纳 package.json ⇒
/// npm test 之类启发式命令，不把构建入口文件表述为业务源入口或可执行命令。
fn member_rotatable_children(aggregate_root: &Path, prefix: &str) -> Vec<String> {
    let member_root = if prefix.is_empty() {
        aggregate_root.to_path_buf()
    } else {
        aggregate_root.join(prefix)
    };
    let path_prefix = if prefix.is_empty() {
        String::new()
    } else {
        format!("{prefix}/")
    };
    let Ok(entries) = std::fs::read_dir(&member_root) else {
        return vec![format!("  {path_prefix}(成员目录不可读)")];
    };
    let mut directories: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == ".git" {
            continue;
        }
        directories.push(name);
    }
    directories.sort();
    let mut items: Vec<String> = Vec::with_capacity(directories.len());
    for directory in directories {
        items.push(format!("  {path_prefix}{directory}/"));
    }
    for entry_file in detect_entrypoints(&member_root) {
        items.push(format!("  {path_prefix}{entry_file}"));
    }
    items.truncate(MEMBER_REALITY_MAX_TOP_DIRECTORIES);
    items
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

    // ---- SC 成员感知 structure 装配（REQ-WSC-06，repair-wig-fact-projections C1）----

    use crate::product::logical_codebase::LogicalRepositoryId;
    use uuid::Uuid;

    /// 构造本轮已确认成员表：logical UUID 序号 → 物理 ID `phys-{slug}`。
    fn member_targets_fixture(slugs: &[&str], fallback: Option<&str>) -> PlanMemberTargets {
        let mut physical_by_logical = BTreeMap::new();
        for (index, slug) in slugs.iter().enumerate() {
            physical_by_logical.insert(
                LogicalRepositoryId(
                    Uuid::parse_str(&format!("00000000-0000-4000-8000-{index:012}"))
                        .expect("parse uuid"),
                ),
                format!("phys-{slug}"),
            );
        }
        PlanMemberTargets {
            physical_by_logical,
            fallback_physical_id: fallback.map(str::to_string),
        }
    }

    /// 构造同源前缀表（物理 ID → 聚合根相对前缀；空串 = 根成员）。
    fn checkout_prefixes_fixture(mapping: &[(&str, &str)]) -> BTreeMap<String, String> {
        mapping
            .iter()
            .map(|(id, prefix)| (format!("phys-{id}"), prefix.to_string()))
            .collect()
    }

    #[test]
    fn sc_member_structure_keeps_late_member_repo_when_point_files_crowd() {
        // REQ-WSC-06：>12 个点文件挤占顶层时，字典序靠后的成员仓身份头必须
        // 在预算内全场；点文件不得以非成员身份冒充成员子项。
        let dir = TempDir::new().expect("temp repo");
        for index in 0..16 {
            std::fs::write(dir.path().join(format!("a{index:02}.txt")), "x")
                .expect("write point file");
        }
        std::fs::create_dir_all(dir.path().join("zzz-member").join("src")).expect("mkdir member");
        let targets = member_targets_fixture(&["late"], None);
        let prefixes = checkout_prefixes_fixture(&[("late", "zzz-member")]);

        let summary = summarize_repository_structure_sc_members(
            dir.path(),
            true,
            Some(&targets),
            Some(&prefixes),
        );

        assert!(
            summary.contains("member phys-late: root zzz-member"),
            "成员身份头必须在场：\n{summary}"
        );
        assert!(
            summary.contains("  zzz-member/src/"),
            "成员一级目录必须在场：\n{summary}"
        );
        assert!(
            !summary.contains("a00.txt") && !summary.contains("a15.txt"),
            "点文件不得冒充成员子项：\n{summary}"
        );
        assert!(
            summary.len() <= 3_200,
            "structure 段恒 ≤3,200B：\n{summary}"
        );
    }

    #[test]
    fn sc_member_structure_rotates_children_for_twelve_members() {
        // REQ-WSC-06：12 成员 × 6 子项超出 40 行上限 ⇒ 轮转保底：所有身份头
        // 在场、截断显式标记、末行完整（无半行）。
        let dir = TempDir::new().expect("temp repo");
        let slugs: Vec<String> = (0..12).map(|member| format!("m{member:02}")).collect();
        for slug in &slugs {
            for child in 0..6 {
                std::fs::create_dir_all(dir.path().join(slug).join(format!("c{child}")))
                    .expect("mkdir member child");
            }
        }
        let slug_refs: Vec<&str> = slugs.iter().map(String::as_str).collect();
        let targets = member_targets_fixture(&slug_refs, None);
        let prefixes = checkout_prefixes_fixture(
            &slugs
                .iter()
                .map(|slug| (slug.as_str(), slug.as_str()))
                .collect::<Vec<_>>(),
        );

        let summary = summarize_repository_structure_sc_members(
            dir.path(),
            true,
            Some(&targets),
            Some(&prefixes),
        );

        for slug in &slugs {
            assert!(
                summary.contains(&format!("member phys-{slug}: root {slug}")),
                "成员 {slug} 身份头必须在场：\n{summary}"
            );
        }
        assert!(
            summary.contains("  m11/c2/"),
            "轮转（非顺序）必须覆盖末位成员：\n{summary}"
        );
        assert!(
            !summary.contains("  m11/c5/"),
            "超限子项轮转截断：\n{summary}"
        );
        assert!(
            summary.ends_with(MEMBER_STRUCTURE_TRUNCATION_NOTE),
            "截断说明必须完整收尾：\n{summary}"
        );
        assert!(summary.len() <= 3_200);
    }

    #[test]
    fn sc_member_structure_reports_budget_exceeded_for_huge_identity_headers() {
        // REQ-WSC-06「成员事实超预算时显式失败而非假装在场」：身份头自身超
        // 3,200B ⇒ 显式报总数与所需字节，不假称全员在场、不让作者轮失败。
        let dir = TempDir::new().expect("temp repo");
        let slugs: Vec<String> = (0..40).map(|index| format!("heavy{index:02}")).collect();
        let slug_refs: Vec<&str> = slugs.iter().map(String::as_str).collect();
        let long_prefix = "成员仓相对前缀超长段".repeat(12);
        let targets = member_targets_fixture(&slug_refs, None);
        let prefixes = checkout_prefixes_fixture(
            &slugs
                .iter()
                .map(|slug| (slug.as_str(), long_prefix.as_str()))
                .collect::<Vec<_>>(),
        );

        let summary = summarize_repository_structure_sc_members(
            dir.path(),
            true,
            Some(&targets),
            Some(&prefixes),
        );

        assert!(
            summary.starts_with("member_inventory_budget_exceeded:"),
            "超限必须显式失败：\n{summary}"
        );
        assert!(
            summary.contains("members=40"),
            "必须报成员总数：\n{summary}"
        );
        assert!(
            summary.contains("required=") && summary.contains("budget=3200B"),
            "必须报所需字节与预算：\n{summary}"
        );
        assert!(
            !summary.contains("member phys-heavy00: root"),
            "不得假称全员在场：\n{summary}"
        );
        assert!(summary.len() <= 3_200);
    }

    #[test]
    fn sc_member_structure_renders_root_member_dot_and_nested_prefix() {
        // 根成员（空前缀）渲染为 `.`、子项无前缀段；嵌套前缀按聚合根相对
        // 路径渲染；fallback 与既有成员同仓时去重（单一身份头）。
        let dir = TempDir::new().expect("temp repo");
        std::fs::create_dir_all(dir.path().join("src")).expect("mkdir root src");
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").expect("write manifest");
        std::fs::create_dir_all(dir.path().join("busi").join("gateway").join("api"))
            .expect("mkdir nested member");
        // fallback_physical_id 与 physical_by_logical 值同域：完整物理仓 ID
        // （同 prompt_contract_* 的 `Some("repo_backend_0001")`），非 slug。
        let targets = member_targets_fixture(&["root", "gw"], Some("phys-root"));
        let prefixes = checkout_prefixes_fixture(&[("root", ""), ("gw", "busi/gateway")]);

        let summary = summarize_repository_structure_sc_members(
            dir.path(),
            true,
            Some(&targets),
            Some(&prefixes),
        );

        assert!(
            summary.contains("member phys-root: root ."),
            "根成员必须渲染为 `.`：\n{summary}"
        );
        assert_eq!(
            summary.matches("member phys-root").count(),
            1,
            "fallback 同仓必须去重：\n{summary}"
        );
        assert!(
            summary.contains("  src/") && summary.contains("  Cargo.toml"),
            "根成员子项无前缀段：\n{summary}"
        );
        assert!(
            summary.contains("member phys-gw: root busi/gateway")
                && summary.contains("  busi/gateway/api/"),
            "嵌套成员按聚合根相对路径渲染：\n{summary}"
        );
    }

    #[test]
    fn sc_member_structure_keeps_header_for_unreadable_member() {
        // 成员目录不可读：身份头仍在场、单行显式标记、非成员条目不得顶替。
        let dir = TempDir::new().expect("temp repo");
        std::fs::write(dir.path().join("root-file.txt"), "x").expect("write root file");
        std::fs::create_dir_all(dir.path().join("svc").join("src")).expect("mkdir member");
        let targets = member_targets_fixture(&["ok", "missing"], None);
        let prefixes = checkout_prefixes_fixture(&[("ok", "svc"), ("missing", "gone")]);

        let summary = summarize_repository_structure_sc_members(
            dir.path(),
            true,
            Some(&targets),
            Some(&prefixes),
        );

        assert!(
            summary.contains("member phys-missing: root gone"),
            "不可读成员身份头必须在场：\n{summary}"
        );
        assert!(
            summary.contains("  gone/(成员目录不可读)"),
            "不可读必须显式标记：\n{summary}"
        );
        assert!(
            summary.contains("member phys-ok: root svc") && summary.contains("  svc/src/"),
            "可读成员不受影响：\n{summary}"
        );
        assert!(
            !summary.contains("root-file.txt"),
            "非成员根文件不得顶替成员子项：\n{summary}"
        );
    }

    #[test]
    fn sc_member_structure_missing_prefix_mapping_fails_explicitly() {
        // 缺权威映射：既有错误面显式失败并点名成员，绝不猜目录；前缀表整体
        // 缺席同样显式失败。
        let dir = TempDir::new().expect("temp repo");
        let targets = member_targets_fixture(&["known", "unknown"], None);
        let prefixes = checkout_prefixes_fixture(&[("known", "svc")]);

        let summary = summarize_repository_structure_sc_members(
            dir.path(),
            true,
            Some(&targets),
            Some(&prefixes),
        );
        assert!(
            summary.starts_with(REPOSITORY_STRUCTURE_UNAVAILABLE_PREFIX)
                && summary.contains("member checkout prefix missing")
                && summary.contains("phys-unknown"),
            "缺映射必须显式失败并点名成员：\n{summary}"
        );
        assert!(
            !summary.contains("member phys-known: root"),
            "映射不完整 ⇒ 整段失败，不输出部分成员视图：\n{summary}"
        );

        let no_table =
            summarize_repository_structure_sc_members(dir.path(), true, Some(&targets), None);
        assert!(
            no_table.starts_with(REPOSITORY_STRUCTURE_UNAVAILABLE_PREFIX),
            "前缀表缺席同样显式失败：\n{no_table}"
        );
    }

    #[test]
    fn sc_member_structure_without_members_matches_legacy_summary_exactly() {
        // REQ-WSC-06「单仓摘要逐字不变」：成员集合缺席/为空 ⇒ 与旧摘要逐字节一致。
        let dir = TempDir::new().expect("temp repo");
        std::fs::create_dir_all(dir.path().join("src").join("core")).expect("mkdir src/core");
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n").expect("write manifest");
        std::fs::write(dir.path().join("README.md"), "readme\n").expect("write readme");

        for logical in [true, false] {
            assert_eq!(
                summarize_repository_structure_sc_members(dir.path(), logical, None, None),
                summarize_repository_structure(dir.path(), logical),
                "成员缺席 ⇒ 逐字节同旧摘要（logical={logical}）"
            );
        }
        let empty_targets = PlanMemberTargets::default();
        assert_eq!(
            summarize_repository_structure_sc_members(dir.path(), true, Some(&empty_targets), None,),
            summarize_repository_structure(dir.path(), true),
            "空成员表 ⇒ 逐字节同旧摘要"
        );
    }

    #[test]
    fn sc_member_structure_lists_entry_manifest_files_without_commands() {
        // 事实边界：只列 detect_entrypoints 的根下清单文件存在性；不采纳
        // package.json ⇒ npm test 之类启发式命令。
        let dir = TempDir::new().expect("temp repo");
        let member = dir.path().join("svc");
        std::fs::create_dir_all(member.join("src")).expect("mkdir member");
        std::fs::write(member.join("Cargo.toml"), "[package]\n").expect("write manifest");
        std::fs::write(member.join("package.json"), "{}\n").expect("write manifest");
        let targets = member_targets_fixture(&["svc"], None);
        let prefixes = checkout_prefixes_fixture(&[("svc", "svc")]);

        let summary = summarize_repository_structure_sc_members(
            dir.path(),
            true,
            Some(&targets),
            Some(&prefixes),
        );

        assert!(
            summary.contains("  svc/Cargo.toml") && summary.contains("  svc/package.json"),
            "入口清单文件存在性必须在场：\n{summary}"
        );
        assert!(
            !summary.contains("cargo test") && !summary.contains("npm test"),
            "不得采纳启发式命令：\n{summary}"
        );
    }

    #[test]
    fn sc_member_structure_keeps_whole_lines_for_multibyte_prefixes() {
        // 多字节长前缀 + 每成员 15 个子项（12 项上限 + 字节预算）⇒ 身份头均为
        // 完整行、总输出 ≤3,200B、每成员 ≤12 子项、截断说明完整收尾。
        let dir = TempDir::new().expect("temp repo");
        // 单段路径分量 ≤255B（Linux NAME_MAX），多段拼出 332B 多字节长前缀。
        let long_prefix = vec!["业务网关成员仓相对前缀段"; 9].join("/");
        let member_root = dir.path().join(&long_prefix);
        for child in 0..15 {
            std::fs::create_dir_all(member_root.join(format!("目录{child:02}")))
                .expect("mkdir member child");
        }
        let targets = member_targets_fixture(&["mb"], None);
        let prefixes = checkout_prefixes_fixture(&[("mb", &long_prefix)]);

        let summary = summarize_repository_structure_sc_members(
            dir.path(),
            true,
            Some(&targets),
            Some(&prefixes),
        );

        let header = format!("member phys-mb: root {long_prefix}");
        assert!(
            summary.lines().any(|line| line == header),
            "身份头必须是完整行（不被字节预算切半）：\n{summary}"
        );
        assert!(
            summary.contains(&format!("  {long_prefix}/目录00/")),
            "预算内子项在场：\n{summary}"
        );
        assert!(summary.len() <= 3_200, "总输出恒 ≤3,200B：\n{summary}");
        assert!(
            summary.lines().all(|line| line.starts_with("member ")
                || line.starts_with("  ")
                || line == MEMBER_STRUCTURE_TRUNCATION_NOTE),
            "所有行都是完整装配行：\n{summary}"
        );
        assert!(
            !summary.contains("目录12/") && !summary.contains("目录11/"),
            "每成员子项 ≤12 项且超字节子项整行省略：\n{summary}"
        );
        assert!(
            summary.ends_with(MEMBER_STRUCTURE_TRUNCATION_NOTE),
            "截断说明必须完整收尾：\n{summary}"
        );
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
