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
    IssueRecord, IssueRuntimeBindingRecord, IssueSharedWorktree, LifecycleWorkItemRecord,
    RepositoryProfile, RepositoryRecord, StorySpecRecord,
};

const IDENTITY_MIGRATION_JOURNAL_FILE: &str = "identity-migration.json";

include!("migration_types.inc.rs");
include!("migration_executor.inc.rs");
include!("migration_verifier.inc.rs");
include!("migration_helpers.inc.rs");
include!("migration_tests.inc.rs");
