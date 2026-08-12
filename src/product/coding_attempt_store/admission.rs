use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_models::{CodingAttemptStatus, CodingExecutionAttempt};
use crate::product::json_store::{ProductStoreError, read_json, write_json};
use crate::product::logical_codebase::snapshot_validator::validate_snapshot_fields;
use crate::product::logical_codebase::{AggregatePolicyArtifactStore, RepositoryRouting};

use super::CodingAttemptStore;
use super::locking::with_exclusive_lock;

/// 可持久化的 admission 票据。它绑定一次批准时的 attempt 版本和快照摘要，且只能消费一次。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdmissionTicketRecord {
    pub attempt_id: String,
    pub attempt_version: u64,
    pub snapshot_digest: String,
    pub approved_at: String,
    pub expires_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consumed_at: Option<String>,
}

const ADMISSION_TICKET_TTL: Duration = Duration::minutes(5);

const TARGET_SNAPSHOT_MISSING_FOR_LOGICAL: &str = "target_snapshot_missing_for_logical";
const TARGET_SNAPSHOT_IDENTITY_DRIFTED: &str = "target_snapshot_identity_drifted";
const TARGET_SNAPSHOT_POLICY_DRIFTED: &str = "target_snapshot_policy_drifted";
const ADMISSION_TICKET_INVALID: &str = "admission_ticket_invalid";
const ADMISSION_TICKET_EXPIRED: &str = "admission_ticket_expired";
const ADMISSION_TICKET_CONSUMED: &str = "admission_ticket_consumed";
const ATTEMPT_AWAITING_MANUAL_RECOVERY: &str = "attempt_awaiting_manual_recovery";

/// Admission API failure code. The public boundary uses a stable string while its variants keep
/// internal failures distinguishable for callers and tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StableCode {
    TargetSnapshotMissingForLogical,
    TargetSnapshotIdentityDrifted,
    TargetSnapshotPolicyDrifted,
    AdmissionTicketInvalid,
    AdmissionTicketExpired,
    AdmissionTicketConsumed,
    AttemptAwaitingManualRecovery,
    AttemptTerminalTransitionInvalid,
    StoreFailure,
}

impl StableCode {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::TargetSnapshotMissingForLogical => TARGET_SNAPSHOT_MISSING_FOR_LOGICAL,
            Self::TargetSnapshotIdentityDrifted => TARGET_SNAPSHOT_IDENTITY_DRIFTED,
            Self::TargetSnapshotPolicyDrifted => TARGET_SNAPSHOT_POLICY_DRIFTED,
            Self::AdmissionTicketInvalid => ADMISSION_TICKET_INVALID,
            Self::AdmissionTicketExpired => ADMISSION_TICKET_EXPIRED,
            Self::AdmissionTicketConsumed => ADMISSION_TICKET_CONSUMED,
            Self::AttemptAwaitingManualRecovery => ATTEMPT_AWAITING_MANUAL_RECOVERY,
            Self::AttemptTerminalTransitionInvalid => "attempt_terminal_transition_invalid",
            Self::StoreFailure => "admission_store_failure",
        }
    }
}

impl std::fmt::Display for StableCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::error::Error for StableCode {}

#[allow(dead_code)]
/// Resolves and persists an admission ticket for the globally unique attempt id.
pub(crate) fn admit_attempt_for_execution(
    paths: &ProductAppPaths,
    attempt_id: &str,
) -> Result<AdmissionTicketRecord, StableCode> {
    CodingAttemptStore::new(paths.clone()).admit_attempt_for_execution(attempt_id)
}

#[allow(dead_code)]
/// Consumes a persisted admission ticket and atomically transitions the attempt to Running.
pub(crate) fn transition_to_executable(
    paths: &ProductAppPaths,
    attempt_id: &str,
    ticket: &AdmissionTicketRecord,
) -> Result<(), StableCode> {
    CodingAttemptStore::new(paths.clone()).transition_to_executable(attempt_id, ticket)
}

#[allow(dead_code)]
/// Persists the fail-closed manual-recovery state and its stable reason code.
pub(crate) fn transition_to_awaiting_manual_recovery(
    paths: &ProductAppPaths,
    attempt_id: &str,
    reason_stable_code: &str,
) -> Result<(), StableCode> {
    CodingAttemptStore::new(paths.clone())
        .transition_to_awaiting_manual_recovery(attempt_id, reason_stable_code)
}

