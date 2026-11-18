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

/// 跳过原因：当前只有「经有界盘检确认无源码」这一种合法值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateIndexMemberSkipReason {
    NoSource, // wire: "no_source"
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateIndexMemberSnapshot {
    pub logical_repository_id: LogicalRepositoryId,
    pub checkout_id: RepositoryCheckoutId,
    pub revision: String,
    pub dirty: bool,
    pub included: bool,
    pub indexed_at: String,
    /// `Some(NoSource)` = 经盘检确认无源码，本次未纳入代码检索范围；`None` = 无跳过事实。
    /// 无 schema_version，兼容只能靠 serde 缺省：老记录无该键 ⇒ `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<AggregateIndexMemberSkipReason>,
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
            skip_reason: None,
        }
    }

    /// 记录「经有界盘检确认无源码」的跳过事实；`included` 语义不变（成员仍在快照里）。
    pub fn mark_source_less(&mut self) {
        self.skip_reason = Some(AggregateIndexMemberSkipReason::NoSource);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateIndexRecord {
    pub aggregate_index_id: String,
    pub project_id: String,
    pub membership_revision: u64,
    pub status: AggregateIndexStatus,
    pub member_snapshots: Vec<AggregateIndexMemberSnapshot>,
    /// Snapshot captured after the CodeGraph command for drift auditing. The
    /// active generation always continues to reference `member_snapshots`.
    #[serde(default)]
    pub observed_after_member_snapshots: Vec<AggregateIndexMemberSnapshot>,
    pub codegraph_version: String,
    pub codegraph_root: PathBuf,
    #[serde(default)]
    pub config_digest: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_id: Option<String>,
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
            observed_after_member_snapshots: Vec::new(),
            // 空值＝尚未实测：激活时由 operation 透传 verify_version 的
            // 实测 CLI 版本；building/failed 世代不伪造版本号误导排障
            // （现场 durable record 曾以硬编码 1.6.0 冒充实测值）。
            codegraph_version: String::new(),
            codegraph_root: PathBuf::new(),
            config_digest: String::new(),
            updated_at: created_at.clone(),
            created_at,
            command_id: None,
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

    fn member_snapshot_fixture() -> AggregateIndexMemberSnapshot {
        AggregateIndexMemberSnapshot::indexed(
            LogicalRepositoryId(Uuid::parse_str("018f0f8e-2c2d-7a10-8a11-111111111111").unwrap()),
            RepositoryCheckoutId(Uuid::parse_str("018f0f8e-2c2d-7a10-8a11-222222222222").unwrap()),
            "a".repeat(40),
            false,
            "2026-08-09T00:00:00Z".into(),
        )
    }

    #[test]
    fn member_snapshot_without_skip_reason_deserializes_to_none() {
        // 老记录没有 skip_reason 键：必须仍可读，且缺省值不得被投影成跳过证据。
        let legacy = r#"{
            "logical_repository_id": "018f0f8e-2c2d-7a10-8a11-111111111111",
            "checkout_id": "018f0f8e-2c2d-7a10-8a11-222222222222",
            "revision": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "dirty": false,
            "included": true,
            "indexed_at": "2026-08-09T00:00:00Z"
        }"#;
        let snapshot: AggregateIndexMemberSnapshot =
            serde_json::from_str(legacy).expect("老 JSON 必须可读");
        assert_eq!(snapshot.skip_reason, None);
        assert!(snapshot.included);
        // 既有构造器默认不变：indexed() 仍产 included=true 且无跳过事实。
        let constructed = member_snapshot_fixture();
        assert_eq!(constructed.skip_reason, None);
        assert!(constructed.included);
    }

    #[test]
    fn member_snapshot_roundtrips_no_source_skip_reason() {
        // included=false + no_source 的组合必须往返一致（wire 值为 "no_source"）。
        let mut snapshot = member_snapshot_fixture();
        snapshot.included = false;
        snapshot.mark_source_less();
        let wire = serde_json::to_string(&snapshot).unwrap();
        assert!(
            wire.contains(r#""skip_reason":"no_source""#),
            "wire 应含 no_source：{wire}"
        );
        let back: AggregateIndexMemberSnapshot = serde_json::from_str(&wire).unwrap();
        assert_eq!(back, snapshot);
        assert_eq!(
            back.skip_reason,
            Some(AggregateIndexMemberSkipReason::NoSource)
        );
        assert!(!back.included);
    }

    #[test]
    fn skip_reason_is_omitted_from_wire_when_none() {
        // None 时不写键；未登记的非法变体必须整体拒绝，不得被当作跳过证据。
        let wire = serde_json::to_string(&member_snapshot_fixture()).unwrap();
        assert!(
            !wire.contains("skip_reason"),
            "wire 不应含 skip_reason：{wire}"
        );
        let illegal = r#"{
            "logical_repository_id": "018f0f8e-2c2d-7a10-8a11-111111111111",
            "checkout_id": "018f0f8e-2c2d-7a10-8a11-222222222222",
            "revision": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "dirty": false,
            "included": true,
            "indexed_at": "2026-08-09T00:00:00Z",
            "skip_reason": "not_a_reason"
        }"#;
        assert!(
            serde_json::from_str::<AggregateIndexMemberSnapshot>(illegal).is_err(),
            "非法跳过变体不得被当作跳过证据读入"
        );
    }

    #[test]
    fn record_without_skip_reason_stays_readable_and_keeps_included() {
        // 整条老 record 反序列化成功：included 保持原值、skip_reason 为 None
        // （历史世代不产生跳过证据）。
        let legacy_record = r#"{
            "aggregate_index_id": "aggregate_index_018f0f8e-2c2d-7a10-8a11-333333333333",
            "project_id": "project_0001",
            "membership_revision": 7,
            "status": "active",
            "member_snapshots": [
                {
                    "logical_repository_id": "018f0f8e-2c2d-7a10-8a11-111111111111",
                    "checkout_id": "018f0f8e-2c2d-7a10-8a11-222222222222",
                    "revision": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "dirty": false,
                    "included": true,
                    "indexed_at": "2026-08-09T00:00:00Z"
                },
                {
                    "logical_repository_id": "018f0f8e-2c2d-7a10-8a11-444444444444",
                    "checkout_id": "018f0f8e-2c2d-7a10-8a11-555555555555",
                    "revision": "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    "dirty": true,
                    "included": false,
                    "indexed_at": "2026-08-09T00:00:01Z"
                }
            ],
            "codegraph_version": "1.6.0",
            "codegraph_root": "/tmp/aria/projects/project_0001/logical-codebase",
            "created_at": "2026-08-09T00:00:00Z",
            "updated_at": "2026-08-09T00:01:00Z"
        }"#;
        let record: AggregateIndexRecord = serde_json::from_str(legacy_record).unwrap();
        assert_eq!(record.member_snapshots.len(), 2);
        assert!(record.member_snapshots[0].included);
        assert_eq!(record.member_snapshots[0].skip_reason, None);
        assert!(!record.member_snapshots[1].included);
        assert_eq!(record.member_snapshots[1].skip_reason, None);
    }
}
