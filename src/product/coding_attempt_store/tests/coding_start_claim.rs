// 从 tests.rs 拆出以满足 large_file_guard 的 1200 行上限（纯移动，无行为变化）。

use super::*;

// ------------------------------------------------------------------
// P2 Task 4：attempt 同文件 durable 单发首启 claim（不可复位身份）。
// ------------------------------------------------------------------

#[test]
fn coding_start_claim_is_single_shot_and_frozen_against_updates() {
    let (_tmp, store, attempt) = setup();
    // 旧 JSON 缺 start_claim 字段反序列化为 None。
    let mut json = serde_json::to_value(&attempt).unwrap();
    json.as_object_mut().unwrap().remove("start_claim");
    let old: CodingExecutionAttempt = serde_json::from_value(json).unwrap();
    assert!(old.start_claim.is_none());

    let origin = crate::product::coding_models::CodingStartOrigin::Manual;
    let first = store
        .claim_coding_start(&attempt, "start-command-a", &origin)
        .expect("first claim");
    let ClaimCodingStartOutcome::Claimed(claimed) = first else {
        panic!("first claim must create the durable claim");
    };
    let claim = claimed.start_claim.as_ref().expect("durable claim");
    assert_eq!(claim.command_id, "start-command-a");
    assert_eq!(claim.origin, origin);
    assert_eq!(claim.phase, CodingStartPhase::Claimed);

    // 同 command 重放与异 command 竞争都返回 Existing 原身份，绝不覆写。
    for command_id in ["start-command-a", "start-command-b"] {
        match store
            .claim_coding_start(&attempt, command_id, &origin)
            .expect("repeat claim read")
        {
            ClaimCodingStartOutcome::Existing(saved) => {
                let existing = saved.start_claim.as_ref().expect("existing claim");
                assert_eq!(existing.command_id, "start-command-a");
                assert_eq!(existing.phase, CodingStartPhase::Claimed);
            }
            ClaimCodingStartOutcome::Claimed(_) => {
                panic!("claim must be single-shot for command {command_id}")
            }
        }
    }

    // copy-update 保留冻结 claim（与 status/admission 同款不可覆写）。
    let mut replacement = store
        .get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id)
        .unwrap();
    replacement.start_claim = None;
    store
        .update_attempt_non_status_fields(&replacement)
        .unwrap();
    let stored = store
        .get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id)
        .unwrap();
    assert_eq!(
        stored.start_claim.as_ref().map(|claim| claim.command_id.as_str()),
        Some("start-command-a")
    );
}

#[test]
fn coding_start_phase_advances_only_for_claimed_command() {
    let (_tmp, store, attempt) = setup();
    let origin = crate::product::coding_models::CodingStartOrigin::Manual;
    let ClaimCodingStartOutcome::Claimed(claimed) = store
        .claim_coding_start(&attempt, "start-command-a", &origin)
        .expect("first claim")
    else {
        panic!("first claim must succeed");
    };
    assert!(store
        .advance_coding_start_phase(
            &claimed,
            "start-command-b",
            CodingStartPhase::RunnerRegistered,
        )
        .is_err());
    let advanced = store
        .advance_coding_start_phase(
            &claimed,
            "start-command-a",
            CodingStartPhase::RunnerRegistered,
        )
        .expect("advance phase");
    assert_eq!(
        advanced.start_claim.as_ref().unwrap().phase,
        CodingStartPhase::RunnerRegistered
    );
}

#[test]
fn coding_start_claim_rejects_non_created_attempt_without_existing_claim() {
    let (_tmp, store, attempt) = setup();
    let mut running = attempt.clone();
    running.status = CodingAttemptStatus::Running;
    store.write_coding_attempt_for_test(&running).unwrap();
    let result =
        store.claim_coding_start(&running, "start-command-a", &CodingStartOrigin::Manual);
    assert!(result.is_err(), "claim must fail-closed off the first-start state");
}
