use super::*;
use crate::product::lifecycle_store::{
    CompileReservationError, PolicyRoutePersist, single_candidate_approval_attempt_id,
    single_candidate_compile_id,
};
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::work_item_plan_compiler::{
    PlanCandidatePublicationProvenance, verify_publish_freshness,
};
use crate::product::work_item_plan_policy::{PolicyDiagnostic, RunPolicy, WorkItemPlanFlowKind};
use crate::product::work_item_plan_source_store::{SourceStoreScope, WorkItemPlanSourceStore};
use crate::product::work_item_split_validator::WorkItemSplitValidationReport;
use crate::web::workspace_ws_types::WorkItemPlanOutlineCandidateDto;

pub(crate) mod ir_adapter;
mod single_candidate;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialPlanCompileInput {
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub previous_plan: IssueWorkItemPlan,
    pub active_index: WorkItemPlanDraftActiveIndex,
    pub outline_candidate: WorkItemPlanOutlineCandidateDto,
    pub outline_order: Vec<String>,
    pub draft_records: Vec<WorkItemDraftRecord>,
    pub logical_targets: Option<BTreeMap<LogicalRepositoryId, String>>,
    pub repository_id: String,
    pub change_order: Vec<LogicalRepositoryId>,
    pub compile_id: String,
    pub now: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitialPlanCompileDurableContext {
    pub flow_kind: Option<WorkItemPlanFlowKind>,
    pub source_revision_id: Option<String>,
    pub source_revision_ref: Option<String>,
    pub plan_candidate_ir_ref: Option<String>,
    pub mechanical_report_ref: Option<String>,
    pub publication_provenance_ref: Option<String>,
    pub publication_provenance_content_hash: Option<String>,
}

impl InitialPlanCompileDurableContext {
    pub fn legacy() -> Self {
        Self {
            flow_kind: None,
            source_revision_id: None,
            source_revision_ref: None,
            plan_candidate_ir_ref: None,
            mechanical_report_ref: None,
            publication_provenance_ref: None,
            publication_provenance_content_hash: None,
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        let required = [
            ("source_revision_id", self.source_revision_id.as_deref()),
            ("source_revision_ref", self.source_revision_ref.as_deref()),
            (
                "plan_candidate_ir_ref",
                self.plan_candidate_ir_ref.as_deref(),
            ),
            (
                "mechanical_report_ref",
                self.mechanical_report_ref.as_deref(),
            ),
            (
                "publication_provenance_ref",
                self.publication_provenance_ref.as_deref(),
            ),
            (
                "publication_provenance_content_hash",
                self.publication_provenance_content_hash.as_deref(),
            ),
        ];
        match self.flow_kind {
            None => {
                if required.iter().any(|(_, value)| value.is_some()) {
                    return Err("legacy compile durable context must be all None".to_string());
                }
            }
            Some(WorkItemPlanFlowKind::Legacy) => {
                return Err("legacy compile durable context must be all None".to_string());
            }
            Some(WorkItemPlanFlowKind::SingleCandidate) => {
                for (field, value) in required {
                    if value.is_none() {
                        return Err(format!("single candidate durable context missing {field}"));
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedInitialPlanCompile {
    pub transaction: WorkItemPlanCompileTransaction,
    pub compiled_plan: IssueWorkItemPlan,
    pub work_items: Vec<LifecycleWorkItemRecord>,
    pub verification_plans: Vec<VerificationPlan>,
    pub validation_report: WorkItemSplitValidationReport,
    pub publication_input: Option<super::plan_projection::InitialPlanPublicationInput>,
}

#[derive(Debug, Clone)]
pub struct CompileStores {
    pub plan_store: WorkItemPlanStore,
    pub revision_store: WorkItemRevisionStore,
}

/// 将 legacy/IR adapter 已读取的值变成确定性的 initial compile 草稿。
/// 此函数不持有任何 store，亦不读取系统时钟。
pub fn prepare_initial_plan_compile(
    input: InitialPlanCompileInput,
    durable_context: InitialPlanCompileDurableContext,
) -> Result<PreparedInitialPlanCompile, String> {
    durable_context.validate()?;
    let publication_provenance_ref = durable_context.publication_provenance_ref.clone();
    let publication_provenance_content_hash =
        durable_context.publication_provenance_content_hash.clone();
    if input.project_id != input.previous_plan.project_id
        || input.issue_id != input.previous_plan.issue_id
        || input.plan_id != input.previous_plan.id
        || input.project_id != input.active_index.project_id
        || input.issue_id != input.active_index.issue_id
        || input.plan_id != input.active_index.plan_id
    {
        return Err("initial plan compile input identity mismatch".to_string());
    }
    if input.outline_order.is_empty() {
        return Err("initial plan compile requires at least one outline".to_string());
    }
    let outline_to_work_item_id = input
        .outline_order
        .iter()
        .enumerate()
        .map(|(index, outline_id)| {
            (
                outline_id.clone(),
                compile_work_item_id(&input.compile_id, index),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let outline_to_verification_plan_id = input
        .outline_order
        .iter()
        .enumerate()
        .map(|(index, outline_id)| {
            (
                outline_id.clone(),
                compile_verification_plan_id(&input.compile_id, index),
            )
        })
        .collect::<BTreeMap<_, _>>();
    if outline_to_work_item_id.len() != input.outline_order.len() {
        return Err("initial plan compile outline order contains duplicates".to_string());
    }
    let (compiled_plan, work_items, verification_plans) =
        draft_batch::compile_support::project_work_item_plan_drafts_for_compile(
            &input.previous_plan,
            &input.draft_records,
            WorkItemPlanCompileProjectionContext {
                outline_order: &input.outline_order,
                outline_to_work_item_id: &outline_to_work_item_id,
                outline_to_verification_plan_id: &outline_to_verification_plan_id,
                repository_id: &input.repository_id,
                logical_targets: input.logical_targets.as_ref(),
                now: &input.now,
            },
            &input.change_order,
        )?;
    let validation_report =
        WorkItemSplitValidator::validate(&compiled_plan, &work_items, None, &verification_plans);
    let transaction = WorkItemPlanCompileTransaction {
        compile_id: input.compile_id.clone(),
        project_id: input.project_id,
        issue_id: input.issue_id,
        plan_id: input.plan_id,
        flow_kind: durable_context.flow_kind,
        source_revision_id: durable_context.source_revision_id,
        source_revision_ref: durable_context.source_revision_ref,
        plan_candidate_ir_ref: durable_context.plan_candidate_ir_ref,
        mechanical_report_ref: durable_context.mechanical_report_ref,
        publication_provenance_ref: durable_context.publication_provenance_ref,
        publication_provenance_content_hash: durable_context.publication_provenance_content_hash,
        generation_round_id: input.active_index.current_generation_round_id,
        outline_version_ref: input.outline_candidate.outline.id.clone(),
        active_draft_ids: input
            .draft_records
            .iter()
            .map(|record| record.draft_id.clone())
            .collect(),
        status: WorkItemPlanCompileStatus::Preparing,
        plan_commit_state: WorkItemPlanCommitState::NotStarted,
        step_cursor: "preparing".to_string(),
        outline_to_work_item_id: BTreeMap::new(),
        outline_to_verification_plan_id: BTreeMap::new(),
        created_work_item_ids: Vec::new(),
        created_verification_plan_ids: Vec::new(),
        child_session_ids: Vec::new(),
        validator_findings: Vec::new(),
        abort_requested_at: None,
        failure_reason: None,
        previous_plan_snapshot: input.previous_plan.clone(),
        created_at: input.now.clone(),
        updated_at: input.now.clone(),
        committed_at: None,
    };
    let publication_input = if validation_report.has_errors() {
        None
    } else {
        let accepted_drafts = input
            .draft_records
            .iter()
            .map(work_item_draft_revision_from_record)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let logical_ids = input
            .outline_candidate
            .outline
            .work_item_outlines
            .iter()
            .map(|outline| outline.logical_work_item_id.clone())
            .collect::<Vec<_>>();
        let allocated_ids =
            crate::product::work_item_revision_store::allocate_initial_plan_publication_ids(
                &input.previous_plan.project_id,
                &input.previous_plan.issue_id,
                &input.previous_plan.id,
                &input.compile_id,
                &logical_ids,
            )
            .map_err(|error| error.to_string())?;
        Some(super::plan_projection::InitialPlanPublicationInput {
            previous_plan: input.previous_plan,
            outline: input.outline_candidate.outline,
            outline_order: input.outline_order,
            accepted_drafts,
            compile_id: input.compile_id,
            now: input.now,
            allocated_ids,
            publication_provenance_ref,
            publication_provenance_content_hash,
        })
    };
    Ok(PreparedInitialPlanCompile {
        transaction,
        compiled_plan,
        work_items,
        verification_plans,
        validation_report,
        publication_input,
    })
}

/// 执行阶段仅持有 writer；所有输入读取和时间/ID 分配均在 adapter 外层完成。
pub fn execute_initial_plan_compile(
    stores: &CompileStores,
    prepared: PreparedInitialPlanCompile,
) -> Result<(InitialPlanCompileOutcome, WorkItemPlanCompileTransaction), String> {
    let mut transaction = prepared.transaction;
    stores
        .plan_store
        .put_compile_transaction(&transaction)
        .map_err(|error| format!("save compile transaction failed: {error}"))?;

    transaction.status = WorkItemPlanCompileStatus::Validating;
    transaction.step_cursor = "validating".to_string();
    transaction.outline_to_work_item_id = prepared
        .work_items
        .iter()
        .map(|work_item| {
            let outline_id = work_item.source_outline_id.clone().ok_or_else(|| {
                format!(
                    "prepared work item `{}` has no source outline id",
                    work_item.id
                )
            })?;
            Ok((outline_id, work_item.id.clone()))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    transaction.outline_to_verification_plan_id = prepared
        .verification_plans
        .iter()
        .map(|verification_plan| {
            let outline_id = prepared
                .work_items
                .iter()
                .find(|work_item| work_item.id == verification_plan.work_item_id)
                .and_then(|work_item| work_item.source_outline_id.clone())
                .ok_or_else(|| {
                    format!(
                        "prepared verification plan `{}` has no matching work item outline",
                        verification_plan.id
                    )
                })?;
            Ok((outline_id, verification_plan.id.clone()))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    stores
        .plan_store
        .put_compile_transaction(&transaction)
        .map_err(|error| format!("save validating compile transaction failed: {error}"))?;

    transaction.validator_findings = prepared.validation_report.findings.clone();
    if prepared.validation_report.has_errors() {
        transaction.status = WorkItemPlanCompileStatus::Failed;
        transaction.failure_reason = Some(work_item_plan_findings_summary(
            "Final Compile strict validator failed",
            &prepared.validation_report.findings,
        ));
        stores
            .plan_store
            .put_compile_transaction(&transaction)
            .map_err(|error| format!("save failed compile transaction failed: {error}"))?;
        return Err(work_item_plan_findings_summary(
            "Final Compile strict validator failed",
            &prepared.validation_report.findings,
        ));
    }

    transaction.status = WorkItemPlanCompileStatus::Committing;
    transaction.step_cursor = "committing".to_string();
    stores
        .plan_store
        .put_compile_transaction(&transaction)
        .map_err(|error| format!("save committing compile transaction failed: {error}"))?;
    let publication_input = prepared.publication_input.ok_or_else(|| {
        "valid initial compile preparation is missing publication input".to_string()
    })?;
    let journal = super::plan_projection::prepare_initial_plan_publication(publication_input)
        .map_err(|error| error.to_string())?;
    let outcome = publish_initial_plan_revision(&stores.revision_store, &journal)
        .map_err(|error| error.to_string())?;
    Ok((outcome, transaction))
}

mod finalizer;
#[cfg(test)]
mod ir_adapter_tests;

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum WorkItemPlanCompileFinalizerCheckpoint {
    PlanSummaryPrepared,
    FirstChildSessionEnsured,
    FirstChildBindingEnsured,
    FirstChildContextPrepared,
    CompileReportPersisted,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct WorkItemPlanCompileFinalizerFailpointKey {
    project_id: String,
    issue_id: String,
    plan_id: String,
    compile_id: String,
    checkpoint: WorkItemPlanCompileFinalizerCheckpoint,
}

#[cfg(test)]
pub(crate) struct WorkItemPlanCompileFinalizerFailpointGuard {
    key: WorkItemPlanCompileFinalizerFailpointKey,
    registration_id: u64,
}

#[cfg(test)]
static WORK_ITEM_PLAN_COMPILE_FINALIZER_FAILPOINTS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<WorkItemPlanCompileFinalizerFailpointKey, u64>>,
> = std::sync::OnceLock::new();

#[cfg(test)]
static NEXT_WORK_ITEM_PLAN_COMPILE_FINALIZER_FAILPOINT_ID: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

pub(crate) fn next_compile_id() -> String {
    format!("compile_{}", chrono::Utc::now().format("%Y%m%d%H%M%S%3f"))
}

pub(crate) fn compile_work_item_id(compile_id: &str, index: usize) -> String {
    format!("work_item_{compile_id}_{:03}", index + 1)
}

pub(crate) fn compile_verification_plan_id(compile_id: &str, index: usize) -> String {
    format!("verification_plan_{compile_id}_{:03}", index + 1)
}

impl WorkspaceEngine {
    pub(crate) async fn enter_work_item_plan_compile(&mut self) {
        self.enter_work_item_plan_compile_with_auto_confirmation(false)
            .await;
    }

    pub(crate) async fn enter_policy_valid_work_item_plan_compile(&mut self) {
        self.enter_work_item_plan_compile_with_auto_confirmation(true)
            .await;
    }

    async fn enter_work_item_plan_compile_with_auto_confirmation(&mut self, auto_confirm: bool) {
        self.transition_stage(WorkspaceStage::Running).await;
        self.create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::WorkItemPlanCompile,
            agent: None,
            stage: WorkspaceStage::Running,
            round: None,
            title: "WorkItemPlan Final Compile".to_string(),
            summary: Some("编译已确认 Draft 并写入真实 Work Item".to_string()),
            status: TimelineNodeStatus::Active,
        })
        .await;

        match self.run_work_item_plan_compile().await {
            Ok(outcome) => {
                let work_item_count = outcome.work_items.len();
                self.complete_active_node(Some(format!(
                    "Final Compile 完成，已创建 {work_item_count} 个 Work Item"
                )))
                .await;
                if auto_confirm && self.session.run_policy == RunPolicy::AutoIfValid {
                    if let Err(message) = self
                        .complete_policy_valid_work_item_plan(work_item_count)
                        .await
                    {
                        let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                        self.enter_human_confirm(Some(
                            "Final Compile 已完成，但自动确认失败，等待人工确认".to_string(),
                        ))
                        .await;
                    }
                } else {
                    self.enter_human_confirm(Some(format!(
                        "Final Compile 完成，已创建 {work_item_count} 个 Work Item，等待最终确认"
                    )))
                    .await;
                }
            }
            Err(message) => {
                self.complete_active_node(Some(format!("Final Compile 失败：{message}")))
                    .await;
                if self.mark_latest_compile_transaction_recovery_required(&message) {
                    self.enter_work_item_plan_compile_recovery(Some(format!(
                        "Final Compile 需要恢复：{message}"
                    )))
                    .await;
                } else if self.is_current_work_item_plan_batch_mode() {
                    self.enter_work_item_batch_confirm(Some(format!(
                        "Final Compile strict validator 失败：{message}"
                    )))
                    .await;
                } else {
                    self.enter_human_confirm(Some(format!("Final Compile 失败：{message}")))
                        .await;
                }
            }
        }
    }

    async fn complete_policy_valid_work_item_plan(
        &mut self,
        work_item_count: usize,
    ) -> Result<(), String> {
        self.validate_confirm_aggregate_spec_gate()?;
        self.mark_latest_artifact_confirmed(Some("auto_if_valid".to_string()));
        let (plan, child_sessions) = self.confirm_work_item_plan().await?;
        self.transition_stage(WorkspaceStage::Completed).await;
        self.create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::Completed,
            agent: None,
            stage: WorkspaceStage::Completed,
            round: None,
            title: "WorkItemPlan 已自动确认".to_string(),
            summary: Some(format!(
                "Final Compile 完成，plan {} 已自动确认；已创建 {work_item_count} 个 Work Item 和 {} 个子 WorkItem session",
                plan.id,
                child_sessions.len()
            )),
            status: TimelineNodeStatus::Completed,
        })
        .await;
        Ok(())
    }

    pub(crate) async fn enter_work_item_plan_compile_recovery(&mut self, summary: Option<String>) {
        self.transition_stage(WorkspaceStage::HumanConfirm).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::WorkItemPlanCompileRecovery,
                agent: None,
                stage: WorkspaceStage::HumanConfirm,
                round: None,
                title: "WorkItemPlan Compile Recovery".to_string(),
                summary,
                status: TimelineNodeStatus::Active,
            })
            .await;
        if let Some(store) = &self.lifecycle_store {
            let _ = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::WaitingForHuman,
            );
        }
    }

    pub(crate) fn mark_latest_compile_transaction_recovery_required(&self, message: &str) -> bool {
        let Ok(store) = self.work_item_plan_store() else {
            return false;
        };
        let Ok(Some(mut tx)) = store
            .list_compile_transactions(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map(|transactions| {
                transactions
                    .into_iter()
                    .filter(|tx| {
                        matches!(
                            tx.status,
                            WorkItemPlanCompileStatus::Preparing
                                | WorkItemPlanCompileStatus::Validating
                                | WorkItemPlanCompileStatus::Committing
                                | WorkItemPlanCompileStatus::RecoveryRequired
                        )
                    })
                    .max_by(|left, right| left.created_at.cmp(&right.created_at))
            })
        else {
            return false;
        };
        tx.status = WorkItemPlanCompileStatus::RecoveryRequired;
        tx.failure_reason = Some(message.to_string());
        tx.updated_at = chrono::Utc::now().to_rfc3339();
        store.put_compile_transaction(&tx).is_ok()
    }

    #[cfg(test)]
    pub(crate) fn register_work_item_plan_compile_finalizer_failpoint(
        &self,
        compile_id: &str,
        checkpoint: WorkItemPlanCompileFinalizerCheckpoint,
    ) -> WorkItemPlanCompileFinalizerFailpointGuard {
        let key = WorkItemPlanCompileFinalizerFailpointKey {
            project_id: self.session.project_id.clone(),
            issue_id: self.session.issue_id.clone(),
            plan_id: self.session.entity_id.clone(),
            compile_id: compile_id.to_string(),
            checkpoint,
        };
        let registration_id = NEXT_WORK_ITEM_PLAN_COMPILE_FINALIZER_FAILPOINT_ID
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let previous = work_item_plan_compile_finalizer_failpoints()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key.clone(), registration_id);
        assert!(
            previous.is_none(),
            "work item plan compile finalizer failpoint already registered"
        );
        WorkItemPlanCompileFinalizerFailpointGuard {
            key,
            registration_id,
        }
    }

    #[cfg(test)]
    fn maybe_fail_work_item_plan_compile_finalizer(
        &self,
        tx: &WorkItemPlanCompileTransaction,
        checkpoint: WorkItemPlanCompileFinalizerCheckpoint,
    ) -> Result<(), String> {
        let key = WorkItemPlanCompileFinalizerFailpointKey {
            project_id: tx.project_id.clone(),
            issue_id: tx.issue_id.clone(),
            plan_id: tx.plan_id.clone(),
            compile_id: tx.compile_id.clone(),
            checkpoint,
        };
        if work_item_plan_compile_finalizer_failpoints()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&key)
            .is_some()
        {
            return Err(format!(
                "work_item_plan_compile_finalizer_failpoint:{checkpoint:?}"
            ));
        }
        Ok(())
    }

    pub(crate) async fn run_work_item_plan_compile(
        &mut self,
    ) -> Result<InitialPlanCompileOutcome, String> {
        if self.session.flow_kind == WorkItemPlanFlowKind::SingleCandidate {
            return self.run_single_candidate_initial_plan_compile().await;
        }
        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let input = self.legacy_initial_plan_compile_input(
            &lifecycle,
            next_compile_id(),
            chrono::Utc::now().to_rfc3339(),
        )?;
        let failure_report = WorkItemPlanCompileReportPayload {
            compile_id: input.compile_id.clone(),
            generation_round_id: input.active_index.current_generation_round_id.clone(),
            status: WorkItemPlanCompileStatus::Failed,
            plan_commit_state: WorkItemPlanCommitState::NotStarted,
            work_item_ids: Vec::new(),
            verification_plan_ids: Vec::new(),
            child_session_ids: Vec::new(),
            validator_findings: Vec::new(),
        };
        let draft_records = input.draft_records.clone();
        let prepared =
            prepare_initial_plan_compile(input, InitialPlanCompileDurableContext::legacy())?;
        let mut failure_report = failure_report;
        failure_report.validator_findings =
            work_item_split_findings_to_dto(&prepared.validation_report.findings);
        let stores = CompileStores {
            plan_store: self.work_item_plan_store()?,
            revision_store: self.revision_store(),
        };
        let (outcome, mut tx) = match execute_initial_plan_compile(&stores, prepared) {
            Ok(result) => result,
            Err(error) => {
                self.update_artifact(ArtifactPayload::WorkItemPlanCompileReport {
                    compile_report: Box::new(failure_report),
                })
                .await;
                return Err(error);
            }
        };
        let compiled_by_logical_id = outcome
            .work_items
            .iter()
            .map(|item| (item.work_item_revision.logical_work_item_id.as_str(), item))
            .collect::<BTreeMap<_, _>>();
        tx.outline_to_work_item_id = draft_records
            .iter()
            .map(|draft| {
                (
                    draft.outline_id.clone(),
                    draft.candidate.logical_work_item_id.clone(),
                )
            })
            .collect();
        tx.outline_to_verification_plan_id = draft_records
            .iter()
            .map(|draft| {
                let compiled = compiled_by_logical_id
                    .get(draft.candidate.logical_work_item_id.as_str())
                    .expect("compiled outcome must contain every accepted draft");
                (
                    draft.outline_id.clone(),
                    compiled.verification_plan_revision.id.clone(),
                )
            })
            .collect();
        self.finalize_initial_plan_compile(&lifecycle, &stores.plan_store, &mut tx, &outcome)
            .await?;
        Ok(outcome)
    }

    fn legacy_initial_plan_compile_input(
        &self,
        lifecycle: &LifecycleStore,
        compile_id: String,
        now: String,
    ) -> Result<InitialPlanCompileInput, String> {
        let store = self.work_item_plan_store()?;
        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let plan_id = self.session.entity_id.clone();
        let previous_plan = lifecycle
            .get_issue_work_item_plan(&project_id, &issue_id, &plan_id)
            .map_err(|error| format!("load issue work item plan failed: {error}"))?;
        let active_index = store
            .load_active_index(&project_id, &issue_id, &plan_id)
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
        let outline_order = work_item_plan_outline_topological_order(&outline_candidate.outline)?;
        let draft_records =
            self.accepted_active_draft_records_for_compile(&store, &active_index, &outline_order)?;
        let logical_targets =
            self.logical_work_item_plan_repository_targets(lifecycle, &previous_plan)?;
        let repository_id = if logical_targets.is_none() {
            self.work_item_plan_repository_id(lifecycle, &previous_plan)?
        } else {
            String::new()
        };
        let change_order = draft_batch::compile_support::load_change_order_from_confirmed_design(
            lifecycle,
            &previous_plan,
        )?;
        Ok(InitialPlanCompileInput {
            project_id,
            issue_id,
            plan_id,
            previous_plan,
            active_index,
            outline_candidate,
            outline_order,
            draft_records,
            logical_targets,
            repository_id,
            change_order,
            compile_id,
            now,
        })
    }
}

impl WorkspaceEngine {
    pub async fn handle_work_item_plan_compile_recovery_action(
        &mut self,
        action: WorkItemPlanCompileRecoveryActionDto,
        reason: Option<String>,
    ) -> Result<WorkItemPlanCompileRecoveryOutcome, String> {
        if self.session.workspace_type != WorkspaceType::WorkItemPlan
            || self.active_node_type() != Some(TimelineNodeType::WorkItemPlanCompileRecovery)
        {
            return Err(
                "work_item_plan_compile_recovery_action requires active work_item_plan_compile_recovery node"
                    .to_string(),
            );
        }

        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let store = self.work_item_plan_store()?;
        let mut tx = self.latest_work_item_plan_recovery_transaction(&store)?;

        match action {
            WorkItemPlanCompileRecoveryActionDto::AbortAndRollback => {
                if tx.plan_commit_state == WorkItemPlanCommitState::Committed {
                    return Err(
                        "abort_and_rollback is not allowed when plan_commit_state=committed"
                            .to_string(),
                    );
                }
                let revision_store = WorkItemRevisionStore::new(lifecycle.app_paths());
                match revision_store.get_plan_lineage(&tx.project_id, &tx.issue_id, &tx.plan_id) {
                    Ok(lineage) if lineage.active_revision_id.is_some() => {
                        return Err(
                            "abort_and_rollback is not allowed when an active PlanRevision exists; use Continue or HumanTriage"
                                .to_string(),
                        );
                    }
                    Ok(_) => {
                        return Err(
                            "abort_and_rollback is not allowed when an inactive v2 Plan lineage exists; use Continue or HumanTriage"
                                .to_string(),
                        );
                    }
                    Err(ProductStoreError::NotFound { .. }) => {}
                    Err(error) => {
                        return Err(format!(
                            "check v2 Plan lineage during compile recovery failed: {error}"
                        ));
                    }
                }
                match revision_store.get_initial_plan_publication_journal(
                    &tx.project_id,
                    &tx.issue_id,
                    &tx.plan_id,
                    &tx.compile_id,
                ) {
                    Ok(journal) => {
                        return Err(format!(
                            "abort_and_rollback is not allowed when an initial Plan publication journal exists in phase {:?}; use Continue or HumanTriage",
                            journal.phase
                        ));
                    }
                    Err(ProductStoreError::NotFound { .. }) => {}
                    Err(error) => {
                        return Err(format!(
                            "check initial Plan publication journal during compile recovery failed: {error}"
                        ));
                    }
                }

                for verification_plan_id in tx.created_verification_plan_ids.clone() {
                    lifecycle
                        .delete_verification_plan(
                            &tx.project_id,
                            &tx.issue_id,
                            &verification_plan_id,
                        )
                        .map_err(|error| {
                            format!("delete verification plan during rollback failed: {error}")
                        })?;
                }
                for work_item_id in tx.created_work_item_ids.clone() {
                    lifecycle
                        .delete_work_item(&tx.project_id, &tx.issue_id, &work_item_id)
                        .map_err(|error| {
                            format!("delete work item during rollback failed: {error}")
                        })?;
                }
                lifecycle
                    .restore_issue_work_item_plan_snapshot(
                        &tx.project_id,
                        &tx.issue_id,
                        &tx.plan_id,
                        &tx.previous_plan_snapshot,
                    )
                    .map_err(|error| format!("restore previous WorkItemPlan failed: {error}"))?;

                tx.status = WorkItemPlanCompileStatus::Failed;
                tx.created_work_item_ids.clear();
                tx.created_verification_plan_ids.clear();
                tx.child_session_ids.clear();
                tx.failure_reason = Some(
                    reason
                        .unwrap_or_else(|| "compile recovery aborted and rolled back".to_string()),
                );
                tx.step_cursor = "rolled_back".to_string();
                tx.updated_at = chrono::Utc::now().to_rfc3339();
                store.put_compile_transaction(&tx).map_err(|error| {
                    format!("save rolled back compile transaction failed: {error}")
                })?;

                self.complete_active_node(Some(
                    "已放弃本次 Final Compile 并恢复旧 Plan".to_string(),
                ))
                .await;
                self.enter_human_confirm(Some(
                    "Final Compile 已回滚，等待人工确认下一步".to_string(),
                ))
                .await;
                Ok(WorkItemPlanCompileRecoveryOutcome::HumanConfirm)
            }
            WorkItemPlanCompileRecoveryActionDto::Continue => {
                if tx.effective_flow_kind() == WorkItemPlanFlowKind::SingleCandidate {
                    self.validate_single_candidate_transaction_refs(&tx)?;
                }
                // A Prepared publication journal is the pre-active recovery boundary. Even when
                // the last write already made the lineage active (for example, the
                // PlanActivated failpoint fired after that write), resume through the original
                // transaction path so it records `publication_resumed` before finalization.
                let publication_is_pre_active = match WorkItemRevisionStore::new(lifecycle.app_paths())
                    .get_initial_plan_publication_journal(
                        &tx.project_id,
                        &tx.issue_id,
                        &tx.plan_id,
                        &tx.compile_id,
                    ) {
                    Ok(journal) => journal.phase
                        == crate::product::work_item_revision_store::InitialPlanPublicationPhase::Prepared,
                    Err(ProductStoreError::NotFound { .. }) => false,
                    Err(error) => {
                        return Err(format!(
                            "load initial Plan publication journal during Continue failed: {error}"
                        ));
                    }
                };
                let outcome = if tx.effective_flow_kind() == WorkItemPlanFlowKind::SingleCandidate
                    && (publication_is_pre_active
                        || tx.status != WorkItemPlanCompileStatus::Committed)
                {
                    self.resume_single_candidate_compile_transaction(&store, &mut tx)?
                } else if publication_is_pre_active {
                    self.resume_initial_plan_compile_transaction(&store, &mut tx)?
                } else {
                    match self
                        .load_initial_plan_compile_outcome(&tx)
                        .map_err(|error| error.to_string())?
                    {
                        Some(outcome) => outcome,
                        None => self.resume_initial_plan_compile_transaction(&store, &mut tx)?,
                    }
                };
                tx.failure_reason = reason.or(tx.failure_reason);
                self.finalize_initial_plan_compile(&lifecycle, &store, &mut tx, &outcome)
                    .await?;
                self.complete_active_node(Some(
                    "Final Compile 已从原 compile transaction 恢复".to_string(),
                ))
                .await;
                self.enter_human_confirm(Some("Final Compile 已提交，等待最终确认".to_string()))
                    .await;
                Ok(WorkItemPlanCompileRecoveryOutcome::HumanConfirm)
            }
            WorkItemPlanCompileRecoveryActionDto::HumanTriage => {
                tx.failure_reason = reason.or(tx.failure_reason);
                tx.updated_at = chrono::Utc::now().to_rfc3339();
                store.put_compile_transaction(&tx).map_err(|error| {
                    format!("save human triage compile transaction failed: {error}")
                })?;
                self.complete_active_node(Some("Final Compile 转人工处理".to_string()))
                    .await;
                self.enter_human_confirm(Some("Final Compile 需要人工整理".to_string()))
                    .await;
                Ok(WorkItemPlanCompileRecoveryOutcome::HumanConfirm)
            }
        }
    }

    pub(crate) fn latest_work_item_plan_recovery_transaction(
        &self,
        store: &WorkItemPlanStore,
    ) -> Result<WorkItemPlanCompileTransaction, String> {
        store
            .list_compile_transactions(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("list compile transactions failed: {error}"))?
            .into_iter()
            .filter(|tx| tx.status == WorkItemPlanCompileStatus::RecoveryRequired)
            .max_by(|left, right| left.created_at.cmp(&right.created_at))
            .ok_or_else(|| "work item plan compile recovery transaction is missing".to_string())
    }

    pub(crate) fn accepted_active_draft_records_for_compile(
        &self,
        store: &WorkItemPlanStore,
        index: &WorkItemPlanDraftActiveIndex,
        outline_order: &[String],
    ) -> Result<Vec<WorkItemDraftRecord>, String> {
        let mut records = Vec::with_capacity(outline_order.len());
        for outline_id in outline_order {
            let draft_id = index
                .outline_to_current_draft_id
                .get(outline_id)
                .ok_or_else(|| format!("outline `{outline_id}` has no active draft"))?;
            if index.draft_statuses.get(draft_id) != Some(&WorkItemDraftStatus::Accepted) {
                return Err(format!("draft `{draft_id}` is not accepted"));
            }
            let record = store
                .get_draft_record(
                    &index.project_id,
                    &index.issue_id,
                    &index.plan_id,
                    &index.current_generation_round_id,
                    draft_id,
                )
                .map_err(|error| format!("load active draft `{draft_id}` failed: {error}"))?;
            if !record.active || record.status != WorkItemDraftStatus::Accepted {
                return Err(format!(
                    "draft `{draft_id}` is not an accepted active draft"
                ));
            }
            if record.superseded_by_draft_id.is_some()
                || record.supersede_reason.is_some()
                || record.superseded_at.is_some()
            {
                return Err(format!("draft `{draft_id}` has been superseded"));
            }
            records.push(record);
        }
        Ok(records)
    }
}

#[cfg(test)]
fn work_item_plan_compile_finalizer_failpoints() -> &'static std::sync::Mutex<
    std::collections::HashMap<WorkItemPlanCompileFinalizerFailpointKey, u64>,
> {
    WORK_ITEM_PLAN_COMPILE_FINALIZER_FAILPOINTS
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

#[cfg(test)]
impl Drop for WorkItemPlanCompileFinalizerFailpointGuard {
    fn drop(&mut self) {
        let mut failpoints = work_item_plan_compile_finalizer_failpoints()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if failpoints.get(&self.key) == Some(&self.registration_id) {
            failpoints.remove(&self.key);
        }
    }
}
