//! REQ-WSC-18（owner 2026-10-31 真机反馈「点了之后会报错，但是我看不到报错
//! 内容」）：把 SC Final Compile 的 typed 失败转成 owner 可读的门卡事实。
//!
//! 现场两个缺口同源：
//! 1. 失败原因只落在 timeline node 的 `summary`（`Final Compile 失败：[code]
//!    ...`）与内存 stash，`policy_diagnostics` 为空、门快照 findings 为空——
//!    owner 在门卡上读不到任何报错内容；
//! 2. 原始 cause 是机器话（`dependency_cycle: ... work_item_<hash>_003, ...`），
//!    即使读到也看不出「谁把谁写成了 provider、为什么成环」。
//!
//! 本模块有两条出口，同一份 typed 失败单源派生：
//! - [`candidate_compile_status`]：durable 记录（`detail` 为完整诊断原文，
//!   不截断）；
//! - [`owner_message`]：给 owner 的一句人话（`message`）。
//!
//! 人话只从真实诊断事实推导（环上 Work Item 标题/仓序、稳定码），不编造
//! 未出现过的原因；无法推导时退化为「稳定码 + 原文」，绝不吞掉原因。

use crate::product::models::{CandidateCompileFailure, CandidateCompileStatus};
use crate::product::workspace_engine::types::{
    EngineEvent, PlanningCompileFailure, PlanningCompileFailureKind,
};
use crate::web::workspace_ws_types::OwnerOption;

/// 编译失败 cause 里「环上 Work Item id」行的起点标记。producer
/// （`draft_batch::compile_support` 的 change_order 环检测）与本模块共用，
/// 保证「人话段 / 机器 id 段」的切分只有一个来源。
pub(crate) const CYCLE_IDS_MARKER: &str = "\n环上 Work Item id: ";

/// 编译失败 cause 里「环结构指纹」行的起点标记（producer 与本模块共用，
/// REQ-WSC-22：同一环的重复判定只看结构摘要，不看文案）。
pub(crate) const CYCLE_FINGERPRINT_MARKER: &str = "\n环结构指纹: ";

/// A3 / REQ-WSC-25：**平台合成**的编译失败 finding 的单源标识。
///
/// 由 [`auto_rewrite_verdict`] 单点生产（finding `message` 前缀）；策略层
///（`review/routing.rs` / `review/routing_scope.rs`）据此且仅据此前缀识别
/// 「平台编译失败轮」——模型自己产出的 finding 不带此前缀，照常受
/// Verification 相位 new-findings 守卫约束（controller 2026-10-31 批复的
/// 红线：不得按 kind/severity/轮次豁免）。
pub(crate) const PLATFORM_COMPILE_FAILURE_FINDING_PREFIX: &str = "candidate_compile_failure:";

/// REQ-WG-05：依赖环 owner 面呈现所需的 typed 事实——逆仓序的那一跳。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CycleWitnessFacts {
    /// 消费方（把 provider 写进 Inputs 的那个 Work Item，展示名：标题优先）。
    pub consumer: String,
    /// 被写成 provider 的 Work Item（展示名）。
    pub provider: String,
    /// provider 所在仓在 change_order 的位次（1 起）。
    pub provider_order: usize,
    /// consumer 所在仓在 change_order 的位次（1 起）。
    pub consumer_order: usize,
}

/// REQ-WG-05：从 durable `message`（producer `cycle_diagnostic::cycle_reason`
/// offending 分支的固定模板）提取逆仓序一跳的 typed 事实。
///
/// 锚点与 producer 模板逐字对应（该模板由 `cycle_diagnostic` 的测试钉住）；
/// **任何**一跳解析失败或位次不构成逆序 → `None`（witness 不完整，不猜，
/// 也不虚构环形状/原因）。标题本身包含锚点子串时同样安全落 `None`。
pub(crate) fn cycle_witness_facts(message: &str) -> Option<CycleWitnessFacts> {
    let rest = message.split_once("其中「")?.1;
    let (consumer, rest) = rest.split_once("」把「")?;
    let (provider, rest) = rest.split_once("」写成 provider，但「")?;
    let rest = rest.strip_prefix(format!("{provider}」所在仓是 change_order 第 ").as_str())?;
    let (provider_order, rest) = rest.split_once(" 个仓、「")?;
    let rest = rest.strip_prefix(format!("{consumer}」所在仓是第 ").as_str())?;
    let (consumer_order, _) = rest.split_once(" 个仓")?;
    let provider_order: usize = provider_order.parse().ok()?;
    let consumer_order: usize = consumer_order.parse().ok()?;
    (provider_order > consumer_order).then_some(CycleWitnessFacts {
        consumer: consumer.to_string(),
        provider: provider.to_string(),
        provider_order,
        consumer_order,
    })
}

/// REQ-WG-05 / REQ-CG-09：由 witness 事实生成的 owner 互斥选项（两个合法
/// 修法各带后果）。不声称唯一合法托管仓（写范围约束可能使多解并存），
/// 不含「改 design」选项（仓序本身可能完全正常，不得归因 design）。
pub(crate) fn cycle_owner_options(facts: &CycleWitnessFacts) -> Vec<OwnerOption> {
    vec![
        OwnerOption {
            label: format!(
                "去掉「{}」对「{}」的跨仓消费",
                facts.consumer, facts.provider
            ),
            consequence: format!(
                "环立即消除；「{consumer}」不再消费第 {provider_order} 个仓「{provider}」的产出，\
                 它的验收/集成范围相应收窄（不再覆盖「{provider}」负责的那部分改动）",
                consumer = facts.consumer,
                provider = facts.provider,
                provider_order = facts.provider_order,
            ),
        },
        OwnerOption {
            label: format!(
                "把「{}」连同写范围与测试整体迁到第 {} 个仓或仓序更后",
                facts.consumer, facts.provider_order
            ),
            consequence: format!(
                "环同样消除，跨仓验收保持在后仓执行；代价是「{consumer}」的写范围路径与\
                 执行命令都要改写，重写量最大（两个方向都合法，平台不预设唯一解）",
                consumer = facts.consumer,
            ),
        },
    ]
}

/// REQ-WSC-21/22：自动重写的路由裁定（纯函数结果，便于逐条钉钉）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CandidateCompileRoute {
    /// 非候选自身无效类失败：不开自动重写，按既有失败路径（人工门）。
    Gate,
    /// 首次或**新结构**失败：平台应自动让 author 重写（`remaining` 为本轮之后的剩余）。
    AutoRewrite { remaining: u32, repeated: u32 },
    /// 同一依赖环重复，或自动重写预算用尽：停预算、升级提示、交人工门。
    Escalate { repeated: u32, remaining: u32 },
}

