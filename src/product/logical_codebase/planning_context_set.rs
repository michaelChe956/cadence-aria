use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;
use crate::product::logical_codebase::member_reality::{
    collect_member_reality, render_member_reality, MemberReality, MEMBER_REALITY_SECTION_MARKER,
};
use crate::product::logical_codebase::{
    CheckoutKind, CodebaseMemberRecord, IssueCodebaseSelectionStore, LogicalCodebaseStore,
    LogicalRepositoryId, MemberStatus, RepositoryAuthorityResolver, RepositoryCheckoutRecord,
    RepositoryType,
};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

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

/// 解析结果：参与仓库集合 + 失效成员 id + manifest 成员修订号（供 freshness 比对）
/// + 各成员仓本地 checkout 根目录（逐仓实况与逐仓基线的唯一来源）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryContextResolution {
    pub set: Vec<RepositoryContextSet>,
    pub invalid_member_ids: Vec<LogicalRepositoryId>,
    pub membership_revision: u64,
    /// 成员 → 本地 checkout 根目录。缺失该成员的 checkout 时该成员不出现（逐仓实况
    /// 相应项显「未识别」）。
    pub member_checkout_paths: BTreeMap<LogicalRepositoryId, PathBuf>,
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
        // C4 Task 2：身份先经唯一 authority resolver 冻结（kind/重复来源/legacy
        // 布局冲突 fail-closed），成员/checkout/selection 只从 resolver 确认的
        // LC 子树读取；无归属且无 legacy 别名 record 的旧数据保留 project 级
        // 兼容路径（不猜“最新可用记录”）。
        let resolution = RepositoryAuthorityResolver::new(self.paths.clone())
            .resolve_for_issue(project_id, issue_id)?;
        let lc_id = resolution
            .as_ref()
            .and_then(|resolution| resolution.target.logical_codebase_id.clone());
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
        let member_checkout_paths = member_checkout_paths(&checkouts);

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
            member_checkout_paths,
        })
    }
}

/// 评审材料仓库清单的绝对字节上限：500 KiB。
///
/// REQ-PLN-12：仓库清单 MUST 全量注入评审材料（不裁剪成员、不输出缺员标记）；
/// 渲染结果超过本上限时 MUST 以 `planning_material_over_budget` 显式阻断本轮评审，
/// 绝不静默截断后继续。
pub const PLANNING_MATERIAL_MAX_BYTES: usize = 500 * 1024;

/// `render_compact_inventory` 的渲染结果。Task 5 与 6 只注入该结果，禁止重新枚举 manifest。
///
/// `rendered` = 紧凑成员清单 + 逐仓实况段（REQ-PLN-14）。二者同属「评审材料」，
/// 共同受 `PLANNING_MATERIAL_MAX_BYTES` 绝对上限约束。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryInjection {
    pub rendered: String,
}

/// 渲染紧凑 inventory 清单（全量渲染，绝不裁剪）+ 逐仓实况段。
///
/// 渲染顺序：目标成员在前（保持 `target_member_ids` 顺序），其余成员按 alias 序。
/// 全部成员逐行输出（`id | alias | path | role | profile`），MUST NOT 按 soft/hard 预算
/// 裁剪成员，MUST NOT 输出 `omitted_member_count` 等缺员标记（REQ-PLN-12）。
///
/// 清单之后追加逐仓实况段（REQ-PLN-14）：每仓固定 5 行（`职责` / `关键目录` / `入口` /
/// `构建/测试命令` / `基线`），缺项显「未识别」并保留该行。逐仓基线取该仓 checkout
/// 当前 HEAD 所在分支；detached / 空仓 → 该仓「基线未识别」，其余仓继续。
///
/// 渲染字节数超过 `PLANNING_MATERIAL_MAX_BYTES`（500 KiB）→ 返回
/// `Err(ProductStoreError::InvalidRecord { kind: "planning_material_over_budget" })`，
/// 阻断本轮评审（不静默缺员、不截断后继续）。
///
/// 紧凑行格式与 Plan 2 `CompactMemberInventory::render` 对齐（每行投影
/// id | alias | path | role | profile）。
pub fn render_compact_inventory(
    resolution: &RepositoryContextResolution,
    target_member_ids: &[LogicalRepositoryId],
) -> Result<InventoryInjection, ProductStoreError> {
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

    let inventory = render_inventory_lines(&ordered);
    // 清单本身已超限时立即阻断（不进入逐仓实况采集，避免超限场景的无谓 IO）。
    if inventory.len() > PLANNING_MATERIAL_MAX_BYTES {
        return Err(material_over_budget(inventory.len(), ordered.len()));
    }
    let mut rendered = inventory;
    rendered.push_str(&render_member_reality_section(&ordered, resolution));
    if rendered.len() > PLANNING_MATERIAL_MAX_BYTES {
        return Err(material_over_budget(rendered.len(), ordered.len()));
    }
    Ok(InventoryInjection { rendered })
}

