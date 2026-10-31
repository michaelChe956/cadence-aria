//! SG3a.1 候选结构门（系统门）**作者阶段**拒收的 typed 失败（REQ-SG-02/03、
//! REQ-WSC-18/21/25 同族）。
//!
//! 现场（2026-10-31 真机）：author 产出候选后被候选结构门拒收
//!（`single-candidate structure gate rejected: dependency_cycle[...]`），
//! 作者阶段把它当 `Err(String)` 硬失败——owner 只看到「生成失败」，没有
//! 修法、没有自动重写、没有尝试流水。Final Compile 侧的同一个结构门走的是
//! typed 失败路（durable 诊断 + 尝试流水 + 自动重写/人工门），本模块把作者
//! 阶段对齐到同一条路：
//!
//! 1. **事实单源**：有效图装配与 `assess_single_candidate_structure` 共用
//!    [`project_single_candidate_structure`]，环模板与结构指纹与终编译共用
//!    `cycle_diagnostic`——同一依赖环在两处得到同一人话段与同一结构摘要；
//! 2. **仓序位次**：逆仓序那一跳的两位次由**真实有效边**给出（该端点对的
//!    反向仓序自动边携带 `order_indices = [前仓索引, 后仓索引]`），拿不到就
//!    只说真实环链、不给位次、不给选项（不虚构）；
//! 3. **稳定码**：一律用 [`STRUCTURE_STABLE_CODE`]（结构拒收码，Gate 路由）：
//!    owner 文案不指向没跑过的终编译、失败码统计能区分阶段、且不虚记自动
//!    重写预算（作者阶段候选 refs 未提升，重写通道根本不可达）。
//!
//! 本模块只做纯事实构造 + 引擎侧收口，不新增 provider attempt 通道：重写委托
//! 与人工门仍走 `route_compile_failure_to_rewrite` / `ensure_human_gate_snapshot_
//! after_compile_failure` 既有两个出口。

use crate::product::logical_codebase::EnrollmentTarget;
use crate::product::models::{CandidateCompileFailure, WorkspaceSessionRecord};
use crate::product::planning_failure::{
    PlanningCycleWitnessHop, PlanningEdgeSource, PlanningFailureKind, PlanningFailureSet,
};
use crate::product::work_item_plan_compiler::PlanCandidateIr;
use crate::product::workspace_engine::compile_failure_surface;
use crate::product::workspace_engine::draft_batch::compile_support::{
    PlanningStructureContext, PlanningStructureReport,
};
use crate::product::workspace_engine::draft_batch::cycle_diagnostic;
use crate::product::workspace_engine::plan_projection;
use crate::product::workspace_engine::single_candidate::SingleCandidateAuthorCompletion;
use crate::product::workspace_engine::types::PlanningCompileFailure;

/// 作者阶段候选结构门拒收的稳定码：**一律**用结构拒收码，不复用终编译
/// prepare 期的 `final_compile_prepare_failed`。
///
/// 理由（2026-10-31 真机复盘）：
/// - **owner 文案要对得上事实**：作者阶段还没跑到终编译，复用该码会让 owner
///   看到「最终编译前的准备校验没有通过」（指向没跑过的步骤）。本码的文案是
///   「候选没有通过结构校验」。
/// - **失败码统计要能区分阶段**：通过率读数按失败码 Top N 聚合，作者阶段与
///   终编译阶段混用同一码 → 统计无法分辨失败发生在哪一段。
/// - **不白扣重写预算**：作者阶段候选 refs 尚未提升，自动重写通道不可达
///   （强行进入只会硬失败），因此**没有发生重写**；终编译 prepare 码属
///   `is_auto_rewrite_code` 族，会把 `auto_rewrites_used` 记 +1（虚记预算）。
///   本码走 Gate 路由（`is_auto_rewrite_code` 不收），预算不扣。
///
/// 修复指引不丢：`rewrite_guidance` 的准入是 `is_candidate_invalid_code`
/// （含本码），环事实照常由 message 承载（`cycle_witness_facts` 同一模板解析）。
pub(crate) const STRUCTURE_STABLE_CODE: &str = "final_compile_structure_rejected";

