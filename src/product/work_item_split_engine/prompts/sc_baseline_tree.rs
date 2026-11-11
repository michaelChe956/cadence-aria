//! SC markdown author 的**基线树事实直供**面（D5 事前 / REQ-WSC-06
//! 「基线树事实直供」「基线树清单不得占用固定规范面预算」）。
//!
//! 本模块从 `prompts.rs` 拆出，独立承载：
//! - 上下文面的三段配额压缩（`budget_markdown_context` / `truncate_markdown_context`）；
//! - 基线树清单的渲染（按成员仓分组、成员仓优先、根文件归组、轮转保底截断）；
//! - 上下文面**同一配额**下的清单额度计算（固定开销先行、剩余额度给清单、
//!   额度不足按行截断并显式标记、额度为 0 ⇒ 整段缺席）。
//!
//! 事实来源唯一：`plan_preflight::plan_baseline_tree`（与校验器同源）；本模块
//! **不新增第二套基线口径**，也**不臆造路径**。

pub(crate) const WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES: usize = 32_000;

pub(crate) fn budget_markdown_context(
    story_context: &str,
    design_context: &str,
    repository_structure: &str,
) -> (String, String, String) {
    (
        truncate_markdown_context(
            story_context,
            WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES * 9 / 20,
        ),
        truncate_markdown_context(
            design_context,
            WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES * 9 / 20,
        ),
        truncate_markdown_context(
            repository_structure,
            WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES / 10,
        ),
    )
}

fn truncate_markdown_context(value: &str, budget: usize) -> String {
    if value.len() <= budget {
        return value.to_string();
    }
    let marker =
        "\n[上下文因 prompt 预算截断；请仅依据保留内容和 source_spec_ids 输出，不得猜测。]\n";
    let keep = budget.saturating_sub(marker.len());
    let mut end = keep.min(value.len());
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &value[..end], marker)
}

const BASELINE_TREE_TRUNCATION_MARKER: &str = "（基线树清单超出上下文面配额，已按成员仓轮转保底截断；未列出的路径仍属于基线树，引用前按同前缀就近确认）";

fn truncate_baseline_tree_section(section: &str, cap: usize) -> String {
    if section.len() <= cap {
        return section.to_string();
    }
    let marker_cost = BASELINE_TREE_TRUNCATION_MARKER.len() + 1;
    if cap < marker_cost {
        // 额度连标记都放不下 ⇒ 整段缺席（不越预算、不留半截）。
        return String::new();
    }
    let budget = cap - marker_cost;
    let mut kept = String::new();
    for line in section.split_inclusive('\n') {
        if kept.len() + line.len() > budget {
            break;
        }
        kept.push_str(line);
    }
    kept.push_str(BASELINE_TREE_TRUNCATION_MARKER);
    kept.push('\n');
    kept
}

