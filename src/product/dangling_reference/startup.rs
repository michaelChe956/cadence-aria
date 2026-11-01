//! 启动时自动巡检 + 自动销账（REQ-DRG-13 自动模式；REQ-DRG-11 清单自动产出）。
//!
//! owner 拍板（2026-10-28）：**不做人工触发**——巡检与销账只在**服务启动时**
//! 自动跑一次；`aria inspect dangling-references` 人工入口已删除（「我重启下服务
//! 不就行了」）。清单**仍然写出来**供事后查。
//!
//! 三段语义：
//!
//! 1. **巡检**（只读、零副作用）：`patrol_state_root` 遍历五个分区，产出
//!    `PatrolReport`（REQ-DRG-08 / 09 / 23）。
//! 2. **自动销账**：`run_cleanup` 的自动模式——删除判据 = **白名单类型 + 证据
//!    达标**（L1 墓碑 / W-1..W-4 全成立），**不放宽**任何既有删除条件
//!    （REQ-DRG-12 / 14 / 19）；幂等 + 留痕 + 自证归零语义不变（REQ-DRG-15 /
//!    16 / 17）。清单摘要仅用于审计关联，**不是删除凭据**（REQ-DRG-22）。
//! 3. **清单写出**：`cadence/analysis-docs/<运行日>_分析报告_悬空引用巡检清单_v1.0.md`
//!    （REQ-DRG-11）。清单前三节（明细 / 只报不删残留 / 不可判定）记录**本次
//!    巡检（销账前现场）**的发现集合，便于人核对「这次看到了什么、删了什么」；
//!    模式行**如实标注**本次模式与销账结果（自动销账写明实际删除数；销账失败
//!    写明错误原因，MUST NOT 声称未删除），并指向留痕台账供逐条核对。
//!    「归零自证」节（第 6 节）写**销账后**现场：本次可删类型的归零计数、未归零
//!    可删项逐条、不可判定项逐条、治理是否完成——与前三节分节并列、MUST NOT
//!    混列（REQ-DRG-11 / 17 / 23）。
//! 4. **启动日志**：除计数行外，`StartupGovernanceOutcome::log_lines` 把未归零
//!    可删项与不可判定项**逐条**列出（REQ-DRG-17「未归零必须列出」/ REQ-DRG-23
//!    「不可判定逐条列出」）；`governance_complete` 为假时 MUST NOT 宣称完成。
//!
//! **失败口径**：本入口的失败一律以 `Err` 上抛；调用方（服务启动序列）**只打
//! 日志、不阻断启动**。清单在销账失败时**仍然写出**（销账前的现场 + 「未取得
//! 自证结果」），保证「失败也可事后查」。

use std::path::{Path, PathBuf};

use std::collections::BTreeMap;

use crate::product::app_paths::ProductAppPaths;
use crate::product::dangling_reference::cleanup::{CleanupOutcome, CleanupRequest, run_cleanup};
use crate::product::dangling_reference::patrol::{
    PatrolFinding, UndeterminedEntry, patrol_state_root,
};
use crate::product::dangling_reference::report::{
    CleanupReportStatus, GovernanceSelfProof, findings_token, render_markdown_report,
    report_file_name, write_markdown_report,
};
use crate::product::json_store::ProductStoreError;

/// 清单落点的相对目录段（`<workspace>/cadence/analysis-docs/`）。
const REPORT_DIR_SEGMENTS: [&str; 2] = ["cadence", "analysis-docs"];

/// 启动期治理的一次结果（供启动序列打日志 / 核对）。
///
/// `dangling_count` / `undetermined_count` / `findings_token` 描述**销账前**现场
/// （= 清单前三节的数据面）；`unzeroed` / `unzeroed_by_type` / `residual` /
/// `undetermined` / `governance_complete` 描述**销账后**自证现场（= 清单
/// 「归零自证」节的数据面），两组 MUST NOT 混用（REQ-DRG-11 / 17）。
#[derive(Debug, Clone)]
pub(crate) struct StartupGovernanceOutcome {
    /// 巡检清单落点。
    pub(crate) report_path: PathBuf,
    /// 本次巡检发现集合的规范化摘要（仅用于审计关联，**不是删除凭据**）。
    pub(crate) findings_token: String,
    /// 悬空条目数（判定为「目标已删」的条目）。
    pub(crate) dangling_count: usize,
    /// 不可判定条目数（读失败或所需数据源 / 作用域不可用）。
    pub(crate) undetermined_count: usize,
    /// 本次实际删除条数（重入时为 0）。
    pub(crate) deleted_count: usize,
    /// 自证（销账后）**仍未归零的可删条目**（逐条；REQ-DRG-17 要求逐条列出）。
    pub(crate) unzeroed: Vec<PatrolFinding>,
    /// 自证（销账后）未归零可删条目按记录类型的计数（空表 = 可删类型全部归零）。
    pub(crate) unzeroed_by_type: BTreeMap<String, usize>,
    /// 自证（销账后）「只报不删残留」（逐条；不计入归零）。
    pub(crate) residual: Vec<PatrolFinding>,
    /// 自证（销账后）不可判定条目（逐条）。
    pub(crate) undetermined: Vec<UndeterminedEntry>,
    /// 是否治理完成：可删条目全部归零 **且** 无残留 **且** 无不可判定。
    pub(crate) governance_complete: bool,
}

