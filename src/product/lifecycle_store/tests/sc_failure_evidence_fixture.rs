// P05t 共享 fixture：常量 + 真实 store / handle 构造助手。
// 落位：src/product/lifecycle_store/tests/sc_failure_evidence_fixture.rs
// 由主测试模块 tests/sc_failure_evidence.rs 以 #[path] 挂载并整体 glob 引入。
// 内容自 P05 RED 测试原样搬迁，只把 item 可见性放宽到 pub(crate) 以便跨文件引用。

pub(crate) use std::path::PathBuf;

pub(crate) use tempfile::TempDir;

pub(crate) use crate::product::app_paths::ProductAppPaths;
pub(crate) use crate::product::lifecycle_store::sc_failure_evidence::{
    ScFailureEvidenceError, ScFailureEvidenceEvent, ScFailureEvidenceRead, sc_failure_chain_id,
};
pub(crate) use crate::product::lifecycle_store::{
    CreateWorkspaceSessionInput, LifecycleStore, WorkItemPlanSessionOptions,
};
pub(crate) use crate::product::models::{ProviderName, WorkspaceType};
pub(crate) use crate::product::work_item_plan_policy::{RunPolicy, WorkItemPlanFlowKind};
pub(crate) use crate::product::work_item_split_engine::parse::WorkItemSplitProviderRunHandle;
pub(crate) use crate::web::workspace_ws_types::{
    ScEvidenceRef, ScFailureChainIdentity, ScFailureClass, ScFailureScope, ScPromptDisposition,
    ScRoundIdentity, ScTeachingPromptEvidence,
};

pub(crate) const PROJECT_ID: &str = "project_0001";
pub(crate) const ISSUE_ID: &str = "issue_0001";
pub(crate) const BUSINESS_KEY: &str = "business_key_sc_0001";
pub(crate) const PLAN_ID: &str = "entity_0001";
pub(crate) const NODE_ID: &str = "node_original_0001";
pub(crate) const RAW1: &str = "provider raw 1：{\"plan\":\"候选一\"}\n第二行 tab\t quotes \"x\"";
pub(crate) const RAW2: &str = "结构差异 raw 2：ir_preflight 拒绝\r\n末尾空格与 CJK 複雑 字符";
pub(crate) const SOURCE1: &str = "normalized author delivery source 1（去 preamble 后）";
pub(crate) const PROMPT_TEXT: &str = "教学提示：先修 A 再修 B\n尾部 ❯";

pub(crate) fn setup() -> (TempDir, LifecycleStore) {
    let tmp = TempDir::new().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    (tmp, store)
}

/// 真实 SC session（single-candidate 流，实体归属 Plan），与既有 SC fixture 同构。
pub(crate) fn live_sc_session(store: &LifecycleStore) -> String {
    store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: "entity_0001".to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Codex,
            reviewer_provider: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: Some(WorkItemPlanSessionOptions {
                flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                run_policy: RunPolicy::Interactive,
                rollout_snapshot: true,
            }),
        })
        .expect("SC session")
        .id
}

/// handle/seq 只来自真实 begin，不猜最新目录。
pub(crate) fn begin(store: &LifecycleStore, session_id: &str) -> WorkItemSplitProviderRunHandle {
    store
        .begin_work_item_split_provider_run(PROJECT_ID, ISSUE_ID, &ProviderName::Codex, session_id)
        .expect("begin")
}

pub(crate) fn complete_raw(
    store: &LifecycleStore,
    handle: &WorkItemSplitProviderRunHandle,
    raw: &str,
) {
    store
        .complete_work_item_split_provider_run(
            handle,
            "split prompt",
            &serde_json::Value::String(raw.into()),
        )
        .expect("complete")
}

