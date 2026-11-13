// P05 严格证据 store 消费者边界矩阵（补主测试未覆盖的 6 组边界；不是实现自证）。
// 落位：src/product/lifecycle_store/tests/sc_failure_evidence_boundaries.rs
// 挂载：sc_failure_evidence.rs 末尾 #[cfg(test)] #[path = "tests/sc_failure_evidence_boundaries.rs"] mod boundary_tests;
// 复用 store 模块统一挂载的同一 fixture 实例（不重复实例化，内容/可见性不变）。
// 断言只落在消费者可见面：错误 code / step / verified_prefix / 源身份 / 正文逐字。

use super::fixture::{
    ISSUE_ID, NODE_ID, PLAN_ID, PROJECT_ID, PROMPT_TEXT, RAW1, RAW2, available, begin, bound_round,
    chain_dir, complete_raw, live_sc_session, observed, parts, raw_locator, setup, sha256_hex,
};
use sha2::{Digest, Sha256};

use crate::product::lifecycle_store::LifecycleStore;
use crate::product::lifecycle_store::sc_failure_evidence::{
    ScFailureEvidenceEntry, ScFailureEvidenceError, ScFailureEvidenceEvent, sc_failure_chain_id,
};
use crate::product::work_item_plan_source_store::{
    SourceRevisionRecord, SourceStoreScope, WorkItemPlanSourceStore,
};
use crate::web::workspace_ws_types::{
    ScEvidenceRef, ScFailureChainIdentity, ScFailureClass, ScFailureScope, ScPromptDisposition,
    ScRoundIdentity, ScTeachingPromptEvidence,
};

/// 精确正文：JSON 转义/反斜杠/引号/换行转义/CJK 必须逐字往返（不重编码）。
const RAW_EXACT: &str =
    r#"{"k":"v\u0041","path":"c:\\dir","quote":"a\"b","nl":"line1\nline2","cjk":"複雑"}"#;

/// 独立 oracle（§2.3 冻结配方）：四段各用 u64 **大端**字节长度前缀 + UTF-8 正文，
/// seq 先十进制字符串化再加前缀，无额外 domain tag。刻意用整块 buffer + 一次性
/// digest（生产实现是流式 update），避免"实现对自己"的同形验证。
fn oracle_chain_id(session: &str, business_key: &str, node: &str, seq: u64) -> String {
    let seq_text = seq.to_string();
    let mut buffer: Vec<u8> = Vec::new();
    for part in [
        session.as_bytes(),
        business_key.as_bytes(),
        node.as_bytes(),
        seq_text.as_bytes(),
    ] {
        buffer.extend_from_slice(&(part.len() as u64).to_be_bytes());
        buffer.extend_from_slice(part);
    }
    let digest = Sha256::digest(&buffer);
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("chain-v1-{hex}")
}

fn bind(store: &LifecycleStore, chain: &ScFailureChainIdentity, round: &ScRoundIdentity) {
    store
        .append_sc_failure_evidence(
            chain,
            &ScFailureEvidenceEvent::RoundBound {
                round: round.clone(),
            },
        )
        .unwrap();
}

fn put(
    store: &LifecycleStore,
    chain: &ScFailureChainIdentity,
    round_id: &str,
    ordinal: u32,
    text: &str,
    disposition: ScPromptDisposition,
) -> Result<ScTeachingPromptEvidence, ScFailureEvidenceError> {
    store.put_sc_teaching_prompt(
        chain,
        round_id,
        ordinal,
        "builder_compile_0001",
        text,
        disposition,
    )
}

/// locator/身份类拒绝的统一面：code、step、round 归属、非不完整发布、无前缀。
fn assert_locator_invalid(err: &ScFailureEvidenceError, step: &str, round_id: Option<&str>) {
    assert_eq!(
        err.code, "sc_failure_locator_invalid",
        "detail={}",
        err.detail
    );
    assert_eq!(err.step, step);
    assert_eq!(err.round_id.as_deref(), round_id);
    assert!(!err.publication_incomplete);
    assert!(err.verified_prefix.is_none());
}

