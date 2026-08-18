    use std::path::PathBuf;

    use super::*;
    use crate::product::coding_attempt_store::attempt::register_attempt_write_gap_hook;
    use crate::product::coding_attempt_store::locking::register_lock_attempt_hook;
    use crate::product::coding_attempt_store::{CodingAttemptStore, CreateCodingAttemptInput};
    use crate::product::coding_models::{
        AttemptTargetSnapshot, CodingAttemptStatus, CodingExecutionAttempt,
    };
    use crate::product::json_store::{ProductStoreError, read_json, write_json};
    use crate::product::logical_codebase::{
        AggregatePolicyArtifactStore, CheckoutAvailability, CheckoutKind, CodebaseMemberRecord,
        IssueCodebaseSelection, IssueCodebaseSelectionStore, LogicalCodebaseManifest,
        LogicalCodebaseStore, LogicalRepositoryId, MemberStatus, RepositoryCheckoutId,
        RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
    };
    use crate::product::models::{
        ProviderConversationRef, ProviderConversationRole, ProviderName, RepositoryRecord,
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
        assert!(attempt.admission_ticket_consumed_at.is_some());
        assert!(
            !fixture
                .store
                .admission_ticket_path(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
                .exists(),
            "ticket is only an admission credential and is removed after the authoritative attempt commit"
        );
    }

    #[test]
    fn ticket_cleanup_failure_cannot_leave_a_half_committed_transition() {
        let fixture = legacy_fixture();
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
    fn manual_recovery_leaving_running_clears_admission_marker() {
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
        let fixture = legacy_fixture();
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
        let fixture = legacy_fixture();
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

    fn seed_attempt_status(
        fixture: &Fixture,
        status: CodingAttemptStatus,
    ) -> CodingExecutionAttempt {
        let mut attempt = fixture
            .store
            .get_attempt(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("attempt");
        attempt.status = status;
        attempt.admission_ticket_consumed_at = None;
        fixture
            .store
            .write_coding_attempt_for_test(&attempt)
            .expect("seed status");
        attempt
    }

    #[test]
    fn blocked_attempt_cannot_reach_running_through_direct_status_update() {
        let fixture = legacy_fixture();
        seed_attempt_status(&fixture, CodingAttemptStatus::Blocked);
        let error = fixture
            .store
            .update_attempt_status(
                PROJECT_ID,
                ISSUE_ID,
                &fixture.attempt.id,
                CodingAttemptStatus::Running,
            )
            .expect_err("Blocked→Running 直达已删除，必须重走 admission");
        assert!(
            matches!(&error, ProductStoreError::Io(message)
                if message.contains("invalid_coding_attempt_status_transition")),
            "unexpected error: {error:?}"
        );
        let attempt = fixture
            .store
            .get_attempt(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("attempt");
        assert_eq!(attempt.status, CodingAttemptStatus::Blocked);
    }

    #[test]
    fn blocked_attempt_re_enters_running_through_admission_cas() {
        let fixture = legacy_fixture();
        seed_attempt_status(&fixture, CodingAttemptStatus::Blocked);
        let version_before = fixture.attempt.version;

        let ticket = fixture
            .store
            .admit_attempt_for_execution(&fixture.attempt.id)
            .expect("Blocked 恢复必须重走 admission 校验");
        fixture
            .store
            .transition_to_executable(&fixture.attempt.id, &ticket)
            .expect("CAS 消费 ticket 进入 Running");

        let attempt = fixture
            .store
            .get_attempt(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("running attempt");
        assert_eq!(attempt.status, CodingAttemptStatus::Running);
        assert_eq!(attempt.version, version_before + 1);
        assert!(attempt.admission_ticket_consumed_at.is_some());
    }

    #[test]
    fn paused_attempt_resume_cycle_consumes_fresh_admission_ticket() {
        let fixture = legacy_fixture();
        // 第一个 Running 会话：admit → transition。
        let ticket = fixture
            .store
            .admit_attempt_for_execution(&fixture.attempt.id)
            .expect("first ticket");
        fixture
            .store
            .transition_to_executable(&fixture.attempt.id, &ticket)
            .expect("first session");

        // 暂停：Running → WaitingForHuman 结束当前会话。
        fixture
            .store
            .update_attempt_status(
                PROJECT_ID,
                ISSUE_ID,
                &fixture.attempt.id,
                CodingAttemptStatus::WaitingForHuman,
            )
            .expect("pause");

        // 复跑：重新 admit 拿新 ticket 并经 CAS 回到 Running。
        let resume_ticket = fixture
            .store
            .admit_attempt_for_execution(&fixture.attempt.id)
            .expect("resume requires a fresh admission");
        fixture
            .store
            .transition_to_executable(&fixture.attempt.id, &resume_ticket)
            .expect("resume through CAS");

        let attempt = fixture
            .store
            .get_attempt(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("resumed attempt");
        assert_eq!(attempt.status, CodingAttemptStatus::Running);
        assert_eq!(attempt.version, 2);
        assert!(attempt.admission_ticket_consumed_at.is_some());
    }

    #[test]
    fn stale_ticket_replay_after_pause_is_rejected_by_version_cas() {
        let fixture = legacy_fixture();
        let first_ticket = fixture
            .store
            .admit_attempt_for_execution(&fixture.attempt.id)
            .expect("first ticket");
        fixture
            .store
            .transition_to_executable(&fixture.attempt.id, &first_ticket)
            .expect("first session");
        fixture
            .store
            .update_attempt_status(
                PROJECT_ID,
                ISSUE_ID,
                &fixture.attempt.id,
                CodingAttemptStatus::Blocked,
            )
            .expect("pause");
        let paused = fixture
            .store
            .get_attempt(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("paused attempt");
        assert!(
            paused.admission_ticket_consumed_at.is_none(),
            "离开 Running 必须在同一次锁内结束 admission 会话"
        );

        // 暂停后重放上一会话的 ticket：version CAS 必须拒绝。
        assert_eq!(
            fixture
                .store
                .transition_to_executable(&fixture.attempt.id, &first_ticket)
                .expect_err("stale ticket from a finished session"),
            StableCode::AdmissionTicketInvalid
        );
        let attempt = fixture
            .store
            .get_attempt(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("attempt");
        assert_eq!(attempt.status, CodingAttemptStatus::Blocked);
    }

    #[test]
    fn max_auto_rework_update_after_admission_gap_preserves_running_cas_commit() {
        let persisted = full_record_write_gap_preserves_admission_commit(|store, stale| {
            store
                .update_attempt_max_auto_rework(&stale.project_id, &stale.issue_id, &stale.id, 5)
                .map(|_| ())
        });
        assert_eq!(persisted.status, CodingAttemptStatus::Running);
        assert_eq!(persisted.version, 1);
        assert!(persisted.admission_ticket_consumed_at.is_some());
        assert_eq!(persisted.max_auto_rework, 5);
    }

    #[test]
    fn combined_admission_entry_transitions_created_attempt_to_running() {
        let fixture = legacy_fixture();
        let attempt = fixture
            .store
            .admit_and_transition_attempt_to_executable(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("combined admission entry");
        assert_eq!(attempt.status, CodingAttemptStatus::Running);
        assert_eq!(attempt.version, 1);
        assert!(attempt.admission_ticket_consumed_at.is_some());
    }

    #[test]
    fn combined_admission_entry_rejects_attempt_that_is_already_running() {
        let fixture = legacy_fixture();
        fixture
            .store
            .admit_and_transition_attempt_to_executable(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("first entry");
        let error = fixture
            .store
            .admit_and_transition_attempt_to_executable(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect_err("Running 中不得重复进入");
        assert!(
            matches!(&error, ProductStoreError::Io(message)
                if message.contains("invalid_coding_attempt_status_transition")),
            "unexpected error: {error:?}"
        );
    }

    #[test]
    fn combined_admission_entry_fails_closed_for_logical_attempt_without_snapshot() {
        let fixture = logical_fixture();
        let error = fixture
            .store
            .admit_and_transition_attempt_to_executable(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect_err("logical 缺快照必须 fail-closed");
        assert!(
            matches!(error, ProductStoreError::Io(message) if message == TARGET_SNAPSHOT_MISSING_FOR_LOGICAL)
        );
        let attempt = fixture
            .store
            .get_attempt(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("attempt");
        assert_eq!(attempt.status, CodingAttemptStatus::Created);
        assert!(attempt.admission_ticket_consumed_at.is_none());
    }

    #[test]
    fn combined_admission_entry_resumes_applying_plan_amendment_attempt() {
        let fixture = legacy_fixture();
        seed_attempt_status(&fixture, CodingAttemptStatus::ApplyingPlanAmendment);
        let attempt = fixture
            .store
            .admit_and_transition_attempt_to_executable(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("amendment resume through admission");
        assert_eq!(attempt.status, CodingAttemptStatus::Running);
        assert!(attempt.admission_ticket_consumed_at.is_some());
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
                multi_repo: false,
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
