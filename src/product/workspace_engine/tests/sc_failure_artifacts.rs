// P11 纯组包器行为用例（自 `sc_failure_artifacts.rs` 原样迁出，行为不变）。
// 挂载：`sc_failure_artifacts.rs` 末尾
//   #[cfg(test)]
//   #[path = "tests/sc_failure_artifacts.rs"]
//   mod tests;
// `super` 仍解析到 `sc_failure_artifacts` 模块，函数体逐字未改。
// 文件末尾另附 Q1 行为 RED 用例（已绑定但明确 NotStarted 的轮不得谎称已启动未交付）。

use super::build_sc_failure_repair_package;
use crate::product::lifecycle_store::sc_failure_evidence::{
    ScFailureEvidenceEntry, ScFailureEvidenceEvent, ScFailureEvidenceRead,
    sc_failure_evidence_log_ref,
};
use crate::product::lifecycle_store::sc_failure_publication::{ScLocalTerminal, ScTerminalInput};
use crate::product::models::ProviderName;
use crate::web::workspace_ws_types::{
    ScEvidenceRef, ScFailureChainIdentity, ScFailureClass, ScFailureScope, ScMissingEvidence,
    ScPackageCompleteness, ScPromptDisposition, ScRoundIdentity, ScRoundLaunchState,
    ScTeachingDecision, ScTeachingPromptEvidence,
};
use sha2::{Digest, Sha256};

const SESSION: &str = "ws-sc-p11";
const CHAIN: &str = "chain-v1-p11";
const RAW1: &str = "provider raw 1：{\"plan\":\"候选一\"}\n第二行 tab\t quotes \"x\"";
const SOURCE1: &str = "normalized author delivery source 1（去掉 preamble 后）";
const IR_REASON: &str = "IR preflight rejected: dependency cycle（保留原机械失败措辞）";
const INTERRUPTED_REASON: &str = "教学中断：第二轮 provider 调用被 operational 错误终止";

fn sha_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn available(locator: &str, body: &str) -> ScEvidenceRef {
    ScEvidenceRef::Available {
        locator: locator.into(),
        sha256: sha_hex(body.as_bytes()),
        bytes: body.as_bytes().len() as u64,
    }
}

fn split_locator(seq: u64) -> String {
    format!("sc-split-output/v1/{SESSION}/{seq}")
}

fn chain() -> ScFailureChainIdentity {
    let scope = ScFailureScope {
        session_id: SESSION.into(),
        project_id: "proj-p11".into(),
        issue_id: "issue-p11".into(),
        plan_id: "plan-p11".into(),
    };
    ScFailureChainIdentity {
        scope,
        chain_id: CHAIN.into(),
        business_key: "biz-p11".into(),
        original_node_id: "node-original".into(),
        original_role_run_seq: 10,
    }
}

fn round(ordinal: u32, node: &str, seq: u64) -> ScRoundIdentity {
    let round_id = format!("round-{ordinal}");
    ScRoundIdentity {
        chain_id: CHAIN.into(),
        round_id,
        node_id: node.into(),
        round_ordinal: ordinal,
        provider: ProviderName::Fake,
        workspace_session_id: SESSION.into(),
        role_run_seq: seq,
        run_ref: format!("single-candidate/runs/{seq}"),
    }
}

fn entry(
    seq: u64,
    key: String,
    chain: &ScFailureChainIdentity,
    event: ScFailureEvidenceEvent,
) -> ScFailureEvidenceEntry {
    ScFailureEvidenceEntry {
        schema_version: 1,
        entry_seq: seq,
        entry_key: key,
        chain: chain.clone(),
        event,
    }
}

fn read(entries: Vec<ScFailureEvidenceEntry>, prefix: Vec<u8>) -> ScFailureEvidenceRead {
    ScFailureEvidenceRead {
        entries,
        verified_prefix_bytes: prefix,
        evidence_through_entry_key: "terminal".into(),
        missing_evidence: Vec::new(),
    }
}

fn bound(seq: u64, chain: &ScFailureChainIdentity, r: &ScRoundIdentity) -> ScFailureEvidenceEntry {
    entry(
        seq,
        format!("round/{}", r.round_id),
        chain,
        ScFailureEvidenceEvent::RoundBound { round: r.clone() },
    )
}