#[allow(dead_code)]
/// Performs a terminal status transition under the attempt lock. Only terminal targets are
/// accepted; version advancement invalidates any admission ticket issued before termination.
pub(crate) fn transition_to_terminal(
    paths: &ProductAppPaths,
    attempt_id: &str,
    status: CodingAttemptStatus,
) -> Result<(), StableCode> {
    CodingAttemptStore::new(paths.clone()).transition_to_terminal(attempt_id, status)
}

impl CodingAttemptStore {
    /// 在 attempt 级文件锁内重新校验路由、快照身份和 policy，再生成持久化 ticket。
    ///
    /// `Legacy + None` 保留原来的单仓路径；任何 logical 状态中的缺失或漂移均 fail-closed。
    pub(crate) fn admit_attempt_for_execution(
        &self,
        attempt_id: &str,
    ) -> Result<AdmissionTicketRecord, StableCode> {
        let attempt = self
            .get_attempt_by_id(attempt_id)
            .map_err(admission_store_code)?;
        let path = self.attempt_path(&attempt.project_id, &attempt.issue_id, &attempt.id);
        with_exclusive_lock(&path, || {
            self.admit_attempt_for_execution_locked(&attempt.id)
        })
        .map_err(admission_store_code)
    }

    /// 消费此前持久化的 ticket，并在同一个 attempt 级文件锁内完成 version CAS、状态和
    /// ticket consumed_at 的写入。任何失败均不写半状态。
    pub(crate) fn transition_to_executable(
        &self,
        attempt_id: &str,
        ticket: &AdmissionTicketRecord,
    ) -> Result<(), StableCode> {
        if ticket.attempt_id != attempt_id {
            return Err(StableCode::AdmissionTicketInvalid);
        }
        let attempt = self
            .get_attempt_by_id(attempt_id)
            .map_err(admission_store_code)?;
        let path = self.attempt_path(&attempt.project_id, &attempt.issue_id, &attempt.id);
        with_exclusive_lock(&path, || {
            self.transition_to_executable_locked(&attempt.id, ticket)
        })
        .map_err(admission_store_code)
    }

    /// 受控地将 attempt 置于 abort-only 的人工恢复状态，并持久化机器可读原因。
    pub(crate) fn transition_to_awaiting_manual_recovery(
        &self,
        attempt_id: &str,
        reason_stable_code: &str,
    ) -> Result<(), StableCode> {
        let attempt = self
            .get_attempt_by_id(attempt_id)
            .map_err(admission_store_code)?;
        let path = self.attempt_path(&attempt.project_id, &attempt.issue_id, &attempt.id);
        with_exclusive_lock(&path, || {
            let mut attempt =
                self.get_attempt(&attempt.project_id, &attempt.issue_id, attempt_id)?;
            if attempt.status == CodingAttemptStatus::AwaitingManualRecovery {
                return Ok(());
            }
            if !attempt.status.is_active() {
                return Err(ProductStoreError::Io(format!(
                    "{ATTEMPT_AWAITING_MANUAL_RECOVERY}: {:?}",
                    attempt.status
                )));
            }
            attempt.status = CodingAttemptStatus::AwaitingManualRecovery;
            attempt.version += 1;
            attempt.manual_recovery_reason = Some(reason_stable_code.to_string());
            attempt.updated_at = Utc::now().to_rfc3339();
            self.save_coding_attempt_with_status(&attempt)
        })
        .map_err(admission_store_code)
    }

    fn transition_to_terminal(
        &self,
        attempt_id: &str,
        status: CodingAttemptStatus,
    ) -> Result<(), StableCode> {
        if !matches!(
            status,
            CodingAttemptStatus::Completed
                | CodingAttemptStatus::Failed
                | CodingAttemptStatus::Aborted
        ) {
            return Err(StableCode::AttemptTerminalTransitionInvalid);
        }
        let attempt = self
            .get_attempt_by_id(attempt_id)
            .map_err(admission_store_code)?;
        let path = self.attempt_path(&attempt.project_id, &attempt.issue_id, &attempt.id);
        with_exclusive_lock(&path, || {
            let mut attempt =
                self.get_attempt(&attempt.project_id, &attempt.issue_id, attempt_id)?;
            if !super::attempt::valid_status_transition(&attempt.status, &status) {
                return Err(ProductStoreError::Io(
                    "attempt_terminal_transition_invalid".to_string(),
                ));
            }
            let now = Utc::now().to_rfc3339();
            attempt.status = status;
            attempt.version += 1;
            attempt.updated_at = now.clone();
            attempt.completed_at = Some(now);
            self.save_coding_attempt_with_status(&attempt)
        })
        .map_err(admission_store_code)
    }

