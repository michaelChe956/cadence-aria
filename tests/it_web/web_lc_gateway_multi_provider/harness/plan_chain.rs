//! T23 plan 定稿链 durable 事实(F2/F3 修复轮)。
//!
//! 诊断 `/tmp/aria-bug45/t25/diag-round-contract.md` §2.2/§4:F2 与 F3 共用
//! **同一套 durable 取数**——plan Confirmed + 子 work item 落库,而不是任何
//! 格的 `capability_state`。反例(claude r2 全绿现场):plan resume 格是
//! `denied`(审计投影摘要 != 冻结投影摘要),但 typed confirm 成功、work
//! item 已落库、coding fresh confirmed——所以「plan 定稿」不能用格状态判。
//!
//! - F2:capture 轮落快照前与 resume 轮入口都用本口径 fail-closed;基线未
//!   定稿时 **不写 manifest**,该轮以 `snapshot_baseline_not_confirmed` 结束。
//! - F3:full/capture 轮的轮次覆盖断言先判本口径,再判逐格覆盖——否则
//!   plan 未定稿的下游缺格会被报成「coding 缺 fresh 执行格」(根因错指)。
//!
//! 大文件守卫:harness.rs 已按祖父条款登记 14966 行上限(棘轮只许删不许
//! 涨),capture 基线闸门与取数整体落本文件,harness 只留接线。

use std::path::Path;

use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::lifecycle_store::LifecycleStore;
use cadence_aria::product::models::IssueWorkItemPlanStatus;

use super::super::snapshot::{RunMode, SnapshotManifest};
use super::{LiveMatrixFailure, MatrixEnvironment, PROJECT_ID, fp, matrix_failure};

/// plan 定稿链的 durable 事实(产品 store 读到的事实,不是格状态)。
#[derive(Debug, Clone, Default)]
pub(crate) struct PlanChainDurability {
    pub(crate) plan_id: String,
    pub(crate) work_item_id: String,
    /// durable plan 状态(`confirmed`/`draft`/`change_requested`;读不到见
    /// `read_error`,不得伪造)。
    pub(crate) plan_status: String,
    /// 读取失败的真实原因(plan 不存在/store 错误;不伪造状态)。
    pub(crate) read_error: Option<String>,
}

impl PlanChainDurability {
    /// 定稿 = plan_id 在场 + plan Confirmed + 子 work item 落库(非空)。
    pub(crate) fn durably_confirmed(&self) -> bool {
        self.read_error.is_none()
            && self.plan_status == IssueWorkItemPlanStatus::Confirmed.as_str()
            && !self.plan_id.trim().is_empty()
            && !self.work_item_id.trim().is_empty()
    }

    /// 现场摘要(错误文本与证据落盘共用;不夹带推断)。
    pub(crate) fn describe(&self) -> String {
        match &self.read_error {
            Some(error) => format!(
                "plan 状态不可读(plan_id={:?},work_item_id={:?}):{error}",
                self.plan_id, self.work_item_id
            ),
            None => format!(
                "plan_id={:?} plan_status={:?} work_item_id={:?}",
                self.plan_id, self.plan_status, self.work_item_id
            ),
        }
    }
}

/// durable 计划定稿链取数(唯一取数点:capture 基线/续跑入口/轮次断言共用)。
pub(crate) fn plan_chain_durability(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
    work_item_id: &str,
) -> PlanChainDurability {
    let mut chain = PlanChainDurability {
        plan_id: plan_id.to_string(),
        work_item_id: work_item_id.to_string(),
        ..PlanChainDurability::default()
    };
    if plan_id.trim().is_empty() {
        chain.read_error = Some("plan_id 为空(plan prepare 未落 durable id)".to_string());
        return chain;
    }
    match lifecycle.get_issue_work_item_plan(project_id, issue_id, plan_id) {
        Ok(plan) => chain.plan_status = plan.status.as_str().to_string(),
        Err(error) => chain.read_error = Some(format!("{error:?}")),
    }
    chain
}

