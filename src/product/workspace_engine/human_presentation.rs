use std::collections::{BTreeMap, BTreeSet};

use crate::product::json_store::ProductStoreError;
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::models::{
    HumanPresentationRevision, LifecycleWorkItemRecord, WorkItemPlanLineage, WorkItemStatus,
    WorkspaceType,
};
use crate::product::work_item_projection::{
    HumanPresentationBase, validate_human_presentation_revision,
};
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::web::workspace_ws_types::ArtifactPayload;

use super::{WorkspaceEngine, WorkspaceEngineError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HumanPresentationScope {
    Plan,
    WorkItem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveHumanPresentationRevision {
    pub source_projection_bundle_id: String,
    pub scope: HumanPresentationScope,
    pub supersedes: Option<String>,
    pub human_summary: String,
    pub why_split: Option<String>,
    pub dependency_explanation: Vec<String>,
    pub risk_explanation: Vec<String>,
    pub source_refs: Vec<String>,
}

pub fn save_human_presentation_revision(
    store: &WorkItemRevisionStore,
    plan: &WorkItemPlanLineage,
    mut revision: HumanPresentationRevision,
) -> Result<HumanPresentationRevision, WorkspaceEngineError> {
    revision.normative = false;
    revision.used_by_provider = false;
    match (
        revision.source_plan_projection_bundle_id.as_deref(),
        revision.source_work_item_projection_bundle_id.as_deref(),
    ) {
        (Some(bundle_id), None) => {
            let bundle = store.get_plan_projection_bundle(plan, bundle_id)?;
            validate_human_presentation_revision(
                HumanPresentationBase::Plan {
                    projection_bundle_id: &bundle.id,
                    projection: &bundle.human_group_projection,
                },
                &revision,
            )?;
        }
        (None, Some(bundle_id)) => {
            let bundle = store.get_work_item_projection_bundle(plan, bundle_id)?;
            validate_human_presentation_revision(
                HumanPresentationBase::WorkItem {
                    projection_bundle_id: &bundle.id,
                    projection: &bundle.human_projection,
                },
                &revision,
            )?;
        }
        _ => return Err(WorkspaceEngineError::InvalidHumanPresentationTarget),
    }
    Ok(store.put_human_presentation_revision_cas(plan, revision)?)
}

impl WorkspaceEngine {
    pub fn save_human_presentation_revision_command(
        &self,
        command: SaveHumanPresentationRevision,
    ) -> Result<HumanPresentationRevision, WorkspaceEngineError> {
        if self.session.workspace_type != WorkspaceType::WorkItemPlan {
            return Err(WorkspaceEngineError::InvalidHumanPresentationTarget);
        }
        let store = self.revision_store();
        let plan = store.get_plan_lineage(
            &self.session.project_id,
            &self.session.issue_id,
            &self.session.entity_id,
        )?;
        let (source_plan_projection_bundle_id, source_work_item_projection_bundle_id) =
            match command.scope {
                HumanPresentationScope::Plan => (Some(command.source_projection_bundle_id), None),
                HumanPresentationScope::WorkItem => {
                    (None, Some(command.source_projection_bundle_id))
                }
            };
        save_human_presentation_revision(
            &store,
            &plan,
            HumanPresentationRevision {
                id: String::new(),
                source_plan_projection_bundle_id,
                source_work_item_projection_bundle_id,
                supersedes: command.supersedes,
                human_summary: command.human_summary,
                why_split: command.why_split,
                dependency_explanation: command.dependency_explanation,
                risk_explanation: command.risk_explanation,
                source_refs: command.source_refs,
                normative: false,
                used_by_provider: false,
                created_at: chrono::Utc::now().to_rfc3339(),
            },
        )
    }

    pub(crate) fn latest_human_presentation_revisions(&self) -> Vec<HumanPresentationRevision> {
        if self.session.workspace_type != WorkspaceType::WorkItemPlan {
            return Vec::new();
        }
        let store = self.revision_store();
        let Ok(plan) = store.get_plan_lineage(
            &self.session.project_id,
            &self.session.issue_id,
            &self.session.entity_id,
        ) else {
            return Vec::new();
        };
        let mut bundle_ids = BTreeSet::new();
        for version in &self.artifact_versions {
            collect_projection_bundle_id(&version.payload, &mut bundle_ids);
        }
        if let Some(artifact) = self.session.artifact.as_ref() {
            collect_projection_bundle_id(artifact, &mut bundle_ids);
        }
        bundle_ids
            .into_iter()
            .filter_map(|bundle_id| {
                store
                    .get_latest_human_presentation_revision(&plan, &bundle_id)
                    .ok()
                    .flatten()
            })
            .collect()
    }
}

fn collect_projection_bundle_id(payload: &ArtifactPayload, bundle_ids: &mut BTreeSet<String>) {
    match payload {
        ArtifactPayload::WorkItemPlanProjection { projection } => {
            bundle_ids.insert(projection.id.clone());
        }
        ArtifactPayload::WorkItemProjection { projection } => {
            bundle_ids.insert(projection.id.clone());
        }
        _ => {}
    }
}

/// Issue 下 WorkItem 按 `target_repository_id` 分组的聚合视图（REQ-TGT-05）。
/// 单 item 字段不变，只改聚合展示层。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkItemRepositoryGroup {
    /// 逻辑代码库成员 ID；`None` 表示遗留/未指定仓库（`compatibility_projection = true`）。
    pub target_repository_id: Option<LogicalRepositoryId>,
    /// 仓库展示名（member alias；member_index 缺失时回落到该组首个 item 的物理投影名）。
    pub alias: String,
    /// 仓库级聚合状态：任一 Blocked→blocked、任一 Pending→pending、任一 Planning→planning、
    /// 任一 Coding→coding、全部 Completed→completed（空组以 pending 兜底）。
    pub status: String,
    /// 遗留兼容投影标记：`target_repository_id == None` 的组为 true。
    pub compatibility_projection: bool,
    pub items: Vec<LifecycleWorkItemRecord>,
}

/// 将 Issue 下 WorkItem 列表按 `target_repository_id` 分组（REQ-TGT-05）。
///
/// - 分组顺序：按 target 在 `items` 中首次出现顺序（对应 plan 内 ordinal 顺序），
///   `None`（未指定仓库）组恒置末。
/// - alias 由 `member_index`（LogicalRepositoryId → alias，来源 `RepositoryContextSet`）
///   解析；缺失时回落到该组首个 item 的 `repository_id`（Task 9 语义：该字段即 target 的
///   物理投影名）。
/// - 聚合状态规则见 [`WorkItemRepositoryGroup::status`]。
pub fn group_work_items_by_target(
    items: &[LifecycleWorkItemRecord],
    member_index: &BTreeMap<LogicalRepositoryId, String>,
) -> Result<Vec<WorkItemRepositoryGroup>, ProductStoreError> {
    let mut groups: Vec<WorkItemRepositoryGroup> = Vec::new();
    let mut unassigned: Vec<LifecycleWorkItemRecord> = Vec::new();

    for item in items {
        match item.target_repository_id {
            Some(target) => {
                if let Some(group) = groups
                    .iter_mut()
                    .find(|group| group.target_repository_id == Some(target))
                {
                    group.items.push(item.clone());
                } else {
                    let alias = member_index
                        .get(&target)
                        .cloned()
                        .unwrap_or_else(|| item.repository_id.clone());
                    groups.push(WorkItemRepositoryGroup {
                        target_repository_id: Some(target),
                        alias,
                        status: String::new(),
                        compatibility_projection: false,
                        items: vec![item.clone()],
                    });
                }
            }
            None => unassigned.push(item.clone()),
        }
    }

    if !unassigned.is_empty() {
        groups.push(WorkItemRepositoryGroup {
            target_repository_id: None,
            alias: "未指定仓库".to_string(),
            status: String::new(),
            compatibility_projection: true,
            items: unassigned,
        });
    }

    for group in &mut groups {
        group.status = aggregate_group_status(&group.items);
    }

    Ok(groups)
}

/// 仓库级聚合状态：任一 Blocked→blocked、任一 Pending→pending、任一 Planning→planning、
/// 任一 Coding→coding、全部 Completed→completed（空组以 pending 兜底）。
fn aggregate_group_status(items: &[LifecycleWorkItemRecord]) -> String {
    if items
        .iter()
        .any(|item| item.execution_status == WorkItemStatus::Blocked)
    {
        return "blocked".to_string();
    }
    if items
        .iter()
        .any(|item| item.execution_status == WorkItemStatus::Pending)
    {
        return "pending".to_string();
    }
    if items
        .iter()
        .any(|item| item.execution_status == WorkItemStatus::Planning)
    {
        return "planning".to_string();
    }
    if items
        .iter()
        .any(|item| item.execution_status == WorkItemStatus::Coding)
    {
        return "coding".to_string();
    }
    if items
        .iter()
        .all(|item| item.execution_status == WorkItemStatus::Completed)
    {
        return "completed".to_string();
    }
    "pending".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::logical_codebase::LogicalRepositoryId;
    use crate::product::models::{
        LifecycleWorkItemRecord, WorkItemExecutionPlanStatus, WorkItemKind, WorkItemPlanStatus,
        WorkItemStatus,
    };
    use std::collections::BTreeMap;
    use uuid::Uuid;

    /// 稳定 UUID：禁止运行时随机，保证测试可复现（version 7 + variant 10xx）。
    const fn stable_uuid(seed: u16) -> Uuid {
        let mut bytes = [0u8; 16];
        bytes[14] = (seed >> 8) as u8;
        bytes[15] = seed as u8;
        bytes[6] = 0x70;
        bytes[8] = 0x80;
        Uuid::from_bytes(bytes)
    }

    struct PresentationFixture {
        api_id: LogicalRepositoryId,
        web_id: LogicalRepositoryId,
    }

    impl PresentationFixture {
        fn member_index(&self) -> BTreeMap<LogicalRepositoryId, String> {
            let mut index = BTreeMap::new();
            index.insert(self.api_id, "api".to_string());
            index.insert(self.web_id, "web".to_string());
            index
        }

        /// 3 个 WorkItem：api 2 个、web 1 个。
        fn items_across_two_repositories(&self) -> Vec<LifecycleWorkItemRecord> {
            vec![
                self.work_item(
                    "work_item_0001",
                    Some(self.api_id),
                    "repository_api",
                    WorkItemStatus::Pending,
                ),
                self.work_item(
                    "work_item_0002",
                    Some(self.api_id),
                    "repository_api",
                    WorkItemStatus::Completed,
                ),
                self.work_item(
                    "work_item_0003",
                    Some(self.web_id),
                    "repository_web",
                    WorkItemStatus::Planning,
                ),
            ]
        }

        fn work_item(
            &self,
            id: &str,
            target_repository_id: Option<LogicalRepositoryId>,
            repository_id: &str,
            execution_status: WorkItemStatus,
        ) -> LifecycleWorkItemRecord {
            let now = "2026-08-10T00:00:00Z".to_string();
            LifecycleWorkItemRecord {
                id: id.to_string(),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                repository_id: repository_id.to_string(),
                target_repository_id,
                story_spec_ids: Vec::new(),
                design_spec_ids: Vec::new(),
                title: format!("Work Item {id}"),
                plan_status: WorkItemPlanStatus::Confirmed,
                execution_status,
                worktree_path: None,
                work_item_set_id: None,
                source_work_item_plan_id: None,
                source_outline_id: None,
                source_draft_id: None,
                planned_implementation_context: None,
                kind: WorkItemKind::Backend,
                sequence_hint: None,
                depends_on: Vec::new(),
                exclusive_write_scopes: Vec::new(),
                forbidden_write_scopes: Vec::new(),
                context_budget: crate::product::models::WorkItemContextBudget::default(),
                verification_plan_ref: None,
                require_execution_plan_confirm: false,
                execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
                completion_commit: None,
                completion_diff_summary_ref: None,
                created_at: now.clone(),
                updated_at: now,
            }
        }
    }

    fn presentation_fixture() -> PresentationFixture {
        PresentationFixture {
            api_id: LogicalRepositoryId(stable_uuid(0x1001)),
            web_id: LogicalRepositoryId(stable_uuid(0x1002)),
        }
    }

    #[test]
    fn work_items_are_grouped_by_target_repository_with_alias_and_status() {
        let fixture = presentation_fixture();
        let items = fixture.items_across_two_repositories();

        let groups = group_work_items_by_target(&items, &fixture.member_index()).unwrap();
        assert_eq!(groups.len(), 2);
        assert!(
            groups
                .iter()
                .any(|group| group.alias == "api" && group.items.len() == 2)
        );
        assert!(
            groups
                .iter()
                .any(|group| group.alias == "web" && group.items.len() == 1)
        );
        assert!(groups.iter().all(|group| !group.status.is_empty()));
    }

    #[test]
    fn unassigned_items_form_unspecified_group_with_compatibility_projection_last() {
        let fixture = presentation_fixture();
        let mut items = fixture.items_across_two_repositories();
        items.push(fixture.work_item(
            "work_item_0004",
            None,
            "repository_primary",
            WorkItemStatus::Coding,
        ));

        let groups = group_work_items_by_target(&items, &fixture.member_index()).unwrap();
        assert_eq!(groups.len(), 3);
        let unassigned = groups.last().expect("None 组应置末");
        assert_eq!(unassigned.alias, "未指定仓库");
        assert_eq!(unassigned.target_repository_id, None);
        assert!(unassigned.compatibility_projection);
        assert_eq!(unassigned.items.len(), 1);
        assert!(
            groups
                .iter()
                .take(groups.len() - 1)
                .all(|group| !group.compatibility_projection)
        );
    }

    #[test]
    fn group_status_aggregates_any_pending_and_all_completed() {
        let fixture = presentation_fixture();
        let items = vec![
            fixture.work_item(
                "work_item_0001",
                Some(fixture.api_id),
                "repository_api",
                WorkItemStatus::Completed,
            ),
            fixture.work_item(
                "work_item_0002",
                Some(fixture.api_id),
                "repository_api",
                WorkItemStatus::Completed,
            ),
            fixture.work_item(
                "work_item_0003",
                Some(fixture.web_id),
                "repository_web",
                WorkItemStatus::Pending,
            ),
        ];

        let groups = group_work_items_by_target(&items, &fixture.member_index()).unwrap();
        let api = groups
            .iter()
            .find(|group| group.alias == "api")
            .expect("api 组存在");
        let web = groups
            .iter()
            .find(|group| group.alias == "web")
            .expect("web 组存在");
        assert_eq!(api.status, "completed");
        assert_eq!(web.status, "pending");
    }

    #[test]
    fn missing_member_index_entry_falls_back_to_physical_projection_name() {
        let fixture = presentation_fixture();
        let items = vec![fixture.work_item(
            "work_item_0001",
            Some(fixture.web_id),
            "repository_web",
            WorkItemStatus::Pending,
        )];
        // member_index 只含 api，不含 web：web 目标应回落到物理投影名 repository_web。
        let mut sparse_index = BTreeMap::new();
        sparse_index.insert(fixture.api_id, "api".to_string());

        let groups = group_work_items_by_target(&items, &sparse_index).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].alias, "repository_web");
        assert!(!groups[0].compatibility_projection);
        assert_eq!(groups[0].items.len(), 1);
    }
}
