use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;
use crate::product::logical_codebase::{
    CodebaseMemberRecord, IssueCodebaseSelectionStore, LogicalCodebaseStore, LogicalRepositoryId,
    RepositoryCheckoutRecord, RepositoryType,
};
use std::collections::BTreeMap;
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
pub struct PlanningContextSetResolver {
    logical: LogicalCodebaseStore,
    selections: IssueCodebaseSelectionStore,
}

impl PlanningContextSetResolver {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self {
            logical: LogicalCodebaseStore::new(paths.clone()),
            selections: IssueCodebaseSelectionStore::new(paths),
        }
    }

    pub fn resolve(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<RepositoryContextResolution, ProductStoreError> {
        let manifest =
            self.logical
                .load_manifest(project_id)?
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "logical_codebase_manifest",
                    id: project_id.to_string(),
                })?;
        let resolution = self.selections.resolve_effective_members(
            project_id,
            issue_id,
            &manifest.member_ids,
        )?;
        let members: BTreeMap<_, _> = self
            .logical
            .list_members(project_id)?
            .into_iter()
            .map(|member| (member.logical_repository_id, member))
            .collect();
        let checkouts = self.logical.list_checkouts(project_id)?;
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
            invalid_member_ids: resolution.invalid_member_ids,
            membership_revision: manifest.membership_revision,
        })
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
}
