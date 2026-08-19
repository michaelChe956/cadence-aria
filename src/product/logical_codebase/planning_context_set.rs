use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;
use crate::product::logical_codebase::aggregate_index::{
    COMPACT_INVENTORY_HARD_BUDGET_BYTES, COMPACT_INVENTORY_SOFT_BUDGET_BYTES,
};
use crate::product::logical_codebase::{
    CodebaseMemberRecord, IssueCodebaseSelectionStore, LogicalCodebaseStore, LogicalRepositoryId,
    MemberStatus, RepositoryCheckoutRecord, RepositoryType,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// 单个参与仓库的 inventory 摘要条目。Task 4 inventory 渲染与 Task 5 resolver 只消费该集合，
/// 禁止直接枚举 manifest。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryContextSet {
    pub member_id: LogicalRepositoryId,
    pub alias: String,
    pub root_relative_path: String,
    pub role: String,
    pub repo_type: RepositoryType,
    pub tech_stack: Vec<String>,
}

/// 解析结果：参与仓库集合 + 失效成员 id + manifest 成员修订号（供 freshness 比对）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryContextResolution {
    pub set: Vec<RepositoryContextSet>,
    pub invalid_member_ids: Vec<LogicalRepositoryId>,
    pub membership_revision: u64,
}

/// 从 manifest + issue selection 解析参与仓库集合与成员 inventory 摘要。
///
/// v1.3：按 issue 唯一归属的代码库（`IssueRecord.logical_codebase_id`）把
/// manifest/member/checkout/selection 全部解析到 lc_id 子树；无 lc_id 的旧 issue
/// 回退 project 级路径（默认首个逻辑代码库）。
pub struct PlanningContextSetResolver {
    paths: ProductAppPaths,
}

impl PlanningContextSetResolver {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn resolve(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<RepositoryContextResolution, ProductStoreError> {
        let lc_id = crate::product::logical_codebase::resolve_issue_logical_codebase_id(
            &self.paths,
            project_id,
            issue_id,
        )?;
        let (logical, selections) = match lc_id.as_deref() {
            Some(lc_id) => (
                LogicalCodebaseStore::for_lc(self.paths.clone(), lc_id),
                IssueCodebaseSelectionStore::for_lc(self.paths.clone(), lc_id),
            ),
            None => (
                LogicalCodebaseStore::new(self.paths.clone()),
                IssueCodebaseSelectionStore::new(self.paths.clone()),
            ),
        };
        let manifest =
            logical
                .load_manifest(project_id)?
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "logical_codebase_manifest",
                    id: project_id.to_string(),
                })?;
        let listed_members = logical.list_members(project_id)?;
        // REQ-PLN-02：active 集合从成员记录状态推导（apply_delete_tombstone 只置
        // MemberStatus::Tombstoned、不改 manifest.member_ids）；有效成员 =
        // manifest.member_ids ∩ active_member_ids（manifest 外的 active member 不算
        // 逻辑代码库成员），删除/停用成员不进入有效成员集合，selection 据此自动失效。
        let active_set: BTreeSet<LogicalRepositoryId> = listed_members
            .iter()
            .filter(|member| member.status == MemberStatus::Active)
            .map(|member| member.logical_repository_id)
            .collect();
        let effective_active_member_ids: Vec<LogicalRepositoryId> = manifest
            .member_ids
            .iter()
            .copied()
            .filter(|id| active_set.contains(id))
            .collect();
        let resolution = selections.resolve_effective_members(
            project_id,
            issue_id,
            &effective_active_member_ids,
        )?;
        let members: BTreeMap<_, _> = listed_members
            .into_iter()
            .map(|member| (member.logical_repository_id, member))
            .collect();
        // REQ-PLN-02：manifest.member_ids 与 active 集合不一致（成员被 tombstone 但
        // manifest 未移除）→ 显式标记 selection 失效。resolve_effective_members 仅在
        // selection 引用失效成员时自动标记，manifest 层面不一致需此处兜底。
        let stale_manifest_members: Vec<LogicalRepositoryId> = manifest
            .member_ids
            .iter()
            .copied()
            .filter(|id| !active_set.contains(id))
            .collect();
        if !stale_manifest_members.is_empty() && resolution.selection.invalidation.is_none() {
            selections.mark_invalidated(project_id, issue_id, "member_removed")?;
        }
        // 合并 selection 失效成员与 manifest 层面失效成员，供 snapshot 失效传播。
        let mut invalid_member_ids = resolution.invalid_member_ids;
        for id in stale_manifest_members {
            if !invalid_member_ids.contains(&id) {
                invalid_member_ids.push(id);
            }
        }
        let checkouts = logical.list_checkouts(project_id)?;
        let paths_by_member =
            member_root_relative_paths(&manifest.provider_context_root, &checkouts);

