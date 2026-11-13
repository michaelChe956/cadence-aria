//! P08 replay-safe 分类红测（一）：`Ready` / `Completed`。
//!
//! **RED 面**：本文件按「真实 durable 事实 → typed 启动分类」写断言，当前实现
//! 的 `begin_sc_failure_round` 仍返回 `Result<ScBoundProviderRound, _>`，因此本
//! 文件在分类 API 落地前**编译不过**——这正是缺失面：`ScRoundEntry`
//! （`Ready`/`Completed{raw_text, raw}`/`Waiting{reason_code, detail}`）与其
//! `is_ready()`。
//!
//! 断言口径只取消费者可见事实：分类、逐字 raw、证据行数、provider 启动计数、
//! run.json 字节不变。fixture 与启动计数复用父模块 `tests` 的既有 helper
//! （`sc_round_fixture` / `actual_starts`），不复制第二份。

use super::tests::{actual_starts, sc_round_fixture};
use super::*;

const NODE_ID: &str = "node_sc_0001";
const BUSINESS_KEY: &str = "workspace_author:session:0";

/// 一次真实绑定（首绑必须 Ready：新 seq 无任何启动事实）。
pub(super) fn bind_original(
    lifecycle: &LifecycleStore,
    scope: &ScFailureScope,
) -> ScBoundProviderRound {
    match begin_sc_failure_round(
        lifecycle,
        scope,
        BUSINESS_KEY,
        None,
        NODE_ID,
        &ProviderName::Fake,
        0,
    )
    .expect("round binds a real handle")
    {
        ScRoundEntry::Ready(bound) => bound,
        _ => panic!("a freshly bound round has no start fact and must classify Ready"),
    }
}

/// 按同一身份重放并取出分类。
pub(super) fn classify(
    lifecycle: &LifecycleStore,
    scope: &ScFailureScope,
    original: Option<&ScFailureChainIdentity>,
    ordinal: u32,
) -> ScRoundEntry {
    begin_sc_failure_round(
        lifecycle,
        scope,
        BUSINESS_KEY,
        original,
        NODE_ID,
        &ProviderName::Fake,
        ordinal,
    )
    .expect("round lookup is strict, not absent")
}

pub(super) fn evidence_rows(lifecycle: &LifecycleStore, chain: &ScFailureChainIdentity) -> usize {
    lifecycle
        .read_sc_failure_evidence(chain)
        .expect("evidence readable")
        .entries
        .len()
}

fn run_handle(round: &ScRoundIdentity) -> WorkItemSplitProviderRunHandle {
    WorkItemSplitProviderRunHandle {
        run_ref: round.run_ref.clone(),
        workspace_session_id: round.workspace_session_id.clone(),
        role_run_seq: round.role_run_seq,
    }
}

pub(super) fn run_json_bytes(lifecycle: &LifecycleStore, round: &ScRoundIdentity) -> Vec<u8> {
    let path = lifecycle
        .work_item_split_provider_run_dir(&run_handle(round))
        .expect("run dir")
        .join("run.json");
    std::fs::read(&path).expect("run.json readable")
}

pub(super) fn delimited_output_path(
    lifecycle: &LifecycleStore,
    round: &ScRoundIdentity,
) -> std::path::PathBuf {
    lifecycle
        .work_item_split_provider_run_dir(&run_handle(round))
        .expect("run dir")
        .join("structured_output.json")
}

// 行 1：RoundBound 存在 + run running + 严格审计分区为空 ⇒ Ready（同 handle/seq；
// 本层只放行，不启动）。
#[test]
fn classifier_ready_for_bound_running_round_with_empty_audit_partition() {
    let (_tmp, lifecycle, scope) = sc_round_fixture();
    let bound = bind_original(&lifecycle, &scope);
    match classify(&lifecycle, &scope, None, 0) {
        ScRoundEntry::Ready(replay) => {
            assert_eq!(replay.round.round_id, bound.round.round_id);
            assert_eq!(replay.round.role_run_seq, bound.round.role_run_seq);
            assert_eq!(replay.round.run_ref, bound.round.run_ref);
            assert_eq!(
                replay.split.handle.role_run_seq,
                bound.split.handle.role_run_seq
            );
        }
        _ => panic!("running + empty audit partition must classify Ready"),
    }
    assert_eq!(actual_starts(&lifecycle, &scope.session_id), 0);
}

// 行 2：completed + 严格 raw 可读 ⇒ Completed（逐字 raw，含中文/preamble），
// provider start/claim 增量 0，run.json 字节不变（不重写完成时间）。
#[test]
fn classifier_completed_round_replays_verbatim_raw_without_rewrite() {
    let (_tmp, lifecycle, scope) = sc_round_fixture();
    let bound = bind_original(&lifecycle, &scope);
    let raw = "以下是完整方案：\n\n```json\n{\"work_items\":[{\"id\":\"a\",\"title\":\"中文标题\"}]}\n```\n";
    close_sc_split_round(&lifecycle, &bound, "author prompt", raw).expect("strict close");
    let bytes_before = run_json_bytes(&lifecycle, &bound.round);
    let rows_before = evidence_rows(&lifecycle, &bound.chain);
    let starts_before = actual_starts(&lifecycle, &scope.session_id);

    match classify(&lifecycle, &scope, None, 0) {
        ScRoundEntry::Completed {
            bound: replay,
            raw_text,
            raw: raw_ref,
        } => {
            assert_eq!(raw_text, raw, "durable raw replay is verbatim");
            assert_eq!(replay.round.round_id, bound.round.round_id);
            assert!(matches!(raw_ref, ScEvidenceRef::Available { .. }));
            let delivery = replayed_sc_round_delivery(raw_text, raw_ref, replay.round);
            assert_eq!(
                delivery.source,
                prepare_author_delivery_for_compile(raw).source,
                "same normalizer as the first delivery"
            );
            assert!(delivery.source.contains("中文标题"));
        }
        _ => panic!("completed round must classify Completed, not Ready/Waiting"),
    }
    assert_eq!(evidence_rows(&lifecycle, &bound.chain), rows_before);
    assert_eq!(actual_starts(&lifecycle, &scope.session_id), starts_before);
    assert_eq!(
        run_json_bytes(&lifecycle, &bound.round),
        bytes_before,
        "completed run record is not rewritten"
    );
}

// 行 4c：completed 但 raw 缺失 ⇒ 显式来源错误、零 start（不退回 Ready、不补跑）。
#[test]
fn classifier_completed_with_missing_raw_is_error_not_ready() {
    let (_tmp, lifecycle, scope) = sc_round_fixture();
    let bound = bind_original(&lifecycle, &scope);
    close_sc_split_round(&lifecycle, &bound, "author prompt", "{}").expect("strict close");
    std::fs::remove_file(delimited_output_path(&lifecycle, &bound.round)).expect("remove raw");

    let failure = begin_sc_failure_round(
        &lifecycle,
        &scope,
        BUSINESS_KEY,
        None,
        NODE_ID,
        &ProviderName::Fake,
        0,
    )
    .err()
    .expect("completed-without-raw must be an explicit source error");
    assert_eq!(failure.code, "sc_failure_raw_missing");
    assert_eq!(actual_starts(&lifecycle, &scope.session_id), 0);
}
