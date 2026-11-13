// P05 lookup 真实查找：store 私有分区枚举 + 严格读回。
// 落位：src/product/lifecycle_store/tests/sc_failure_round_lookup.rs
// 由 sc_failure_evidence.rs 以 #[path] 挂载。
//
// 约束（Plan P05 lookup / §2.4）：身份只来自真实 store / 真实 handle（不猜最新
// 目录、不解析不透明 run_ref）；查找只读（不写、不建目录、不分配 seq、不读
// provider 产物）；坏日志不得静默跳过（带已验证前缀报错）；多命中必须报错
// （不得挑"最新"）。

use super::fixture::*;

const BUSINESS_KEY_SECOND: &str = "business_key_sc_0002";
const NODE_ID_SECOND: &str = "node_original_0002";

/// 真实 begin：provider 参与 run 记录，但 handle 只带 seq/run_ref。
fn begin_with(
    store: &LifecycleStore,
    session_id: &str,
    provider: ProviderName,
) -> WorkItemSplitProviderRunHandle {
    store
        .begin_work_item_split_provider_run(PROJECT_ID, ISSUE_ID, &provider, session_id)
        .expect("begin split run")
}

/// 真实会话的 scope（project/issue/plan 与会话身份一致）。
fn scope_of(session_id: &str) -> ScFailureScope {
    ScFailureScope {
        session_id: session_id.to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: PLAN_ID.to_string(),
    }
}

/// chain/round 全部由真实 handle 构造（chain_id 用冻结的长度前缀哈希）。
fn chain_and_round(
    session_id: &str,
    business_key: &str,
    original_node_id: &str,
    handle: &WorkItemSplitProviderRunHandle,
    provider: ProviderName,
) -> (ScFailureChainIdentity, ScRoundIdentity) {
    let chain = ScFailureChainIdentity {
        scope: scope_of(session_id),
        chain_id: sc_failure_chain_id(
            session_id,
            business_key,
            original_node_id,
            handle.role_run_seq,
        )
        .expect("chain id"),
        business_key: business_key.to_string(),
        original_node_id: original_node_id.to_string(),
        original_role_run_seq: handle.role_run_seq,
    };
    let round = ScRoundIdentity {
        chain_id: chain.chain_id.clone(),
        round_id: handle.run_ref.clone(),
        node_id: original_node_id.to_string(),
        round_ordinal: 1,
        provider,
        workspace_session_id: handle.workspace_session_id.clone(),
        role_run_seq: handle.role_run_seq,
        run_ref: handle.run_ref.clone(),
    };
    (chain, round)
}

/// 绑定一轮（真实追加 RoundBound）。
fn bind(store: &LifecycleStore, chain: &ScFailureChainIdentity, round: &ScRoundIdentity) {
    let event = ScFailureEvidenceEvent::RoundBound {
        round: round.clone(),
    };
    store
        .append_sc_failure_evidence(chain, &event)
        .expect("append round bound");
}

/// 证据分区目录名（只读列举，不创建）。
fn partition_dirs(root: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = match std::fs::read_dir(root) {
        Ok(entries) => entries
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect(),
        Err(_) => Vec::new(),
    };
    names.sort();
    names
}

/// 重建 store 后按真实身份找回已绑定轮；未命中返回 None 且零副作用。
#[test]
fn sc_failure_round_lookup_finds_bound_round_after_store_rebuild() {
    let (_tmp, store) = setup();
    let session_id = live_sc_session(&store);
    let scope = scope_of(&session_id);

    // 两条真实链：不同 seq、不同 provider、不同业务键与节点。
    let first_handle = begin_with(&store, &session_id, ProviderName::Codex);
    let second_handle = begin_with(&store, &session_id, ProviderName::ClaudeCode);
    assert!(
        first_handle.role_run_seq < second_handle.role_run_seq,
        "真实 begin 必须分配递增 seq"
    );
    let (first_chain, first_round) = chain_and_round(
        &session_id,
        BUSINESS_KEY,
        NODE_ID,
        &first_handle,
        ProviderName::Codex,
    );
    let (second_chain, second_round) = chain_and_round(
        &session_id,
        BUSINESS_KEY_SECOND,
        NODE_ID_SECOND,
        &second_handle,
        ProviderName::ClaudeCode,
    );
    bind(&store, &first_chain, &first_round);
    bind(&store, &second_chain, &second_round);

    let first_log = log_bytes(&store, &first_chain);
    let second_log = log_bytes(&store, &second_chain);
    let evidence_root = store
        .workspace_timeline_root_for_session(&session_id)
        .expect("timeline root")
        .join("sc-failure-evidence");
    let partitions_before = partition_dirs(&evidence_root);

    // 重建 store 后查找：命中的是真实身份（node/ordinal/provider 来自日志）。
    let reopened = LifecycleStore::new(store.app_paths());
    let found = reopened
        .find_sc_failure_round(&scope, BUSINESS_KEY, NODE_ID, &first_round.node_id, 1)
        .expect("lookup");
    let (found_chain, found_round) = found.expect("必须命中真实轮");
    assert_eq!(found_chain, first_chain);
    assert_eq!(found_round, first_round);
    assert_eq!(found_round.node_id, first_round.node_id);
    assert_eq!(found_round.round_ordinal, 1);
    assert_eq!(found_round.provider, ProviderName::Codex);

    // 未命中：另一条链的轮、错 ordinal、未知业务键。
    let misses: [(&str, &str, &String, u32); 3] = [
        (BUSINESS_KEY, NODE_ID, &second_round.node_id, 1),
        (BUSINESS_KEY, NODE_ID, &first_round.node_id, 2),
        (
            "business_key_missing_0003",
            NODE_ID,
            &first_round.node_id,
            1,
        ),
    ];
    for (business_key, original_node_id, node_id, ordinal) in misses {
        assert!(
            reopened
                .find_sc_failure_round(&scope, business_key, original_node_id, node_id, ordinal)
                .expect("lookup")
                .is_none(),
            "未命中必须返回 None"
        );
    }

    // 只读：日志字节与分区目录零变化。
    assert_eq!(log_bytes(&store, &first_chain), first_log);
    assert_eq!(log_bytes(&store, &second_chain), second_log);
    assert_eq!(partition_dirs(&evidence_root), partitions_before);

    // 不分配 seq：查找后的真实 begin 继续按高水位递增。
    let after_handle = begin_with(&store, &session_id, ProviderName::Codex);
    assert_eq!(
        after_handle.role_run_seq,
        second_handle.role_run_seq + 1,
        "查找不得消费全局 seq 分配"
    );
}

