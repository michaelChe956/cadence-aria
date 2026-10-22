//! T23 轮次合同断言(轮次覆盖 / plan 定稿链 / 级联归因优先)。
//!
//! 诊断 `/tmp/aria-bug45/t25/diag-round-contract.md` §2/§4 的 F3/F4/F6:三处
//! 判据原先散在 `live_b45_t23` 主体里,连同 docstring 与 panic 文本一起
//! 整体落到本文件(大文件守卫:`live_b45_t23` 已按祖父条款登记上限,只许
//! 删不许涨)。
//!
//! 判据口径(合同表 A/B/C):
//! - full/capture 轮:streaming 入口逐 stage 至少 1 个 confirmed fresh 执行
//!   格;plan 另需 **durable 定稿**证据(plan Confirmed + 子 work item 落库,
//!   不看格状态——claude r2 现场 plan resume 格 denied 而已定稿);split_sync
//!   对照入口交 `split_sync_coverage` 单判。
//! - resume 轮:story/design/plan(streaming)+split(split_sync)各恰 1 个
//!   carried 格且来源链可复核;coding/review 各恰 1 个 resume 格,状态为
//!   confirmed 或自证齐备的 not_applicable(不是「必须真实执行」——终门
//!   空转是产品设计语义)。
//! - 级联归因优先:同阶段 fresh 未确认时先报 fresh(下游格不可单独判);
//!   plan 未定稿时先报 plan(下游缺格是结果,不是根因)。

use super::super::harness::plan_chain::PlanChainDurability;
use super::super::harness::{
    ENTRYPOINT_SPLIT_SYNC, ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT, EvidenceCell, FRESH, RESUME,
};
use super::split_sync_coverage;

/// resume 轮必需承继阶段(阶段, 入口):story/design/plan(streaming)+
/// split(split_sync)。
pub(super) const RESUME_CARRIED_STAGES: [(&str, &str); 4] = [
    ("story", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT),
    ("design", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT),
    ("plan", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT),
    ("split", ENTRYPOINT_SPLIT_SYNC),
];

/// full_chain 轮 streaming 入口逐 stage 至少 1 个 confirmed fresh 执行格;
/// split_sync 对照入口按 sync 桥合同单判(见 `verify_split_sync_control`)。
pub(super) const FULL_CHAIN_FRESH_STAGES: [(&str, &str); 5] = [
    ("story", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT),
    ("design", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT),
    ("plan", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT),
    ("coding", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT),
    ("review", ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT),
];

/// 轮次覆盖断言(纯函数;负测在 `mod tests` 钉死拒绝形态)。
///
/// - resume 轮:story/design/plan(streaming)+split(split_sync)各恰 1 个
///   carried 格且来源链可复核;coding/review 各恰 1 个 resume 格,状态为
///   **confirmed 或自证齐备的 not_applicable**(终门空转/无可续评审会话是
///   产品设计语义,不是恢复失败);carried 冒充 resume 拒绝。
/// - full_chain 轮:streaming 入口逐 stage 至少 1 个 fresh 执行格;
///   split_sync 对照入口按 sync 桥合同单判(carried 承继或任意状态的
///   真实 fresh 尝试均可,不以 confirmed 口径要求)。
pub(super) fn verify_round_coverage(
    resume_round: bool,
    cells: &[EvidenceCell],
) -> Result<(), String> {
    if resume_round {
        verify_resume_round(cells)
    } else {
        verify_full_chain_round(cells)
    }
}