/// 作者阶段候选结构门拒收 → typed 编译失败（**纯函数**，零 I/O）。
///
/// `set` 是 `StructuralAssessment::Rejected` 携带的真实失败集合；有效图在此
/// 按同一装配重投影，只为取仓序位次与结构指纹（同一次纯计算，结论必然一致）。
pub(crate) fn author_structure_gate_failure(
    ir: &PlanCandidateIr,
    context: &PlanningStructureContext,
    set: &PlanningFailureSet,
) -> PlanningCompileFailure {
    let report = plan_projection::project_single_candidate_structure(ir, context);
    let cycle = dependency_cycle_witness(set);
    let cause = match cycle {
        Some(witness) => cycle_cause(ir, context, &report, &witness),
        None => structural_cause(set),
    };
    let mut failure = PlanningCompileFailure::operational(STRUCTURE_STABLE_CODE, cause);
    // 候选身份（本次 source/IR）随失败上行：durable 事实得以对账到具体候选，
    // 但 **不** 提升会话 current refs（zero promotion 语义零变化）。
    failure.source_revision_ref = Some(context.source_revision_ref.clone());
    failure.plan_candidate_ir_ref = Some(context.plan_candidate_ir_ref.clone());
    failure.context_digest = Some(context.context_digest.clone());
    failure
}

/// 面 owner 的人工门摘要（durable 事实单源派生）：人话原因 + 两个带后果的
/// 修法（witness 完整时）/ 通用下一步（witness 不完整时不虚构选项）。
pub(crate) fn author_gate_summary(failure: &PlanningCompileFailure) -> String {
    let durable = durable_failure(failure);
    // 环拒收回放 witness 事实（两个修法带后果）；非环结构缺口不得套用「环」
    // 措辞（不虚构原因），只给稳定事实 + 可执行下一步。
    let hint = if durable.message.contains("依赖环") {
        compile_failure_surface::owner_hint(&durable)
    } else {
        "下一步二选一：按上面的结构校验原因修正 Work Item 的依赖与契约声明后重新生成；\
         或终止本计划（平台不会替你重试同一个坏候选）。"
            .to_string()
    };
    format!(
        "候选没有通过候选结构门，已停在人工门等你决定（不会静默重试）：{message}\n{hint}",
        message = durable.message,
    )
}

/// typed 失败 → durable 失败事实（与 `candidate_compile_status` 同一派生，
/// 只做投影、不落盘）。摘要与门卡因此与 durable 记录逐字一致。
fn durable_failure(failure: &PlanningCompileFailure) -> CandidateCompileFailure {
    compile_failure_surface::candidate_compile_status(None, failure, String::new())
        .0
        .failure
        .expect("candidate_compile_status 恒为失败分支产出 Some(failure)")
}

/// 失败集合里的首个依赖环 witness（空 witness 不算事实，不构造环文案）。
fn dependency_cycle_witness(
    failure_set: &PlanningFailureSet,
) -> Option<Vec<PlanningCycleWitnessHop>> {
    failure_set
        .failures
        .iter()
        .find_map(|failure| match &failure.kind {
            PlanningFailureKind::DependencyCycle { witness } if !witness.is_empty() => {
                Some(witness.clone())
            }
            _ => None,
        })
}

