use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;
use crate::product::json_store::{read_json, validate_relative_id, write_json};
use crate::product::logical_codebase::LogicalRepositoryId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SelectionPolicy {
    AllMembers,
    Explicit,
}

/// 失效标记（REQ-PLN-02）：规划后成员被删除/停用时由 store 显式写入，不删除既有 JSON。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InvalidationRecord {
    pub reason: String,
    pub invalidated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IssueCodebaseSelection {
    pub schema_version: u16,
    pub project_id: String,
    pub issue_id: String,
    pub selection_policy: SelectionPolicy,
    #[serde(default)]
    pub included_repository_ids: Vec<LogicalRepositoryId>,
    #[serde(default)]
    pub excluded_repository_ids: Vec<LogicalRepositoryId>,
    #[serde(default)]
    pub focus_repository_ids: Vec<LogicalRepositoryId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_ref: Option<String>,
    /// 失效标记：成员删除/停用后为 Some；旧文件缺失时默认 None。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub invalidation: Option<InvalidationRecord>,
    pub created_at: String,
    pub updated_at: String,
}

impl IssueCodebaseSelection {
    pub fn all_members(project_id: &str, issue_id: &str, snapshot_ref: Option<String>) -> Self {
        Self::new(
            project_id,
            issue_id,
            SelectionPolicy::AllMembers,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            snapshot_ref,
        )
    }

    pub fn explicit(
        project_id: &str,
        issue_id: &str,
        included: Vec<LogicalRepositoryId>,
        excluded: Vec<LogicalRepositoryId>,
        focus: Vec<LogicalRepositoryId>,
        snapshot_ref: Option<String>,
    ) -> Self {
        Self::new(
            project_id,
            issue_id,
            SelectionPolicy::Explicit,
            included,
            excluded,
            focus,
            snapshot_ref,
        )
    }

    fn new(
        project_id: &str,
        issue_id: &str,
        policy: SelectionPolicy,
        included: Vec<LogicalRepositoryId>,
        excluded: Vec<LogicalRepositoryId>,
        focus: Vec<LogicalRepositoryId>,
        snapshot_ref: Option<String>,
    ) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Self {
            schema_version: 1,
            project_id: project_id.into(),
            issue_id: issue_id.into(),
            selection_policy: policy,
            included_repository_ids: included,
            excluded_repository_ids: excluded,
            focus_repository_ids: focus,
            snapshot_ref,
            invalidation: None,
            created_at: now.clone(),
            updated_at: now,
        }
    }

    /// 校验 focus ⊆ include；include/exclude 重叠时 exclude 优先由 resolve 表达，不在此报错。
    pub fn validate_focus_subset(&self) -> Result<(), ProductStoreError> {
        let include: std::collections::BTreeSet<_> =
            self.included_repository_ids.iter().copied().collect();
        for focus in &self.focus_repository_ids {
            if !include.contains(focus) {
                return Err(ProductStoreError::Conflict {
                    kind: "focus_repository_outside_include",
                    id: focus.0.to_string(),
                });
            }
        }
        Ok(())
    }

    /// 返回 include − exclude 的有效成员集合，保持 include 顺序。
    pub fn resolve_effective_members(&self) -> Vec<LogicalRepositoryId> {
        let excluded: std::collections::BTreeSet<_> =
            self.excluded_repository_ids.iter().copied().collect();
        self.included_repository_ids
            .iter()
            .copied()
            .filter(|id| !excluded.contains(id))
            .collect()
    }
}

pub struct IssueCodebaseSelectionStore {
    paths: ProductAppPaths,
}

impl IssueCodebaseSelectionStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn save(&self, selection: &IssueCodebaseSelection) -> Result<(), ProductStoreError> {
        validate_relative_id(&selection.project_id)?;
        validate_relative_id(&selection.issue_id)?;
        write_json(
            &self
                .paths
                .codebase_selection_path(&selection.project_id, &selection.issue_id),
            selection,
        )
    }

    pub fn load(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Option<IssueCodebaseSelection>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let path = self.paths.codebase_selection_path(project_id, issue_id);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(read_json::<IssueCodebaseSelection>(&path)?))
    }

    /// 显式标记 selection 失效（成员删除/停用等）；只写标记，不删除既有 JSON。
    pub fn mark_invalidated(
        &self,
        project_id: &str,
        issue_id: &str,
        reason: &str,
    ) -> Result<(), ProductStoreError> {
        let mut selection =
            self.load(project_id, issue_id)?
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "issue_codebase_selection",
                    id: format!("{project_id}/{issue_id}"),
                })?;
        selection.invalidation = Some(InvalidationRecord {
            reason: reason.to_string(),
            invalidated_at: chrono::Utc::now().to_rfc3339(),
        });
        selection.updated_at = chrono::Utc::now().to_rfc3339();
        self.save(&selection)
    }

    /// selection 是否已标记失效。
    pub fn is_invalidated(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<bool, ProductStoreError> {
        Ok(self
            .load(project_id, issue_id)?
            .is_some_and(|selection| selection.invalidation.is_some()))
    }

    /// 校验 effective 成员仍属当前 manifest active 成员；返回 (selection, 有效, 失效)。
    /// 失效成员非空时自动写入失效标记（REQ-PLN-02：规划后成员删除/停用），不删除既有 JSON。
    pub fn resolve_effective_members(
        &self,
        project_id: &str,
        issue_id: &str,
        active_member_ids: &[LogicalRepositoryId],
    ) -> Result<EffectiveMemberResolution, ProductStoreError> {
        let mut selection =
            self.load(project_id, issue_id)?
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "issue_codebase_selection",
                    id: format!("{project_id}/{issue_id}"),
                })?;
        let active: std::collections::BTreeSet<_> = active_member_ids.iter().copied().collect();
        let candidate = match selection.selection_policy {
            SelectionPolicy::AllMembers => active_member_ids.to_vec(),
            SelectionPolicy::Explicit => selection.resolve_effective_members(),
        };
        let mut effective = Vec::new();
        let mut invalid = Vec::new();
        for id in candidate {
            if active.contains(&id) {
                effective.push(id);
            } else {
                invalid.push(id);
            }
        }
        // 失效是显式标记：检测到失效成员时首次写入（幂等，不重复覆盖既有失效记录）。
        if !invalid.is_empty() && selection.invalidation.is_none() {
            selection.invalidation = Some(InvalidationRecord {
                reason: "member_removed".to_string(),
                invalidated_at: chrono::Utc::now().to_rfc3339(),
            });
            selection.updated_at = chrono::Utc::now().to_rfc3339();
            self.save(&selection)?;
        }
        Ok(EffectiveMemberResolution {
            selection,
            effective_member_ids: effective,
            invalid_member_ids: invalid,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveMemberResolution {
    pub selection: IssueCodebaseSelection,
    pub effective_member_ids: Vec<LogicalRepositoryId>,
    pub invalid_member_ids: Vec<LogicalRepositoryId>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::LogicalRepositoryId;
    use uuid::Uuid;

    #[test]
    fn focus_must_be_within_include_and_exclude_overrides_include() {
        // 稳定 UUID：禁止运行时随机，保证测试可复现（与仓库其他测试一致）。
        let a = LogicalRepositoryId(stable_uuid(0x0001));
        let b = LogicalRepositoryId(stable_uuid(0x0002));
        let c = LogicalRepositoryId(stable_uuid(0x0003));

        let explicit = IssueCodebaseSelection::explicit(
            "project_0001",
            "issue_0001",
            vec![a, b],
            vec![b],
            vec![a],
            None,
        );
        assert_eq!(explicit.resolve_effective_members(), vec![a]);

        assert!(
            IssueCodebaseSelection::explicit(
                "project_0001",
                "issue_0001",
                vec![a],
                vec![],
                vec![c],
                None,
            )
            .validate_focus_subset()
            .is_err()
        );
    }

    #[test]
    fn all_members_policy_roundtrips_with_serde_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let store = IssueCodebaseSelectionStore::new(ProductAppPaths::new(temp.path()));
        let selection = IssueCodebaseSelection::all_members("project_0001", "issue_0001", None);
        store.save(&selection).unwrap();

        let loaded = store.load("project_0001", "issue_0001").unwrap().unwrap();
        assert_eq!(loaded.selection_policy, SelectionPolicy::AllMembers);
        assert!(loaded.included_repository_ids.is_empty());
        assert_eq!(
            ProductAppPaths::new(temp.path()).codebase_selection_path("project_0001", "issue_0001"),
            temp.path()
                .join("projects/project_0001/issues/issue_0001/codebase-selection.json")
        );
    }

    /// 稳定 UUID：禁止运行时随机，保证测试可复现；ID 组成磁盘路径前受 validate_relative_id 约束。
    const fn stable_uuid(seed: u16) -> Uuid {
        let mut bytes = [0u8; 16];
        bytes[14] = (seed >> 8) as u8;
        bytes[15] = seed as u8;
        // version 7 + variant 10xx，满足 Uuid::from_bytes 的合法构造。
        bytes[6] = 0x70;
        bytes[8] = 0x80;
        Uuid::from_bytes(bytes)
    }

    #[test]
    fn deleted_member_auto_invalidates_selection_without_deleting_json() {
        let api = LogicalRepositoryId(stable_uuid(0x0001));
        let temp = tempfile::tempdir().unwrap();
        let store = IssueCodebaseSelectionStore::new(ProductAppPaths::new(temp.path()));
        let selection = IssueCodebaseSelection::explicit(
            "project_0001",
            "issue_0001",
            vec![api],
            Vec::new(),
            Vec::new(),
            None,
        );
        store.save(&selection).unwrap();

        // 成员删除后 active 集合不再含 api → invalid，resolve 自动写入失效标记。
        let resolution = store
            .resolve_effective_members("project_0001", "issue_0001", &[])
            .unwrap();
        assert!(resolution.invalid_member_ids.contains(&api));
        assert!(resolution.effective_member_ids.is_empty());
        assert!(store.is_invalidated("project_0001", "issue_0001").unwrap());

        // 失效是显式标记：既有 JSON 仍可读取，include 字段保留。
        let loaded = store.load("project_0001", "issue_0001").unwrap().unwrap();
        assert_eq!(
            loaded
                .invalidation
                .as_ref()
                .map(|record| record.reason.as_str()),
            Some("member_removed")
        );
        assert!(loaded.included_repository_ids.contains(&api));
    }

    #[test]
    fn mark_invalidated_explicit_is_invalidated_roundtrip() {
        let temp = tempfile::tempdir().unwrap();
        let store = IssueCodebaseSelectionStore::new(ProductAppPaths::new(temp.path()));
        let selection = IssueCodebaseSelection::all_members("project_0001", "issue_0001", None);
        store.save(&selection).unwrap();
        assert!(!store.is_invalidated("project_0001", "issue_0001").unwrap());

        store
            .mark_invalidated("project_0001", "issue_0001", "member_removed")
            .unwrap();
        assert!(store.is_invalidated("project_0001", "issue_0001").unwrap());
        let loaded = store.load("project_0001", "issue_0001").unwrap().unwrap();
        assert_eq!(
            loaded
                .invalidation
                .as_ref()
                .map(|record| record.reason.as_str()),
            Some("member_removed")
        );
        assert!(
            !loaded
                .invalidation
                .as_ref()
                .unwrap()
                .invalidated_at
                .is_empty()
        );
        assert!(loaded.included_repository_ids.is_empty());
    }
}
