//! REQ-WSC-18/22：Final Compile 的依赖环诊断（人话 + 结构指纹）。
//!
//! [`compile_support`](super::compile_support) 的 `apply_change_order_cross_repo_depends_on`
//! 检出环后，本模块负责把真实事实翻译成 owner/author 都能用的文本：
//! - 人话段：环 witness 的有向链、各 Work Item 所在仓在 `change_order` 的
//!   位次、逆了仓序的那条显式依赖、该怎么改；
//! - 结构指纹段：成员集合 + 边集合的规范化摘要（跨编译尝试稳定），供
//!   「同一依赖环重复出现」的提前停止判定使用（不靠文案/行号/运行 id）。
//!
//! 拆文件原因：large_file_guard 对 `compile_support.rs` 的登记上限锁定在
//! 2310 行（不得增长），本组纯函数因此独立成文。

use std::collections::{BTreeMap, BTreeSet};

use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::models::LifecycleWorkItemRecord;

/// REQ-WSC-18：依赖环失败的 cause 由**人话段**在前、机器 id 段在后组成，
/// 二者以 [`CYCLE_IDS_MARKER`] 分隔——门卡据此把「message（人话）」与
/// 「detail（完整原文/机器 id）」分开呈现，不必二次解析自由文本。
pub(super) fn dependency_cycle_error(
    work_items: &[LifecycleWorkItemRecord],
    change_order: &[LogicalRepositoryId],
    cycle_ids: &[String],
) -> String {
    let fingerprint = dependency_cycle_witness(work_items)
        .map(|hops| cycle_structure_fingerprint(work_items, &hops))
        .unwrap_or_else(|| fallback_cycle_fingerprint(cycle_ids));
    format!(
        "dependency_cycle: {}{}{}{}{}",
        cycle_reason(work_items, change_order),
        crate::product::workspace_engine::compile_failure_surface::CYCLE_FINGERPRINT_MARKER,
        fingerprint,
        crate::product::workspace_engine::compile_failure_surface::CYCLE_IDS_MARKER,
        cycle_ids.join(", ")
    )
}

/// REQ-WSC-22：依赖环的**结构**指纹——成员集合 + 边集合的规范化摘要。
///
/// 稳定性来源（owner 2026-10-31 要求「同一组环不得因措辞变化被当成新环」）：
/// - 只用**结构**（哪些成员、哪些有向边），不用文案、行号、标题；
/// - 成员用「该 Work Item 在编译序中的序号 + target 仓」表达——编译运行 id
///   每次尝试都会变（`work_item_<compile_id>_003`），进指纹会让同一环每次
///   都算新环，白耗预算；
/// - 集合排序后序列化，跨尝试可复现。
fn cycle_structure_fingerprint(
    work_items: &[LifecycleWorkItemRecord],
    hops: &[(&str, &str)],
) -> String {
    use sha2::{Digest, Sha256};
    let mut members: Vec<String> = hops
        .iter()
        .flat_map(|(provider, consumer)| [*provider, *consumer])
        .map(|id| cycle_node_key(work_items, id))
        .collect();
    members.sort();
    members.dedup();
    let mut edges: Vec<String> = hops
        .iter()
        .map(|(provider, consumer)| {
            format!(
                "{}>{}",
                cycle_node_key(work_items, provider),
                cycle_node_key(work_items, consumer)
            )
        })
        .collect();
    edges.sort();
    let payload = serde_json::json!({ "members": members, "edges": edges });
    hex::encode(Sha256::digest(payload.to_string().as_bytes()))
}

/// 无 witness 时的兜底指纹（环上残余节点 id 排序后摘要）：仍只取结构 id 的
/// 序号 + 仓，不含文案。
fn fallback_cycle_fingerprint(cycle_ids: &[String]) -> String {
    use sha2::{Digest, Sha256};
    let mut members = cycle_ids.to_vec();
    members.sort();
    members.dedup();
    hex::encode(Sha256::digest(members.join(",").as_bytes()))
}

/// 结构身份里的单点表达：编译序序号（`work_item_<compile_id>_003` 的 `003`）
/// + target 仓；都不参与编译运行身份。
fn cycle_node_key(work_items: &[LifecycleWorkItemRecord], id: &str) -> String {
    let ordinal = id.rsplit('_').next().unwrap_or(id);
    let repository = work_items
        .iter()
        .find(|item| item.id == id)
        .and_then(|item| item.target_repository_id)
        .map(|repository| repository.0.to_string())
        .unwrap_or_default();
    format!("{ordinal}@{repository}")
}