fn failure(
    seq: u64,
    chain: &ScFailureChainIdentity,
    r: &ScRoundIdentity,
    class: ScFailureClass,
    reasons: Vec<String>,
    instructions: Vec<String>,
    raw: ScEvidenceRef,
    source: ScEvidenceRef,
) -> ScFailureEvidenceEntry {
    entry(
        seq,
        format!("failureobserved/{}", r.round_id),
        chain,
        ScFailureEvidenceEvent::FailureObserved {
            round_id: r.round_id.clone(),
            failure_class: Some(class),
            blocking_reasons: reasons,
            repair_instructions: instructions,
            raw,
            source,
            existing_failure_id: None,
            attempt_key: None,
        },
    )
}

fn decision(
    seq: u64,
    chain: &ScFailureChainIdentity,
    failed: &ScRoundIdentity,
    decision: ScTeachingDecision,
) -> ScFailureEvidenceEntry {
    entry(
        seq,
        format!("teachingdecision/{}", failed.round_id),
        chain,
        ScFailureEvidenceEvent::TeachingDecision {
            failed_round_id: failed.round_id.clone(),
            decision,
        },
    )
}

fn sent_prompt(r: &ScRoundIdentity, target_ordinal: u32) -> ScTeachingPromptEvidence {
    ScTeachingPromptEvidence {
        failed_round_id: r.round_id.clone(),
        target_round_ordinal: target_ordinal,
        builder_id: "sc-teach-compile-v1".into(),
        prompt: available(
            "sc-teaching-prompt/v1/ws-sc-p11/chain-v1-p11/2/p",
            "教学提示全文",
        ),
        disposition: ScPromptDisposition::Sent,
    }
}

fn terminal(
    seq: u64,
    chain: &ScFailureChainIdentity,
    last_failure: &ScRoundIdentity,
    last_physical: &ScRoundIdentity,
    class: &str,
    reason: &str,
    launches: Vec<(String, ScRoundLaunchState)>,
    completeness: ScPackageCompleteness,
    missing: Vec<ScMissingEvidence>,
) -> ScFailureEvidenceEntry {
    entry(
        seq,
        "terminal".into(),
        chain,
        ScFailureEvidenceEvent::ChainTerminal {
            last_failure_round_id: last_failure.round_id.clone(),
            last_physical_round_id: Some(last_physical.round_id.clone()),
            terminal_class: class.into(),
            terminal_reason: reason.into(),
            launch_observations: launches,
            completeness,
            missing_evidence: missing,
        },
    )
}

fn started(r: &ScRoundIdentity) -> (String, ScRoundLaunchState) {
    (r.round_id.clone(), ScRoundLaunchState::Started)
}

fn complete_prefix() -> Vec<u8> {
    "{\"entry_seq\":2,\"kind\":\"failure_observed\",\"reasons\":[\"E0308:3: mismatched types\",\"E0425:9: not found\"]}\n{\"entry_seq\":8,\"kind\":\"chain_terminal\",\"terminal_class\":\"ir_preflight\"}\n中文尾部\n".as_bytes().to_vec()
}

fn interrupted_prefix() -> Vec<u8> {
    "{\"entry_seq\":2,\"kind\":\"failure_observed\",\"reasons\":[\"E0308:3: mismatched types\"]}\n{\"entry_seq\":6,\"kind\":\"chain_terminal\",\"terminal_class\":\"teaching_interrupted\"}\n中断前缀\n".as_bytes().to_vec()
}

fn one_round_prefix() -> Vec<u8> {
    "{\"entry_seq\":1,\"kind\":\"round_bound\"}\n{\"entry_seq\":2,\"kind\":\"failure_observed\"}\n{\"entry_seq\":4,\"kind\":\"chain_terminal\"}\n".as_bytes().to_vec()
}

