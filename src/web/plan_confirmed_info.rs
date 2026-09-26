//! P1 WIGA Task 8：成功 publication/compile + durable Confirmed 的只读确认
//! 信息投影（REQ-WIGA-07）。
//!
//! 不建通知表——事实只从 enrollment 精确绑定的 session/plan/compile 事务/
//! publication provenance 派生；approve 点击、compile Failed/RecoveryRequired、
//! 缺 provenance、未 Confirmed 一律 `None`，引用/文件不可读显式上抛错误，
//! 绝不报告成功。稳定 key `plan_confirmed:{plan_id}:{compile_id}`，时间取
//! 成功事务 `committed_at`（不取会变的 `updated_at`）。

use crate::product::app_paths::ProductAppPaths;
use crate::product::issue_automation_store::IssueAutomationStore;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::automation::IssueAutomationEnrollment;
use crate::product::models::lifecycle::IssueWorkItemPlanStatus;
use crate::product::models::outline::{WorkItemPlanCompileStatus, WorkItemPlanCommitState};
use crate::product::models::{SingleCandidatePhase, WorkspaceSessionStatus};
use crate::product::work_item_plan_policy::RunPolicy;
use crate::product::work_item_plan_source_store::{SourceStoreScope, WorkItemPlanSourceStore};
use crate::product::work_item_plan_store::WorkItemPlanStore;

/// 驾驶舱只读分区的固定文案：只称「Work Item Plan 已确认」，不称 coding
/// 已完成或全交付（P1 无 §3/§4 事实）。
pub const PLAN_CONFIRMED_INFO_TITLE: &str = "Work Item Plan 已确认";

/// 只读确认信息 DTO（`IssueLifecycleResponse.plan_confirmed_info` 条目）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PlanConfirmedInfoDto {
    pub key: String,
    pub plan_id: String,
    pub session_id: String,
    pub occurred_at: String,
    pub title: String,
}

/// 从 durable 事实派生绑定 plan 的成功确认信息；任一成功判据不满足返回
/// `None`，读取失败原样上抛（不吞为 off）。
pub fn plan_confirmed_info(
    paths: &ProductAppPaths,
    enrollment: &IssueAutomationEnrollment,
) -> Result<Option<PlanConfirmedInfoDto>, ProductStoreError> {
    // 仅当前 enabled 精确绑定 enrollment 派生；禁用不是假成功。
    if !enrollment.enabled {
        return Ok(None);
    }
    let (Some(plan_id), Some(session_id)) = (&enrollment.plan_id, &enrollment.session_id) else {
        return Ok(None);
    };
    let lifecycle = LifecycleStore::new(paths.clone());
    let session = lifecycle.get_workspace_session(session_id)?;
    if session.project_id != enrollment.project_id
        || session.issue_id != enrollment.issue_id
        || session.run_policy != RunPolicy::Interactive
        || session.status != WorkspaceSessionStatus::Confirmed
        || session.single_candidate_phase != Some(SingleCandidatePhase::Completed)
    {
        return Ok(None);
    }
    let plan = lifecycle.get_issue_work_item_plan(
        &enrollment.project_id,
        &enrollment.issue_id,
        plan_id,
    )?;
    if plan.status != IssueWorkItemPlanStatus::Confirmed {
        return Ok(None);
    }
    // 多成功事务时只认 session compile_reservation 精确绑定的那一条。
    let Some(reservation) = session.compile_reservation.as_ref() else {
        return Ok(None);
    };
    let compile_id = reservation.compile_id.clone();
    let tx = WorkItemPlanStore::new(paths.clone()).get_compile_transaction(
        &enrollment.project_id,
        &enrollment.issue_id,
        plan_id,
        &compile_id,
    )?;
    if tx.status != WorkItemPlanCompileStatus::Committed
        || tx.plan_commit_state != WorkItemPlanCommitState::Committed
        || tx.step_cursor != "committed"
        || tx.committed_at.is_none()
    {
        return Ok(None);
    }
    // tx 的 provenance ref 必须与 session 的 ref 一致，且源文件可加载验证；
    // provenance.id 即 reservation.compile_id（ref 尾段），与绑定事务互证。
    let (Some(tx_provenance_ref), Some(session_provenance_ref)) = (
        tx.publication_provenance_ref.as_deref(),
        session.publication_provenance_ref.as_deref(),
    ) else {
        return Ok(None);
    };
    if tx_provenance_ref != session_provenance_ref {
        return Ok(None);
    }
    let scope = SourceStoreScope {
        project_id: enrollment.project_id.clone(),
        issue_id: enrollment.issue_id.clone(),
        plan_id: plan_id.clone(),
    };
    let source_store = WorkItemPlanSourceStore::new(paths.clone());
    let provenance = source_store
        .get_publication_provenance(&scope, session_provenance_ref)
        .map_err(source_store_error)?;
    if provenance.plan_id != *plan_id || provenance.id != compile_id {
        return Ok(None);
    }
    Ok(Some(PlanConfirmedInfoDto {
        key: format!("plan_confirmed:{plan_id}:{compile_id}"),
        plan_id: plan_id.clone(),
        session_id: session_id.clone(),
        occurred_at: tx.committed_at.clone().unwrap_or_default(),
        title: PLAN_CONFIRMED_INFO_TITLE.to_string(),
    }))
}

