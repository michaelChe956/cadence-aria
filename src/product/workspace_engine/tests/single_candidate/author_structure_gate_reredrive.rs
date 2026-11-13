// 作者阶段结构门拒收的「恰一次教学重驱」（REQ-SG-02/03、REQ-WSC-18/25）。
//
// 上一批（author_structure_gate_typed.rs）把拒收从硬失败改成 typed 失败路
// （durable + 尝试流水 + 人工门），owner 拿到了人话原因与两个修法，但仍要
// owner 自己动手。本文件钉住结构门重驱**自己的槽位**（2026-10-31 真机修订：
// 与 compile/IR 槽互不挤占，每类至多一次）下的三件事：
// 1. 槽位空闲（allow_structure_rerun=true）：不开人工门、不硬失败，返回
//    `Rejected { failure, structure_failures }`——**sealed 的原失败身份**
//    （kind/stable_code/failure_id/detail 逐字不变）＋**原 typed 结构失败集**
//    （多成员不丢、序不变；不是重算，不是从 sealed 文本反解析）；
// 2. 槽位已占用（allow_structure_rerun=false）：保持既有落门语义，会话不是
//    failed；
// 3. 非环拒收不得虚构环事实（不虚构原因）。
//
// 每次尝试都进 `candidate-attempts.jsonl`（两次拒收 = 两行失败）在 1+2 中
// 一并钉住；`auto_rewrites_used` 不因作者阶段重驱而变、durable `auto_rewrite`
// 不留 running 记录同样在此钉住（重驱走 runner 的 provider 通道，不是引擎
// prerevision 路由——作者阶段候选 refs 未提升，重写通道不可达）。
//
// 措辞纪律：本文件只钉**结构化事实**（sealed kind/code/id、typed 失败集成员、
// durable 流水行、scope/状态/计数），不钉人话宣传句——原「contains 文案」断言
// 循环已删除，改由 sealed 数据／scope／状态承担。
mod author_structure_gate_reredrive {
    use super::contract_prerevision::{author_round_record, clean_candidate};
    use super::sg3a1_structure_gate::{cycle_candidate, unknown_provider_candidate};
    use super::*;
    use crate::product::planning_failure::PlanningFailureKind;
    use crate::product::workspace_engine::SingleCandidateAuthorCompletion;
    use crate::product::workspace_engine::single_candidate_structure_failure::STRUCTURE_STABLE_CODE;
    use crate::product::workspace_engine::types::PlanningCompileFailureKind;
    use crate::web::workspace_ws_types::TimelineNodeType;

    type Attempts =
        Vec<crate::product::lifecycle_store::candidate_attempts::CandidateAttemptRecord>;

