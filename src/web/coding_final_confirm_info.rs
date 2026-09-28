//! P2 WIGA Task 8（tasks.md §3.3 / REQ-WIGA-07 / R5）：durable FinalConfirm
//! 等待信息的后端只读投影。
//!
//! 不建通知表——事实只从「enrolled 且已认领的单 target group attempt」的
//! 持久状态派生：`WaitingForHuman ∧ FinalConfirm ∧
//! GroupFinalReadinessStatus::Complete（无 diagnostics、units 完整）∧ 同
//! attempt 的 FinalConfirm Pending 节点」才有「编码执行完成，待最终确认」；
//! `Completed` 是人工确认后的另一事实：复用**同 key/原 `started_at`** 把标题
//! 改「已最终确认」，不产生新提醒。稳定时间取 FinalConfirm 节点首次
//! `started_at`（readiness `created_at` 每次重写、不可用）。缺节点/identity
//! 不匹配/坏 snapshot fail-closed 跳过，读取错误原样上抛（不吞作空）。

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{
    CodingAttemptScope, CodingAttemptStatus, CodingExecutionStage, CodingStartOrigin,
    CodingTimelineNodeStatus, GroupFinalReadinessStatus,
};
use crate::product::json_store::ProductStoreError;

/// 等待态固定文案：不称「已完成全部交付」（该用语仅适用
/// `PlanGroupOverall::AllDelivered`，人工 Final Confirm 之前不代点）。
pub const CODING_FINAL_CONFIRM_WAITING_TITLE: &str = "编码执行完成，待最终确认";
/// 人工确认后的另一事实：同 key 改文案，不产生第二条提醒。
pub const CODING_FINAL_CONFIRM_CONFIRMED_TITLE: &str = "已最终确认";

/// 只读 FinalConfirm 信息 DTO（`IssueLifecycleResponse.coding_final_confirm_info`
/// 条目；Task 9 TS 定义同 snake_case 字段）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CodingFinalConfirmInfoDto {
    pub key: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub attempt_id: String,
    pub occurred_at: String,
    pub title: String,
    pub final_confirmed: bool,
}

