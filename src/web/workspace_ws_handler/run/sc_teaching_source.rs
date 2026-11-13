// P20 RED（实现未落地态）：受控完整失败 source resolver（内部 source-input 切面）。
#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::lifecycle_store::{
        sc_failure_evidence::{ScFailureEvidenceError, sc_failure_chain_id, sc_failure_split_locator},
        CreateWorkspaceSessionInput, LifecycleStore, WorkItemPlanSessionOptions,
    };
    use crate::product::work_item_plan_source_store::{SourceRevisionRecord, WorkItemPlanSourceStore};
    use crate::product::work_item_split_engine::parse::WorkItemSplitProviderRunHandle;
    use crate::product::{
        models::{ProviderName, WorkspaceType},
        work_item_plan_policy::{RunPolicy, ScTeachingWindowRequest, WorkItemPlanFlowKind} };
    use crate::web::workspace_ws_types::{
        ScEvidenceRef, ScFailureClass, ScFailureScope, ScRoundIdentity, ScTeachingSourceOrigin };
    use sha2::Digest as _;
    use tempfile::TempDir;

    const PROJECT_ID: &str = "project_0001"; const ISSUE_ID: &str = "issue_0001";
    const PLAN_ID: &str = "entity_0001"; const BUSINESS_KEY: &str = "business_key_sc_0001";
    const NODE_ID: &str = "node_original_0001";
    // 真实差异样本：前言 + 固定词表中文标题 + EARS 邻接空白 + CJK 尾部空白。
    const RAW: &str = "以下是修订后的计划，请审阅：\n# 工作项计划\n## 工作项 WI-001: x\n### 溯源清单\n- statement: WHENthe levels API receives GET /api/levels\nthe system SHALL return 200\n尾部空白 与 CJK 複雑 字符   \n";

    fn sha256_hex(body: &str) -> String { hex::encode(sha2::Sha256::digest(body.as_bytes())) }

    fn setup() -> (TempDir, LifecycleStore) {
        let tmp = TempDir::new().unwrap();
        let store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
        (tmp, store)
    }

    fn live_sc_session(store: &LifecycleStore) -> String {
        store.create_workspace_session(CreateWorkspaceSessionInput {
            project_id: PROJECT_ID.to_string(), issue_id: ISSUE_ID.to_string(),
            entity_id: PLAN_ID.to_string(), workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Codex, reviewer_provider: Some(ProviderName::ClaudeCode),
            review_rounds: 1, superpowers_enabled: false, openspec_enabled: false,
            work_item_plan_options: Some(WorkItemPlanSessionOptions {
                flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                run_policy: RunPolicy::Interactive, rollout_snapshot: true,
            }),
        }).expect("SC session").id
    }

    fn begin(store: &LifecycleStore, session: &str) -> WorkItemSplitProviderRunHandle {
        store.begin_work_item_split_provider_run(PROJECT_ID, ISSUE_ID, &ProviderName::Codex, session).expect("begin")
    }

    fn complete_raw(store: &LifecycleStore, h: &WorkItemSplitProviderRunHandle, raw: &str) {
        store.complete_work_item_split_provider_run(h, "split prompt",
            &serde_json::Value::String(raw.into())).expect("complete raw");
    }

    fn scope(s: &str) -> ScFailureScope {
        ScFailureScope { session_id: s.to_string(), project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(), plan_id: PLAN_ID.to_string() }
    }

    fn round_identity(s: &str, h: &WorkItemSplitProviderRunHandle) -> ScRoundIdentity {
        ScRoundIdentity {
            chain_id: sc_failure_chain_id(s, BUSINESS_KEY, NODE_ID, h.role_run_seq).expect("chain id"),
            round_id: h.run_ref.clone(), node_id: NODE_ID.to_string(), round_ordinal: 1,
            provider: ProviderName::Codex, workspace_session_id: h.workspace_session_id.clone(),
            role_run_seq: h.role_run_seq, run_ref: h.run_ref.clone(),
        }
    }

    fn available(locator: String, body: &str) -> ScEvidenceRef {
        ScEvidenceRef::Available { locator, sha256: sha256_hex(body), bytes: body.len() as u64 }
    }

    /// P05 冻结语义：raw 与 source 共用同一 split locator，摘要/bytes 计量不同正文。
    fn refs(id: &ScRoundIdentity, raw: &str, source: &str) -> (ScEvidenceRef, ScEvidenceRef) {
        let locator = sc_failure_split_locator(&id.workspace_session_id, id.role_run_seq);
        (available(locator.clone(), raw), available(locator, source))
    }

    /// 直接用真实 round 引用拼装 resolver 输入；无 From shim、无默认教学决定。
    fn input<'a>(
        identity: &'a ScRoundIdentity, class: &'a ScFailureClass,
        raw: &'a ScEvidenceRef, source: &'a ScEvidenceRef,
    ) -> ScTeachingSourceInput<'a> {
        ScTeachingSourceInput { identity, failure_class: class, raw, source }
    }

    /// 生产链真实归一化：实现必须调用同一函数，不得自造 normalizer。
    fn normalized_source(raw: &str) -> String {
        super::super::single_candidate_compile_preflight::prepare_author_delivery_for_compile(raw)
            .source
    }

    fn revision(id: &str, source: &str) -> SourceRevisionRecord {
        let mut r = SourceRevisionRecord { id: id.to_string(), source: source.to_string(),
            source_revision_hash: sha256_hex(source), content_hash: String::new() };
        r.content_hash = r.content_hash().expect("content hash"); r
    }

    fn teaching_request(reference: Option<&str>, digest: &str) -> ScTeachingWindowRequest {
        ScTeachingWindowRequest {
            provider_path_admissible: true, failure_source_ref: reference.map(str::to_string),
            failure_source_sha256: digest.to_string(),
            // resolver 不得在此判资格：空 typed kinds 也必须通过 source 校验。
            failure_typed_kinds: Vec::new(), business_request_digest: "business".to_string(),
            material_digests: Vec::new(), context_digest: "context".to_string(),
            allowed_edit_closure: "src".to_string(), instruction_digest: "instruction".to_string(),
            prompt_ref: "builder".to_string(), prompt_digest: "prompt".to_string(),
        }
    }

    fn reject_code(r: Result<ScResolvedTeachingSource, ScFailureEvidenceError>) -> String {
        r.expect_err("必须拒绝").code
    }

    #[test]
    fn compile_and_ir_rounds_resolve_normalized_source_and_bind_request() {
        let (_tmp, store) = setup();
        let session = live_sc_session(&store);
        let handle = begin(&store, &session);
        complete_raw(&store, &handle, RAW);
        let identity = round_identity(&session, &handle);
        let scope = scope(&session);
        let resolve = |i: &ScTeachingSourceInput<'_>| resolve_sc_teaching_source(&store, &scope, i);
        let locator = sc_failure_split_locator(&session, identity.role_run_seq);
        let normalized = normalized_source(RAW);
        let (raw, source) = refs(&identity, RAW, &normalized);
        let class = ScFailureClass::Compile; let round = input(&identity, &class, &raw, &source);
        let verify = |i: &ScTeachingSourceInput<'_>, r: Option<&str>, d: &str| {
            verify_sc_teaching_request_sources(&store, &scope, i, &teaching_request(r, d))
        };

        let resolved = resolve(&round).expect("resolver");
        assert_eq!(resolved.text, normalized, "消费者拿到归一化失败源全文");
        assert_ne!(resolved.text, RAW, "不得直接注入 raw");
        assert_eq!(resolved.sha256, sha256_hex(&normalized));
        assert_eq!(resolved.bytes, normalized.len() as u64);
        assert_eq!(resolved.failure_source_ref, locator);
        assert_eq!(resolved.origin, ScTeachingSourceOrigin::CompileSplitOutput);

        let ir_class = ScFailureClass::IrPreflight;
        assert_eq!(resolve(&input(&identity, &ir_class, &raw, &source)).unwrap().origin,
            ScTeachingSourceOrigin::IrPreflightSplitOutput);

        let digest = sha256_hex(&normalized);
        assert_eq!(verify(&round, Some(locator.as_str()), &digest).expect("绑定一致").text, normalized);
        assert_eq!(reject_code(verify(&round, None, &digest)), "sc_failure_source_unavailable");
        assert_eq!(reject_code(verify(&round, Some(locator.as_str()), &sha256_hex(RAW))),
            "sc_failure_evidence_invalid");
        assert_eq!(reject_code(verify(&round, Some("sc-split-output/v1/other/9"), &digest)),
            "sc_failure_evidence_invalid");

        let mut raw_tampered = raw.clone();
        if let ScEvidenceRef::Available { bytes, .. } = &mut raw_tampered { *bytes += 1; }
        assert_eq!(reject_code(resolve(&input(&identity, &class, &raw_tampered, &source))),
            "sc_failure_evidence_invalid");
        let mut source_tampered = source.clone();
        if let ScEvidenceRef::Available { sha256, .. } = &mut source_tampered {
            *sha256 = sha256_hex(RAW);
        }
        assert_eq!(reject_code(resolve(&input(&identity, &class, &raw, &source_tampered))),
            "sc_failure_evidence_invalid");
        let empty = available(locator.clone(), "");
        assert_eq!(reject_code(resolve(&input(&identity, &class, &empty, &empty))),
            "sc_failure_evidence_invalid", "空 source 不得当成功（no 0-default source）");

        let missing = ScEvidenceRef::Missing { code: "sc_failure_source_unavailable".to_string(),
            detail: "初次可教学失败尚无 durable source".to_string(), round_id: identity.round_id.clone() };
        let err = resolve(&input(&identity, &class, &raw, &missing)).expect_err("显式缺失");
        assert_eq!(err.code, "sc_failure_source_unavailable");
        assert_eq!(err.step, "resolve_sc_teaching_source");
        assert!(!err.publication_incomplete);
    }

    #[test]
    fn structure_round_uses_own_revision_and_rejects_tamper_scope_foreign() {
        let (_tmp, store) = setup();
        let session = live_sc_session(&store);
        let identity = round_identity(&session, &begin(&store, &session));
        let scope = scope(&session);
        let resolve = |i: &ScTeachingSourceInput<'_>| resolve_sc_teaching_source(&store, &scope, i);
        let sources = WorkItemPlanSourceStore::new(store.app_paths());
        let last = revision("rev_last_valid_0001", "# Work Item Plan\nlast valid ✓\n");
        let failed = revision("rev_failed_struct_0002", "# Work Item Plan\nfailed struct source ❯\n");
        sources.put_source_revision(PROJECT_ID, ISSUE_ID, PLAN_ID, &last).expect("put last valid");
        let failed_ref = sources.put_source_revision(PROJECT_ID, ISSUE_ID, PLAN_ID, &failed).unwrap();
        let class = ScFailureClass::StructureGate;
        let raw = ScEvidenceRef::Missing { code: "sc_failure_raw_missing".to_string(),
            detail: "结构门轮次无 split raw".to_string(), round_id: identity.round_id.clone() };
        let source_ref = available(failed_ref.clone(), &failed.source);
        let round = input(&identity, &class, &raw, &source_ref);

        let resolved = resolve(&round).expect("resolver");
        assert_eq!(resolved.text, failed.source);
        assert_ne!(resolved.text, last.source, "不得回退到 last-good 修订");
        assert_eq!(resolved.failure_source_ref, failed_ref);
        assert_eq!(resolved.origin, ScTeachingSourceOrigin::StructureRevision);
        assert_eq!(resolved.sha256, sha256_hex(&failed.source));
        assert_eq!(resolved.bytes, failed.source.len() as u64);

        let mut tampered = source_ref.clone();
        if let ScEvidenceRef::Available { sha256, .. } = &mut tampered {
            *sha256 = sha256_hex("另一份正文");
        }
        let err = resolve(&input(&identity, &class, &raw, &tampered)).expect_err("摘要篡改");
        assert_eq!(err.code, "sc_failure_evidence_invalid");
        assert_eq!(err.round_id.as_deref(), Some(identity.round_id.as_str()));

        let mut wrong_scope = scope.clone();
        wrong_scope.plan_id = "entity_9999".to_string();
        assert_eq!(reject_code(resolve_sc_teaching_source(&store, &wrong_scope, &round)),
            "sc_failure_evidence_invalid");
        let mut foreign = source_ref.clone();
        if let ScEvidenceRef::Available { locator, .. } = &mut foreign {
            *locator = "providers/some-raw-file.txt".to_string();
        }
        assert_eq!(reject_code(resolve(&input(&identity, &class, &raw, &foreign))),
            "sc_failure_evidence_invalid");
    }
}
