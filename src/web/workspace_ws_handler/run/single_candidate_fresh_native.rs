//! T6：SC 评审返修轮的入口 native fresh 装配（拆出以守住单文件行数红线）。
//!
//! 背景：评审裁决（Revise/RequiresRevision）后 policy 路由把 author 重跑委托给
//! `run_single_candidate_author` 接力。旧实现该轮沿用普通 resume builder，而网关
//! 对「resume 证据 Unknown/Denied」稳定拒绝（`provider_gateway_resume_not_supported`，
//! 零 spawn、不静默转 fresh），返修轮因此恒死。入口判定
//! （`WorkspaceEngine::single_candidate_revision_fresh_native_source_ref`）成立时
//! 改走 fresh builder（`resume_provider_session_id` 结构性为 None）并由本模块
//! 装载 durable 上一版 candidate source，使全新物理 native 的返修 prompt 自足；
//! 判定不成立时调用方保持原 resume 路径（fail-closed，不改网关语义）。
use super::*;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::workspace_engine::WorkspaceEngine;

/// 按 durable `source_ref` 装载上一版 candidate source 全文（fresh 返修 prompt
/// 的 `[previous_source]` 段来源）。装载失败保持显式 `Message` 失败面——全新
/// native 拿不到上一版 source 会退化成无上下文的盲改。
#[allow(clippy::result_large_err)] // 与同目录既有 typed 出口同型：错误体必要且不装箱。
pub(super) fn load_previous_candidate_source(
    engine: &WorkspaceEngine,
    lifecycle: &LifecycleStore,
    source_ref: &str,
) -> Result<String, SingleCandidateProviderRunError> {
    let session = engine.session();
    let scope = crate::product::work_item_plan_source_store::SourceStoreScope {
        project_id: session.project_id.clone(),
        issue_id: session.issue_id.clone(),
        plan_id: session.entity_id.clone(),
    };
    crate::product::work_item_plan_source_store::WorkItemPlanSourceStore::new(lifecycle.app_paths())
        .get_source_revision(&scope, source_ref)
        .map_err(|error| {
            SingleCandidateProviderRunError::Message(format!(
                "load previous source for sc fresh revision failed: {error:?}"
            ))
        })
        .map(|revision| revision.source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cross_cutting::streaming_provider::StreamingProviderInput;
    use crate::product::models::{
        ProviderConversationRef, ProviderConversationRole, ProviderName, SingleCandidatePhase,
        WorkspaceType,
    };
    use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
    use crate::product::work_item_plan_source_store::SourceRevisionRecord;
    use crate::product::work_item_plan_source_store::WorkItemPlanSourceStore;
    use crate::product::workspace_engine::PlanAuthorOutputContract;
    use crate::product::workspace_engine::tests::make_work_item_plan_engine_with_accepted_contract_drafts;
    use crate::protocol::contracts::ProviderType;
    use crate::web::workspace_ws_types::{ReviewGate, ReviewVerdict, ReviewVerdictType};
    use sha2::{Digest, Sha256};

    const PREVIOUS_SOURCE: &str = "# Work Item Plan\n## 工作项 WI-001: 上一版内容\n\n旧正文\n";

    fn revise_verdict() -> ReviewVerdict {
        ReviewVerdict {
            verdict: ReviewVerdictType::Revise,
            comments: "上一版缺 exclusive_write_scope".to_string(),
            summary: "需返修".to_string(),
            findings: Vec::new(),
            review_gate: ReviewGate::RequiresRevision,
            work_item_plan_review: None,
            structured_output_diagnostic: None,
        }
    }

    /// 真实落盘上一版 source（durable store），返回其 canonical ref——判定读的
    /// 就是同一 durable 字段，装载面也走生产 loader，不伪造 source 文本。
    fn persist_previous_source(engine: &WorkspaceEngine, lifecycle: &LifecycleStore) -> String {
        let session = engine.session();
        let mut record = SourceRevisionRecord {
            id: "source-t6".to_string(),
            source: PREVIOUS_SOURCE.to_string(),
            source_revision_hash: hex::encode(Sha256::digest(PREVIOUS_SOURCE.as_bytes())),
            content_hash: String::new(),
        };
        record.content_hash = record.content_hash().expect("previous source content hash");
        WorkItemPlanSourceStore::new(lifecycle.app_paths())
            .put_source_revision(
                &session.project_id,
                &session.issue_id,
                &session.entity_id,
                &record,
            )
            .expect("persist previous source revision")
    }

    /// SC 评审裁决后的委托返修轮形态：phase=Generate + 上一版 source ref 已落
    /// durable + 旧 author 会话仍在会话表里（旧 resume builder 会带着它）。
    fn seed_revision_round(engine: &mut WorkspaceEngine, source_ref: &str) {
        engine.session.workspace_type = WorkspaceType::WorkItemPlan;
        engine.session.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
        engine.session.single_candidate_phase = Some(SingleCandidatePhase::Generate);
        engine.session.work_item_plan_source_revision_ref = Some(source_ref.to_string());
        engine.latest_review_verdict = Some(revise_verdict());
        engine.session.provider_conversations = vec![ProviderConversationRef {
            role: ProviderConversationRole::Author,
            provider: ProviderName::ClaudeCode,
            provider_session_id: "author-session-1".to_string(),
            updated_at: chrono::Utc::now().to_rfc3339(),
            last_node_id: Some("node-1".to_string()),
        }];
    }

    fn build_input(
        engine: &WorkspaceEngine,
        fresh: bool,
        prompt: String,
    ) -> StreamingProviderInput {
        let provider_type = ProviderType::ClaudeCode;
        let worktree_path = "/tmp/worktree".to_string();
        if fresh {
            engine
                .build_work_item_plan_streaming_input_fresh(
                    provider_type,
                    prompt,
                    worktree_path,
                    ProviderName::ClaudeCode,
                    PlanAuthorOutputContract::Structured,
                )
                .expect("fresh plan input")
        } else {
            engine
                .build_work_item_plan_streaming_input(
                    provider_type,
                    prompt,
                    worktree_path,
                    ProviderName::ClaudeCode,
                    PlanAuthorOutputContract::Structured,
                )
                .expect("plan input")
        }
    }

    /// 返修轮（pending verdict + durable 上一版 source ref）必须切到 fresh 入口：
    /// input 不再携带 resume id（网关拒绝面消失），且上一版 source 全文可由判定
    /// 返回的 ref 经生产 loader 取回（返修 prompt 自足）。
    #[test]
    fn revision_round_fresh_native_entry_drops_resume_and_keeps_previous_source() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        let source_ref = persist_previous_source(&engine, &lifecycle);
        seed_revision_round(&mut engine, &source_ref);

        let decision_ref = engine
            .single_candidate_revision_fresh_native_source_ref()
            .expect("revision round with a durable source ref must switch to the fresh entry");
        assert_eq!(decision_ref, source_ref);
        let previous_source = load_previous_candidate_source(&engine, &lifecycle, &decision_ref)
            .expect("load the durable previous source");
        assert_eq!(previous_source, PREVIOUS_SOURCE);

        // 对照：原 resume builder 仍会携带旧会话 id —— 正是网关拒绝的那条路径。
        let resumed = build_input(&engine, false, "split prompt".to_string());
        assert_eq!(
            resumed.resume_provider_session_id,
            Some("author-session-1".to_string()),
            "the plain builder must keep resuming the pre-review author session"
        );

        // 新入口：fresh builder（resume=None）+ 自足 prompt（带上一版 source 全文）。
        let prompt = format!("[previous_source]\n{previous_source}\n[review_revision]\n");
        let input = build_input(&engine, true, prompt);
        assert_eq!(
            input.resume_provider_session_id, None,
            "the fresh revision entry must not resume the pre-review author session"
        );
        assert!(
            input.prompt.contains(PREVIOUS_SOURCE),
            "the fresh revision prompt must carry the previous source: {}",
            input.prompt
        );
    }

    /// 对照：首轮（无 verdict）行为逐字节不变——判定不成立，仍走原 resume 路径。
    #[test]
    fn first_round_without_verdict_keeps_resume_path() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        let source_ref = persist_previous_source(&engine, &lifecycle);
        seed_revision_round(&mut engine, &source_ref);
        engine.latest_review_verdict = None;

        assert!(
            engine
                .single_candidate_revision_fresh_native_source_ref()
                .is_none(),
            "the first author round must never switch to the fresh entry"
        );
        assert_eq!(
            build_input(&engine, false, "split prompt".to_string()).resume_provider_session_id,
            Some("author-session-1".to_string()),
            "the first author round must keep the original resume behavior"
        );
    }

    /// 缺 durable source ref ⇒ 不强行 fresh（fail-closed 保留原路径 + 可读 warn）。
    #[test]
    fn missing_durable_source_ref_keeps_resume_path_fail_closed() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        let source_ref = persist_previous_source(&engine, &lifecycle);
        seed_revision_round(&mut engine, &source_ref);
        engine.session.work_item_plan_source_revision_ref = None;

        assert!(
            engine
                .single_candidate_revision_fresh_native_source_ref()
                .is_none(),
            "a revision round without the durable source ref must stay on the resume path"
        );
    }

    /// 非 Generate 相位 / 非 SC 流同样不成立（判定不越界到评审前与 legacy）。
    #[test]
    fn non_generate_phase_or_legacy_flow_keeps_resume_path() {
        let (_tmp, lifecycle, _plan_id, mut engine) =
            make_work_item_plan_engine_with_accepted_contract_drafts();
        let source_ref = persist_previous_source(&engine, &lifecycle);
        seed_revision_round(&mut engine, &source_ref);

        engine.session.single_candidate_phase = Some(SingleCandidatePhase::Evaluate);
        assert!(
            engine
                .single_candidate_revision_fresh_native_source_ref()
                .is_none(),
            "phase=Evaluate must not switch the author round to the fresh entry"
        );

        engine.session.single_candidate_phase = Some(SingleCandidatePhase::Generate);
        engine.session.flow_kind = WorkItemPlanFlowKind::Legacy;
        assert!(
            engine
                .single_candidate_revision_fresh_native_source_ref()
                .is_none(),
            "legacy flow must never consume the single-candidate fresh entry"
        );
    }
}
