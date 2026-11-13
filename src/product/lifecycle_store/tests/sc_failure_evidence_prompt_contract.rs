// P05 严格证据 store：教学提示处置契约 + 日志失败 missing_evidence 的 RED 用例。
// 落位：src/product/lifecycle_store/tests/sc_failure_evidence_prompt_contract.rs
// 挂载：sc_failure_evidence.rs 末尾
//   #[cfg(test)] #[path = "tests/sc_failure_evidence_prompt_contract.rs"] mod prompt_contract_tests;
// 复用 store 模块统一挂载的同一 fixture 实例（不重复实例化）。
// 断言只落在消费者可见面：稳定 code / step / round_id / publication_incomplete /
// verified_prefix / missing_evidence / 正文逐字；不钉 detail 措辞。

use super::fixture::{PROMPT_TEXT, begin, bound_round, chain_dir, live_sc_session, setup};
use crate::product::lifecycle_store::sc_failure_evidence::ScFailureEvidenceEvent;
use crate::web::workspace_ws_types::{
    ScEvidenceRef, ScPromptDisposition, ScTeachingPromptEvidence,
};

/// A：Unavailable 不是 put 的合法处置——必须在 mkdir/lease/写之前拒绝且不留副作用；
/// 缺提示只能走 append 的 PromptUnavailable + Missing 真实原因（不许伪 Available）。
#[test]
fn sc_failure_contract_prompt_unavailable_rejected_and_missing_kept_verbatim() {
    let (_tmp, store) = setup();
    let session_id = live_sc_session(&store);
    let handle = begin(&store, &session_id);
    let (chain, round) = bound_round(&store, &session_id, &handle);
    let log_path = chain_dir(&store, &chain).join("rounds.jsonl");
    // ① put 的 Unavailable：稳定 code，且不得建目录/落文件
    let err = store
        .put_sc_teaching_prompt(
            &chain,
            &round.round_id,
            2,
            "builder_compile_0001",
            PROMPT_TEXT,
            ScPromptDisposition::Unavailable,
        )
        .expect_err("Unavailable 不是 put 的合法处置");
    assert_eq!(err.code, "sc_failure_prompt_disposition_invalid");
    assert_eq!(err.step, "put_sc_teaching_prompt");
    assert_eq!(err.round_id.as_deref(), Some(round.round_id.as_str()));
    assert!(!err.publication_incomplete);
    assert!(err.verified_prefix.is_none());
    assert!(!chain_dir(&store, &chain).exists(), "被拒写不得建证据分区");
    assert!(!log_path.exists(), "被拒写不得落日志");

    // 绑定真实 round 后，缺提示必须带 Missing 真实原因
    store
        .append_sc_failure_evidence(
            &chain,
            &ScFailureEvidenceEvent::RoundBound {
                round: round.clone(),
            },
        )
        .expect("bind");
    // ② Unavailable 的提示带 Available 正文 = 伪形状：必须拒
    let fake_available = ScTeachingPromptEvidence {
        failed_round_id: round.round_id.clone(),
        target_round_ordinal: 2,
        builder_id: "builder_compile_0001".to_string(),
        prompt: ScEvidenceRef::Available {
            locator: format!(
                "sc-teaching-prompt/v1/{}/{}/2/{}",
                session_id,
                chain.chain_id,
                "0".repeat(64)
            ),
            sha256: "0".repeat(64),
            bytes: 0,
        },
        disposition: ScPromptDisposition::Unavailable,
    };
    let before = std::fs::read(&log_path).expect("read log");
    let err = store
        .append_sc_failure_evidence(
            &chain,
            &ScFailureEvidenceEvent::PromptUnavailable {
                evidence: fake_available,
            },
        )
        .expect_err("Unavailable 的提示不得带 Available 正文");
    assert_eq!(err.code, "sc_failure_prompt_disposition_invalid");
    assert_eq!(err.step, "append_sc_failure_evidence");
    assert_eq!(err.round_id.as_deref(), Some(round.round_id.as_str()));
    assert!(!err.publication_incomplete);
    assert_eq!(
        std::fs::read(&log_path).expect("read log"),
        before,
        "被拒写不得改日志"
    );

    // ③ 缺提示的真实形状：Missing 原因必须原样落库并可读回
    let missing = ScTeachingPromptEvidence {
        failed_round_id: round.round_id.clone(),
        target_round_ordinal: 2,
        builder_id: "builder_compile_0001".to_string(),
        prompt: ScEvidenceRef::Missing {
            code: "sc_prompt_unavailable".to_string(),
            detail: "编译失败源不可用，未产出教学提示".to_string(),
            round_id: round.round_id.clone(),
        },
        disposition: ScPromptDisposition::Unavailable,
    };
    store
        .append_sc_failure_evidence(
            &chain,
            &ScFailureEvidenceEvent::PromptUnavailable {
                evidence: missing.clone(),
            },
        )
        .expect("带 Missing 真实原因的缺提示必须可记录");
    let read = store.read_sc_failure_evidence(&chain).expect("read");
    let stored = read
        .entries
        .iter()
        .find_map(|entry| match &entry.event {
            ScFailureEvidenceEvent::PromptUnavailable { evidence } => Some(evidence),
            _ => None,
        })
        .expect("缺提示条目必须可读回");
    assert_eq!(stored.prompt, missing.prompt, "Missing 原因必须原样保留");
    assert_eq!(stored.disposition, ScPromptDisposition::Unavailable);
}