/// 环的人话描述：只写真实事实（环 witness 的有向链、各自所在仓的 change_order
/// 位次、逆仓序的那条显式依赖）。找不到 witness 时退化为通用可行动文案，
/// 不虚构环形状。
fn cycle_reason(
    work_items: &[LifecycleWorkItemRecord],
    change_order: &[LogicalRepositoryId],
) -> String {
    let arrow_note = "（箭头为产出流向：左边 Work Item 的产出被右边消费，即右边依赖左边）";
    let Some(hops) = dependency_cycle_witness(work_items) else {
        return format!(
            "Work Item 之间的 depends_on 构成依赖环{arrow_note}：环上每个 Work Item 都在等环上另一个先完成，谁都无法先执行。请打断环：删掉其中一条 depends_on，或改成不互相等待的先后顺序。"
        );
    };
    let mut nodes: Vec<&str> = vec![hops[0].0];
    nodes.extend(hops.iter().map(|(_, consumer)| *consumer));
    let chain = nodes
        .iter()
        .map(|id| format!("「{}」", work_item_display(work_items, id)))
        .collect::<Vec<_>>()
        .join(" → ");
    let repo_index = |id: &str| -> Option<usize> {
        let repository = work_items
            .iter()
            .find(|item| item.id == id)?
            .target_repository_id?;
        change_order
            .iter()
            .position(|candidate| *candidate == repository)
    };
    let offending = hops.iter().find_map(|(provider, consumer)| {
        let provider_index = repo_index(provider)?;
        let consumer_index = repo_index(consumer)?;
        (provider_index > consumer_index).then_some((
            *provider,
            *consumer,
            provider_index,
            consumer_index,
        ))
    });
    match offending {
        Some((provider, consumer, provider_index, consumer_index)) => format!(
            "{chain} 构成依赖环{arrow_note}。其中「{consumer}」把「{provider}」写成 provider，但「{provider}」所在仓是 change_order 第 {provider_order} 个仓、「{consumer}」所在仓是第 {consumer_order} 个仓——跨仓依赖只允许后仓 Work Item 消费前仓 Work Item 的产出，这条依赖逆了仓序，因此与仓序自动边合成环。请让「{consumer}」改由仓序在前的 Work Item 提供产出，或删掉这条 depends_on。",
            provider_order = provider_index + 1,
            consumer_order = consumer_index + 1,
        ),
        None => format!(
            "{chain} 构成依赖环{arrow_note}：环上每个 Work Item 都在等环上另一个先完成，谁都无法先执行。请打断环：删掉其中一条 depends_on，或改成不互相等待的先后顺序。"
        ),
    }
}

/// Work Item 的呈现名：标题优先（owner/author 都认标题），空标题回落 id。
fn work_item_display<'a>(work_items: &'a [LifecycleWorkItemRecord], id: &'a str) -> &'a str {
    work_items
        .iter()
        .find(|item| item.id == id)
        .map(|item| item.title.as_str())
        .filter(|title| !title.trim().is_empty())
        .unwrap_or(id)
}

