use sha2::{Digest, Sha256};

use super::*;
use crate::product::models::{ProviderName, TrustedDraftVerificationCommand};
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

/// 仅由服务端 outline 机械计数与 provider profile 驱动的内部 generation 选择输入。
///
/// 该输入不通过 WebSocket 反序列化；客户端不能提交候选数、prompt 大小或 mode。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SingleCandidateGenerationDecisionInput {
    pub provider: ProviderName,
    pub candidate_item_count: usize,
    pub prompt_bytes: usize,
    pub provider_input_budget_bytes: usize,
}

/// 根据当前已注册 provider profile 的静态输入预算返回服务端能力值。
///
/// `ProviderRegistry` 目前只注册 adapter，没有 capability/profile 查询接口；因此预算
/// 固定在服务端而非从客户端读取。后续 registry 暴露 profile 时可收敛到该接口，选择
/// 规则和 WebSocket 边界不变。
pub(crate) const fn single_candidate_provider_input_budget_bytes(provider: &ProviderName) -> usize {
    match provider {
        ProviderName::ClaudeCode | ProviderName::Codex | ProviderName::KimiCode => 131_072,
        ProviderName::Pi | ProviderName::Fake => 65_536,
    }
}

/// 在 outline 已成功由本地 compiler parser 计数后，确定性选择后续 author 的内部模式。
///
/// 阈值是内部诊断策略，不是 WS/OpenSpec 对外契约；相同输入始终产生相同输出。
pub(crate) fn select_internal_generation_mode(
    input: &SingleCandidateGenerationDecisionInput,
) -> WorkItemGenerationModeDto {
    let prompt_within_budget =
        input.prompt_bytes.saturating_mul(4) <= input.provider_input_budget_bytes.saturating_mul(3);
    if input.candidate_item_count <= 3 && prompt_within_budget {
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

    /// 将 SingleCandidate provider 的 markdown 原文通过 compiler 与 typed source store
    /// 落盘，并在 durable Evaluate 后启动阶段 1 的 reviewer 路由。
    pub(crate) async fn complete_single_candidate_work_item_plan_author(
        &mut self,
        source: String,
        target_repository_id: String,
        trusted_command_catalog: Vec<TrustedDraftVerificationCommand>,
    ) -> Result<(), String> {
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
                trusted_command_catalog,
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
        if self.session.review_rounds == 0 || self.session.reviewer_provider.is_none() {
            self.route_single_candidate_evaluate_without_reviewer()
                .await;
        } else {
            self.start_review().await;
            if self.session.stage == WorkspaceStage::CrossReview {
                self.request_provider_run(ProviderRunKind::ReviewOnly).await;
            }
        }
        Ok(())
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