        let mut set = Vec::with_capacity(resolution.effective_member_ids.len());
        for member_id in &resolution.effective_member_ids {
            let member: &CodebaseMemberRecord =
                members
                    .get(member_id)
                    .ok_or_else(|| ProductStoreError::Conflict {
                        kind: "selection_member_not_in_manifest",
                        id: member_id.0.to_string(),
                    })?;
            let root_relative_path = paths_by_member
                .get(member_id)
                .cloned()
                .unwrap_or_else(|| member.alias.clone());
            set.push(RepositoryContextSet {
                member_id: member.logical_repository_id,
                alias: member.alias.clone(),
                root_relative_path,
                role: member.role.clone(),
                repo_type: member.repo_type.clone(),
                tech_stack: member.tech_stack.clone(),
            });
        }
        Ok(RepositoryContextResolution {
            set,
            invalid_member_ids,
            membership_revision: manifest.membership_revision,
        })
    }
}

/// Inventory 注入预算。`DEFAULT` 与 Plan 2 `CompactMemberInventory` 的紧凑清单阈值一致
///（spike 4：默认 4 KiB / ~1,400 token；上限 8 KiB / ~2,700 token）。soft 超限先裁剪
/// 非目标成员 profile，hard 超限只保留目标成员 + omitted 计数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InventoryInjectionBudget {
    pub soft_bytes: usize,
    pub hard_bytes: usize,
    pub soft_tokens: usize,
    pub hard_tokens: usize,
}

impl InventoryInjectionBudget {
    /// spike 4 实测：默认最多 4 KiB / ~1,400 token；上限 8 KiB / ~2,700 token。
    /// 引用 `CompactMemberInventory` 已有的紧凑清单常量，保证两处一致。
    pub const DEFAULT: Self = Self {
        soft_bytes: COMPACT_INVENTORY_SOFT_BUDGET_BYTES,
        hard_bytes: COMPACT_INVENTORY_HARD_BUDGET_BYTES,
        soft_tokens: 1_400,
        hard_tokens: 2_700,
    };
}

/// `render_compact_inventory` 的渲染结果。Task 5 与 6 只注入该结果，禁止重新枚举 manifest。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryInjection {
    pub rendered: String,
    pub omitted_member_ids: Vec<LogicalRepositoryId>,
    pub truncated: bool,
    pub budget: InventoryInjectionBudget,
}