fn one_round_read(reason: &str, prefix: Vec<u8>) -> ScFailureEvidenceRead {
    let chain = chain();
    let r = round(1, "node-compile", 11);
    let locator = split_locator(11);
    read(
        vec![
            bound(1, &chain, &r),
            failure(
                2,
                &chain,
                &r,
                ScFailureClass::Compile,
                vec![reason.to_string()],
                vec!["fix the compile error".into()],
                available(&locator, RAW1),
                available(&locator, SOURCE1),
            ),
            decision(3, &chain, &r, ScTeachingDecision::NotRequested),
            terminal(
                4,
                &chain,
                &r,
                &r,
                "compile",
                reason,
                vec![started(&r)],
                ScPackageCompleteness::Complete,
                Vec::new(),
            ),
        ],
        prefix,
    )
}

#[test]
fn complete_terminal_builds_ordered_rounds_and_last_physical_raw() {
    let chain = chain();
    let (r1, r2) = (round(1, "node-compile", 11), round(2, "node-ir", 12));
    let raw1 = available(&split_locator(11), RAW1);
    let src1 = available(&split_locator(11), SOURCE1);
    let raw2 = available(&split_locator(12), "IR provider 原始输出二");
    let src2 = available(&split_locator(12), "IR 归一化 source 2");
    let reasons1 = vec![
        "E0308:3: mismatched types".to_string(),
        "E0425:9: not found".to_string(),
    ];
    let instructions1 = vec![
        "fix the borrow".to_string(),
        "rename the binding".to_string(),
    ];
    let reasons2 = vec!["ir_preflight: dependency cycle".to_string()];
    let prompt = sent_prompt(&r1, 2);
    let entries = vec![
        bound(1, &chain, &r1),
        failure(
            2,
            &chain,
            &r1,
            ScFailureClass::Compile,
            reasons1.clone(),
            instructions1.clone(),
            raw1.clone(),
            src1.clone(),
        ),
        entry(
            3,
            format!("prompt/{}/2/sent", r1.round_id),
            &chain,
            ScFailureEvidenceEvent::PromptPrepared {
                evidence: prompt.clone(),
            },
        ),
        decision(4, &chain, &r1, ScTeachingDecision::Granted),
        bound(5, &chain, &r2),
        failure(
            6,
            &chain,
            &r2,
            ScFailureClass::IrPreflight,
            reasons2.clone(),
            Vec::new(),
            raw2.clone(),
            src2.clone(),
        ),
        decision(7, &chain, &r2, ScTeachingDecision::NotRequested),
        terminal(
            8,
            &chain,
            &r2,
            &r2,
            "ir_preflight",
            IR_REASON,
            vec![started(&r1), started(&r2)],
            ScPackageCompleteness::Complete,
            Vec::new(),
        ),
    ];
    let prefix = complete_prefix();
    let package = build_sc_failure_repair_package(
        &chain,
        &read(entries, prefix.clone()),
        ScTerminalInput::Durable,
    )
    .expect("complete terminal evidence must build a package");

    assert_eq!(package.schema_version, 1);
    assert_eq!(
        (
            package.chain_id.as_str(),
            package.session_id.as_str(),
            package.project_id.as_str(),
            package.issue_id.as_str(),
            package.plan_id.as_str()
        ),
        (CHAIN, SESSION, "proj-p11", "issue-p11", "plan-p11")
    );
    assert_eq!(package.terminal_class, "ir_preflight");
    assert_eq!(package.terminal_reason, IR_REASON);
    assert_eq!(package.completeness, ScPackageCompleteness::Complete);
    assert!(package.missing_evidence.is_empty());
    assert_eq!(package.rounds.len(), 2);
    assert_eq!(package.rounds[0].identity, r1);
    assert_eq!(package.rounds[1].identity, r2);
    assert_eq!(package.rounds[0].launch_state, ScRoundLaunchState::Started);
    assert_eq!(package.rounds[1].launch_state, ScRoundLaunchState::Started);
    assert_eq!(
        package.rounds[0].failure_class,
        Some(ScFailureClass::Compile)
    );
    assert_eq!(package.rounds[0].blocking_reasons, reasons1);
    assert_eq!(package.rounds[0].repair_instructions, instructions1);
    assert_eq!(package.rounds[0].raw, raw1);
    assert_eq!(package.rounds[0].source, src1);
    assert_eq!(
        package.rounds[0].teaching_decision,
        ScTeachingDecision::Granted
    );
    assert_eq!(package.rounds[0].prompts, vec![prompt.clone()]);
    assert_eq!(
        package.rounds[1].failure_class,
        Some(ScFailureClass::IrPreflight)
    );
    assert_eq!(package.rounds[1].blocking_reasons, reasons2);
    assert_eq!(package.rounds[1].raw, raw2);
    assert_eq!(package.rounds[1].source, src2);
    assert_eq!(
        package.rounds[1].teaching_decision,
        ScTeachingDecision::NotRequested
    );
    assert!(package.rounds[1].prompts.is_empty());
    assert_eq!(package.last_provider_output, raw2);
    assert_eq!(package.last_failure_source.source_round_id, r2.round_id);
    assert_eq!(package.last_failure_source.source, src2);
    assert_eq!(package.teaching_prompts, vec![prompt]);
    assert_eq!(package.evidence_through_entry_key, "terminal");
    assert_eq!(package.evidence_digest, sha_hex(&prefix));
    assert_eq!(
        package.evidence_log_ref,
        sc_failure_evidence_log_ref(&chain).expect("strict log ref")
    );
    assert!(package.validate().is_ok(), "{:?}", package.validate());
}