/// 从 depends_on 构造 provider→consumer 邻接，复用 SG1 的确定性环 witness
/// 提取（固定 ID/邻接序，顺带回边闭合），只取首个环。
fn dependency_cycle_witness(work_items: &[LifecycleWorkItemRecord]) -> Option<Vec<(&str, &str)>> {
    let ids: BTreeSet<&str> = work_items.iter().map(|item| item.id.as_str()).collect();
    let mut adjacency: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for item in work_items {
        for dependency in &item.depends_on {
            if dependency != &item.id && ids.contains(dependency.as_str()) {
                adjacency
                    .entry(dependency.as_str())
                    .or_default()
                    .insert(item.id.as_str());
            }
        }
    }
    super::compile_support::find_cycle_witness(&ids, &adjacency)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::models::{
        WorkItemContextBudget, WorkItemExecutionPlanStatus, WorkItemKind, WorkItemPlanStatus,
        WorkItemStatus,
    };
    use uuid::Uuid;

    fn repo(seed: u8) -> LogicalRepositoryId {
        let mut bytes = [0u8; 16];
        bytes[0] = seed;
        LogicalRepositoryId(Uuid::from_bytes(bytes))
    }

    fn item(
        id: &str,
        title: &str,
        target: LogicalRepositoryId,
        depends_on: Vec<&str>,
    ) -> LifecycleWorkItemRecord {
        LifecycleWorkItemRecord {
            id: id.to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_legacy".to_string(),
            target_repository_id: Some(target),
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: title.to_string(),
            plan_status: WorkItemPlanStatus::Confirmed,
            execution_status: WorkItemStatus::Pending,
            worktree_path: None,
            work_item_set_id: None,
            source_work_item_plan_id: None,
            source_outline_id: None,
            source_draft_id: None,
            planned_implementation_context: None,
            kind: WorkItemKind::Backend,
            sequence_hint: None,
            depends_on: depends_on.into_iter().map(str::to_string).collect(),
            exclusive_write_scopes: Vec::new(),
            forbidden_write_scopes: Vec::new(),
            context_budget: WorkItemContextBudget::default(),
            verification_plan_ref: None,
            require_execution_plan_confirm: false,
            execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
            completion_commit: None,
            completion_diff_summary_ref: None,
            created_at: "2026-10-31T00:00:00Z".to_string(),
            updated_at: "2026-10-31T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn cycle_error_is_human_readable_and_keeps_machine_ids() {
        // REQ-WSC-18：人话段必须由真实事实构成——逆行仓序的那条显式依赖、
        // 两个 Work Item 的标题、各自仓在 change_order 的位次；机器 id 段与
        // 结构指纹段单独留在原文里（旧 blocker 语义不变）。
        let frontend = repo(0x11);
        let gateway = repo(0x12);
        let change_order = [gateway, frontend];
        // 现场形态：gateway 的 WI 显式依赖 frontend 的 WI（逆仓序），而 change_order
        // 的自动边是 frontend → gateway，两者合成环。本函数只拿到 depends_on，因此
        // 用例把自动边也显式化，才能产出真实 witness（生产路径由
        // `apply_change_order_cross_repo_depends_on` 先把自动边并入 depends_on）。
        let frontend_item = item(
            "work_item_compile_003",
            "frontend 行删除入口",
            frontend,
            vec!["work_item_compile_004"],
        );
        let gateway_item = item(
            "work_item_compile_004",
            "gateway 删除转发",
            gateway,
            vec!["work_item_compile_003"],
        );
        let work_items = [frontend_item, gateway_item];
        let error = dependency_cycle_error(
            &work_items,
            &change_order,
            &[
                "work_item_compile_003".to_string(),
                "work_item_compile_004".to_string(),
            ],
        );
        assert!(error.starts_with("dependency_cycle: "), "{error}");
        assert!(
            error.contains("frontend 行删除入口") && error.contains("gateway 删除转发"),
            "{error}"
        );
        assert!(
            error.contains("provider") && error.contains("逆了仓序"),
            "{error}"
        );
        assert!(
            error.contains("change_order 第"),
            "必须给出真实仓序位次：{error}"
        );
        let markers =
            crate::product::workspace_engine::compile_failure_surface::CYCLE_FINGERPRINT_MARKER;
        let fingerprint = error
            .split(markers)
            .nth(1)
            .expect("fingerprint segment")
            .lines()
            .next()
            .unwrap();
        assert_eq!(
            fingerprint.len(),
            64,
            "结构指纹必须是 sha256 hex：{fingerprint}"
        );
        assert!(
            error.contains(
                crate::product::workspace_engine::compile_failure_surface::CYCLE_IDS_MARKER
            ),
            "机器 id 段必须留在原文：{error}"
        );
    }

    #[test]
    fn cycle_fingerprint_is_structural_and_stable_across_compile_ids() {
        // REQ-WSC-22：编译运行 id 每次尝试都会变，进指纹会让同一环每次都算新环。
        let frontend = repo(0x21);
        let gateway = repo(0x22);
        let change_order = [gateway, frontend];
        let build = |compile_id: &str, title: &str| {
            let frontend_item = item(
                &format!("work_item_{compile_id}_003"),
                title,
                frontend,
                vec![&format!("work_item_{compile_id}_004")],
            );
            let gateway_item = item(
                &format!("work_item_{compile_id}_004"),
                "gateway 删除转发",
                gateway,
                vec![&format!("work_item_{compile_id}_003")],
            );
            let work_items = [frontend_item, gateway_item];
            let ids = vec![
                format!("work_item_{compile_id}_003"),
                format!("work_item_{compile_id}_004"),
            ];
            dependency_cycle_error(&work_items, &change_order, &ids)
        };
        let first = build("compile_aaa", "frontend 行删除入口");
        // 换编译运行 id，且标题被作者改了措辞——结构没变，指纹必须相同。
        let second = build("compile_bbb", "前端删除入口（改写版）");
        let marker =
            crate::product::workspace_engine::compile_failure_surface::CYCLE_FINGERPRINT_MARKER;
        let fp = |text: &str| {
            text.split(marker)
                .nth(1)
                .unwrap()
                .lines()
                .next()
                .unwrap()
                .to_string()
        };
        assert_eq!(fp(&first), fp(&second), "同一组环必须得到同一结构指纹");
        assert_ne!(first, second, "人话段本身可以随标题变化");
    }
}