/// 环 cause：共享模板（`cycle_diagnostic::cycle_message`）+ 共享结构指纹装配
/// （`cycle_diagnostic::dependency_cycle_cause`）。下游
/// `compile_failure_surface::cycle_witness_facts` 按同一模板锚点解析出
/// consumer/provider/两位次，`decide_route` 据此判定「同一环重复」。
fn cycle_cause(
    ir: &PlanCandidateIr,
    context: &PlanningStructureContext,
    report: &PlanningStructureReport,
    witness: &[PlanningCycleWitnessHop],
) -> String {
    let display = |id: &str| work_item_display(ir, id);
    // 环链：首跳的 provider + 各跳的 consumer（末跳回到起点，与终编译同形）。
    let mut labels = vec![display(&witness[0].from)];
    labels.extend(witness.iter().map(|hop| display(&hop.to)));
    let reason = match offending_hop(report, witness) {
        Some((provider, consumer, provider_order, consumer_order)) => {
            cycle_diagnostic::cycle_message(
                &labels,
                &display(&consumer),
                &display(&provider),
                provider_order,
                consumer_order,
            )
        }
        None => cycle_diagnostic::cycle_closure_message(&labels),
    };
    // 结构指纹：成员（序号@逻辑仓）+ 真实有向边，跨尝试稳定。
    let mut members: Vec<String> = Vec::with_capacity(witness.len() * 2);
    let mut edges: Vec<(String, String)> = Vec::with_capacity(witness.len());
    for hop in witness {
        let provider = node_key(ir, context, &hop.from);
        let consumer = node_key(ir, context, &hop.to);
        members.push(provider.clone());
        members.push(consumer.clone());
        edges.push((provider, consumer));
    }
    let fingerprint = cycle_diagnostic::cycle_structure_fingerprint_from(&members, &edges);
    let mut cycle_ids: Vec<String> = Vec::with_capacity(witness.len() + 1);
    for hop in witness {
        if !cycle_ids.contains(&hop.from) {
            cycle_ids.push(hop.from.clone());
        }
        if !cycle_ids.contains(&hop.to) {
            cycle_ids.push(hop.to.clone());
        }
    }
    cycle_diagnostic::dependency_cycle_cause(reason, fingerprint, &cycle_ids)
}

/// 逆仓序那一跳：witness 中首个「该端点对的**反向**仓序自动边在场」的跳——
/// 反向自动边的 `order_indices` 直接给出 [consumer 仓索引, provider 仓索引]
/// （自动边方向恒为前仓→后仓），因此 provider 位次必然 > consumer 位次。
///
/// 任一跳取不到位次（同仓互等 / 缺成员归属）→ `None`：只说真实环链，不编造
/// 位次与修法选项（与 `cycle_witness_facts` 的降级语义同源）。
fn offending_hop(
    report: &PlanningStructureReport,
    witness: &[PlanningCycleWitnessHop],
) -> Option<(String, String, usize, usize)> {
    witness.iter().find_map(|hop| {
        let reverse = report.effective_edges.iter().find(|edge| {
            edge.from == hop.to
                && edge.to == hop.from
                && edge.sources.contains(&PlanningEdgeSource::ChangeOrder)
                && edge.order_indices.len() == 2
        })?;
        let consumer_index = reverse.order_indices[0];
        let provider_index = reverse.order_indices[1];
        (provider_index > consumer_index).then_some((
            hop.from.clone(),
            hop.to.clone(),
            provider_index + 1,
            consumer_index + 1,
        ))
    })
}

/// 结构指纹里的单点表达：`序号@逻辑仓`——序号取 id 末段（`WI-003` / 编译运行
/// id `work_item_<compile_id>_003` 都稳定），仓取逻辑成员（与终编译
/// `target_repository_id` 同口径），不可得时回落该 item 的物理 target。
fn node_key(ir: &PlanCandidateIr, context: &PlanningStructureContext, id: &str) -> String {
    let ordinal = id.rsplit('_').next().unwrap_or(id);
    let repository = ir
        .items
        .iter()
        .find(|item| item.contract.identity.logical_work_item_id == id)
        .map(|item| {
            match item.contract.intent_contract.as_ref().map(|intent| &intent.target) {
                Some(EnrollmentTarget::LogicalCodebase {
                    logical_repository_id,
                    ..
                }) => logical_repository_id.0.to_string(),
                _ => match crate::product::workspace_engine::member_attribution::reverse_member_logical(
                    &item.target_repository_id,
                    &context.physical_by_logical,
                ) {
                    crate::product::workspace_engine::member_attribution::MemberReverse::Unique(
                        logical,
                    ) => logical.0.to_string(),
                    _ => item.target_repository_id.clone(),
                },
            }
        })
        .unwrap_or_default();
    format!("{ordinal}@{repository}")
}