/// REQ-WSC-21：**author 重写次数预算**（owner 2026-10-31 第 3 条：「还可自动
/// 修复 3 次」＝**还能让 author 重写几次**）。
///
/// 这是同一个预算池，两类重写都扣它：
/// - **平台自动重写**（本 change 的编译类失败自动返修）；
/// - **owner 反馈触发的重写**（门内反馈 turn 预留时 `manual_repairs_remaining`
///   原子减 1，见 `lifecycle_store/human_gate.rs`）。
///
/// 另一个池 `RunBudgets::max_repairs`（默认 1）是**机械返修**池（指纹闸门那类
/// 不重跑 author 的机械 repair），与本条无关，**不得**拿来当自动重写预算。
pub(crate) fn max_auto_rewrites() -> u32 {
    crate::product::work_item_plan_policy::RunBudgets::default().max_manual_repairs
}

/// REQ-WSC-21/22：路由裁定。
///
/// 判据全部是结构事实：
/// - 稳定码必须属候选自身无效族（否则 Gate）；
/// - 依赖环的重复只看**结构指纹**（成员集合 + 边集合规范化摘要），作者换
///   措辞/改标题/换行都不会被判成新环（不白耗预算）；
/// - 预算 = 统一权威余额（工作包 1.5）：`author_budget_available` 由调用方
///   从 durable 记录读侧组装；诊断计数 `auto_rewrites_used` 不再喂余额。
pub(crate) fn decide_route(
    code: &str,
    fingerprint: Option<&str>,
    previous: Option<&CandidateCompileStatus>,
    author_budget_available: Option<u32>,
) -> CandidateCompileRoute {
    if !is_auto_rewrite_code(code) {
        return CandidateCompileRoute::Gate;
    }
    let repeated = match (
        previous.and_then(|status| status.cycle_fingerprint.as_deref()),
        fingerprint,
    ) {
        (Some(previous_fingerprint), Some(current)) if previous_fingerprint == current => previous
            .and_then(|status| status.repeat_count)
            .unwrap_or(1)
            .saturating_add(1),
        _ => 1,
    };
    // None = 余额未知 ⇒ 保守交门（remaining = 0），不臆造满额。
    let remaining = author_budget_available.unwrap_or(0);
    // 同一环重复＝plan 层修不了：提前停，不再白耗剩余预算。
    if fingerprint.is_some() && repeated >= 2 {
        return CandidateCompileRoute::Escalate {
            repeated,
            remaining,
        };
    }
    if remaining == 0 {
        return CandidateCompileRoute::Escalate {
            repeated,
            remaining: 0,
        };
    }
    CandidateCompileRoute::AutoRewrite {
        remaining: remaining - 1,
        repeated,
    }
}

/// REQ-WSC-21：把一次候选编译失败翻译成**机械返修 verdict**——这是既有
/// F5-A 通道的入口形状（`route_single_candidate_contract_prerevision` 消费
/// 同一形状）：`Revise` + `RequiresRevision` + 一条 `MustFix` +
/// `ContractGap`/`Repairable` finding，策略路由据此走
/// `TriggerAggregateRepair` → 真正重驱 SC author（不新造通道）。
///
/// finding `message` 只由**结构**事实构成（稳定码 + 环结构指纹，且带
/// [`PLATFORM_COMPILE_FAILURE_FINDING_PREFIX`] 前缀——策略层据此识别平台
/// 编译失败轮），作者换措辞不会让同一环被当成新事实（REQ-WSC-22）；
/// `required_action` 是给 author 的重写要点（**不要求改 design**）。
///
/// REQ-WG-05 / REQ-CG-09：witness 完整时，finding 同时携带由 typed 事实
/// 生成的 `owner_question` 与 2 个带后果的 `owner_options`（一个 owner 能
/// 懂的问题 + 互斥选项）；witness 不完整时两者保持 `None`（不虚构）。
pub(crate) fn auto_rewrite_verdict(
    failure: &CandidateCompileFailure,
) -> crate::web::workspace_ws_types::ReviewVerdict {
    use crate::product::work_item_plan_policy::{FindingClassHint, ReviewFindingCategory};
    use crate::web::workspace_ws_types::{
        ReviewFinding, ReviewFindingSeverity, ReviewGate, ReviewVerdict, ReviewVerdictType,
    };

    let structural_identity = cycle_fingerprint_from_detail(&failure.detail)
        .unwrap_or_else(|| failure.failure_id.clone());
    let guidance = rewrite_guidance(failure);
    let witness = cycle_witness_facts(&failure.message);
    let finding = ReviewFinding {
        severity: ReviewFindingSeverity::MustFix,
        // 结构身份进 message：同环同文本 → 同指纹 → 复用既有重复指纹闸门。
        message: format!(
            "candidate_compile_failure:{}:{}（候选依赖图仍需消除该环）",
            failure.code, structural_identity
        ),
        evidence: format!(
            "final compile rejected the candidate: {code}",
            code = failure.code
        ),
        required_action: if guidance.is_empty() {
            format!("按最终编译失败原因修正候选：{}", failure.message)
        } else {
            format!("按以下要点修正 plan 的依赖声明：{}", guidance.join("；"))
        },
        category: Some(ReviewFindingCategory::ContractGap),
        class_hint: Some(FindingClassHint::Repairable),
        contract_field: None,
        machine_repair: None,
        owner_question: witness.as_ref().map(|facts| {
            format!(
                "「{consumer}」托管在仓序第 {consumer_order} 个仓，却要消费第 {provider_order} \
                 个仓「{provider}」的产出（与仓序自动边成环，在仓序规则下不可能成立）。\
                 两个修法选哪个？",
                consumer = facts.consumer,
                consumer_order = facts.consumer_order,
                provider = facts.provider,
                provider_order = facts.provider_order,
            )
        }),
        owner_options: witness.as_ref().map(cycle_owner_options),
        disposition: None,
        target_work_item_id: None,
        finding_key: None,
    };
    let mut comments = format!(
        "最终编译拒绝了当前候选（{code}）：{message}\n自动返修：按 required_action 修正后重新输出完整 plan。\n",
        code = failure.code,
        message = failure.message,
    );
    if !guidance.is_empty() {
        for point in &guidance {
            comments.push_str(&format!("- {point}\n"));
        }
    }
    ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments,
        summary: format!(
            "最终编译失败（{code}）自动返修：依赖声明不合法，按重写要点修正",
            code = failure.code
        ),
        findings: vec![finding],
        review_gate: ReviewGate::RequiresRevision,
        work_item_plan_review: None,
        structured_output_diagnostic: None,
    }
}

/// 从 durable `detail`（含 producer 的指纹行）里取结构指纹。
fn cycle_fingerprint_from_detail(detail: &str) -> Option<String> {
    let rest = detail.split_once(CYCLE_FINGERPRINT_MARKER)?.1;
    let fingerprint = rest.lines().next()?.trim();
    (!fingerprint.is_empty()).then(|| fingerprint.to_string())
}

