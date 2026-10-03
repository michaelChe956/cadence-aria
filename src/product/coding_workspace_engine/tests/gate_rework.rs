use super::*;
use crate::product::coding_models::FindingSeverity;

#[tokio::test]
async fn manual_continue_persists_quality_bypass_audit_and_injects_reviewer_context() {
    let paths = ProductAppPaths::new(tempdir().expect("tempdir").path().join(".aria"));
    let store = CodingAttemptStore::new(paths.clone());
    let attempt = store
        .create_attempt(
            crate::product::coding_attempt_store::CreateCodingAttemptInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                work_item_id: "work_item_0001".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
                worktree_path: None,
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::Codex,
                    reviewer: Some(ProviderName::ClaudeCode),
                    review_rounds: 1,
                    permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(
                    ),
                },
                target_snapshot: None,
                max_auto_rework: 2,
            },
        )
        .expect("create attempt");
    let attempt = store
        .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("running");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review");
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("blocked");
    let gate = store
        .create_blocked_gate(
            &attempt,
            CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::CodeReview,
                node_id: Some("coding_node_0001".to_string()),
                role: Some(CodingProviderRole::CodeReviewer),
                title: "Code Review blocked".to_string(),
                description: "manual review required".to_string(),
                reason_code: Some("review_manual_continue".to_string()),
                evidence_refs: Vec::new(),
                raw_provider_output_ref: None,
                available_actions: vec![
                    coding_gate_action_for_id("manual_continue").expect("manual continue action"),
                ],
            },
        )
        .expect("blocked gate");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    assert!(
        engine
            .handle_blocked_gate_response(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &gate.gate_id,
                "manual_continue",
                None,
            )
            .await
            .is_err()
    );

    let updated = engine
        .handle_blocked_gate_response(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate.gate_id,
            "manual_continue",
            Some("operator accepts residual risk".to_string()),
        )
        .await
        .expect("manual continue");
    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::CodeReview);

    let audits = store
        .list_quality_bypass_audits(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("audits");
    assert_eq!(audits.len(), 1);
    assert_eq!(audits[0].gate_id, gate.gate_id);
    assert_eq!(audits[0].operator_context, "operator accepts residual risk");

    let updated = store
        .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("attempt");
    let pack = build_evaluation_context_pack(paths, &updated, EvaluationContextRole::CodeReviewer)
        .expect("evaluation context");
    assert_eq!(pack.quality_bypass_audits.len(), 1);
}

