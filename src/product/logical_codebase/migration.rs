use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::with_exact_exclusive_lock;
use crate::product::coding_models::{
    AttemptTargetSnapshot, CodingAttemptPlanBinding, CodingAttemptScope, CodingExecutionAttempt,
    CodingExecutionUnit,
};
use crate::product::id::repo_hash_for_path;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::logical_codebase::{
    AggregatePolicyArtifactStore, CheckoutAvailability, CheckoutKind, CodebaseMemberRecord,
    IdentityRegistryEntry, IdentityRegistryState, IdentityRegistryStore, LogicalCodebaseLayout,
    LogicalCodebaseManifest, LogicalCodebaseStore, LogicalRepositoryId, MemberStatus,
    RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
};
use crate::product::models::{
    IssueRuntimeBindingRecord, IssueSharedWorktree, LifecycleWorkItemRecord, RepositoryProfile,
    RepositoryRecord, StorySpecRecord,
};

const IDENTITY_MIGRATION_JOURNAL_FILE: &str = "identity-migration.json";

/// 迁移/校验面消费的 issue 投影（只取身份与物理归属字段）。
///
/// REQ-ROUTE-01 后 `IssueRecord.logical_codebase_id` 收敛为必填 `String`，而迁移的
/// 职责**恰恰是回填尚无该字段的存量记录**：直接用 `IssueRecord` 反序列化会被
/// serde 的 `missing field` 打死，迁移器于是读不了它本来要修的那批数据。
/// 这里用只声明必要字段的宽容投影读取（serde 默认忽略未知字段，缺
/// `logical_codebase_id` 不构成错误）；**不放宽**
/// `issue_store::read_issue_record` 的契约（缺字段 ⇒
/// `InvalidRecord(issue_not_bound_to_logical_codebase)`）。
#[derive(Debug, Deserialize)]
struct LegacyIssueProjection {
    id: String,
    project_id: String,
    #[serde(default)]
    repo_id: Option<String>,
}

include!("migration_types.inc.rs");
include!("migration_executor.inc.rs");
include!("migration_executor_backfill.inc.rs");
include!("migration_executor_repair.inc.rs");
include!("migration_verifier.inc.rs");
include!("migration_helpers.inc.rs");
include!("migration_tests.inc.rs");
