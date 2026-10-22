//! T23 轮次覆盖:split_sync 对照入口与承继引用判据。
//!
//! sync 桥无 native session contract(r53 裁定/设计复核):resume 恒零 spawn;
//! full_chain 轮的 fresh 格也不得以 streaming 的 confirmed 口径要求(无
//! permission wire 的 provider 会如实落 denied/unknown)。故本入口的覆盖判读
//! 只要求「可复核落格」:真实 fresh 尝试(run_ref 已绑定,证据状态不限)或
//! 带四摘要的 carried 承继引用;缺失/仅 resume 形态/引用不可复核 → 红。
//!
//! ⚠阶段标签差异(F6,轮次合同修复):本文件的 `stage=="plan"` 过滤**只适用
//! full_chain/capture 轮**(split_sync 落在 plan 阶段的对照入口);resume 轮的
//! split_sync carried 格按合同用 `stage=="split"`(见
//! `round_contract::RESUME_CARRIED_STAGES`),该轮由 `verify_carried_reference`
//! 单判、**不得**复用本文件的 plan 过滤(否则形同漏判)。

use super::super::harness::{ENTRYPOINT_SPLIT_SYNC, EvidenceCell, FRESH};

/// full_chain 轮的 split_sync 对照入口判读(不得 N/A 降格)。
///
/// 只认 `stage=="plan"`(full/capture 轮的对照入口);resume 轮走
/// `verify_carried_reference`(stage=="split"),见模块注释的阶段标签差异。
pub(super) fn verify_split_sync_control(cells: &[EvidenceCell]) -> Result<(), String> {
    let recorded: Vec<&EvidenceCell> = cells
        .iter()
        .filter(|cell| cell.stage == "plan" && cell.entrypoint == ENTRYPOINT_SPLIT_SYNC)
        .collect();
    // carried 格会以 fresh 相位落盘(fresh_or_resume=fresh,capability=carried),
    // 不能算作真实 fresh 尝试。
    if recorded.iter().any(|cell| {
        cell.fresh_or_resume == FRESH
            && cell.capability_state != "carried"
            && !cell.run_ref.trim().is_empty()
    }) {
        return Ok(());
    }
    recorded
        .iter()
        .find(|cell| cell.capability_state == "carried")
        .ok_or_else(|| {
            format!(
                "full_chain 轮 plan/split_sync 缺可复核对照格(仅 resume 形态不计覆盖,\
                 实测 {} 格)",
                recorded.len()
            )
        })
        .and_then(|carried| verify_carried_reference(carried, "full_chain 轮 plan/split_sync"))
}

