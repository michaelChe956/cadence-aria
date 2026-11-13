// P05t 主测试模块：replay / raw / non-SC 三组消费者行为。
// 落位：src/product/lifecycle_store/tests/sc_failure_evidence.rs（由 store 模块 #[path] 挂载）。
// 共享 fixture 由 store 模块统一挂载一次（见 sc_failure_evidence.rs 的 mod fixture）。

use super::fixture::*;

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
