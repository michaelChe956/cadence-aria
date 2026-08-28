use sha2::{Digest, Sha256};

use super::*;
use crate::product::work_item_plan_compiler::{
    PlanCandidateValidationContext, WorkItemPlanSourceContext, compile_work_item_plan,
    validate_plan_candidate_ir,
};
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
use crate::product::work_item_plan_source_store::{
    PlanCandidateIrRecord, PlanCandidateMechanicalReportRecord, SourceRevisionRecord,
    WorkItemPlanSourceStore,
};

impl WorkspaceEngine {
    /// 将 SingleCandidate provider 的 markdown 原文通过 compiler 与 typed source store
    /// 落盘。此处只完成 Generate→Evaluate：review/approval/compile 的推进由 WP5 负责。
    pub(crate) async fn complete_single_candidate_work_item_plan_author(
        &mut self,
        source: String,
        target_repository_id: String,
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
                // Provider 不得从 prompt 伪造 trusted catalog；本轮没有 durable catalog
                // 投影时只接受 manual/blocker，后续 WP5 再接真正的 catalog 选择。
                trusted_command_catalog: Vec::new(),
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

        let expected = lifecycle
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| {
                format!("reload workspace session before source refs CAS failed: {error}")
            })?;
        let saved = lifecycle
            .compare_and_save_single_candidate_generation(
                &expected,
                &source_ref,
                &ir_ref,
                &report_ref,
            )
            .map_err(|error| format!("persist single candidate source refs failed: {error}"))?;
        self.session.work_item_plan_source_revision_ref = saved.work_item_plan_source_revision_ref;
        self.session.plan_candidate_ir_ref = saved.plan_candidate_ir_ref;
        self.session.mechanical_report_ref = saved.mechanical_report_ref;
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
        self.enter_author_confirm(Some(
            "SingleCandidate source 已生成；后续 Evaluate/Review/Approval 由工作流推进".to_string(),
        ))
        .await;
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
