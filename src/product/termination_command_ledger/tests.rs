use super::{
    TerminationLedgerVerdict, record_termination_ledger_accept, termination_intent_digest,
    termination_ledger_verdict,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::issue_automation_store::tests::bound_enrollment_with_binding;
use crate::product::json_store::{read_json, write_json};
use crate::product::models::automation::{
    EnrollmentCommandResult, EnrollmentTerminationRequest, IssueAutomationEnrollment,
    OperationState,
};

fn request(
    command_id: &str,
    enrollment_id: &str,
    revision: u64,
    reason: &str,
) -> EnrollmentTerminationRequest {
    EnrollmentTerminationRequest {
        command_id: command_id.to_string(),
        expected_enrollment_id: enrollment_id.to_string(),
        expected_policy_revision: revision,
        reason: reason.to_string(),
    }
}

fn ledger_entry(command_id: &str, digest: &str) -> EnrollmentCommandResult {
    EnrollmentCommandResult {
        command_id: command_id.to_string(),
        payload_digest: digest.to_string(),
        state: OperationState::Accepted,
        binding_version: 1,
    }
}

/// owner 2026-10-26 真机根修红测（纯判定）：同一终止命令仅
/// `expected_policy_revision` 刷新（服务端 409 details 指引的自愈重试）必须
/// 仍被判为**同一条命令**——若把乐观前置条件计入身份，该重试被判异 payload，
/// 终止入口永久 409。
#[test]
fn termination_verdict_keeps_refreshed_precondition_the_same_command() {
    let first = request("cmd-fe-terminate-1", "enrollment_1", 26, "user terminate");
    let retried = request("cmd-fe-terminate-1", "enrollment_1", 29, "user terminate");
    assert_ne!(
        first.payload_digest(),
        retried.payload_digest(),
        "全量 payload 摘要必然不同（revision 参与）"
    );
    assert_eq!(
        termination_intent_digest(&first),
        termination_intent_digest(&retried),
        "intent 不含乐观前置条件"
    );

    let entry = ledger_entry("cmd-fe-terminate-1", &termination_intent_digest(&first));
    assert_eq!(
        termination_ledger_verdict(&entry, &retried),
        TerminationLedgerVerdict::SameCommand
    );
}

/// 新口径条目 + 不同 reason（真实异 intent）→ fail-closed，不猜哪个是真的。
#[test]
fn termination_verdict_rejects_divergent_intent_for_new_entries() {
    let first = request("cmd-fe-terminate-1", "enrollment_1", 26, "user terminate");
    let divergent = request("cmd-fe-terminate-1", "enrollment_1", 29, "another intent");
    let entry = ledger_entry("cmd-fe-terminate-1", &termination_intent_digest(&first));
    assert_eq!(
        termination_ledger_verdict(&entry, &divergent),
        TerminationLedgerVerdict::Divergent
    );
}

/// 修复前落盘的旧口径条目（含 revision 的 `sha256:` 摘要）：字节全等重放，
/// 或仅前置条件漂移时按同命令续做（否则历史现场永久死锁）。
#[test]
fn termination_verdict_accepts_legacy_digest_entries() {
    let legacy_digest =
        request("cmd-fe-terminate-1", "enrollment_1", 26, "user terminate").payload_digest();
    let entry = ledger_entry("cmd-fe-terminate-1", &legacy_digest);

    let identical = request("cmd-fe-terminate-1", "enrollment_1", 26, "user terminate");
    assert_eq!(
        termination_ledger_verdict(&entry, &identical),
        TerminationLedgerVerdict::SameCommand
    );
    let retried = request("cmd-fe-terminate-1", "enrollment_1", 29, "user terminate");
    assert_eq!(
        termination_ledger_verdict(&entry, &retried),
        TerminationLedgerVerdict::SameCommand
    );
}

/// accepted 落盘只保留一条命令条目：同 `command_id` 就地升级为 intent 摘要，
/// 不追加重复项（账本按 `command_id` 查找）。
#[test]
fn record_termination_accept_upserts_one_entry_per_command() {
    let accept = request("cmd-fe-terminate-1", "enrollment_1", 26, "user terminate");
    let mut ledger = vec![ledger_entry("cmd-fe-terminate-1", "sha256:legacy")];
    record_termination_ledger_accept(&mut ledger, &accept, 3);
    assert_eq!(ledger.len(), 1);
    assert_eq!(ledger[0].payload_digest, termination_intent_digest(&accept));
    assert_eq!(ledger[0].binding_version, 3);
}

/// 历史现场端到端：旧口径账本条目的终止命令，在 revision 漂移后重提必须回报
/// 首次 durable 结果（Replayed，零写入、revision 不涨），而不是 409。
#[test]
fn store_terminate_retry_with_refreshed_revision_replays_legacy_entry() {
    let (_tmp, store, _binding, revision) = bound_enrollment_with_binding();
    let enrollment_id = store
        .get("project_1", "issue_1")
        .unwrap()
        .expect("enrollment exists")
        .enrollment_id;
    let first = request(
        "cmd-fe-terminate-1",
        &enrollment_id,
        revision,
        "user terminate",
    );
    let terminated = store
        .terminate_plan("project_1", "issue_1", first.clone())
        .expect("first terminate applies");
    let terminated_at = terminated.terminated_at.clone();

    // 退化修复前口径：把该条目摘要改写为含 revision 的旧摘要（模拟历史落盘）。
    let path = ProductAppPaths::new(_tmp.path().to_path_buf())
        .issue_root("project_1", "issue_1")
        .join("automation-enrollment.json");
    let mut saved: IssueAutomationEnrollment = read_json(&path).unwrap();
    let entry = saved
        .command_ledger
        .iter_mut()
        .find(|entry| entry.command_id == "cmd-fe-terminate-1")
        .expect("ledger entry");
    entry.payload_digest = first.payload_digest();
    write_json(&path, &saved).unwrap();

    let retried = request(
        "cmd-fe-terminate-1",
        &enrollment_id,
        terminated.enrollment.policy_revision,
        "user terminate",
    );
    let replay = store
        .terminate_plan("project_1", "issue_1", retried)
        .expect("refreshed-precondition retry must converge, not 409");
    assert_eq!(replay.state, OperationState::Replayed);
    assert_eq!(replay.terminated_at, terminated_at);
    let after = store.get("project_1", "issue_1").unwrap().unwrap();
    assert_eq!(
        after.policy_revision, terminated.enrollment.policy_revision,
        "replay must not bump revision"
    );
    assert_eq!(after.command_ledger, saved.command_ledger, "零写入");
}