#[test]
fn interrupted_second_started_keeps_round_one_as_failure_source() {
    let chain = chain();
    let (r1, r2) = (round(1, "node-compile", 11), round(2, "node-compile", 12));
    let raw1 = available(&split_locator(11), RAW1);
    let src1 = available(&split_locator(11), SOURCE1);
    let prompt = sent_prompt(&r1, 2);
    let gap = ScMissingEvidence {
        round_id: r2.round_id.clone(),
        kind: "raw".into(),
        code: "sc_failure_output_not_delivered".into(),
        detail: "第二轮已启动但未交付 raw 即被中断".into(),
        locator: None,
    };
    let entries = vec![
        bound(1, &chain, &r1),
        failure(
            2,
            &chain,
            &r1,
            ScFailureClass::Compile,
            vec!["E0308:3: mismatched types".into()],
            vec!["fix the compile error".into()],
            raw1.clone(),
            src1.clone(),
        ),
        entry(
            3,
            format!("prompt/{}/2/sent", r1.round_id),
            &chain,
            ScFailureEvidenceEvent::PromptPrepared {
                evidence: prompt.clone(),
            },
        ),
        decision(4, &chain, &r1, ScTeachingDecision::Granted),
        bound(5, &chain, &r2),
        terminal(
            6,
            &chain,
            &r1,
            &r2,
            "teaching_interrupted",
            INTERRUPTED_REASON,
            vec![started(&r1), started(&r2)],
            ScPackageCompleteness::Incomplete,
            vec![gap.clone()],
        ),
    ];
    let prefix = interrupted_prefix();
    let package = build_sc_failure_repair_package(
        &chain,
        &read(entries, prefix.clone()),
        ScTerminalInput::Durable,
    )
    .expect("interrupted chain with observed failure must build a package");

    assert_eq!(package.terminal_class, "teaching_interrupted");
    assert_eq!(package.terminal_reason, INTERRUPTED_REASON);
    assert_eq!(package.completeness, ScPackageCompleteness::Incomplete);
    assert_eq!(package.missing_evidence, vec![gap]);
    assert_eq!(package.rounds.len(), 2);
    assert_eq!(package.rounds[0].raw, raw1);
    assert_eq!(
        package.rounds[0].teaching_decision,
        ScTeachingDecision::Granted
    );
    assert_eq!(package.rounds[0].prompts, vec![prompt.clone()]);
    assert_eq!(package.rounds[1].identity, r2);
    assert_eq!(package.rounds[1].launch_state, ScRoundLaunchState::Started);
    assert!(package.rounds[1].failure_class.is_none());
    match &package.rounds[1].raw {
        ScEvidenceRef::Missing {
            code,
            detail,
            round_id,
        } => {
            assert_eq!(code, "sc_failure_output_not_delivered");
            assert_eq!(round_id.as_str(), r2.round_id.as_str());
            assert!(!detail.is_empty());
        }
        other => panic!("started round without delivery must stay explicitly missing: {other:?}"),
    }
    match &package.last_provider_output {
        ScEvidenceRef::Missing { code, .. } => assert_eq!(code, "sc_failure_output_not_delivered"),
        other => panic!(
            "last physical round has no delivery, so its raw must be an explicit missing ref: {other:?}"
        ),
    }
    assert_eq!(package.last_failure_source.source_round_id, r1.round_id);
    assert_eq!(package.last_failure_source.source, src1);
    assert_eq!(package.teaching_prompts, vec![prompt]);
    assert_eq!(package.evidence_digest, sha_hex(&prefix));
    assert_ne!(package.evidence_digest, "0".repeat(64));
    assert!(package.validate().is_ok(), "{:?}", package.validate());
}

