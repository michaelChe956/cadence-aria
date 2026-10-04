//! Task 10a:四家 LC 五阶段真实矩阵 harness(RED 骨架——类型与校验面冻结,
//! `validate_against`/`run_provider_matrix` 待实现,结构测试据真实红点驱动)。
//!
//! 冻结契约(计划 Task 10 Interfaces,454 行):
//! - `LiveLcGatewayHarness::run_provider_matrix(provider: ProviderName,
//!   evidence_root: &Path) -> Result<LiveMatrixEvidence, LiveMatrixFailure>`;
//! - `LiveMatrixEvidence`(Task 10 唯一定义)与 `EvidenceCell`,格键 =
//!   `provider/exact_version/stage/entrypoint/fresh_or_resume`;
//! - `entrypoint` 枚举 `workspace_streaming_plan/split`(WS streaming 主入口,
//!   `start_work_item_plan_author` caller 链)与 `split_sync`
//!   (`WorkItemSplitEngine::generate/generate_revision` 所代表的 sync 对照栈)。

use std::path::{Path, PathBuf};

use cadence_aria::product::models::ProviderName;

/// 真实现场开关:缺失时 `#[ignore]` 测试必须失败而非静默 return 成功。
pub(crate) const LC_GATEWAY_E2E_SWITCH: &str = "LC_GATEWAY_E2E";

/// Plan/split 的 WS streaming 主入口(`start_work_item_plan_author` caller 链)。
pub(crate) const ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT: &str = "workspace_streaming_plan/split";
/// Plan/split 的 sync 对照入口(`WorkItemSplitEngine::generate/generate_revision`)。
pub(crate) const ENTRYPOINT_SPLIT_SYNC: &str = "split_sync";

/// 五阶段顺序固定:Story、Design、Plan、Coding、Review。
pub(crate) const STAGE_ORDER: [&str; 5] = ["story", "design", "plan", "coding", "review"];

/// fresh/resume 两格(键的最后一维)。
pub(crate) const FRESH: &str = "fresh";
pub(crate) const RESUME: &str = "resume";

/// 单格证据(键 + 断言组字段 + 每格证据形态要素)。
///
/// 字段名与计划 Step 1 断言组(459-468 行)逐字对应;`serde` 形态用于
/// `cell.json` 落盘,敏感 token/API key/home 无关 trust 条目不落盘。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EvidenceCell {
    // ---- 键(provider/exact_version/stage/entrypoint/fresh_or_resume) ----
    pub provider: ProviderName,
    pub exact_version: String,
    pub stage: String,
    pub entrypoint: String,
    pub fresh_or_resume: String,
    // ---- 断言组(459-468)字段 ----
    pub process_cwd: PathBuf,
    pub target: PathBuf,
    pub audit_projection_digest: String,
    pub frozen_projection_digest: String,
    pub native_resume_confirmed_id: Option<String>,
    pub requested_resume_id: Option<String>,
    pub argv_or_wire_capture_exists: bool,
    pub approval_and_tool_events_exist: bool,
    pub completed_product_artifact_exists: bool,
    pub run_ref: String,
    pub run_ref_is_unique_within_entrypoint: bool,
    // ---- 每格证据形态(cell.json 全要素) ----
    pub action: String,
    pub role: String,
    pub gateway_dialect: String,
    pub wire_dialect: String,
    pub native_session_id: String,
    pub workspace_session_id: String,
    pub argv: Vec<String>,
    pub capability_state: String,
    pub denied_reason: Option<String>,
}

impl EvidenceCell {
    /// 证据结构校验:缺字段/不一致记录必须拒绝(fail-closed,不以删格缩小验收)。
    ///
    /// 校验语义与 Step 1 断言组同源:exact version 非空、argv/wire 捕获存在、
    /// approval/tool 事件存在、完成产物存在、audit 摘要==冻结摘要、原生恢复
    /// 确认==请求 id、cwd==canonical root、target==成员 worktree、entrypoint
    /// 枚举合法、run_ref 在 entrypoint 内唯一、provider 与所选一致。
    pub(crate) fn validate_against(
        &self,
        expected_provider: &ProviderName,
        canonical_root: &Path,
        member_worktree: &Path,
    ) -> Result<(), String> {
        // RED 骨架:待实现——结构测试据缺字段记录的拒绝断言驱动真实实现。
        let _ = (expected_provider, canonical_root, member_worktree);
        Ok(())
    }

