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

    /// 校验 effective 成员仍属当前 manifest active 成员；返回 (selection, 有效, 失效)。
    pub fn resolve_effective_members(
        &self,
        project_id: &str,
        issue_id: &str,
        active_member_ids: &[LogicalRepositoryId],
    ) -> Result<EffectiveMemberResolution, ProductStoreError> {
        let selection =
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
        let a = LogicalRepositoryId(Uuid::new_v4());
        let b = LogicalRepositoryId(Uuid::new_v4());
        let c = LogicalRepositoryId(Uuid::new_v4());

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
}