/// 评审材料超预算诊断（500 KiB 绝对上限）。
fn material_over_budget(rendered_bytes: usize, member_count: usize) -> ProductStoreError {
    ProductStoreError::InvalidRecord {
        kind: "planning_material_over_budget",
        reason: format!(
            "planning material inventory over budget: rendered {} bytes > {} bytes ({} members)",
            rendered_bytes, PLANNING_MATERIAL_MAX_BYTES, member_count
        ),
    }
}

/// 渲染紧凑清单行：每行 `id | alias | path | role | profile`（全量渲染，无裁剪、无缺员标记）。
fn render_inventory_lines(members: &[&RepositoryContextSet]) -> String {
    let mut out = String::new();
    for member in members {
        out.push_str(&format!(
            "{} | {} | {} | {}",
            member.member_id.0, member.alias, member.root_relative_path, member.role
        ));
        let profile = profile_summary(member);
        if !profile.is_empty() {
            out.push_str(&format!(" | {profile}"));
        }
        out.push('\n');
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

/// 成员 → 本地 checkout 根目录（逐仓实况与逐仓基线的唯一来源）。
///
/// 取 `CheckoutKind::Main` 的 checkout（成员主 checkout）；同一成员存在多个 checkout
/// 时优先 Main，其次按 checkout id 稳定取首个。缺失 checkout 的成员不出现——逐仓
/// 实况渲染时该成员相应项显「未识别」（MUST NOT 省略整仓）。
fn member_checkout_paths(
    checkouts: &[RepositoryCheckoutRecord],
) -> BTreeMap<LogicalRepositoryId, PathBuf> {
    let mut main: BTreeMap<LogicalRepositoryId, PathBuf> = BTreeMap::new();
    let mut fallback: BTreeMap<LogicalRepositoryId, PathBuf> = BTreeMap::new();
    let mut sorted: Vec<&RepositoryCheckoutRecord> = checkouts.iter().collect();
    sorted.sort_by_key(|checkout| checkout.checkout_id.0);
    for checkout in sorted {
        if checkout.kind == CheckoutKind::Main {
            main.entry(checkout.logical_repository_id)
                .or_insert_with(|| checkout.canonical_path.clone());
        } else {
            fallback
                .entry(checkout.logical_repository_id)
                .or_insert_with(|| checkout.canonical_path.clone());
        }
    }
    for (member_id, path) in fallback {
        main.entry(member_id).or_insert(path);
    }
    main
}

/// 逐仓实况渲染（REQ-PLN-14）。
///
/// 每个成员仓固定 5 行：`职责` / `关键目录` / `入口` / `构建/测试命令` / `基线`。
/// 缺项显「未识别」并保留该行（MUST NOT 省略整仓）；基线取该仓 checkout 当前 HEAD
/// 所在分支，detached / 空仓 → 该仓「基线未识别」并附诊断，其余仓继续渲染。
///
/// 段首带 `MEMBER_REALITY_SECTION_MARKER`（材料块渲染器据此分节）。
pub fn render_member_reality_section(
    ordered: &[&RepositoryContextSet],
    resolution: &RepositoryContextResolution,
) -> String {
    if ordered.is_empty() {
        return String::new();
    }
    let mut realities: Vec<MemberReality> = Vec::with_capacity(ordered.len());
    for member in ordered {
        let checkout_root = resolution
            .member_checkout_paths
            .get(&member.member_id)
            .map(|path| path.as_path());
        realities.push(collect_member_reality(
            &member.member_id.0.to_string(),
            &member.role,
            checkout_root,
        ));
    }
    let mut section = String::new();
    section.push_str(MEMBER_REALITY_SECTION_MARKER);
    section.push('\n');
    section.push_str(&render_member_reality(&realities));
    section
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

    // --- REQ-PLN-12（Task 5）：仓库清单全量注入 + 500 KiB 超限阻断 ---
    // 全量渲染与超限阻断的用例落在 `planning_context_resolver_tests.inc.rs`
    // （`inventory_renders_all_members_without_omitted_count` /
    // `inventory_over_500kib_is_rejected_not_truncated`）；本模块只保留解析语义用例。

    // --- REQ-PLN-14（Task 7）：逐仓实况 + 逐仓基线 ---

    /// 逐仓实况必须随材料返回：每仓固定 5 行（职责 / 关键目录 / 入口 / 构建测试命令 /
    /// 基线），基线取该仓 checkout 当前 HEAD 所在分支。
    #[test]
    fn member_reality_section_is_rendered_into_material_with_head_branch_baseline() {
        let fixture = context_set_fixture();
        fixture.write_two_member_manifest_with_one_removed();

        // 造一个真实 git 仓作为成员 checkout（基线来源）。
        let checkout_root = fixture.provider_context_root().join("api");
        std::fs::create_dir_all(&checkout_root).unwrap();
        let run = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(&checkout_root)
                .stdin(std::process::Stdio::null())
                .status()
                .expect("git fixture command");
            assert!(status.success(), "git {args:?}");
        };
        run(&["init", "-b", "main"]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "Test User"]);
        std::fs::write(checkout_root.join("Cargo.toml"), "[package]\n").unwrap();
        std::fs::create_dir_all(checkout_root.join("src")).unwrap();
        run(&["add", "."]);
        run(&["commit", "-m", "base"]);
        run(&["checkout", "-b", "feature/api"]);

        let resolution = fixture
            .resolver()
            .resolve("project_0001", "issue_0001")
            .unwrap();
        let injection = render_compact_inventory(&resolution, &[]).unwrap();

        assert!(
            injection.rendered.contains(MEMBER_REALITY_SECTION_MARKER),
            "材料必须含逐仓实况段：{}",
            injection.rendered
        );
        for label in ["职责", "关键目录", "入口", "构建/测试命令", "基线"] {
            assert!(
                injection.rendered.contains(&format!("{label}:")),
                "实况段必须含「{label}」行：{}",
                injection.rendered
            );
        }
        assert!(
            injection.rendered.contains("基线: feature/api"),
            "基线必须取该仓当前 HEAD 所在分支：{}",
            injection.rendered
        );
        assert!(
            injection.rendered.contains("构建/测试命令: cargo test"),
            "Rust 仓构建测试命令必须可判定：{}",
            injection.rendered
        );
    }

    /// 成员无 checkout 时：实况仍逐行渲染，基线显「基线未识别」，MUST NOT 省略整仓。
    #[test]
    fn member_reality_keeps_every_member_when_checkout_is_missing() {
        let fixture = context_set_fixture();
        fixture.write_two_member_manifest_with_one_removed();

        let resolution = fixture
            .resolver()
            .resolve("project_0001", "issue_0001")
            .unwrap();
        let injection = render_compact_inventory(&resolution, &[]).unwrap();

        assert!(
            injection.rendered.contains(MEMBER_REALITY_SECTION_MARKER),
            "材料必须含逐仓实况段：{}",
            injection.rendered
        );
        assert!(
            injection.rendered.contains("基线未识别"),
            "无 checkout 时基线必须显未识别：{}",
            injection.rendered
        );
        assert!(
            injection.rendered.contains("职责: service"),
            "成员角色仍必须渲染：{}",
            injection.rendered
        );
    }
}
