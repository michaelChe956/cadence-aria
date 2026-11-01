use super::*;

#[test]
fn terminal_attempt_restarts_through_explicit_restart_channel() {
    // F-44：Aborted/Failed 经显式 restart 通道（wire 动作 restart_coding）重走
    // admission CAS 回到 Running；终态时间戳在同一锁内清除。
    for terminal in [CodingAttemptStatus::Aborted, CodingAttemptStatus::Failed] {
        let fixture = legacy_fixture();
        let mut seeded = seed_attempt_status(&fixture, terminal.clone());
        seeded.completed_at = Some("2026-09-23T00:00:00Z".to_string());
        fixture
            .store
            .write_coding_attempt_for_test(&seeded)
            .expect("seed terminal record");

        let restarted = fixture
            .store
            .restart_terminal_attempt_for_execution(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect("explicit restart channel");
        assert_eq!(restarted.status, CodingAttemptStatus::Running);
        assert_eq!(
            restarted.completed_at, None,
            "离开终态必须清除终态时间戳（{terminal:?}）"
        );
        assert!(
            restarted.admission_ticket_consumed_at.is_some(),
            "重开 CAS 必须重新锚定 admission 会话 marker（{terminal:?}）"
        );
    }
}

#[test]
fn restart_channel_rejects_non_terminal_sources() {
    for status in [
        CodingAttemptStatus::Created,
        CodingAttemptStatus::Running,
        CodingAttemptStatus::WaitingForHuman,
        CodingAttemptStatus::Blocked,
        CodingAttemptStatus::AwaitingManualRecovery,
    ] {
        let fixture = legacy_fixture();
        seed_attempt_status(&fixture, status.clone());
        let error = fixture
            .store
            .restart_terminal_attempt_for_execution(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect_err("重开通道是终态专属");
        assert!(
            matches!(&error, ProductStoreError::Io(message)
                if message.contains(ATTEMPT_NOT_TERMINAL_FOR_RESTART)),
            "unexpected error for {status:?}: {error:?}"
        );
        assert_eq!(
            fixture
                .store
                .get_attempt(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
                .expect("attempt")
                .status,
            status,
            "被拒的重开不得改写状态（{status:?}）"
        );
    }
}

#[test]
fn general_admission_still_rejects_terminal_attempts() {
    // F-14 零回归钉：终态重开只能走显式通道（wire 动作 restart_coding），
    // 一般 admission（半启动重启 / sc_advance 等自动路径共用入口）对
    // Aborted/Failed 保持 fail-closed 拒绝。
    for terminal in [CodingAttemptStatus::Aborted, CodingAttemptStatus::Failed] {
        let fixture = legacy_fixture();
        seed_attempt_status(&fixture, terminal.clone());
        let error = fixture
            .store
            .admit_and_transition_attempt_to_executable(
                PROJECT_ID,
                ISSUE_ID,
                &fixture.attempt.id,
            )
            .expect_err("general admission must fail closed for terminal attempts");
        assert!(
            matches!(&error, ProductStoreError::Io(message)
                if message.contains("invalid_coding_attempt_status_transition")),
            "unexpected error for {terminal:?}: {error:?}"
        );
        assert_eq!(
            fixture
                .store
                .get_attempt(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
                .expect("attempt")
                .status,
            terminal
        );
    }
}

#[test]
fn recovery_channel_rejects_terminal_sources() {
    // F-16 与 F-44 两条显式通道互不越界：restart 不接受人工恢复态，
    // recover 也不接受终态（各自由 store 入口的状态门保证）。
    for terminal in [CodingAttemptStatus::Aborted, CodingAttemptStatus::Failed] {
        let fixture = legacy_fixture();
        seed_attempt_status(&fixture, terminal.clone());
        let error = fixture
            .store
            .recover_attempt_from_manual_recovery(PROJECT_ID, ISSUE_ID, &fixture.attempt.id)
            .expect_err("终态不得经人工恢复通道重开");
        assert!(matches!(&error, ProductStoreError::Io(message)
            if message.contains("attempt_not_awaiting_manual_recovery")));
    }
}
