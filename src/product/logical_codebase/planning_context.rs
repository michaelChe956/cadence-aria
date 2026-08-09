use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;
use crate::product::json_store::{read_json, validate_relative_id, write_json};
use crate::product::logical_codebase::{LogicalRepositoryId, RepositoryCheckoutId};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MemberCheckoutFingerprint {
    pub logical_repository_id: LogicalRepositoryId,
    pub checkout_id: RepositoryCheckoutId,
    pub revision: String,
    pub dirty: bool,
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlanningContextSnapshot {
    pub schema_version: u16,
    pub project_id: String,
    pub issue_id: String,
    pub membership_revision: u64,
    pub effective_member_ids: Vec<LogicalRepositoryId>,
    pub member_fingerprints: Vec<MemberCheckoutFingerprint>,
    pub aggregate_index_id: String,
    pub index_revision: u64,
    pub policy_digest: String,
    /// 冻结后由 access_fingerprint_value() 写入；serde 允许缺失以便旧文件读取后补齐。
    #[serde(default)]
    pub access_fingerprint: String,
    pub captured_at: String,
}

impl PlanningContextSnapshot {
    /// 确定性指纹：membership/index revision + policy digest + 成员指纹 canonical 序列化。
    pub fn access_fingerprint_value(&self) -> String {
        let mut members: Vec<_> = self.member_fingerprints.iter().collect();
        members.sort_by_key(|fingerprint| fingerprint.logical_repository_id);
        let mut hasher = Sha256::new();
        hasher.update(self.membership_revision.to_string().as_bytes());
        hasher.update(b"\0");
        hasher.update(self.index_revision.to_string().as_bytes());
        hasher.update(b"\0");
        hasher.update(self.policy_digest.as_bytes());
        for fingerprint in members {
            hasher.update(b"\0");
            hasher.update(fingerprint.logical_repository_id.0.to_string().as_bytes());
            hasher.update(fingerprint.revision.as_bytes());
            hasher.update([fingerprint.dirty as u8, fingerprint.available as u8]);
        }
        format!("sha256:{:x}", hasher.finalize())
    }
}

pub struct PlanningContextSnapshotStore {
    paths: ProductAppPaths,
}

impl PlanningContextSnapshotStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn save(&self, snapshot: &PlanningContextSnapshot) -> Result<(), ProductStoreError> {
        validate_relative_id(&snapshot.project_id)?;
        validate_relative_id(&snapshot.issue_id)?;
        let mut frozen = snapshot.clone();
        frozen.access_fingerprint = snapshot.access_fingerprint_value();
        write_json(
            &self
                .paths
                .planning_context_snapshot_path(&snapshot.project_id, &snapshot.issue_id),
            &frozen,
        )
    }

    pub fn load(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Option<PlanningContextSnapshot>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let path = self
            .paths
            .planning_context_snapshot_path(project_id, issue_id);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(read_json::<PlanningContextSnapshot>(&path)?))
    }

    pub fn access_fingerprint(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Option<String>, ProductStoreError> {
        Ok(self
            .load(project_id, issue_id)?
            .map(|snapshot| snapshot.access_fingerprint))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::{LogicalRepositoryId, RepositoryCheckoutId};
    use uuid::Uuid;

    #[test]
    fn snapshot_freezes_membership_index_policy_and_per_checkout_fingerprint() {
        let member = LogicalRepositoryId(Uuid::new_v4());
        let checkout = RepositoryCheckoutId(Uuid::new_v4());
        let mut snapshot = PlanningContextSnapshot {
            schema_version: 1,
            project_id: "project_0001".into(),
            issue_id: "issue_0001".into(),
            membership_revision: 7,
            effective_member_ids: vec![member],
            member_fingerprints: vec![MemberCheckoutFingerprint {
                logical_repository_id: member,
                checkout_id: checkout,
                revision: "0123456789012345678901234567890123456789".into(),
                dirty: false,
                available: true,
            }],
            aggregate_index_id: "aggregate_index_0001".into(),
            index_revision: 3,
            policy_digest: "sha256:policy".into(),
            access_fingerprint: String::new(),
            captured_at: "2026-08-10T00:00:00Z".into(),
        };
        let expected = snapshot.access_fingerprint_value();
        snapshot.access_fingerprint = expected.clone();
        assert_eq!(snapshot.access_fingerprint_value(), expected);

        let temp = tempfile::tempdir().unwrap();
        let store = PlanningContextSnapshotStore::new(ProductAppPaths::new(temp.path()));
        store.save(&snapshot).unwrap();
        let loaded = store.load("project_0001", "issue_0001").unwrap().unwrap();
        assert_eq!(loaded, snapshot);
        assert_eq!(loaded.access_fingerprint, expected);
    }
}