impl StartupGovernanceOutcome {
    /// 启动日志的逐条行（REQ-DRG-17 / 23）：未归零可删项与不可判定项 MUST
    /// 逐条列出，MUST NOT 只打计数；未完成时 MUST NOT 宣称治理完成。
    pub(crate) fn log_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        lines.push(format!(
            "[aria-startup] dangling reference governance self-proof: governance_complete={} unzeroed={} residual={} undetermined={}",
            self.governance_complete,
            self.unzeroed.len(),
            self.residual.len(),
            self.undetermined.len()
        ));
        for finding in &self.unzeroed {
            lines.push(format!(
                "[aria-startup] dangling reference NOT zeroed: record_path={} record_type={} reference_field={} target_kind={} target_id={} evidence={} action={}",
                finding.record_path,
                finding.record_type,
                finding.reference_field,
                finding.target_kind,
                finding.target_id,
                finding.evidence_level,
                finding.proposed_action
            ));
        }
        for entry in &self.undetermined {
            lines.push(format!(
                "[aria-startup] dangling reference UNDETERMINED: record_path={} reference_field={} reason={}",
                entry.record_path, entry.reference_field, entry.reason
            ));
        }
        lines
    }
}

/// 服务启动时自动巡检 + 自动销账 + 落盘清单（REQ-DRG-13 自动模式）。
///
/// `paths` = 应用状态根（`<workspace>/.aria`）；`workspace_root` = 工作区根
/// （清单落 `cadence/analysis-docs/`）。失败以 `Err` 上抛，由调用方决定是否
/// 阻断启动（本能力的调用方只打日志）。
pub(crate) fn run_startup_governance(
    paths: &ProductAppPaths,
    workspace_root: &Path,
) -> Result<StartupGovernanceOutcome, ProductStoreError> {
    // ① 巡检（只读、零副作用）。
    let report = patrol_state_root(paths)?;
    let token = findings_token(&report.findings);

    // ② 自动销账（白名单 + 证据门不变；无人工令牌环节）。
    let cleanup = run_cleanup(paths, &CleanupRequest::automatic());

    // ③ 清单写出（前三节 = 销账前现场；「归零自证」节 = 销账后现场；
    //    销账失败也写出，模式行如实标注失败原因）。
    let report_path = report_path(workspace_root);
    let status = match &cleanup {
        Ok(outcome) => CleanupReportStatus::Automatic {
            deleted_count: outcome.deleted_count,
        },
        Err(error) => CleanupReportStatus::AutomaticFailed {
            reason: error.to_string(),
        },
    };
    let content = render_markdown_report(
        &report,
        &token,
        &chrono::Utc::now().to_rfc3339(),
        &status,
        self_proof_of(&cleanup).as_ref(),
    );
    write_markdown_report(&report_path, &content)?;

    let outcome = cleanup?;
    Ok(StartupGovernanceOutcome {
        report_path,
        findings_token: token,
        dangling_count: report.findings.len(),
        undetermined_count: report.undetermined.len(),
        deleted_count: outcome.deleted_count,
        unzeroed: outcome.unzeroed,
        unzeroed_by_type: outcome.zeroed_by_type,
        residual: outcome.residual,
        undetermined: outcome.undetermined,
        governance_complete: outcome.governance_complete,
    })
}

/// 销账成功时的清单自证数据视图；销账失败（无自证现场）→ `None`。
fn self_proof_of(
    cleanup: &Result<CleanupOutcome, ProductStoreError>,
) -> Option<GovernanceSelfProof> {
    let outcome = cleanup.as_ref().ok()?;
    Some(GovernanceSelfProof {
        unzeroed: outcome.unzeroed.clone(),
        unzeroed_by_type: outcome.zeroed_by_type.clone(),
        residual_count: outcome.residual.len(),
        undetermined: outcome.undetermined.clone(),
        governance_complete: outcome.governance_complete,
    })
}

/// 巡检清单落点：`<workspace>/cadence/analysis-docs/<运行日>_分析报告_悬空引用巡检清单_v1.0.md`。
fn report_path(workspace_root: &Path) -> PathBuf {
    workspace_root
        .join(REPORT_DIR_SEGMENTS[0])
        .join(REPORT_DIR_SEGMENTS[1])
        .join(report_file_name(&run_day()))
}

/// 运行日（UTC 日期，形如 `2026-10-28`）。
fn run_day() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

#[cfg(test)]
#[path = "startup_tests.rs"]
mod startup_tests;