/// 从 typed 失败里提取结构指纹（producer 已在 cause 中给出；非环失败为 None）。
pub(crate) fn cycle_fingerprint(failure: &PlanningCompileFailure) -> Option<String> {
    let cause = match &failure.kind {
        PlanningCompileFailureKind::Operational { cause, .. } => cause.as_str(),
        PlanningCompileFailureKind::Structure(_) => return None,
    };
    let rest = cause.split_once(CYCLE_FINGERPRINT_MARKER)?.1;
    let fingerprint = rest.lines().next()?.trim();
    (!fingerprint.is_empty()).then(|| fingerprint.to_string())
}

/// REQ-WSC-18：由 typed 失败派生 durable 候选编译事实（失败分支）。
///
/// `previous` 是上一次尝试的事实（同一会话），用于结构指纹重复判定与诊断
/// 计数接续；`recorded_at` 由调用方给（不在此取时钟，便于测试）；余额由
/// `author_budget_available`（统一权威余额，durable 读侧组装）决定。
pub(crate) fn candidate_compile_status(
    previous: Option<&CandidateCompileStatus>,
    failure: &PlanningCompileFailure,
    recorded_at: String,
    author_budget_available: Option<u32>,
) -> (CandidateCompileStatus, CandidateCompileRoute) {
    let code = failure.stable_code().to_string();
    let fingerprint = cycle_fingerprint(failure);
    // 诊断计数：AutoRewrite 派生时 +1（REQ-WSC-25 观测保留；不再是余额来源）。
    let fp = fingerprint.as_deref();
    let route = decide_route(&code, fp, previous, author_budget_available);
    let used = previous.map_or(0, |status| status.auto_rewrites_used);
    let (auto_rewrite, auto_rewrites_used) = match route {
        CandidateCompileRoute::Gate => (None, used),
        CandidateCompileRoute::AutoRewrite {
            remaining,
            repeated,
        } => (
            Some(crate::product::models::CandidateAutoRewrite {
                phase: crate::product::models::CandidateAutoRewritePhase::Running,
                remaining: Some(remaining),
                repeated: Some(repeated),
            }),
            used.saturating_add(1),
        ),
        CandidateCompileRoute::Escalate {
            repeated,
            remaining,
        } => (
            Some(crate::product::models::CandidateAutoRewrite {
                phase: crate::product::models::CandidateAutoRewritePhase::Stopped,
                remaining: Some(remaining),
                repeated: Some(repeated),
            }),
            used,
        ),
    };
    let repeat_count = match route {
        CandidateCompileRoute::AutoRewrite { repeated, .. }
        | CandidateCompileRoute::Escalate { repeated, .. } => Some(repeated),
        CandidateCompileRoute::Gate => None,
    };
    let status = CandidateCompileStatus {
        source_revision_ref: failure.source_revision_ref.clone(),
        plan_candidate_ir_ref: failure.plan_candidate_ir_ref.clone(),
        failure: Some(CandidateCompileFailure {
            failure_id: failure.failure_id.clone(),
            code,
            message: owner_message(failure),
            detail: detail_text(failure),
            recorded_at: recorded_at.clone(),
        }),
        cycle_fingerprint: fingerprint,
        repeat_count,
        auto_rewrites_used,
        auto_rewrite,
        recorded_at,
    };
    (status, route)
}

/// 编译成功分支：同一候选的「编译不过」事实必须被覆盖清除——否则一次瞬态
/// 失败恢复后，确认动作会被一条过期失败永久挡死（新的死路）。编译成功＝清
/// **该候选编译链上『平台自动重写』段**（诊断计数 + 预算域已计费消费）归零；
/// 门反馈段按 REQ-HGC-01/REQ-CG-02 既有语义不清零、不返还——整池回满额须
/// 显式改 REQ-HGC-01/REQ-CG-02，不在本 change（未采用口径）。
pub(crate) fn cleared_candidate_compile_status(
    source_revision_ref: Option<String>,
    plan_candidate_ir_ref: Option<String>,
    recorded_at: String,
) -> CandidateCompileStatus {
    CandidateCompileStatus {
        source_revision_ref,
        plan_candidate_ir_ref,
        failure: None,
        cycle_fingerprint: None,
        repeat_count: None,
        // 编译成功＝清本编译链『平台自动重写』段诊断计数；门反馈段不清、不返还。
        auto_rewrites_used: 0,
        auto_rewrite: None,
        recorded_at,
    }
}
/// 完整诊断原文：失败身份 + 运行身份 + typed 渲染（逐字，不截断、不改写）。
fn detail_text(failure: &PlanningCompileFailure) -> String {
    let mut detail = format!(
        "failure_id: {}\ncode: {}",
        failure.failure_id,
        failure.stable_code()
    );
    if let Some(approval) = failure.approval_attempt_id.as_deref() {
        detail.push_str(&format!("\napproval_attempt_id: {approval}"));
    }
    if let Some(run_id) = failure.compile_run_id.as_deref() {
        detail.push_str(&format!("\ncompile_run_id: {run_id}"));
    }
    if let Some(digest) = failure.context_digest.as_deref() {
        detail.push_str(&format!("\ncontext_digest: {digest}"));
    }
    detail.push('\n');
    detail.push_str(&failure.render());
    detail
}
/// 面向 owner 的一句人话（完整、不截断）。
pub(crate) fn owner_message(failure: &PlanningCompileFailure) -> String {
    match &failure.kind {
        PlanningCompileFailureKind::Operational { stable_code, cause } => {
            operational_owner_message(stable_code, cause)
        }
        PlanningCompileFailureKind::Structure(_) => format!(
            "候选没有通过结构校验：{}。请按诊断原文修正 Work Item 的依赖与契约声明。",
            failure.render()
        ),
    }
}
fn operational_owner_message(stable_code: &str, cause: &str) -> String {
    let cause = cause.trim();
    // 依赖环：producer 已经把真实环事实写成人话（含「请怎么做」），
    // 机器 id 段单独留在 detail，不进 message。
    if let Some(human) = cycle_sentence(cause) {
        return human;
    }
    let prefix = operational_code_label(stable_code);
    if cause.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}：{cause}")
    }
}
/// 从 cause 中取出依赖环人话段（`dependency_cycle: <人话>` 之后、机器 id
/// 段之前）。非环诊断返回 None。
fn cycle_sentence(cause: &str) -> Option<String> {
    let rest = cause.split_once("dependency_cycle:")?.1;
    let human = match rest.split_once(CYCLE_IDS_MARKER) {
        Some((human, _ids)) => human,
        None => rest,
    };
    let human = human.trim();
    (!human.is_empty()).then(|| human.to_string())
}
/// 稳定码 → 人话前缀（未知码不编造原因，只给通用前缀；码本身在 `code`
/// 与 `detail` 里，不丢信息）。
fn operational_code_label(stable_code: &str) -> &'static str {
    match stable_code {
        "final_compile_prepare_failed" => "最终编译前的准备校验没有通过",
        "final_compile_execute_failed" => "最终编译执行失败",
        "final_compile_finalize_failed" => "最终编译已执行，但定稿落盘失败",
        "single_candidate_structure_context_failed" => "候选结构上下文装载失败",
        "single_candidate_structure_operational" => "候选结构校验无法判定",
        // SG3a.1（作者阶段与结构复验同码）：结构门拒收——不是「最终编译」，
        // 用能对上的措辞（owner 侧不指向没跑过的步骤）。
        "final_compile_structure_rejected" => "候选没有通过结构校验",
        "persistence_failure" => "durable 落盘失败",
        _ => "最终编译失败",
    }
}
/// 「确认当前版本」在候选已编译不过时的平台侧拒收文案（REQ-WSC-19）。
/// 三个事实一次说清：为什么不放行、同上一次编译为什么必然再失败、下一步
/// 有哪两条路。
pub(crate) fn approve_rejected_message(failure: &CandidateCompileFailure) -> String {
    format!(
        "CANDIDATE_COMPILE_FAILED: 当前候选没有通过最终编译（{code}）：{message}\n\
         确认当前版本会原样重跑同一次编译并再次失败，因此平台不执行本次确认，门保持打开。\n\
         failure_id: {failure_id}\n\
         {hint}\n\
         下一步：提交一条针对该候选的反馈让 author 重新生成候选（或终止本计划）。",
        code = failure.code,
        message = failure.message,
        failure_id = failure.failure_id,
        hint = owner_hint(failure),
    )
}