#[test]
fn sc_failure_boundary_chain_identity_oracle_is_length_prefixed() {
    // 长度前缀消歧：("ab","c") 与 ("a","bc") 不得同键；四段都参与且不可互换。
    assert_eq!(
        sc_failure_chain_id("s", "ab", "c", 7).unwrap(),
        oracle_chain_id("s", "ab", "c", 7)
    );
    // 已知答案：按 §2.3 配方独立算出的固定摘要（不是从生产实现复制，也不是随意 re-pin）。
    assert_eq!(
        sc_failure_chain_id("s", "b", "n", 1).unwrap(),
        "chain-v1-587e8ed5c9e89def68c1fb9e48037e9a7887bc8855a56c263294a0a3061febe6"
    );
    assert_eq!(
        sc_failure_chain_id("s", "ab", "c", 7).unwrap(),
        "chain-v1-8654d763b33f6e9ab155c1ebd21f089246ec8fdc84712ce4fa8ebd81ae74af47"
    );
    // 业务序号 0 合法（首轮 split run 的 role_run_seq 就是 0）：不得要求 >0。
    assert_eq!(
        sc_failure_chain_id("s", "b", "n", 0).unwrap(),
        "chain-v1-c69e85db83bb94fc7c30e8df2fe0d67ac78d389c620f900682df2d4d14286887"
    );
    assert_ne!(
        sc_failure_chain_id("s", "ab", "c", 7).unwrap(),
        sc_failure_chain_id("s", "a", "bc", 7).unwrap()
    );
    let base = oracle_chain_id("s", "b", "n", 1);
    assert_ne!(base, oracle_chain_id("s", "b", "n", 2));
    assert_ne!(base, oracle_chain_id("b", "s", "n", 1));
    assert_ne!(base, oracle_chain_id("s", "n", "b", 1));
    let id = sc_failure_chain_id("s", "b", "n", 1).unwrap();
    assert!(id.starts_with("chain-v1-") && id.len() == "chain-v1-".len() + 64);
    assert!(
        id[9..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    );
    // 非法身份输入：空/路径分隔符 → 稳定身份错误，绝不静默成合法 id。
    for (s, b, n) in [
        ("", "b", "n"),
        ("s", "", "n"),
        ("s", "b", ""),
        ("s/x", "b", "n"),
        ("s", "b\\x", "n"),
    ] {
        let err = sc_failure_chain_id(s, b, n, 1).expect_err("非法身份必须拒绝");
        assert_eq!(
            (err.code.as_str(), err.step.as_str()),
            ("sc_failure_chain_invalid", "sc_failure_chain_id")
        );
        assert!(!err.publication_incomplete);
    }
    // 四元组合法但 chain_id 摘要不符：写准入即拒，不落证据分区。
    let (_tmp, store) = setup();
    let session_id = live_sc_session(&store);
    let handle = begin(&store, &session_id);
    let (chain, round) = bound_round(&store, &session_id, &handle);
    let mut tampered = chain.clone();
    tampered.chain_id = oracle_chain_id(
        &chain.scope.session_id,
        &chain.business_key,
        &chain.original_node_id,
        chain.original_role_run_seq + 1,
    );
    let err = store
        .append_sc_failure_evidence(&tampered, &ScFailureEvidenceEvent::RoundBound { round })
        .expect_err("篡改 chain_id 必须拒绝");
    assert_eq!(err.code, "sc_failure_chain_invalid");
    assert_eq!(err.step, "append_sc_failure_evidence");
    assert!(!chain_dir(&store, &tampered).join("rounds.jsonl").exists());
}

#[test]
fn sc_failure_boundary_raw_identity_tamper_rejected() {
    let (_tmp, store) = setup();
    let s1 = live_sc_session(&store);
    let h1 = begin(&store, &s1);
    complete_raw(&store, &h1, RAW_EXACT);
    let (chain1, round1) = bound_round(&store, &s1, &h1);
    let scope1 = chain1.scope.clone();
    // 正例：命中的正文是 JSON String 值逐字回读。
    assert_eq!(
        store
            .read_sc_split_output(&scope1, &round1)
            .expect("raw 精确读回"),
        RAW_EXACT
    );
    let s2 = live_sc_session(&store);
    let h2 = begin(&store, &s2);
    complete_raw(&store, &h2, RAW2);
    // ① 两个真实 handle 互换 run_ref：目录仍按 (session,seq) 命中，但 run.json 的
    //    provider_run_id 与 round.run_ref 不符 → 必须拒，不得返回 RAW_EXACT。
    let mut swapped = round1.clone();
    swapped.run_ref = h2.run_ref.clone();
    let err = store
        .read_sc_split_output(&scope1, &swapped)
        .expect_err("run_ref 与 run.json 不符必须拒绝");
    assert_locator_invalid(&err, "read_sc_split_output", Some(round1.round_id.as_str()));
    // ② 跨 session：round 声称 s2 的目录/scope，却引用 s1 的 run_ref。
    let mut cross = round1.clone();
    cross.workspace_session_id = s2.clone();
    let scope2 = ScFailureScope {
        session_id: s2.clone(),
        ..scope1.clone()
    };
    let err = store
        .read_sc_split_output(&scope2, &cross)
        .expect_err("跨 session 的 run_ref 必须拒绝");
    assert_locator_invalid(&err, "read_sc_split_output", Some(round1.round_id.as_str()));
    // ③ run.json 的 session/seq 身份被改写（盘上来源元数据不再属于该 scope）：仍须拒。
    let run_dir = store
        .work_item_split_provider_run_dir(&h1)
        .expect("run dir");
    let run_original = std::fs::read(run_dir.join("run.json")).unwrap();
    let mut run: serde_json::Value = serde_json::from_slice(&run_original).unwrap();
    run["workspace_session_id"] = serde_json::json!("session_forged_0001");
    run["role_run_seq"] = serde_json::json!(999_999);
    std::fs::write(run_dir.join("run.json"), serde_json::to_vec(&run).unwrap()).unwrap();
    let err = store
        .read_sc_split_output(&scope1, &round1)
        .expect_err("run.json 身份被篡改必须拒绝");
    assert_eq!(
        (err.code.as_str(), err.step.as_str()),
        ("sc_failure_scope_mismatch", "read_sc_split_output")
    );
    std::fs::write(run_dir.join("run.json"), &run_original).unwrap();
    // ④ scope 的 project/issue/plan 必须等于该 session 的真实归属。
    let mut bad_scope = scope1.clone();
    bad_scope.plan_id = "entity_forged_9999".to_string();
    let err = store
        .read_sc_split_output(&bad_scope, &round1)
        .expect_err("scope 与真实 session 不符必须拒绝");
    assert_eq!(err.code, "sc_failure_scope_mismatch");
    assert_eq!(err.step, "read_sc_split_output");
    assert_eq!(err.round_id.as_deref(), Some(round1.round_id.as_str()));
}

#[test]
fn sc_failure_boundary_locators_reject_traversal_versions_and_segments() {
    let (_tmp, store) = setup();
    let session_id = live_sc_session(&store);
    let handle = begin(&store, &session_id);
    complete_raw(&store, &handle, RAW1);
    let (chain, round) = bound_round(&store, &session_id, &handle);
    bind(&store, &chain, &round);
    // ① raw locator：绝对路径 / `..` / 未知版本 / 额外段都不得绑定该 round。
    let expected = raw_locator(&round);
    let bad_raw = vec![
        format!("/{expected}"),
        format!("sc-split-output/v1/{session_id}/../{}", round.role_run_seq),
        format!("{expected}/."),
        expected.replacen("sc-split-output/v1", "sc-split-output/v2", 1),
        format!("{expected}/extra"),
    ];
    for locator in bad_raw {
        let mut event = observed(&round, RAW1, RAW1);
        if let ScFailureEvidenceEvent::FailureObserved { raw, .. } = &mut event {
            *raw = available(locator.clone(), RAW1);
        }
        let err = store
            .append_sc_failure_evidence(&chain, &event)
            .expect_err("非法 raw locator 必须拒绝");
        assert_locator_invalid(
            &err,
            "append_sc_failure_evidence",
            Some(round.round_id.as_str()),
        );
    }
    assert_eq!(
        store
            .read_sc_failure_evidence(&chain)
            .unwrap()
            .entries
            .len(),
        1,
        "被拒 locator 不得落盘"
    );
    store
        .append_sc_failure_evidence(&chain, &observed(&round, RAW1, RAW1))
        .expect("正确 locator 可写");
    // ② prompt locator：读侧拒绝穿越/未知版本/额外段/跨 session/跨 chain。
    let prepared = put(
        &store,
        &chain,
        &round.round_id,
        2,
        PROMPT_TEXT,
        ScPromptDisposition::Prepared,
    )
    .expect("prompt");
    let (good, digest, _) = parts(&prepared.prompt);
    let foreign_chain =
        sc_failure_chain_id(&session_id, "business_key_sc_9999", NODE_ID, 1).unwrap();
    let bad_prompt = vec![
        format!("/{good}"),
        format!(
            "sc-teaching-prompt/v1/{session_id}/../{}/2/{digest}",
            chain.chain_id
        ),
        good.replacen("sc-teaching-prompt/v1", "sc-teaching-prompt/v2", 1),
        format!("{good}/extra"),
        format!(
            "sc-teaching-prompt/v1/other_session_0001/{}/2/{digest}",
            chain.chain_id
        ),
        format!("sc-teaching-prompt/v1/{session_id}/{foreign_chain}/2/{digest}"),
    ];
    for locator in bad_prompt {
        let err = store
            .read_sc_teaching_prompt(&chain, &locator)
            .expect_err("非法 prompt locator 必须拒绝");
        assert_locator_invalid(&err, "read_sc_teaching_prompt", None);
    }
    assert_eq!(
        store.read_sc_teaching_prompt(&chain, &good).unwrap(),
        PROMPT_TEXT
    );
}

#[test]
fn sc_failure_boundary_log_cross_scope_keeps_verified_prefix() {
    let (_tmp, store) = setup();
    let session_id = live_sc_session(&store);
    let handle = begin(&store, &session_id);
    let (chain, round) = bound_round(&store, &session_id, &handle);
    let bound = store
        .append_sc_failure_evidence(
            &chain,
            &ScFailureEvidenceEvent::RoundBound {
                round: round.clone(),
            },
        )
        .unwrap();
    let observed_entry = store
        .append_sc_failure_evidence(&chain, &observed(&round, RAW1, RAW1))
        .unwrap();
    let log_path = chain_dir(&store, &chain).join("rounds.jsonl");
    let before = std::fs::read(&log_path).unwrap();
    // 第 3 行是另一个 chain 的合法条目（seq 连续、schema 正确）→ 只能报跨 scope。
    let chain_b = ScFailureChainIdentity {
        business_key: "business_key_sc_0002".to_string(),
        chain_id: sc_failure_chain_id(&session_id, "business_key_sc_0002", NODE_ID, 1).unwrap(),
        ..chain.clone()
    };
    let foreign = ScFailureEvidenceEntry {
        schema_version: 1,
        entry_seq: 3,
        entry_key: "round/foreign".to_string(),
        chain: chain_b.clone(),
        event: ScFailureEvidenceEvent::RoundBound {
            round: ScRoundIdentity {
                chain_id: chain_b.chain_id.clone(),
                round_id: "round_foreign_0001".to_string(),
                ..round.clone()
            },
        },
    };
    let mut payload = serde_json::to_vec(&foreign).unwrap();
    payload.push(b'\n');
    std::io::Write::write_all(
        &mut std::fs::OpenOptions::new()
            .append(true)
            .open(&log_path)
            .unwrap(),
        &payload,
    )
    .unwrap();
    let err = store
        .read_sc_failure_evidence(&chain)
        .expect_err("跨 scope 行必须拒绝");
    assert_eq!(err.code, "sc_failure_scope_mismatch");
    assert_eq!(err.step, "read_sc_failure_evidence");
    assert!(err.publication_incomplete);
    let prefix = err.verified_prefix.as_deref().expect("verified prefix");
    assert_eq!(prefix.entries, vec![bound, observed_entry]);
    assert_eq!(prefix.verified_prefix_bytes, before, "前缀字节止于坏行之前");
    assert_eq!(
        prefix.evidence_through_entry_key,
        format!("failureobserved/{}", round.round_id)
    );
}

#[test]
fn sc_failure_boundary_prompt_disposition_never_changes_body() {
    let (_tmp, store) = setup();
    let session_id = live_sc_session(&store);
    let handle = begin(&store, &session_id);
    let (chain, round) = bound_round(&store, &session_id, &handle);
    bind(&store, &chain, &round);
    // 同一 ordinal：Prepared → Sent 是不同处置事实（允许），正文摘要/文件不得变。
    let prepared = put(
        &store,
        &chain,
        &round.round_id,
        2,
        PROMPT_TEXT,
        ScPromptDisposition::Prepared,
    )
    .expect("prepared");
    let sent = put(
        &store,
        &chain,
        &round.round_id,
        2,
        PROMPT_TEXT,
        ScPromptDisposition::Sent,
    )
    .expect("sent 同文");
    assert_eq!(parts(&sent.prompt).1, parts(&prepared.prompt).1);
    let read = store.read_sc_failure_evidence(&chain).unwrap();
    assert_eq!(read.entries.len(), 3);
    let dispositions: Vec<ScPromptDisposition> = read
        .entries
        .iter()
        .filter_map(|entry| match &entry.event {
            ScFailureEvidenceEvent::PromptPrepared { evidence }
            | ScFailureEvidenceEvent::PromptUnavailable { evidence } => {
                Some(evidence.disposition.clone())
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        dispositions,
        vec![ScPromptDisposition::Prepared, ScPromptDisposition::Sent]
    );
    let prompts_dir = chain_dir(&store, &chain).join("prompts");
    let files_for = |prefix: &str| {
        std::fs::read_dir(&prompts_dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                name.starts_with(prefix) && name.ends_with(".txt")
            })
            .count()
    };
    assert_eq!(files_for("2-"), 1, "同一目标 ordinal 只允许一个正文文件");
    // 另一 ordinal：换处置同时换正文 → 正文不可变必须冲突（不得因 disposition 进槽键而逃逸）。
    let prepared_three = put(
        &store,
        &chain,
        &round.round_id,
        3,
        PROMPT_TEXT,
        ScPromptDisposition::Prepared,
    )
    .expect("ord3 prepared");
    assert_eq!(prepared_three.target_round_ordinal, 3);
    let before = std::fs::read(chain_dir(&store, &chain).join("rounds.jsonl")).unwrap();
    let conflict = put(
        &store,
        &chain,
        &round.round_id,
        3,
        "不同正文 B",
        ScPromptDisposition::Sent,
    )
    .expect_err("Sent 换正文必须拒绝");
    assert_eq!(conflict.code, "sc_failure_entry_conflict");
    assert_eq!(conflict.step, "put_sc_teaching_prompt");
    assert_eq!(conflict.round_id.as_deref(), Some(round.round_id.as_str()));
    assert!(!conflict.publication_incomplete);
    assert_eq!(
        conflict.verified_prefix.as_deref().unwrap().entries.len(),
        4
    );
    assert_eq!(
        std::fs::read(chain_dir(&store, &chain).join("rounds.jsonl")).unwrap(),
        before
    );
    assert!(
        !prompts_dir
            .join(format!("3-{}.txt", sha256_hex("不同正文 B")))
            .exists(),
        "被拒正文不得落文件"
    );
}

#[test]
fn sc_failure_boundary_unbound_round_rejected_before_write() {
    let (_tmp, store) = setup();
    let session_id = live_sc_session(&store);
    let handle = begin(&store, &session_id);
    complete_raw(&store, &handle, RAW1);
    let (chain, round) = bound_round(&store, &session_id, &handle);
    let log_path = chain_dir(&store, &chain).join("rounds.jsonl");
    // 未 RoundBound 的 round：raw 存在也不得被 observational 事件写入或推断绑定。
    let err = store
        .append_sc_failure_evidence(&chain, &observed(&round, RAW1, RAW1))
        .expect_err("未 RoundBound 必须拒绝");
    assert_eq!(
        (err.code.as_str(), err.step.as_str()),
        ("sc_failure_round_not_bound", "append_sc_failure_evidence")
    );
    assert_eq!(err.round_id.as_deref(), Some(round.round_id.as_str()));
    assert!(!err.publication_incomplete);
    assert!(!log_path.exists(), "被拒事件不得落日志");
    let err = put(
        &store,
        &chain,
        &round.round_id,
        2,
        PROMPT_TEXT,
        ScPromptDisposition::Prepared,
    )
    .expect_err("教学提示的失败 round 未绑定必须拒绝");
    assert_eq!(
        (err.code.as_str(), err.step.as_str()),
        ("sc_failure_round_not_bound", "put_sc_teaching_prompt")
    );
    assert_eq!(err.round_id.as_deref(), Some(round.round_id.as_str()));
    assert!(
        !chain_dir(&store, &chain)
            .join("prompts")
            .join(format!("2-{}.txt", sha256_hex(PROMPT_TEXT)))
            .exists(),
        "被拒提示不得落文件"
    );
    assert!(
        store
            .read_sc_failure_evidence(&chain)
            .unwrap()
            .entries
            .is_empty(),
        "拒绝不得隐式生成 RoundBound"
    );
    // 显式 RoundBound 后同一事件才被接受：顺序是事实，不是推断。
    bind(&store, &chain, &round);
    assert_eq!(
        store
            .append_sc_failure_evidence(&chain, &observed(&round, RAW1, RAW1))
            .expect("绑定后可写")
            .entry_seq,
        2
    );
}

/// F1：delete 侧已持真实 scope 的独占 lease（墓碑尚未落盘）时，迟到写必须被拒，
/// 且不得 create_dir 复活已删实体的证据分区。
#[test]
fn sc_failure_boundary_late_write_blocked_by_delete_lease_on_same_scope() {
    let (_tmp, store) = setup();
    let session_id = live_sc_session(&store);
    let handle = begin(&store, &session_id);
    let (chain, round) = bound_round(&store, &session_id, &handle);
    let log_path = chain_dir(&store, &chain).join("rounds.jsonl");
    // 真实 scope 上的独占 delete lease（墓碑未落盘，只有 lease 在位）
    let _lease =
        crate::product::entity_operation_store::EntityOperationStore::new(store.app_paths())
            .try_acquire_delete(&crate::product::entity_deletion_store::EntityScope::new(
                PROJECT_ID,
                ISSUE_ID,
                crate::product::entity_deletion_store::DeletionEntityKind::Plan,
                PLAN_ID.to_string(),
            ))
            .expect("delete lease 必须在真实 scope 上取到独占");
    let err = store
        .append_sc_failure_evidence(
            &chain,
            &ScFailureEvidenceEvent::RoundBound {
                round: round.clone(),
            },
        )
        .expect_err("delete lease 在位时迟到写必须被拒");
    assert_eq!(err.code, "sc_failure_session_not_live");
    assert_eq!(err.step, "append_sc_failure_evidence");
    assert!(err.publication_incomplete);
    assert!(!log_path.exists(), "被拒写不得复活证据分区");
    // prompt 写受同一 lease 约束
    let err = put(
        &store,
        &chain,
        &round.round_id,
        2,
        PROMPT_TEXT,
        ScPromptDisposition::Prepared,
    )
    .expect_err("delete lease 在位时迟到 prompt 写必须被拒");
    assert_eq!(err.code, "sc_failure_session_not_live");
    assert_eq!(err.step, "put_sc_teaching_prompt");
    assert!(
        !chain_dir(&store, &chain)
            .join("prompts")
            .join(format!("2-{}.txt", sha256_hex(PROMPT_TEXT)))
            .exists(),
        "被拒 prompt 不得落文件"
    );
}

/// F4：RoundBound 必须钉住本会话的 round：跨会话 round 或身份不完整（空
/// round_id / 空 run_ref）都不得写入——写进去的证据本 store 永远无法核验。
#[test]
fn sc_failure_boundary_round_bound_rejects_foreign_or_incomplete_round() {
    let (_tmp, store) = setup();
    let session_id = live_sc_session(&store);
    let handle = begin(&store, &session_id);
    let (chain, round) = bound_round(&store, &session_id, &handle);
    let log_path = chain_dir(&store, &chain).join("rounds.jsonl");
    // ① 跨会话 round：写后本 store 的读接口永远无法核验 → 必须拒
    let mut foreign = round.clone();
    foreign.workspace_session_id = "session_foreign_0001".to_string();
    let err = store
        .append_sc_failure_evidence(
            &chain,
            &ScFailureEvidenceEvent::RoundBound {
                round: foreign.clone(),
            },
        )
        .expect_err("跨会话 round 必须拒绝");
    assert_eq!(err.code, "sc_failure_scope_mismatch");
    assert_eq!(err.step, "append_sc_failure_evidence");
    assert_eq!(err.round_id.as_deref(), Some(foreign.round_id.as_str()));
    assert!(!err.publication_incomplete);
    assert!(!log_path.exists(), "被拒 round 不得落日志");
    // ② 身份不完整：空 round_id / 空 run_ref → 必须拒
    let mut empty_round_id = round.clone();
    empty_round_id.round_id = String::new();
    let err = store
        .append_sc_failure_evidence(
            &chain,
            &ScFailureEvidenceEvent::RoundBound {
                round: empty_round_id,
            },
        )
        .expect_err("空 round_id 必须拒绝");
    assert_eq!(err.code, "sc_failure_locator_invalid");
    let mut empty_run_ref = round.clone();
    empty_run_ref.run_ref = String::new();
    let err = store
        .append_sc_failure_evidence(
            &chain,
            &ScFailureEvidenceEvent::RoundBound {
                round: empty_run_ref,
            },
        )
        .expect_err("空 run_ref 必须拒绝");
    assert_eq!(err.code, "sc_failure_locator_invalid");
    assert!(!log_path.exists());
    // ③ 本会话的真实 round 仍可写（守卫不得做成全拒）
    store
        .append_sc_failure_evidence(
            &chain,
            &ScFailureEvidenceEvent::RoundBound {
                round: round.clone(),
            },
        )
        .expect("本会话 round 必须可写");
    assert!(log_path.exists());
}

/// 结构门失败（F5）：source 是失败自身的 canonical revision（不是本轮 split
/// locator）——真实 put_source_revision 的 canonical ref + 本轮真实 split raw 必须
/// 可写可读；跨 plan 的 revision、摘要/bytes 与真实 revision 不符的 source 必须拒。
#[test]
fn sc_failure_boundary_structure_gate_source_is_canonical_revision() {
    let (_tmp, store) = setup();
    let session_id = live_sc_session(&store);
    let handle = begin(&store, &session_id);
    complete_raw(&store, &handle, RAW1);
    let (chain, round) = bound_round(&store, &session_id, &handle);
    bind(&store, &chain, &round);
    let source_store = WorkItemPlanSourceStore::new(store.app_paths());
    let scope = SourceStoreScope {
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: PLAN_ID.to_string(),
    };
    // 本 plan scope 的真实 canonical source revision
    let source_text = "计划正文第一版\n第二行";
    let mut revision = SourceRevisionRecord {
        id: "source-0001".to_string(),
        source: source_text.to_string(),
        source_revision_hash: sha256_hex(source_text),
        content_hash: String::new(),
    };
    revision.content_hash = revision.content_hash().expect("content hash");
    let canonical_ref = source_store
        .put_source_revision(PROJECT_ID, ISSUE_ID, PLAN_ID, &revision)
        .expect("put source revision");
    let stored = source_store
        .get_source_revision(&scope, &canonical_ref)
        .expect("get source revision");
    // 结构门失败的 source 形状：canonical revision ref + 真实 revision 的 hash/bytes
    let structure_event = |locator: &str, sha256: &str, bytes: u64| {
        let mut event = observed(&round, RAW1, RAW1);
        if let ScFailureEvidenceEvent::FailureObserved {
            failure_class,
            source,
            ..
        } = &mut event
        {
            *failure_class = Some(ScFailureClass::StructureGate);
            *source = ScEvidenceRef::Available {
                locator: locator.to_string(),
                sha256: sha256.to_string(),
                bytes,
            };
        }
        event
    };
    // ① 跨 plan 的 canonical revision：scope 不符 → 必须拒
    let foreign_text = "别的 plan 的正文";
    let mut foreign = SourceRevisionRecord {
        id: "source-foreign-0001".to_string(),
        source: foreign_text.to_string(),
        source_revision_hash: sha256_hex(foreign_text),
        content_hash: String::new(),
    };
    foreign.content_hash = foreign.content_hash().expect("content hash");
    let foreign_ref = source_store
        .put_source_revision(PROJECT_ID, ISSUE_ID, "entity_9999", &foreign)
        .expect("put foreign source revision");
    let err = store
        .append_sc_failure_evidence(
            &chain,
            &structure_event(
                &foreign_ref,
                &foreign.source_revision_hash,
                foreign.source.len() as u64,
            ),
        )
        .expect_err("跨 plan 的 source revision 必须拒绝");
    assert_eq!(err.code, "sc_failure_locator_invalid");
    assert_eq!(err.step, "append_sc_failure_evidence");
    assert_eq!(err.round_id.as_deref(), Some(round.round_id.as_str()));
    // ② 摘要与真实 revision 不符 → 必须拒
    let err = store
        .append_sc_failure_evidence(
            &chain,
            &structure_event(&canonical_ref, &"0".repeat(64), stored.source.len() as u64),
        )
        .expect_err("摘要被篡改的 source 必须拒绝");
    assert_eq!(err.code, "sc_failure_locator_invalid");
    // ③ bytes 与真实 revision 不符 → 必须拒
    let err = store
        .append_sc_failure_evidence(
            &chain,
            &structure_event(
                &canonical_ref,
                &stored.source_revision_hash,
                stored.source.len() as u64 + 1,
            ),
        )
        .expect_err("bytes 被篡改的 source 必须拒绝");
    assert_eq!(err.code, "sc_failure_locator_invalid");
    // ④ 正例：真实 canonical revision + 本轮真实 split raw → 必须可写可读
    store
        .append_sc_failure_evidence(
            &chain,
            &structure_event(
                &canonical_ref,
                &stored.source_revision_hash,
                stored.source.len() as u64,
            ),
        )
        .expect("结构门 source 必须是真实 canonical revision");
    let read = store.read_sc_failure_evidence(&chain).expect("read");
    let recorded = read
        .entries
        .iter()
        .find_map(|entry| match &entry.event {
            ScFailureEvidenceEvent::FailureObserved { source, .. } => Some(source),
            _ => None,
        })
        .expect("observed 条目必须可读回");
    match recorded {
        ScEvidenceRef::Available {
            locator,
            sha256,
            bytes,
        } => {
            assert_eq!(locator, &canonical_ref);
            assert_eq!(sha256, &stored.source_revision_hash);
            assert_eq!(*bytes, stored.source.len() as u64);
        }
        ScEvidenceRef::Missing { .. } => {
            panic!("结构门 source 必须是 Available canonical revision")
        }
    }
}
