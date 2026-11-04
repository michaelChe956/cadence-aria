    use std::path::PathBuf;

    use super::*;
    use crate::product::coding_attempt_store::attempt::register_attempt_write_gap_hook;
    use crate::product::coding_attempt_store::locking::register_lock_attempt_hook;
    use crate::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
    use crate::product::coding_models::{
        AttemptTargetSnapshot, CodingAgentRole, CodingAttemptStatus, CodingEntryType,
        CodingExecutionAttempt,
    };
    use crate::product::logical_codebase::{
        AggregatePolicyArtifactStore, CodebaseMemberRecord, IssueCodebaseSelection,
        IssueCodebaseSelectionStore, LogicalCodebaseManifest, LogicalCodebaseStore,
        LogicalRepositoryId, MemberStatus, RepositoryCheckoutId, RepositorySourceIdentity,
        RepositoryType,
    };
    use crate::product::models::{
        ProviderConversationRef, ProviderConversationRole, ProviderName,
    };
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

    /// REQ-ROUTE-01 / 去单仓入口：无 LC 归属且无 `target_snapshot` 的 attempt
    /// 不再走已删除的单仓 `legacy_snapshot_digest` 准入路径，而是一律
    /// fail-closed（`FailClosed(TargetMissing)` ⇒ `TargetSnapshotIdentityDrifted`）。
    ///
    /// 保留的不变量：**admission 的唯一凭证是「LC 权威 + 冻结快照」**——
    /// 拒绝时既不签发 ticket（不得留下半凭证），也不改写 attempt
    /// （status/version/admission marker 全部不变）。
    #[test]
    fn attempt_without_lc_authority_fails_closed_at_admission() {
        let fixture = unbound_fixture();

        assert_eq!(
            fixture
                .store
                .admit_attempt_for_execution(&fixture.attempt.id)
                .expect_err("无 LC 归属的 attempt 不得签发 admission ticket"),
            StableCode::TargetSnapshotIdentityDrifted
        );
        assert!(
            !fixture
                .store
                .admission_ticket_path(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
                .exists(),
            "fail-closed 不得留下半签发的 ticket 凭证"
        );
        let attempt = fixture
            .store
            .get_attempt(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("attempt");
        assert_eq!(attempt.status, CodingAttemptStatus::Created);
        assert_eq!(attempt.version, 0);
        assert!(attempt.admission_ticket_consumed_at.is_none());
    }

    #[test]
    fn ticket_cleanup_failure_cannot_leave_a_half_committed_transition() {
        let fixture = logical_fixture_with_snapshot();
        let ticket = fixture
            .store
            .admit_attempt_for_execution(&fixture.attempt.id)
            .expect("ticket");
        let ticket_path =
            fixture
                .store
                .admission_ticket_path(PROJECT_ID, ISSUE_ID, &fixture.attempt.id);
        let _failpoint = register_admission_ticket_cleanup_failpoint(&ticket_path);

        fixture
            .store
            .transition_to_executable(&fixture.attempt.id, &ticket)
            .expect("attempt commit must not depend on ticket cleanup");

        let attempt = fixture
            .store
            .get_attempt(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("authoritative attempt");
        assert_eq!(attempt.status, CodingAttemptStatus::Running);
        assert_eq!(attempt.version, 1);
        assert!(attempt.admission_ticket_consumed_at.is_some());
        let persisted_ticket: AdmissionTicketRecord = read_json(&ticket_path)
            .expect("cleanup failpoint deliberately retains ticket credential");
        assert!(persisted_ticket.consumed_at.is_none());
        assert_eq!(
            fixture
                .store
                .transition_to_executable(&fixture.attempt.id, &ticket)
                .expect_err("authoritative consumed marker rejects retained credential"),
            StableCode::AdmissionTicketConsumed
        );
    }

    #[test]
    fn transition_rejects_version_mismatch_and_expired_ticket() {
        let fixture = logical_fixture_with_snapshot();
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
        let fixture = logical_fixture_with_snapshot();
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
    fn manual_recovery_leaving_running_clears_admission_marker() {
        let fixture = logical_fixture_with_snapshot();
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
            .expect("manual recovery from Running");
        let attempt = fixture
            .store
            .get_attempt(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("attempt");
        assert_eq!(attempt.status, CodingAttemptStatus::AwaitingManualRecovery);
        assert!(
            attempt.admission_ticket_consumed_at.is_none(),
            "离开 Running 进入人工恢复必须在同一次锁内结束 admission 会话"
        );
    }

    #[test]
    fn transition_to_terminal_leaving_running_clears_admission_marker() {
        let fixture = logical_fixture_with_snapshot();
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
            .transition_to_terminal(&fixture.attempt.id, CodingAttemptStatus::Failed)
            .expect("terminal transition from Running");
        let attempt = fixture
            .store
            .get_attempt(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("attempt");
        assert_eq!(attempt.status, CodingAttemptStatus::Failed);
        assert!(
            attempt.admission_ticket_consumed_at.is_none(),
            "离开 Running 进入终态必须同步清 marker（会话语义）"
        );
    }

    #[test]
    fn non_status_update_after_admission_preserves_running_cas_commit() {
        let fixture = logical_fixture_with_snapshot();
        let ticket = fixture
            .store
            .admit_attempt_for_execution(&fixture.attempt.id)
            .expect("ticket");
        let mut stale_non_status = fixture.attempt.clone();
        stale_non_status.head_commit = Some("concurrent-non-status-write".to_string());

        fixture
            .store
            .transition_to_executable(&fixture.attempt.id, &ticket)
            .expect("CAS transition");
        fixture
            .store
            .update_attempt_non_status_fields(&stale_non_status)
            .expect("stale non-status payload must reload frozen fields under the lock");

        let persisted = fixture
            .store
            .get_attempt(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("persisted attempt");
        assert_eq!(persisted.status, CodingAttemptStatus::Running);
        assert_eq!(persisted.version, 1);
        assert!(persisted.admission_ticket_consumed_at.is_some());
        assert_eq!(
            persisted.head_commit.as_deref(),
            Some("concurrent-non-status-write")
        );
    }

    /// Pauses a full-record write API between its record read and its
    /// write-back, then commits the admission transition into that gap.
    ///
    /// An unlocked API reads before the gap without holding the attempt lock,
    /// so the admission commit lands inside the gap and the stale write-back
    /// must not overwrite it. A locked API holds the attempt lock across the
    /// gap, so the transition can only commit after the API's own write and
    /// still wins the final state. Either way the persisted record must keep
    /// the admission frozen fields.
    fn full_record_write_gap_preserves_admission_commit(
        invoke: impl FnOnce(CodingAttemptStore, CodingExecutionAttempt) -> Result<(), ProductStoreError>
        + Send
        + 'static,
    ) -> CodingExecutionAttempt {
        let fixture = logical_fixture_with_snapshot();
        let ticket = fixture
            .store
            .admit_attempt_for_execution(&fixture.attempt.id)
            .expect("ticket");
        let attempt_path = fixture
            .store
            .attempt_path(PROJECT_ID, ISSUE_ID, &fixture.attempt.id);
        let (_gap_hook, reached_gap, proceed) = register_attempt_write_gap_hook(&attempt_path);
        let (_lock_hook, lock_attempts) = register_lock_attempt_hook(&attempt_path);
        let store = fixture.store.clone();
        let stale_attempt = fixture.attempt.clone();
        let api_thread = std::thread::spawn(move || invoke(store, stale_attempt));

        reached_gap
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("full-record write reached its read-write gap");
        let api_holds_attempt_lock = lock_attempts.try_iter().count() > 0;

        if api_holds_attempt_lock {
            proceed.send(()).expect("release locked full-record write");
            api_thread
                .join()
                .expect("full-record write thread")
                .expect("full-record write");
            fixture
                .store
                .transition_to_executable(&fixture.attempt.id, &ticket)
                .expect("CAS commit after the locked full-record write");
        } else {
            fixture
                .store
                .transition_to_executable(&fixture.attempt.id, &ticket)
                .expect("CAS commit inside the unlocked read-write gap");
            proceed.send(()).expect("release stale full-record write");
            api_thread
                .join()
                .expect("full-record write thread")
                .expect("full-record write");
        }

        fixture
            .store
            .get_attempt(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("persisted attempt")
    }

    #[test]
    fn head_commit_update_after_admission_gap_preserves_running_cas_commit() {
        let persisted = full_record_write_gap_preserves_admission_commit(|store, stale| {
            store
                .update_attempt_head_commit(
                    &stale.project_id,
                    &stale.issue_id,
                    &stale.id,
                    Some("gap-head-commit".to_string()),
                )
                .map(|_| ())
        });
        assert_eq!(persisted.status, CodingAttemptStatus::Running);
        assert_eq!(persisted.version, 1);
        assert!(persisted.admission_ticket_consumed_at.is_some());
        assert_eq!(persisted.head_commit.as_deref(), Some("gap-head-commit"));
    }

    #[test]
    fn review_request_state_update_after_admission_gap_preserves_running_cas_commit() {
        let persisted = full_record_write_gap_preserves_admission_commit(|store, stale| {
            store
                .update_attempt_review_request_state(
                    &stale.project_id,
                    &stale.issue_id,
                    &stale.id,
                    "gap-review-head".to_string(),
                    "origin".to_string(),
                    "review_request_gap".to_string(),
                )
                .map(|_| ())
        });
        assert_eq!(persisted.status, CodingAttemptStatus::Running);
        assert_eq!(persisted.version, 1);
        assert!(persisted.admission_ticket_consumed_at.is_some());
        assert_eq!(persisted.head_commit.as_deref(), Some("gap-review-head"));
        assert_eq!(persisted.pushed_remote.as_deref(), Some("origin"));
        assert_eq!(
            persisted.review_request_id.as_deref(),
            Some("review_request_gap")
        );
    }

    #[test]
    fn provider_config_snapshot_update_after_admission_gap_preserves_running_cas_commit() {
        let persisted = full_record_write_gap_preserves_admission_commit(|store, stale| {
            store
                .update_attempt_provider_config_snapshot(
                    &stale.project_id,
                    &stale.issue_id,
                    &stale.id,
                    ProviderConfigSnapshot {
                        author: ProviderName::Codex,
                        reviewer: None,
                        review_rounds: 4,
                        permission_modes: Default::default(),
                    },
                )
                .map(|_| ())
        });
        assert_eq!(persisted.status, CodingAttemptStatus::Running);
        assert_eq!(persisted.version, 1);
        assert!(persisted.admission_ticket_consumed_at.is_some());
        assert_eq!(
            persisted.provider_config_snapshot.author,
            ProviderName::Codex
        );
        assert_eq!(persisted.provider_config_snapshot.review_rounds, 4);
    }

    #[test]
    fn rework_count_increment_after_admission_gap_preserves_running_cas_commit() {
        let persisted = full_record_write_gap_preserves_admission_commit(|store, stale| {
            store
                .increment_attempt_rework_count(&stale.project_id, &stale.issue_id, &stale.id)
                .map(|_| ())
        });
        assert_eq!(persisted.status, CodingAttemptStatus::Running);
        assert_eq!(persisted.version, 1);
        assert!(persisted.admission_ticket_consumed_at.is_some());
        assert_eq!(persisted.rework_count, 1);
    }

    #[test]
    fn provider_conversation_replacement_after_admission_gap_preserves_running_cas_commit() {
        let persisted = full_record_write_gap_preserves_admission_commit(|store, stale| {
            store
                .replace_attempt_provider_conversations(
                    &stale,
                    vec![ProviderConversationRef {
                        role: ProviderConversationRole::Coder,
                        provider: ProviderName::Fake,
                        provider_session_id: "gap-session".to_string(),
                        updated_at: "2026-08-11T00:00:00Z".to_string(),
                        last_node_id: None,
                    }],
                )
                .map(|_| ())
        });
        assert_eq!(persisted.status, CodingAttemptStatus::Running);
        assert_eq!(persisted.version, 1);
        assert!(persisted.admission_ticket_consumed_at.is_some());
        assert_eq!(persisted.provider_conversations.len(), 1);
        assert_eq!(
            persisted.provider_conversations[0].provider_session_id,
            "gap-session"
        );
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