/// REQ-WG-05（REMOVED REQ-WSC-20）：给 owner 的醒目提示——**只**由 witness
/// 事实构成，不断言根因在 design（现场 2adc5e41 的 design 仓序本身无矛盾，
/// 错在 plan 把集成验收项托管在仓序更靠前的仓；旧文案把 owner 引向无效
/// 操作）。witness 完整时给出两个仓序位次与两个可执行修法及后果；不完整
/// 时只陈述稳定码与已知事实，不给位次、不给选项、不归因。
pub(crate) fn owner_hint(failure: &CandidateCompileFailure) -> String {
    if !is_candidate_invalid_code(&failure.code) {
        return format!(
            "这个候选无效：它没有通过最终编译（{}），不是「待你确认」的版本。请让 author 重新生成。",
            failure.code
        );
    }
    let mut hint = format!(
        "这个候选无效：它没有通过最终编译（{}），不是「待你确认」的版本。",
        failure.code
    );
    match cycle_witness_facts(&failure.message) {
        Some(facts) => {
            hint.push_str(&format!(
                "\n事实（由环 witness 计算）：「{consumer}」托管在仓序第 {consumer_order} 个仓，\
                 却要消费仓序第 {provider_order} 个仓「{provider}」的产出——跨仓依赖只允许\
                 后仓消费前仓，这条依赖在仓序规则下不可能成立，与仓序自动边合成环。",
                consumer = facts.consumer,
                consumer_order = facts.consumer_order,
                provider = facts.provider,
                provider_order = facts.provider_order,
            ));
            hint.push_str("\n两个合法修法（业务取舍，平台不能替你选，也不存在唯一正确解）：");
            for (index, option) in cycle_owner_options(&facts).iter().enumerate() {
                hint.push_str(&format!(
                    "\n{}. {}——后果：{}",
                    index + 1,
                    option.label,
                    option.consequence
                ));
            }
        }
        None => {
            hint.push_str(
                "\n环 witness 不完整：平台没有足够事实指出具体哪条依赖逆了仓序\
                 （完整诊断原文见上），因此不给出位次与选项，也不做任何归因断言。",
            );
        }
    }
    hint.push_str(
        "\n编译类失败会先按剩余预算自动让 author 重写候选；若同一依赖环重复出现或预算用尽，\
         平台会提前停下并回到这里。",
    );
    hint
}

/// REQ-WG-05/06（REMOVED REQ-WSC-20）：重写要点（给 owner 的摘要段与给
/// author 的逐条指令各自取用）。witness 完整时携带算出的位次与两个消环
/// 方向 + 保护项；不完整时退化为无归因的通用消环要点。任何情况下都
/// **不含**「根因可能在 design / 需要修改 design」条目。
pub(crate) fn rewrite_guidance(failure: &CandidateCompileFailure) -> Vec<String> {
    if !is_candidate_invalid_code(&failure.code) {
        return Vec::new();
    }
    match cycle_witness_facts(&failure.message) {
        Some(facts) => vec![
            format!(
                "「{consumer}」托管在仓序第 {consumer_order} 个仓，却把仓序第 {provider_order} \
                 个仓的「{provider}」写成 provider：这条跨仓依赖与仓序自动边互逆，是本次成环的那一跳",
                consumer = facts.consumer,
                consumer_order = facts.consumer_order,
                provider = facts.provider,
                provider_order = facts.provider_order,
            ),
            format!(
                "消环方向二选一：去掉「{consumer}」对「{provider}」的消费（其验收范围相应收窄）；\
                 或把「{consumer}」连同写范围与测试整体迁到第 {provider_order} 个仓或更后\
                 （写范围路径与执行命令同步修改）",
                consumer = facts.consumer,
                provider = facts.provider,
                provider_order = facts.provider_order,
            ),
            "保护项：不得为消环删除真实的业务数据流；不得改动仓序本身（仓序由 owner 在 \
             design 侧决定，plan 内只能调整归属与依赖）"
                .to_string(),
        ],
        None => vec![
            "依赖图必须无环：环上任一条依赖反向都会让整条链无法执行，需整体检查依赖方向"
                .to_string(),
            "同一成员仓内的 Work Item 之间没有仓序自动边，同仓顺序只能由 depends_on 显式声明"
                .to_string(),
        ],
    }
}

/// REQ-WSC-21：**允许自动重写**的稳定码族——只收 owner 2026-10-31 点名的
/// `final_compile_prepare_failed`（现场环就出在这里）。
///
/// 候选门结构拒收（`final_compile_structure_rejected` 等）仍按既有 REQ-SG-03
/// 契约停点落门：那是结构门自己的 typed 停点语义，不属于本 change 的自动重写
/// 范围（改它会动结构门既有的失败呈现与测试契约）。
fn is_auto_rewrite_code(stable_code: &str) -> bool {
    matches!(stable_code, "final_compile_prepare_failed")
}

/// 候选自身无效（编译类失败）的稳定码族：这一族不允许 owner 拿它去「确认」。
pub(crate) fn is_candidate_invalid_code(stable_code: &str) -> bool {
    matches!(
        stable_code,
        "final_compile_prepare_failed"
            | "final_compile_structure_rejected"
            | "single_candidate_structure_operational"
            | "single_candidate_structure_context_failed"
    )
}