/// 候选链日志坏行：停下报错并带已验证前缀（不得静默跳过候选）。
#[test]
fn sc_failure_round_lookup_reports_corrupt_candidate_log() {
    let (_tmp, store) = setup();
    let session_id = live_sc_session(&store);
    let scope = scope_of(&session_id);

    let handle = begin_with(&store, &session_id, ProviderName::Codex);
    let (chain, round) = chain_and_round(
        &session_id,
        BUSINESS_KEY,
        NODE_ID,
        &handle,
        ProviderName::Codex,
    );
    let bound = store
        .append_sc_failure_evidence(
            &chain,
            &ScFailureEvidenceEvent::RoundBound {
                round: round.clone(),
            },
        )
        .expect("append round bound");
    store
        .append_sc_failure_evidence(&chain, &observed(&round, RAW1, SOURCE1))
        .expect("append failure observed");

    // 坏行落在首行之后：已验证前缀保留，坏行之后的行不得被跳过吞下。
    let good = log_bytes(&store, &chain);
    let first_line = String::from_utf8(good)
        .expect("utf8 log")
        .lines()
        .next()
        .expect("first line")
        .to_string();
    std::fs::write(
        chain_dir(&store, &chain).join("rounds.jsonl"),
        format!("{first_line}\n{{\"schema_version\":1,\"entry_seq\":\n"),
    )
    .expect("write corrupt log");

    let reopened = LifecycleStore::new(store.app_paths());
    let err = reopened
        .find_sc_failure_round(&scope, BUSINESS_KEY, NODE_ID, &round.node_id, 1)
        .expect_err("坏日志必须报错");
    let prefix = expect_prefix(&err, "sc_failure_log_corrupt");
    assert_eq!(prefix.entries, vec![bound]);
    assert!(err.publication_incomplete);
}

/// 同业务键 + 同原始节点的两条真实链同时命中 → 报错（不得挑"最新"）。
#[test]
fn sc_failure_round_lookup_rejects_ambiguous_matches() {
    let (_tmp, store) = setup();
    let session_id = live_sc_session(&store);
    let scope = scope_of(&session_id);

    let first_handle = begin_with(&store, &session_id, ProviderName::Codex);
    let retry_handle = begin_with(&store, &session_id, ProviderName::ClaudeCode);
    let (first_chain, first_round) = chain_and_round(
        &session_id,
        BUSINESS_KEY,
        NODE_ID,
        &first_handle,
        ProviderName::Codex,
    );
    let (retry_chain, retry_round) = chain_and_round(
        &session_id,
        BUSINESS_KEY,
        NODE_ID,
        &retry_handle,
        ProviderName::ClaudeCode,
    );
    bind(&store, &first_chain, &first_round);
    bind(&store, &retry_chain, &retry_round);

    let reopened = LifecycleStore::new(store.app_paths());
    let err = reopened
        .find_sc_failure_round(&scope, BUSINESS_KEY, NODE_ID, &first_round.node_id, 1)
        .expect_err("多命中必须报错");
    assert_eq!(err.code, "sc_failure_lookup_ambiguous");
    assert_eq!(err.step, "find_sc_failure_round");
    assert_eq!(err.round_id, None);
}
