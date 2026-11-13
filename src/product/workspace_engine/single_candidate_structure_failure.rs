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
use crate::product::workspace_engine::prompts::REVISION_TEACHING;
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

/// 作者阶段结构拒收的**教学重驱修复指令**（给 author 的可执行 prompt）。
///
/// 与 [`author_gate_summary`]（面 owner）同源派生，但收件人是 author：
/// - 环在哪（durable `message` 的人话段，与终编译同一共享环模板）；
/// - 两个合法修法**各带后果**（witness 完整时，`cycle_owner_options` 同源；
///   witness 不完整时退化为 `rewrite_guidance` 的通用消环要点，不虚构位次）；
/// - 非环结构拒收给通用结构修正指令（不套用「环」措辞——不虚构原因）；
/// - 保护项（不得删除真实业务数据流、不得改仓序）；
/// - 此处只构造指令面；legacy 摘要正文和完整 prompt 分别在尾部追加
///   同一 [`REVISION_TEACHING`]，不得改变既有摘要输入字节。
fn render_author_structure_repair_instructions(failure: &PlanningCompileFailure) -> String {
    let durable = durable_failure(failure);
    let mut prompt = format!(
        "候选没有通过候选结构门（稳定码 {code}），请立即修复后重新输出完整候选。\n\
         失败原因：{message}",
        code = durable.code,
        message = durable.message,
    );
    if durable.message.contains("依赖环") {
        match compile_failure_surface::cycle_witness_facts(&durable.message) {
            Some(facts) => {
                prompt.push_str("\n两个合法修法（业务取舍，二选一，各自后果如下）：");
                for (index, option) in compile_failure_surface::cycle_owner_options(&facts)
                    .iter()
                    .enumerate()
                {
                    prompt.push_str(&format!(
                        "\n{}. {}——后果：{}",
                        index + 1,
                        option.label,
                        option.consequence
                    ));
                }
            }
            None => {
                prompt.push_str("\n消环要点（环 witness 不完整，不给位次与选项，不虚构）：");
                for line in compile_failure_surface::rewrite_guidance(&durable) {
                    prompt.push_str(&format!("\n- {line}"));
                }
            }
        }
    } else {
        prompt.push_str(
            "\n请按上面的结构校验原因修正 Work Item 的依赖与契约声明\
             （provider_logical_work_item_id 必须指向本候选内真实存在的 work item；\
             不得删除该真实 Inputs 引用来消除缺口）。",
        );
    }
    prompt.push_str(
        "\n保护项：不得删除真实业务数据流（canonical Inputs 引用）来消除校验失败；\
         不得改动仓序本身（仓序由 owner 在 design 侧决定，plan 内只能调整归属与依赖）。",
    );
    prompt
}

/// 结构臂 instruction_digest 的永久输入；逐字保留 source cutover 前的指令正文。
/// 不加入 typed 全因或 source 元数据，避免同一失败因 prompt 扩充而改变指令摘要。
pub(crate) fn legacy_author_structure_repair_body(failure: &PlanningCompileFailure) -> String {
    let mut body = render_author_structure_repair_instructions(failure);
    body.push_str(REVISION_TEACHING);
    body
}

/// 事实、typed 全集、完整失败 source 与最终输出纪律；预算只由最终 facade 裁决。
pub(crate) fn author_structure_repair_prompt(
    failure: &PlanningCompileFailure,
    structure_failures: &PlanningFailureSet,
    previous_source: &str,
    source: &crate::product::workspace_engine::ScTeachingSourceMetadata<'_>,
) -> Result<String, crate::product::workspace_engine::ScTeachingPromptError> {
    let source_block =
        crate::product::workspace_engine::render_sc_previous_source_block(previous_source, source)?;
    let mut prompt = render_author_structure_repair_instructions(failure);
    prompt.push_str("\n结构失败事实（按原失败集顺序逐项保留）：\n");
    for structure_failure in &structure_failures.failures {
        // PlanningFailure 只包含可序列化的字符串、整数、枚举、序列及字符串键 map。
        // 不投影成首个环、不去重；证据与来源字段也随每条 typed 记录保留。
        prompt.push_str(
            &serde_json::to_string(structure_failure)
                .expect("PlanningFailure contains only JSON-serializable facts"),
        );
        prompt.push('\n');
    }
    prompt.push_str(&source_block);
    prompt.push('\n');
    prompt.push_str(REVISION_TEACHING);
    Ok(prompt)
}

/// typed 失败 → durable 失败事实（与 `candidate_compile_status` 同一派生，
/// 只做投影、不落盘）。摘要与门卡因此与 durable 记录逐字一致。
fn durable_failure(failure: &PlanningCompileFailure) -> CandidateCompileFailure {
    compile_failure_surface::candidate_compile_status(None, failure, String::new(), None)
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
    /// append-only 尝试流水（REQ-WSC-18/25，**每次尝试都进流水**）→
    /// 恰一次教学重驱（`allow_structure_rerun=true`）或既有路由（人工门）。
    ///
    /// **候选零提升**：本路径不改写会话 current refs、不落机械报告、不开评审，
    /// 只把「这次候选被结构门拒收」变成可观测事实并交出一条下一步。
    ///
    /// - 调用方传入 `allow_structure_rerun=true`（Task 6 起恒 true：重驱资格
    ///   由调用方消费同一耐久教学窗口的 claim 结果决定）：返回
    ///   [`SingleCandidateAuthorCompletion::Rejected`]，修复
    ///   指令交还 runner 经 provider 通道重驱作者一次——**不开人工门**。
    ///   重驱走 runner 通道而非引擎 prerevision 路由（候选 refs 未提升，
    ///   重写通道不可达）：不虚记自动重写预算，durable `auto_rewrite` 不留
    ///   running（未委托即对账，REQ-WSC-25）。
    /// - `allow_structure_rerun=false`：保持既有落门收口，返回
    ///   [`SingleCandidateAuthorCompletion::Routed`]：调用方不得再按硬失败
    ///   处理（现场缺口：owner 只看到「生成失败」，没有修法也没有流水）。
    pub(crate) async fn route_author_structure_gate_failure(
        &mut self,
        lifecycle: &crate::product::lifecycle_store::LifecycleStore,
        mut failure: PlanningCompileFailure,
        structure_failures: PlanningFailureSet,
        source_ref: &str,
        ir_ref: &str,
        allow_structure_rerun: bool,
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
        // 必有一次真实尝试记录（重驱轮同样落一行——通过率读数靠它）。
        self.record_candidate_compile_failure(&failure);
        if allow_structure_rerun {
            // 恰一次教学重驱槽位仍空闲：不开人工门、不硬失败，把 **sealed 的
            // 原失败身份 ＋ 原 typed 结构失败集** 交还 runner 重驱作者一次。
            // 修复指令正文由 runner 用 `author_structure_repair_prompt` 携完整失败 source
            // 生成（引擎侧不再提前造 prompt——避免二次渲染与文本反解析）。
            // 重驱是 runner 的 provider 通道，不是引擎 prerevision 路由——此处同样
            // 不虚记自动重写预算，且 durable `auto_rewrite` 不得留 running
            // （未委托即对账）。
            self.reconcile_auto_rewrite_not_delegated().await;
            return SingleCandidateAuthorCompletion::Rejected {
                failure,
                structure_failures,
            };
        }
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