/// REQ-WG-06（REMOVED REQ-WSC-20）：给 author 的教学块（进入 SC 门内修订
/// 提示词）。措辞与真实判定同源：稳定码 + 人话原因 + **算出的位次**与两个
/// 消环方向 + 保护项；witness 不完整时退化为无归因的通用消环要点。
/// **不要求 author 去改 design**，也不复述仓序规则教条（REQ-WG-04：任何
/// 教学面不得要求 author 理解或复述 change_order 规则）。
pub(crate) fn author_teaching_block(failure: &CandidateCompileFailure) -> String {
    let mut block = format!(
        "[candidate_compile_failure]\n\
         上一版候选没有通过最终编译（{code}）：{message}\n",
        code = failure.code,
        message = failure.message,
    );
    if is_candidate_invalid_code(&failure.code) {
        match cycle_witness_facts(&failure.message) {
            Some(facts) => {
                block.push_str(&format!(
                    "必须按以下算出的事实修正 plan 的依赖声明，否则再次编译仍会失败：\n\
                     - 「{consumer}」当前托管在第 {consumer_order} 个仓，而它消费的「{provider}」\
                     在第 {provider_order} 个仓——在这两个位置下这条依赖无法成立\n\
                     - 消环方向二选一（都是合法修法）：① 去掉「{consumer}」对「{provider}」的消费\
                     （其验收范围相应收窄）；② 把「{consumer}」连同写范围与测试整体迁到\
                     第 {provider_order} 个仓或更后（写范围路径与执行命令同步修改）\n\
                     - 保护项：不得为消环删除真实的业务数据流；不得改动仓序本身\
                     （仓序由 owner 在 design 侧决定，plan 内只能调整归属与依赖）\n",
                    consumer = facts.consumer,
                    consumer_order = facts.consumer_order,
                    provider = facts.provider,
                    provider_order = facts.provider_order,
                ));
            }
            None => {
                block.push_str(
                    "必须消依赖环后重新输出完整候选：环上任一条依赖反向都会让整条链无法\
                     执行，需整体检查依赖方向；同仓内顺序只能由 depends_on 显式声明。\n",
                );
            }
        }
    } else {
        block.push_str(
            "请按上面的失败原因修正候选；若原因指向平台外的既有事实，先补齐事实再重新输出完整候选。\n",
        );
    }
    block.push_str("[/candidate_compile_failure]\n");
    block
}

impl crate::product::workspace_engine::WorkspaceEngine {
    /// REQ-WSC-21/25：候选编译类失败的**自动重写执行**（不是只发布决策）。
    ///
    /// 判据与动作全部来自 durable 事实与既有通道：
    /// - durable `candidate_compile_status.auto_rewrite.phase == Running` 才执行
    ///   （`Stopped`＝同一环重复或自动池用尽 → 交人工门，见 REQ-WSC-22）；
    /// - 动作是 `route_single_candidate_contract_prerevision`：与 F5-A 机械返修
    ///   完全同一条 ingestion（Reviewerrun 节点 + `complete_review` + 策略路由
    ///   `TriggerAggregateRepair` → `request_provider_run(SC author)`），不新造
    ///   通道、不改状态机。平台编译失败 finding 在策略层按
    ///   [`PLATFORM_COMPILE_FAILURE_FINDING_PREFIX`] 识别（见 review/routing.rs）。
    ///
    /// 返回 `true` 表示本次失败已被通道消费——**要么**已委托 author 重写
    /// （phase=Generate），**要么**策略路由已开好人工门；两种情况调用方都
    /// 不得再开第二扇门（现场 2adc5e41 曾双门：「需要人工接管」+「人工确认」）。
    /// 委托未发生但门已开时，durable `auto_rewrite` 立即对账为 `Stopped`
    /// （REQ-WSC-25：不得对 owner 谎报 running）。
    pub(crate) async fn route_compile_failure_to_rewrite(&mut self) -> bool {
        use crate::product::models::CandidateAutoRewritePhase;
        let Some(status) = self.current_candidate_compile_status() else {
            return false;
        };
        let Some(auto_rewrite) = status.auto_rewrite.as_ref() else {
            return false;
        };
        if auto_rewrite.phase != CandidateAutoRewritePhase::Running {
            return false;
        }
        let Some(failure) = status.failure.as_ref() else {
            return false;
        };
        let verdict =
            crate::product::workspace_engine::compile_failure_surface::auto_rewrite_verdict(
                failure,
            );
        let gate_open_before = self.session.human_gate_snapshot.is_some();
        // E0733（类型级递归）：apply_policy_route 的 ContinueToCompleted 臂会回到
        // enter_policy_valid_work_item_plan_compile，与本方法同处一条 async 递归
        // 路径——Box::pin 打断无限大 future 的类型推导。
        Box::pin(self.route_single_candidate_contract_prerevision(verdict)).await;
        let delegated = self.sc_author_rerun_delegated();
        let gate_opened_by_route = !gate_open_before && self.session.human_gate_snapshot.is_some();
        if !delegated && !gate_opened_by_route {
            // 评审轮没开成（如 reviewer 配置缺失/Fake 快速路径直落人工确认但
            // 不带门快照）：维持既有回落语义，由调用方落门。
            return false;
        }
        if !delegated {
            self.reconcile_auto_rewrite_not_delegated().await;
        }
        true
    }

