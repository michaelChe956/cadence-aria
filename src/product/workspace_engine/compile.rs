use super::*;
use crate::product::entity_deletion_store::{DeletionEntityKind, EntityScope};
use crate::product::entity_operation_store::{
    EntityOperationError, EntityOperationStore, EntityRunLease,
};
use crate::product::issue_automation_store::IssueAutomationStore;
use crate::product::lifecycle_store::workspace_policy_route::carried_author_rewrite_budget;
use crate::product::lifecycle_store::{
    CompileReservationError, PolicyRoutePersist, single_candidate_approval_attempt_id,
    single_candidate_compile_id,
};
use crate::product::logical_codebase::LogicalRepositoryId;
use crate::product::models::automation::{
    AutomationAttentionFact, AutomationAttentionKind, AutomationAttentionStage,
    EnrollmentBindingIdentity,
};
use crate::product::work_item_plan_compiler::{
    PlanCandidatePublicationProvenance, verify_publish_freshness,
};
use crate::product::work_item_plan_policy::{
    HumanGateSnapshot, HumanReason, PolicyDiagnostic, RunPolicy, WorkItemPlanFlowKind,
};
use crate::product::work_item_plan_source_store::{SourceStoreScope, WorkItemPlanSourceStore};
use crate::product::work_item_split_validator::WorkItemSplitValidationReport;
use crate::web::workspace_ws_types::WorkItemPlanOutlineCandidateDto;

pub(crate) mod ir_adapter;
pub(crate) mod prepare_failure;
mod single_candidate;
#[cfg(test)]
pub(crate) use single_candidate::SingleCandidateCompileCheckpoint;

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
    /// 编译输入的 source draft 记录。SC 链由 IR adapter 在内存确定性构造,
    /// 必须在提交段落盘才能让 advance 的 authoritative binding 解析溯源
    /// (legacy 链已在草稿生成阶段落盘,重写为同 id 覆盖,内容相同)。
    pub draft_records: Vec<WorkItemDraftRecord>,
}

#[derive(Debug, Clone)]
pub struct CompileStores {
    pub plan_store: WorkItemPlanStore,
    pub revision_store: WorkItemRevisionStore,
}

/// D2.1b（fix-cascade-deletion Task 3.1；REQ-DEL-06，plan §0.4）：Final
/// Compile 及恢复入口的 owning Plan run lease。
///
/// 在任何 durable 写之前取得（try-shared＋锁内墓碑复核，由
/// `EntityOperationStore::acquire_run` 在 scope 锁内精确分类），整个
/// preparing→validating→committing→finalizer 提交段持有到 final 提交/失败
/// 返回才释放；准入在持期间删除独占立即 Busy——无 attempt 也不能清运行中
/// compile。外层已持同 scope lease（D2.1a spawn 全周期 run lease／enter
/// 状态机帧）时，flock 共享锁跨 open file description 幂等可重入，重取只
/// 做锁内墓碑复核，等价消费，不引入锁序倒置；内层各 store 写沿用
/// D1.2/D1.3 已准入 helper，不在 engine/session/object 锁内首次取 scope。
pub struct WorkItemPlanCompileAdmission {
    _lease: EntityRunLease,
}

impl WorkItemPlanCompileAdmission {
    /// 入口准入（try-shared＋锁内墓碑复核）：purging/deleted/争用由
    /// `EntityOperationStore::acquire_run` 在 scope 锁内精确分类。
    pub fn acquire(
        paths: &crate::product::app_paths::ProductAppPaths,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<Self, EntityOperationError> {
        let scope = EntityScope::new(project_id, issue_id, DeletionEntityKind::Plan, plan_id);
        scope.validate().map_err(EntityOperationError::Store)?;
        Ok(Self {
            _lease: EntityOperationStore::new(paths.clone()).acquire_run(&scope)?,
        })
    }
}

/// D2.1b：准入失败的稳定错误串（engine Err 通道；message 前缀携带稳定
/// store kind，D2.1a/D2.1d 同惯例）。
pub fn work_item_plan_compile_admission_error(
    error: EntityOperationError,
    plan_id: &str,
) -> String {
    let code = match &error {
        EntityOperationError::Deleting => "entity_deleting",
        EntityOperationError::Deleted => "entity_deleted",
        EntityOperationError::Busy => "entity_busy",
        EntityOperationError::Store(_) => "entity_admission_failed",
    };
    format!("{code}: work item plan {plan_id} compile admission rejected: {error}")
}

/// 将 legacy/IR adapter 已读取的值变成确定性的 initial compile 草稿；不持有 store、不读系统时钟。
pub fn prepare_initial_plan_compile(
    input: InitialPlanCompileInput,
    durable_context: InitialPlanCompileDurableContext,
) -> Result<PreparedInitialPlanCompile, super::types::InitialPlanCompileFailure> {
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
        return Err("initial plan compile input identity mismatch".into());
    }
    if input.outline_order.is_empty() {
        return Err("initial plan compile requires at least one outline".into());
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
        return Err("initial plan compile outline order contains duplicates".into());
    }
    let (compiled_plan, work_items, verification_plans) =
        prepare_failure::project_work_item_plan_drafts(
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
        draft_records: input.draft_records,
    })
}