/// 渲染紧凑 inventory 清单并施加预算截断。
///
/// 渲染顺序：目标成员在前（保持 `target_member_ids` 顺序），其余成员按 alias 序。
/// 全量渲染（含 profile）≤ soft 预算时返回未截断结果；超过 soft 预算先裁剪非目标成员
/// profile；仍超 soft 则只保留目标成员并标记 `omitted_member_ids`、`truncated=true`，
/// 并在末行输出 `omitted_member_count`；绝不超 hard 预算。
///
/// 紧凑行格式与 Plan 2 `CompactMemberInventory::render` 对齐（每行投影
/// id | alias | path | role | profile）。
pub fn render_compact_inventory(
    resolution: &RepositoryContextResolution,
    target_member_ids: &[LogicalRepositoryId],
) -> Result<InventoryInjection, ProductStoreError> {
    let budget = InventoryInjectionBudget::DEFAULT;
    let target_set: BTreeSet<LogicalRepositoryId> = target_member_ids.iter().copied().collect();

    // 目标成员在前（保持 target_member_ids 顺序），其余成员按 alias 序排在后。
    let mut ordered: Vec<&RepositoryContextSet> = resolution.set.iter().collect();
    ordered.sort_by(|left, right| {
        let left_is_target = target_set.contains(&left.member_id);
        let right_is_target = target_set.contains(&right.member_id);
        match (left_is_target, right_is_target) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => left.alias.cmp(&right.alias),
        }
    });

    // 1) 全量（含 profile）。≤ soft 预算直接返回。
    let full = render_inventory_lines(&ordered, true, 0);
    if full.len() <= budget.soft_bytes {
        return Ok(InventoryInjection {
            rendered: full,
            omitted_member_ids: Vec::new(),
            truncated: false,
            budget,
        });
    }

    // 2) 裁剪非目标成员 profile（保留全部成员行）。≤ soft 仍未截断成员。
    let trimmed = render_inventory_lines(&ordered, false, 0);
    if trimmed.len() <= budget.soft_bytes {
        return Ok(InventoryInjection {
            rendered: trimmed,
            omitted_member_ids: Vec::new(),
            truncated: false,
            budget,
        });
    }

    // 3) 只保留目标成员 + omitted 计数；truncated=true。
    let kept: Vec<&RepositoryContextSet> = ordered
        .iter()
        .copied()
        .filter(|member| target_set.contains(&member.member_id))
        .collect();
    let omitted: Vec<LogicalRepositoryId> = ordered
        .iter()
        .map(|member| member.member_id)
        .filter(|id| !target_set.contains(id))
        .collect();
    let omitted_count = omitted.len();
    let mut minimal = render_inventory_lines(&kept, true, omitted_count);
    if minimal.len() > budget.hard_bytes {
        minimal.truncate(budget.hard_bytes);
    }
    Ok(InventoryInjection {
        rendered: minimal,
        omitted_member_ids: omitted,
        truncated: true,
        budget,
    })
}

/// 渲染紧凑清单行：每行 `id | alias | path | role | profile`（`include_profile=false`
/// 时省略 profile）。`omitted_count > 0` 时末行附 `omitted_member_count=N`。
fn render_inventory_lines(
    members: &[&RepositoryContextSet],
    include_profile: bool,
    omitted_count: usize,
) -> String {
    let mut out = String::new();
    for member in members {
        out.push_str(&format!(
            "{} | {} | {} | {}",
            member.member_id.0, member.alias, member.root_relative_path, member.role
        ));
        if include_profile {
            let profile = profile_summary(member);
            if !profile.is_empty() {
                out.push_str(&format!(" | {profile}"));
            }
        }
        out.push('\n');
    }
    if omitted_count > 0 {
        out.push_str(&format!("omitted_member_count={omitted_count}\n"));
    }
    out
}

/// 投影成员 profile 摘要：`RepositoryContextSet` 仅携带 tech_stack，不含 tags/owner
///（后者在 freshness 的 CompactMemberInventory 全量渲染中存在）。
fn profile_summary(member: &RepositoryContextSet) -> String {
    if member.tech_stack.is_empty() {
        String::new()
    } else {
        format!("tech_stack=[{}]", member.tech_stack.join(","))
    }
}