#[tokio::test]
async fn send_to_coder_after_review_limit_uses_latest_code_review_without_quality_bypass() {
    let paths = ProductAppPaths::new(tempdir().expect("tempdir").path().join(".aria"));
    let store = CodingAttemptStore::new(paths);
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("create attempt");
    let mut attempt = store
        .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("running");
    attempt = store
        .increment_attempt_rework_count(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("first rework");
    attempt = store
        .increment_attempt_rework_count(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("second rework");
    attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Blocked,
        )
        .expect("blocked");
    store
        .save_code_review_report(
            &attempt,
            &CodeReviewReport {
                id: "code_review_report_0001".to_string(),
                attempt_id: attempt.id.clone(),
                round: 3,
                verdict: ReviewVerdict::RequestChanges,
                findings: vec![ReviewFinding {
                    severity: FindingSeverity::Error,
                    file_path: Some("src/lib.rs".to_string()),
                    line: Some(42),
                    message: "missing validation".to_string(),
                    required_action: Some("add validation".to_string()),
                    source_stage: CodingExecutionStage::CodeReview,
                    evidence: vec!["code_review_0001/findings[0]".to_string()],
                    plan_defect_evidence: Vec::new(),
                    related_requirements: Vec::new(),
                    related_design_constraints: Vec::new(),
                    related_work_item_tasks: Vec::new(),
                    defect_class: crate::product::models::PlanDefectClass::ImplementationDefect,
                    reason_code: None,
                    contract_refs: Vec::new(),
                    capability_refs: Vec::new(),
                    repair_target: None,
                    recommended_route: crate::product::models::PlanDefectRoute::CoderRework,
                    confidence: None,
                }],
                tested_evidence_refs: Vec::new(),
                diff_refs: vec!["diffs/code_review_0001.patch".to_string()],
                summary: "reviewer requested validation fix".to_string(),
                created_at: "2026-06-14T00:00:00Z".to_string(),
                raw_provider_output_ref: Some(
                    "provider-raw/code_review/code_review_0001.txt".to_string(),
                ),
                role_run_id: None,
                run_no: Some(1),
                unit_run_id: None,
            },
        )
        .expect("code review report");
    let gate = store
        .create_blocked_gate(
            &attempt,
            CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::CodeReview,
                node_id: None,
                role: Some(CodingProviderRole::CodeReviewer),
                title: "Code Review 修复超上限".to_string(),
                description: "已达到自动修复上限".to_string(),
                reason_code: Some("reviewer_rework_limit_reached".to_string()),
                evidence_refs: vec!["code_review_0001/findings[0]".to_string()],
                raw_provider_output_ref: Some(
                    "provider-raw/code_review/code_review_0001.txt".to_string(),
                ),
                available_actions: vec![
                    coding_gate_action_for_id("provide_context").expect("provide context action"),
                    coding_gate_action_for_id("send_to_coder").expect("send to coder action"),
                    coding_gate_action_for_id("abort").expect("abort action"),
                ],
            },
        )
        .expect("blocked gate");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let updated = engine
        .handle_blocked_gate_response(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate.gate_id,
            "send_to_coder",
            Some("继续修 CodeReview findings".to_string()),
        )
        .await
        .expect("send to coder");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::Coding);
    assert_eq!(updated.rework_count, 3);
    assert!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates")
            .is_empty()
    );
    let instructions = store
        .list_rework_instructions(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("rework instructions");
    assert_eq!(instructions.len(), 1);
    assert_eq!(instructions[0].summary, "reviewer requested validation fix");
    assert_eq!(
        instructions[0].fix_hints,
        vec!["src/lib.rs:42 missing validation -> add validation"]
    );
    let notes = store
        .list_context_notes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("context notes");
    assert_eq!(notes.len(), 1);
    assert_eq!(notes[0].content, "继续修 CodeReview findings");
    assert!(
        store
            .list_quality_bypass_audits(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("quality bypass audits")
            .is_empty()
    );
}

#[tokio::test]
async fn send_to_coder_after_review_limit_accepts_actionable_blocked_code_review() {
    let paths = ProductAppPaths::new(tempdir().expect("tempdir").path().join(".aria"));
    let store = CodingAttemptStore::new(paths);
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
                permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 2,
        })
        .expect("create attempt");
    let mut attempt = store
        .seed_running_attempt_for_test(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("running");
    attempt = store
        .increment_attempt_rework_count(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("first rework");
    attempt = store
        .increment_attempt_rework_count(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("second rework");
    attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::WaitingForHuman,
        )
        .expect("waiting");
    store
        .save_code_review_report(
            &attempt,
            &CodeReviewReport {
                id: "code_review_report_0001".to_string(),
                attempt_id: attempt.id.clone(),
                round: 3,
                verdict: ReviewVerdict::Blocked,
                findings: vec![ReviewFinding {
                    severity: FindingSeverity::Error,
                    file_path: Some("src/lib.rs".to_string()),
                    line: Some(42),
                    message: "missing validation".to_string(),
                    required_action: Some("add validation".to_string()),
                    source_stage: CodingExecutionStage::CodeReview,
                    evidence: vec!["code_review_0001/findings[0]".to_string()],
                    plan_defect_evidence: Vec::new(),
                    related_requirements: Vec::new(),
                    related_design_constraints: Vec::new(),
                    related_work_item_tasks: Vec::new(),
                    defect_class: crate::product::models::PlanDefectClass::ImplementationDefect,
                    reason_code: None,
                    contract_refs: Vec::new(),
                    capability_refs: Vec::new(),
                    repair_target: None,
                    recommended_route: crate::product::models::PlanDefectRoute::CoderRework,
                    confidence: None,
                }],
                tested_evidence_refs: Vec::new(),
                diff_refs: vec!["diffs/code_review_0001.patch".to_string()],
                summary: "reviewer blocked on actionable validation fix".to_string(),
                created_at: "2026-06-14T00:00:00Z".to_string(),
                raw_provider_output_ref: Some(
                    "provider-raw/code_review/code_review_0001.txt".to_string(),
                ),
                role_run_id: None,
                run_no: Some(1),
                unit_run_id: None,
            },
        )
        .expect("code review report");
    let gate = store
        .create_blocked_gate(
            &attempt,
            CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::CodeReview,
                node_id: None,
                role: Some(CodingProviderRole::CodeReviewer),
                title: "Code Review 修复超上限".to_string(),
                description: "已达到自动修复上限".to_string(),
                reason_code: Some("reviewer_rework_limit_reached".to_string()),
                evidence_refs: vec!["code_review_0001/findings[0]".to_string()],
                raw_provider_output_ref: Some(
                    "provider-raw/code_review/code_review_0001.txt".to_string(),
                ),
                available_actions: vec![
                    coding_gate_action_for_id("provide_context").expect("provide context action"),
                    coding_gate_action_for_id("send_to_coder").expect("send to coder action"),
                    coding_gate_action_for_id("abort").expect("abort action"),
                ],
            },
        )
        .expect("blocked gate");
    let (tx, _rx) = mpsc::channel(8);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);

    let updated = engine
        .handle_blocked_gate_response(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &gate.gate_id,
            "send_to_coder",
            Some("人工意见：按 blocked finding 继续修复".to_string()),
        )
        .await
        .expect("send to coder");

    assert_eq!(updated.status, CodingAttemptStatus::Running);
    assert_eq!(updated.stage, CodingExecutionStage::Coding);
    assert_eq!(updated.rework_count, 3);
    let instructions = store
        .list_rework_instructions(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("rework instructions");
    assert_eq!(instructions.len(), 1);
    assert_eq!(
        instructions[0].summary,
        "reviewer blocked on actionable validation fix"
    );
    assert_eq!(
        instructions[0].fix_hints,
        vec!["src/lib.rs:42 missing validation -> add validation"]
    );
}