    /// REQ-WSC-25（尝试可序列观测）：自动重写承诺与真实委托的对账。
    ///
    /// 策略路由把合成 verdict 改道人工门（复评预算已耗尽 / 结构授权窗口等）
    /// 时，durable `auto_rewrite.phase=running` 会永远谎报「正在自动重写」
    /// （现场 2adc5e41 即此形态）——此处对账为 `Stopped`（保留剩余/重复计数），
    /// 并以 Error 事件留下可观测痕迹。
    ///
    /// Task 3（工作包 3.3/3.4）：对账同时**回退该次预领扣减**——预领未被
    /// 消费为 ProviderStarted 且无实际启动观察时按预领键退款（不退就是白占
    /// 额度）；已启动 ⇒ 不退（真跑过）。回退保留诊断计数（`auto_rewrites_used`
    /// 不回退）与 Error 事件痕迹、refunded 留痕不删（REQ-WSC-25 尝试序列
    /// 不回归）。
    pub(crate) async fn reconcile_auto_rewrite_not_delegated(&mut self) {
        use crate::product::models::CandidateAutoRewritePhase;
        let Some(lifecycle) = self.lifecycle_store.clone() else {
            return;
        };
        let Ok(Some(mut status)) =
            lifecycle.load_candidate_compile_status(&self.session.session_id)
        else {
            return;
        };
        let Some(auto_rewrite) = status.auto_rewrite.as_mut() else {
            return;
        };
        if auto_rewrite.phase != CandidateAutoRewritePhase::Running {
            return;
        }
        // Task 3（工作包 3.3/3.4）：未委托 ⇒ 回退该次预领扣减（不退就是白占
        // 额度）。识别判据 = durable 事实（`unconsumed_preclaim_key`）：预领
        // 未被消费为 ProviderStarted 且无实际启动观察 ⇒ 退；已启动 ⇒ 不退
        // （真跑过）。回退先于 phase→Stopped 落盘——中途崩溃时 phase 仍
        // Running，对账重放幂等重入（同键 refund 只写一次），不留悬空扣减。
        // ⚠️ 回退未落 ⇒ **不**推进 phase→Stopped：phase 仍 Running ⇒ 对账可重放
        // （同键 refund 只写一次，幂等重入）⇒ 不把「已扣未退」固化成第三态
        // （Task 3 评审 Minor-2：原实现把读错误与退款错误双双吞掉后仍落
        // Stopped，而 `phase != Running` 的早退会让对账再不可达）。
        let refund_failed = match lifecycle.get_workspace_session(&self.session.session_id) {
            Ok(record) => match unconsumed_preclaim_key(&record) {
                Some(key) => lifecycle
                    .refund_author_rewrite_charge(
                        &self.session.session_id,
                        &key,
                        chrono::Utc::now().to_rfc3339(),
                    )
                    .is_err(),
                None => false,
            },
            Err(error) => {
                tracing::warn!(
                    %error,
                    "auto rewrite reconcile could not read the durable session for refund"
                );
                true
            }
        };
        if refund_failed {
            let _ = self
                .event_tx
                .send(EngineEvent::Error {
                    message: format!(
                        "candidate compile auto rewrite reconcile could not refund the unconsumed \
                         preclaim charge; the durable phase stays running for replay (session {})",
                        self.session.session_id
                    ),
                })
                .await;
            return;
        }
        auto_rewrite.phase = CandidateAutoRewritePhase::Stopped;
        if lifecycle
            .save_candidate_compile_status(&self.session.session_id, &status)
            .is_err()
        {
            return;
        }
        let _ = self
            .event_tx
            .send(EngineEvent::Error {
                message: format!(
                    "candidate compile auto rewrite was routed to the human gate instead of an \
                     author rerun; durable auto_rewrite reconciled to stopped (session {})",
                    self.session.session_id
                ),
            })
            .await;
    }
}

