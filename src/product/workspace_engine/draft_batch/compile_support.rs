use super::*;
use crate::product::logical_codebase::repository_routing::RepositoryRouting;
use crate::product::logical_codebase::{
    IssueCodebaseSelectionStore, LogicalCodebaseFeature, LogicalCodebaseStore, LogicalRepositoryId,
    MemberStatus,
};
use crate::product::repository_store::RepositoryStore;

impl WorkspaceEngine {
    pub(crate) fn is_current_work_item_plan_batch_mode(&self) -> bool {
        let Ok(store) = self.work_item_plan_store() else {
            return false;
        };
        store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .ok()
            .flatten()
            .map(|index| {
                index.batches.iter().any(|batch| {
                    batch.generation_round_id == index.current_generation_round_id
                        && batch.mode == WorkItemGenerationMode::Batch
                })
            })
            .unwrap_or(false)
    }

    pub(crate) fn logical_work_item_plan_repository_targets(
        &self,
        lifecycle: &LifecycleStore,
        plan: &IssueWorkItemPlan,
    ) -> Result<Option<std::collections::BTreeMap<LogicalRepositoryId, String>>, String> {
        resolve_logical_work_item_plan_repository_targets(lifecycle, plan)
    }
}

/// 解析 issue 的 logical work item plan 仓库 target 映射（REQ-PLN-02 消费点）。
/// 独立为自由函数以便无 engine 的单元测试直接验证 target 有效性校验。
/// 成对判定复用 RepositoryRouting（Task 11），避免与 resume 入口漂移：
/// Legacy → Ok(None)；Logical → 既有 target map 构建逻辑；
/// FailClosed → Err 带稳定错误码（repository_routing_*，B3）供 HTTP 映射识别。
pub(crate) fn resolve_logical_work_item_plan_repository_targets(
    lifecycle: &LifecycleStore,
    plan: &IssueWorkItemPlan,
) -> Result<Option<std::collections::BTreeMap<LogicalRepositoryId, String>>, String> {
    let paths = lifecycle.app_paths();
    let logical_store = LogicalCodebaseStore::new(paths.clone());
    let selection_store = IssueCodebaseSelectionStore::new(paths.clone());
    let manifest = logical_store
        .load_manifest(&plan.project_id)
        .map_err(|error| format!("load logical codebase manifest failed: {error}"))?;
    let selection = selection_store
        .load(&plan.project_id, &plan.issue_id)
        .map_err(|error| format!("load issue codebase selection failed: {error}"))?;
    let (manifest, _selection) = match RepositoryRouting::classify(manifest, selection) {
        // (None, None)：无 manifest 且无 selection → 传统单仓，target map 为空。
        RepositoryRouting::Legacy { .. } => return Ok(None),
        // (Some, Some)：逻辑代码库 → 继续按既有 target map 构建逻辑。
        RepositoryRouting::Logical {
            manifest,
            selection,
        } => (manifest, *selection),
        // 其余不成对状态 → fail-closed，稳定错误码（B3），绝不静默回退物理仓库。
        RepositoryRouting::FailClosed { code, .. } => {
            return Err(format!(
                "{}: logical codebase manifest and issue selection must both exist",
                code.stable_code()
            ));
        }
    };
    // REQ-PLN-02：有效成员 = manifest.member_ids ∩ active_member_ids（manifest 外的
    // active member 不算逻辑代码库成员）；manifest 中非 active 成员（删除/停用）进入
    // 失效集合，selection 据此失效并阻断指向它们的新工作项。
    let active_set: std::collections::BTreeSet<LogicalRepositoryId> = logical_store
        .list_members(&plan.project_id)
        .map_err(|error| format!("list logical codebase members failed: {error}"))?
        .into_iter()
        .filter(|member| member.status == MemberStatus::Active)
        .map(|member| member.logical_repository_id)
        .collect();
    let effective_active_member_ids: Vec<LogicalRepositoryId> = manifest
        .member_ids
        .iter()
        .copied()
        .filter(|id| active_set.contains(id))
        .collect();
    let stale_manifest_members: Vec<LogicalRepositoryId> = manifest
        .member_ids
        .iter()
        .copied()
        .filter(|id| !active_set.contains(id))
        .collect();
    let resolution = selection_store
        .resolve_effective_members(
            &plan.project_id,
            &plan.issue_id,
            &effective_active_member_ids,
        )
        .map_err(|error| format!("resolve issue codebase selection failed: {error}"))?;
    // REQ-PLN-02：manifest 不一致（删除成员仍留在 manifest.member_ids）→ 显式标记
    // selection 失效（resolve_effective_members 仅在 selection 引用失效成员时自动标记）。
    if !stale_manifest_members.is_empty() && resolution.selection.invalidation.is_none() {
        selection_store
            .mark_invalidated(&plan.project_id, &plan.issue_id, "member_removed")
            .map_err(|error| {
                format!("mark issue codebase selection invalidated failed: {error}")
            })?;
    }
    // REQ-PLN-02：目标指向已删除/停用成员时 blocker。manifest 中非 active 成员直接
    // 判定为删除；selection 引用的失效成员若 on-disk 非 Active 也判为删除。
    let mut removed_targets = stale_manifest_members;
    let on_disk_removed: Vec<LogicalRepositoryId> = resolution
        .effective_member_ids
        .iter()
        .chain(resolution.invalid_member_ids.iter())
        .copied()
        .filter(|id| {
            logical_store
                .load_member(&plan.project_id, *id)
                .ok()
                .flatten()
                .is_some_and(|member| member.status != MemberStatus::Active)
        })
        .collect();
    for id in on_disk_removed {
        if !removed_targets.contains(&id) {
            removed_targets.push(id);
        }
    }
    if !removed_targets.is_empty() {
        return Err(format!(
            "work_item_target_missing: target_member_removed: {:?}",
            removed_targets
        ));
    }
    if !resolution.invalid_member_ids.is_empty() {
        return Err(format!(
            "work_item_target_missing: issue codebase selection has invalid members: {:?}",
            resolution.invalid_member_ids
        ));
    }

    let repository_store =
        RepositoryStore::with_logical_codebase_feature(paths, LogicalCodebaseFeature::enabled());
    resolution
        .effective_member_ids
        .into_iter()
        .map(|target_repository_id| {
            repository_store
                .resolve_logical_repository(&plan.project_id, target_repository_id)
                .map(|(_, _, repository)| (target_repository_id, repository.id))
                .map_err(|error| {
                    format!(
                        "work_item_target_missing: cannot resolve target repository `{target_repository_id:?}`: {error}"
                    )
                })
        })
        .collect::<Result<_, _>>()
        .map(Some)
}

