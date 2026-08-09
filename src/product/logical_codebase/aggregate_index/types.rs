use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::product::logical_codebase::{LogicalRepositoryId, RepositoryCheckoutId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateIndexStatus {
    Building,
    Active,
    Stale,
    Degraded,
    Superseded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateIndexMemberSnapshot {
    pub logical_repository_id: LogicalRepositoryId,
    pub checkout_id: RepositoryCheckoutId,
    pub revision: String,
    pub dirty: bool,
    pub included: bool,
    pub indexed_at: String,
}

impl AggregateIndexMemberSnapshot {
    pub fn indexed(
        logical_repository_id: LogicalRepositoryId,
        checkout_id: RepositoryCheckoutId,
        revision: String,
        dirty: bool,
        indexed_at: String,
    ) -> Self {
        Self {
            logical_repository_id,
            checkout_id,
            revision,
            dirty,
            included: true,
            indexed_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateIndexRecord {
    pub aggregate_index_id: String,
    pub project_id: String,
    pub membership_revision: u64,
    pub status: AggregateIndexStatus,
    pub member_snapshots: Vec<AggregateIndexMemberSnapshot>,
    pub codegraph_version: String,
    pub codegraph_root: PathBuf,
    #[serde(default)]
    pub config_digest: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes_aggregate_index_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

impl AggregateIndexRecord {
    pub fn building(
        aggregate_index_id: String,
        project_id: String,
        membership_revision: u64,
        member_snapshots: Vec<AggregateIndexMemberSnapshot>,
        created_at: String,
    ) -> Self {
        Self {
            aggregate_index_id,
            project_id,
            membership_revision,
            status: AggregateIndexStatus::Building,
            member_snapshots,
            codegraph_version: "1.5.0".into(),
            codegraph_root: PathBuf::new(),
            config_digest: String::new(),
            updated_at: created_at.clone(),
            created_at,
            supersedes_aggregate_index_id: None,
            warning: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AggregateIndexBudget {
    pub target_secs: u64,
    pub warn_secs: u64,
    pub fail_secs: u64,
}

impl AggregateIndexBudget {
    pub const FIFTY_MEMBER_INITIAL: Self = Self {
        target_secs: 10,
        warn_secs: 30,
        fail_secs: 120,
    };
    pub const INCREMENTAL: Self = Self {
        target_secs: 3,
        warn_secs: 10,
        fail_secs: 30,
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::{LogicalRepositoryId, RepositoryCheckoutId};
    use uuid::Uuid;

    #[test]
    fn aggregate_index_record_preserves_immutable_member_snapshot_and_budget() {
        let paths = ProductAppPaths::new("/tmp/aria");
        let member =
            LogicalRepositoryId(Uuid::parse_str("018f0f8e-2c2d-7a10-8a11-111111111111").unwrap());
        let checkout =
            RepositoryCheckoutId(Uuid::parse_str("018f0f8e-2c2d-7a10-8a11-222222222222").unwrap());
        let snapshot = AggregateIndexMemberSnapshot::indexed(
            member,
            checkout,
            "a".repeat(40),
            false,
            "2026-08-09T00:00:00Z".into(),
        );
        let record = AggregateIndexRecord::building(
            "aggregate_index_018f0f8e-2c2d-7a10-8a11-333333333333".into(),
            "project_0001".into(),
            7,
            vec![snapshot.clone()],
            "2026-08-09T00:00:00Z".into(),
        );

        assert_eq!(record.status, AggregateIndexStatus::Building);
        assert_eq!(record.member_snapshots, vec![snapshot]);
        assert_eq!(AggregateIndexBudget::FIFTY_MEMBER_INITIAL.target_secs, 10);
        assert_eq!(
            paths.aggregate_indexes_root("project_0001"),
            std::path::PathBuf::from(
                "/tmp/aria/projects/project_0001/logical-codebase/aggregate-indexes"
            )
        );
    }
}