#[test]
fn equal_terminal_content_replays_same_package_id_and_reason_change_flips_it() {
    let chain = chain();
    let first = build_sc_failure_repair_package(
        &chain,
        &one_round_read("E0308:3: mismatched types", one_round_prefix()),
        ScTerminalInput::Durable,
    )
    .expect("build");
    let replay = build_sc_failure_repair_package(
        &chain,
        &one_round_read("E0308:3: mismatched types", one_round_prefix()),
        ScTerminalInput::Durable,
    )
    .expect("replay from independently constructed equal evidence");
    assert_eq!(first.package_id.len(), 64);
    assert!(
        first
            .package_id
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    );
    assert_eq!(
        first, replay,
        "same terminal content must replay an identical package"
    );

    let changed = build_sc_failure_repair_package(
        &chain,
        &one_round_read("E0308:3: different wording", one_round_prefix()),
        ScTerminalInput::Durable,
    )
    .expect("changed reason");
    assert_ne!(first.package_id, changed.package_id);
    assert_eq!(
        first.evidence_digest, changed.evidence_digest,
        "evidence digest covers the log prefix bytes, not event wording"
    );

    let other_prefix = b"other rounds.jsonl prefix bytes".to_vec();
    let rehashed = build_sc_failure_repair_package(
        &chain,
        &one_round_read("E0308:3: mismatched types", other_prefix.clone()),
        ScTerminalInput::Durable,
    )
    .expect("other prefix");
    assert_eq!(rehashed.evidence_digest, sha_hex(&other_prefix));
    assert_ne!(first.package_id, rehashed.package_id);
}

#[test]
fn non_terminal_read_is_rejected_instead_of_fabricating_terminal() {
    let mut evidence = one_round_read("E0308:3: mismatched types", one_round_prefix());
    evidence.entries.pop();
    let error = build_sc_failure_repair_package(&chain(), &evidence, ScTerminalInput::Durable)
        .expect_err("non-terminal evidence must not build a terminal package");
    assert!(
        error.code.starts_with("sc_failure_"),
        "unexpected code: {}",
        error.code
    );
    assert!(!error.step.is_empty());
    assert!(!error.detail.is_empty());
}

#[test]
fn missing_teaching_decision_is_rejected_instead_of_defaulted() {
    let mut evidence = one_round_read("E0308:3: mismatched types", one_round_prefix());
    evidence
        .entries
        .retain(|item| !matches!(&item.event, ScFailureEvidenceEvent::TeachingDecision { .. }));
    let error = build_sc_failure_repair_package(&chain(), &evidence, ScTerminalInput::Durable)
        .expect_err("missing decision on a failed round must not be defaulted");
    assert_eq!(error.code, "sc_failure_teaching_decision_missing");
    assert!(!error.step.is_empty());
}

// P11 Q1 行为 RED（独立件，仅新增用例；不改实现）。
// 落位：`src/product/workspace_engine/tests/sc_failure_artifacts.rs` 末尾
// （既有用例之后、文件末尾）。
//
// 观察目标：当前实现把“已绑定但明确 NotStarted”的轮无条件写成
// `Missing{code:"sc_failure_output_not_delivered"}`（谎称已启动未交付）。
// 期望：显式 NotStarted 轮必须用 `sc_failure_raw_missing`，且 last physical 仍是
// 真实交付的 r1（raw 保持 Available，last_failure_source 仍取 r1 的 source）。
// 本用例只断 code 与结构，不钉 detail 文案。