/// 枚举同 issue 下有 durable 首启认领（origin=Enrolled）的 group attempts，
/// 从 readiness snapshot 与 FinalConfirm timeline 节点推导等待/已确认信息；
/// 读取错误传播，判据不满足的 attempt fail-closed 跳过。
pub fn issue_coding_final_confirm_info(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
) -> Result<Vec<CodingFinalConfirmInfoDto>, ProductStoreError> {
    let store = CodingAttemptStore::new(paths.clone());
    let attempts = store.list_attempts_for_issue(project_id, issue_id)?;
    let mut infos = Vec::new();
    for attempt in attempts {
        if attempt.scope != CodingAttemptScope::WorkItemGroup {
            continue;
        }
        // 已认领的自动单 target attempt：claim.origin=Enrolled 是 enrollment
        // 精确授权首启的 durable 事实；未认领的 Manual attempt 不冒充自动完成。
        let Some(claim) = attempt.start_claim.as_ref() else {
            continue;
        };
        if !matches!(claim.origin, CodingStartOrigin::Enrolled { .. }) {
            continue;
        }
        let Some(plan_id) = attempt.work_item_group_id.as_deref() else {
            continue;
        };
        // readiness 必须完整：Complete、无 diagnostics、units 非空；坏
        // snapshot（identity/InvalidRecord）由 store 显式上抛，不吞作空。
        let Some(snapshot) = store.get_group_final_readiness_snapshot(&attempt)? else {
            continue;
        };
        if snapshot.status != GroupFinalReadinessStatus::Complete
            || !snapshot.diagnostics.is_empty()
            || snapshot.units.is_empty()
        {
            continue;
        }
        // 同 attempt 的 FinalConfirm 节点：等待态认 Pending，Completed 只认
        // 同一节点的 Completed + attempt 已人工 Completed。
        let nodes = store.get_timeline_nodes(project_id, issue_id, &attempt.id)?;
        let Some(node) = nodes
            .iter()
            .find(|node| node.stage == CodingExecutionStage::FinalConfirm)
        else {
            continue;
        };
        let (title, final_confirmed) = match (&attempt.status, &node.status) {
            (CodingAttemptStatus::WaitingForHuman, CodingTimelineNodeStatus::Pending) => {
                (CODING_FINAL_CONFIRM_WAITING_TITLE, false)
            }
            (CodingAttemptStatus::Completed, CodingTimelineNodeStatus::Completed) => {
                (CODING_FINAL_CONFIRM_CONFIRMED_TITLE, true)
            }
            // 等待态节点已被人工确认但 attempt 未落 Completed（或反之）等
            // 中间态：fail-closed 不投影，等 durable 事实一致。
            _ => continue,
        };
        infos.push(CodingFinalConfirmInfoDto {
            key: format!("coding_final_confirm:{}:{}", attempt.id, node.id),
            project_id: attempt.project_id.clone(),
            issue_id: attempt.issue_id.clone(),
            plan_id: plan_id.to_string(),
            attempt_id: attempt.id.clone(),
            // R5：稳定时间取节点首次 started_at，不取每次重写的 readiness
            // created_at，也不取 completed_at。
            occurred_at: node.started_at.clone(),
            title: title.to_string(),
            final_confirmed,
        });
    }
    Ok(infos)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::coding_models::{
        CodingAdmissionKind, GroupFinalReadinessSnapshot, GroupFinalReadinessUnit,
    };
    use crate::web::wiga_gate_fixture::{
        EnrolledGateFixture, ISSUE_ID, PROJECT_ID, complete_enrolled_group_waiting_for_final_confirm,
    };

    /// 主证据（R5）：Fake 真实业务链跑到 FinalConfirm 等待后，readiness 被
    /// 重复准备重写，投影 key/occurred_at 保持稳定且不误报已确认；人工
    /// `handle_final_confirm` 后同 key/同 occurred_at 改「已最终确认」。
    #[tokio::test]
    async fn coding_info_stays_stable_when_readiness_is_rewritten() {
        let fixture = complete_enrolled_group_waiting_for_final_confirm().await;
        let first = issue_coding_final_confirm_info(&fixture.inner.paths, PROJECT_ID, ISSUE_ID)
            .unwrap()
            .remove(0);
        assert_eq!(first.title, CODING_FINAL_CONFIRM_WAITING_TITLE);
        assert!(!first.final_confirmed);
        assert_eq!(first.project_id, PROJECT_ID);
        assert_eq!(first.issue_id, ISSUE_ID);
        assert_eq!(first.plan_id, fixture.plan_id());
        assert_eq!(first.attempt_id, fixture.attempt().id);

        fixture.prepare_group_final_confirm_again().await;
        let second = issue_coding_final_confirm_info(&fixture.inner.paths, PROJECT_ID, ISSUE_ID)
            .unwrap()
            .remove(0);
        assert_eq!(&first.key, &second.key);
        assert_eq!(&first.occurred_at, &second.occurred_at);
        assert!(!second.final_confirmed);
        assert_eq!(fixture.attempt().status, CodingAttemptStatus::WaitingForHuman);

        fixture.confirm_final_by_human().await;
        let completed = issue_coding_final_confirm_info(&fixture.inner.paths, PROJECT_ID, ISSUE_ID)
            .unwrap()
            .remove(0);
        assert_eq!(completed.key, second.key);
        assert_eq!(completed.occurred_at, second.occurred_at);
        assert_eq!(completed.title, CODING_FINAL_CONFIRM_CONFIRMED_TITLE);
        assert!(completed.final_confirmed);
    }

    /// 反例矩阵（fail-closed）：未认领的 Manual attempt 即便凑齐等待事实也
    /// 不投影；Incomplete/带 diagnostics 的 readiness 不投影；Completed 但
    /// FinalConfirm 节点未完成（非人工确认路径）不投影；缺 Pending 节点
    /// 不投影。
    #[test]
    fn coding_info_fail_closed_for_unproven_facts() {
        let root = tempfile::tempdir().expect("root");
        let paths = crate::product::app_paths::ProductAppPaths::new(root.path().join(".aria"));
        let store = CodingAttemptStore::new(paths.clone());

        // store 限定每 issue 唯一 group attempt：反例矩阵各落独立 issue，
        // 逐 issue 断言零投影。
        let manual = seed_claimed_group_attempt(
            &store,
            "manual-waiting",
            crate::product::coding_models::CodingStartOrigin::Manual,
            CodingAttemptStatus::WaitingForHuman,
            complete_snapshot("manual-waiting"),
            Some(CodingTimelineNodeStatus::Pending),
        );
        assert_eq!(manual.scope, CodingAttemptScope::WorkItemGroup);
        let _ = seed_claimed_group_attempt(
            &store,
            "incomplete-readiness",
            enrolled_origin(),
            CodingAttemptStatus::WaitingForHuman,
            incomplete_snapshot("incomplete-readiness"),
            Some(CodingTimelineNodeStatus::Pending),
        );
        let _ = seed_claimed_group_attempt(
            &store,
            "completed-no-node",
            enrolled_origin(),
            CodingAttemptStatus::Completed,
            complete_snapshot("completed-no-node"),
            None,
        );
        let _ = seed_claimed_group_attempt(
            &store,
            "completed-pending-node",
            enrolled_origin(),
            CodingAttemptStatus::Completed,
            complete_snapshot("completed-pending-node"),
            Some(CodingTimelineNodeStatus::Pending),
        );
        for issue_id in [
            "issue_manual-waiting",
            "issue_incomplete-readiness",
            "issue_completed-no-node",
            "issue_completed-pending-node",
        ] {
            let infos = issue_coding_final_confirm_info(&paths, "project_0001", issue_id).unwrap();
            assert!(
                infos.is_empty(),
                "unproven facts must not project coding final confirm info for {issue_id}: {infos:?}"
            );
        }
    }

    fn enrolled_origin() -> crate::product::coding_models::CodingStartOrigin {
        crate::product::coding_models::CodingStartOrigin::Enrolled {
            enrollment_id: "enrollment_0001".to_string(),
            policy_revision: 1,
            binding_version: None,
            target: None,
        }
    }

    fn base_unit(unit_id: &str) -> GroupFinalReadinessUnit {
        GroupFinalReadinessUnit {
            unit_id: unit_id.to_string(),
            logical_work_item_id: "work_item_0001".to_string(),
            unit_run_id: Some(format!("{unit_id}_run")),
            start_commit: Some("start_0001".to_string()),
            completion_commit: Some("commit_0001".to_string()),
            commit_shas: vec!["sha_0001".to_string()],
            diff_ref: "diff_0001".to_string(),
            code_review_report_id: Some("code_review_0001".to_string()),
            review_verdict: Some(crate::product::coding_models::ReviewVerdict::Approve),
            review_summary: Some("审核通过".to_string()),
            review_findings: Some(Vec::new()),
            handoff_revision_id: Some("handoff_revision_0001".to_string()),
            plan_revision_id: Some("plan_revision_0001".to_string()),
            ..Default::default()
        }
    }

    fn complete_snapshot(attempt_id: &str) -> GroupFinalReadinessSnapshot {
        GroupFinalReadinessSnapshot {
            attempt_id: attempt_id.to_string(),
            status: GroupFinalReadinessStatus::Complete,
            units: vec![base_unit("coding_unit_0001")],
            diagnostics: Vec::new(),
            created_at: "2026-09-27T00:00:00Z".to_string(),
        }
    }

    fn incomplete_snapshot(attempt_id: &str) -> GroupFinalReadinessSnapshot {
        GroupFinalReadinessSnapshot {
            attempt_id: attempt_id.to_string(),
            status: GroupFinalReadinessStatus::Incomplete,
            units: Vec::new(),
            // store 校验：Incomplete 快照必须携带 diagnostics（说明缺口）。
            diagnostics: vec![crate::product::coding_models::GroupFinalReadinessDiagnostic {
                unit_id: None,
                kind: crate::product::coding_models::GroupFinalReadinessDiagnosticKind::CodeReviewMissing,
                message: "units not complete".to_string(),
            }],
            created_at: "2026-09-27T00:00:00Z".to_string(),
        }
    }

    /// 构造带 durable claim 的 group attempt 及其 readiness/节点事实；
    /// 状态经测试写入口直落（反例矩阵只测投影判据，不重跑业务链）。
    fn seed_claimed_group_attempt(
        store: &CodingAttemptStore,
        attempt_key: &str,
        origin: crate::product::coding_models::CodingStartOrigin,
        status: CodingAttemptStatus,
        snapshot: GroupFinalReadinessSnapshot,
        node_status: Option<CodingTimelineNodeStatus>,
    ) -> crate::product::coding_models::CodingExecutionAttempt {
        let issue_id = format!("issue_{attempt_key}");
        let created = store
            .create_group_attempt(crate::product::coding_attempt_store::CreateGroupCodingAttemptInput {
                project_id: "project_0001".to_string(),
                issue_id: issue_id.clone(),
                plan_id: "work_item_plan_0001".to_string(),
                current_work_item_id: "work_item_0001".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/issues/issue_0001".to_string(),
                worktree_path: None,
                provider_config_snapshot: crate::web::workspace_ws_types::ProviderConfigSnapshot {
                    author: crate::product::models::ProviderName::Fake,
                    reviewer: Some(crate::product::models::ProviderName::Fake),
                    review_rounds: 1,
                    permission_modes: Default::default(),
                },
                target_snapshot: None,
                max_auto_rework: 1,
                start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
            })
            .expect("create group attempt");
        // 反例矩阵以固定 id 落盘（`write_coding_attempt_for_test` 支持），
        // 与 snapshot.attempt_id/节点 attempt_id 保持一致。
        let mut attempt = created.clone();
        attempt.id = attempt_key.to_string();
        attempt.issue_id = issue_id;
        attempt.scope = CodingAttemptScope::WorkItemGroup;
        attempt.admission_kind = CodingAdmissionKind::ScAdvance;
        attempt.work_item_group_id = Some("work_item_plan_0001".to_string());
        attempt.status = status.clone();
        attempt.stage = CodingExecutionStage::FinalConfirm;
        attempt.start_claim = Some(crate::product::coding_models::CodingStartClaim {
            command_id: format!("claim-{attempt_key}"),
            origin,
            phase: crate::product::coding_models::CodingStartPhase::ProviderMayHaveStarted,
            claimed_at: "2026-09-27T00:00:00Z".to_string(),
        });
        store
            .delete_attempt(&created.project_id, &created.issue_id, &created.id)
            .expect("drop generated id record");
        store
            .write_coding_attempt_for_test(&attempt)
            .expect("seed attempt record");
        store
            .write_group_final_readiness_snapshot(&attempt, &snapshot)
            .expect("seed readiness snapshot");
        if let Some(node_status) = node_status {
            store
                .save_timeline_node(
                    &attempt,
                    crate::product::coding_models::CodingTimelineNode {
                        id: format!("{attempt_key}-node"),
                        attempt_id: attempt.id.clone(),
                        stage: CodingExecutionStage::FinalConfirm,
                        title: "等待人工最终确认".to_string(),
                        status: node_status,
                        agent_role: None,
                        summary: None,
                        started_at: "2026-09-27T03:20:00Z".to_string(),
                        completed_at: None,
                        artifact_refs: Vec::new(),
                    },
                )
                .expect("seed final confirm node");
        }
        attempt
    }

    /// P1 plan info 不被 coding 事实覆盖：同 issue 两投影并存、key 前缀不同。
    #[tokio::test]
    async fn coding_info_does_not_shadow_plan_confirmed_info() {
        let fixture = complete_enrolled_group_waiting_for_final_confirm().await;
        let paths = fixture.inner.paths.clone();
        let coding = issue_coding_final_confirm_info(&paths, PROJECT_ID, ISSUE_ID).unwrap();
        let plan =
            crate::web::plan_confirmed_info::issue_plan_confirmed_info(&paths, PROJECT_ID, ISSUE_ID)
                .unwrap();
        assert_eq!(coding.len(), 1);
        assert!(coding[0].key.starts_with("coding_final_confirm:"));
        assert!(
            plan.iter().all(|info| info.key.starts_with("plan_confirmed:")),
            "plan info keys must stay in their own namespace: {plan:?}"
        );
    }
}