    /// `cell.json` 形态(键 + 断言组字段 + 全要素;敏感项不落盘)。
    pub(crate) fn to_cell_json(&self) -> serde_json::Value {
        serde_json::json!({
            "provider": self.provider,
            "exact_version": self.exact_version,
            "stage": self.stage,
            "entrypoint": self.entrypoint,
            "fresh_or_resume": self.fresh_or_resume,
            "process_cwd": self.process_cwd,
            "target": self.target,
            "audit_projection_digest": self.audit_projection_digest,
            "frozen_projection_digest": self.frozen_projection_digest,
            "native_resume_confirmed_id": self.native_resume_confirmed_id,
            "requested_resume_id": self.requested_resume_id,
            "argv_or_wire_capture_exists": self.argv_or_wire_capture_exists,
            "approval_and_tool_events_exist": self.approval_and_tool_events_exist,
            "completed_product_artifact_exists": self.completed_product_artifact_exists,
            "run_ref": self.run_ref,
            "run_ref_is_unique_within_entrypoint": self.run_ref_is_unique_within_entrypoint,
            "action": self.action,
            "role": self.role,
            "gateway_dialect": self.gateway_dialect,
            "wire_dialect": self.wire_dialect,
            "native_session_id": self.native_session_id,
            "workspace_session_id": self.workspace_session_id,
            "argv": self.argv,
            "capability_state": self.capability_state,
            "denied_reason": self.denied_reason,
        })
    }
}

/// 一格被证据结构校验拒绝时的稳定记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EvidenceCellRejection {
    pub stage: String,
    pub entrypoint: String,
    pub fresh_or_resume: String,
    pub reason: String,
}

/// 四家五阶段真实矩阵证据(Task 10 唯一定义)。
#[derive(Debug, Clone)]
pub(crate) struct LiveMatrixEvidence {
    pub(crate) provider: ProviderName,
    pub(crate) canonical_root: PathBuf,
    pub(crate) member_worktree: PathBuf,
    pub(crate) cells: Vec<EvidenceCell>,
    pub(crate) rejections: Vec<EvidenceCellRejection>,
}

impl LiveMatrixEvidence {
    pub(crate) fn canonical_root(&self) -> &Path {
        &self.canonical_root
    }

    pub(crate) fn member_worktree(&self) -> &Path {
        &self.member_worktree
    }

    /// 全部格子(Confirmed 与未 Confirmed 分列,不合并隐藏)。
    pub(crate) fn cells(&self) -> &[EvidenceCell] {
        &self.cells
    }

    /// 成功支持格:具备全部断言要素的 Confirmed 格。
    pub(crate) fn confirmed_cells(&self) -> Vec<&EvidenceCell> {
        self.cells
            .iter()
            .filter(|cell| cell.capability_state == "confirmed")
            .collect()
    }

    /// 缺证据格:保持 Unknown/Denied + reason,不以删格缩小验收范围。
    pub(crate) fn unconfirmed_cells(&self) -> Vec<&EvidenceCell> {
        self.cells
            .iter()
            .filter(|cell| cell.capability_state != "confirmed")
            .collect()
    }
}

/// harness 自身无法尝试矩阵(环境/前置失败)时的失败记录;
/// 阶段内失败不进入此类型,一律落格为 Unknown/Denied + reason。
#[derive(Debug, Clone)]
pub(crate) struct LiveMatrixFailure {
    pub(crate) reason_code: String,
    pub(crate) message: String,
    pub(crate) stage: Option<String>,
}

/// 四家五阶段真实矩阵 harness。
pub(crate) struct LiveLcGatewayHarness;

impl LiveLcGatewayHarness {
    pub(crate) fn new() -> Self {
        Self
    }

    /// 对单个 provider 执行五阶段(Story→Design→Plan→Coding→Review)×
    /// fresh/resume 真实矩阵,逐格落证据到 `evidence_root`。
    ///
    /// 阶段顺序与两入口分列冻结(计划 Step 3):Plan 的
    /// `workspace_streaming_plan/split`(WS begin handle→bind sink→start→
    /// parse/complete)与 `split_sync`(sync bridge,无 native session
    /// contract 时 resume 记 Unknown + 零 spawn)各自独立 run_ref 与证据。
    pub(crate) async fn run_provider_matrix(
        &self,
        provider: ProviderName,
        evidence_root: &Path,
    ) -> Result<LiveMatrixEvidence, LiveMatrixFailure> {
        // RED 骨架:待实现——四个 lcg_live_* 真实测试归 Step 4 现场执行,
        // 本体由结构测试红点驱动后在 GREEN 段落地。
        Err(LiveMatrixFailure {
            reason_code: "harness_not_implemented".to_string(),
            message: "Task 10a RED:run_provider_matrix 尚未实现".to_string(),
            stage: None,
        })
    }
}

impl Default for LiveLcGatewayHarness {
    fn default() -> Self {
        Self::new()
    }
}