pub(super) fn verify_resume_round(cells: &[EvidenceCell]) -> Result<(), String> {
    for (stage, entrypoint) in RESUME_CARRIED_STAGES {
        // F6:承继格以 fresh 相位落盘(fresh_or_resume=fresh + capability=
        // carried);相位不钉死会让同 stage 的执行格混进承继计数。
        let carried: Vec<&EvidenceCell> = cells
            .iter()
            .filter(|cell| {
                cell.stage == stage
                    && cell.entrypoint == entrypoint
                    && cell.fresh_or_resume == FRESH
                    && cell.capability_state == "carried"
            })
            .collect();
        if carried.len() != 1 {
            return Err(format!(
                "resume 轮 stage={stage} entrypoint={entrypoint} 必须恰 1 个 carried 格,\
                 实际 {}",
                carried.len()
            ));
        }
        let cell = carried[0];
        split_sync_coverage::verify_carried_reference(
            cell,
            &format!("resume 轮 stage={stage} entrypoint={entrypoint}"),
        )?;
    }
    for stage in ["coding", "review"] {
        // F4(级联归因优先):先看同阶段 fresh。fresh 未确认时 resume 格是
        // 被上游拖红的,单判「resume 格状态非法」会把根因指错下游。
        if let Some(fresh) = cells
            .iter()
            .find(|cell| cell.stage == stage && cell.fresh_or_resume == FRESH)
            && fresh.capability_state != "confirmed"
        {
            return Err(format!(
                "resume 轮 stage={stage} entrypoint={ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT} \
                 fresh 未确认(状态={:?},原因={:?});resume 格不可单独判",
                fresh.capability_state, fresh.denied_reason
            ));
        }
        let phase_cells: Vec<&EvidenceCell> = cells
            .iter()
            .filter(|cell| cell.stage == stage && cell.fresh_or_resume == RESUME)
            .collect();
        if phase_cells.len() != 1 {
            return Err(format!(
                "resume 轮 stage={stage} 必须恰 1 个 resume 格,实际 {}",
                phase_cells.len()
            ));
        }
        let cell = phase_cells[0];
        // A1:终门空转格(attempt 已停终门、产品不重启 runner)合法但须自证;
        // 其余状态一律非法,不借状态名逃避 resume 判等。
        super::super::harness::r2_evidence_semantics::verify_resume_phase_cell(stage, cell)?;
        if cell.execution_origin.starts_with("carried:") {
            return Err(format!(
                "resume 轮 stage={stage}:carried 格冒充 executed-resume\
                 (承继格不重新计票,不得计作原生恢复)"
            ));
        }
        match (
            cell.requested_resume_id.as_deref(),
            cell.native_resume_confirmed_id.as_deref(),
        ) {
            (Some(requested), Some(confirmed))
                if !requested.trim().is_empty() && requested == confirmed => {}
            _ => {
                return Err(format!(
                    "resume 轮 stage={stage} resume 格的 native ID 不一致:请求 {:?} 确认 {:?}",
                    cell.requested_resume_id, cell.native_resume_confirmed_id
                ));
            }
        }
    }
    Ok(())
}

fn verify_full_chain_round(cells: &[EvidenceCell]) -> Result<(), String> {
    for (stage, entrypoint) in FULL_CHAIN_FRESH_STAGES {
        let fresh_executed = cells.iter().any(|cell| {
            cell.stage == stage
                && cell.entrypoint == entrypoint
                && cell.fresh_or_resume == FRESH
                && cell.capability_state == "confirmed"
                && !cell.execution_origin.starts_with("carried:")
        });
        if !fresh_executed {
            // F4:下游(coding/review)缺格时带上游 plan 现场——根因通常不在
            // 本格(F3 的 durable 定稿断言会先报 plan;此处在断言顺序之外
            // 单独调用时仍能一眼看到上游原因)。
            let upstream_plan = if stage == "coding" || stage == "review" {
                cells
                    .iter()
                    .find(|cell| {
                        cell.stage == "plan"
                            && cell.entrypoint == ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT
                            && cell.fresh_or_resume == RESUME
                    })
                    .and_then(|cell| cell.denied_reason.clone())
            } else {
                None
            };
            return Err(format!(
                "full_chain 轮 stage={stage} entrypoint={entrypoint} 缺 fresh 执行格\
                 (不得以 carried/resume 冒充);plan resume 格现场={}",
                upstream_plan.as_deref().unwrap_or("-")
            ));
        }
    }
    split_sync_coverage::verify_split_sync_control(cells)
}

