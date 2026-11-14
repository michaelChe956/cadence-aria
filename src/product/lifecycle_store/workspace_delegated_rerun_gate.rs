//! C（陈旧人工门）：SC 委托返修接力接管接续路径后，关闭由
//! `recover_delegated_author_rerun_failure` 回落开出的陈旧人工门（durable CAS）。
//!
//! 现场：接力瞬时启动失败 ⇒ 回落人工门；随后接力真的跑成功——人工门没人关，
//! 会话永远停在 `waiting_for_human`（臂终态 `stopped_needs_human`，门节点仍
//! active）。本文件只负责**会话门开态证据**（status）的原子收口；门节点收口走
//! 引擎侧既有 `update_timeline_node` 链路（时间线与会话记录分属两个 durable
//! 文件，无法原子同事务）。

use chrono::Utc;

use super::*;
use crate::product::coding_attempt_store::locking::with_exclusive_lock;
use crate::product::json_store::{ProductStoreError, read_json, write_json};
use crate::product::models::{SingleCandidatePhase, WorkspaceSessionRecord, WorkspaceType};
use crate::product::work_item_plan_policy::{RepairReservationState, WorkItemPlanFlowKind};

impl LifecycleStore {
    /// C（陈旧人工门）：关闭「SC 委托返修接力启动失败」回落开出的陈旧人工门。
    ///
    /// 接力随后真正接管接续路径时，恢复路径开出的门已不是当前停点——必须关闭，
    /// 否则会话永远停在 `waiting_for_human`。
    ///
    /// CAS 判据与 `compare_and_save_human_gate_close` 同族（锁内重读 stored、
    /// 整记录比对，漂移即 `Conflict` 上抛——不覆盖并发写、不吞错）：仅受理 durable
    /// `WaitingForHuman` ＋ 单相位 `Generate` ＋ repair 预领已被消费
    /// （`state != Reserved`＝接力确实领走了 provider start）的 SingleCandidate
    /// WorkItemPlan 会话，锁内原子置回 `Running`（该门不写门快照，
    /// `WaitingForHuman` 本身即门开态证据，故只改 status，不动相位/预算/账本/
    /// 门快照）。
    pub fn compare_and_save_delegated_rerun_gate_close(
        &self,
        expected: &WorkspaceSessionRecord,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        let admission_scope = self.entity_scope_for_session(expected)?;
        self.with_optional_entity_live_write(admission_scope.as_ref(), move || {
            if expected.workspace_type != WorkspaceType::WorkItemPlan
                || expected.flow_kind != WorkItemPlanFlowKind::SingleCandidate
                || expected.status
                    != crate::product::models::WorkspaceSessionStatus::WaitingForHuman
                || expected.single_candidate_phase != Some(SingleCandidatePhase::Generate)
                || !expected
                    .repair_reservation
                    .as_ref()
                    .is_some_and(|reservation| {
                        reservation.state != RepairReservationState::Reserved
                    })
            {
                return Err(ProductStoreError::InvalidRecord {
                    kind: "delegated_rerun_gate_close",
                    reason: "session is not an open delegated-rerun human gate".to_string(),
                });
            }
            let session_path = self.find_workspace_session_path(&expected.id)?;
            with_exclusive_lock(&session_path, || {
                let mut stored: WorkspaceSessionRecord = read_json(&session_path)?;
                if stored != *expected {
                    return Err(ProductStoreError::Conflict {
                        kind: "workspace_session",
                        id: expected.id.clone(),
                    });
                }
                stored.status = crate::product::models::WorkspaceSessionStatus::Running;
                stored.updated_at = Utc::now().to_rfc3339();
                write_json(&session_path, &stored)?;
                Ok(stored)
            })
        })
    }
}
