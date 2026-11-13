//! Strict SC failure evidence store（Plan §2.3 前 6 个冻结接口）。
//!
//! RED 阶段：本文件只落消费者行为测试（`mod tests`）。冻结类型
//! （`ScFailureEvidenceEvent` / `ScFailureEvidenceRead` / `ScFailureEvidenceError` /
//! `sc_failure_chain_id`）与 `LifecycleStore` 的
//! `append_sc_failure_evidence` / `read_sc_failure_evidence` /
//! `put_sc_teaching_prompt` / `read_sc_teaching_prompt` / `read_sc_split_output`
//! 尚未实现，故
//! `cargo test --locked --lib lifecycle_store::sc_failure_evidence`
//! 预期在编译期 RED（E0432 未解析导入 / E0599 方法不存在）——证明该能力缺失，
//! 而不是用行为假红冒充。取得实现授权后本文件补齐真实实现并转绿。

// P05 RED 测试（只含测试模块，不含实现）。落位：新建的
// src/product/lifecycle_store/sc_failure_evidence.rs 末尾内联 `#[cfg(test)] mod tests`。
// 6 个消费者行为：重复幂等 / 异文冲突 / 坏行带前缀 / raw 正确绑定 / raw 错误 / prompt 损坏 /
// 墓碑无复活。真实接口尚不存在 → 编译 RED 即证明缺功能；本文件不含 panic/Err 占位。

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::{ScFailureEvidenceEvent, ScFailureEvidenceRead, sc_failure_chain_id};
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::lifecycle_store::{
        CreateWorkspaceSessionInput, LifecycleStore, WorkItemPlanSessionOptions,
    };
    use crate::product::models::{ProviderName, WorkspaceType};
    use crate::product::work_item_plan_policy::{RunPolicy, WorkItemPlanFlowKind};
    use crate::product::work_item_split_engine::parse::WorkItemSplitProviderRunHandle;
    use crate::web::workspace_ws_types::{
        ScEvidenceRef, ScFailureChainIdentity, ScFailureClass, ScFailureScope, ScPromptDisposition,
        ScRoundIdentity, ScTeachingPromptEvidence,
    };

    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_0001";
    const BUSINESS_KEY: &str = "business_key_sc_0001";
    const PLAN_ID: &str = "entity_0001";
    const NODE_ID: &str = "node_original_0001";
    const RAW1: &str = "provider raw 1：{\"plan\":\"候选一\"}\n第二行 tab\t quotes \"x\"";
    const RAW2: &str = "结构差异 raw 2：ir_preflight 拒绝\r\n末尾空格与 CJK 複雑 字符";
    const SOURCE1: &str = "normalized author delivery source 1（去 preamble 后）";
    const PROMPT_TEXT: &str = "教学提示：先修 A 再修 B\n尾部 ❯";

    fn setup() -> (TempDir, LifecycleStore) {
        let tmp = TempDir::new().unwrap();
        let store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
        (tmp, store)
    }

    /// 真实 SC session（single-candidate 流，实体归属 Plan），与既有 SC fixture 同构。
    fn live_sc_session(store: &LifecycleStore) -> String {
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
    fn begin(store: &LifecycleStore, session_id: &str) -> WorkItemSplitProviderRunHandle {
        store
            .begin_work_item_split_provider_run(
                PROJECT_ID,
                ISSUE_ID,
                &ProviderName::Codex,
                session_id,
            )
            .expect("begin")
    }

    fn complete_raw(store: &LifecycleStore, handle: &WorkItemSplitProviderRunHandle, raw: &str) {
        store
            .complete_work_item_split_provider_run(
                handle,
                "split prompt",
                &serde_json::Value::String(raw.into()),
            )
            .expect("complete")
    }

    /// chain/round 全部由真实 handle 构造（locator 不解析不透明 run_ref 猜路径）。
    fn bound_round(
        store: &LifecycleStore,
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

    fn raw_locator(round: &ScRoundIdentity) -> String {
        format!(
            "sc-split-output/v1/{}/{}",
            round.workspace_session_id, round.role_run_seq
        )
    }

    fn available(locator: String, body: &str) -> ScEvidenceRef {
        ScEvidenceRef::Available {
            locator,
            sha256: sha256_hex(body),
            bytes: body.len() as u64,
        }
    }

    /// §2.2：raw 与 source 共用同一 split locator，摘要/bytes 计量不同正文。
    fn observed(
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

    fn chain_dir(store: &LifecycleStore, chain: &ScFailureChainIdentity) -> PathBuf {
        store
            .workspace_timeline_root_for_session(&chain.scope.session_id)
            .expect("timeline root")
            .join("sc-failure-evidence")
            .join(&chain.chain_id)
    }

    fn log_bytes(store: &LifecycleStore, chain: &ScFailureChainIdentity) -> Vec<u8> {
        std::fs::read(chain_dir(store, chain).join("rounds.jsonl")).expect("read log")
    }

    /// 只改冻结字段 entry_seq（断号造例）。
    fn with_entry_seq(line: &str, entry_seq: u64) -> String {
        let mut value: serde_json::Value = serde_json::from_str(line).expect("entry");
        value["entry_seq"] = serde_json::json!(entry_seq);
        serde_json::to_string(&value).expect("serialize")
    }

    fn parts(reference: &ScEvidenceRef) -> (String, String, u64) {
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
    fn expect_prefix(err: &super::ScFailureEvidenceError, code: &str) -> &ScFailureEvidenceRead {
        assert_eq!(err.code, code, "detail={}", err.detail);
        assert!(!err.step.is_empty());
        err.verified_prefix.as_deref().expect("verified prefix")
    }

    fn sha256_hex(body: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(body.as_bytes());
        hasher
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    #[test]
    fn sc_failure_store_replay_conflict_and_corrupt_prefix() {
        let (_tmp, store) = setup();
        let session_id = live_sc_session(&store);
        let handle = begin(&store, &session_id);
        let (chain, round) = bound_round(&store, &session_id, &handle);
        let bound_event = ScFailureEvidenceEvent::RoundBound {
            round: round.clone(),
        };

        // ① + ② 首写 entry_seq=1；同槽同文重放零膨胀（重建 store 仍可读）。
        let bound = store
            .append_sc_failure_evidence(&chain, &bound_event)
            .expect("append");
        assert_eq!(bound.entry_seq, 1);
        let after_first = log_bytes(&store, &chain);
        assert_eq!(
            store
                .append_sc_failure_evidence(&chain, &bound_event)
                .expect("replay"),
            bound
        );
        assert_eq!(log_bytes(&store, &chain), after_first, "同槽同文不得膨胀");
        let reopened = LifecycleStore::new(store.app_paths());
        assert_eq!(
            reopened
                .read_sc_failure_evidence(&chain)
                .expect("read")
                .entries
                .len(),
            1
        );

        // ③ 同槽异文：Err 携带已验证前缀且不写盘。
        let observed_event = observed(&round, RAW1, SOURCE1);
        store
            .append_sc_failure_evidence(&chain, &observed_event)
            .expect("observed");
        let before = log_bytes(&store, &chain);
        let mut conflicting = observed_event.clone();
        if let ScFailureEvidenceEvent::FailureObserved {
            repair_instructions,
            ..
        } = &mut conflicting
        {
            repair_instructions.push("追加不同正文".to_string());
        }
        let err = store
            .append_sc_failure_evidence(&chain, &conflicting)
            .expect_err("异文必须拒绝");
        assert_eq!(
            expect_prefix(&err, "sc_failure_entry_conflict")
                .entries
                .len(),
            2
        );
        assert!(!err.publication_incomplete);
        assert_eq!(log_bytes(&store, &chain), before);

        // ④ 坏行：坏行处停止，其后的合法行不得被跳过吞下。
        let good = log_bytes(&store, &chain);
        let lines: Vec<String> = String::from_utf8(good.clone())
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        let corrupt = format!(
            "{}\n{{\"schema_version\":1,\"entry_seq\":\n{}\n",
            lines[0], lines[1]
        );
        std::fs::write(chain_dir(&store, &chain).join("rounds.jsonl"), corrupt).unwrap();
        let err = store
            .read_sc_failure_evidence(&chain)
            .expect_err("坏行必须拒绝");
        let prefix = expect_prefix(&err, "sc_failure_log_corrupt");
        assert_eq!(prefix.entries, vec![bound.clone()]);
        assert!(err.publication_incomplete);

        // ⑤ 断 entry_seq：断点停止，前缀到断点为止。
        std::fs::write(chain_dir(&store, &chain).join("rounds.jsonl"), &good).unwrap();
        let gap = format!("{}\n{}\n", lines[0], with_entry_seq(&lines[1], 3));
        std::fs::write(chain_dir(&store, &chain).join("rounds.jsonl"), gap).unwrap();
        let err = store
            .read_sc_failure_evidence(&chain)
            .expect_err("断号必须拒绝");
        assert_eq!(
            expect_prefix(&err, "sc_failure_log_sequence_gap").entries,
            vec![bound]
        );
    }

    #[test]
    fn sc_failure_raw_binding_prompt_immutability_and_tombstone() {
        let (_tmp, store) = setup();
        let session_id = live_sc_session(&store);
        let first_handle = begin(&store, &session_id);
        let second_handle = begin(&store, &session_id);
        complete_raw(&store, &first_handle, RAW1);
        complete_raw(&store, &second_handle, RAW2);
        let scope = ScFailureScope {
            session_id: session_id.clone(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: PLAN_ID.to_string(),
        };
        let (chain, first_round) = bound_round(&store, &session_id, &first_handle);
        let (_, second_round) = bound_round(&store, &session_id, &second_handle);

        // ① raw 正确绑定：两个真实 handle 各自逐字回读（重建 store 后仍相同）。
        assert_eq!(
            store
                .read_sc_split_output(&scope, &first_round)
                .expect("raw1"),
            RAW1
        );
        assert_eq!(
            store
                .read_sc_split_output(&scope, &second_round)
                .expect("raw2"),
            RAW2
        );
        let reopened = LifecycleStore::new(store.app_paths());
        assert_eq!(
            reopened.read_sc_split_output(&scope, &first_round).unwrap(),
            RAW1
        );
        // 同文另一 round 的 locator 不得冒用：正文逐字相同、绑定 seq 不同。
        let same_handle = begin(&store, &session_id);
        complete_raw(&store, &same_handle, RAW1);
        let (_, same_round) = bound_round(&store, &session_id, &same_handle);
        assert_eq!(
            store.read_sc_split_output(&scope, &same_round).unwrap(),
            RAW1
        );

        // ② raw 错误：缺 handle / 非 JSON String 都是稳定 Missing，不返回空串。
        let ghost = WorkItemSplitProviderRunHandle {
            run_ref: format!("ws-{session_id}-never-begun"),
            workspace_session_id: session_id.clone(),
            role_run_seq: 999_999,
        };
        let (_, ghost_round) = bound_round(&store, &session_id, &ghost);
        let missing = store
            .read_sc_split_output(&scope, &ghost_round)
            .expect_err("缺 handle 必须 Missing");
        assert_eq!(missing.code, "sc_failure_raw_missing");
        assert_eq!(missing.step, "read_sc_split_output");
        assert_eq!(
            missing.round_id.as_deref(),
            Some(ghost_round.round_id.as_str())
        );
        store
            .complete_work_item_split_provider_run(
                &same_handle,
                "split prompt",
                &serde_json::json!({"not": "a string"}),
            )
            .unwrap();
        let non_string = store
            .read_sc_split_output(&scope, &same_round)
            .expect_err("非 String 必须 Missing");
        assert_eq!(non_string.code, "sc_failure_raw_missing");
        assert!(non_string.publication_incomplete);

        // ③ raw locator 绑定：正确绑定可写；冒用另一 round 的 locator 被拒。
        store
            .append_sc_failure_evidence(
                &chain,
                &ScFailureEvidenceEvent::RoundBound {
                    round: first_round.clone(),
                },
            )
            .unwrap();
        let mut wrong = observed(&first_round, RAW1, SOURCE1);
        if let ScFailureEvidenceEvent::FailureObserved { raw, .. } = &mut wrong {
            *raw = available(raw_locator(&same_round), RAW1);
        }
        let err = store
            .append_sc_failure_evidence(&chain, &wrong)
            .expect_err("locator 绑定不符必须拒绝");
        assert_eq!(err.code, "sc_failure_locator_invalid");
        assert_eq!(
            store
                .append_sc_failure_evidence(&chain, &observed(&first_round, RAW1, SOURCE1))
                .expect("正确绑定可写")
                .entry_seq,
            2
        );

        // ④ prompt 同文重放不重写、同槽异文拒、篡改后按摘要不符拒。
        let prepared: ScTeachingPromptEvidence = store
            .put_sc_teaching_prompt(
                &chain,
                &first_round.round_id,
                2,
                "builder_compile_0001",
                PROMPT_TEXT,
                ScPromptDisposition::Prepared,
            )
            .expect("put prompt");
        let (locator, prompt_sha, prompt_bytes) = parts(&prepared.prompt);
        assert_eq!(prompt_sha, sha256_hex(PROMPT_TEXT));
        assert_eq!(prompt_bytes, PROMPT_TEXT.len() as u64);
        let file = chain_dir(&store, &chain)
            .join("prompts")
            .join(format!("2-{prompt_sha}.txt"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), PROMPT_TEXT);
        let after_prompt = log_bytes(&store, &chain);
        assert_eq!(
            store
                .put_sc_teaching_prompt(
                    &chain,
                    &first_round.round_id,
                    2,
                    "builder_compile_0001",
                    PROMPT_TEXT,
                    ScPromptDisposition::Prepared,
                )
                .expect("replay prompt"),
            prepared
        );
        assert_eq!(log_bytes(&store, &chain), after_prompt, "同文重放零膨胀");
        let conflict = store
            .put_sc_teaching_prompt(
                &chain,
                &first_round.round_id,
                2,
                "builder_compile_0001",
                "不同的教学行文",
                ScPromptDisposition::Prepared,
            )
            .expect_err("同槽异文必须拒绝");
        assert_eq!(conflict.code, "sc_failure_entry_conflict");
        std::fs::write(&file, format!("{PROMPT_TEXT}X")).unwrap();
        let tampered = store
            .read_sc_teaching_prompt(&chain, &locator)
            .expect_err("正文被篡改必须拒绝");
        assert_eq!(tampered.code, "sc_failure_prompt_digest_mismatch");
        assert!(tampered.publication_incomplete);
        std::fs::write(&file, PROMPT_TEXT).unwrap();
        assert_eq!(
            store.read_sc_teaching_prompt(&chain, &locator).unwrap(),
            PROMPT_TEXT
        );

        // ⑤ 墓碑（purging）：append/prompt 写稳定拒绝，日志与 prompt 目录不复活。
        let lease =
            crate::product::entity_operation_store::EntityOperationStore::new(store.app_paths())
                .try_acquire_delete(&crate::product::entity_deletion_store::EntityScope::new(
                    PROJECT_ID,
                    ISSUE_ID,
                    crate::product::entity_deletion_store::DeletionEntityKind::Plan,
                    "entity_0001".to_string(),
                ))
                .expect("delete lease");
        crate::product::entity_deletion_store::EntityDeletionStore::new(store.app_paths())
            .begin(&lease)
            .expect("purging tombstone");
        let before = log_bytes(&store, &chain);
        let ghost_round = ScRoundIdentity {
            round_id: "ghost-round-0001".to_string(),
            ..first_round.clone()
        };
        let denied = store
            .append_sc_failure_evidence(
                &chain,
                &ScFailureEvidenceEvent::RoundBound { round: ghost_round },
            )
            .expect_err("未存活实体必须拒绝");
        assert_eq!(denied.code, "sc_failure_session_not_live");
        assert!(denied.publication_incomplete);
        assert!(denied.verified_prefix.is_none());
        assert_eq!(log_bytes(&store, &chain), before, "被拒 append 不得改日志");
        let prompt_denied = store
            .put_sc_teaching_prompt(
                &chain,
                &first_round.round_id,
                3,
                "builder_compile_0001",
                "第三个目标槽正文",
                ScPromptDisposition::Prepared,
            )
            .expect_err("未存活实体必须拒绝 prompt 写");
        assert_eq!(prompt_denied.step, "put_sc_teaching_prompt");
        assert!(
            !chain_dir(&store, &chain)
                .join("prompts")
                .join(format!("3-{}.txt", sha256_hex("第三个目标槽正文")))
                .exists(),
            "被拒 prompt 不得落文件"
        );
    }

    #[test]
    fn sc_failure_evidence_rejects_non_sc_session() {
        let (_tmp, store) = setup();
        // 既有 tests.rs 同名 fixture：Story session、无 work_item_plan_options → 非 SC。
        let session = store
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: "story_spec_0001".to_string(),
                workspace_type: WorkspaceType::Story,
                author_provider: ProviderName::Codex,
                reviewer_provider: None,
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .expect("non-SC session");
        let handle = begin(&store, &session.id);
        let (chain, round) = bound_round(&store, &session.id, &handle);
        let err = store
            .append_sc_failure_evidence(
                &chain,
                &ScFailureEvidenceEvent::RoundBound {
                    round: round.clone(),
                },
            )
            .expect_err("非 SC session 必须拒绝 evidence 写");
        assert_eq!(err.code, "sc_failure_session_not_sc");
        assert_eq!(err.step, "append_sc_failure_evidence");
        assert!(err.publication_incomplete);
        let prompt_err = store
            .put_sc_teaching_prompt(
                &chain,
                &round.round_id,
                1,
                "builder_compile_0001",
                PROMPT_TEXT,
                ScPromptDisposition::Prepared,
            )
            .expect_err("非 SC session 必须拒绝 prompt 写");
        assert_eq!(prompt_err.code, "sc_failure_session_not_sc");
        assert!(!chain_dir(&store, &chain).join("rounds.jsonl").exists());
    }
}