/// 复用 freshness 已有的根相对路径投影逻辑签名；本 resolver 仅投影 checkout 路径相对
/// manifest.provider_context_root 的后缀，用于 inventory 摘要。
fn member_root_relative_paths(
    aggregate_root: &Path,
    checkouts: &[RepositoryCheckoutRecord],
) -> BTreeMap<LogicalRepositoryId, String> {
    let mut map = BTreeMap::new();
    for checkout in checkouts {
        if let Ok(relative) = checkout.canonical_path.strip_prefix(aggregate_root) {
            map.insert(
                checkout.logical_repository_id,
                relative.to_string_lossy().into_owned(),
            );
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::logical_codebase::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, IssueCodebaseSelection,
        IssueCodebaseSelectionStore, LogicalCodebaseManifest, LogicalCodebaseStore, MemberStatus,
        RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
    };
    use std::path::PathBuf;
    use tempfile::TempDir;
    use uuid::Uuid;

    /// 稳定 UUID：禁止运行时随机，保证测试可复现；ID 组成磁盘路径前受 validate_relative_id 约束。
    const API_MEMBER_UUID: Uuid = stable_uuid(0x0001);
    const REMOVED_MEMBER_UUID: Uuid = stable_uuid(0x0002);

    const fn stable_uuid(seed: u16) -> Uuid {
        let mut bytes = [0u8; 16];
        bytes[14] = (seed >> 8) as u8;
        bytes[15] = seed as u8;
        // version 7 + variant 10xx，满足 Uuid::from_bytes 的合法构造。
        bytes[6] = 0x70;
        bytes[8] = 0x80;
        Uuid::from_bytes(bytes)
    }

    struct ContextSetFixture {
        temp: TempDir,
        paths: ProductAppPaths,
        api_member_id: LogicalRepositoryId,
        removed_member_id: LogicalRepositoryId,
    }

    impl ContextSetFixture {
        fn resolver(&self) -> PlanningContextSetResolver {
            PlanningContextSetResolver::new(self.paths.clone())
        }

        fn provider_context_root(&self) -> PathBuf {
            self.temp.path().join("aggregate-root")
        }

        /// 写入两成员 manifest：api（active）+ removed 成员（status=Removed，不在 manifest.member_ids）。
        /// 再写入 issue_0001 selection：显式 include 两个成员，使 api 进入 effective、removed 进入 invalid。
        fn write_two_member_manifest_with_one_removed(&self) {
            let store = LogicalCodebaseStore::new(self.paths.clone());

            // manifest.member_ids 仅含 active 成员（selection/resolve 视该集合为 active）。
            let manifest = LogicalCodebaseManifest::new(
                "project_0001",
                self.provider_context_root(),
                vec![self.api_member_id],
            );
            store.save_manifest("project_0001", &manifest).unwrap();

            store
                .save_member(
                    "project_0001",
                    &self.member_record(self.api_member_id, "api", MemberStatus::Active),
                )
                .unwrap();
            // removed 成员仍落盘，但 status=Removed 且不在 manifest.member_ids，因此进入 invalid。
            store
                .save_member(
                    "project_0001",
                    &self.member_record(self.removed_member_id, "web", MemberStatus::Removed),
                )
                .unwrap();

            // checkout 记录，让 root_relative_path 投影覆盖真实分支。
            store
                .save_checkout("project_0001", &self.api_checkout())
                .unwrap();

            // selection 显式含两个成员：api 有效、removed 失效。
            let selection = IssueCodebaseSelection::explicit(
                "project_0001",
                "issue_0001",
                vec![self.api_member_id, self.removed_member_id],
                Vec::new(),
                Vec::new(),
                None,
            );
            IssueCodebaseSelectionStore::new(self.paths.clone())
                .save(&selection)
                .unwrap();
        }

        /// 真实 tombstone 语义（Plan 1 `apply_delete_tombstone`）：member.status →
        /// Tombstoned，但 manifest.member_ids 不变（仍含删除成员）。selection 只 include
        /// api（active）。resolver 必须按 Active 过滤，删除成员仍留在 manifest 时
        /// selection/snapshot 也要标记失效。
        fn write_manifest_with_tombstoned_member_still_included(&self) {
            let store = LogicalCodebaseStore::new(self.paths.clone());

            // manifest 仍含 removed 成员（tombstone 不修改 manifest.member_ids）。
            let manifest = LogicalCodebaseManifest::new(
                "project_0001",
                self.provider_context_root(),
                vec![self.api_member_id, self.removed_member_id],
            );
            store.save_manifest("project_0001", &manifest).unwrap();

            store
                .save_member(
                    "project_0001",
                    &self.member_record(self.api_member_id, "api", MemberStatus::Active),
                )
                .unwrap();
            store
                .save_member(
                    "project_0001",
                    &self.member_record(self.removed_member_id, "web", MemberStatus::Tombstoned),
                )
                .unwrap();

            store
                .save_checkout("project_0001", &self.api_checkout())
                .unwrap();

            // selection 不含删除成员：失效需由 manifest 不一致兜底触发。
            let selection = IssueCodebaseSelection::explicit(
                "project_0001",
                "issue_0001",
                vec![self.api_member_id],
                Vec::new(),
                Vec::new(),
                None,
            );
            IssueCodebaseSelectionStore::new(self.paths.clone())
                .save(&selection)
                .unwrap();
        }

        fn member_record(
            &self,
            id: LogicalRepositoryId,
            alias: &str,
            status: MemberStatus,
        ) -> CodebaseMemberRecord {
            let now = "2026-08-10T00:00:00Z".to_string();
            let checkout_path = self.provider_context_root().join(alias);
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

        fn api_checkout(&self) -> RepositoryCheckoutRecord {
            let now = "2026-08-10T00:00:00Z".to_string();
            RepositoryCheckoutRecord {
                checkout_id: RepositoryCheckoutId(Uuid::nil()),
                logical_repository_id: self.api_member_id,
                physical_repository_id: "repository_api".to_string(),
                kind: CheckoutKind::Main,
                canonical_path: self.provider_context_root().join("api"),
                checkout_path_hash: "sha256:checkout".to_string(),
                git_dir_identity: "sha256:git-dir".to_string(),
                revision: Some("abc123".to_string()),
                availability: CheckoutAvailability::Available,
                observed_at: now.clone(),
                created_at: now.clone(),
                updated_at: now,
            }
        }
    }

    fn context_set_fixture() -> ContextSetFixture {
        let temp = tempfile::tempdir().unwrap();
        ContextSetFixture {
            paths: ProductAppPaths::new(temp.path()),
            temp,
            api_member_id: LogicalRepositoryId(API_MEMBER_UUID),
            removed_member_id: LogicalRepositoryId(REMOVED_MEMBER_UUID),
        }
    }

    #[test]
    fn resolve_set_only_includes_active_members_in_selection_order() {
        let fixture = context_set_fixture();
        fixture.write_two_member_manifest_with_one_removed();

        let resolution = fixture
            .resolver()
            .resolve("project_0001", "issue_0001")
            .unwrap();
        assert_eq!(resolution.set.len(), 1);
        assert_eq!(resolution.set[0].alias, "api");
        assert_eq!(resolution.invalid_member_ids.len(), 1);
    }

    #[test]
    fn tombstoned_member_still_in_manifest_marks_selection_invalid_and_excludes_from_set() {
        // REQ-PLN-02 fix round 1：apply_delete_tombstone 只置 member.status=Tombstoned、
        // 不改 manifest.member_ids。resolver 按 Active 过滤后删除成员仍留在 manifest →
        // selection 被标记失效、删除成员从参与集合排除并进入 invalid（供 snapshot 失效）。
        let fixture = context_set_fixture();
        fixture.write_manifest_with_tombstoned_member_still_included();

        let resolution = fixture
            .resolver()
            .resolve("project_0001", "issue_0001")
            .unwrap();
        // set 只含 api（Tombstoned 成员被排除）。
        assert_eq!(resolution.set.len(), 1);
        assert_eq!(resolution.set[0].alias, "api");
        // 删除成员虽在 manifest.member_ids，但 status=Tombstoned → invalid（snapshot 失效）。
        assert!(
            resolution
                .invalid_member_ids
                .contains(&fixture.removed_member_id)
        );

        // selection 已被标记失效（manifest 不一致兜底，selection 自身不含删除成员）。
        let selection_store = IssueCodebaseSelectionStore::new(fixture.paths.clone());
        assert!(
            selection_store
                .is_invalidated("project_0001", "issue_0001")
                .unwrap()
        );
        // 失效是显式标记：既有 JSON 保留。
        let loaded = selection_store
            .load("project_0001", "issue_0001")
            .unwrap()
            .unwrap();
        assert!(
            loaded
                .included_repository_ids
                .contains(&fixture.api_member_id)
        );
    }

    #[test]
    fn active_member_outside_manifest_is_not_included_as_logical_member() {
        // REQ-PLN-02 fix round 2：有效成员 = manifest.member_ids ∩ active_member_ids；
        // manifest 外的 active member 不算逻辑代码库成员，不进入参与集合。
        let fixture = context_set_fixture();
        let orphan = LogicalRepositoryId(stable_uuid(0x0003));
        let store = LogicalCodebaseStore::new(fixture.paths.clone());
        // manifest 只含 api；orphan 为 manifest 外的 active member。
        let manifest = LogicalCodebaseManifest::new(
            "project_0001",
            fixture.provider_context_root(),
            vec![fixture.api_member_id],
        );
        store.save_manifest("project_0001", &manifest).unwrap();
        store
            .save_member(
                "project_0001",
                &fixture.member_record(fixture.api_member_id, "api", MemberStatus::Active),
            )
            .unwrap();
        store
            .save_member(
                "project_0001",
                &fixture.member_record(orphan, "orphan", MemberStatus::Active),
            )
            .unwrap();
        store
            .save_checkout("project_0001", &fixture.api_checkout())
            .unwrap();
        // AllMembers selection：候选 = manifest ∩ active = [api]，orphan 不得纳入。
        let selection = IssueCodebaseSelection::all_members("project_0001", "issue_0001", None);
        IssueCodebaseSelectionStore::new(fixture.paths.clone())
            .save(&selection)
            .unwrap();

        let resolution = fixture
            .resolver()
            .resolve("project_0001", "issue_0001")
            .unwrap();
        assert_eq!(resolution.set.len(), 1);
        assert_eq!(resolution.set[0].alias, "api");
        // orphan 不在 manifest → 不纳入也不判失效。
        assert!(!resolution.invalid_member_ids.contains(&orphan));
    }

    #[test]
    fn missing_selection_is_a_blocker_not_all_members() {
        let fixture = context_set_fixture();
        fixture.write_two_member_manifest_with_one_removed();
        assert!(matches!(
            fixture
                .resolver()
                .resolve("project_0001", "issue_missing")
                .err(),
            Some(ProductStoreError::NotFound { .. })
        ));
    }

    // --- Task 4: inventory 注入预算截断 ---

    /// 纯内存 fixture：直接构造 RepositoryContextResolution，避免落盘依赖；
    /// `count` 个成员，每个成员的 alias/path 足够长以触发预算阈值。
    struct InventoryFixture {
        members: Vec<RepositoryContextSet>,
    }

    impl InventoryFixture {
        fn new(count: usize) -> Self {
            let mut members = Vec::with_capacity(count);
            for index in 0..count {
                let seed = 0x0100 + index as u16;
                members.push(RepositoryContextSet {
                    member_id: LogicalRepositoryId(stable_uuid(seed)),
                    alias: format!("member_{index:03}_alias_padding_xxxxxxxxxxxxx"),
                    root_relative_path: format!(
                        "services/member_{index:03}/src/path_padding_yyyyyyyyyyyyy"
                    ),
                    role: "service".to_string(),
                    repo_type: RepositoryType::Backend,
                    tech_stack: vec!["rust".to_string()],
                });
            }
            Self { members }
        }

        fn resolution(&self) -> RepositoryContextResolution {
            RepositoryContextResolution {
                set: self.members.clone(),
                invalid_member_ids: Vec::new(),
                membership_revision: 1,
            }
        }

        /// 取前三个成员作为 target，保持输入顺序。
        fn first_three_targets(&self) -> Vec<LogicalRepositoryId> {
            self.members
                .iter()
                .take(3)
                .map(|member| member.member_id)
                .collect()
        }
    }

    #[test]
    fn inventory_over_soft_budget_truncates_non_targets_and_marks_omitted() {
        let fixture = InventoryFixture::new(50);
        let injection =
            render_compact_inventory(&fixture.resolution(), &fixture.first_three_targets())
                .unwrap();

        assert!(injection.rendered.len() <= InventoryInjectionBudget::DEFAULT.hard_bytes);
        assert!(injection.truncated);
        assert!(!injection.omitted_member_ids.is_empty());
        assert!(injection.rendered.contains("omitted_member_count"));
    }

    #[test]
    fn inventory_under_soft_budget_is_not_truncated() {
        let fixture = InventoryFixture::new(2);
        let injection =
            render_compact_inventory(&fixture.resolution(), &fixture.first_three_targets())
                .unwrap();
        assert!(!injection.truncated);
        assert!(injection.omitted_member_ids.is_empty());
    }
}
