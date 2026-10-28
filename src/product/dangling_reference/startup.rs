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
//!    （REQ-DRG-11）。清单记录**本次巡检（销账前现场）**的发现集合，便于人核对
//!    「这次看到了什么、删了什么」。
//!
//! **失败口径**：本入口的失败一律以 `Err` 上抛；调用方（服务启动序列）**只打
//! 日志、不阻断启动**。清单在销账失败时**仍然写出**（销账前的现场），保证
//! 「失败也可事后查」。

use std::path::{Path, PathBuf};

use crate::product::app_paths::ProductAppPaths;
use crate::product::dangling_reference::cleanup::{CleanupRequest, run_cleanup};
use crate::product::dangling_reference::patrol::patrol_state_root;
use crate::product::dangling_reference::report::{
    findings_token, render_markdown_report, report_file_name, write_markdown_report,
};
use crate::product::json_store::ProductStoreError;

/// 清单落点的相对目录段（`<workspace>/cadence/analysis-docs/`）。
const REPORT_DIR_SEGMENTS: [&str; 2] = ["cadence", "analysis-docs"];

/// 启动期治理的一次结果（供启动序列打日志 / 核对）。
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

    // ③ 清单写出（销账前现场；销账失败也写出，便于事后查）。
    let report_path = report_path(workspace_root);
    let content = render_markdown_report(&report, &token, &chrono::Utc::now().to_rfc3339());
    write_markdown_report(&report_path, &content)?;

    let outcome = cleanup?;
    Ok(StartupGovernanceOutcome {
        report_path,
        findings_token: token,
        dangling_count: report.findings.len(),
        undetermined_count: report.undetermined.len(),
        deleted_count: outcome.deleted_count,
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
