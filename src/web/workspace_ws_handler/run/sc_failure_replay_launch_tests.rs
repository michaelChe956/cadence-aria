//! P08 replay-safe 分类红测（二）：审计启动事实 → `Waiting`（含教学轮 ordinal ≥ 1）。
//!
//! **RED 面**：与（一）同一缺失 API（`ScRoundEntry` 及其 `Waiting{reason_code,
//! detail}` 分支、`SC_ROUND_LAUNCH_UNKNOWN` / `SC_ROUND_LAUNCH_STATE_UNAVAILABLE`）。
//! 本文件只断言「不得 Ready、不得第二 start、不得 Err」三类事实：审计已有真实
//! `provider_start`、分区损坏、run 记录读不出。fixture/计数复用父模块 `tests`。
//!
//! 不伪造 provider 启动：`provider_start` 一律经真实审计 sink（`LifecycleStore`
//! 的 `ToolPolicyAuditSink::append`，与 adapter 的 run-bound sink 同一准入）。

use super::replay_completed_tests::{bind_original, classify, evidence_rows};
use super::tests::actual_starts;
use super::*;
use crate::cross_cutting::tool_policy_audit::{
    DurableToolPolicyEvent, ProviderStartAudit, ToolPolicyAuditSink,
};

const NODE_ID: &str = "node_sc_0001";
const BUSINESS_KEY_FOR_REPLAY: &str = "workspace_author:session:0";

fn audited_start(lifecycle: &LifecycleStore, session_id: &str, role_run_seq: u64) {
    lifecycle
        .append(
            session_id,
            role_run_seq,
            DurableToolPolicyEvent::ProviderStart(ProviderStartAudit {
                provider: "fake".to_string(),
                ..ProviderStartAudit::default()
            }),
        )
        .expect("real provider start audit line");
}

/// 按 `role_run_seq` 在应用路径下定位审计分区文件（目录布局不由测试 pin）。
fn find_partition_by_seq(root: &std::path::Path, seq: u64) -> Option<std::path::PathBuf> {
    let target = format!("{seq}.jsonl");
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.file_name().is_some_and(|name| name == target.as_str()) {
                return Some(path);
            }
        }
    }
    None
}

// 行 3：audited provider_start + running + 无交付 ⇒ Waiting（不是 Err/Failed），
// 且不产生第二 start、不新增证据行、分区仍只有一条真实 provider_start。
#[test]
fn classifier_waiting_when_audited_provider_start_exists_for_same_seq() {
    let (_tmp, lifecycle, scope) = super::tests::sc_round_fixture();
    let bound = bind_original(&lifecycle, &scope);
    audited_start(&lifecycle, &scope.session_id, bound.round.role_run_seq);
    let rows_before = evidence_rows(&lifecycle, &bound.chain);
    let starts_before = actual_starts(&lifecycle, &scope.session_id);

    match classify(&lifecycle, &scope, None, 0) {
        ScRoundEntry::Waiting {
            round: replay,
            reason_code,
            detail,
        } => {
            assert_eq!(reason_code, SC_ROUND_LAUNCH_UNKNOWN);
            assert!(!detail.is_empty());
            assert_eq!(replay.role_run_seq, bound.round.role_run_seq);
        }
        _ => panic!("audited start without delivery must classify Waiting"),
    }
    assert_eq!(actual_starts(&lifecycle, &scope.session_id), starts_before);
    assert_eq!(evidence_rows(&lifecycle, &bound.chain), rows_before);
    let lines = lifecycle
        .read_tool_policy_lines(&scope.session_id, bound.round.role_run_seq)
        .expect("audit partition readable");
    assert_eq!(
        lines
            .iter()
            .filter(|line| line.event_type() == "provider_start")
            .count(),
        1,
        "classification never appends a second start line"
    );
}