/// 加载 issue 的 confirmed Design 的 change_order（仓级执行顺序，REQ-TGT-04 消费点）。
/// compile 入口经 lifecycle + plan.source_design_spec_ids 加载 confirmed design，取
/// DesignSpecRecord.change_order。单仓/无 aggregate scope（无 confirmed design）→ 返回空
/// （不映射），compile 行为不变（红线）。
pub(crate) fn load_change_order_from_confirmed_design(
    lifecycle: &LifecycleStore,
    plan: &IssueWorkItemPlan,
) -> Result<Vec<LogicalRepositoryId>, String> {
    let designs = lifecycle
        .list_design_specs(&plan.project_id, &plan.issue_id)
        .map_err(|error| format!("list design specs failed: {error}"))?;
    // 只取 plan.source_design_spec_ids 指向且已 Confirmed 的 design（Task 6 confirm gate 产物）。
    let confirmed = designs.iter().find(|design| {
        plan.source_design_spec_ids.contains(&design.id)
            && design.confirmation_status == LifecycleConfirmationStatus::Confirmed
    });
    Ok(confirmed
        .map(|design| design.change_order.clone())
        .unwrap_or_default())
}

/// 把 Design 的 change_order（仓级执行顺序）映射成跨仓 WorkItem depends_on（REQ-TGT-04）。
/// 规则：
/// ① 两个 WorkItem target 仓 tA≠tB 且 change_order 中 tA 在 tB 之前 → B depends_on A；
/// ② 同 target 仓的多 item 不因 change_order 建跨仓依赖（同仓顺序由既有 WorkItem 级
///    depends_on/sequence_hint 决定）；
/// ③ 合并既有 WorkItem 级 depends_on（candidate input_contracts 提取）；
/// ④ 环检测 → blocker dependency_cycle。
fn apply_change_order_cross_repo_depends_on(
    work_items: &mut [LifecycleWorkItemRecord],
    change_order: &[LogicalRepositoryId],
) -> Result<(), String> {
    if change_order.is_empty() {
        // 单仓/无 aggregate scope → 空，不映射（红线：单仓 compile 行为不变）。
        return Ok(());
    }
    // 按 target 仓聚合 work item id（仅 Logical 分支有 target_repository_id）。
    let mut by_target: BTreeMap<LogicalRepositoryId, Vec<String>> = BTreeMap::new();
    for item in work_items.iter() {
        if let Some(target) = item.target_repository_id {
            by_target.entry(target).or_default().push(item.id.clone());
        }
    }
    // 对每对 (earlier_repo, later_repo)：所有 target=later_repo 的 item depends_on 所有
    // target=earlier_repo 的 item（规则①②——change_order 内索引 i<j 保证 tA≠tB 且 tA 在前）。
    for (index, earlier_repo) in change_order.iter().enumerate() {
        for later_repo in change_order.iter().skip(index + 1) {
            let earlier_items = by_target.get(earlier_repo);
            let later_items = by_target.get(later_repo);
            if let (Some(earlier_items), Some(later_items)) = (earlier_items, later_items) {
                for later_item in later_items {
                    for earlier_item in earlier_items {
                        let item = work_items
                            .iter_mut()
                            .find(|it| &it.id == later_item)
                            .expect("later item id must exist in work_items");
                        // 规则③：合并既有 WorkItem 级 depends_on，去重。
                        if !item.depends_on.contains(earlier_item) {
                            item.depends_on.push(earlier_item.clone());
                        }
                    }
                }
            }
        }
    }
    // 规则④：环检测 → blocker dependency_cycle。
    if let Some(cycle_ids) = detect_dependency_cycle(work_items) {
        return Err(format!(
            "dependency_cycle: change_order 与既有 WorkItem 依赖构成环: {}",
            cycle_ids.join(", ")
        ));
    }
    Ok(())
}

