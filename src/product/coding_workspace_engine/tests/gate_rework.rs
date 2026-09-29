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
        .list_rework_instruction_claims(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )
        .expect("rework instruction claims");
    assert_eq!(claims.len(), 1, "一次 role run 恰一条认领：{claims:?}");
    let claim = claims[0].clone();
    assert_eq!(claim.attempt_id, attempt.id);
    let instructions = store
        .list_rework_instructions(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("rework instructions");
    assert_eq!(instructions.len(), 1);
    let instruction = &instructions[0];
    assert!(instruction.consumed_at.is_some(), "指令被该次 role run 消费");
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
        .list_rework_instruction_claims(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )
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