// 行 4a：审计分区损坏 ⇒ 不可证明 ⇒ Waiting(state_unavailable)，绝不 Ready。
#[test]
fn classifier_waiting_when_audit_partition_is_corrupt() {
    let (tmp, lifecycle, scope) = super::tests::sc_round_fixture();
    let bound = bind_original(&lifecycle, &scope);
    audited_start(&lifecycle, &scope.session_id, bound.round.role_run_seq);
    let partition = find_partition_by_seq(tmp.path(), bound.round.role_run_seq)
        .expect("audit partition exists after a real append");
    std::fs::write(&partition, "{not-json\n").expect("corrupt partition");

    match classify(&lifecycle, &scope, None, 0) {
        ScRoundEntry::Waiting {
            reason_code, detail, ..
        } => {
            assert_eq!(reason_code, SC_ROUND_LAUNCH_STATE_UNAVAILABLE);
            assert!(!detail.is_empty());
        }
        _ => panic!("unreadable audit partition must never classify Ready"),
    }
    assert_eq!(actual_starts(&lifecycle, &scope.session_id), 0);
}

// 行 4b：run 记录读不出（身份不可读）⇒ Waiting(state_unavailable)，不是 Ready。
#[test]
fn classifier_waiting_when_run_record_is_missing() {
    let (_tmp, lifecycle, scope) = super::tests::sc_round_fixture();
    let bound = bind_original(&lifecycle, &scope);
    let handle = WorkItemSplitProviderRunHandle {
        run_ref: bound.round.run_ref.clone(),
        workspace_session_id: bound.round.workspace_session_id.clone(),
        role_run_seq: bound.round.role_run_seq,
    };
    let run_json = lifecycle
        .work_item_split_provider_run_dir(&handle)
        .expect("run dir")
        .join("run.json");
    std::fs::remove_file(&run_json).expect("remove run record");

    match classify(&lifecycle, &scope, None, 0) {
        ScRoundEntry::Waiting { reason_code, .. } => {
            assert_eq!(reason_code, SC_ROUND_LAUNCH_STATE_UNAVAILABLE);
        }
        _ => panic!("unreadable run record must classify Waiting"),
    }
    assert_eq!(actual_starts(&lifecycle, &scope.session_id), 0);
}

// 行 7：教学轮（ordinal ≥ 1）三类分支各自成立，且不额外 claim/发送。
#[test]
fn teaching_round_classifies_ready_completed_and_waiting() {
    let (_tmp, lifecycle, scope) = super::tests::sc_round_fixture();
    let original = bind_original(&lifecycle, &scope);
    let previous = (
        ScEvidenceRef::Missing {
            code: "sc_failure_raw_missing".to_string(),
            detail: "not yet delivered".to_string(),
            round_id: original.round.round_id.clone(),
        },
        original.round.clone(),
    );

    // not started ⇒ Ready（沿用原链，新物理 handle）。
    let teaching = match begin_sc_teaching_round(
        &lifecycle,
        &original,
        Some(&previous),
        NODE_ID,
        &ProviderName::Fake,
    )
    .expect("teaching round binds")
    {
        ScRoundEntry::Ready(bound) => bound,
        _ => panic!("un-started teaching round must classify Ready"),
    };
    assert_eq!(teaching.round.round_ordinal, 1);
    assert_eq!(teaching.chain.chain_id, original.chain.chain_id);
    assert_ne!(teaching.round.role_run_seq, original.round.role_run_seq);
    assert_eq!(actual_starts(&lifecycle, &scope.session_id), 0);

    // completed ⇒ Completed（逐字 raw），仍零 start。
    let raw = "教学轮完整修订版：中文正文";
    close_sc_split_round(&lifecycle, &teaching, "teaching prompt", raw).expect("strict close");
    // 重放同一教学轮（ordinal 1）：`previous` 仍是原轮（教学轮序号由当前轮推进，
    // 传教学轮自己会推进到 ordinal 2 = 新物理轮，不是重放）。
    let previous_teaching = (
        ScEvidenceRef::Available {
            locator: String::new(),
            sha256: String::new(),
            bytes: raw.len() as u64,
        },
        original.round.clone(),
    );
    match begin_sc_teaching_round(
        &lifecycle,
        &original,
        Some(&previous_teaching),
        NODE_ID,
        &ProviderName::Fake,
    )
    .expect("teaching replay resolves")
    {
        ScRoundEntry::Completed { raw_text, .. } => assert_eq!(raw_text, raw),
        _ => panic!("completed teaching round must classify Completed"),
    }
    assert_eq!(actual_starts(&lifecycle, &scope.session_id), 0);

    // audited start + no delivery ⇒ Waiting（不落 Failed、不 err）。
    // ordinal 2 = 教学轮(1) 的下一轮：用教学轮身份推进入参，不是再重放 ordinal 1。
    let third = match begin_sc_failure_round(
        &lifecycle,
        &original.chain.scope,
        &original.chain.business_key,
        Some(&original.chain),
        NODE_ID,
        &ProviderName::Fake,
        teaching.round.round_ordinal + 1,
    )
    .expect("ordinal 2 binds")
    {
        ScRoundEntry::Ready(bound) => bound,
        _ => panic!("ordinal 2 must start Ready"),
    };
    audited_start(&lifecycle, &scope.session_id, third.round.role_run_seq);
    let previous_third = (
        ScEvidenceRef::Missing {
            code: "sc_failure_raw_missing".to_string(),
            detail: "not delivered".to_string(),
            round_id: third.round.round_id.clone(),
        },
        third.round.clone(),
    );
    match begin_sc_teaching_round(
        &lifecycle,
        &original,
        Some(&previous_third),
        NODE_ID,
        &ProviderName::Fake,
    )
    .expect("teaching replay resolves")
    {
        ScRoundEntry::Waiting { reason_code, .. } => {
            assert_eq!(reason_code, SC_ROUND_LAUNCH_UNKNOWN)
        }
        _ => panic!("audited teaching start must classify Waiting"),
    }
    // 一轮 RoundBound 一行：原轮 + 教学 1 + 教学 2。
    assert_eq!(evidence_rows(&lifecycle, &original.chain), 3);
}

