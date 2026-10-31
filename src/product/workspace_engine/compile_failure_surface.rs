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
use crate::product::workspace_engine::types::{PlanningCompileFailure, PlanningCompileFailureKind};

/// 编译失败 cause 里「环上 Work Item id」行的起点标记。producer
/// （`draft_batch::compile_support` 的 change_order 环检测）与本模块共用，
/// 保证「人话段 / 机器 id 段」的切分只有一个来源。
pub(crate) const CYCLE_IDS_MARKER: &str = "\n环上 Work Item id: ";

/// 编译失败 cause 里「环结构指纹」行的起点标记（producer 与本模块共用，
/// REQ-WSC-22：同一环的重复判定只看结构摘要，不看文案）。
pub(crate) const CYCLE_FINGERPRINT_MARKER: &str = "\n环结构指纹: ";

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
/// - 预算账本 `auto_rewrites_used` 会话内累加，编译成功才清零。
pub(crate) fn decide_route(
    code: &str,
    fingerprint: Option<&str>,
    previous: Option<&CandidateCompileStatus>,
    max_auto_rewrites: u32,
) -> CandidateCompileRoute {
    if !is_auto_rewrite_code(code) {
        return CandidateCompileRoute::Gate;
    }
    let used = previous.map_or(0, |status| status.auto_rewrites_used);
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
    let remaining = max_auto_rewrites.saturating_sub(used);
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
/// finding `message` 只由**结构**事实构成（稳定码 + 环结构指纹），作者换措辞
/// 不会让同一环被当成新事实（REQ-WSC-22）；`required_action` 才是给人看的
/// 具体重写要点，且**不要求 author 去改 design**（owner 2026-10-31 第 4 条）。
pub(crate) fn auto_rewrite_verdict(
    failure: &CandidateCompileFailure,
) -> crate::web::workspace_ws_types::ReviewVerdict {
    use crate::product::work_item_plan_policy::{FindingClassHint, ReviewFindingCategory};
    use crate::web::workspace_ws_types::{
        ReviewFinding, ReviewFindingSeverity, ReviewGate, ReviewVerdict, ReviewVerdictType,
    };

    let structural_identity = cycle_fingerprint_from_detail(&failure.detail)
        .unwrap_or_else(|| failure.failure_id.clone());
    let guidance = rewrite_guidance(&failure.code);
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
        owner_question: None,
        owner_options: None,
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
/// `previous` 是上一次尝试的事实（同一会话），用于结构指纹重复判定与预算
/// 账本接续；`recorded_at` 由调用方给（不在此取时钟，便于测试）。
pub(crate) fn candidate_compile_status(
    previous: Option<&CandidateCompileStatus>,
    failure: &PlanningCompileFailure,
    recorded_at: String,
) -> (CandidateCompileStatus, CandidateCompileRoute) {
    let code = failure.stable_code().to_string();
    let fingerprint = cycle_fingerprint(failure);
    let max = max_auto_rewrites();
    let route = decide_route(&code, fingerprint.as_deref(), previous, max);
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
/// 失败恢复后，确认动作会被一条过期失败永久挡死（新的死路）。
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
        // 编译成功＝账本清零：新事实从零计。
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
        hint = owner_hint(&failure.code),
    )
}

/// REQ-WSC-20：给 owner 的醒目提示（owner 2026-10-31 第 4 条裁定：**不要**写成
/// 「系统会自动去改 design」，要讲清 design 的什么原因导致成环，并明确**需要
/// owner 改 design**）。同时说明平台已/将继续自动让 author 重写几次，避免 owner
/// 误以为必须自己手动推。
pub(crate) fn owner_hint(stable_code: &str) -> String {
    if is_candidate_invalid_code(stable_code) {
        return "这个候选无效：它没有通过最终编译，不是「待你确认」的版本。\n\
                根因指向 design：候选里的依赖顺序与 design 声明的仓序（change_order）互相矛盾——\
                design 把仓序定成「后仓先于前仓」，plan 里就必然出现「后仓 Work Item 当前仓 Work Item 的 \
                provider」这类逆仓序依赖，一旦合成环，plan 层面无论怎么换序都无法推进。\
                要根治需要**修改 design 的依赖顺序/仓序**；平台不会自动去改 design。\n\
                平台已经/将继续按剩余预算自动让 author 重写候选：若同一依赖环重复出现，平台会提前停下并回到这里，\
                此时只能由你修改 design 后重新发起。\n\
                重写时着重注意：provider 只能指向仓序在前的成员仓 Work Item；同一成员仓内的顺序只能靠 depends_on 显式声明；依赖图必须无环；\
                根因很可能在 design 的依赖顺序，需要你决定是否修改 design。"
            .to_string();
    }
    format!(
        "这个候选无效：它没有通过最终编译（{stable_code}），不是「待你确认」的版本。请让 author 重新生成。"
    )
}

/// REQ-WSC-20：重写要点（给 owner 的摘要段与给 author 的逐条指令各自取用；
/// 两者的措辞分开发布——owner 面允许提「改 design」，author 面**不得**要求
/// author 去改 design（owner 2026-10-31 第 4 条裁定））。
pub(crate) fn rewrite_guidance(stable_code: &str) -> Vec<String> {
    if is_candidate_invalid_code(stable_code) {
        return vec![
            "provider 只能指向仓序在前的成员仓 Work Item（依赖按仓序从前往后声明），不得把后仓 Work Item 写成前仓 Work Item 的 provider".to_string(),
            "同一成员仓内的 Work Item 之间没有仓序自动边，同仓顺序只能由 depends_on 显式声明".to_string(),
            "依赖图必须无环（dependency_cycle 的判据）：环上任一条依赖反向都会让整条链无法执行，需整体检查依赖方向".to_string(),
            "根因可能在 design 的依赖顺序/仓序：plan 内换序解决不了环，需要由 owner 决定是否修改 design".to_string(),
        ];
    }
    Vec::new()
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

/// REQ-WSC-20：给 author 的教学块（进入 SC 门内修订提示词）。措辞与真实
/// 判定同源：稳定码 + 人话原因 + 逐条技术要点，明确「再犯同样的错仍会失败」。
/// **不要求 author 去改 design**——design 的问题只告知 owner（第 4 条裁定），
/// 这里只要求 author 在 plan 内把依赖方向改成合法形状。
pub(crate) fn author_teaching_block(failure: &CandidateCompileFailure) -> String {
    let mut block = format!(
        "[candidate_compile_failure]\n\
         上一版候选没有通过最终编译（{code}）：{message}\n",
        code = failure.code,
        message = failure.message,
    );
    if is_candidate_invalid_code(&failure.code) {
        block.push_str(
            "必须按以下要点修正 plan 的依赖声明，否则再次编译仍会失败：\n\
             - provider 只能指向仓序在前的成员仓 Work Item（依赖按仓序从前往后声明），不得把后仓 Work Item 写成前仓 Work Item 的 provider\n\
             - 同一成员仓内的 Work Item 之间没有仓序自动边，同仓顺序只能由 depends_on 显式声明\n\
             - 依赖图必须无环：环上任一条依赖反向都会让整条链无法执行，需整体检查依赖方向并消环\n\
             - 若按上述规则在 plan 内确实无法消环（根因在 design 的仓序），不要绕过规则硬写：保持依赖合法并说明冲突位置\n",
        );
    } else {
        block.push_str(
            "请按上面的失败原因修正候选；若原因指向平台外的既有事实，先补齐事实再重新输出完整候选。\n",
        );
    }
    block.push_str("[/candidate_compile_failure]\n");
    block
}

impl crate::product::workspace_engine::WorkspaceEngine {
    /// REQ-WSC-21：候选编译类失败的**自动重写执行**（不是只发布决策）。
    ///
    /// 判据与动作全部来自 durable 事实与既有通道：
    /// - durable `candidate_compile_status.auto_rewrite.phase == Running` 才执行
    ///   （`Stopped`＝同一环重复或自动池用尽 → 交人工门，见 REQ-WSC-22）；
    /// - 动作是 `route_single_candidate_contract_prerevision`：与 F5-A 机械返修
    ///   完全同一条 ingestion（Reviserun 节点 + `complete_review` + 策略路由
    ///   `TriggerAggregateRepair` → `request_provider_run(SC author)`），不新造
    ///   通道、不改状态机。
    ///
    /// 返回 `true` 表示已把失败转成返修轮（调用方不得再开人工确认门）。
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
        // E0733（类型级递归）：apply_policy_route 的 ContinueToCompleted 臂会回到
        // enter_policy_valid_work_item_plan_compile，与本方法同处一条 async 递归
        // 路径——Box::pin 打断无限大 future 的类型推导。
        Box::pin(self.route_single_candidate_contract_prerevision(verdict)).await;
        // 路由是否真的开出评审轮由 stage 判据兜底：`start_review` 成功即
        // CrossReview（后续 ingestion 会重驱 author）；否则当场回落人工门，
        // 绝不静默停在一个既无门也无跑的状态。
        self.session.stage == super::WorkspaceStage::CrossReview
    }
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
            candidate_compile_status(None, &failure, "2026-10-31T00:00:00Z".to_string());
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
            candidate_compile_status(None, &failure, "2026-10-31T00:00:00Z".to_string());
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

        let (second, second_route) =
            candidate_compile_status(Some(&first), &failure, "2026-10-31T00:05:00Z".to_string());
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
        );
        let (second, route) = candidate_compile_status(
            Some(&first),
            &operational("final_compile_prepare_failed", &second_cause),
            "2026-10-31T00:05:00Z".to_string(),
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
        // 重写要点给 author，但**不得**要求它去改 design（owner 第 4 条）。
        assert!(
            finding
                .required_action
                .contains("仓序在前的成员仓 Work Item"),
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