/// 承继引用形态与四摘要齐备(引用可复核);resume 轮与 full_chain 轮的
/// split_sync carried 形态共用同一判据。
pub(super) fn verify_carried_reference(cell: &EvidenceCell, label: &str) -> Result<(), String> {
    if !cell.execution_origin.starts_with("carried:") || cell.run_ref.trim().is_empty() {
        return Err(format!(
            "{label} 承继引用形态不完整:origin={:?} run_ref={:?}",
            cell.execution_origin, cell.run_ref
        ));
    }
    if cell
        .carried_source_evidence_root
        .as_deref()
        .unwrap_or_default()
        .trim()
        .is_empty()
    {
        return Err(format!("{label} 承继格缺来源证据根(引用不可复核)"));
    }
    for (kind, digest) in [
        ("workspace", &cell.carried_workspace_digest),
        ("aggregate", &cell.carried_aggregate_digest),
        ("capability", &cell.carried_capability_digest),
        ("policy", &cell.carried_policy_digest),
    ] {
        if digest.trim().is_empty() {
            return Err(format!("{label} 承继格缺 {kind} 摘要(引用不可复核)"));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 负测:先红后绿——split_sync 对照入口不被 confirmed-fresh 口径误判,
// 同时缺失/仅 resume/不可复核引用仍红。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod t23_split_sync_tests {
    use super::super::t23_assertion_tests::{
        base_cell, carried_test_cell, full_chain_cells, test_manifest,
    };
    use super::super::*;

    #[test]
    fn t23_full_chain_split_sync_control_accepts_contract_forms() {
        let cells = full_chain_cells();
        assert!(
            verify_round_coverage(false, &cells).is_ok(),
            "合法 full_chain 轮必须通过: {:?}",
            verify_round_coverage(false, &cells)
        );

        // fresh 尝试落 denied(pi 等无 permission wire,sync 桥不产 approval/
        // tool 事件)→ 不判红。
        let mut denied_split = cells.clone();
        for cell in &mut denied_split {
            if cell.stage == "plan" && cell.entrypoint == ENTRYPOINT_SPLIT_SYNC {
                cell.capability_state = "denied".to_string();
                cell.denied_reason = Some("approval/tool 事件缺失".to_string());
            }
        }
        assert!(
            verify_round_coverage(false, &denied_split).is_ok(),
            "split_sync fresh 落 denied 不得判红: {:?}",
            verify_round_coverage(false, &denied_split)
        );

        // split_sync 以 carried 承继(四摘要引用可复核)→ 不判红。
        let manifest = test_manifest("/tmp/unused-source-root");
        let mut carried_split = cells.clone();
        carried_split
            .retain(|cell| !(cell.stage == "plan" && cell.entrypoint == ENTRYPOINT_SPLIT_SYNC));
        carried_split.push(carried_test_cell("plan", ENTRYPOINT_SPLIT_SYNC, &manifest));
        assert!(
            verify_round_coverage(false, &carried_split).is_ok(),
            "split_sync carried 承继不得判红: {:?}",
            verify_round_coverage(false, &carried_split)
        );

        // carried 引用不可复核(缺摘要)→ 红。
        let mut broken_carried = carried_split.clone();
        for cell in &mut broken_carried {
            if cell.stage == "plan" && cell.entrypoint == ENTRYPOINT_SPLIT_SYNC {
                cell.carried_policy_digest = String::new();
            }
        }
        assert!(
            verify_round_coverage(false, &broken_carried).is_err(),
            "split_sync carried 引用不可复核必须拒绝"
        );
    }

    #[test]
    fn t23_full_chain_still_requires_streaming_fresh_and_split_sync_cell() {
        let cells = full_chain_cells();

        // streaming 入口缺 confirmed fresh(只留 resume)→ 红。
        let mut missing_streaming = cells.clone();
        missing_streaming.retain(|cell| {
            !(cell.stage == "coding"
                && cell.entrypoint == ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT
                && cell.fresh_or_resume == FRESH)
        });
        let error = verify_round_coverage(false, &missing_streaming)
            .expect_err("缺 streaming fresh 执行格必须拒绝");
        assert!(error.contains("coding"), "{error}");

        // split_sync 仅 resume 形态(无 fresh 尝试、无 carried)→ 红。
        let mut resume_only_split = cells.clone();
        resume_only_split.retain(|cell| {
            !(cell.stage == "plan"
                && cell.entrypoint == ENTRYPOINT_SPLIT_SYNC
                && cell.fresh_or_resume == FRESH)
        });
        let error = verify_round_coverage(false, &resume_only_split)
            .expect_err("split_sync 仅 resume 形态必须拒绝");
        assert!(error.contains("split_sync"), "{error}");

        // split_sync 整格缺失(入口未落证据)→ 红。
        let mut absent_split = cells.clone();
        absent_split
            .retain(|cell| !(cell.stage == "plan" && cell.entrypoint == ENTRYPOINT_SPLIT_SYNC));
        assert!(
            verify_round_coverage(false, &absent_split).is_err(),
            "split_sync 无任何落格必须拒绝"
        );

        // 空的 fresh 尝试(run_ref 未绑定=未真实启动)+ 无 carried → 红。
        let mut empty_attempt = cells.clone();
        empty_attempt.retain(|cell| {
            !(cell.stage == "plan"
                && cell.entrypoint == ENTRYPOINT_SPLIT_SYNC
                && cell.fresh_or_resume == FRESH)
        });
        let mut placeholder = base_cell("plan", ENTRYPOINT_SPLIT_SYNC, FRESH);
        placeholder.run_ref = String::new();
        empty_attempt.push(placeholder);
        assert!(
            verify_round_coverage(false, &empty_attempt).is_err(),
            "split_sync 空 fresh 尝试(run_ref 未绑定)必须拒绝"
        );
    }
}