// P08 review F1 回归：已绑定 operation 的重入**必须**沿原链解析既有轮。
// 传 None 会以 (scope, key, bound_node, bound_node, ordinal) 查询而落空，创建
// 新 chain/新 seq（= 同一 operation 的第二物理启动）。此处只用消费者可见事实：
// 原链 id 不变、seq 不变、证据行不增、零 start。
#[test]
fn bound_operation_reentry_keeps_original_chain_and_run_seq() {
    let (_tmp, lifecycle, scope) = super::tests::sc_round_fixture();
    let original = bind_original(&lifecycle, &scope);
    let teaching = match begin_sc_teaching_round(
        &lifecycle,
        &original,
        None,
        NODE_ID,
        &ProviderName::Fake,
    )
    .expect("teaching round binds")
    {
        ScRoundEntry::Ready(bound) => bound,
        _ => panic!("un-started teaching round must classify Ready"),
    };
    let rows_before = evidence_rows(&lifecycle, &original.chain);
    let seq_before = teaching.round.role_run_seq;
    let starts_before = actual_starts(&lifecycle, &scope.session_id);

    // 重入：同一 (node, ordinal) + 原链身份 ⇒ 命中既有轮，不建新链/新 seq。
    match begin_sc_failure_round(
        &lifecycle,
        &scope,
        BUSINESS_KEY_FOR_REPLAY,
        Some(&original.chain),
        NODE_ID,
        &ProviderName::Fake,
        teaching.round.round_ordinal,
    )
    .expect("bound re-entry resolves the existing round")
    {
        ScRoundEntry::Ready(replay) => {
            assert_eq!(
                replay.chain.chain_id, original.chain.chain_id,
                "re-entry must stay on the original chain (no new chain/seq)"
            );
            assert_eq!(
                replay.round.role_run_seq, seq_before,
                "re-entry must reuse the bound run seq"
            );
            assert_eq!(replay.round.round_id, teaching.round.round_id);
        }
        _ => panic!("un-started bound round must classify Ready on re-entry"),
    }
    assert_eq!(
        evidence_rows(&lifecycle, &original.chain),
        rows_before,
        "re-entry adds no RoundBound row"
    );
    assert_eq!(
        actual_starts(&lifecycle, &scope.session_id),
        starts_before,
        "re-entry starts nothing"
    );
}