    fn admit_attempt_for_execution_locked(
        &self,
        attempt_id: &str,
    ) -> Result<AdmissionTicketRecord, ProductStoreError> {
        let attempt = self.get_attempt_by_id(attempt_id)?;
        if attempt.status == CodingAttemptStatus::AwaitingManualRecovery {
            return Err(ProductStoreError::Io(
                ATTEMPT_AWAITING_MANUAL_RECOVERY.to_string(),
            ));
        }

        let routing =
            RepositoryRouting::load_for_issue(&self.paths, &attempt.project_id, &attempt.issue_id)?;
        let snapshot_digest = match (routing, attempt.target_snapshot.as_ref()) {
            (RepositoryRouting::Legacy { .. }, None) => legacy_snapshot_digest(&attempt),
            (RepositoryRouting::Logical { .. }, None) => {
                return Err(ProductStoreError::Io(
                    TARGET_SNAPSHOT_MISSING_FOR_LOGICAL.to_string(),
                ));
            }
            (RepositoryRouting::Logical { .. }, Some(snapshot)) => {
                validate_snapshot_fields(&self.paths, &attempt).map_err(|_| {
                    ProductStoreError::Io(TARGET_SNAPSHOT_IDENTITY_DRIFTED.to_string())
                })?;
                let policy = AggregatePolicyArtifactStore::new(self.paths.clone())
                    .get(&attempt.project_id)?
                    .ok_or_else(|| {
                        ProductStoreError::Io(TARGET_SNAPSHOT_POLICY_DRIFTED.to_string())
                    })?;
                if policy.digest != snapshot.policy_digest {
                    return Err(ProductStoreError::Io(
                        TARGET_SNAPSHOT_POLICY_DRIFTED.to_string(),
                    ));
                }
                snapshot_digest(snapshot)
            }
            (RepositoryRouting::Legacy { .. }, Some(_))
            | (RepositoryRouting::FailClosed { .. }, _) => {
                return Err(ProductStoreError::Io(
                    TARGET_SNAPSHOT_IDENTITY_DRIFTED.to_string(),
                ));
            }
        };

        let now = Utc::now();
        let ticket = AdmissionTicketRecord {
            attempt_id: attempt.id.clone(),
            attempt_version: attempt.version,
            snapshot_digest,
            approved_at: now.to_rfc3339(),
            expires_at: (now + ADMISSION_TICKET_TTL).to_rfc3339(),
            consumed_at: None,
        };
        write_json(
            &self.admission_ticket_path(&attempt.project_id, &attempt.issue_id, &attempt.id),
            &ticket,
        )?;
        Ok(ticket)
    }

    fn transition_to_executable_locked(
        &self,
        attempt_id: &str,
        ticket: &AdmissionTicketRecord,
    ) -> Result<(), ProductStoreError> {
        let mut attempt = self.get_attempt_by_id(attempt_id)?;
        if attempt.status == CodingAttemptStatus::AwaitingManualRecovery {
            return Err(ProductStoreError::Io(
                ATTEMPT_AWAITING_MANUAL_RECOVERY.to_string(),
            ));
        }
        let ticket_path =
            self.admission_ticket_path(&attempt.project_id, &attempt.issue_id, &attempt.id);
        let mut persisted: AdmissionTicketRecord = read_json(&ticket_path)?;
        if persisted.attempt_id != ticket.attempt_id
            || persisted.attempt_id != attempt.id
            || persisted.attempt_version != ticket.attempt_version
            || persisted.snapshot_digest != ticket.snapshot_digest
            || persisted.approved_at != ticket.approved_at
            || persisted.expires_at != ticket.expires_at
        {
            return Err(ProductStoreError::Io(ADMISSION_TICKET_INVALID.to_string()));
        }
        if persisted.consumed_at.is_some() {
            return Err(ProductStoreError::Io(ADMISSION_TICKET_CONSUMED.to_string()));
        }
        if ticket_expired(&persisted)? {
            return Err(ProductStoreError::Io(ADMISSION_TICKET_EXPIRED.to_string()));
        }
        if persisted.attempt_version != attempt.version
            || attempt.status == CodingAttemptStatus::Running
            || !super::attempt::valid_status_transition(
                &attempt.status,
                &CodingAttemptStatus::Running,
            )
        {
            return Err(ProductStoreError::Io(ADMISSION_TICKET_INVALID.to_string()));
        }

        let now = Utc::now().to_rfc3339();
        attempt.status = CodingAttemptStatus::Running;
        attempt.version += 1;
        attempt.updated_at = now.clone();
        persisted.consumed_at = Some(now);
        self.save_coding_attempt_with_status(&attempt)?;
        write_json(&ticket_path, &persisted)
    }
}