#[test]
fn bound_but_not_started_round_keeps_raw_missing_instead_of_not_delivered() {
    let chain = chain();
    let (r1, r2) = (round(1, "node-compile", 11), round(2, "node-compile", 12));
    let raw1 = available(&split_locator(11), RAW1);
    let src1 = available(&split_locator(11), SOURCE1);
    let gap = ScMissingEvidence {
        round_id: r2.round_id.clone(),
        kind: "raw".into(),
        code: "sc_failure_raw_missing".into(),
        detail: "第二轮教学 handle 已绑定但 provider 未启动".into(),
        locator: None,
    };
    let entries = vec![
        bound(1, &chain, &r1),
        failure(
            2,
            &chain,
            &r1,
            ScFailureClass::Compile,
            vec!["E0308:3: mismatched types".into()],
            vec!["fix the compile error".into()],
            raw1.clone(),
            src1.clone(),
        ),
        decision(3, &chain, &r1, ScTeachingDecision::Granted),
        bound(4, &chain, &r2),
        terminal(
            5,
            &chain,
            &r1,
            &r1,
            "teaching_interrupted",
            INTERRUPTED_REASON,
            vec![
                started(&r1),
                (r2.round_id.clone(), ScRoundLaunchState::NotStarted),
            ],
            ScPackageCompleteness::Incomplete,
            vec![gap.clone()],
        ),
    ];
    let prefix = interrupted_prefix();
    let package = build_sc_failure_repair_package(
        &chain,
        &read(entries, prefix.clone()),
        ScTerminalInput::Durable,
    )
    .expect("a bound but never started round must still build an honest package");

    assert_eq!(package.rounds.len(), 2);
    assert_eq!(package.rounds[1].identity, r2);
    assert_eq!(
        package.rounds[1].launch_state,
        ScRoundLaunchState::NotStarted
    );
    assert!(package.rounds[1].failure_class.is_none());
    match &package.rounds[1].raw {
        ScEvidenceRef::Missing {
            code,
            detail,
            round_id,
        } => {
            assert_eq!(
                code, "sc_failure_raw_missing",
                "a never-started round must not claim it started and lost its output"
            );
            assert_eq!(round_id.as_str(), r2.round_id.as_str());
            assert!(!detail.is_empty());
        }
        other => {
            panic!("bound but never started round must stay explicitly raw-missing: {other:?}")
        }
    }
    assert_eq!(
        package.rounds[1].source, package.rounds[1].raw,
        "raw and source share the same honesty verdict"
    );
    assert_eq!(
        package.last_provider_output, raw1,
        "last physical round is r1 and it did deliver, so its raw stays available"
    );
    assert_eq!(package.last_failure_source.source_round_id, r1.round_id);
    assert_eq!(package.last_failure_source.source, src1);
    assert_eq!(package.completeness, ScPackageCompleteness::Incomplete);
    assert_eq!(package.missing_evidence, vec![gap]);
    assert_eq!(package.evidence_digest, sha_hex(&prefix));
    assert!(package.validate().is_ok(), "{:?}", package.validate());
}

// P19d 行为 RED（仅新增用例，不改实现）：Local 终局携带真实 source 读失败
// （`kind == "source"`，round 已绑定）时，该轮 `source` 与 `last_failure_source`
// 必须投影成显式 `Missing{code/detail/round_id}`，raw / 前缀字节 / digest /
// through key / prompts 不动；同一轮互相矛盾的 source missing 事实必须拒绝。

const LOCAL_REASON: &str = "证据日志尾部不可读：source 读取在真实已验证前缀之外失败";

fn local_terminal(
    r: &ScRoundIdentity,
    observations: Vec<(String, ScRoundLaunchState)>,
    missing: Vec<ScMissingEvidence>,
) -> ScLocalTerminal {
    ScLocalTerminal {
        last_failure_round_id: r.round_id.clone(),
        last_physical_round_id: Some(r.round_id.clone()),
        terminal_class: "log_corrupt".into(),
        terminal_reason: LOCAL_REASON.into(),
        launch_observations: observations,
        completeness: ScPackageCompleteness::Incomplete,
        missing_evidence: missing,
    }
}

