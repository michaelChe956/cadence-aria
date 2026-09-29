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

    #[test]
    fn manual_recovery_attempt_recovers_through_explicit_channel() {
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
                "coding_runner_failed_while_running",
            )
            .expect("quarantine");

        let recovered = fixture
            .store
            .recover_attempt_from_manual_recovery(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("explicit recovery channel");
        assert_eq!(recovered.status, CodingAttemptStatus::Running);
        assert_eq!(recovered.version, 3);
        assert!(
            recovered.admission_ticket_consumed_at.is_some(),
            "恢复 CAS 必须重新锚定 admission 会话 marker"
        );
        assert_eq!(
            recovered.manual_recovery_reason, None,
            "恢复成功必须清除人工恢复 reason（会话结束）"
        );
    }

    #[test]
    fn recovery_channel_rejects_non_manual_recovery_sources() {
        let fixture = legacy_fixture();
        let error = fixture
            .store
            .recover_attempt_from_manual_recovery(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect_err("Created 态无恢复语义");
        assert!(matches!(&error, ProductStoreError::Io(message)
            if message.contains("attempt_not_awaiting_manual_recovery")));

        let ticket = fixture
            .store
            .admit_attempt_for_execution(&fixture.attempt.id)
            .expect("ticket");
        fixture
            .store
            .transition_to_executable(&fixture.attempt.id, &ticket)
            .expect("running");
        let error = fixture
            .store
            .recover_attempt_from_manual_recovery(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect_err("Running 态不得经恢复通道重复进入");
        assert!(matches!(&error, ProductStoreError::Io(message)
            if message.contains("attempt_not_awaiting_manual_recovery")));
    }

    #[test]
    fn general_admission_still_rejects_awaiting_manual_recovery() {
        // F-14 零回归钉：恢复只能走显式通道（wire 动作 recover_coding），
        // 一般 admission（半启动重启 / sc_advance 等自动路径共用入口）对
        // AwaitingManualRecovery 保持 fail-closed 拒绝。
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
            .expect("quarantine");
        let error = fixture
            .store
            .admit_and_transition_attempt_to_executable(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect_err("general admission must fail closed for awaiting_manual_recovery");
        assert!(
            matches!(&error, ProductStoreError::Io(message)
                if message == ATTEMPT_AWAITING_MANUAL_RECOVERY),
            "unexpected error: {error:?}"
        );
        let attempt = fixture
            .store
            .get_attempt(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("attempt");
        assert_eq!(attempt.status, CodingAttemptStatus::AwaitingManualRecovery);
    }

    #[test]
    fn manual_recovery_persists_diagnostic_chat_entry() {
        // 死因可考（§3#5）：人工恢复转换同步落 System/SystemEvent 尾帧
        // （稳定 reason 码 + 原始错误串），durable 层不再零痕迹。
        let fixture = legacy_fixture();
        let diagnostic = fixture
            .store
            .append_manual_recovery_diagnostic(
                &fixture.attempt,
                "coding_runner_failed_while_running",
                "provider spawn failed: exit 127",
            )
            .expect("diagnostic entry");
        assert_eq!(
            diagnostic.id, "coding_manual_recovery_diagnostic_0001",
            "诊断尾帧必须用独立号段前缀（k3 P2），不得占用 coding_chat_entry 号段"
        );
        let entries = fixture
            .store
            .list_chat_entries(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("entries");
        let entry = entries
            .iter()
            .find(|entry| matches!(
                &entry.entry_type,
                CodingEntryType::SystemEvent { event_type, .. }
                    if event_type == "manual_recovery_transition"
            ))
            .expect("manual recovery diagnostic entry");
        assert_eq!(entry.role, CodingAgentRole::System);
        match &entry.entry_type {
            CodingEntryType::SystemEvent { message, .. } => {
                assert_eq!(
                    message,
                    "coding_runner_failed_while_running: provider spawn failed: exit 127"
                );
            }

            other => panic!("unexpected entry type: {other:?}"),
        }
    }

    #[test]
    fn manual_recovery_diagnostic_and_context_note_chat_entries_coexist() {
        // k3 P2 回归钉：诊断尾帧用独立号段（coding_manual_recovery_diagnostic_NNNN），
        // 后到的同号备注（chat entry id 由备注序号派生为同号 coding_chat_entry_NNNN，
        // chat_entry_id_for_context_note）不得 write_json 静默覆盖死因证据——两者共存。
        let fixture = legacy_fixture();
        let diagnostic = fixture
            .store
            .append_manual_recovery_diagnostic(
                &fixture.attempt,
                "coding_runner_failed_while_running",
                "provider spawn failed: exit 127",
            )
            .expect("diagnostic entry");
        assert_eq!(diagnostic.id, "coding_manual_recovery_diagnostic_0001");

        // 备注路径同款形态：首个备注 coding_context_note_0001 → 派生 chat entry
        // coding_chat_entry_0001（同号不同前缀）→ save_chat_entry 落盘。
        let note = fixture
            .store
            .create_context_note(&fixture.attempt, "manual fix".to_string())
            .expect("context note");
        assert_eq!(note.id, "coding_context_note_0001");
        let note_entry_id = note.id.replacen("coding_context_note", "coding_chat_entry", 1);
        assert_eq!(note_entry_id, "coding_chat_entry_0001");
        fixture
            .store
            .save_chat_entry(
                &fixture.attempt,
                &crate::product::coding_models::CodingChatEntry {
                    id: note_entry_id,
                    attempt_id: fixture.attempt.id.clone(),
                    node_id: None,
                    role: CodingAgentRole::Author,
                    entry_type: CodingEntryType::UserMessage,
                    content: Some(note.content.clone()),
                    metadata: Some(serde_json::json!({
                        "context_note_id": note.id,
                    })),
                    created_at: note.created_at,
                },
            )
            .expect("note chat entry");

        let entries = fixture
            .store
            .list_chat_entries(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("entries");
        assert_eq!(entries.len(), 2, "诊断尾帧与备注 entry 必须共存：{entries:#?}");
        assert!(
            entries
                .iter()
                .any(|entry| entry.id == "coding_manual_recovery_diagnostic_0001"),
            "死因尾帧不得被同号备注覆盖"
        );
        assert!(
            entries
                .iter()
                .any(|entry| entry.id == "coding_chat_entry_0001"
                    && entry.entry_type == CodingEntryType::UserMessage),
            "备注 entry 必须正常落盘"
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

    mod admission_restart_tests;

    // ─── C1 Task 6（REQ-WIGA-03）：租约三态判定与确认接管 ───

    fn lease_engine(
        fixture: &Fixture,
    ) -> crate::product::coding_workspace_engine::CodingWorkspaceEngine {
        let (event_tx, _event_rx) =
            tokio::sync::mpsc::channel::<crate::web::coding_ws_handler::CodingWsOutMessage>(1);
        crate::product::coding_workspace_engine::CodingWorkspaceEngine::new(
            fixture.store.clone(),
            crate::product::git_workspace_service::GitWorkspaceService::new(),
            event_tx,
        )
    }

    fn lease_worktree(fixture: &Fixture) -> crate::product::lifecycle_store::LifecycleStore {
        let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(fixture.paths.clone());
        lifecycle
            .upsert_issue_shared_worktree(
                crate::product::lifecycle_store::UpsertIssueSharedWorktreeInput {
                    project_id: PROJECT_ID.to_string(),
                    issue_id: ISSUE_ID.to_string(),
                    repository_id: "repository_0001".to_string(),
                    branch_name: "main".to_string(),
                    worktree_path: std::path::PathBuf::from("/tmp/lease-worktree"),
                    base_branch: "main".to_string(),
                },
            )
            .expect("seed issue shared worktree");
        lifecycle
    }

    fn lease_worktree_and_bind_attempt(fixture: &Fixture) -> crate::product::lifecycle_store::LifecycleStore {
        let lifecycle = lease_worktree(fixture);
        let lease = lifecycle
            .try_acquire_issue_worktree_lock(
                PROJECT_ID,
                ISSUE_ID,
                WORK_ITEM_ID,
                &format!("issue_worktree_lease_{}", fixture.attempt.id),
            )
            .expect("acquire lease");
        assert!(lease.acquired);
        lifecycle
            .bind_issue_worktree_lock_to_attempt(
                PROJECT_ID,
                ISSUE_ID,
                WORK_ITEM_ID,
                &fixture.attempt.id,
            )
            .expect("bind lease to attempt");
        lifecycle
    }

    /// 三态 fail-closed：活跃只等待（第二 acquire 不改 owner/attempt）；
    /// 终态 attempt/已释放 owner → DeadNeedsTakeover（未确认不推进）；
    /// owner 未绑定 attempt、记录缺失 → UnknownNeedsHuman（绝不抢占）。
    #[test]
    fn lease_disposition_active_dead_unknown_is_fail_closed() {
        use crate::product::models::automation::LeaseDisposition;

        // ── 活跃：owner attempt 活跃 → ActiveWait；第二 try_acquire 不改
        //    owner/attempt。
        let fixture = legacy_fixture();
        let lifecycle = lease_worktree_and_bind_attempt(&fixture);
        let engine = lease_engine(&fixture);
        let decision = engine.classify_worktree_lease(PROJECT_ID, ISSUE_ID);
        assert_eq!(decision.disposition, LeaseDisposition::ActiveWait);
        assert_eq!(decision.lease_id, fixture.attempt.id);
        let second = lifecycle
            .try_acquire_issue_worktree_lock(
                PROJECT_ID,
                ISSUE_ID,
                "work_item_0002",
                "issue_worktree_lease_second",
            )
            .expect_err("second work item cannot steal an active lease");
        assert!(second.to_string().contains("issue_worktree_active"));
        let record = lifecycle
            .get_issue_shared_worktree(PROJECT_ID, ISSUE_ID)
            .expect("record")
            .expect("present");
        assert_eq!(record.current_lock_owner_id.as_deref(), Some(fixture.attempt.id.as_str()));
        assert_eq!(
            record.current_active_work_item_id.as_deref(),
            Some(WORK_ITEM_ID)
        );

        // ── 死亡（终态 attempt）：owner attempt Failed → DeadNeedsTakeover；
        //    判定只读，未确认不推进。
        seed_attempt_status(&fixture, CodingAttemptStatus::Failed);
        let decision = engine.classify_worktree_lease(PROJECT_ID, ISSUE_ID);
        assert_eq!(decision.disposition, LeaseDisposition::DeadNeedsTakeover);
        assert_eq!(decision.lease_id, fixture.attempt.id);
        let record = lifecycle
            .get_issue_shared_worktree(PROJECT_ID, ISSUE_ID)
            .expect("record")
            .expect("present");
        assert_eq!(record.current_lock_owner_id.as_deref(), Some(fixture.attempt.id.as_str()));

        // ── 死亡（已明确释放 owner）：owner/active 清空 → DeadNeedsTakeover。
        lifecycle
            .release_issue_worktree_lock_by_owner(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("release by owner");
        let decision = engine.classify_worktree_lease(PROJECT_ID, ISSUE_ID);
        assert_eq!(decision.disposition, LeaseDisposition::DeadNeedsTakeover);

        // ── 未知（owner 是未绑定 attempt 的瞬态 lease）→ 绝不抢占。
        let fixture = legacy_fixture();
        let lifecycle = lease_worktree(&fixture);
        lifecycle
            .try_acquire_issue_worktree_lock(
                PROJECT_ID,
                ISSUE_ID,
                WORK_ITEM_ID,
                "issue_worktree_lease_transient",
            )
            .expect("acquire transient lease");
        let decision = lease_engine(&fixture).classify_worktree_lease(PROJECT_ID, ISSUE_ID);
        assert_eq!(decision.disposition, LeaseDisposition::UnknownNeedsHuman);

        // ── 未知（记录缺失）：无 worktree 证据 → UnknownNeedsHuman。
        let fixture = legacy_fixture();
        let decision = lease_engine(&fixture).classify_worktree_lease(PROJECT_ID, ISSUE_ID);
        assert_eq!(decision.disposition, LeaseDisposition::UnknownNeedsHuman);
    }

    fn lease_enrollment_binding(fixture: &Fixture) -> crate::product::models::automation::EnrollmentBindingIdentity {
        use crate::product::issue_automation_store::IssueAutomationStore;
        use crate::product::models::automation::{
            EnrollmentOptions, EnrollmentSource, EnrollmentWriteCommand, SourceRevisionRef,
        };

        let store = IssueAutomationStore::new(fixture.paths.clone());
        store
            .compare_and_set(
                PROJECT_ID,
                ISSUE_ID,
                None,
                EnrollmentWriteCommand::Enable {
                    selection_key: "selection_0001".to_string(),
                    source: EnrollmentSource {
                        stories: vec![SourceRevisionRef {
                            id: "story_spec_0001".to_string(),
                            version: 1,
                        }],
                        designs: vec![],
                    },
                    options: EnrollmentOptions {
                        author_provider: ProviderName::Fake,
                        reviewer_provider: ProviderName::Fake,
                        review_rounds: 1,
                        superpowers_enabled: false,
                        openspec_enabled: false,
                        plan_options: crate::product::models::IssueWorkItemPlanOptions {
                            include_integration_tests: true,
                            include_e2e_tests: false,
                            force_frontend_backend_split: false,
                            require_execution_plan_confirm: false,
                        },
                    },
                    target: crate::product::logical_codebase::EnrollmentTarget::SingleRepository {
                        repository_id: "repository_0001".to_string(),
                    },
                },
            )
            .expect("enable enrollment");
        store
            .get(PROJECT_ID, ISSUE_ID)
            .expect("read enrollment")
            .expect("present")
            .binding_history
            .expect("binding history")
            .current
    }

    /// 确认接管：死亡 owner 在 worktree 文件锁内 CAS 清出（新 acquire 成为
    /// 新 owner）；活跃/未知不写任何文件；迟到旧 lease/旧 binding 写入
    /// IdentityMismatch 且 current owner 不变；并发确认只有一个成功；
    /// 同 command 重放返回首次 durable 结果。
    #[test]
    fn lease_takeover_confirm_writes_owner_with_cas_and_rejects_stale_writes() {
        use crate::product::json_store::ProductStoreError;
        use crate::product::models::automation::{
            LeaseTakeoverRequest, LeaseTakeoverResult, OperationState,
        };

        let fixture = legacy_fixture();
        let lifecycle = lease_worktree_and_bind_attempt(&fixture);
        seed_attempt_status(&fixture, CodingAttemptStatus::Failed);
        let binding = lease_enrollment_binding(&fixture);
        let engine = lease_engine(&fixture);

        let request = LeaseTakeoverRequest {
            command_id: "lease_takeover_cmd_0001".to_string(),
            expected_binding: binding.clone(),
            expected_lease_id: fixture.attempt.id.clone(),
            expected_attempt_id: fixture.attempt.id.clone(),
        };

        // 活跃租约拒绝接管（先复活 attempt 验证 Rejected 不写文件）。
        {
            let mut revived = fixture
                .store
                .get_attempt(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
                .expect("attempt");
            revived.status = CodingAttemptStatus::Running;
            fixture
                .store
                .write_coding_attempt_for_test(&revived)
                .expect("revive");
            let result = engine
                .confirm_takeover(PROJECT_ID, ISSUE_ID, &request)
                .expect("classified without error");
            assert_eq!(result.state, OperationState::Rejected);
            let record = lifecycle
                .get_issue_shared_worktree(PROJECT_ID, ISSUE_ID)
                .expect("record")
                .expect("present");
            assert_eq!(
                record.current_lock_owner_id.as_deref(),
                Some(fixture.attempt.id.as_str())
            );
            seed_attempt_status(&fixture, CodingAttemptStatus::Failed);
        }

        // 合法接管：Accepted，死亡 owner 原子清出，下一次 acquire 成为
        // 新 owner（不建第二 attempt）。
        let LeaseTakeoverResult { command_id, state, lease } = engine
            .confirm_takeover(PROJECT_ID, ISSUE_ID, &request)
            .expect("takeover accepted");
        assert_eq!(command_id, "lease_takeover_cmd_0001");
        assert_eq!(state, OperationState::Accepted);
        assert!(lease
            .evidence
            .iter()
            .any(|line| line.contains("takeover_confirmed")));
        let record = lifecycle
            .get_issue_shared_worktree(PROJECT_ID, ISSUE_ID)
            .expect("record")
            .expect("present");
        assert_eq!(record.current_lock_owner_id, None);
        assert_eq!(record.current_active_work_item_id, None);
        let next = lifecycle
            .try_acquire_issue_worktree_lock(
                PROJECT_ID,
                ISSUE_ID,
                WORK_ITEM_ID,
                "issue_worktree_lease_next",
            )
            .expect("next owner acquires after takeover");
        assert!(next.acquired);

        // 同 command 同 payload 重放：Replayed（首次 durable 结果）。
        lifecycle
            .release_issue_worktree_lock_by_owner(PROJECT_ID, ISSUE_ID, "issue_worktree_lease_next")
            .expect("release next");
        let replay = engine
            .confirm_takeover(PROJECT_ID, ISSUE_ID, &request)
            .expect("replay");
        assert_eq!(replay.state, OperationState::Replayed);

        // 迟到旧 lease 写入：owner 已换代 → IdentityMismatch，current
        // owner/attempt 不变。
        let stale = lifecycle
            .takeover_issue_worktree_lock_after_dead_owner(
                PROJECT_ID,
                ISSUE_ID,
                &fixture.attempt.id,
            )
            .expect_err("stale owner write rejected");
        assert!(matches!(
            stale,
            ProductStoreError::IdentityMismatch { kind: "issue_worktree_lock_takeover", .. }
        ));

        // 过期 binding 拒绝：expected binding 漂移 → IdentityMismatch。
        let mut drifted = binding.clone();
        drifted.binding_version += 1;
        let drifted_request = LeaseTakeoverRequest {
            command_id: "lease_takeover_cmd_0002".to_string(),
            expected_binding: drifted,
            expected_lease_id: fixture.attempt.id.clone(),
            expected_attempt_id: fixture.attempt.id.clone(),
        };
        let error = engine
            .confirm_takeover(PROJECT_ID, ISSUE_ID, &drifted_request)
            .expect_err("drifted binding rejected");
        assert!(matches!(
            error,
            ProductStoreError::IdentityMismatch { kind: "enrollment_binding", .. }
        ));
    }

    // ─── C2 Task 2（REQ-CRO-02）：最小编码互斥、接管判别与 attempt 命令账本 ───

    /// 第二个 kick 撞上活跃租约（另一活跃 attempt 持有）：admission 必须
    /// `AlreadyRunning`（通知"已在运行／请等待"），不抢租约、不启动 provider；
    /// 命令账本记录首次 durable 结果，同 command 同 payload 重放同一结论，
    /// 同 command 异 payload fail-closed。
    #[test]
    fn second_kick_on_active_lease_returns_already_running() {
        use crate::product::coding_attempt_store::CodingRunExclusionDecision;
        use crate::product::models::automation::LeaseDisposition;

        let fixture = legacy_fixture();
        // 第二个 kick 的目标：同 issue 下另一 attempt B。B 必须在 A 活跃前
        // 创建（active_coding_attempt 不变式），创建后停留在 Created——双 kick
        // 撞活跃租约的现场形态。
        let attempt_b = fixture
            .store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: "work_item_0002".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/attempt-b".to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: None,
                max_auto_rework: 0,
            })
            .unwrap();
        // 活跃持有者：attempt A（Running）绑定 issue worktree 租约。
        seed_attempt_status(&fixture, CodingAttemptStatus::Running);
        let lifecycle = lease_worktree_and_bind_attempt(&fixture);
        let decision = fixture
            .store
            .admit_coding_run_exclusive(&attempt_b, "cmd-kick-b-1", "digest-b-1")
            .expect("admission must classify instead of failing");
        match &decision {
            CodingRunExclusionDecision::AlreadyRunning { lease } => {
                assert_eq!(lease.disposition, LeaseDisposition::ActiveWait);
                assert_eq!(lease.lease_id, fixture.attempt.id);
            }
            other => panic!("expected AlreadyRunning, got {other:?}"),
        }
        // 租约事实未被抢占：owner 仍是 attempt A。
        let record = lifecycle
            .get_issue_shared_worktree(PROJECT_ID, ISSUE_ID)
            .expect("record")
            .expect("present");
        assert_eq!(
            record.current_lock_owner_id.as_deref(),
            Some(fixture.attempt.id.as_str())
        );
        // 不启动 provider：attempt B 无任何 role run。
        assert!(
            fixture
                .store
                .list_role_runs(PROJECT_ID, ISSUE_ID, &attempt_b.id)
                .expect("role runs")
                .is_empty()
        );
        // 命令账本已记录首次 durable 结果（NeedsHuman 停等）。
        let recorded = fixture
            .store
            .find_attempt_command_result(PROJECT_ID, ISSUE_ID, &attempt_b.id, "cmd-kick-b-1")
            .expect("ledger read")
            .expect("ledger entry recorded");
        assert_eq!(
            recorded,
            crate::product::coding_attempt_store::CodingAttemptCommandRecord {
                command_id: "cmd-kick-b-1".to_string(),
                payload_digest: "digest-b-1".to_string(),
                state: crate::product::models::automation::OperationState::NeedsHuman,
                recorded_at: recorded.recorded_at.clone(),
            }
        );
        // 同 command 同 payload 重放首次 durable 结果（仍 AlreadyRunning）。
        let replay = fixture
            .store
            .admit_coding_run_exclusive(&attempt_b, "cmd-kick-b-1", "digest-b-1")
            .expect("replay must not fail");
        assert!(matches!(
            replay,
            CodingRunExclusionDecision::AlreadyRunning { .. }
        ));
        // 同 command 异 payload fail-closed（请刷新）。
        let conflict = fixture
            .store
            .admit_coding_run_exclusive(&attempt_b, "cmd-kick-b-1", "digest-b-OTHER")
            .expect_err("payload drift must fail closed");
        assert!(matches!(
            conflict,
            crate::product::json_store::ProductStoreError::Conflict { .. }
        ));
    }

    /// 死亡租约 → `TakeoverRequired`（未确认不继续）；接管清出后同请求
    /// 继续 → `Allowed`；证据缺失/瞬态 owner → `LeaseUnknown` 停等。
    #[test]
    fn coding_run_admission_takeover_and_unknown_lease() {
        use crate::product::coding_attempt_store::CodingRunExclusionDecision;
        use crate::product::models::automation::LeaseDisposition;

        // ── 死亡（owner attempt 终态）：TakeoverRequired，未确认不继续。
        let fixture = legacy_fixture();
        let attempt_b = fixture
            .store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: "work_item_0002".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/attempt-b".to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: None,
                max_auto_rework: 0,
            })
            .unwrap();
        seed_attempt_status(&fixture, CodingAttemptStatus::Running);
        let lifecycle = lease_worktree_and_bind_attempt(&fixture);
        // 持有者 attempt A 转终态 → 租约死亡。
        seed_attempt_status(&fixture, CodingAttemptStatus::Failed);
        let decision = fixture
            .store
            .admit_coding_run_exclusive(&attempt_b, "cmd-takeover-1", "digest-t-1")
            .expect("classify");
        match &decision {
            CodingRunExclusionDecision::TakeoverRequired { lease } => {
                assert_eq!(lease.disposition, LeaseDisposition::DeadNeedsTakeover);
            }
            other => panic!("expected TakeoverRequired, got {other:?}"),
        }
        // 用户确认接管（复用 C1 owner CAS 清出）后，同请求继续 → Allowed。
        lifecycle
            .release_issue_worktree_lock_by_owner(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("takeover cleared the dead owner");
        let decision = fixture
            .store
            .admit_coding_run_exclusive(&attempt_b, "cmd-takeover-1", "digest-t-1")
            .expect("classify after takeover");
        assert!(matches!(
            decision,
            CodingRunExclusionDecision::Allowed(ref admitted)
                if admitted.id == attempt_b.id
        ));

        // ── 未知（瞬态 lease owner，未绑定 attempt）：LeaseUnknown 停等。
        let fixture = legacy_fixture();
        let attempt_b = fixture
            .store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: "work_item_0002".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/attempt-b2".to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: None,
                max_auto_rework: 0,
            })
            .unwrap();
        seed_attempt_status(&fixture, CodingAttemptStatus::Running);
        let lifecycle = lease_worktree(&fixture);
        lifecycle
            .try_acquire_issue_worktree_lock(
                PROJECT_ID,
                ISSUE_ID,
                WORK_ITEM_ID,
                "issue_worktree_lease_transient",
            )
            .expect("acquire transient lease");
        let decision = fixture
            .store
            .admit_coding_run_exclusive(&attempt_b, "cmd-unknown-1", "digest-u-1")
            .expect("classify");
        match &decision {
            CodingRunExclusionDecision::LeaseUnknown { lease } => {
                assert_eq!(lease.disposition, LeaseDisposition::UnknownNeedsHuman);
            }
            other => panic!("expected LeaseUnknown, got {other:?}"),
        }
        // 无租约事实（未启动过 worktree 锁）不得拦截既有链路：自持活跃租约的
        // 阶段续跑（self re-entry）照常 Allowed。
        let fixture = legacy_fixture();
        let running = fixture
            .store
            .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .unwrap();
        let decision = fixture
            .store
            .admit_coding_run_exclusive(&running, "cmd-self-1", "digest-s-1")
            .expect("no lease facts must not block legacy attempts");
        assert!(matches!(
            decision,
            CodingRunExclusionDecision::Allowed(ref admitted) if admitted.id == running.id
        ));
        // 自持活跃租约（owner == 本 attempt）也是合法续跑。
        lease_worktree_and_bind_attempt(&fixture);
        let decision = fixture
            .store
            .admit_coding_run_exclusive(&running, "cmd-self-2", "digest-s-2")
            .expect("self-held active lease is legitimate re-entry");
        assert!(matches!(
            decision,
            CodingRunExclusionDecision::Allowed(ref admitted) if admitted.id == running.id
        ));
    }