fn ticket_expired(ticket: &AdmissionTicketRecord) -> Result<bool, ProductStoreError> {
    let expires_at = DateTime::parse_from_rfc3339(&ticket.expires_at)
        .map_err(|_| ProductStoreError::Io(ADMISSION_TICKET_INVALID.to_string()))?;
    Ok(expires_at <= Utc::now())
}

fn snapshot_digest(snapshot: &crate::product::coding_models::AttemptTargetSnapshot) -> String {
    let encoded = serde_json::to_vec(snapshot).expect("AttemptTargetSnapshot must serialize");
    format!("sha256:{:x}", Sha256::digest(encoded))
}

fn legacy_snapshot_digest(attempt: &CodingExecutionAttempt) -> String {
    format!(
        "legacy:{}:{}:{}",
        attempt.project_id, attempt.issue_id, attempt.id
    )
}

fn admission_store_code(error: ProductStoreError) -> StableCode {
    match error {
        ProductStoreError::Io(message) if message == TARGET_SNAPSHOT_MISSING_FOR_LOGICAL => {
            StableCode::TargetSnapshotMissingForLogical
        }
        ProductStoreError::Io(message) if message == TARGET_SNAPSHOT_IDENTITY_DRIFTED => {
            StableCode::TargetSnapshotIdentityDrifted
        }
        ProductStoreError::Io(message) if message == TARGET_SNAPSHOT_POLICY_DRIFTED => {
            StableCode::TargetSnapshotPolicyDrifted
        }
        ProductStoreError::Io(message) if message == ADMISSION_TICKET_INVALID => {
            StableCode::AdmissionTicketInvalid
        }
        ProductStoreError::Io(message) if message == ADMISSION_TICKET_EXPIRED => {
            StableCode::AdmissionTicketExpired
        }
        ProductStoreError::Io(message) if message == ADMISSION_TICKET_CONSUMED => {
            StableCode::AdmissionTicketConsumed
        }
        ProductStoreError::Io(message) if message == ATTEMPT_AWAITING_MANUAL_RECOVERY => {
            StableCode::AttemptAwaitingManualRecovery
        }
        ProductStoreError::Io(message) if message == "attempt_terminal_transition_invalid" => {
            StableCode::AttemptTerminalTransitionInvalid
        }
        _ => StableCode::StoreFailure,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
    use crate::product::coding_models::{
        AttemptTargetSnapshot, CodingAttemptStatus, CodingExecutionAttempt,
    };
    use crate::product::json_store::{read_json, write_json};
    use crate::product::logical_codebase::{
        AggregatePolicyArtifactStore, CheckoutAvailability, CheckoutKind, CodebaseMemberRecord,
        IssueCodebaseSelection, IssueCodebaseSelectionStore, LogicalCodebaseManifest,
        LogicalCodebaseStore, LogicalRepositoryId, MemberStatus, RepositoryCheckoutId,
        RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
    };
    use crate::product::models::{ProviderName, RepositoryRecord};
    use crate::product::project_store::{CreateProjectInput, ProjectStore};
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;
    use uuid::Uuid;

    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_0001";
    const WORK_ITEM_ID: &str = "work_item_0001";

    struct Fixture {
        _temp: tempfile::TempDir,
        paths: ProductAppPaths,
        store: CodingAttemptStore,
        attempt: CodingExecutionAttempt,
        logical_id: LogicalRepositoryId,
        checkout_id: RepositoryCheckoutId,
    }

    #[test]
    fn legacy_attempt_without_snapshot_is_admitted_and_transitions_once() {
        let fixture = legacy_fixture();

        let ticket = fixture
            .store
            .admit_attempt_for_execution(&fixture.attempt.id)
            .expect("legacy ticket");
        assert_eq!(ticket.attempt_version, 0);
        assert!(ticket.consumed_at.is_none());
        assert!(ticket.snapshot_digest.starts_with("legacy:"));

        fixture
            .store
            .transition_to_executable(&fixture.attempt.id, &ticket)
            .expect("consume ticket");
        let attempt = fixture
            .store
            .get_attempt(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("running attempt");
        assert_eq!(attempt.status, CodingAttemptStatus::Running);
        assert_eq!(attempt.version, 1);

        assert_eq!(
            fixture
                .store
                .transition_to_executable(&fixture.attempt.id, &ticket)
                .expect_err("ticket cannot be consumed twice"),
            StableCode::AdmissionTicketConsumed
        );
        let persisted: AdmissionTicketRecord = read_json(&fixture.store.admission_ticket_path(
            PROJECT_ID,
            ISSUE_ID,
            &fixture.attempt.id,
        ))
        .expect("ticket persisted");
        assert!(persisted.consumed_at.is_some());
    }

    #[test]
    fn transition_rejects_version_mismatch_and_expired_ticket() {
        let fixture = legacy_fixture();
        let ticket = fixture
            .store
            .admit_attempt_for_execution(&fixture.attempt.id)
            .expect("ticket");
        let mut attempt = fixture
            .store
            .get_attempt(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("attempt");
        attempt.version += 1;
        fixture
            .store
            .write_coding_attempt_for_test(&attempt)
            .expect("simulate concurrent write");
        assert_eq!(
            fixture
                .store
                .transition_to_executable(&fixture.attempt.id, &ticket)
                .expect_err("stale ticket"),
            StableCode::AdmissionTicketInvalid
        );

        let fresh = fixture
            .store
            .admit_attempt_for_execution(&fixture.attempt.id)
            .expect("fresh ticket");
        let ticket_path =
            fixture
                .store
                .admission_ticket_path(PROJECT_ID, ISSUE_ID, &fixture.attempt.id);
        let mut expired: AdmissionTicketRecord = read_json(&ticket_path).expect("ticket");
        expired.expires_at = "2000-01-01T00:00:00Z".to_string();
        write_json(&ticket_path, &expired).expect("expire persisted ticket");
        assert_eq!(
            fixture
                .store
                .transition_to_executable(&fixture.attempt.id, &expired)
                .expect_err("expired ticket"),
            StableCode::AdmissionTicketExpired
        );
        assert_ne!(fresh, expired);
    }

    #[test]
    fn logical_attempt_without_snapshot_fails_closed_and_can_enter_manual_recovery() {
        let fixture = logical_fixture();
        let error = fixture
            .store
            .admit_attempt_for_execution(&fixture.attempt.id)
            .expect_err("logical attempt without snapshot fails closed");
        assert_eq!(error, StableCode::TargetSnapshotMissingForLogical);

        fixture
            .store
            .transition_to_awaiting_manual_recovery(&fixture.attempt.id, error.as_str())
            .expect("persist manual recovery");
        let attempt = fixture
            .store
            .get_attempt(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("attempt");
        assert_eq!(attempt.status, CodingAttemptStatus::AwaitingManualRecovery);
        assert_eq!(
            attempt.manual_recovery_reason.as_deref(),
            Some(TARGET_SNAPSHOT_MISSING_FOR_LOGICAL)
        );
        assert_eq!(
            fixture
                .store
                .transition_to_executable(
                    &fixture.attempt.id,
                    &AdmissionTicketRecord {
                        attempt_id: fixture.attempt.id.clone(),
                        attempt_version: attempt.version,
                        snapshot_digest: "forged".to_string(),
                        approved_at: Utc::now().to_rfc3339(),
                        expires_at: (Utc::now() + Duration::minutes(1)).to_rfc3339(),
                        consumed_at: None,
                    },
                )
                .expect_err("manual recovery is abort-only"),
            StableCode::AttemptAwaitingManualRecovery
        );
    }

    #[test]
    fn manual_recovery_transitions_active_attempt_and_terminal_transition_advances_version() {
        let fixture = legacy_fixture();
        let ticket = fixture
            .store
            .admit_attempt_for_execution(&fixture.attempt.id)
            .expect("ticket");
        fixture
            .store
            .transition_to_executable(&fixture.attempt.id, &ticket)
            .expect("running");

        fixture
            .store
            .transition_to_awaiting_manual_recovery(
                &fixture.attempt.id,
                TARGET_SNAPSHOT_IDENTITY_DRIFTED,
            )
            .expect("active attempt can be quarantined");
        let quarantined = fixture
            .store
            .get_attempt(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("quarantined");
        assert_eq!(
            quarantined.status,
            CodingAttemptStatus::AwaitingManualRecovery
        );
        assert_eq!(quarantined.version, 2);

        fixture
            .store
            .transition_to_terminal(&fixture.attempt.id, CodingAttemptStatus::Aborted)
            .expect("abort is the only manual-recovery exit");
        let aborted = fixture
            .store
            .get_attempt(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("aborted");
        assert_eq!(aborted.status, CodingAttemptStatus::Aborted);
        assert_eq!(aborted.version, 3);
        assert!(aborted.completed_at.is_some());
    }

    #[test]
    fn logical_snapshot_identity_and_policy_drift_are_rejected() {
        let fixture = logical_fixture_with_snapshot();
        let mut checkout = LogicalCodebaseStore::new(fixture.paths.clone())
            .load_checkout(PROJECT_ID, fixture.checkout_id)
            .unwrap()
            .unwrap();
        checkout.git_dir_identity = "sha256:drifted".to_string();
        LogicalCodebaseStore::new(fixture.paths.clone())
            .save_checkout(PROJECT_ID, &checkout)
            .unwrap();
        assert_eq!(
            fixture
                .store
                .admit_attempt_for_execution(&fixture.attempt.id)
                .expect_err("identity drift"),
            StableCode::TargetSnapshotIdentityDrifted
        );

        let fixture = logical_fixture_with_snapshot();
        let policy_store = AggregatePolicyArtifactStore::new(fixture.paths.clone());
        let policy = policy_store.get(PROJECT_ID).unwrap().unwrap();
        policy_store
            .save(
                PROJECT_ID,
                &policy.with_revised_policy("changed policy", "2026-08-12T00:00:00Z"),
            )
            .unwrap();
        assert_eq!(
            fixture
                .store
                .admit_attempt_for_execution(&fixture.attempt.id)
                .expect_err("policy drift"),
            StableCode::TargetSnapshotPolicyDrifted
        );
    }

    fn legacy_fixture() -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let store = CodingAttemptStore::new(paths.clone());
        let attempt = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: WORK_ITEM_ID.to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/attempt".to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: None,
                max_auto_rework: 0,
            })
            .unwrap();
        Fixture {
            _temp: temp,
            paths,
            store,
            attempt,
            logical_id: LogicalRepositoryId(Uuid::nil()),
            checkout_id: RepositoryCheckoutId(Uuid::nil()),
        }
    }

    fn logical_fixture() -> Fixture {
        logical_fixture_inner(None)
    }

    fn logical_fixture_with_snapshot() -> Fixture {
        let mut fixture = logical_fixture_inner(None);
        let snapshot = snapshot_for(&fixture);
        let mut attempt = fixture.attempt.clone();
        attempt.target_snapshot = Some(snapshot);
        fixture
            .store
            .write_coding_attempt_for_test(&attempt)
            .expect("persist snapshot");
        fixture.attempt = attempt;
        fixture
    }

    fn logical_fixture_inner(snapshot: Option<AttemptTargetSnapshot>) -> Fixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".to_string(),
                description: None,
            })
            .unwrap();
        let logical_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        let repository_path = temp.path().join("repository_0001");
        let source_identity = RepositorySourceIdentity::from_git_parts(
            &repository_path,
            repository_path.join(".git"),
            None,
        );
        let authority = LogicalCodebaseStore::new(paths.clone());
        let manifest = LogicalCodebaseManifest::new(
            PROJECT_ID,
            temp.path().join("aggregate-root"),
            vec![logical_id],
        );
        authority.save_manifest(PROJECT_ID, &manifest).unwrap();
        authority
            .save_member(
                PROJECT_ID,
                &CodebaseMemberRecord {
                    logical_repository_id: logical_id,
                    physical_repository_id: "repository_0001".to_string(),
                    alias: "repository_0001".to_string(),
                    role: "repository".to_string(),
                    ordinal: 1,
                    source_identity: source_identity.clone(),
                    repo_type: RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_id],
                    status: MemberStatus::Active,
                    created_at: "2026-08-11T00:00:00Z".to_string(),
                    updated_at: "2026-08-11T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        authority
            .save_checkout(
                PROJECT_ID,
                &RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: logical_id,
                    physical_repository_id: "repository_0001".to_string(),
                    kind: CheckoutKind::Main,
                    canonical_path: repository_path.clone(),
                    checkout_path_hash: "sha256:checkout".to_string(),
                    git_dir_identity: source_identity.git_dir_identity(),
                    revision: Some("abcdef".to_string()),
                    availability: CheckoutAvailability::Available,
                    observed_at: "2026-08-11T00:00:00Z".to_string(),
                    created_at: "2026-08-11T00:00:00Z".to_string(),
                    updated_at: "2026-08-11T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        IssueCodebaseSelectionStore::new(paths.clone())
            .save(&IssueCodebaseSelection::explicit(
                PROJECT_ID,
                ISSUE_ID,
                vec![logical_id],
                Vec::new(),
                vec![logical_id],
                None,
            ))
            .unwrap();
        write_json(
            &paths.project_root(PROJECT_ID).join("repos.json"),
            &[RepositoryRecord {
                id: "repository_0001".to_string(),
                project_id: PROJECT_ID.to_string(),
                name: "repository_0001".to_string(),
                path: repository_path,
                repo_hash: "sha256:repository".to_string(),
                runtime_root: PathBuf::from("runtime"),
                default_policy_preset: "manual-write".to_string(),
                default_provider_mode: "fake".to_string(),
                created_at: "2026-08-11T00:00:00Z".to_string(),
                updated_at: "2026-08-11T00:00:00Z".to_string(),
                logical_repository_id: Some(logical_id),
                primary_checkout_id: Some(checkout_id),
                identity_schema_version: 1,
            }],
        )
        .unwrap();
        AggregatePolicyArtifactStore::new(paths.clone())
            .ensure_bootstrap(&manifest)
            .unwrap();
        let store = CodingAttemptStore::new(paths.clone());
        let attempt = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: WORK_ITEM_ID.to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/attempt".to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: snapshot,
                max_auto_rework: 0,
            })
            .unwrap();
        Fixture {
            _temp: temp,
            paths,
            store,
            attempt,
            logical_id,
            checkout_id,
        }
    }

    fn snapshot_for(fixture: &Fixture) -> AttemptTargetSnapshot {
        let checkout = LogicalCodebaseStore::new(fixture.paths.clone())
            .load_checkout(PROJECT_ID, fixture.checkout_id)
            .unwrap()
            .unwrap();
        let manifest = LogicalCodebaseStore::new(fixture.paths.clone())
            .load_manifest(PROJECT_ID)
            .unwrap()
            .unwrap();
        let policy = AggregatePolicyArtifactStore::new(fixture.paths.clone())
            .get(PROJECT_ID)
            .unwrap()
            .unwrap();
        AttemptTargetSnapshot {
            logical_repository_id: fixture.logical_id,
            checkout_id: fixture.checkout_id,
            physical_repository_id: "repository_0001".to_string(),
            canonical_path: checkout.canonical_path,
            git_dir_identity: checkout.git_dir_identity,
            revision: checkout.revision,
            policy_digest: policy.digest,
            membership_revision: manifest.membership_revision,
            captured_at: "2026-08-11T00:00:00Z".to_string(),
            capture_source: "test".to_string(),
        }
    }

    fn provider_snapshot() -> ProviderConfigSnapshot {
        ProviderConfigSnapshot {
            author: ProviderName::Fake,
            reviewer: None,
            review_rounds: 0,
            permission_modes: Default::default(),
        }
    }
}
