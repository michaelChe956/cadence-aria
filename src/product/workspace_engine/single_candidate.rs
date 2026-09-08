use sha2::{Digest, Sha256};

use super::*;
use crate::product::models::ProviderName;
use crate::product::work_item_plan_compiler::{
    PlanCandidateValidationContext, WorkItemPlanSourceContext, compile_work_item_plan,
    validate_plan_candidate_ir,
};
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
use crate::product::work_item_plan_source_store::{
    PlanCandidateIrRecord, PlanCandidateMechanicalReportRecord, SourceRevisionRecord,
    WorkItemPlanSourceStore,
};
use crate::web::workspace_ws_types::WorkItemGenerationModeDto;
use crate::web::workspace_ws_types::{ReviewGate, ReviewVerdict, ReviewVerdictType};

/// 仅由已编译 IR 的 item 数与服务端 provider profile 驱动的内部诊断输入。
///
/// 该输入不通过 WebSocket 反序列化；客户端不能提交候选数或 mode。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SingleCandidateGenerationDecisionInput {
    pub provider: ProviderName,
    pub candidate_item_count: usize,
}

/// 在完整 markdown 已编译为 IR 后，确定性记录内部 generation mode。
///
/// 这是诊断，不是 WS/OpenSpec 对外契约；相同 IR item 数与 provider profile 始终产生
/// 相同结果。Pi/Fake profile 保守记录 serial，其余 profile 在三项以内记录 batch。
pub(crate) fn select_internal_generation_mode(
    input: &SingleCandidateGenerationDecisionInput,
) -> WorkItemGenerationModeDto {
    if input.candidate_item_count <= 3
        && !matches!(input.provider, ProviderName::Pi | ProviderName::Fake)
    {
        WorkItemGenerationModeDto::Batch
    } else {
        WorkItemGenerationModeDto::Serial
    }
}

impl WorkspaceEngine {
    pub(crate) fn persist_single_candidate_terminal_phase(
        &mut self,
        phase: crate::product::models::SingleCandidatePhase,
    ) {
        if self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate {
            return;
        }
        let Some(lifecycle) = self.lifecycle_store.as_ref() else {
            self.session.single_candidate_phase = Some(phase);
            return;
        };
        let Ok(expected) = lifecycle.get_workspace_session(&self.session.session_id) else {
            return;
        };
        let status = match phase {
            crate::product::models::SingleCandidatePhase::Completed => {
                crate::product::models::WorkspaceSessionStatus::Confirmed
            }
            crate::product::models::SingleCandidatePhase::Failed => {
                crate::product::models::WorkspaceSessionStatus::Failed
            }
            _ => return,
        };
        if let Ok(saved) =
            lifecycle.compare_and_save_single_candidate_phase(&expected, phase, status)
        {
            self.session.single_candidate_phase = saved.single_candidate_phase;
            self.session.session_status = saved.status;
        }
    }

