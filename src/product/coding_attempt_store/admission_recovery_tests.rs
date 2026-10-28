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