/// F2 基线闸门:快照基线只能建立在 durable 定稿的 plan 上。
///
/// capture 侧与 resume 侧共用——前者不写 manifest(见
/// [`MatrixEnvironment::capture_plan_baseline`]),后者在入口前置拒收,
/// 避免空 `work_item_id` 被 `build_from_snapshot` 塞成 `Some("")` 后一路
/// 跑到 coding 的 400(`work_item_plan_not_confirmed`)。
pub(crate) fn verify_snapshot_baseline(chain: &PlanChainDurability) -> Result<(), String> {
    if chain.durably_confirmed() {
        return Ok(());
    }
    Err(format!(
        "快照基线未确认(plan 未 durable 定稿:{});不写 manifest、不产生续跑基线\
         ——否则续跑轮必然在 coding 处以 work_item_plan_not_confirmed 级联失败",
        chain.describe()
    ))
}

/// F2(resume 入口前置拒收):基线 manifest 的空 `work_item_id` / 非
/// Confirmed plan 直接拒绝,不进 runtime/健康探测。
pub(crate) fn verify_snapshot_baseline_at_open(
    workspace_root: &Path,
    manifest: &SnapshotManifest,
) -> Result<(), String> {
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(workspace_root.join(".aria")));
    verify_snapshot_baseline(&plan_chain_durability(
        &lifecycle,
        PROJECT_ID,
        &manifest.issue_id,
        &manifest.plan_id,
        &manifest.work_item_id,
    ))
}

impl MatrixEnvironment {
    /// 取本环境当前的 durable 计划定稿链事实(轮次断言 F3 消费)。
    pub(super) fn durable_plan_chain(&self) -> PlanChainDurability {
        plan_chain_durability(
            &self.lifecycle,
            PROJECT_ID,
            &self.issue_id,
            &self.plan_id,
            self.work_item_id.as_deref().unwrap_or_default(),
        )
    }