#[test]
fn local_source_read_failure_projects_round_and_last_failure_source_to_missing() {
    let chain = chain();
    let r1 = round(1, "node-compile", 11);
    let raw1 = available(&split_locator(11), RAW1);
    let src1 = available(&split_locator(11), SOURCE1);
    let prompt = sent_prompt(&r1, 2);
    let source_gap = ScMissingEvidence {
        round_id: r1.round_id.clone(),
        kind: "source".into(),
        code: "sc_failure_source_unreadable".into(),
        detail: "source 读取失败：逻辑前缀之后的行真实不可读".into(),
        locator: None,
    };
    let entries = vec![
        bound(1, &chain, &r1),
        failure(
            2,
            &chain,
            &r1,
            ScFailureClass::Compile,
            vec!["E0308:3: mismatched types".into()],
            vec!["fix the compile error".into()],
            raw1.clone(),
            src1.clone(),
        ),
        entry(
            3,
            format!("prompt/{}/2/sent", r1.round_id),
            &chain,
            ScFailureEvidenceEvent::PromptPrepared {
                evidence: prompt.clone(),
            },
        ),
        decision(4, &chain, &r1, ScTeachingDecision::Granted),
    ];
    let prefix = interrupted_prefix();
    let local = local_terminal(&r1, vec![started(&r1)], vec![source_gap.clone()]);
    let package = build_sc_failure_repair_package(
        &chain,
        &read(entries, prefix.clone()),
        ScTerminalInput::Local(&local),
    )
    .expect("Local 终局必须把真实 source 读失败投影成显式 missing，而不是继续谎报 Available");

    let projected = ScEvidenceRef::Missing {
        code: source_gap.code.clone(),
        detail: source_gap.detail.clone(),
        round_id: r1.round_id.clone(),
    };
    assert_eq!(package.completeness, ScPackageCompleteness::Incomplete);
    assert_eq!(package.missing_evidence, vec![source_gap.clone()]);
    assert_eq!(package.rounds.len(), 1);
    assert_eq!(
        package.rounds[0].source, projected,
        "Local source 读失败必须投影成该轮显式 missing"
    );
    assert_eq!(package.rounds[0].raw, raw1, "raw 不是 source，投影不得碰它");
    assert_eq!(
        package.rounds[0].prompts,
        vec![prompt.clone()],
        "prompts 与 source 投影无关"
    );
    assert_eq!(package.last_failure_source.source_round_id, r1.round_id);
    assert_eq!(
        package.last_failure_source.source, projected,
        "last_failure_source 必须取同一投影后的该轮 source"
    );
    assert_eq!(package.last_provider_output, raw1);
    assert_eq!(package.teaching_prompts, vec![prompt]);
    assert_eq!(
        package.evidence_digest,
        sha_hex(&prefix),
        "digest 只由真实前缀字节决定"
    );
    assert_eq!(package.evidence_through_entry_key, "terminal");
    assert_eq!(package.terminal_reason, LOCAL_REASON);
    assert!(package.validate().is_ok(), "{:?}", package.validate());
}

#[test]
fn local_duplicate_source_missing_with_conflicting_detail_is_rejected() {
    let chain = chain();
    let r1 = round(1, "node-compile", 11);
    let first = ScMissingEvidence {
        round_id: r1.round_id.clone(),
        kind: "source".into(),
        code: "sc_failure_source_unreadable".into(),
        detail: "source 读取失败：locator 不存在".into(),
        locator: None,
    };
    let second = ScMissingEvidence {
        detail: "source 读取失败：权限拒绝".into(),
        ..first.clone()
    };
    let entries = vec![
        bound(1, &chain, &r1),
        failure(
            2,
            &chain,
            &r1,
            ScFailureClass::Compile,
            vec!["E0308:3: mismatched types".into()],
            vec!["fix the compile error".into()],
            available(&split_locator(11), RAW1),
            available(&split_locator(11), SOURCE1),
        ),
        decision(3, &chain, &r1, ScTeachingDecision::Granted),
    ];
    let local = local_terminal(&r1, vec![started(&r1)], vec![first, second]);
    let error = build_sc_failure_repair_package(
        &chain,
        &read(entries, interrupted_prefix()),
        ScTerminalInput::Local(&local),
    )
    .expect_err("同一轮出现两个互相矛盾的 source missing 事实时不得任选其一");
    assert_eq!(error.code, "sc_failure_round_evidence_inconsistent");
    assert_eq!(error.round_id.as_deref(), Some(r1.round_id.as_str()));
}