    fn attempt_outcomes(attempts: &Attempts) -> Vec<&'static str> {
        attempts
            .iter()
            .map(|record| match record.outcome {
                crate::product::lifecycle_store::candidate_attempts::CandidateAttemptOutcome::Passed => "passed",
                crate::product::lifecycle_store::candidate_attempts::CandidateAttemptOutcome::Failed => "failed",
            })
            .collect()
    }

    fn prompt_source_body(prompt: &str) -> &str {
        let mut offset = 0;
        for line in prompt.split_inclusive('\n') {
            let fence = line.strip_suffix('\n').unwrap_or(line);
            if fence.len() >= 3 && fence.bytes().all(|byte| byte == b'`') {
                let body_start = offset + line.len();
                let body_and_tail = &prompt[body_start..];
                let body_end = body_and_tail
                    .find(&format!("\n{fence}\n"))
                    .expect("previous-source fence must close before output instructions");
                return &body_and_tail[..body_end];
            }
            offset += line.len();
        }
        panic!("previous-source block missing");
    }

    fn prompt_typed_failures(
        prompt: &str,
    ) -> Vec<crate::product::planning_failure::PlanningFailure> {
        // JSON rows carry actual typed records, not human wording. Stop before source
        // so JSON-looking source data is never mistaken for a diagnostic record.
        prompt
            .lines()
            .take_while(|line| !(line.len() >= 3 && line.bytes().all(|byte| byte == b'`')))
            .filter_map(|line| serde_json::from_str(line).ok())
            .collect()
    }

    fn structure_prompt(
        failure: &crate::product::workspace_engine::types::PlanningCompileFailure,
        failures: &crate::product::planning_failure::PlanningFailureSet,
        source: &str,
        source_ref: &str,
    ) -> String {
        use sha2::Digest as _;
        let sha256 = hex::encode(sha2::Sha256::digest(source.as_bytes()));
        crate::product::workspace_engine::single_candidate_structure_failure::author_structure_repair_prompt(
            failure,
            failures,
            source,
            &crate::product::workspace_engine::ScTeachingSourceMetadata {
                source_ref,
                sha256: &sha256,
                bytes: source.len() as u64,
                origin: crate::web::workspace_ws_types::ScTeachingSourceOrigin::StructureRevision,
            },
        ).expect("matching source metadata must render")
    }

    #[test]
    fn structure_prompt_round_trips_backticks_cjk_and_large_source() {
        use crate::product::planning_failure::PlanningFailureSet;
        use crate::product::workspace_engine::types::PlanningCompileFailure;
        let failure = PlanningCompileFailure::operational(
            STRUCTURE_STABLE_CODE,
            "unknown_provider_logical_work_item".to_string(),
        );
        let mut source = " \t中文失败候选\r\n```\n内层代码\n```\n```````\n".to_string();
        source.push_str(&"保留原文".repeat(30_000));
        source.push_str("\r\n尾部空格  \t");
        let prompt = structure_prompt(
            &failure,
            &PlanningFailureSet::default(),
            &source,
            "failed-revision-backticks",
        );
        assert_eq!(prompt_source_body(&prompt).as_bytes(), source.as_bytes());
        assert!(prompt_typed_failures(&prompt).is_empty());
    }

    #[test]
    fn structure_prompt_rejects_mismatched_source_metadata() {
        use crate::product::planning_failure::PlanningFailureSet;
        use crate::product::workspace_engine::single_candidate_structure_failure::author_structure_repair_prompt;
        use crate::product::workspace_engine::{
            ScTeachingSourceMetadata, types::PlanningCompileFailure,
        };
        use sha2::Digest as _;
        let failure = PlanningCompileFailure::operational(
            STRUCTURE_STABLE_CODE,
            "unknown_provider_logical_work_item".to_string(),
        );
        let source = "失败候选\n";
        let sha256 = hex::encode(sha2::Sha256::digest(source.as_bytes()));
        let metadata = ScTeachingSourceMetadata {
            source_ref: "failed-revision",
            sha256: &sha256,
            bytes: source.len() as u64,
            origin: crate::web::workspace_ws_types::ScTeachingSourceOrigin::StructureRevision,
        };
        let failures = PlanningFailureSet::default();
        let invalid_hash = "0".repeat(64);
        for invalid in [
            ScTeachingSourceMetadata {
                bytes: metadata.bytes + 1,
                ..metadata.clone()
            },
            ScTeachingSourceMetadata {
                sha256: &invalid_hash,
                ..metadata
            },
        ] {
            assert_eq!(
                author_structure_repair_prompt(&failure, &failures, source, &invalid)
                    .expect_err("invalid source must fail closed")
                    .code,
                "sc_failure_evidence_invalid"
            );
        }
    }

    fn failed_revision_source(
        lifecycle: &crate::product::lifecycle_store::LifecycleStore,
        engine: &crate::product::workspace_engine::WorkspaceEngine,
        source_ref: &str,
    ) -> String {
        let session = engine.session();
        crate::product::work_item_plan_source_store::WorkItemPlanSourceStore::new(
            lifecycle.app_paths(),
        )
        .get_source_revision(
            &crate::product::work_item_plan_source_store::SourceStoreScope {
                project_id: session.project_id.clone(),
                issue_id: session.issue_id.clone(),
                plan_id: session.entity_id.clone(),
            },
            source_ref,
        )
        .expect("read the failure's own canonical source revision")
        .source
    }

    #[test]
    fn both_cycle_repairs_reach_the_structure_gate_without_dropping_inputs() {
        use super::structure_parity::{Repos, ir_of, item, structure_context};
        use crate::product::workspace_engine::plan_projection::assess_single_candidate_structure;
        use crate::product::workspace_engine::single_candidate_structure_failure::author_structure_gate_failure;
        use crate::product::workspace_engine::types::StructuralAssessment;
        let repos = Repos::new();
        let context = structure_context(repos.table(), repos.change_order());
        let ir = ir_of(vec![
            item("WI-001", "repository_a", None),
            item("WI-002", "repository_b", Some("WI-001")),
            item("WI-003", "repository_c", Some("WI-002")),
            item("WI-004", "repository_b", Some("WI-003")),
        ]);
        let StructuralAssessment::Rejected(failures, _) =
            assess_single_candidate_structure(String::new(), &ir, &context)
        else {
            panic!("reverse-order input must be rejected before repair");
        };
        let failure = author_structure_gate_failure(&ir, &context, &failures);
        let source = serde_json::to_string(&ir).expect("serialize rejected candidate fixture");
        let prompt = structure_prompt(&failure, &failures, &source, &context.source_revision_ref);
        assert_eq!(prompt_typed_failures(&prompt), failures.failures);
        assert_eq!(prompt_source_body(&prompt), source);
        for (index, physical) in [(3, "repository_c"), (2, "repository_b")] {
            let mut repaired = ir.clone();
            repaired.items[index].target_repository_id = physical.to_string();
            repaired.items[index]
                .contract
                .intent_contract
                .as_mut()
                .expect("fixture intent")
                .target = crate::product::logical_codebase::EnrollmentTarget::SingleRepository {
                repository_id: physical.to_string(),
            };
            for (before, after) in ir.items.iter().zip(&repaired.items) {
                assert_eq!(
                    before.contract.input_contracts,
                    after.contract.input_contracts
                );
            }
            assert!(matches!(
                assess_single_candidate_structure(String::new(), &repaired, &context),
                StructuralAssessment::Accepted(_)
            ));
        }
    }

    #[test]
    fn legacy_structure_instruction_digest_keeps_pre_cutover_bytes() {
        use crate::product::workspace_engine::single_candidate_structure_failure::legacy_author_structure_repair_body;
        use crate::product::workspace_engine::types::PlanningCompileFailure;
        use sha2::Digest as _;
        // This pins the already-load-bearing instruction_digest, not new prompt wording.
        // Expected SHA-256 was computed independently with Python hashlib from the
        // pre-cutover body (empty operational cause), including REVISION_TEACHING.
        let failure = PlanningCompileFailure::operational(STRUCTURE_STABLE_CODE, String::new());
        let body = legacy_author_structure_repair_body(&failure);
        assert_eq!(
            hex::encode(sha2::Sha256::digest(body.as_bytes())),
            "8cb01f8988ac125bbc3f0e3bc1b247c617e107026542199bad095ed20fc9deeb"
        );
    }

    /// ① 槽位空闲：拒收返回 sealed 失败证据（原 sealed failure ＋ 原 typed
    ///    结构失败集），不开人工门、不硬失败、零提升，流水恰好一行失败，预算
    ///    不虚记。
    ///
    ///    语义钉点：
    ///    - sealed 失败仍是 Operational 形态 ＋ 原稳定码（**不得迁移 kind**）；
    ///    - sealed failure_id 与 durable 尝试流水同一身份（原 record 写点不变，
    ///      kind/detail 逐字不变）；
    ///    - 随拒收返回的 typed 集是**原**结构失败集（非 sealed 包装重算、非
    ///      文本反解析）；
    ///    - sealed source_ref 指向**失败候选**自己的 revision，不是 last-valid。
    #[tokio::test]
    async fn structure_rejection_with_rerun_slot_free_returns_sealed_failure_without_gate() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        author_round_record(&lifecycle, &mut engine);
        let failed_source = cycle_candidate();
        let completion = engine
            .complete_single_candidate_work_item_plan_author(
                failed_source.clone(),
                fixture_sc_target(&engine),
                fixture_sc_member_targets(&engine),
                true,
            )
            .await
            .expect("槽位空闲时的拒收必须交还失败证据，不得 Err");
        let SingleCandidateAuthorCompletion::Rejected {
            failure,
            structure_failures,
        } = completion
        else {
            panic!("槽位空闲时必须返回 Rejected（sealed 失败证据），得到 {completion:?}");
        };

        // sealed 失败仍是 Operational 形态 ＋ 原稳定码（不得迁移 kind）。
        let PlanningCompileFailureKind::Operational { stable_code, cause } = &failure.kind else {
            panic!(
                "作者阶段结构拒收必须保持 Operational 形态：{:?}",
                failure.kind
            );
        };
        assert_eq!(
            stable_code.as_str(),
            STRUCTURE_STABLE_CODE,
            "结构拒收的稳定码必须原样保留"
        );
        assert!(
            !cause.is_empty(),
            "sealed 失败必须携带原 detail（cause 非空）"
        );
        assert!(
            !failure.failure_id.is_empty(),
            "拒收必须返回已 seal 的失败身份（failure_id 非空）"
        );

        // 原 typed 结构失败集：随拒收原样透传（非 sealed 包装重算、非文本反解析）。
        assert!(
            !structure_failures.failures.is_empty(),
            "拒收必须随附原 typed 结构失败集"
        );
        assert!(
            structure_failures
                .failures
                .iter()
                .any(|failure| matches!(failure.kind, PlanningFailureKind::DependencyCycle { .. })),
            "原 typed 失败集必须保留真实环事实（typed kind，非文本）"
        );
        assert!(
            structure_failures
                .failures
                .iter()
                .all(|failure| !matches!(failure.kind, PlanningFailureKind::Operational { .. })),
            "随附的必须是原结构失败集成员，不得是 sealed Operational 的重包装：{:?}",
            structure_failures.failures
        );

        let session_id = engine.session().session_id.clone();
        let after = lifecycle
            .get_workspace_session(&session_id)
            .expect("reload session after rerun-eligible rejection");
        // 不落人工门、不硬失败：无门快照、相位留在 Generate。
        assert!(
            after.human_gate_snapshot.is_none(),
            "重驱机会还在时不得打开人工门（等作者重驱后再说）"
        );
        assert_eq!(
            after.single_candidate_phase,
            Some(crate::product::models::SingleCandidatePhase::Generate),
            "重驱机会还在时相位必须留在 Generate，不开确认面"
        );
        assert_ne!(
            after.status,
            crate::product::models::WorkspaceSessionStatus::WaitingForHuman,
            "重驱机会还在时会话不得停在人工门"
        );
        // 候选零提升（REQ-SG-03 不放宽）：last-valid 未变。
        assert!(
            after.work_item_plan_source_revision_ref.is_none()
                && after.plan_candidate_ir_ref.is_none(),
            "被拒候选不得提升 current refs（last-valid 不变）"
        );
        let confirm_nodes = engine
            .timeline_nodes
            .iter()
            .filter(|node| matches!(node.node_type, TimelineNodeType::HumanConfirm))
            .count();
        assert_eq!(confirm_nodes, 0, "重驱机会还在时不得出现 HumanConfirm 节点");

        // 每次尝试都进流水：恰好一行失败（通过率读数靠它）。
        let attempts: Attempts = lifecycle
            .list_candidate_attempts(&session_id)
            .expect("read candidate attempt log");
        assert_eq!(attempt_outcomes(&attempts), vec!["failed"]);
        let row = &attempts[0];
        let row_failure = row.failure.as_ref().expect("failure facts");
        assert_eq!(
            row_failure.code, STRUCTURE_STABLE_CODE,
            "尝试流水行的稳定码不得变"
        );
        assert_eq!(
            row_failure.failure_id, failure.failure_id,
            "sealed failure_id 必须与 durable 尝试流水同 id（身份不变）"
        );
        assert_eq!(
            row.attempt_key,
            format!("fail/{}", failure.failure_id),
            "尝试幂等键仍是内容寻址失败身份"
        );
        assert_eq!(
            row_failure.typed_kinds,
            vec![PlanningFailureKind::Operational {
                stable_code: STRUCTURE_STABLE_CODE.to_string(),
                cause: cause.clone(),
            }],
            "尝试流水行的 typed 种类仍是原 Operational 包装（kind/detail 逐字不变）"
        );
        assert_eq!(
            row.stage,
            crate::product::lifecycle_store::candidate_attempts::CandidateAttemptStage::Structure,
            "作者阶段结构拒收行仍记 structure 阶段"
        );
        assert_eq!(
            row.source_revision_ref.as_deref(),
            failure.source_revision_ref.as_deref(),
            "尝试流水行绑定的必须是失败候选的 source revision"
        );
        assert_eq!(
            row.plan_candidate_ir_ref.as_deref(),
            failure.plan_candidate_ir_ref.as_deref(),
            "尝试流水行绑定的必须是失败候选的 IR revision"
        );

        // sealed source_ref 指向失败候选自己的 revision（不是 last-valid）。
        let rejected_source_ref = failure
            .source_revision_ref
            .clone()
            .expect("sealed 失败必须带失败 revision 定位");
        assert!(
            !rejected_source_ref.is_empty(),
            "sealed source_ref 必须指向被拒候选自己的 source revision"
        );

        let rejected_source = failed_revision_source(&lifecycle, &engine, &rejected_source_ref);
        assert_eq!(rejected_source.as_bytes(), failed_source.as_bytes());
        let prompt = structure_prompt(
            &failure,
            &structure_failures,
            &rejected_source,
            &rejected_source_ref,
        );
        assert_eq!(
            prompt_source_body(&prompt).as_bytes(),
            failed_source.as_bytes()
        );
        assert_eq!(prompt_typed_failures(&prompt), structure_failures.failures);

        // 不虚记自动重写预算：durable 不留 running，auto_rewrites_used 不变。
        let status = lifecycle
            .load_candidate_compile_status(&session_id)
            .expect("load candidate compile status")
            .expect("durable candidate compile status must exist");
        assert!(
            status.auto_rewrite.is_none(),
            "不得留下 auto_rewrite running 记录"
        );
        assert_eq!(status.auto_rewrites_used, 0, "作者阶段重驱不扣自动重写预算");

        // 合法候选随后提升 last-valid：被拒候选的 sealed source_ref 仍不是
        // last-valid（拒收零提升 ＋ 定位指向失败 revision）。
        let clean = engine
            .complete_single_candidate_work_item_plan_author(
                clean_candidate(),
                fixture_sc_target(&engine),
                fixture_sc_member_targets(&engine),
                true,
            )
            .await
            .expect("clean candidate must complete");
        assert_eq!(
            clean,
            SingleCandidateAuthorCompletion::Candidate { item_count: 3 },
            "合法候选不受结构拒收重驱语义影响"
        );
        let promoted = lifecycle
            .get_workspace_session(&session_id)
            .expect("reload session after accepted candidate");
        let last_valid = promoted
            .work_item_plan_source_revision_ref
            .clone()
            .expect("合法候选必须提升 last-valid source ref");
        assert_ne!(
            rejected_source_ref, last_valid,
            "被拒候选的 sealed source_ref 必须是失败 revision，不得是 last-valid"
        );
        let last_valid_source = failed_revision_source(&lifecycle, &engine, &last_valid);
        let still_rejected_source =
            failed_revision_source(&lifecycle, &engine, &rejected_source_ref);
        assert_ne!(still_rejected_source, last_valid_source);
        let prompt = structure_prompt(
            &failure,
            &structure_failures,
            &still_rejected_source,
            &rejected_source_ref,
        );
        assert_eq!(
            prompt_source_body(&prompt).as_bytes(),
            failed_source.as_bytes()
        );
        assert_ne!(prompt_source_body(&prompt), last_valid_source);
    }

    /// ② 槽位已占用：第二次拒收落人工门，会话不是 failed；两次拒收在流水中
    ///    留下两行失败（两次尝试、两个身份、同一稳定码、同一阶段）。
    #[tokio::test]
    async fn second_structure_rejection_after_slot_used_lands_human_gate_with_two_rows() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        author_round_record(&lifecycle, &mut engine);
        let first = engine
            .complete_single_candidate_work_item_plan_author(
                cycle_candidate(),
                fixture_sc_target(&engine),
                fixture_sc_member_targets(&engine),
                true,
            )
            .await
            .expect("首轮拒收不得 Err");
        assert!(matches!(
            first,
            SingleCandidateAuthorCompletion::Rejected { .. }
        ));
        // 槽位已被（模拟的）compile/IR 重驱占用 → allow_structure_rerun=false。
        // 第二轮换一份内容不同（改标题）的同构环候选：attempt_key 内容寻址
        // 幂等——同源候选重放＝同一次尝试（零膨胀），真实重驱轮的 source
        // 必然不同，故此处以改标题体现「第二次真实尝试」。
        let second = engine
            .complete_single_candidate_work_item_plan_author(
                cycle_candidate().replace("Backend levels API", "Backend levels API round-2"),
                fixture_sc_target(&engine),
                fixture_sc_member_targets(&engine),
                false,
            )
            .await
            .expect("槽位占用后的拒收同样不得 Err");
        assert_eq!(
            second,
            SingleCandidateAuthorCompletion::Routed,
            "槽位占用后必须回到既有落门收口（Routed）"
        );

        let session_id = engine.session().session_id.clone();
        let after = lifecycle
            .get_workspace_session(&session_id)
            .expect("reload session after second rejection");
        assert_eq!(
            after.status,
            crate::product::models::WorkspaceSessionStatus::WaitingForHuman,
            "重驱后仍被拒必须落人工门"
        );
        assert!(
            after.human_gate_snapshot.is_some(),
            "人工门快照必须在场（owner 拿到人话摘要）"
        );
        assert_ne!(
            after.single_candidate_phase,
            Some(crate::product::models::SingleCandidatePhase::Failed),
            "落门不是终态失败"
        );
        // 门卡节点在场且带非空摘要（摘要由 durable 事实派生；措辞不钉）。
        let summary = engine
            .timeline_nodes
            .iter()
            .rev()
            .find(|node| matches!(node.node_type, TimelineNodeType::HumanConfirm))
            .and_then(|node| node.summary.clone())
            .expect("人工门节点必须带摘要");
        assert!(
            !summary.trim().is_empty(),
            "人工门摘要不得为空（owner 侧可读事实）"
        );

        // 两次拒收 = 两行失败（每次尝试都进流水）。
        let attempts: Attempts = lifecycle
            .list_candidate_attempts(&session_id)
            .expect("read candidate attempt log");
        assert_eq!(attempt_outcomes(&attempts), vec!["failed", "failed"]);
        assert!(attempts.iter().all(|record| {
            record
                .failure
                .as_ref()
                .is_some_and(|failure| failure.code == STRUCTURE_STABLE_CODE)
        }));
        assert!(attempts.iter().all(|record| {
            record.stage
            == crate::product::lifecycle_store::candidate_attempts::CandidateAttemptStage::Structure
        }));
        assert_ne!(
            attempts[0].attempt_seq, attempts[1].attempt_seq,
            "两次尝试序号必须不同（两次真实尝试）"
        );
        assert_ne!(
            attempts[0].attempt_key, attempts[1].attempt_key,
            "两次尝试幂等键必须不同（两次失败身份不同）"
        );
        let status = lifecycle
            .load_candidate_compile_status(&session_id)
            .expect("load candidate compile status")
            .expect("durable candidate compile status must exist");
        assert_eq!(
            status.auto_rewrites_used, 0,
            "两次作者阶段拒收都不扣自动重写预算"
        );
        assert!(status.auto_rewrite.is_none(), "不得留下 running 记录");
    }

    /// ③ 非环拒收（unknown_provider）：随拒收返回的 typed 集必须携带真实的
    ///    「provider 不在本批」事实，且**不得虚构依赖环**；sealed 失败仍是
    ///    Operational 形态 ＋ 原稳定码。
    #[tokio::test]
    async fn unknown_provider_rejection_returns_typed_set_without_cycle_fabrication() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        author_round_record(&lifecycle, &mut engine);
        let failed_source = unknown_provider_candidate();
        let completion = engine
            .complete_single_candidate_work_item_plan_author(
                failed_source.clone(),
                fixture_sc_target(&engine),
                fixture_sc_member_targets(&engine),
                true,
            )
            .await
            .expect("非环拒收同样不得 Err");
        let SingleCandidateAuthorCompletion::Rejected {
            failure,
            structure_failures,
        } = completion
        else {
            panic!("非环拒收在槽位空闲时同样返回 Rejected（sealed 失败证据）：{completion:?}");
        };
        let source_ref = failure
            .source_revision_ref
            .as_deref()
            .expect("sealed source revision");
        let source = failed_revision_source(&lifecycle, &engine, source_ref);
        let prompt = structure_prompt(&failure, &structure_failures, &source, source_ref);
        assert_eq!(
            prompt_source_body(&prompt).as_bytes(),
            failed_source.as_bytes()
        );
        assert_eq!(prompt_typed_failures(&prompt), structure_failures.failures);
        let PlanningCompileFailureKind::Operational { stable_code, .. } = &failure.kind else {
            panic!("非环拒收同样保持 Operational 形态：{:?}", failure.kind);
        };
        assert_eq!(
            stable_code.as_str(),
            STRUCTURE_STABLE_CODE,
            "非环拒收的稳定码同样原样保留"
        );
        assert!(
            !failure.failure_id.is_empty(),
            "非环拒收同样必须返回已 seal 的失败身份"
        );
        assert!(
            !structure_failures.failures.is_empty(),
            "非环拒收必须随附原 typed 结构失败集"
        );
        assert!(
            structure_failures
                .failures
                .iter()
                .any(|failure| matches!(failure.kind, PlanningFailureKind::UnknownProvider { .. })),
            "非环拒收必须随附真实 typed 事实（provider 不在本批）：{:?}",
            structure_failures.failures
        );
        assert!(
            structure_failures.failures.iter().all(|failure| !matches!(
                failure.kind,
                PlanningFailureKind::DependencyCycle { .. }
            )),
            "非环拒收不得虚构依赖环事实：{:?}",
            structure_failures.failures
        );
        assert!(
            structure_failures
                .failures
                .iter()
                .all(|failure| !matches!(failure.kind, PlanningFailureKind::Operational { .. })),
            "随附的必须是原结构失败集成员，不得是 sealed Operational 的重包装：{:?}",
            structure_failures.failures
        );
    }

    /// ⑤ 原 typed 失败集透传：route 必须把**原**结构失败集原样带回（多成员
    ///    不丢、序不变），不得重算、不得从 sealed 文本反解析；sealed 失败
    ///    仍是 Operational 形态 ＋ 原稳定码 ＋ 原失败身份。
    #[tokio::test]
    async fn rejected_sidecar_keeps_original_typed_set_members_and_order() {
        use super::structure_parity::{Repos, ir_of, item, structure_context};
        use crate::product::workspace_engine::plan_projection::assess_single_candidate_structure;
        use crate::product::workspace_engine::single_candidate_structure_failure::author_structure_gate_failure;
        use crate::product::workspace_engine::types::StructuralAssessment;

        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        author_round_record(&lifecycle, &mut engine);

        let repos = Repos::new();
        // 多成员失败集 fixture：环事实 ＋ 未在本批的 provider 事实并存（同一
        // 候选门评估同时产出，两条 typed 失败按评估序排列）。
        let ir = ir_of(vec![
            item("WI-001", "repository_a", None),
            item("WI-002", "repository_b", Some("WI-001")),
            item("WI-003", "repository_c", Some("WI-002")),
            item("WI-004", "repository_b", Some("WI-003")),
            item("WI-005", "repository_c", Some("WI-009")),
        ]);
        let context = structure_context(repos.table(), repos.change_order());
        let StructuralAssessment::Rejected(mut set, _) =
            assess_single_candidate_structure(String::new(), &ir, &context)
        else {
            panic!("多成员失败集 fixture 必须在候选门被拒收");
        };
        assert!(
            set.failures.len() >= 2,
            "多成员失败集 fixture 必须产出多条 typed 失败：{:?}",
            set.failures
        );

        assert!(
            set.failures
                .iter()
                .any(|failure| matches!(failure.kind, PlanningFailureKind::DependencyCycle { .. }))
        );
        assert!(
            set.failures
                .iter()
                .any(|failure| matches!(failure.kind, PlanningFailureKind::UnknownProvider { .. }))
        );
        // A supplied duplicate remains a separate member; no deduplication or sorting.
        set.failures.push(set.failures[0].clone());
        let failure = author_structure_gate_failure(&ir, &context, &set);
        let completion = engine
            .route_author_structure_gate_failure(
                &lifecycle,
                failure,
                set.clone(),
                &context.source_revision_ref,
                &context.plan_candidate_ir_ref,
                true,
            )
            .await;
        let SingleCandidateAuthorCompletion::Rejected {
            failure: sealed,
            structure_failures,
        } = completion
        else {
            panic!("槽位空闲时必须返回 Rejected（sealed 失败证据）：{completion:?}");
        };
        assert_eq!(
            structure_failures, set,
            "随拒收返回的 typed 失败集必须与原结构失败集逐条相等（多成员不丢、序不变）"
        );

        let source = " \t失败候选：中文\r\n```\n正文\n```\n``````\n尾部  ";
        let prompt = structure_prompt(
            &sealed,
            &structure_failures,
            source,
            &context.source_revision_ref,
        );
        assert_eq!(prompt_source_body(&prompt).as_bytes(), source.as_bytes());
        assert_eq!(prompt_typed_failures(&prompt), set.failures);

        // sealed 身份：仍是 Operational ＋ 原稳定码（不得迁移 kind）。
        let PlanningCompileFailureKind::Operational { stable_code, .. } = &sealed.kind else {
            panic!(
                "作者阶段结构拒收必须保持 Operational 形态：{:?}",
                sealed.kind
            );
        };
        assert_eq!(
            stable_code.as_str(),
            STRUCTURE_STABLE_CODE,
            "结构拒收的稳定码必须原样保留"
        );
        assert!(!sealed.failure_id.is_empty(), "sealed 失败身份不得为空");
        assert_eq!(
            sealed.source_revision_ref.as_deref(),
            Some(context.source_revision_ref.as_str()),
            "sealed source_ref 必须是被拒候选自己的 revision"
        );

        // durable 写点不变：同一次 seal 的身份进尝试流水（恰好一行）。
        let session_id = engine.session().session_id.clone();
        let attempts: Attempts = lifecycle
            .list_candidate_attempts(&session_id)
            .expect("read candidate attempt log");
        assert_eq!(attempt_outcomes(&attempts), vec!["failed"]);
        let row_failure = attempts[0].failure.as_ref().expect("failure facts");
        assert_eq!(
            row_failure.failure_id, sealed.failure_id,
            "sealed failure_id 必须与 durable 尝试流水同 id（身份不变）"
        );
    }
}