/// issue lifecycle 的 additive 投影入口：只读派生当前 enabled 绑定 enrollment
/// 的确认信息（0 或 1 条）；enrollment 读取失败按 HTTP 显式错误传播。
pub fn issue_plan_confirmed_info(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
) -> Result<Vec<PlanConfirmedInfoDto>, ProductStoreError> {
    let enrollment = IssueAutomationStore::new(paths.clone()).get(project_id, issue_id)?;
    let Some(enrollment) = enrollment else {
        return Ok(Vec::new());
    };
    Ok(plan_confirmed_info(paths, &enrollment)?.into_iter().collect())
}

/// SourceStoreError → ProductStoreError：NotFound 保留语义，其余以 Io 文本
/// 上抛（引用/文件不可读不得报告成功）。
fn source_store_error(
    error: crate::product::work_item_plan_source_store::SourceStoreError,
) -> ProductStoreError {
    ProductStoreError::Io(format!("plan publication provenance unreadable: {}", error.code()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::wiga_gate_fixture::EnrolledGateFixture;
    use crate::web::wiga_gate_fixture::{ISSUE_ID, PROJECT_ID};
    use tower::ServiceExt;

    struct CompiledPlanFixture {
        gate: EnrolledGateFixture,
    }

    impl CompiledPlanFixture {
        async fn new() -> Self {
            Self {
                gate: EnrolledGateFixture::new().await,
            }
        }

        fn bound_enrollment(&self) -> IssueAutomationEnrollment {
            IssueAutomationStore::new(self.gate.inner.paths.clone())
                .get(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .unwrap()
        }

        fn paths(&self) -> &ProductAppPaths {
            &self.gate.inner.paths
        }

        async fn approve_and_persist_failed_compile(&mut self) {
            self.gate.fail_compile_after_human_approve().await;
        }

        /// 人工 CompileRecovery Continue + Approve 关门（fixture 共享面）。
        async fn recover_and_commit_compile(&mut self) {
            self.gate.recover_and_confirm_compile().await;
        }
    }

    #[tokio::test]
    async fn plan_confirmed_info_requires_published_compile_and_confirmed_session() {
        let mut fixture = Box::new(CompiledPlanFixture::new().await);
        let enrollment = fixture.bound_enrollment();
        assert!(
            plan_confirmed_info(fixture.paths(), &enrollment)
                .unwrap()
                .is_none(),
            "未编译/未确认不得产生成功信息"
        );
        fixture.approve_and_persist_failed_compile().await;
        assert!(
            plan_confirmed_info(fixture.paths(), &enrollment)
                .unwrap()
                .is_none(),
            "failpoint 失败 compile（门保持开启）不得产生成功信息"
        );
        fixture.recover_and_commit_compile().await;
        let first = plan_confirmed_info(fixture.paths(), &enrollment)
            .unwrap()
            .expect("恢复并确认后必须有且仅有一条成功信息");
        let second = plan_confirmed_info(fixture.paths(), &fixture.bound_enrollment())
            .unwrap()
            .expect("重复读取幂等");
        assert_eq!(first.key, second.key);
        assert_eq!(first.occurred_at, second.occurred_at);
        assert!(first.key.contains(&first.plan_id));
        assert!(first.key.contains("plan_confirmed:"));
        assert_eq!(first.session_id, fixture.gate.session_id);
        assert_eq!(first.title, PLAN_CONFIRMED_INFO_TITLE);
        assert!(!first.occurred_at.is_empty(), "occurred_at 取 tx.committed_at");
    }

    /// 禁用 enrollment 后历史成功不投影为当前信息（禁用不是假成功）。
    #[tokio::test]
    async fn plan_confirmed_info_hidden_after_enrollment_disabled() {
        let mut fixture = Box::new(CompiledPlanFixture::new().await);
        fixture.approve_and_persist_failed_compile().await;
        fixture.recover_and_commit_compile().await;
        let enrollment = fixture.bound_enrollment();
        assert!(
            plan_confirmed_info(fixture.paths(), &enrollment)
                .unwrap()
                .is_some()
        );
        let disabled = IssueAutomationEnrollment {
            enabled: false,
            ..enrollment
        };
        assert!(
            plan_confirmed_info(fixture.paths(), &disabled)
                .unwrap()
                .is_none(),
            "禁用后不得把历史成功当当前信息"
        );
    }
}