/// chain/round 全部由真实 handle 构造（locator 不解析不透明 run_ref 猜路径）。
pub(crate) fn bound_round(
    _store: &LifecycleStore,
    session_id: &str,
    handle: &WorkItemSplitProviderRunHandle,
) -> (ScFailureChainIdentity, ScRoundIdentity) {
    let chain = ScFailureChainIdentity {
        scope: ScFailureScope {
            session_id: session_id.to_string(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: PLAN_ID.to_string(),
        },
        chain_id: sc_failure_chain_id(session_id, BUSINESS_KEY, NODE_ID, handle.role_run_seq)
            .expect("chain id"),
        business_key: BUSINESS_KEY.to_string(),
        original_node_id: NODE_ID.to_string(),
        original_role_run_seq: handle.role_run_seq,
    };
    let round = ScRoundIdentity {
        chain_id: chain.chain_id.clone(),
        round_id: handle.run_ref.clone(),
        node_id: NODE_ID.to_string(),
        round_ordinal: 1,
        provider: ProviderName::Codex,
        workspace_session_id: handle.workspace_session_id.clone(),
        role_run_seq: handle.role_run_seq,
        run_ref: handle.run_ref.clone(),
    };
    (chain, round)
}

pub(crate) fn raw_locator(round: &ScRoundIdentity) -> String {
    format!(
        "sc-split-output/v1/{}/{}",
        round.workspace_session_id, round.role_run_seq
    )
}

pub(crate) fn available(locator: String, body: &str) -> ScEvidenceRef {
    ScEvidenceRef::Available {
        locator,
        sha256: sha256_hex(body),
        bytes: body.len() as u64,
    }
}

/// §2.2：raw 与 source 共用同一 split locator，摘要/bytes 计量不同正文。
pub(crate) fn observed(
    round: &ScRoundIdentity,
    raw_body: &str,
    source_body: &str,
) -> ScFailureEvidenceEvent {
    let locator = raw_locator(round);
    ScFailureEvidenceEvent::FailureObserved {
        round_id: round.round_id.clone(),
        failure_class: Some(ScFailureClass::Compile),
        blocking_reasons: vec!["compile_error:line 3".to_string()],
        repair_instructions: vec!["先修 A".to_string()],
        raw: available(locator.clone(), raw_body),
        source: available(locator, source_body),
        existing_failure_id: Some("failure_0001".to_string()),
        attempt_key: Some("attempt_0001".to_string()),
    }
}

pub(crate) fn chain_dir(store: &LifecycleStore, chain: &ScFailureChainIdentity) -> PathBuf {
    store
        .workspace_timeline_root_for_session(&chain.scope.session_id)
        .expect("timeline root")
        .join("sc-failure-evidence")
        .join(&chain.chain_id)
}

pub(crate) fn log_bytes(store: &LifecycleStore, chain: &ScFailureChainIdentity) -> Vec<u8> {
    std::fs::read(chain_dir(store, chain).join("rounds.jsonl")).expect("read log")
}

/// 只改冻结字段 entry_seq（断号造例）。
pub(crate) fn with_entry_seq(line: &str, entry_seq: u64) -> String {
    let mut value: serde_json::Value = serde_json::from_str(line).expect("entry");
    value["entry_seq"] = serde_json::json!(entry_seq);
    serde_json::to_string(&value).expect("serialize")
}

pub(crate) fn parts(reference: &ScEvidenceRef) -> (String, String, u64) {
    match reference {
        ScEvidenceRef::Available {
            locator,
            sha256,
            bytes,
        } => (locator.clone(), sha256.clone(), *bytes),
        ScEvidenceRef::Missing { code, .. } => panic!("expected available, got {code}"),
    }
}

/// 错误必须带稳定 code/step 与可消费的已验证前缀（坏行不被跳过）。
pub(crate) fn expect_prefix<'a>(
    err: &'a ScFailureEvidenceError,
    code: &str,
) -> &'a ScFailureEvidenceRead {
    assert_eq!(err.code, code, "detail={}", err.detail);
    assert!(!err.step.is_empty());
    err.verified_prefix.as_deref().expect("verified prefix")
}

pub(crate) fn sha256_hex(body: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(body.as_bytes());
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