/// 把编译输入的 source draft 记录落盘(8.3 review Major-1 修复)。
///
/// * SC 链:IR adapter 只在内存构造 `InitialPlanCompileInput.draft_records`,
///   不落盘则生产 advance 的 authoritative binding 解析必落
///   `source_draft_error` → `mixed_target_group_rejected`,SC confirm→advance
///   是死路。draft_id=`draft_{compile_id}_{n}`、generation_round_id=
///   `single_candidate_{compile_id}` 均由 durable compile reservation
///   确定性派生,编译恢复重放是同路径覆盖写,天然幂等。
/// * legacy 链:草稿在生成阶段已落盘,重写为同 id 同内容覆盖(仅在共享
///   提交段被调用时发生);此函数只写不读,不改变 SC compile
///   「不读草稿库」的纪律(panic_on_legacy_compile_reads 只挂读方法)。
fn persist_initial_plan_compile_draft_records(
    store: &WorkItemPlanStore,
    records: &[WorkItemDraftRecord],
) -> Result<(), String> {
    for record in records {
        store.put_draft_record(record).map_err(|error| {
            format!(
                "persist compile source draft `{}` failed: {error}",
                record.draft_id
            )
        })?;
    }
    Ok(())
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

    // D2.1b 探针：preparing 首次落盘位点（运行中 compile 的删除 busy 证据）。
    #[cfg(test)]
    probe_work_item_plan_compile_admission(
        &transaction.project_id,
        &transaction.issue_id,
        &transaction.plan_id,
        "preparing",
    );
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

    // D2.1b 探针：validating 位点。
    #[cfg(test)]
    probe_work_item_plan_compile_admission(
        &transaction.project_id,
        &transaction.issue_id,
        &transaction.plan_id,
        "validating",
    );
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

    // D2.1b 探针：committing 位点。
    #[cfg(test)]
    probe_work_item_plan_compile_admission(
        &transaction.project_id,
        &transaction.issue_id,
        &transaction.plan_id,
        "committing",
    );
    // SC 链的 source draft 必须先于 publication 落盘:任何 durable work item
    // revision 一旦可见,其 source_draft_revision_id 必须可在草稿库解析。
    // legacy 链同样重写一份同 id 同内容记录(幂等覆盖)。
    persist_initial_plan_compile_draft_records(&stores.plan_store, &prepared.draft_records)?;
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
// 单测(part_03)按 `compile::work_item_record_attribution` 路径引用该模块;库构建无消费方。
#[cfg(test)]
pub(crate) use finalizer::work_item_record_attribution;
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
    WorkItemRecordsPersisted,
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

// ── D2.1b 测试探针（cfg(test)，plan Step 1「各 checkpoint 删除 busy」）──
//
// 在真实 compile/恢复的各 durable checkpoint 处回调，供测试验证运行中
// compile 持 run lease 期间删除独占立即 Busy 且零墓碑。线程级注册表：探针
// 与被测 compile 同线程（#[tokio::test] current_thread），并行测试线程的
// 编译互不可见，无跨夹具串扰；guard 撤销注册。
#[cfg(test)]
pub(crate) type WorkItemPlanCompileAdmissionProbeFn = std::rc::Rc<dyn Fn()>;

#[cfg(test)]
thread_local! {
    static WORK_ITEM_PLAN_COMPILE_ADMISSION_PROBES: std::cell::RefCell<
        std::collections::HashMap<String, WorkItemPlanCompileAdmissionProbeFn>,
    > = std::cell::RefCell::new(std::collections::HashMap::new());
}

#[cfg(test)]
pub(crate) struct WorkItemPlanCompileAdmissionProbeGuard {
    key: String,
    probe: WorkItemPlanCompileAdmissionProbeFn,
}

#[cfg(test)]
impl Drop for WorkItemPlanCompileAdmissionProbeGuard {
    fn drop(&mut self) {
        WORK_ITEM_PLAN_COMPILE_ADMISSION_PROBES.with(|probes| {
            let mut probes = probes.borrow_mut();
            if probes
                .get(&self.key)
                .is_some_and(|registered| std::rc::Rc::ptr_eq(registered, &self.probe))
            {
                probes.remove(&self.key);
            }
        });
    }
}

#[cfg(test)]
pub(crate) fn register_work_item_plan_compile_admission_probe(
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    checkpoint: &str,
    probe: impl Fn() + 'static,
) -> WorkItemPlanCompileAdmissionProbeGuard {
    let key = format!("{project_id}|{issue_id}|{plan_id}|{checkpoint}");
    let probe: WorkItemPlanCompileAdmissionProbeFn = std::rc::Rc::new(probe);
    WORK_ITEM_PLAN_COMPILE_ADMISSION_PROBES.with(|probes| {
        let previous = probes.borrow_mut().insert(key.clone(), probe.clone());
        assert!(
            previous.is_none(),
            "work item plan compile admission probe already registered: {key}"
        );
    });
    WorkItemPlanCompileAdmissionProbeGuard { key, probe }
}

#[cfg(test)]
fn probe_work_item_plan_compile_admission(
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    checkpoint: &str,
) {
    let key = format!("{project_id}|{issue_id}|{plan_id}|{checkpoint}");
    WORK_ITEM_PLAN_COMPILE_ADMISSION_PROBES.with(|probes| {
        if let Some(probe) = probes.borrow().get(&key).cloned() {
            probe();
        }
    });
}

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
        self.enter_work_item_plan_compile_with_auto_confirmation(false, false)
            .await;
    }
    pub(crate) async fn enter_policy_valid_work_item_plan_compile(&mut self) {
        self.enter_work_item_plan_compile_with_auto_confirmation(true, false)
            .await;
    }
    /// 批 2 P3：调用方已在同一确认链、同一 engine 锁内完成
    /// `ensure_confirmation_trust_ready`（`handle_confirm` 的 SC Approval 臂与
    /// `close_human_gate` Approve 臂——两处在直入 compile 前均已做 trust 按需
    /// 准备）时使用本入口：跳过 P3 的二次 trust 预检（避免对同一确认重复
    /// 登记/重复计数），但保持 Ready 路径的停点事实 resolve。
    pub(crate) async fn enter_policy_valid_work_item_plan_compile_trust_ready(&mut self) {
        self.enter_work_item_plan_compile_with_auto_confirmation(true, true)
            .await;
    }
    /// F-42/F-49：收口仍处 Active 的 HumanConfirm 门节点。
    ///
    /// F-42：确认链（SC 人门 approve → Final Compile → Confirmed）离开 human_confirm
    /// 时收口门节点。此前链路只关 compile 节点，门节点 human_confirm 留在 Active——
    /// 持久层不自洽（实测 workspace_session_0003：node_004 human_confirm 仍 active，
    /// 而 005 compile / 006 completed 已 completed），重连投影据残留 Active 节点恢复
    /// 门态，对话流门卡仍渲染可点的确认/终止按钮，二次点击被服务端矩阵拒
    /// （INVALID_MESSAGE_FOR_STAGE: confirm not allowed in stage completed）。
    ///
    /// F-49：F-42 的判据是「活动节点恰为 Active HumanConfirm」——门内轮次切换后该
    /// 前提不再成立：人工反馈触发的修订轮在 author 节点上执行、复评在 review 节点上
    /// 执行，活动节点早已推进，被取代的旧门节点滞留 Active（实测
    /// workspace_session_0009：timeline_node_006 与 timeline_node_010 同时 active）。
    /// 故判据放宽为「收口所有 HumanConfirm + Active 节点」——同一会话至多一个当前门，
    /// 多个 Active 门即持久层不自洽。
    ///
    /// 只触碰 HumanConfirm + Active 节点，不得覆写其他节点的状态/摘要/完成时间。
    /// 本入口的两类 callsite：
    /// - 终态确认链：`conversational_gate.rs` 的 close_human_gate approve 臂与
    ///   `controls.rs` 的 handle_confirm（HumanConfirm + SC Approval 臂）——活动节点
    ///   正是待收口的门节点；以及 `review/routing.rs` 的 apply_policy_route
    ///   （ContinueToCompleted + SC + AutoIfValid）、batch review Pass、serial draft
    ///   完结三处直入 compile——活动节点是 review/draft 节点，无 Active 门节点时为
    ///   no-op（评审要求「跳过可选建议」的路径经
    ///   `work_item_plan_optional_pass_review` → `enter_review_decision`，不直入本入口）；
    /// - 门内轮次切换：`decisions.rs` 的 enter_human_confirm 在建新一轮门节点前收口
    ///   被取代的旧门。
    ///
    /// 时间/摘要走既有同源链路（update_timeline_node 取 now，与 compile 节点一致）。
    pub(crate) async fn close_active_human_confirm_node(&mut self, summary: &str) {
        let superseded: Vec<String> = self
            .timeline_nodes
            .iter()
            .filter(|node| {
                node.node_type == TimelineNodeType::HumanConfirm
                    && node.status == TimelineNodeStatus::Active
            })
            .map(|node| node.node_id.clone())
            .collect();
        for node_id in superseded {
            self.update_timeline_node(
                &node_id,
                TimelineNodeStatus::Completed,
                Some(summary.to_string()),
            )
            .await;
        }
    }
    async fn enter_work_item_plan_compile_with_auto_confirmation(
        &mut self,
        auto_confirm: bool,
        trust_preverified: bool,
    ) {
        // D2.1b：compile 状态机帧的 run admission——覆盖 compile 前的门
        // 收口/timeline 写、compile 本体与失败后的 mark_recovery/恢复
        // 节点写入（「无 attempt 也不能清运行中 compile」的完整窗口）；
        // compile 本体自取 admission 时共享 flock 幂等可重入（等价消费）。
        // 删除已赢（purging/deleted/争用）时不启动任何 durable 面，错误
        // 经事件通道上抛。
        let _admission = match self.acquire_work_item_plan_compile_admission() {
            Ok(admission) => admission,
            Err(message) => {
                let _ = self
                    .event_tx
                    .send(EngineEvent::Error {
                        message: format!("work item plan compile admission rejected: {message}"),
                    })
                    .await;
                return;
            }
        };
        // 批 2 P3（REQ-WIGA-02/12）：自动确认臂（与 compile 完成后的
        // auto-confirm 同一判据，见 `work_item_plan_compile_auto_confirm_arm`）
        // 在进入 compile 之前先复用 lifecycle.rs 的
        // `ensure_confirmation_trust_ready` 做确认 trust 按需准备：
        // - 非 Ready 返回 typed `ConfirmationTrustWaiting`：零 compile 副作用
        //   （不建 compile 节点、不推进 stage、不跑 compile），保留 waiting
        //   原 reason/session 目标；对精确绑定 enabled enrollment 的会话经
        //   N0 store upsert `plan_question(stage=plan)`——未开页面也可读事实
        //   通知；落人工确认门兜底（人工确认路径自身会再核 trust）。
        // - trust Ready（含调用方在同一确认链内已验证的 `trust_preverified`
        //   入口——不再二次预检，避免对同一确认重复登记/重复计数）：先
        //   resolve 既有停点事实再继续（Ready resolve 后自动 Confirmed）；
        //   resolve 失败仅经 Error 事件上抛，不阻断已合法的自动确认链
        //   （停点条件已消失，不因通知簿记失败让链路倒退）。
        if self.work_item_plan_compile_auto_confirm_arm(auto_confirm) {
            if !trust_preverified
                && let Err(waiting) = self
                    .ensure_confirmation_trust_ready(
                        self.session.author_provider.clone(),
                        self.session.reviewer_provider.clone(),
                    )
                    .await
            {
                if let Err(error) = self.upsert_automation_confirmation_trust_fact(&waiting) {
                    let _ = self
                        .event_tx
                        .send(EngineEvent::Error {
                            message: format!(
                                "upsert automation plan_question for confirmation trust \
                                 waiting failed: {error}"
                            ),
                        })
                        .await;
                }
                // k3 波1 F1：停等落门前补建 SC 门快照（幂等助手，与 compile
                // 失败第三分支同构、预算自 durable 计数接续）——AutoIfValid
                // 经 ContinueToCompleted 路由只落 (Running, None)，人工按
                // 「修复后重新确认」走 close_human_gate(Approve) 会被静态
                // 资格（要求快照在场）恒拒，自动链在 compile 前永久停滞。
                self.ensure_human_gate_snapshot_after_compile_failure()
                    .await;
                self.enter_human_confirm(Some(format!(
                    "确认 trust 未就绪（{}），已暂停自动 Final Compile，等待人工确认",
                    waiting.reason_code()
                )))
                .await;
                return;
            }
            if let Err(error) = self.resolve_automation_confirmation_trust_fact() {
                let _ = self
                    .event_tx
                    .send(EngineEvent::Error {
                        message: format!(
                            "resolve automation plan_question after confirmation trust \
                             became ready failed: {error}"
                        ),
                    })
                    .await;
            }
        }
        self.close_active_human_confirm_node("已确认通过").await;
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
                if self.work_item_plan_compile_auto_confirm_arm(auto_confirm) {
                    if let Err(message) = self
                        .complete_policy_valid_work_item_plan(work_item_count)
                        .await
                    {
                        let _ = self.event_tx.send(EngineEvent::Error { message }).await;
                        self.enter_human_confirm(Some(
                            "Final Compile 已完成，但确认落盘失败，等待人工确认".to_string(),
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
                } else if !self.route_compile_failure_to_rewrite().await {
                    // R3（oracle 裁决）：落门持久化 WaitingForHuman 之前先补建
                    // SC 门快照，保证 HumanGateOpened 事件对应的 SessionState
                    // 广播已带门快照（反馈入口读内存/持久快照）。
                    self.ensure_human_gate_snapshot_after_compile_failure()
                        .await;
                    self.enter_human_confirm(Some(format!("Final Compile 失败：{message}")))
                        .await;
                }
            }
        }
    }
    pub(crate) async fn complete_policy_valid_work_item_plan(
        &mut self,
        work_item_count: usize,
    ) -> Result<(), String> {
        self.validate_confirm_aggregate_spec_gate()?;
        let confirmed_by = match self.session.run_policy {
            RunPolicy::AutoIfValid => "auto_if_valid",
            RunPolicy::Interactive => "human",
        };
        self.mark_latest_artifact_confirmed(Some(confirmed_by.to_string()));
        let (plan, child_sessions) = self.confirm_work_item_plan().await?;
        self.transition_stage(WorkspaceStage::Completed).await;
        self.persist_single_candidate_terminal_phase(
            crate::product::models::SingleCandidatePhase::Completed,
        );
        self.create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::Completed,
            agent: None,
            stage: WorkspaceStage::Completed,
            round: None,
            title: "WorkItemPlan 已确认".to_string(),
            summary: Some(format!(
                "Final Compile 完成，plan {} 已确认；已创建 {work_item_count} 个 Work Item 和 {} 个子 WorkItem session",
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
        // 批 2 P3（REQ-WIGA-12）：恢复门打开即对精确绑定 enabled enrollment
        // 的会话 upsert plan_question(stage=plan)，focus 取真实 gate（durable
        // compile transaction 的 compile_id，不伪造）；绑定不匹配的会话零
        // 写入；无法解析真实 gate/写失败经 Error 事件上抛，门保持开态。
        if let Err(error) = self.upsert_automation_compile_recovery_fact() {
            let _ = self
                .event_tx
                .send(EngineEvent::Error {
                    message: format!(
                        "upsert automation plan_question for compile recovery gate failed: {error}"
                    ),
                })
                .await;
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
    // ─── 批 2 P3（REQ-WIGA-02/12）：trust/compile 停点的 plan_question 写清 ───
    /// 批 2 P3：本次 compile 入口是否为自动确认臂（与 compile 完成后的
    /// auto-confirm 判据同一表达式：显式 policy-valid 入口，且
    /// run_policy=AutoIfValid 或 SC Approval 相位）。
    fn work_item_plan_compile_auto_confirm_arm(&self, auto_confirm: bool) -> bool {
        auto_confirm
            && (self.session.run_policy == RunPolicy::AutoIfValid
                || (self.session.flow_kind == WorkItemPlanFlowKind::SingleCandidate
                    && self.session.single_candidate_phase
                        == Some(crate::product::models::SingleCandidatePhase::Approval)))
    }

    /// 批 2 P3：engine 层调 N0 store 的依赖接法（接法裁定与理由记录于
    /// `.superpowers/sdd/b2wave1-P3-report.md`）：不新增引擎字段/构造参数——
    /// app_paths 取自引擎持有的 lifecycle_store（与 advance.rs D2.1d retry 的
    /// `IssueAutomationStore::new(app_paths)` 同一先例）；绑定身份按
    /// (project_id, issue_id) 读 enrollment，仅当 enabled 且当前代绑定与本
    /// 会话 (entity_id=plan, session_id) 精确相等才返回 Some——manual/未绑定/
    /// 错绑会话一律零写入；enrollment 读取失败原样上抛（不吞错、不猜绑定）。
    fn automation_attention_binding(
        &self,
    ) -> Result<Option<(IssueAutomationStore, EnrollmentBindingIdentity)>, String> {
        let Some(lifecycle) = self.lifecycle_store.as_ref() else {
            return Ok(None);
        };
        let store = IssueAutomationStore::new(lifecycle.app_paths().clone());
        let Some(enrollment) = store
            .get(&self.session.project_id, &self.session.issue_id)
            .map_err(|error| format!("read automation enrollment failed: {error}"))?
        else {
            return Ok(None);
        };
        if !enrollment.enabled {
            return Ok(None);
        }
        match enrollment.binding_history.map(|history| history.current) {
            Some(binding)
                if binding.plan_id == self.session.entity_id
                    && binding.session_id == self.session.session_id =>
            {
                Ok(Some((store, binding)))
            }
            _ => Ok(None),
        }
    }

    /// 批 2 P3：trust 停点事实的稳定 key（durable 会话身份派生：plan+session
    /// +停点类别；与 compile 恢复门事实互不重叠）。
    fn automation_confirmation_trust_fact_key(&self) -> String {
        format!(
            "{}:{}:confirmation_trust_waiting",
            self.session.entity_id, self.session.session_id
        )
    }

    /// 批 2 P3：trust 非 Ready 的停点 upsert——reason 保留 typed waiting 的
    /// 原 reason_code（Home 原码透传/Context 稳定码）；trust 停点尚无
    /// durable 门 id，不伪造 focus。
    fn upsert_automation_confirmation_trust_fact(
        &self,
        waiting: &super::types::ConfirmationTrustWaiting,
    ) -> Result<(), String> {
        let Some((store, binding)) = self.automation_attention_binding()? else {
            return Ok(());
        };
        let fact = AutomationAttentionFact {
            key: self.automation_confirmation_trust_fact_key(),
            kind: AutomationAttentionKind::PlanQuestion,
            stage: AutomationAttentionStage::Plan,
            plan_id: Some(self.session.entity_id.clone()),
            session_id: Some(self.session.session_id.clone()),
            attempt_id: None,
            reason_code: waiting.reason_code().to_string(),
            occurred_at: chrono::Utc::now().to_rfc3339(),
            choice_id: None,
            gate_id: None,
        };
        store
            .upsert_automation_attention_fact(
                &self.session.project_id,
                &self.session.issue_id,
                &binding,
                fact,
            )
            .map_err(|error| format!("upsert confirmation trust waiting fact failed: {error}"))
    }

    /// 批 2 P3：trust 转 Ready 后清对应停点事实（N0 resolve 同 key 幂等；
    /// 未绑定/manual 会话零调用）。
    fn resolve_automation_confirmation_trust_fact(&self) -> Result<(), String> {
        let Some((store, binding)) = self.automation_attention_binding()? else {
            return Ok(());
        };
        store
            .resolve_automation_attention_fact(
                &self.session.project_id,
                &self.session.issue_id,
                &binding,
                &self.automation_confirmation_trust_fact_key(),
            )
            .map_err(|error| format!("resolve confirmation trust waiting fact failed: {error}"))
    }

    /// 批 2 P3：compile 恢复门事实的稳定 key（plan+真实 compile_id；同一
    /// transaction 重开不换 key，不同 compile 不同 key）。
    fn automation_compile_recovery_fact_key(plan_id: &str, compile_id: &str) -> String {
        format!("{plan_id}:{compile_id}:compile_recovery")
    }

    /// 批 2 P3：compile 恢复门打开的停点 upsert——focus 取真实 gate（durable
    /// compile transaction 的 compile_id）；无 RecoveryRequired transaction
    /// 时无从取真实 gate，不伪造、报错上抛。
    fn upsert_automation_compile_recovery_fact(&self) -> Result<(), String> {
        let plan_store = self
            .work_item_plan_store()
            .map_err(|error| format!("load plan store for attention fact failed: {error}"))?;
        let tx = self
            .latest_work_item_plan_recovery_transaction(&plan_store)
            .map_err(|error| format!("load recovery compile transaction failed: {error}"))?;
        let Some((store, binding)) = self.automation_attention_binding()? else {
            return Ok(());
        };
        let fact = AutomationAttentionFact {
            key: Self::automation_compile_recovery_fact_key(
                &self.session.entity_id,
                &tx.compile_id,
            ),
            kind: AutomationAttentionKind::PlanQuestion,
            stage: AutomationAttentionStage::Plan,
            plan_id: Some(self.session.entity_id.clone()),
            session_id: Some(self.session.session_id.clone()),
            attempt_id: None,
            reason_code: "work_item_plan_compile_recovery_required".to_string(),
            occurred_at: chrono::Utc::now().to_rfc3339(),
            choice_id: None,
            gate_id: Some(tx.compile_id.clone()),
        };
        store
            .upsert_automation_attention_fact(
                &self.session.project_id,
                &self.session.issue_id,
                &binding,
                fact,
            )
            .map_err(|error| format!("upsert compile recovery attention fact failed: {error}"))
    }

    /// 批 2 P3：恢复动作成功关门后清对应停点事实（key 不存在幂等成功）。
    fn resolve_automation_compile_recovery_fact(&self, compile_id: &str) -> Result<(), String> {
        let Some((store, binding)) = self.automation_attention_binding()? else {
            return Ok(());
        };
        store
            .resolve_automation_attention_fact(
                &self.session.project_id,
                &self.session.issue_id,
                &binding,
                &Self::automation_compile_recovery_fact_key(&self.session.entity_id, compile_id),
            )
            .map_err(|error| format!("resolve compile recovery attention fact failed: {error}"))
    }

    /// R3（oracle 裁决）：Final Compile 失败落 HumanConfirm 的第三分支此前不建
    /// human gate snapshot，`handle_human_gate_feedback` 的硬前置
    /// （conversational_gate「human gate snapshot is missing」拒收）令该门上的
    /// 反馈入口形同虚设（缺口主体是 AutoIfValid 链：ContinueToCompleted 路由只落
    /// (Running, None)，见 policy_route_record_values）。仅对 SingleCandidate 且
    /// 快照缺席的会话补建：在场快照（Interactive 链自 Approval 门接续保留的，
    /// close(Running) 与 WaitingForHuman 写入均不清快照）原样保留，不做重置。
    ///
    /// 预算口径（Task 4，工作包 1.5）：`manual_repairs_remaining` := 统一权威
    /// 余额（余额未知 ⇒ 0 保守）；自动段随同一 CAS 结转清零（防后续读双计）。
    /// ⚠️ **未决（Reserved 未消费）预领扣减在场时，快照与账本都不吞并它**
    /// （快照走既有初值公式、账本原样保留）——否则 Task 3 对账退款将找不到键 ⇒
    /// 白占额度（owner 永久失去一个从未跑过的重写槽位）；与另两处门快照站点同口径。
    pub(crate) async fn ensure_human_gate_snapshot_after_compile_failure(&mut self) {
        if self.session.workspace_type != WorkspaceType::WorkItemPlan
            || self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate
            || self.session.human_gate_snapshot.is_some()
        {
            return;
        }
        let Some(store) = self.lifecycle_store.clone() else {
            return;
        };
        let Ok(record) = store.get_workspace_session(&self.session.session_id) else {
            return;
        };
        let history = record.run_history.clone();
        let unconsumed_preclaim_present =
            crate::product::workspace_engine::compile_failure_surface::unconsumed_preclaim_key(
                &record,
            )
            .is_some();
        let manual_repairs_remaining = if unconsumed_preclaim_present {
            let b = crate::product::work_item_plan_policy::RunBudgets::default();
            b.max_manual_repairs
                .saturating_sub(history.manual_repairs_used)
        } else {
            self.author_rewrite_balance().unwrap_or(0)
        };
        let snapshot = HumanGateSnapshot {
            candidate_recovery: None,
            findings: Vec::new(),
            repeated_fingerprints: Vec::new(),
            attempts_used: history
                .repairs_used
                .saturating_add(history.manual_repairs_used),
            manual_repairs_remaining,
            trigger: HumanReason::NativeHumanRequired,
            resumable: true,
            accepted_feedback_turns: Some(0),
        };
        match store.compare_and_save_policy_route(
            &record,
            PolicyRoutePersist {
                status: record.status.clone(),
                single_candidate_phase: record.single_candidate_phase.clone(),
                run_history: record.run_history.clone(),
                scope: record.review_invocation_scope.clone(),
                gate: Some(snapshot),
                diagnostics: record.policy_diagnostics.clone(),
                repair_reservation: record.repair_reservation.clone(),
                provider_start_ledger: record.provider_start_ledger.clone(),
                author_rewrite_budget: if unconsumed_preclaim_present {
                    // 账本不清：保留原样，让 Task 3 对账能按键退款。
                    record.author_rewrite_budget.clone()
                } else {
                    carried_author_rewrite_budget(&store, &record)
                },
            },
        ) {
            Ok(saved) => {
                self.session.human_gate_snapshot = saved.human_gate_snapshot;
                self.session.run_history = saved.run_history;
                self.session.author_rewrite_budget = saved.author_rewrite_budget.clone();
            }
            Err(error) => {
                // CAS 失败＝并发推进已抢先改写该 record：保持快照缺席的既有
                // 语义（反馈按缺失拒收），不阻塞失败落门本身，仅让错误可见。
                let _ = self
                    .event_tx
                    .send(EngineEvent::Error {
                        message: format!(
                            "rebuild human gate snapshot after compile failure lost the durable race: {error}"
                        ),
                    })
                    .await;
            }
        }
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
        // D2.1b 探针：finalizer 各 checkpoint 位点（与 failpoint 同位，无
        // failpoint 注册也触发）。
        probe_work_item_plan_compile_admission(
            &tx.project_id,
            &tx.issue_id,
            &tx.plan_id,
            &format!("{checkpoint:?}"),
        );
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

    /// D2.1b：按 session 身份取 owning Plan run admission；无 lifecycle_store
    /// 的退化引擎保持原行为返回 None（无 durable 面可保护，后续路径自行
    /// 报 lifecycle_store unavailable）。
    fn acquire_work_item_plan_compile_admission(
        &self,
    ) -> Result<Option<WorkItemPlanCompileAdmission>, String> {
        let Some(lifecycle) = &self.lifecycle_store else {
            return Ok(None);
        };
        WorkItemPlanCompileAdmission::acquire(
            &lifecycle.app_paths(),
            &self.session.project_id,
            &self.session.issue_id,
            &self.session.entity_id,
        )
        .map(Some)
        .map_err(|error| work_item_plan_compile_admission_error(error, &self.session.entity_id))
    }

    pub(crate) async fn run_work_item_plan_compile(
        &mut self,
    ) -> Result<InitialPlanCompileOutcome, String> {
        // D2.1b（fix-cascade-deletion Task 3.1；REQ-DEL-06，plan §0.4）：
        // Final Compile 的最外层 run admission——任何 durable 写之前取得
        // owning Plan run lease（try-shared＋锁内墓碑复核），SC initial 与
        // legacy execute/finalizer 全提交段共享同一 admission，final 提交/
        // 失败返回才释放。外层状态机帧（enter_*）已持同 scope lease 时
        // 共享 flock 幂等可重入（等价消费，不引入锁序倒置）。
        let _admission = self.acquire_work_item_plan_compile_admission()?;
        if self.session.flow_kind == WorkItemPlanFlowKind::SingleCandidate {
            // SG3b：SC 链 typed PlanningCompileFailure 在 engine 内经
            // `last_gate_close_compile_failure` stash 直达 close/WS/REST；
            // 此 String 边界（时间线/事件与既有 legacy 测试面）只取人读
            // 渲染，不作为身份来源。
            return self
                .run_single_candidate_initial_plan_compile()
                .await
                .map_err(|failure| failure.render());
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
                }).await?;
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

        // D2.1b（plan §0.4）：恢复动作的最外层 run admission——删除已赢
        // （purging/deleted）时先于任何 durable 读/写稳定拒绝：Continue/
        // HumanTriage 不写 transaction/WorkItems/source/provenance，
        // AbortAndRollback 不复活已删内容；resume 运行中（同 scope run
        // lease 在持）删除独占立即 Busy。
        let _compile_admission = self.acquire_work_item_plan_compile_admission()?;
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
                // 批 2 P3：恢复门成功关闭（状态迁移已全部落定）后清对应
                // plan_question 事实；清失败上抛（事实保留可见，不假成功）。
                self.resolve_automation_compile_recovery_fact(&tx.compile_id)?;
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
                // 批 2 P3：恢复门成功关闭（状态迁移已全部落定）后清对应
                // plan_question 事实；清失败上抛（事实保留可见，不假成功）。
                self.resolve_automation_compile_recovery_fact(&tx.compile_id)?;
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
                // 批 2 P3：恢复门成功关闭（状态迁移已全部落定）后清对应
                // plan_question 事实；清失败上抛（事实保留可见，不假成功）。
                self.resolve_automation_compile_recovery_fact(&tx.compile_id)?;
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