/// 未消费预领键判别（Task 3，durable 事实）：reservation 在场且仍
/// `Reserved`、且该键无实际启动观察（SG4a 口径：预领占位条目 started=true
/// 但 provider/started_at 缺席，不构成启动证据）⇒ 返回该键（未真跑，可
/// 回退）；已 `ProviderStarted` 或有实际启动观察 ⇒ `None`（真跑过，不退）。
pub(super) fn unconsumed_preclaim_key(
    record: &crate::product::models::WorkspaceSessionRecord,
) -> Option<String> {
    let reservation = record.repair_reservation.as_ref()?;
    let key = reservation.provider_start_idempotency_key.as_str();
    let consumed = reservation.state
        == crate::product::work_item_plan_policy::RepairReservationState::ProviderStarted
        || record.provider_start_ledger.iter().any(|entry| {
            entry.provider_start_idempotency_key == key && entry.is_actual_launch_observation()
        });
    (!consumed).then_some(key.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::planning_failure::{
        PlanningFailure, PlanningFailureEvidence, PlanningFailureKind, PlanningFailureOrigin,
        PlanningFailureSet, PlanningStage,
    };
    use std::collections::BTreeMap;

    fn operational(stable_code: &str, cause: &str) -> PlanningCompileFailure {
        let mut failure = PlanningCompileFailure::operational(stable_code, cause.to_string());
        failure.failure_id = "planning_compile_failure_test".to_string();
        failure.source_revision_ref = Some("source-0001".to_string());
        failure.plan_candidate_ir_ref = Some("ir-0001".to_string());
        failure
    }

    #[test]
    fn dependency_cycle_message_is_human_and_detail_keeps_raw_ids() {
        let cause = "dependency_cycle: 「B 仓数据迁移」把「A 仓表结构」写成了 provider\n\
                     环上 Work Item id: work_item_compile_001_003, work_item_compile_001_004";
        let failure = operational("final_compile_prepare_failed", cause);
        let message = owner_message(&failure);
        assert!(
            message.contains("把「A 仓表结构」写成了 provider"),
            "人话段必须保留真实环事实：{message}"
        );
        assert!(
            !message.contains("work_item_compile_001_003"),
            "机器 id 留在 detail，不进 message：{message}"
        );
        let detail = detail_text(&failure);
        assert!(
            detail.contains("work_item_compile_001_003"),
            "完整诊断原文必须含环上 id：{detail}"
        );
        assert!(detail.contains("planning_compile_failure_test"), "{detail}");
        assert!(
            detail.contains("[final_compile_prepare_failed]"),
            "{detail}"
        );
    }

    #[test]
    fn operational_code_without_cycle_keeps_full_cause() {
        let failure = operational(
            "final_compile_execute_failed",
            "work item store rejected the transaction",
        );
        let message = owner_message(&failure);
        assert!(message.contains("最终编译执行失败"), "{message}");
        assert!(
            message.contains("work item store rejected the transaction"),
            "完整原因不得被截断：{message}"
        );
    }
    #[test]
    fn candidate_compile_status_binds_failure_to_candidate() {
        let failure = operational("final_compile_prepare_failed", "boom");
        let (status, route) =
            candidate_compile_status(None, &failure, "2026-10-31T00:00:00Z".to_string(), Some(3));
        assert!(
            matches!(
                route,
                CandidateCompileRoute::AutoRewrite {
                    remaining: 2,
                    repeated: 1
                }
            ),
            "首次候选无效失败应进入自动重写（预算 3 → 本次之后剩 2）：{route:?}"
        );
        assert_eq!(status.source_revision_ref.as_deref(), Some("source-0001"));
        assert_eq!(status.plan_candidate_ir_ref.as_deref(), Some("ir-0001"));
        let record = status.failure.as_ref().expect("failure");
        assert_eq!(record.failure_id, "planning_compile_failure_test");
        assert_eq!(record.code, "final_compile_prepare_failed");
        assert_eq!(record.recorded_at, "2026-10-31T00:00:00Z");
        assert!(
            status
                .current_failure(Some("source-0001"), Some("ir-0001"))
                .is_some()
        );
        assert!(
            status
                .current_failure(Some("source-0002"), Some("ir-0001"))
                .is_none()
        );
    }

    #[test]
    fn structure_failure_message_states_structural_rejection() {
        let set = PlanningFailureSet {
            failures: vec![PlanningFailure {
                kind: PlanningFailureKind::DependencyCycle {
                    witness: Vec::new(),
                },
                origin: PlanningFailureOrigin::CanonicalContract,
                stage: PlanningStage::Plan,
                diagnostic_ref: "planning-structure/digest/dependency_cycle".to_string(),
                source_ref: "plan-source/one".to_string(),
                source_hash: "digest".to_string(),
                evidence: PlanningFailureEvidence {
                    affected_ids: vec!["WI-1".to_string(), "WI-2".to_string()],
                    details: BTreeMap::new(),
                },
            }],
        };
        let failure = PlanningCompileFailure::structure(set);
        let message = owner_message(&failure);
        assert!(message.contains("结构校验"), "{message}");
        assert!(message.contains("dependency_cycle"), "{message}");
    }
    #[test]
    fn same_structural_cycle_stops_auto_rewrite_instead_of_burning_budget() {
        // REQ-WSC-22：同一依赖环（结构指纹相同）第二次出现即提前停，不白耗预算。
        let fingerprint = "a".repeat(64);
        let cause = format!(
            "dependency_cycle: 「A」把「B」写成 provider{}{fingerprint}{}\nwork_item_x_003",
            CYCLE_FINGERPRINT_MARKER, CYCLE_IDS_MARKER
        );
        let failure = operational("final_compile_prepare_failed", &cause);
        let (first, first_route) =
            candidate_compile_status(None, &failure, "2026-10-31T00:00:00Z".to_string(), Some(3));
        assert_eq!(
            first_route,
            CandidateCompileRoute::AutoRewrite {
                remaining: 2,
                repeated: 1
            }
        );
        assert_eq!(
            first.cycle_fingerprint.as_deref(),
            Some(fingerprint.as_str())
        );
        assert_eq!(first.auto_rewrites_used, 1);
        let (second, second_route) = candidate_compile_status(
            Some(&first),
            &failure,
            "2026-10-31T00:05:00Z".to_string(),
            Some(2),
        );
        assert_eq!(
            second_route,
            CandidateCompileRoute::Escalate {
                repeated: 2,
                remaining: 2
            },
            "同一环重复必须提前停（不消耗第二次预算）"
        );
        assert_eq!(second.auto_rewrites_used, 1, "停下时不得再扣预算");
        let auto_rewrite = second.auto_rewrite.expect("stopped phase present");
        assert_eq!(
            auto_rewrite.phase,
            crate::product::models::CandidateAutoRewritePhase::Stopped
        );
        assert_eq!(auto_rewrite.repeated, Some(2));
    }
    #[test]
    fn decide_route_reads_unified_balance_only() {
        // 工作包 1.5（Task 4）：余额事实源 = 统一权威余额（第四参）——
        // previous.auto_rewrites_used 只是诊断计数，不再喂余额。
        let code = "final_compile_prepare_failed";
        let failure = operational(code, "boom");
        let previous = candidate_compile_status(None, &failure, String::new(), Some(3)).0;
        let route = |available| decide_route(code, None, Some(&previous), available);
        assert!(matches!(
            route(Some(0)),
            CandidateCompileRoute::Escalate { remaining: 0, .. }
        ));
        assert!(matches!(
            route(Some(2)),
            CandidateCompileRoute::AutoRewrite { remaining: 1, .. }
        ));
        assert!(matches!(
            route(None),
            CandidateCompileRoute::Escalate { remaining: 0, .. }
        ));
    }
    #[test]
    fn different_cycle_structure_keeps_auto_rewrite_but_shares_budget() {
        // 换成另一个环（结构指纹不同）＝新事实，仍允许重写，但预算账本连续。
        let first_cause = format!(
            "dependency_cycle: 环一{}{}{}\nwork_item_x_001",
            CYCLE_FINGERPRINT_MARKER,
            "b".repeat(64),
            CYCLE_IDS_MARKER
        );
        let second_cause = format!(
            "dependency_cycle: 环二{}{}{}\nwork_item_x_002",
            CYCLE_FINGERPRINT_MARKER,
            "c".repeat(64),
            CYCLE_IDS_MARKER
        );
        let (first, _) = candidate_compile_status(
            None,
            &operational("final_compile_prepare_failed", &first_cause),
            "2026-10-31T00:00:00Z".to_string(),
            Some(3),
        );
        let (second, route) = candidate_compile_status(
            Some(&first),
            &operational("final_compile_prepare_failed", &second_cause),
            "2026-10-31T00:05:00Z".to_string(),
            Some(2),
        );
        assert_eq!(
            route,
            CandidateCompileRoute::AutoRewrite {
                remaining: 1,
                repeated: 1
            },
            "不同结构＝新事实，按剩余预算继续自动重写：{route:?}"
        );
        assert_eq!(second.auto_rewrites_used, 2);
    }
    #[test]
    fn exhausted_budget_escalates_even_for_a_new_cycle() {
        let mut previous = candidate_compile_status(
            None,
            &operational("final_compile_prepare_failed", "boom"),
            "2026-10-31T00:00:00Z".to_string(),
            Some(3),
        )
        .0;
        previous.auto_rewrites_used = max_auto_rewrites();
        let cause = format!(
            "dependency_cycle: 环三{}{}{}\nwork_item_x_003",
            CYCLE_FINGERPRINT_MARKER,
            "d".repeat(64),
            CYCLE_IDS_MARKER
        );
        let (_, route) = candidate_compile_status(
            Some(&previous),
            &operational("final_compile_prepare_failed", &cause),
            "2026-10-31T00:09:00Z".to_string(),
            Some(0),
        );
        assert_eq!(
            route,
            CandidateCompileRoute::Escalate {
                repeated: 1,
                remaining: 0
            },
            "预算用尽即停并交人工门：{route:?}"
        );
    }
    #[test]
    fn structure_gate_rejection_keeps_existing_contract_without_auto_rewrite() {
        // REQ-SG-03 的结构门停点不属于本 change 的自动重写范围：不得静默改成
        // 「开评审轮 + 重驱 author」。
        let (status, route) = candidate_compile_status(
            None,
            &operational("final_compile_structure_rejected", "structure rejected"),
            "2026-10-31T00:00:00Z".to_string(),
            None,
        );
        assert_eq!(route, CandidateCompileRoute::Gate);
        assert!(status.auto_rewrite.is_none(), "结构门停点不得宣称自动重写");
        // 门卡仍按候选无效呈现（owner 可见 + 确认 fail-closed）。
        assert_eq!(
            status.failure.as_ref().map(|failure| failure.code.as_str()),
            Some("final_compile_structure_rejected")
        );
    }
    #[test]
    fn non_candidate_invalid_code_never_opens_auto_rewrite() {
        let (status, route) = candidate_compile_status(
            None,
            &operational("persistence_failure", "store unavailable"),
            "2026-10-31T00:00:00Z".to_string(),
            None,
        );
        assert_eq!(route, CandidateCompileRoute::Gate);
        assert!(
            status.auto_rewrite.is_none(),
            "非候选无效类不得宣称自动重写"
        );
    }
    #[test]
    fn auto_rewrite_verdict_is_mechanical_revise_with_stable_structural_identity() {
        use crate::product::work_item_plan_policy::{FindingClassHint, ReviewFindingCategory};
        use crate::web::workspace_ws_types::{
            ReviewFindingSeverity, ReviewGate, ReviewVerdictType,
        };
        let fingerprint = "e".repeat(64);
        let detail = format!("failure_id: x\ncode: y{CYCLE_FINGERPRINT_MARKER}{fingerprint}");
        let failure = CandidateCompileFailure {
            failure_id: "planning_compile_failure_one".to_string(),
            code: "final_compile_prepare_failed".to_string(),
            message: "「gateway 删除转发」把「frontend 行删除入口」写成 provider：构成依赖环"
                .to_string(),
            detail: detail.clone(),
            recorded_at: "2026-10-31T00:00:00Z".to_string(),
        };
        let verdict = auto_rewrite_verdict(&failure);
        assert_eq!(verdict.verdict, ReviewVerdictType::Revise);
        assert_eq!(verdict.review_gate, ReviewGate::RequiresRevision);
        let finding = verdict.findings.first().expect("finding");
        assert_eq!(finding.severity, ReviewFindingSeverity::MustFix);
        assert_eq!(finding.category, Some(ReviewFindingCategory::ContractGap));
        assert_eq!(finding.class_hint, Some(FindingClassHint::Repairable));
        assert!(
            finding.message.contains(&format!(
                "candidate_compile_failure:final_compile_prepare_failed:{fingerprint}"
            )),
            "结构身份进 message（同环同文本 → 同指纹）：{}",
            finding.message
        );
        // 跨编译尝试/不同 failure_id：结构身份必须不变（REQ-WSC-22）。
        let mut next_attempt = failure.clone();
        next_attempt.failure_id = "planning_compile_failure_two".to_string();
        next_attempt.detail =
            format!("failure_id: z\ncode: y{CYCLE_FINGERPRINT_MARKER}{fingerprint}");
        assert_eq!(
            auto_rewrite_verdict(&next_attempt).findings[0].message,
            finding.message
        );
        // witness 不完整（非 offending 模板）：required_action 走通用消环要点，
        // **不得**要求改 design、不得含任何归因断言。
        assert!(
            finding.required_action.contains("依赖图必须无环"),
            "{}",
            finding.required_action
        );
        assert!(
            !finding.required_action.contains("必须同步修订 Design"),
            "author 侧不得被要求改 design：{}",
            finding.required_action
        );
        assert!(
            !verdict.comments.contains("必须同步修订 Design"),
            "{}",
            verdict.comments
        );
        // REQ-WG-05：witness 不完整 → 不虚构 owner 问句与选项。
        assert!(finding.owner_question.is_none());
        assert!(finding.owner_options.is_none());
    }
    #[test]
    fn cycle_witness_facts_parse_offending_template_or_stay_none() {
        let offending = "「A」 → 「B」 → 「A」 构成依赖环（箭头为产出流向）。其中「B」把「A」写成 provider，但「A」所在仓是 change_order 第 3 个仓、「B」所在仓是第 2 个仓——跨仓依赖只允许后仓消费前仓。";
        let facts = cycle_witness_facts(offending).expect("offending template parses");
        assert_eq!(facts.consumer, "B");
        assert_eq!(facts.provider, "A");
        assert_eq!(facts.provider_order, 3);
        assert_eq!(facts.consumer_order, 2);
        // 无 offending 句（如同仓环 / 兜底文案）→ None，不猜。
        assert!(cycle_witness_facts("「A」 → 「B」 构成依赖环：谁都无法先执行。").is_none());
        // 位次不构成逆序（模板错乱）→ None。
        let not_reversed = "其中「B」把「A」写成 provider，但「A」所在仓是 change_order 第 1 个仓、「B」所在仓是第 2 个仓——";
        assert!(cycle_witness_facts(not_reversed).is_none());
        // 标题含锚点子串 → 解析失败安全落 None（fail-closed）。
        let poisoned = "其中「B」把「A」写成 provider，但「A」所在仓是 change_order 第 3 个仓、「B 所在仓是第 2 个仓——";
        assert!(cycle_witness_facts(poisoned).is_none());
    }
    #[test]
    fn auto_rewrite_verdict_carries_owner_question_and_options_from_witness() {
        let fingerprint = "9".repeat(64);
        let failure = CandidateCompileFailure {
            failure_id: "planning_compile_failure_witness".to_string(),
            code: "final_compile_prepare_failed".to_string(),
            message: "「公告列表行删除交互与中文确认」 → 「删除链路端到端验收」 → 「公告列表行删除交互与中文确认」 构成依赖环。其中「删除链路端到端验收」把「公告列表行删除交互与中文确认」写成 provider，但「公告列表行删除交互与中文确认」所在仓是 change_order 第 3 个仓、「删除链路端到端验收」所在仓是第 2 个仓——跨仓依赖只允许后仓消费前仓。"
                .to_string(),
            detail: format!("failure_id: x\ncode: y{CYCLE_FINGERPRINT_MARKER}{fingerprint}"),
            recorded_at: "2026-10-31T00:00:00Z".to_string(),
        };
        let verdict = auto_rewrite_verdict(&failure);
        let finding = verdict.findings.first().expect("finding");
        // REQ-CG-09：一个 owner 能懂的问题 + 2 个带后果的选项，全部由
        // typed 事实（位次/端点）生成。
        let question = finding.owner_question.as_deref().expect("owner question");
        assert!(question.contains("托管在仓序第 2 个仓"), "{question}");
        assert!(question.contains("第 3 个仓"), "{question}");
        assert!(question.contains("两个修法选哪个"), "{question}");
        let options = finding.owner_options.as_ref().expect("owner options");
        assert_eq!(options.len(), 2, "两个合法修法（REQ-WG-05）");
        assert!(options[0].label.contains("跨仓消费"), "{:?}", options[0]);
        assert!(
            options[1].label.contains("迁到第 3 个仓或仓序更后"),
            "{:?}",
            options[1]
        );
        for option in options {
            assert!(
                option.consequence.contains("代价") || option.consequence.contains("收窄"),
                "选项必须说明后果：{option:?}"
            );
        }
        // required_action 携带算出的位次（REQ-WG-06），且不要求改 design。
        assert!(
            finding.required_action.contains("托管在仓序第 2 个仓"),
            "{}",
            finding.required_action
        );
        assert!(
            !finding.required_action.contains("design 依赖顺序"),
            "{}",
            finding.required_action
        );
    }

    #[test]
    fn approve_rejection_says_why_and_what_next() {
        let failure = CandidateCompileFailure {
            failure_id: "planning_compile_failure_test".to_string(),
            code: "final_compile_prepare_failed".to_string(),
            message: "依赖环：…".to_string(),
            detail: "detail".to_string(),
            recorded_at: "2026-10-31T00:00:00Z".to_string(),
        };
        let message = approve_rejected_message(&failure);
        assert!(message.starts_with("CANDIDATE_COMPILE_FAILED"), "{message}");
        assert!(message.contains("依赖环"), "{message}");
        assert!(message.contains("再次失败"), "{message}");
        assert!(message.contains("下一步"), "{message}");
    }
}