pub(crate) fn render_baseline_tree_section(
    tree: &std::collections::BTreeSet<String>,
    member_dirs: &[String],
) -> String {
    const ROOT_FILES_GROUP: &str = "(根文件)";
    let cap = WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES / 4;

    let mut groups: Vec<(&str, Vec<&str>)> = member_dirs
        .iter()
        .map(|dir| (dir.as_str(), Vec::new()))
        .collect();
    // 未命中 member_dir 的路径按**路径自身首段**分组。理由：当前
    // `plan_baseline_tree` 返回的是**单个成员仓的成员内相对路径**（实测
    // 案例：busi 的 `.gitignore`/`pom.xml`/`src/main/java/…`），而
    // `member_dirs` 是聚合根目录名（`busi`/`frontend`/…）⇒ 两者不同域，
    // 硬套会产出误导性的归并组。按树自身首段分组对单仓基线树是自然
    // 表达；多仓基线树尚未成立（见 `design.md` 登记的多仓覆盖缺陷）。
    let mut extra_groups: Vec<(&str, Vec<&str>)> = Vec::new();
    for path in tree {
        let head = path.split('/').next().unwrap_or(path);
        let group = if head == path.as_str() {
            ROOT_FILES_GROUP
        } else {
            head
        };
        if let Some((_, paths)) = groups.iter_mut().find(|(dir, _)| *dir == group) {
            paths.push(path.as_str());
            continue;
        }
        match extra_groups.iter_mut().find(|(dir, _)| *dir == group) {
            Some((_, paths)) => paths.push(path.as_str()),
            None => extra_groups.push((group, vec![path.as_str()])),
        }
    }
    groups.extend(extra_groups);

    let line_bytes = |dir: &str, path: &str| dir.len() + ": ".len() + path.len() + 1;
    let full_bytes: usize = groups
        .iter()
        .flat_map(|(dir, paths)| paths.iter().map(move |path| line_bytes(dir, path)))
        .sum();
    let mut section = String::new();
    if full_bytes <= cap {
        for (dir, paths) in &groups {
            for path in paths {
                section.push_str(dir);
                section.push_str(": ");
                section.push_str(path);
                section.push('\n');
            }
        }
        return section;
    }

    // 成员仓轮转保底：每轮每组各取 1 条，直到用满「cap − 截断标记行」；
    // 放不下的行跳过（上限不可破），保证轮 0 尽量为每组保住至少 1 条。
    let mut remaining = cap.saturating_sub(BASELINE_TREE_TRUNCATION_MARKER.len() + 1);
    let max_rounds = groups
        .iter()
        .map(|(_, paths)| paths.len())
        .max()
        .unwrap_or(0);
    for round in 0..max_rounds {
        for (dir, paths) in &groups {
            let Some(path) = paths.get(round) else {
                continue;
            };
            let cost = line_bytes(dir, path);
            if cost > remaining {
                continue;
            }
            remaining -= cost;
            section.push_str(dir);
            section.push_str(": ");
            section.push_str(path);
            section.push('\n');
        }
    }
    section.push_str(BASELINE_TREE_TRUNCATION_MARKER);
    section.push('\n');
    section
}

pub(crate) fn author_baseline_tree_section(
    loaded: Result<Option<std::collections::BTreeSet<String>>, String>,
    member_dirs: &[String],
) -> Option<String> {
    match loaded {
        Ok(Some(tree)) if !tree.is_empty() => {
            Some(render_baseline_tree_section(&tree, member_dirs))
        }
        Ok(_) => None,
        Err(error) => {
            eprintln!("[aria] plan baseline tree unavailable for author prompt: {error}");
            None
        }
    }
}

/// D5 事前（REQ-WSC-06）：计算作者 prompt 中 `baseline:` 段的最终文本。
///
/// 清单**计入上下文面同一配额**（owner 2026-11-10 裁决：新增直供内容计入其
/// 所属面）：上下文面的**全部**字节都算在这 32,000 内 —— 三段压缩正文 + 清单
/// + `story:`/`design:`/`structure:`/`baseline:` 前缀 + `story_spec_ids:`/
///   `design_spec_ids:` 行。故先算固定开销，再把**剩余**额度给清单（上限
///   32,000/4）；额度不足则按行截断并显式标记；额度为 0 ⇒ 整段缺席 ⇒
///   上下文面恒 ≤ 32,000。
pub(crate) fn baseline_tree_section(
    story_context: &str,
    design_context: &str,
    repository_structure: &str,
    story_spec_ids: &[String],
    design_spec_ids: &[String],
    baseline_tree: Option<&str>,
) -> String {
    let context_overhead =
        "story:\ndesign:\nstructure:\nbaseline:\nstory_spec_ids:\ndesign_spec_ids:\n".len()
            + story_spec_ids.join(", ").len()
            + design_spec_ids.join(", ").len();
    let context_used =
        story_context.len() + design_context.len() + repository_structure.len() + context_overhead;
    let baseline_cap = (WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES / 4)
        .min(WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES.saturating_sub(context_used));
    baseline_tree
        .filter(|tree| !tree.is_empty())
        .and_then(|tree| {
            let section = truncate_baseline_tree_section(tree, baseline_cap);
            if section.is_empty() {
                None
            } else {
                Some(format!("baseline:{section}\n"))
            }
        })
        .unwrap_or_default()
}