/// 非环结构拒收的 cause：与 `plan_projection::structural_assessment_stop_reason`
/// 的 Rejected 渲染同形（稳定码 + witness 路径），既有 typed evidence 不丢。
fn structural_cause(failure_set: &PlanningFailureSet) -> String {
    let rendered = failure_set
        .failures
        .iter()
        .map(|failure| {
            format!(
                "{}[{}]",
                failure.kind.stable_code(),
                failure.evidence.affected_ids.join("->")
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!("single-candidate structure gate rejected: {rendered}")
}

/// Work Item 呈现名：标题优先（owner/author 都认标题），空标题回落 id。
fn work_item_display(ir: &PlanCandidateIr, id: &str) -> String {
    ir.items
        .iter()
        .find(|item| item.contract.identity.logical_work_item_id == id)
        .map(|item| item.contract.identity.title.trim())
        .filter(|title| !title.is_empty())
        .unwrap_or(id)
        .to_string()
}

impl crate::product::workspace_engine::WorkspaceEngine {
    /// 作者阶段候选结构门拒收的 typed 收口：seal → durable 候选编译事实 +
    /// append-only 尝试流水（REQ-WSC-18/25）→ 既有路由（自动重写 / 人工门）。
    ///
    /// **候选零提升**：本路径不改写会话 current refs、不落机械报告、不开评审，
    /// 只把「这次候选被结构门拒收」变成可观测事实并交出一条下一步。
    ///
    /// 返回 [`SingleCandidateAuthorCompletion::Routed`]：调用方不得再按硬失败
    /// 处理（现场缺口：owner 只看到「生成失败」，没有修法也没有流水）。
    pub(crate) async fn route_author_structure_gate_failure(
        &mut self,
        lifecycle: &crate::product::lifecycle_store::LifecycleStore,
        mut failure: PlanningCompileFailure,
        source_ref: &str,
        ir_ref: &str,
    ) -> SingleCandidateAuthorCompletion {
        let session: Option<WorkspaceSessionRecord> = lifecycle
            .get_workspace_session(&self.session.session_id)
            .ok();
        let approval_attempt_id = session.and_then(|record| record.approval_attempt_id);
        failure.seal_with_identity(
            &self.session.session_id,
            approval_attempt_id,
            Some(source_ref.to_string()),
            Some(ir_ref.to_string()),
            None,
        );
        let summary = author_gate_summary(&failure);
        // durable 诊断 + 尝试流水：只要走到这里，`candidate-attempts.jsonl`
        // 必有一次真实尝试记录（此前该缺口完全空白）。
        self.record_candidate_compile_failure(&failure);
        // 与 Final Compile 同一条路由出口。作者阶段的候选 refs **尚未提升**，
        // `current_candidate_compile_status` 按候选 ref 绑定（`belongs_to_
        // candidate` 要求两侧都在场且相等）——绑定不成立时重写通道不可达
        // （review invocation scope 需要 durable 候选 ref，强行进入只会硬失败），
        // 此时由本处按既有落门语义收口。
        let routed = self.current_candidate_compile_status().is_some()
            && self.route_compile_failure_to_rewrite().await;
        if !routed {
            // 未真正委托 author：durable auto_rewrite 不得继续谎报 running
            //（REQ-WSC-25 对账语义）。
            self.reconcile_auto_rewrite_not_delegated().await;
            self.ensure_human_gate_snapshot_after_compile_failure()
                .await;
            self.enter_human_confirm(Some(summary)).await;
        }
        SingleCandidateAuthorCompletion::Routed
    }
}