/// F3:full/capture 轮的 plan 定稿链断言——判据是产品 durable 事实
/// (plan Confirmed + 子 work item 落库),不是任何格的状态。
///
/// 反例(claude r2 全绿):plan resume 格 `denied`(审计投影摘要 != 冻结投影
/// 摘要),但 typed confirm 成功、work item 已落库、coding fresh confirmed
/// ——若按格状态要求 confirmed,这一轮会被误判红。反向(pi r3-capture):
/// plan resume 被产品机械校验拒绝、plan 永不定稿 → 该断言直接指名根因,
/// 不再把 coding 不可达报成「缺 fresh 执行格」。
pub(super) fn verify_plan_chain_durable(
    chain: &PlanChainDurability,
    cells: &[EvidenceCell],
) -> Result<(), String> {
    if chain.durably_confirmed() {
        return Ok(());
    }
    let plan_resume = cells
        .iter()
        .find(|cell| {
            cell.stage == "plan"
                && cell.entrypoint == ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT
                && cell.fresh_or_resume == RESUME
        })
        .and_then(|cell| cell.denied_reason.clone())
        .unwrap_or_else(|| "-".to_string());
    Err(format!(
        "stage=plan entrypoint={ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT} 未定稿:\
         plan 链 durable 事实缺「plan Confirmed + 子 work item 落库」({}),\
         下游 stage=coding/review 不可达;plan resume 格现场={plan_resume}",
        chain.describe()
    ))
}

