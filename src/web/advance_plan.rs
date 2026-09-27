//! P2 Task 1（tasks.md §3.1）：单一 `AdvancePlan` 共用薄入口。
//!
//! 人工 WS 与自动编排共用 `WorkspaceEngine::handle_advance` 的唯一业务实现
//! （journal/per-target 判据不在此重复）；本服务只做 origin 归属与自动路径
//! 的 enrollment/身份/单 target fail-closed 预检，不启动 provider、不调
//! StartCoding（`advance_completed` 仅状态）。人工 origin 不按 enrollment
//! 擅自升级；Enrolled origin 必须与 durable enrollment 的 id/revision/plan/
//! session 精确一致且 Confirmed+Completed SC 才放行 engine。

use crate::product::advance_store::{AdvanceInput, AdvanceOutcome};
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::issue_automation_store::IssueAutomationStore;
use crate::product::json_store::validate_relative_id;
use crate::product::models::{SingleCandidatePhase, WorkspaceSessionStatus, WorkspaceType};
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::web::state::WebAppState;

/// advance 请求归属：Manual 走人工 WS 语义；Enrolled 是自动编排的精确
/// enrollment 身份（id + policy_revision 快照），两者不得互相冒充。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdvancePlanOrigin {
    Manual,
    Enrolled {
        enrollment_id: String,
        policy_revision: u64,
    },
}