/// 环检测（Kahn）：对 work_items 的 depends_on（限定在本批 item id 内的边）做拓扑排序，
/// 有环则返回环内 work item id 列表，无环返回 None。
fn detect_dependency_cycle(work_items: &[LifecycleWorkItemRecord]) -> Option<Vec<String>> {
    let ids: HashSet<&str> = work_items.iter().map(|item| item.id.as_str()).collect();
    let mut indegree: HashMap<&str, usize> = work_items
        .iter()
        .map(|item| (item.id.as_str(), 0usize))
        .collect();
    let mut adjacency: HashMap<&str, Vec<&str>> = HashMap::new();
    for item in work_items {
        for dependency_id in &item.depends_on {
            if ids.contains(dependency_id.as_str()) && dependency_id != &item.id {
                adjacency
                    .entry(dependency_id.as_str())
                    .or_default()
                    .push(item.id.as_str());
                *indegree.entry(item.id.as_str()).or_default() += 1;
            }
        }
    }
    let mut queue: VecDeque<&str> = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(id, _)| *id)
        .collect();
    let mut visited = 0usize;
    while let Some(id) = queue.pop_front() {
        visited += 1;
        if let Some(next_ids) = adjacency.get(id) {
            for &next_id in next_ids {
                let degree = indegree.get_mut(next_id).expect("next id in indegree");
                *degree -= 1;
                if *degree == 0 {
                    queue.push_back(next_id);
                }
            }
        }
    }
    if visited == ids.len() {
        None
    } else {
        Some(
            indegree
                .iter()
                .filter(|(_, degree)| **degree > 0)
                .map(|(id, _)| (*id).to_string())
                .collect(),
        )
    }
}

impl WorkspaceEngine {
    pub(crate) fn work_item_plan_repository_id(
        &self,
        lifecycle: &LifecycleStore,
        plan: &IssueWorkItemPlan,
    ) -> Result<String, String> {
        let story_specs = lifecycle
            .list_story_specs(&plan.project_id, &plan.issue_id)
            .map_err(|error| format!("list story specs failed: {error}"))?;
        for story_id in &plan.source_story_spec_ids {
            if let Some(story) = story_specs.iter().find(|story| &story.id == story_id) {
                return Ok(story.repository_id.clone());
            }
        }
        Err("cannot resolve repository_id for WorkItemPlan compile".to_string())
    }