/// C2 Task 7（#18／BYPASS-18，REQ-GCE-C2-INSTR）：返修指令单次消费事务。
/// rework 落地新指令后启动返修 Coder：provider 收到的 prompt 含指令全文，
/// 认领 journal 绑定渲染 digest 与上下文 hash；中断后重放同一认领命中
/// 同一结果（Replayed），指令不被消费第二次；异渲染 fail-closed。
#[tokio::test]
async fn rework_instruction_claim_binds_render_before_consumption() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let attempt = store
        .replace_attempt_provider_conversations(
            &attempt,
            vec![ProviderConversationRef {
                role: ProviderConversationRole::Coder,
                provider: ProviderName::Codex,
                provider_session_id: "coder-session-before-rework".to_string(),
                updated_at: "2026-06-01T00:00:00Z".to_string(),
                last_node_id: Some("coding_node_0001".to_string()),
            }],
        )
        .expect("record coder conversation");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = super::provider_driven::ReviewerDrivenReworkProvider::default();
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let updated = engine
        .execute_coder_fix_from_review(
            &attempt,
            &super::provider_driven::review_report_requesting_changes(&attempt),
            &CodingExecutionContext::default(),
            &provider,
            &mut command_rx,
        )
        .await
        .expect("coder fix from review");

    // 实际发送给 provider 的 prompt 包含指令全文（摘要与修复提示，非摘要）。
    let input = provider.recorded_input();
    assert!(input.prompt.contains("本轮修复要求"));
    assert!(input.prompt.contains("missing validation"));
    assert!(input.prompt.contains("add validation"));

    // 认领 journal：绑定实际 prompt 的渲染 digest 与执行上下文 hash。
    let claims = store
        .list_rework_instruction_claims(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("rework instruction claims");
    assert_eq!(claims.len(), 1, "一次 role run 恰一条认领：{claims:?}");
    let claim = claims[0].clone();
    assert_eq!(claim.attempt_id, attempt.id);
    let instructions = store
        .list_rework_instructions(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("rework instructions");
    assert_eq!(instructions.len(), 1);
    let instruction = &instructions[0];
    assert!(
        instruction.consumed_at.is_some(),
        "指令被该次 role run 消费"
    );
    assert!(instruction.consumed_by_node_id.is_some());
    assert_eq!(claim.instruction_ids, vec![instruction.id.clone()]);
    let expected_digest = {
        use sha2::Digest as _;
        hex::encode(sha2::Sha256::digest(input.prompt.as_bytes()))
    };
    assert_eq!(claim.rendered_prompt_digest, expected_digest);
    // 单 Work Item attempt 无 unit-run 渲染上下文：执行上下文 hash 即实际 prompt。
    assert_eq!(claim.context_hash, expected_digest);
    assert!(claim.consumed_at.is_some());

    // 中断后重放：同一认领同一渲染结果 → Replayed，同一 claim 不变、不二次消费。
    let node_id = instruction
        .consumed_by_node_id
        .clone()
        .expect("consumed node");
    let replay = store
        .claim_and_consume_rework_instructions(
            &updated,
            &node_id,
            1,
            &input.prompt,
            None,
            &[instruction.id.clone()],
        )
        .expect("replay the same claim");
    assert_eq!(
        replay,
        crate::product::coding_attempt_store::ReworkClaimOutcome::Replayed {
            claim: claim.clone()
        }
    );
    let claims_after = store
        .list_rework_instruction_claims(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("claims after replay");
    assert_eq!(claims_after, vec![claim.clone()], "重放命中同一认领");
    let instructions_after = store
        .list_rework_instructions(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("instructions after replay");
    assert_eq!(instructions_after.len(), 1, "指令不被消费第二次");

    // 已记录执行上下文 hash 不被覆盖为新含义：异渲染重放 fail-closed。
    let tampered_prompt = format!("{}\n额外内容", input.prompt);
    let conflict = store
        .claim_and_consume_rework_instructions(
            &updated,
            &node_id,
            1,
            &tampered_prompt,
            None,
            &[instruction.id.clone()],
        )
        .expect_err("different render must fail closed");
    assert!(
        matches!(
            conflict,
            crate::product::json_store::ProductStoreError::IdentityMismatch { ref kind, .. }
                if *kind == "coding_rework_instruction_claim"
        ),
        "unexpected conflict: {conflict:?}"
    );
}

/// C-1b（oracle 裁决）：消费标记后、spawn 前中断的认领在恢复重驱时被
/// 对账强制回放——指令重新进入实际 prompt（不重写 claim、不二次消费），
/// instruction_claim_interrupted 等待事实落账并在重放完成后清除。
#[tokio::test]
async fn interrupted_rework_claim_is_force_replayed_on_recovery_redrive() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    // Window B 现场：上一轮返修已认领并标记消费（node coding_node_0001），
    // 但 spawn 前中断——该 attempt 无任何 role run / ProviderPrompt。
    let instruction = crate::product::coding_models::CodingReworkInstruction {
        id: "coding_rework_instruction_0001".to_string(),
        attempt_id: attempt.id.clone(),
        source_stage: CodingExecutionStage::CodeReview,
        rework_round: 1,
        summary: "补齐缺失校验".to_string(),
        fix_hints: vec!["src/lib.rs:42 missing validation -> add validation".to_string()],
        questions: Vec::new(),
        created_at: chrono::Utc::now().to_rfc3339(),
        consumed_by_node_id: None,
        consumed_at: None,
    };
    store
        .save_rework_instruction(&attempt, &instruction)
        .expect("seed interrupted instruction");
    let claimed = store
        .claim_and_consume_rework_instructions(
            &attempt,
            "coding_node_0001",
            1,
            "上一轮完整 prompt（渲染后中断，未发出）",
            None,
            &["coding_rework_instruction_0001".to_string()],
        )
        .expect("claim as the interrupted run");
    let crate::product::coding_attempt_store::ReworkClaimOutcome::Claimed { claim } = claimed
    else {
        panic!("first claim must be Claimed");
    };

    // 对账：判中断 → 落等待事实＋返回强制回放指令。
    let renders = store
        .reconcile_interrupted_rework_claims(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reconcile");
    assert_eq!(
        renders.len(),
        1,
        "exactly the interrupted claim: {renders:?}"
    );
    assert_eq!(renders[0].claim.claim_id, claim.claim_id);
    assert_eq!(renders[0].instructions.len(), 1);
    assert_eq!(renders[0].instructions[0].id, instruction.id);
    let facts = store
        .list_instruction_claim_interrupted_facts(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )
        .expect("interrupted facts");
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].claim_id, claim.claim_id);
    assert_eq!(facts[0].instruction_ids, claim.instruction_ids);

    // 恢复重驱（execute_rework 路径）：渲染新指令的同时，把中断认领的
    // 指令强制回放进实际发送 prompt。
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::CodeReview,
        )
        .expect("code review stage");
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = super::provider_driven::ReviewerDrivenReworkProvider::default();
    let (_command_tx, mut command_rx) = mpsc::channel(1);
    engine
        .execute_coder_fix_from_review(
            &attempt,
            &super::provider_driven::review_report_requesting_changes(&attempt),
            &CodingExecutionContext::default(),
            &provider,
            &mut command_rx,
        )
        .await
        .expect("re-drive after interruption");

    let input = provider.recorded_input();
    assert!(
        input.prompt.contains("中断认领强制回放"),
        "forced replay section must enter the actual prompt: {}",
        input.prompt
    );
    assert!(input.prompt.contains("missing validation"));

    // 不重写 claim、不二次消费：中断认领原样保留；新认领只绑定新指令。
    let claims_after = store
        .list_rework_instruction_claims(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("claims after re-drive");
    assert_eq!(claims_after.len(), 2, "{claims_after:?}");
    assert!(
        claims_after
            .iter()
            .any(|record| record.claim_id == claim.claim_id
                && record.rendered_prompt_digest == claim.rendered_prompt_digest
                && record.context_hash == claim.context_hash),
        "interrupted claim must not be rewritten"
    );
    let new_claim = claims_after
        .iter()
        .find(|record| record.claim_id != claim.claim_id)
        .expect("re-drive claim");
    assert_eq!(
        new_claim.instruction_ids,
        vec!["coding_rework_instruction_0002".to_string()],
        "forced replay ids must not enter the new claim set: {new_claim:?}"
    );

    // 重放完成后（本次重驱 run 已发 ProviderPrompt）：对账清除等待事实。
    let renders_after = store
        .reconcile_interrupted_rework_claims(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reconcile after replay");
    assert!(renders_after.is_empty(), "{renders_after:?}");
    let facts_after = store
        .list_instruction_claim_interrupted_facts(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )
        .expect("facts after replay");
    assert!(facts_after.is_empty(), "waiting fact cleared after replay");
}

/// C-1b 对账边界：认领自身 run 已发 ProviderPrompt（正常流）→ 不判中断、
/// 不落事实；纯 context note 认领无 node 可归因，不参与对账。
#[tokio::test]
async fn reconcile_skips_prompted_claims_and_note_only_claims() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let instruction = crate::product::coding_models::CodingReworkInstruction {
        id: "coding_rework_instruction_0001".to_string(),
        attempt_id: attempt.id.clone(),
        source_stage: CodingExecutionStage::CodeReview,
        rework_round: 1,
        summary: "正常流指令".to_string(),
        fix_hints: vec!["按计划修复".to_string()],
        questions: Vec::new(),
        created_at: chrono::Utc::now().to_rfc3339(),
        consumed_by_node_id: None,
        consumed_at: None,
    };
    store
        .save_rework_instruction(&attempt, &instruction)
        .expect("seed instruction");
    store
        .claim_and_consume_rework_instructions(
            &attempt,
            "coding_node_0001",
            1,
            "正常流完整 prompt",
            None,
            &["coding_rework_instruction_0001".to_string()],
        )
        .expect("claim");
    // 认领 node 的 role run 已发出 prompt：正常流，非中断。
    let run = store
        .create_role_run(
            &attempt,
            CodingExecutionStage::Coding,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::Initial,
            Some("coding_node_0001".to_string()),
        )
        .expect("role run");
    store
        .append_role_run_event(
            &attempt,
            &run,
            crate::product::coding_models::CodingRoleRunEventType::ProviderPrompt,
            serde_json::json!({ "prompt": "正常流完整 prompt" }),
        )
        .expect("provider prompt event");
    let renders = store
        .reconcile_interrupted_rework_claims(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reconcile");
    assert!(renders.is_empty(), "prompted claim is not interrupted");
    assert!(
        store
            .list_instruction_claim_interrupted_facts(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id
            )
            .expect("facts")
            .is_empty()
    );

    // 纯 context note 认领：无 node 归因，不参与对账（备注为辅助上下文）。
    let note = store
        .create_context_note(&attempt, "补充上下文备注".to_string())
        .expect("context note");
    store
        .claim_and_consume_rework_instructions(
            &attempt,
            "coding_node_0002",
            2,
            &format!("含备注的完整 prompt：{}", note.content),
            None,
            &[note.id.clone()],
        )
        .expect("note-only claim");
    let renders = store
        .reconcile_interrupted_rework_claims(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("reconcile with note claim");
    assert!(
        renders.is_empty(),
        "note-only claims have no node attribution: {renders:?}"
    );
}