/// B：日志类失败必须带具体 missing 项：round 不可知时不得伪 round，locator 只能是
/// 受控的内部相对引用（本 chain 的日志 ref），前缀字节/条目/through key 不变。
#[test]
fn sc_failure_contract_log_failure_reports_concrete_missing_evidence() {
    let (_tmp, store) = setup();
    let session_id = live_sc_session(&store);
    let handle = begin(&store, &session_id);
    let (chain, round) = bound_round(&store, &session_id, &handle);
    store
        .append_sc_failure_evidence(
            &chain,
            &ScFailureEvidenceEvent::RoundBound {
                round: round.clone(),
            },
        )
        .expect("bind");
    let log_path = chain_dir(&store, &chain).join("rounds.jsonl");
    let good = std::fs::read(&log_path).expect("read log");
    // 坏行：JSON 不可解析（该行对应的 round 不可知）
    let corrupt = format!(
        "{}\n{{\"schema_version\":1,\"entry_seq\":\n",
        String::from_utf8_lossy(&good)
    );
    std::fs::write(&log_path, corrupt).expect("write");
    let err = store
        .read_sc_failure_evidence(&chain)
        .expect_err("坏行必须拒绝");
    assert_eq!(err.code, "sc_failure_log_corrupt");
    assert_eq!(err.step, "read_sc_failure_evidence");
    assert_eq!(err.round_id, None, "日志类错误不得伪 round");
    let prefix = err.verified_prefix.as_deref().expect("verified prefix");
    let missing = prefix
        .missing_evidence
        .first()
        .expect("日志类失败必须带具体 missing");
    assert_eq!(missing.round_id, "unknown");
    assert_eq!(missing.kind, "evidence_log");
    assert_eq!(missing.code, "sc_failure_log_corrupt");
    assert!(!missing.detail.is_empty(), "detail 必须承载原始故障原因");
    let locator = missing.locator.as_deref().expect("日志 ref 必须可知");
    assert!(
        !locator.starts_with('/'),
        "locator 必须是内部相对引用，不是绝对路径"
    );
    assert!(
        locator.contains(&chain.chain_id) && locator.ends_with("rounds.jsonl"),
        "locator 必须指向本 chain 的日志 ref"
    );
    // 已验证前缀内容不变
    assert_eq!(prefix.verified_prefix_bytes, good);
    assert_eq!(prefix.entries.len(), 1);
    assert_eq!(
        prefix.evidence_through_entry_key,
        format!("round/{}", round.round_id)
    );
}