// ---------------------------------------------------------------------------
// 负测(先红后绿):F3 plan 定稿链 / F4 级联归因优先 / F6 承继相位钉死。
// 夹具复用 `t23_assertion_tests` 的合成格(不需要真实 CLI/LC_GATEWAY_E2E)。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod round_contract_tests {
    use super::*;
    use crate::web_lc_gateway_multi_provider::live_b45_t23::t23_assertion_tests::{
        carried_test_cell, full_chain_cells, resume_round_cells, test_manifest,
    };
    /// F3(轮次合同修复)负测:full/capture 轮的「plan 定稿」判据是 **durable
    /// 事实**,不是 plan resume 格状态。
    ///
    /// - claude r2 现场反例:plan resume 格 denied(审计投影摘要 != 冻结投影
    ///   摘要),但 typed confirm 成功、work item 已落库 → 必须放行;
    /// - pi r3-capture 形态:plan resume denied 且 work item 未落库 → 必须红,
    ///   且错误指名 plan 未定稿(**先于**下游 coding 缺格报出)。
    #[test]
    fn t23_full_chain_plan_chain_durable_ignores_cell_state() {
        let mut cells = full_chain_cells();
        for cell in &mut cells {
            if cell.stage == "plan" && cell.fresh_or_resume == RESUME {
                cell.capability_state = "denied".to_string();
                cell.denied_reason = Some("audit 投影摘要 != 冻结投影摘要".to_string());
            }
        }
        let confirmed = PlanChainDurability {
            plan_id: "plan_0001".to_string(),
            work_item_id: "WI-001".to_string(),
            plan_status: "confirmed".to_string(),
            read_error: None,
        };
        // 格 denied 但链定稿 → 放行(不得按格状态判)。
        assert_eq!(
            verify_plan_chain_durable(&confirmed, &cells),
            Ok(()),
            "plan resume 格 denied 但 durable 定稿必须放行(claude r2 反例)"
        );

        // work item 未落库(空 id)→ 红,且错误先指 plan、带 plan resume 现场。
        let undrafted = PlanChainDurability {
            work_item_id: String::new(),
            ..confirmed.clone()
        };
        let error = verify_plan_chain_durable(&undrafted, &cells).expect_err("未定稿必须拒绝");
        assert!(error.contains("未定稿"), "{error}");
        assert!(
            error.contains("stage=plan") && error.contains("entrypoint="),
            "错误必须用 stage=/entrypoint= 指名(不与 split_sync 的 plan/split 撞车):{error}"
        );
        assert!(
            error.contains("audit 投影摘要 != 冻结投影摘要"),
            "错误必须带 plan resume 格现场:{error}"
        );

        // plan 非 Confirmed / 不可读 → 同样红。
        for status in ["draft", "change_requested"] {
            let draft = PlanChainDurability {
                plan_status: status.to_string(),
                ..confirmed.clone()
            };
            assert!(
                verify_plan_chain_durable(&draft, &cells).is_err(),
                "{status}"
            );
        }
    }

    /// F4(级联归因优先)负测:resume 轮先看同阶段 fresh——fresh 未确认时
    /// 必须报「fresh 未确认」,不得把级联结果报成「resume 格状态非法」。
    #[test]
    fn t23_resume_round_attributes_fresh_before_resume() {
        let manifest = test_manifest("/tmp/unused-source-root");
        let mut cascade = resume_round_cells(&manifest);
        for cell in &mut cascade {
            if cell.stage == "coding"
                && cell.entrypoint == ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT
                && cell.fresh_or_resume == FRESH
            {
                cell.capability_state = "denied".to_string();
                cell.denied_reason = Some("work_item_plan_not_confirmed".to_string());
            }
            if cell.stage == "coding" && cell.fresh_or_resume == RESUME {
                cell.capability_state = "denied".to_string();
                cell.denied_reason = Some("级联".to_string());
            }
        }
        let error = verify_round_coverage(true, &cascade).expect_err("fresh 未确认必须红");
        assert!(
            error.contains("fresh 未确认") && error.contains("resume 格不可单独判"),
            "必须按级联归因先报 fresh:{error}"
        );
        assert!(
            error.contains("work_item_plan_not_confirmed"),
            "错误必须带 fresh 现场原因:{error}"
        );

        // fresh confirmed + resume 状态非法 → 仍如实报 resume 格状态。
        let mut resume_illegal = resume_round_cells(&manifest);
        for cell in &mut resume_illegal {
            if cell.stage == "review" && cell.fresh_or_resume == RESUME {
                cell.capability_state = "denied".to_string();
            }
        }
        let error = verify_round_coverage(true, &resume_illegal).expect_err("resume 非法必须红");
        assert!(
            error.contains("resume 格状态 denied 非法"),
            "fresh 已确认时必须报 resume 格状态:{error}"
        );
    }

    /// F6(轮次合同修复):承继格必须钉死 `fresh_or_resume=fresh` 相位——
    /// 否则同 stage 的执行格会被混进承继计数(计数 2 即红,而不是被
    /// 「恰好 1 个 carried」掩盖)。
    #[test]
    fn t23_resume_round_carried_filter_pins_fresh_phase() {
        let manifest = test_manifest("/tmp/unused-source-root");
        let mut cells = resume_round_cells(&manifest);
        // 追加一个 resume 相位、capability=carried 的伪格(承继格不可能是
        // resume 相位;旧过滤会把它计进 carried 计数 → 掩盖真实承继格)。
        let mut masquerade = carried_test_cell(
            "story",
            ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT,
            &manifest,
        );
        masquerade.fresh_or_resume = RESUME.to_string();
        cells.push(masquerade);
        // 真实承继格只有 1 个 + 这个伪格相位不符 → 计数仍为 1,断言通过
        // (相位钉死使其不参与计数,而不是把它当第二个承继格)。
        assert!(
            verify_round_coverage(true, &cells).is_ok(),
            "resume 相位的 carried 伪格不得计入承继计数"
        );
        // 反向:真实承继格自身被改成 resume 相位 → 承继计数变 0,必须红。
        let mut missing = resume_round_cells(&manifest);
        for cell in &mut missing {
            if cell.stage == "design" && cell.capability_state == "carried" {
                cell.fresh_or_resume = RESUME.to_string();
            }
        }
        assert!(
            verify_round_coverage(true, &missing).is_err(),
            "承继格相位不是 fresh 时不得被认作承继格"
        );
    }
}