/// 共用 advance 薄服务：`input` 按值传入，`origin` 决定 enrollment 预检。
pub async fn advance_plan(
    state: &WebAppState,
    input: AdvanceInput,
    origin: AdvancePlanOrigin,
) -> Result<AdvanceOutcome, String> {
    validate_relative_id(&input.command_id)
        .map_err(|error| format!("invalid advance command id: {error}"))?;

    let paths = ProductAppPaths::new(state.workspace_root.join(".aria"));
    let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(paths.clone());

    // 解析 plan 对应的 WorkItemPlan 会话（单候选一 plan 一会话；歧义即
    // fail-closed，不猜）。
    let session_record = lifecycle
        .list_workspace_sessions(&input.project_id, &input.issue_id)
        .map_err(|error| format!("list plan workspace sessions failed: {error}"))?
        .into_iter()
        .filter(|record| {
            record.workspace_type == WorkspaceType::WorkItemPlan
                && record.entity_id == input.plan_id
        })
        .max_by(|left, right| left.created_at.cmp(&right.created_at))
        .ok_or_else(|| {
            format!(
                "no workspace session bound to plan {} for advance",
                input.plan_id
            )
        })?;

    if let AdvancePlanOrigin::Enrolled {
        enrollment_id,
        policy_revision,
    } = &origin
    {
        // 自动身份精确复核：enabled + id/revision/plan/session 与 durable
        // enrollment 完全一致，固定 command_id（同 enrollment+plan）保证跨
        // 重启同一 durable replay 线。漂移即显式拒绝，不隐式新 command。
        let enrollment = IssueAutomationStore::new(paths.clone())
            .get(&input.project_id, &input.issue_id)
            .map_err(|error| format!("load enrollment for advance failed: {error}"))?
            .ok_or_else(|| "advance enrollment is missing".to_string())?;
        if !enrollment.enabled {
            return Err("advance enrollment is disabled".to_string());
        }
        if enrollment.enrollment_id != *enrollment_id {
            return Err(format!(
                "advance enrollment id mismatch: expected {enrollment_id}, actual {}",
                enrollment.enrollment_id
            ));
        }
        if enrollment.policy_revision != *policy_revision {
            return Err(format!(
                "advance enrollment policy revision mismatch: expected {policy_revision}, actual {}",
                enrollment.policy_revision
            ));
        }
        let bound_plan_id = enrollment
            .plan_id
            .as_deref()
            .ok_or_else(|| "advance enrollment has no bound plan".to_string())?;
        if bound_plan_id != input.plan_id {
            return Err(format!(
                "advance enrollment plan mismatch: expected plan {bound_plan_id}, got {}",
                input.plan_id
            ));
        }
        let bound_session_id = enrollment
            .session_id
            .as_deref()
            .ok_or_else(|| "advance enrollment has no bound session".to_string())?;
        if bound_session_id != session_record.id {
            return Err(format!(
                "advance enrollment session mismatch: expected {bound_session_id}, got {}",
                session_record.id
            ));
        }
        let expected_command_id =
            format!("wiga-advance-{enrollment_id}-{}", input.plan_id);
        if input.command_id != expected_command_id {
            return Err(format!(
                "enrolled advance command id must be the stable per-enrollment id \
                 {expected_command_id}"
            ));
        }
        // durable Confirmed + Completed SC 是自动 advance 的最低前置（成功
        // publication/compile 由该终态与 engine 的 compile/revision 检查共同
        // 保证）；未确认现场显式拒绝。
        if session_record.status != WorkspaceSessionStatus::Confirmed
            || session_record.single_candidate_phase != Some(SingleCandidatePhase::Completed)
        {
            return Err(format!(
                "enrolled advance requires confirmed single-candidate session, got status {:?} phase {:?}",
                session_record.status, session_record.single_candidate_phase
            ));
        }
        // 自动路径恰一 target fail-closed：engine 支持 split，但 enrolled 自动
        // 授权只对恰一 logical repository 生效（REQ-MTG-03），无唯一目标也拒。
        let revision_store = WorkItemRevisionStore::new(paths.clone());
        let lineage = revision_store
            .get_plan_lineage(&input.project_id, &input.issue_id, &input.plan_id)
            .map_err(|error| format!("load plan lineage failed: {error}"))?;
        let active_revision_id = lineage
            .active_revision_id
            .ok_or_else(|| "plan has no active revision for enrolled advance".to_string())?;
        let authoritative = CodingAttemptStore::new(paths.clone())
            .resolve_authoritative_group_plan_binding_for_revision(
                &input.project_id,
                &input.issue_id,
                &input.plan_id,
                &active_revision_id,
            )
            .map_err(|error| {
                format!("resolve authoritative group plan binding failed: {error}")
            })?;
        let grouped = crate::product::coding_attempt_store::units_by_target(&authoritative);
        if grouped.by_target.len() != 1 || !grouped.unattributed.is_empty() {
            return Err(format!(
                "enrolled advance requires exactly one logical target, got {} target(s) \
                 and {} unattributed unit(s)",
                grouped.by_target.len(),
                grouped.unattributed.len()
            ));
        }
    }

    // registry 命中：取得与 socket/编排器**同一** manager（生产路径）。
    // registry miss（隔离 fixture 等场景）：按 durable 记录构造裸 engine 执行
    // 同一段 `handle_advance`——不冷启 manager，避免 `ensure_workspace_context_message`
    // 重写会话消息的附带副作用；engine 侧身份校验兜底。
    let outcome = match state.workspace_sessions.peek(&session_record.id).await {
        Some(manager) => {
            let engine_arc = manager.engine();
            let mut engine = engine_arc.lock().await;
            engine.handle_advance(input).await?
        }
        None => {
            let full_record = lifecycle
                .get_workspace_session(&session_record.id)
                .map_err(|error| format!("load workspace session failed: {error}"))?;
            let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
            let mut engine = crate::product::workspace_engine::WorkspaceEngine::new_persistent(
                std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
                    state.workspace_root.join("advance-engine-checkpoints"),
                )),
                lifecycle,
                event_tx,
                crate::product::workspace_engine::WorkspaceSession::from_record(full_record),
            );
            engine.handle_advance(input).await?
        }
    };

    // 回放后仍核 target_attempts 不超过一条：engine 支持 split，enrolled
    // 自动路径不允许出现多 target 绑定结果。
    if let AdvancePlanOrigin::Enrolled { .. } = origin
        && let AdvanceOutcome::Replayed { record } = &outcome
        && record.target_attempts.len() > 1
    {
        return Err(format!(
            "enrolled advance replay produced {} target attempts; expected at most one",
            record.target_attempts.len()
        ));
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::advance_store::AdvanceOutcome;
    use crate::web::wiga_gate_fixture::{
        ISSUE_ID, PROJECT_ID, confirmed_enrolled_fixture,
    };

    /// P2 Task 1：Enrolled origin 经真实 Confirmed fixture advance 到稳定
    /// attempt——同 command_id 二次调用是 durable replay（同 attempt），不启
    /// 动任何 coding runner。
    #[tokio::test]
    async fn enrolled_advance_replays_one_ready_attempt_without_starting_runner() {
        let fixture = confirmed_enrolled_fixture().await;
        let enrollment = fixture.enrollment();
        assert_eq!(enrollment.project_id, PROJECT_ID);
        assert_eq!(enrollment.issue_id, ISSUE_ID);
        let input = AdvanceInput {
            command_id: format!(
                "wiga-advance-{}-{}",
                enrollment.enrollment_id,
                enrollment.plan_id.as_deref().unwrap()
            ),
            project_id: enrollment.project_id.clone(),
            issue_id: enrollment.issue_id.clone(),
            plan_id: enrollment.plan_id.clone().unwrap(),
        };
        let origin = AdvancePlanOrigin::Enrolled {
            enrollment_id: enrollment.enrollment_id.clone(),
            policy_revision: enrollment.policy_revision,
        };
        let first = advance_plan(&fixture.state, input.clone(), origin.clone())
            .await
            .unwrap();
        let second = advance_plan(&fixture.state, input, origin).await.unwrap();
        let first_id = match &first {
            AdvanceOutcome::Completed { attempt_id, .. } => attempt_id.as_str(),
            AdvanceOutcome::Replayed { record } => record.attempt_id.as_deref().unwrap(),
            AdvanceOutcome::Rejected { code, .. } => panic!("unexpected reject: {code}"),
        };
        let second_id = match &second {
            AdvanceOutcome::Completed { attempt_id, .. } => attempt_id.as_str(),
            AdvanceOutcome::Replayed { record } => record.attempt_id.as_deref().unwrap(),
            AdvanceOutcome::Rejected { code, .. } => panic!("unexpected reject: {code}"),
        };
        assert_eq!(first_id, second_id);
        assert_eq!(fixture.coding_attempts().len(), 1);
        assert_eq!(fixture.coding_runner_count(), 0);
    }

    /// 自动身份漂移（policy_revision/enrollment id/command id）与禁用 enrollment
    /// 都必须在 engine 前显式拒绝：零 coding attempt、零 runner。
    #[tokio::test]
    async fn enrolled_advance_rejects_identity_drift_and_disabled_enrollment() {
        let fixture = confirmed_enrolled_fixture().await;
        let enrollment = fixture.enrollment();
        let base_input = AdvanceInput {
            command_id: format!(
                "wiga-advance-{}-{}",
                enrollment.enrollment_id,
                enrollment.plan_id.as_deref().unwrap()
            ),
            project_id: enrollment.project_id.clone(),
            issue_id: enrollment.issue_id.clone(),
            plan_id: enrollment.plan_id.clone().unwrap(),
        };
        let drifted_revision = AdvancePlanOrigin::Enrolled {
            enrollment_id: enrollment.enrollment_id.clone(),
            policy_revision: enrollment.policy_revision + 1,
        };
        let error = advance_plan(&fixture.state, base_input.clone(), drifted_revision)
            .await
            .unwrap_err();
        assert!(error.contains("policy revision mismatch"), "{error}");
        assert_eq!(fixture.coding_attempts().len(), 0);

        let wrong_command = AdvanceInput {
            command_id: "wiga-advance-adhoc-command".to_string(),
            ..base_input.clone()
        };
        let error = advance_plan(
            &fixture.state,
            wrong_command,
            AdvancePlanOrigin::Enrolled {
                enrollment_id: enrollment.enrollment_id.clone(),
                policy_revision: enrollment.policy_revision,
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("stable per-enrollment id"), "{error}");
        assert_eq!(fixture.coding_attempts().len(), 0);

        // Disable 后旧 origin 不得再 advance（D2：禁用先胜未消费即失效）。
        let store = IssueAutomationStore::new(
            crate::product::app_paths::ProductAppPaths::new(
                fixture.state.workspace_root.join(".aria"),
            ),
        );
        store
            .compare_and_set(
                &enrollment.project_id,
                &enrollment.issue_id,
                Some(enrollment.policy_revision),
                crate::product::models::automation::EnrollmentWriteCommand::Disable,
            )
            .expect("disable enrollment");
        let error = advance_plan(
            &fixture.state,
            base_input,
            AdvancePlanOrigin::Enrolled {
                enrollment_id: enrollment.enrollment_id.clone(),
                policy_revision: enrollment.policy_revision,
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("disabled"), "{error}");
        assert_eq!(fixture.coding_attempts().len(), 0);
        assert_eq!(fixture.coding_runner_count(), 0);
    }

    /// Manual origin 走同一 engine 实现：enrolled advance 已存在的 plan 级
    /// record 对人工命令按 durable replay 返回同一 attempt（人工不升级、
    /// 不隐式新 command 偷跑第二个 attempt）。
    #[tokio::test]
    async fn manual_advance_replays_same_plan_attempt_after_enrolled_advance() {
        let fixture = confirmed_enrolled_fixture().await;
        let enrollment = fixture.enrollment();
        let enrolled_input = AdvanceInput {
            command_id: format!(
                "wiga-advance-{}-{}",
                enrollment.enrollment_id,
                enrollment.plan_id.as_deref().unwrap()
            ),
            project_id: enrollment.project_id.clone(),
            issue_id: enrollment.issue_id.clone(),
            plan_id: enrollment.plan_id.clone().unwrap(),
        };
        let enrolled_origin = AdvancePlanOrigin::Enrolled {
            enrollment_id: enrollment.enrollment_id.clone(),
            policy_revision: enrollment.policy_revision,
        };
        let first = advance_plan(&fixture.state, enrolled_input.clone(), enrolled_origin)
            .await
            .unwrap();
        let first_id = match &first {
            AdvanceOutcome::Completed { attempt_id, .. } => attempt_id.clone(),
            AdvanceOutcome::Replayed { record } => {
                record.attempt_id.clone().expect("replayed attempt id")
            }
            AdvanceOutcome::Rejected { code, .. } => panic!("unexpected reject: {code}"),
        };

        let manual = advance_plan(
            &fixture.state,
            AdvanceInput {
                command_id: "manual-advance-command-1".to_string(),
                ..enrolled_input
            },
            AdvancePlanOrigin::Manual,
        )
        .await
        .unwrap();
        match &manual {
            AdvanceOutcome::Completed { attempt_id, .. } => assert_eq!(*attempt_id, first_id),
            AdvanceOutcome::Replayed { record } => {
                assert_eq!(record.attempt_id.as_deref(), Some(first_id.as_str()))
            }
            AdvanceOutcome::Rejected { code, reason, .. } => {
                panic!("unexpected manual reject: {code} {reason}")
            }
        }
        assert_eq!(fixture.coding_attempts().len(), 1);
        assert_eq!(fixture.coding_runner_count(), 0);
    }
}