    /// 先以 durable ledger 保留 provider start，再允许 SingleCandidate author 启动。
    pub(crate) fn reserve_single_candidate_author_start(&mut self) -> Result<bool, String> {
        if self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate {
            return Err(
                "single candidate provider reservation requires SingleCandidate flow".to_string(),
            );
        }
        let lifecycle = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let expected = lifecycle
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| format!("load session for provider reservation failed: {error}"))?;
        let key = format!(
            "single_candidate_author:{}:{}",
            expected.id, expected.run_history.repairs_used
        );
        let (saved, should_start) = lifecycle
            .reserve_single_candidate_provider_start(&expected, &key)
            .map_err(|error| format!("persist provider reservation failed: {error}"))?;
        self.session = WorkspaceSession::from_record(saved);
        Ok(should_start)
    }

    /// F5-A：SC author 修订轮判定。最近一次 review verdict 要求返修（Revise 或
    /// RequiresRevision）时，本轮 SC author 重跑是修订轮，必须把 reviewer findings
    /// 回灌进 prompt；首轮（无 verdict）与 Pass verdict 返回 None，prompt 逐字节不变。
    pub(crate) fn single_candidate_pending_revision_verdict(&self) -> Option<ReviewVerdict> {
        if self.session.workspace_type != WorkspaceType::WorkItemPlan
            || self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate
        {
            return None;
        }
        self.latest_review_verdict.clone().filter(|verdict| {
            verdict.verdict == ReviewVerdictType::Revise
                || verdict.review_gate == ReviewGate::RequiresRevision
        })
    }

    /// 将 SingleCandidate provider 的 markdown 原文通过 compiler 与 typed source store
    /// 落盘，并在 durable Evaluate 后启动阶段 1 的 reviewer 路由。
    pub(crate) async fn complete_single_candidate_work_item_plan_author(
        &mut self,
        source: String,
        target_repository_id: String,
    ) -> Result<usize, String> {
        if self.session.workspace_type != WorkspaceType::WorkItemPlan
            || self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate
        {
            return Err(
                "single candidate author requires a SingleCandidate WorkItemPlan session"
                    .to_string(),
            );
        }
        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let plan_id = self.session.entity_id.clone();
        let plan = lifecycle
            .get_issue_work_item_plan(&project_id, &issue_id, &plan_id)
            .map_err(|error| format!("load plan for single candidate source failed: {error}"))?;
        let repository_profile = plan
            .repository_profile_ref
            .as_deref()
            .map(|profile_id| {
                lifecycle
                    .get_repository_profile(&project_id, &issue_id, profile_id)
                    .map_err(|error| {
                        format!(
                            "load repository profile for single candidate source failed: {error}"
                        )
                    })
            })
            .transpose()?;

        let source_hash = hex::encode(Sha256::digest(source.as_bytes()));
        let source_id = format!("source-{}", &source_hash[..16]);
        let mut source_revision = SourceRevisionRecord {
            id: source_id.clone(),
            source: source.clone(),
            source_revision_hash: source_hash,
            content_hash: String::new(),
        };
        source_revision.content_hash = source_revision
            .content_hash()
            .map_err(|error| format!("hash source revision failed: {error:?}"))?;
        let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
        let source_ref = source_store
            .put_source_revision(&project_id, &issue_id, &plan_id, &source_revision)
            .map_err(|error| {
                format!(
                    "persist source revision failed [{}]: {error:?}",
                    error.code()
                )
            })?;

        let ir = compile_work_item_plan(
            &source,
            &WorkItemPlanSourceContext {
                target_repository_id,
            },
        )
        .map_err(|diagnostics| {
            format_compiler_diagnostics("compile markdown source", &diagnostics)
        })?;
        let ir_id = format!("ir-{}", &source_revision.source_revision_hash[..16]);
        let mut ir_record = PlanCandidateIrRecord {
            id: ir_id.clone(),
            source_revision_id: source_id,
            ir,
            content_hash: String::new(),
        };
        ir_record.content_hash = ir_record
            .content_hash()
            .map_err(|error| format!("hash plan candidate IR failed: {error:?}"))?;
        let ir_ref = source_store
            .put_plan_candidate_ir(&project_id, &issue_id, &plan_id, &ir_record)
            .map_err(|error| {
                format!(
                    "persist plan candidate IR failed [{}]: {error:?}",
                    error.code()
                )
            })?;

        let expected = lifecycle
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| {
                format!("reload workspace session before generated refs CAS failed: {error}")
            })?;
        let generated = lifecycle
            .compare_and_save_single_candidate_generation(&expected, &source_ref, &ir_ref)
            .map_err(|error| format!("persist single candidate generated refs failed: {error}"))?;

        let validation_now = chrono::Utc::now().to_rfc3339();
        let report = validate_plan_candidate_ir(
            &ir_record.ir,
            &PlanCandidateValidationContext {
                project_id: &project_id,
                issue_id: &issue_id,
                plan_id: &plan_id,
                source_story_spec_ids: &plan.source_story_spec_ids,
                source_design_spec_ids: &plan.source_design_spec_ids,
                repository_profile: repository_profile.as_ref(),
                now: &validation_now,
            },
        )
        .map_err(|diagnostics| {
            format_compiler_diagnostics("validate plan candidate IR", &diagnostics)
        })?;
        // F5 回灌扩展（3.6 矩阵 codex×重）：对 IR 既有 canonical 契约跑 Approval
        // compile 同源机械校验（build/validate_dependency_contract_graph）。
        // 存在 Error 级缺口时在本轮 Evaluate 即产生机械返修 verdict（经
        // complete_review 既有 ingestion 驱动 F5-A 修订轮回灌），不再等到
        // Approval compile 才失败；无 Error 缺口时零变化（不产生 verdict）。
        // Warning 级 findings 以 Suggestion+Advisory 搭车同一 verdict。
        let contract_prerevision_verdict =
            contract_prerevision::single_candidate_contract_prerevision_verdict(&ir_record.ir);
        let report_id = format!("report-{}", &ir_record.ir.source_revision_hash[..16]);
        let mut report_record = PlanCandidateMechanicalReportRecord {
            id: report_id,
            source_revision_id: source_revision.id.clone(),
            ir_id: ir_id.clone(),
            report,
            content_hash: String::new(),
        };
        report_record.content_hash = report_record
            .content_hash()
            .map_err(|error| format!("hash mechanical report failed: {error:?}"))?;
        let report_ref = source_store
            .put_mechanical_report(&project_id, &issue_id, &plan_id, &report_record)
            .map_err(|error| {
                format!(
                    "persist mechanical report failed [{}]: {error:?}",
                    error.code()
                )
            })?;
        let saved = lifecycle
            .compare_and_save_single_candidate_evaluation(&generated, &report_ref)
            .map_err(|error| {
                format!("persist single candidate Evaluate report ref failed: {error}")
            })?;
        self.session.work_item_plan_source_revision_ref = saved.work_item_plan_source_revision_ref;
        self.session.plan_candidate_ir_ref = saved.plan_candidate_ir_ref;
        self.session.mechanical_report_ref = saved.mechanical_report_ref;
        self.session.publication_provenance_ref = saved.publication_provenance_ref;
        self.session.single_candidate_phase = saved.single_candidate_phase;
        self.session.session_status = saved.status;

        self.update_artifact(ArtifactPayload::Markdown {
            markdown: source,
            diff: None,
        })
        .await;
        self.complete_active_node(Some(
            "SingleCandidate markdown source 已编译并持久化，等待 Evaluate".to_string(),
        ))
        .await;
        if let Some(verdict) = contract_prerevision_verdict {
            self.route_single_candidate_contract_prerevision(verdict)
                .await;
        } else if self.session.review_rounds == 0 || self.session.reviewer_provider.is_none() {
            self.route_single_candidate_evaluate_without_reviewer()
                .await;
        } else {
            self.start_review().await;
            if self.session.stage == WorkspaceStage::CrossReview {
                self.request_provider_run(ProviderRunKind::ReviewOnly).await;
            }
        }
        Ok(ir_record.ir.items.len())
    }

    /// 机械 canonical 契约缺口（Error 级）的返修路由：与 reviewer verdict 完全
    /// 同一条既有链路——建同一 ReviewerRun 评审轮节点（start_review）、按
    /// durable 候选 refs 重建 invocation scope（与 drive_review_session 同源），
    /// 再经 complete_review 既有 ingestion 完成：
    /// - `latest_review_verdict` 注入 → F5-A 修订轮判定命中，下一轮 SC author
    ///   重跑回灌机械 findings；
    /// - verdict 持久化到 ReviewerRun 节点 → 跨轮指纹对比数据源；
    /// - policy 路由（ContractGap+Repairable）→ TriggerAggregateRepair 重驱
    ///   SC author（repairs_used 与 reviewer 返修轮同池）；连续 2 轮同指纹
    ///   由既有 RepeatedFingerprint 人工门兜底，不新增终止逻辑。
    ///
    /// Fake reviewer 快速路径（start_review 直接进 HumanConfirm）不叠加
    /// synthetic verdict，保持其既有语义零变化。
    pub(crate) async fn route_single_candidate_contract_prerevision(
        &mut self,
        verdict: ReviewVerdict,
    ) {
        self.start_review().await;
        if self.session.stage != WorkspaceStage::CrossReview {
            return;
        }
        if let Err(message) = self.ensure_review_invocation_scope().await {
            let _ = self.event_tx.send(EngineEvent::Error { message }).await;
            self.finish_failed_run().await;
            return;
        }
        let readable = contract_prerevision::contract_prerevision_readable_output(&verdict);
        self.complete_review(ProviderCompletion::plain(readable, None), verdict)
            .await;
    }
}

fn format_compiler_diagnostics(
    operation: &str,
    diagnostics: &[crate::product::work_item_plan_compiler::CompilerDiagnostic],
) -> String {
    let detail = diagnostics
        .iter()
        .map(|diagnostic| {
            format!(
                "{}:{}:{}",
                diagnostic.code, diagnostic.line, diagnostic.message
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!("{operation} failed: {detail}")
}