    /// capture 轮在 plan 定稿的静止边界落快照。
    ///
    /// F2:基线未 durable 定稿时**不写 manifest**,直接以
    /// `snapshot_baseline_not_confirmed` 结束本轮(不再产出带空
    /// `work_item_id` 的假 `plan_confirmed` 基线)。快照 IO 失败仍按原口径
    /// 不阻断全链证据(capture 是运营加速面),如实 fp 留痕。
    pub(super) fn capture_plan_baseline(&mut self) -> Result<(), LiveMatrixFailure> {
        if !matches!(RunMode::from_env(), Ok(RunMode::CapturePlanSnapshot))
            || self.snapshot_id.is_some()
        {
            return Ok(());
        }
        if let Err(message) = verify_snapshot_baseline(&self.durable_plan_chain()) {
            return Err(matrix_failure(
                "snapshot_baseline_not_confirmed",
                format!("capture 轮 {message}"),
                Some("plan"),
            ));
        }
        let lc_root = self
            .app_paths
            .logical_codebase_record_root(PROJECT_ID, &self.lc_id);
        let audits =
            self.scan_session_audits(&self.prior_plan_session_id.clone().unwrap_or_default());
        let cli_version = audits
            .first()
            .map(|(_, record)| record.provider_version.clone())
            .unwrap_or_default();
        let input = super::super::snapshot::CaptureInput {
            provider_wire: &self.provider_wire,
            cli_version: &cli_version,
            project_id: PROJECT_ID,
            issue_id: &self.issue_id,
            lc_id: &self.lc_id,
            plan_id: &self.plan_id,
            work_item_id: self.work_item_id.as_deref().unwrap_or_default(),
            workspace_root: self.workspace_root_path(),
            aggregate_root: self.aggregate_root_path(),
            lc_root: &lc_root,
            policy_root: &self.canonical_root,
            source_evidence_root: &self.evidence_root,
        };
        match super::super::snapshot::capture_plan_snapshot(
            &self.evidence_root,
            &input,
            self.capture_snapshot_dir.as_deref(),
        ) {
            Ok(captured) => {
                self.snapshot_id = Some(captured.snapshot_id.clone());
                fp(
                    "snapshot_captured",
                    format_args!(
                        "snapshot={} dir={}",
                        captured.snapshot_id,
                        captured.directory.display()
                    ),
                );
            }
            Err(message) => {
                // 快照失败不伪造、不阻断全链证据:fp 留痕,本轮无快照
                //(capture 是运营加速面,矩阵证据语义不受影响)。
                fp("snapshot_capture_failed", format_args!("{message}"));
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// 负测(F2):基线未定稿一律拒绝,不得产出续跑基线。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod plan_chain_tests {
    use super::*;
    use crate::web_lc_gateway_multi_provider::snapshot::{
        CaptureInput, capture_plan_snapshot, snapshots_root,
    };

    fn chain(status: &str, work_item_id: &str) -> PlanChainDurability {
        PlanChainDurability {
            plan_id: "plan_0001".to_string(),
            work_item_id: work_item_id.to_string(),
            plan_status: status.to_string(),
            read_error: None,
        }
    }

    #[test]
    fn snapshot_baseline_requires_confirmed_plan_and_work_item() {
        assert_eq!(
            verify_snapshot_baseline(&chain("confirmed", "WI-001")),
            Ok(())
        );
        // 空 work_item_id(claude/pi r3 现场)→ 拒绝,指名根因。
        let empty_id = chain("confirmed", "  ");
        let error = verify_snapshot_baseline(&empty_id).expect_err("空 work_item_id 必须拒绝");
        assert!(
            error.contains("快照基线未确认") && error.contains("work_item_id"),
            "{error}"
        );
        // plan 未 Confirmed → 拒绝。
        assert!(verify_snapshot_baseline(&chain("draft", "WI-001")).is_err());
        assert!(verify_snapshot_baseline(&chain("change_requested", "WI-001")).is_err());
        // plan 不可读 → 拒绝(不得当已定稿)。
        let mut unreadable = chain("confirmed", "WI-001");
        unreadable.plan_status.clear();
        unreadable.read_error = Some("NotFound".to_string());
        assert!(verify_snapshot_baseline(&unreadable).is_err());
        // 空 plan_id → 拒绝。
        let mut no_plan = chain("confirmed", "WI-001");
        no_plan.plan_id.clear();
        assert!(verify_snapshot_baseline(&no_plan).is_err());
    }

    /// F2「不写 manifest」:空 work_item_id 时 capture 必须在建目录/写
    /// manifest 之前拒绝——快照根下不得出现任何快照目录。
    #[test]
    fn capture_refuses_empty_work_item_id_without_writing_manifest() {
        let temp = tempfile::tempdir().expect("tempdir");
        let evidence_root = temp.path().join("matrix-capture");
        let workspace = temp.path().join("workspace");
        let aggregate = temp.path().join("aggregate");
        let policy = temp.path().join("policy");
        for directory in [&workspace, &aggregate, &policy] {
            std::fs::create_dir_all(directory).expect("fixture dir");
        }
        let input = CaptureInput {
            provider_wire: "claude_code",
            cli_version: "claude 2.1.283",
            project_id: PROJECT_ID,
            issue_id: "issue_0001",
            lc_id: "lc_0001",
            plan_id: "plan_0001",
            work_item_id: "",
            workspace_root: &workspace,
            aggregate_root: &aggregate,
            lc_root: &policy,
            policy_root: &policy,
            source_evidence_root: &evidence_root,
        };
        let error = capture_plan_snapshot(&evidence_root, &input, None)
            .err()
            .expect("空 work_item_id 不得落快照基线");
        assert!(error.contains("work_item_id"), "{error}");
        let root = snapshots_root(&evidence_root);
        assert!(!root.exists(), "拒绝时不得创建快照目录({})", root.display());
    }
}