    pub(crate) fn project_work_item_plan_drafts_for_compile(
        &self,
        previous_plan: &IssueWorkItemPlan,
        draft_records: &[WorkItemDraftRecord],
        context: WorkItemPlanCompileProjectionContext<'_>,
        change_order: &[LogicalRepositoryId],
    ) -> Result<
        (
            IssueWorkItemPlan,
            Vec<LifecycleWorkItemRecord>,
            Vec<VerificationPlan>,
        ),
        String,
    > {
        let outline_order = context.outline_order;
        let outline_to_work_item_id = context.outline_to_work_item_id;
        let outline_to_verification_plan_id = context.outline_to_verification_plan_id;
        let logical_targets = context.logical_targets;
        let now = context.now;
        let draft_by_outline: HashMap<&str, &WorkItemDraftRecord> = draft_records
            .iter()
            .map(|record| (record.outline_id.as_str(), record))
            .collect();
        let mut logical_to_outline_id = HashMap::new();
        for record in draft_records {
            if logical_to_outline_id
                .insert(
                    record.candidate.logical_work_item_id.as_str(),
                    record.outline_id.as_str(),
                )
                .is_some()
            {
                return Err(format!(
                    "duplicate logical work item identity `{}` during compile",
                    record.candidate.logical_work_item_id
                ));
            }
        }
        let mut work_items = Vec::with_capacity(outline_order.len());
        let mut verification_plans = Vec::with_capacity(outline_order.len());
        if let Some(logical_targets) = logical_targets {
            for outline_id in outline_order {
                let record = draft_by_outline
                    .get(outline_id.as_str())
                    .ok_or_else(|| format!("accepted draft for outline `{outline_id}` missing"))?;
                let target_repository_id = record.candidate.target_repository_id.ok_or_else(|| {
                    format!(
                        "work_item_target_missing: target_repository_id_missing for outline `{outline_id}`"
                    )
                })?;
                if !logical_targets.contains_key(&target_repository_id) {
                    return Err(format!(
                        "work_item_target_missing: target_repository_id_not_effective for outline `{outline_id}`"
                    ));
                }
            }
        }
        for (index, outline_id) in outline_order.iter().enumerate() {
            let record = draft_by_outline
                .get(outline_id.as_str())
                .ok_or_else(|| format!("accepted draft for outline `{outline_id}` missing"))?;
            let candidate = &record.candidate;
            let work_item_id = outline_to_work_item_id
                .get(outline_id)
                .cloned()
                .ok_or_else(|| format!("work item id for outline `{outline_id}` missing"))?;
            let verification_plan_id = outline_to_verification_plan_id
                .get(outline_id)
                .cloned()
                .ok_or_else(|| {
                    format!("verification plan id for outline `{outline_id}` missing")
                })?;
            let (repository_id, target_repository_id) = match logical_targets {
                Some(logical_targets) => {
                    let target_repository_id = candidate.target_repository_id.expect(
                        "logical target prevalidation must ensure target_repository_id is present",
                    );
                    let repository_id = logical_targets.get(&target_repository_id).expect(
                        "logical target prevalidation must ensure target_repository_id is effective",
                    );
                    (repository_id.as_str(), Some(target_repository_id))
                }
                None => (context.repository_id, None),
            };
            let depends_on = candidate
                .canonical_contract_candidate
                .input_contracts
                .iter()
                .map(|input| {
                    let dependency_outline_id = logical_to_outline_id
                        .get(input.provider_logical_work_item_id.as_str())
                        .ok_or_else(|| {
                            format!(
                                "provider logical identity `{}` for `{outline_id}` missing",
                                input.provider_logical_work_item_id
                            )
                        })?;
                    outline_to_work_item_id
                        .get(*dependency_outline_id)
                        .cloned()
                        .ok_or_else(|| {
                            format!(
                                "dependency outline `{dependency_outline_id}` for `{outline_id}` missing"
                            )
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            work_items.push(LifecycleWorkItemRecord {
                id: work_item_id.clone(),
                project_id: previous_plan.project_id.clone(),
                issue_id: previous_plan.issue_id.clone(),
                repository_id: repository_id.to_string(),
                target_repository_id,
                story_spec_ids: previous_plan.source_story_spec_ids.clone(),
                design_spec_ids: previous_plan.source_design_spec_ids.clone(),
                title: candidate
                    .canonical_contract_candidate
                    .identity
                    .title
                    .clone(),
                plan_status: WorkItemPlanStatus::Confirmed,
                execution_status: crate::product::models::WorkItemStatus::Pending,
                worktree_path: None,
                work_item_set_id: Some(previous_plan.id.clone()),
                source_work_item_plan_id: Some(previous_plan.id.clone()),
                source_outline_id: Some(record.outline_id.clone()),
                source_draft_id: Some(record.draft_id.clone()),
                planned_implementation_context: None,
                kind: crate::product::work_item_split_engine::types::parse_work_item_kind(
                    &candidate.canonical_contract_candidate.identity.kind,
                ),
                sequence_hint: Some((index + 1) as u32),
                depends_on,
                exclusive_write_scopes: candidate
                    .canonical_contract_candidate
                    .write_policy
                    .exclusive_scopes
                    .clone(),
                forbidden_write_scopes: candidate
                    .canonical_contract_candidate
                    .write_policy
                    .forbidden_scopes
                    .clone(),
                context_budget: crate::product::models::WorkItemContextBudget::default(),
                verification_plan_ref: Some(verification_plan_id.clone()),
                require_execution_plan_confirm: previous_plan
                    .options
                    .require_execution_plan_confirm,
                execution_plan_status:
                    crate::product::models::WorkItemExecutionPlanStatus::NotStarted,
                completion_commit: None,
                completion_diff_summary_ref: None,
                created_at: now.to_string(),
                updated_at: now.to_string(),
            });
            verification_plans.push(parse_compile_verification_plan(
                &candidate.verification_plan,
                verification_plan_id,
                previous_plan.project_id.clone(),
                previous_plan.issue_id.clone(),
                work_item_id,
                now.to_string(),
            ));
        }
        // Task 8：把 Design 的 change_order（仓级执行顺序）映射成跨仓 WorkItem depends_on。
        // 单仓/无 aggregate scope → change_order 为空 → 不映射，compile 行为不变（红线）。
        apply_change_order_cross_repo_depends_on(&mut work_items, change_order)?;
        let work_item_ids: Vec<String> = outline_order
            .iter()
            .filter_map(|outline_id| outline_to_work_item_id.get(outline_id).cloned())
            .collect();
        let verification_plan_ids: Vec<String> = outline_order
            .iter()
            .filter_map(|outline_id| outline_to_verification_plan_id.get(outline_id).cloned())
            .collect();
        let dependency_graph = work_items
            .iter()
            .flat_map(|work_item| {
                work_item
                    .depends_on
                    .iter()
                    .map(|dependency_id| IssueWorkItemDependencyEdge {
                        from_work_item_id: dependency_id.clone(),
                        to_work_item_id: work_item.id.clone(),
                    })
            })
            .collect();
        let mut compiled_plan = previous_plan.clone();
        compiled_plan.status = crate::product::models::IssueWorkItemPlanStatus::Confirmed;
        compiled_plan.work_item_ids = work_item_ids;
        compiled_plan.verification_plan_ids = verification_plan_ids;
        compiled_plan.repository_profile_ref = None;
        compiled_plan.dependency_graph = dependency_graph;
        compiled_plan.validator_findings = Vec::new();
        compiled_plan.updated_at = now.to_string();
        Ok((compiled_plan, work_items, verification_plans))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::repository_routing::RepositoryRoutingErrorCode;
    use crate::product::logical_codebase::{
        CodebaseMemberRecord, IssueCodebaseSelection, LogicalCodebaseManifest, MemberStatus,
        RepositoryCheckoutId, RepositorySourceIdentity, RepositoryType,
    };
    use crate::product::models::{IssueWorkItemPlanOptions, IssueWorkItemPlanStatus};
    use tempfile::TempDir;
    use uuid::Uuid;

    /// 稳定 UUID：禁止运行时随机，保证测试可复现。
    const fn stable_uuid(seed: u16) -> Uuid {
        let mut bytes = [0u8; 16];
        bytes[14] = (seed >> 8) as u8;
        bytes[15] = seed as u8;
        // version 7 + variant 10xx，满足 Uuid::from_bytes 的合法构造。
        bytes[6] = 0x70;
        bytes[8] = 0x80;
        Uuid::from_bytes(bytes)
    }

    struct CompileInvalidationFixture {
        _temp: TempDir,
        lifecycle: LifecycleStore,
    }

    impl CompileInvalidationFixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let lifecycle = LifecycleStore::new(ProductAppPaths::new(temp.path()));
            Self {
                _temp: temp,
                lifecycle,
            }
        }

        fn aggregate_root(&self) -> std::path::PathBuf {
            self._temp.path().join("aggregate-root")
        }

        /// 写入含单个 active 成员的 manifest + selection（include 该成员）。
        fn write_selection_with_member(&self, member_id: LogicalRepositoryId, alias: &str) {
            let store = LogicalCodebaseStore::new(self.lifecycle.app_paths());
            let manifest = LogicalCodebaseManifest::new(
                "project_0001",
                self.aggregate_root(),
                vec![member_id],
            );
            store.save_manifest("project_0001", &manifest).unwrap();
            store
                .save_member(
                    "project_0001",
                    &self.member_record(member_id, alias, MemberStatus::Active),
                )
                .unwrap();
            let selection = IssueCodebaseSelection::explicit(
                "project_0001",
                "issue_0001",
                vec![member_id],
                Vec::new(),
                Vec::new(),
                None,
            );
            IssueCodebaseSelectionStore::new(self.lifecycle.app_paths())
                .save(&selection)
                .unwrap();
        }

        /// 删除成员（tombstone 语义）：从 manifest.member_ids 移除并推进 revision，
        /// member record 保留但 status=Removed。
        fn delete_member(&self, member_id: LogicalRepositoryId, alias: &str) {
            let store = LogicalCodebaseStore::new(self.lifecycle.app_paths());
            let mut manifest = store.load_manifest("project_0001").unwrap().unwrap();
            manifest.member_ids.retain(|id| *id != member_id);
            manifest.membership_revision += 1;
            store.save_manifest("project_0001", &manifest).unwrap();
            store
                .save_member(
                    "project_0001",
                    &self.member_record(member_id, alias, MemberStatus::Removed),
                )
                .unwrap();
        }

        /// 写入含多个 active 成员的 manifest + member records（不写 selection）。
        fn write_manifest_with_members(
            &self,
            member_ids: &[LogicalRepositoryId],
            aliases: &[&str],
        ) {
            let store = LogicalCodebaseStore::new(self.lifecycle.app_paths());
            let manifest = LogicalCodebaseManifest::new(
                "project_0001",
                self.aggregate_root(),
                member_ids.to_vec(),
            );
            store.save_manifest("project_0001", &manifest).unwrap();
            for (id, alias) in member_ids.iter().zip(aliases) {
                store
                    .save_member(
                        "project_0001",
                        &self.member_record(*id, alias, MemberStatus::Active),
                    )
                    .unwrap();
            }
        }

        /// 写入含多个 active 成员的 manifest + selection（include 全部成员）。
        fn write_selection_with_members(
            &self,
            member_ids: &[LogicalRepositoryId],
            aliases: &[&str],
        ) {
            self.write_manifest_with_members(member_ids, aliases);
            let selection = IssueCodebaseSelection::explicit(
                "project_0001",
                "issue_0001",
                member_ids.to_vec(),
                Vec::new(),
                Vec::new(),
                None,
            );
            IssueCodebaseSelectionStore::new(self.lifecycle.app_paths())
                .save(&selection)
                .unwrap();
        }

        /// 真实 tombstone 语义（Plan 1 `apply_delete_tombstone`）：manifest.member_ids
        /// 不变，仅 member.status → Tombstoned。
        fn tombstone_member(&self, member_id: LogicalRepositoryId, alias: &str) {
            let store = LogicalCodebaseStore::new(self.lifecycle.app_paths());
            store
                .save_member(
                    "project_0001",
                    &self.member_record(member_id, alias, MemberStatus::Tombstoned),
                )
                .unwrap();
        }

        fn member_record(
            &self,
            id: LogicalRepositoryId,
            alias: &str,
            status: MemberStatus,
        ) -> CodebaseMemberRecord {
            let now = "2026-08-10T00:00:00Z".to_string();
            let checkout_path = self.aggregate_root().join(alias);
            CodebaseMemberRecord {
                logical_repository_id: id,
                physical_repository_id: format!("repository_{alias}"),
                alias: alias.to_string(),
                role: "service".to_string(),
                ordinal: 1,
                source_identity: RepositorySourceIdentity::from_git_parts(
                    &checkout_path,
                    checkout_path.join(".git"),
                    Some(format!("ssh://git@example.test/acme/{alias}.git")),
                ),
                repo_type: RepositoryType::Backend,
                tech_stack: vec!["rust".to_string()],
                owner: None,
                tags: Vec::new(),
                default_ref: None,
                checkout_ids: vec![RepositoryCheckoutId(Uuid::nil())],
                status,
                created_at: now.clone(),
                updated_at: now,
            }
        }

        fn plan(&self) -> IssueWorkItemPlan {
            IssueWorkItemPlan {
                id: "issue_work_item_plan_0001".to_string(),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                source_story_spec_ids: Vec::new(),
                source_design_spec_ids: Vec::new(),
                options: IssueWorkItemPlanOptions {
                    include_integration_tests: false,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                status: IssueWorkItemPlanStatus::Draft,
                work_item_ids: Vec::new(),
                repository_profile_ref: None,
                verification_plan_ids: Vec::new(),
                dependency_graph: Vec::new(),
                created_from_provider_run: None,
                validator_findings: Vec::new(),
                review_summary: None,
                created_at: "2026-08-10T00:00:00Z".to_string(),
                updated_at: "2026-08-10T00:00:00Z".to_string(),
            }
        }
    }

    #[test]
    fn compile_blocks_new_work_item_targeting_deleted_member() {
        let fixture = CompileInvalidationFixture::new();
        let api = LogicalRepositoryId(stable_uuid(0x0001));
        fixture.write_selection_with_member(api, "api");
        fixture.delete_member(api, "api");

        // compile 校验触发 resolve_effective_members，阻断指向已删除成员的新工作项。
        let compile =
            resolve_logical_work_item_plan_repository_targets(&fixture.lifecycle, &fixture.plan());
        assert!(matches!(
            compile,
            Err(ref reason) if reason.contains("target_member_removed")
        ));

        // resolve 过程中 selection 失效标记已自动写入（REQ-PLN-02）。
        let selection_store = IssueCodebaseSelectionStore::new(fixture.lifecycle.app_paths());
        assert!(
            selection_store
                .is_invalidated("project_0001", "issue_0001")
                .unwrap()
        );
    }

    #[test]
    fn compile_blocks_work_item_targeting_tombstoned_member_still_in_manifest() {
        // REQ-PLN-02 fix round 1：真实 tombstone 语义（apply_delete_tombstone 只置
        // status=Tombstoned、不改 manifest.member_ids）→ 仍被 target_member_removed 阻塞，
        // 且 selection 自动失效。
        let fixture = CompileInvalidationFixture::new();
        let api = LogicalRepositoryId(stable_uuid(0x0001));
        let web = LogicalRepositoryId(stable_uuid(0x0002));
        fixture.write_selection_with_members(&[api, web], &["api", "web"]);
        // tombstone web：manifest 仍含 web，仅 status → Tombstoned。
        fixture.tombstone_member(web, "web");

        // compile 校验触发 resolve_effective_members（active 集合），阻断指向已删除成员的新工作项。
        let compile =
            resolve_logical_work_item_plan_repository_targets(&fixture.lifecycle, &fixture.plan());
        assert!(matches!(
            compile,
            Err(ref reason) if reason.contains("target_member_removed")
        ));

        // resolve 过程中 selection 失效标记已自动写入（REQ-PLN-02）。
        let selection_store = IssueCodebaseSelectionStore::new(fixture.lifecycle.app_paths());
        assert!(
            selection_store
                .is_invalidated("project_0001", "issue_0001")
                .unwrap()
        );
    }

    #[test]
    fn compile_blocks_and_invalidates_all_members_selection_when_member_tombstoned() {
        // REQ-PLN-02 fix round 2：AllMembers selection 不含已删除成员，compile 仍须
        // 返回 target_member_removed 且 selection 被标记失效（manifest 不一致兑底）。
        let fixture = CompileInvalidationFixture::new();
        let api = LogicalRepositoryId(stable_uuid(0x0001));
        let web = LogicalRepositoryId(stable_uuid(0x0002));
        fixture.write_manifest_with_members(&[api, web], &["api", "web"]);
        let selection = IssueCodebaseSelection::all_members("project_0001", "issue_0001", None);
        IssueCodebaseSelectionStore::new(fixture.lifecycle.app_paths())
            .save(&selection)
            .unwrap();
        fixture.tombstone_member(web, "web");

        let compile =
            resolve_logical_work_item_plan_repository_targets(&fixture.lifecycle, &fixture.plan());
        assert!(matches!(
            compile,
            Err(ref reason) if reason.contains("target_member_removed")
        ));

        // selection 已被标记失效（manifest 不一致兑底，selection 自身不含删除成员）。
        let selection_store = IssueCodebaseSelectionStore::new(fixture.lifecycle.app_paths());
        assert!(
            selection_store
                .is_invalidated("project_0001", "issue_0001")
                .unwrap()
        );
    }

    #[test]
    fn compile_blocks_and_invalidates_when_deleted_member_absent_from_explicit_selection() {
        // REQ-PLN-02 fix round 2：selection 未包含已删除成员，compile 仍须返回
        // target_member_removed 且 selection 被标记失效（manifest 不一致兑底）。
        let fixture = CompileInvalidationFixture::new();
        let api = LogicalRepositoryId(stable_uuid(0x0001));
        let web = LogicalRepositoryId(stable_uuid(0x0002));
        fixture.write_manifest_with_members(&[api, web], &["api", "web"]);
        // selection 只 include api（不含已删除的 web）。
        let selection = IssueCodebaseSelection::explicit(
            "project_0001",
            "issue_0001",
            vec![api],
            Vec::new(),
            Vec::new(),
            None,
        );
        IssueCodebaseSelectionStore::new(fixture.lifecycle.app_paths())
            .save(&selection)
            .unwrap();
        fixture.tombstone_member(web, "web");

        let compile =
            resolve_logical_work_item_plan_repository_targets(&fixture.lifecycle, &fixture.plan());
        assert!(matches!(
            compile,
            Err(ref reason) if reason.contains("target_member_removed")
        ));

        // selection 已被标记失效（manifest 不一致兑底，selection 自身不含删除成员）。
        let selection_store = IssueCodebaseSelectionStore::new(fixture.lifecycle.app_paths());
        assert!(
            selection_store
                .is_invalidated("project_0001", "issue_0001")
                .unwrap()
        );
    }

    #[test]
    fn compile_unpaired_state_uses_repository_routing_stable_error() {
        // 评审 Warning：Task 11 必须 TDD——先断言 compile 的不成对错误是稳定错误码
        // （repository_routing_target_missing）。当前 compile 返回自由字符串
        // "work_item_target_missing: ..."，无稳定错误码 → 本测试 FAIL。
        let fixture = CompileInvalidationFixture::new();
        let api = LogicalRepositoryId(stable_uuid(0x0001));
        // 只写 manifest + member record，不写 selection → (Some, None) 不成对状态。
        fixture.write_manifest_with_members(&[api], &["api"]);
        let result =
            resolve_logical_work_item_plan_repository_targets(&fixture.lifecycle, &fixture.plan());
        let err = result.expect_err("(Some, None) must fail-closed");
        assert!(
            err.contains("repository_routing_target_missing"),
            "compile unpaired error must carry stable error code, got: {err}"
        );

        // 与 RepositoryRouting 判定一致（防漂移契约）
        let logical_store = LogicalCodebaseStore::new(fixture.lifecycle.app_paths());
        let manifest = logical_store
            .load_manifest("project_0001")
            .unwrap()
            .unwrap();
        let routing = RepositoryRouting::classify(Some(manifest), None);
        assert!(matches!(
            routing,
            RepositoryRouting::FailClosed {
                code: RepositoryRoutingErrorCode::TargetMissing,
                ..
            }
        ));
    }

    /// Task 8 测试用最小 WorkItem：只需 id/target_repository_id/depends_on 三个字段有意义，
    /// 其余用默认值，聚焦 change_order → 跨仓 depends_on 映射逻辑。
    fn compile_test_work_item(
        id: &str,
        target_repository_id: Option<LogicalRepositoryId>,
    ) -> LifecycleWorkItemRecord {
        let now = "2026-08-10T00:00:00Z".to_string();
        LifecycleWorkItemRecord {
            id: id.to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_legacy".to_string(),
            target_repository_id,
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: format!("Work Item {id}"),
            plan_status: crate::product::models::WorkItemPlanStatus::Confirmed,
            execution_status: crate::product::models::WorkItemStatus::Pending,
            worktree_path: None,
            work_item_set_id: None,
            source_work_item_plan_id: None,
            source_outline_id: None,
            source_draft_id: None,
            planned_implementation_context: None,
            kind: crate::product::models::WorkItemKind::Backend,
            sequence_hint: None,
            depends_on: Vec::new(),
            exclusive_write_scopes: Vec::new(),
            forbidden_write_scopes: Vec::new(),
            context_budget: crate::product::models::WorkItemContextBudget::default(),
            verification_plan_ref: None,
            require_execution_plan_confirm: false,
            execution_plan_status: crate::product::models::WorkItemExecutionPlanStatus::NotStarted,
            completion_commit: None,
            completion_diff_summary_ref: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    #[test]
    fn compile_maps_change_order_to_cross_repo_depends_on() {
        // design change_order=[A,B]；work item 1 target=A，work item 2 target=B
        // compile → item 2 depends_on item 1（B 在 A 后）
        let repo_a = LogicalRepositoryId(stable_uuid(0x0101));
        let repo_b = LogicalRepositoryId(stable_uuid(0x0102));
        let mut items = [
            compile_test_work_item("work_item_a", Some(repo_a)),
            compile_test_work_item("work_item_b", Some(repo_b)),
        ];
        apply_change_order_cross_repo_depends_on(&mut items, &[repo_a, repo_b]).unwrap();
        assert!(
            items[1].depends_on.contains(&"work_item_a".to_string()),
            "B 在 change_order 中位于 A 之后 → item_b 应 depends_on item_a"
        );
        assert!(
            !items[0].depends_on.contains(&"work_item_b".to_string()),
            "A 在 change_order 中位于 B 之前 → item_a 不应 depends_on item_b"
        );
    }

    #[test]
    fn compile_rejects_change_order_cycle() {
        // change_order=[A,B] 且既有 WorkItem 级 depends_on A→B（A 依赖 B），叠加后形成环
        // A→B + B→A → blocker dependency_cycle。
        let repo_a = LogicalRepositoryId(stable_uuid(0x0103));
        let repo_b = LogicalRepositoryId(stable_uuid(0x0104));
        let mut items = [
            compile_test_work_item("work_item_a", Some(repo_a)),
            compile_test_work_item("work_item_b", Some(repo_b)),
        ];
        items[0].depends_on.push("work_item_b".to_string());
        let err = apply_change_order_cross_repo_depends_on(&mut items, &[repo_a, repo_b])
            .expect_err("change_order 与既有 depends_on 构成环必须失败");
        assert!(
            err.contains("dependency_cycle"),
            "环错误必须携带稳定 blocker 码 dependency_cycle，got: {err}"
        );
    }

    #[test]
    fn compile_same_repo_items_no_cross_repo_depends_from_change_order() {
        // 两 item 同 target=A → change_order 不建跨仓依赖（同仓顺序由既有 WorkItem 级
        // depends_on/sequence_hint 决定）。
        let repo_a = LogicalRepositoryId(stable_uuid(0x0105));
        let mut items = [
            compile_test_work_item("work_item_1", Some(repo_a)),
            compile_test_work_item("work_item_2", Some(repo_a)),
        ];
        apply_change_order_cross_repo_depends_on(&mut items, &[repo_a]).unwrap();
        assert!(
            items[0].depends_on.is_empty() && items[1].depends_on.is_empty(),
            "同 target 仓的 item 不因 change_order 建跨仓依赖"
        );
    }
}
