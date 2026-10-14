use std::process::Command;

#[cfg(test)]
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum AdvanceInitializationFailpoint {
    RecordPersisted,
    JournalPrepared,
    GroupAttemptPersisted,
    AttemptPersisted,
    WorktreeBound,
    PlanBindingSaved,
    UnitsMaterialized,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdvanceInitializationFailpointAction {
    Crash,
    Error,
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdvanceInitializationFailpointMode {
    Crash,
    Error,
}

#[cfg(test)]
pub(super) fn maybe_fail_advance_initialization(
    input: &AdvanceInput,
    checkpoint: AdvanceInitializationFailpoint,
) -> Result<(), String> {
    let key = AdvanceInitializationFailpointKey {
        project_id: input.project_id.clone(),
        issue_id: input.issue_id.clone(),
        plan_id: input.plan_id.clone(),
        command_id: input.command_id.clone(),
        checkpoint,
    };
    let action = advance_initialization_failpoints()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&key)
        .map(|(_, action)| action);
    match action {
        Some(AdvanceInitializationFailpointAction::Crash) => {
            panic!("advance_initialization_failpoint:{checkpoint:?}");
        }
        Some(AdvanceInitializationFailpointAction::Error) => {
            Err(format!("advance_initialization_failpoint:{checkpoint:?}"))
        }
        None => Ok(()),
    }
}

#[cfg(not(test))]
pub(super) fn maybe_fail_advance_initialization(
    _input: &AdvanceInput,
    _checkpoint: AdvanceInitializationFailpoint,
) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
pub(crate) fn register_advance_initialization_failpoint(
    input: &AdvanceInput,
    checkpoint: AdvanceInitializationFailpoint,
    mode: AdvanceInitializationFailpointMode,
) -> AdvanceInitializationFailpointGuard {
    let key = AdvanceInitializationFailpointKey {
        project_id: input.project_id.clone(),
        issue_id: input.issue_id.clone(),
        plan_id: input.plan_id.clone(),
        command_id: input.command_id.clone(),
        checkpoint,
    };
    let registration_id =
        NEXT_ADVANCE_INITIALIZATION_FAILPOINT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let action = match mode {
        AdvanceInitializationFailpointMode::Crash => AdvanceInitializationFailpointAction::Crash,
        AdvanceInitializationFailpointMode::Error => AdvanceInitializationFailpointAction::Error,
    };
    let previous = advance_initialization_failpoints()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(key.clone(), (registration_id, action));
    assert!(
        previous.is_none(),
        "advance initialization failpoint already registered"
    );
    AdvanceInitializationFailpointGuard {
        key,
        registration_id,
    }
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct AdvanceInitializationFailpointKey {
    project_id: String,
    issue_id: String,
    plan_id: String,
    command_id: String,
    checkpoint: AdvanceInitializationFailpoint,
}

#[cfg(test)]
pub(crate) struct AdvanceInitializationFailpointGuard {
    key: AdvanceInitializationFailpointKey,
    registration_id: u64,
}

#[cfg(test)]
static ADVANCE_INITIALIZATION_FAILPOINTS: OnceLock<
    Mutex<
        std::collections::HashMap<
            AdvanceInitializationFailpointKey,
            (u64, AdvanceInitializationFailpointAction),
        >,
    >,
> = OnceLock::new();

#[cfg(test)]
static NEXT_ADVANCE_INITIALIZATION_FAILPOINT_ID: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

#[cfg(test)]
fn advance_initialization_failpoints() -> &'static Mutex<
    std::collections::HashMap<
        AdvanceInitializationFailpointKey,
        (u64, AdvanceInitializationFailpointAction),
    >,
> {
    ADVANCE_INITIALIZATION_FAILPOINTS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

#[cfg(test)]
impl Drop for AdvanceInitializationFailpointGuard {
    fn drop(&mut self) {
        let mut failpoints = advance_initialization_failpoints()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if failpoints
            .get(&self.key)
            .is_some_and(|(registration_id, _)| *registration_id == self.registration_id)
        {
            failpoints.remove(&self.key);
        }
    }
}

use crate::product::advance_store::AdvanceTargetAttemptBinding;
pub use crate::product::advance_store::{
    AdvanceInitializationPhase, AdvanceInput, AdvanceOutcome, AdvanceRecord, AdvanceStatus,
    AdvanceStore,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::target_snapshot::build_attempt_target_snapshot;
use crate::product::coding_attempt_store::{
    AuthoritativeGroupPlanBinding, CodingAttemptStore, CreateGroupCodingAttemptInput,
    units_by_target,
};
use crate::product::coding_models::CodingAdmissionKind;
use crate::product::coding_plan_run::CodingPlanRunStore;
use crate::product::entity_deletion_store::{
    DeletionEntityKind, DeletionPhase, EntityDeletionStore, EntityScope,
};
use crate::product::entity_operation_store::{EntityOperationError, EntityOperationStore};
use crate::product::issue_store::IssueStore;
use crate::product::json_store::ProductStoreError;
use crate::product::logical_codebase::{RepositoryRouting, resolve_issue_logical_codebase_id};
use crate::product::models::{IssueWorkItemPlanStatus, WorkspaceType};
use crate::product::repository_store::RepositoryStore;
use crate::product::work_item_plan_store::WorkItemPlanStore;
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::web::workspace_ws_types::ProviderConfigSnapshot;

use super::types::WorkspaceEngine;

/// D2.1d/H3（fix-cascade-deletion＋设计 2026-10-14 H3）：advance 真实入口
/// 的 owning Plan run lease。
///
/// 在 `manager.engine` 锁与任何 arbitration/对象锁**之前**取得（scope →
/// engine 锁固定序，plan §0.4），任务全生命周期持有——准入在持期间删除
/// 独占（Plan 删除/工作区重置）立即 Busy；内层经 `guard_round_admission`
/// 只读消费（墓碑复核＋执行轮退役围栏），不重取 scope、不在
/// engine/arbitration 锁内首次取 scope。
pub struct AdvancePlanAdmission {
    lease: crate::product::entity_operation_store::EntityRunLease,
}

impl AdvancePlanAdmission {
    /// 入口准入（try-shared＋锁内墓碑复核）：purging/deleted/争用由
    /// `EntityOperationStore::acquire_run` 在 scope 锁内精确分类。
    pub fn acquire(
        paths: &ProductAppPaths,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<Self, EntityOperationError> {
        let scope = EntityScope::new(project_id, issue_id, DeletionEntityKind::Plan, plan_id);
        scope.validate().map_err(EntityOperationError::Store)?;
        Ok(Self {
            lease: EntityOperationStore::new(paths.clone()).acquire_run(&scope)?,
        })
    }

    pub fn scope(&self) -> &EntityScope {
        self.lease.scope()
    }

    /// 已持准入守卫（消费外层 lease，不重取 scope）：purging/deleted 墓碑与
    /// 执行轮退役围栏（clearing/退役 command）先于 command_id/plan_id 旧幂
    /// 等命中。真实 IO/JSON 错误 fail-closed 上抛（`Store`），不当稳定拒绝。
    pub(crate) fn guard_round_admission(
        &self,
        paths: &ProductAppPaths,
        command_id: &str,
    ) -> Result<(), AdvanceRoundGuardFailure> {
        let tombstone = EntityDeletionStore::new(paths.clone())
            .get(self.lease.scope())
            .map_err(AdvanceRoundGuardFailure::Store)?;
        if let Some(record) = tombstone {
            return Err(AdvanceRoundGuardFailure::Round(match record.phase {
                DeletionPhase::Purging => AdvanceRoundRejection::PlanDeleting,
                DeletionPhase::Deleted => AdvanceRoundRejection::PlanDeleted,
            }));
        }
        let scope = self.lease.scope();
        if let Err(error) = CodingPlanRunStore::new(paths.clone()).verify_workspace_round_admission(
            &scope.project_id,
            &scope.issue_id,
            &scope.entity_id,
            command_id,
        ) {
            return Err(match error {
                ProductStoreError::Conflict { kind, .. }
                    if kind == "coding_plan_workspace_clearing" =>
                {
                    AdvanceRoundGuardFailure::Round(AdvanceRoundRejection::WorkspaceClearing)
                }
                ProductStoreError::Conflict { kind, .. } if kind == "coding_plan_run_retired" => {
                    AdvanceRoundGuardFailure::Round(AdvanceRoundRejection::CommandRetired)
                }
                other => AdvanceRoundGuardFailure::Store(other),
            });
        }
        Ok(())
    }
}

/// 稳定轮次拒绝（映射 `AdvanceOutcome::Rejected`，不发 AdvanceCompleted）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdvanceRoundRejection {
    PlanDeleting,
    PlanDeleted,
    WorkspaceClearing,
    CommandRetired,
}

impl AdvanceRoundRejection {
    /// 稳定 store kind（HTTP/REST 面与 H1 白名单同族）。
    pub(crate) fn stable_kind(self) -> &'static str {
        match self {
            Self::PlanDeleting => "entity_deleting",
            Self::PlanDeleted => "entity_deleted",
            Self::WorkspaceClearing => "coding_plan_workspace_clearing",
            Self::CommandRetired => "coding_plan_run_retired",
        }
    }

    /// WS advance 面稳定码（AdvanceRejected.code）。
    pub(crate) fn ws_code(self) -> &'static str {
        match self {
            Self::PlanDeleting => "ADVANCE_PLAN_DELETING",
            Self::PlanDeleted => "ADVANCE_PLAN_DELETED",
            Self::WorkspaceClearing => "ADVANCE_PLAN_WORKSPACE_CLEARING",
            Self::CommandRetired => "ADVANCE_PLAN_RUN_RETIRED",
        }
    }

    pub(crate) fn reason_text(self, plan_id: &str) -> String {
        match self {
            Self::PlanDeleting => format!(
                "entity_deleting: owning plan {plan_id} is being deleted; advance is rejected"
            ),
            Self::PlanDeleted => format!(
                "entity_deleted: owning plan {plan_id} has been deleted; advance is rejected"
            ),
            Self::WorkspaceClearing => format!(
                "coding_plan_workspace_clearing: the coding workspace round of plan {plan_id} is being cleared"
            ),
            Self::CommandRetired => format!(
                "coding_plan_run_retired: the command belongs to a retired workspace round of plan {plan_id}"
            ),
        }
    }

    pub(crate) fn into_rejected(self, plan_id: &str) -> AdvanceOutcome {
        AdvanceOutcome::Rejected {
            record: None,
            code: self.ws_code().to_string(),
            reason: self.reason_text(plan_id),
        }
    }
}

/// 守卫失败：`Round`＝稳定拒绝（Replayed/Completed 均不得发生）；`Store`＝
/// 真实 IO/JSON fail-closed（上抛，不吞错当拒绝）。
pub(crate) enum AdvanceRoundGuardFailure {
    Round(AdvanceRoundRejection),
    Store(ProductStoreError),
}

/// D2.1d：入口准入失败的稳定 Rejected（web advance_plan 与 engine 独立
/// 路径共用；争用/IO 与删除相位分码，不伪装成业务前置失败）。
pub fn advance_admission_rejected(error: EntityOperationError, plan_id: &str) -> AdvanceOutcome {
    let (code, reason) = match error {
        EntityOperationError::Deleting => (
            AdvanceRoundRejection::PlanDeleting.ws_code(),
            AdvanceRoundRejection::PlanDeleting.reason_text(plan_id),
        ),
        EntityOperationError::Deleted => (
            AdvanceRoundRejection::PlanDeleted.ws_code(),
            AdvanceRoundRejection::PlanDeleted.reason_text(plan_id),
        ),
        EntityOperationError::Busy => (
            "ADVANCE_PLAN_ADMISSION_CONTENDED",
            format!(
                "entity_busy: plan {plan_id} admission is contended by an exclusive deletion; retry"
            ),
        ),
        EntityOperationError::Store(error) => (
            "ADVANCE_PLAN_ADMISSION_FAILED",
            format!("entity_admission_failed: plan {plan_id} admission failed: {error}"),
        ),
    };
    AdvanceOutcome::Rejected {
        record: None,
        code: code.to_string(),
        reason,
    }
}

/// D2.1d：retry 面准入失败的稳定错误串（engine Err 通道；message 前缀携带
/// 稳定 store kind，D2.1a 同惯例）。
pub(crate) fn advance_admission_error(error: EntityOperationError, plan_id: &str) -> String {
    match error {
        EntityOperationError::Deleting
        | EntityOperationError::Deleted
        | EntityOperationError::Busy => {
            let stable = match error {
                EntityOperationError::Deleted => AdvanceRoundRejection::PlanDeleted.stable_kind(),
                EntityOperationError::Busy => "entity_busy",
                _ => AdvanceRoundRejection::PlanDeleting.stable_kind(),
            };
            format!("{stable}: owning plan {plan_id} is not admissible for advance retry")
        }
        EntityOperationError::Store(error) => {
            format!("entity_admission_failed: plan {plan_id} admission failed: {error}")
        }
    }
}

impl WorkspaceEngine {
    /// Runs only the first-request preflight. Record creation and group
    /// initialization belong to the following advance tasks; until then a valid
    /// request is deliberately rejected without durable side effects.
    ///
    /// D2.1d：独立 engine 路径同 guard——在未持任何 engine/arbitration 锁处
    /// 取 owning Plan run lease（session 身份）；web 入口已在 engine.lock 前
    /// 取得准入时不走本包装（直接调 `handle_advance_with_start_policy` 消费
    /// 同一 lease）。
    pub async fn handle_advance(&mut self, input: AdvanceInput) -> Result<AdvanceOutcome, String> {
        let paths = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?
            .app_paths();
        let admission = match AdvancePlanAdmission::acquire(
            &paths,
            &self.session.project_id,
            &self.session.issue_id,
            &self.session.entity_id,
        ) {
            Ok(admission) => admission,
            Err(error) => return Ok(advance_admission_rejected(error, &self.session.entity_id)),
        };
        self.handle_advance_with_start_policy(
            input,
            crate::product::coding_models::CodingStartRunPolicy::Manual,
            &admission,
        )
        .await
    }

    /// P2 Task 2：带首启策略的 advance——仅供 Task 1 自动路径在精确核验后
    /// 传入 AutoStartOnce；只在**新建单 target group journal** 时穿透
    /// `CreateGroupCodingAttemptInput.start_run_policy`，已有 journal replay
    /// 保留原 policy，不随当前 enrollment 变更覆盖。
    ///
    /// D2.1d/H3：`admission` 是入口在 engine.lock 前取得的 owning Plan run
    /// lease——purging/deleted 身份守卫与执行轮退役围栏先于 command_id/
    /// plan_id 旧幂等命中（删除/重置身份优先，不回放旧 Ready/旧 command）。
    pub async fn handle_advance_with_start_policy(
        &mut self,
        input: AdvanceInput,
        start_policy: crate::product::coding_models::CodingStartRunPolicy,
        admission: &AdvancePlanAdmission,
    ) -> Result<AdvanceOutcome, String> {
        if input.project_id != self.session.project_id
            || input.issue_id != self.session.issue_id
            || input.plan_id != self.session.entity_id
        {
            return Ok(AdvanceOutcome::Rejected {
                record: None,
                code: "ADVANCE_IDENTITY_MISMATCH".to_string(),
                reason: "advance identity does not match the workspace session".to_string(),
            });
        }

        let app_paths = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?
            .app_paths();
        // D2.1d/H3：purging/deleted 身份守卫与执行轮退役围栏先于下方
        // command_id/plan_id 旧幂等命中——旧 Ready record 不因幂等回放成
        // AdvanceCompleted 旧入口；真实 IO/JSON 错误 fail-closed 上抛。
        if let Err(failure) = admission.guard_round_admission(&app_paths, &input.command_id) {
            return match failure {
                AdvanceRoundGuardFailure::Round(rejection) => {
                    Ok(rejection.into_rejected(&input.plan_id))
                }
                AdvanceRoundGuardFailure::Store(error) => {
                    Err(format!("advance round admission check failed: {error}"))
                }
            };
        }
        let advance_store = AdvanceStore::new(app_paths.clone());

        // A completed/terminal record is an idempotent replay. An initializing
        // record must continue through the durable checkpoint path so a
        // restarted request resumes the same attempt rather than stopping at
        // the preflight facade.
        if let Some(record) = advance_store
            .get_advance_by_command_id(&input.project_id, &input.issue_id, &input.command_id)
            .map_err(|error| format!("load advance command record failed: {error}"))?
        {
            if record.plan_id != input.plan_id {
                return Ok(AdvanceOutcome::Rejected {
                    record: Some(record),
                    code: "ADVANCE_IDENTITY_MISMATCH".to_string(),
                    reason: "command_id is already bound to another plan".to_string(),
                });
            }
            if record.status != AdvanceStatus::Initializing {
                return Ok(AdvanceOutcome::Replayed { record });
            }
        }
        if let Some(record) = advance_store
            .get_advance_for_plan(&input.project_id, &input.issue_id, &input.plan_id)
            .map_err(|error| format!("load advance plan record failed: {error}"))?
            && record.status != AdvanceStatus::Initializing
        {
            return Ok(AdvanceOutcome::Replayed { record });
        }

        let lifecycle = self
            .lifecycle_store
            .as_ref()
            .expect("lifecycle store checked above");
        let plan = match lifecycle.get_issue_work_item_plan(
            &input.project_id,
            &input.issue_id,
            &input.plan_id,
        ) {
            Ok(plan) => plan,
            Err(error) => {
                return Ok(AdvanceOutcome::Rejected {
                    record: None,
                    code: "ADVANCE_PLAN_NOT_FOUND".to_string(),
                    reason: format!("load confirmed work item plan failed: {error}"),
                });
            }
        };
        if plan.status != IssueWorkItemPlanStatus::Confirmed {
            return Ok(AdvanceOutcome::Rejected {
                record: None,
                code: "ADVANCE_PLAN_NOT_CONFIRMED".to_string(),
                reason: "work item plan must be durably confirmed before advance".to_string(),
            });
        }

        let revision_store = WorkItemRevisionStore::new(app_paths.clone());
        let lineage = match revision_store.get_plan_lineage(
            &input.project_id,
            &input.issue_id,
            &input.plan_id,
        ) {
            Ok(lineage) => lineage,
            Err(error) => {
                return Ok(AdvanceOutcome::Rejected {
                    record: None,
                    code: "ADVANCE_PLAN_REVISION_MISSING".to_string(),
                    reason: format!("load active plan revision failed: {error}"),
                });
            }
        };
        let Some(active_revision_id) = lineage.active_revision_id.as_deref() else {
            return Ok(AdvanceOutcome::Rejected {
                record: None,
                code: "ADVANCE_PLAN_REVISION_MISSING".to_string(),
                reason: "confirmed work item plan has no active plan revision".to_string(),
            });
        };
        if lineage.active_amendment_id.is_some() {
            return Ok(AdvanceOutcome::Rejected {
                record: None,
                code: "ADVANCE_ACTIVE_PLAN_REVISION".to_string(),
                reason: "work item plan has an active amendment/revision".to_string(),
            });
        }

        let plan_store = WorkItemPlanStore::new(app_paths.clone());
        let active_compile = plan_store
            .list_compile_transactions(&input.project_id, &input.issue_id, &input.plan_id)
            .map_err(|error| format!("load plan compile transactions failed: {error}"))?
            .into_iter()
            .find(|transaction| {
                matches!(
                    transaction.status,
                    crate::product::models::WorkItemPlanCompileStatus::Preparing
                        | crate::product::models::WorkItemPlanCompileStatus::Validating
                        | crate::product::models::WorkItemPlanCompileStatus::Committing
                        | crate::product::models::WorkItemPlanCompileStatus::RecoveryRequired
                )
            });
        if let Some(transaction) = active_compile {
            return Ok(AdvanceOutcome::Rejected {
                record: None,
                code: "ADVANCE_ACTIVE_PLAN_COMPILE".to_string(),
                reason: format!("plan compile {} is still active", transaction.compile_id),
            });
        }

        let child_sessions = lifecycle
            .list_workspace_sessions(&input.project_id, &input.issue_id)
            .map_err(|error| format!("list plan child sessions failed: {error}"))?;
        let missing_child = plan.work_item_ids.iter().find(|work_item_id| {
            !child_sessions.iter().any(|session| {
                session.workspace_type == WorkspaceType::WorkItem
                    && session.entity_id == **work_item_id
            })
        });
        if let Some(work_item_id) = missing_child {
            return Ok(AdvanceOutcome::Rejected {
                record: None,
                code: "ADVANCE_CHILD_SESSION_MISSING".to_string(),
                reason: format!("work item child session is missing: {work_item_id}"),
            });
        }

        let coding_store = CodingAttemptStore::new(app_paths.clone());

        let authoritative = coding_store
            .resolve_authoritative_group_plan_binding_for_revision(
                &input.project_id,
                &input.issue_id,
                &input.plan_id,
                active_revision_id,
            )
            .map_err(|error| format!("resolve authoritative group plan binding failed: {error}"))?;
        if authoritative.units.is_empty() {
            return Ok(AdvanceOutcome::Rejected {
                record: None,
                code: "ADVANCE_GROUP_EMPTY".to_string(),
                reason: "confirmed work item plan has no authoritative coding units".to_string(),
            });
        }

        self.initialize_advance(
            input,
            advance_store,
            coding_store,
            authoritative,
            start_policy,
            admission,
        )
        .await
        .map_err(|error| error.to_string())
    }

    #[allow(clippy::too_many_arguments)]
    async fn initialize_advance(
        &mut self,
        input: AdvanceInput,
        advance_store: AdvanceStore,
        coding_store: CodingAttemptStore,
        authoritative: AuthoritativeGroupPlanBinding,
        start_policy: crate::product::coding_models::CodingStartRunPolicy,
        admission: &AdvancePlanAdmission,
    ) -> Result<AdvanceOutcome, String> {
        let result = self
            .initialize_advance_inner(
                input.clone(),
                advance_store.clone(),
                coding_store,
                authoritative,
                start_policy,
                admission,
            )
            .await;
        if let Err(error) = &result
            && let Ok(Some(record)) = advance_store.get_advance_by_command_id(
                &input.project_id,
                &input.issue_id,
                &input.command_id,
            )
        {
            // REQ-MTG-02（分流失败标记适配，D2.1）：多 target（journal 数 ≥2）下
            // error 落每个 per-target journal；单 target 保持原分支零变化。
            let split_journals = {
                let journals = CodingAttemptStore::new(advance_store.app_paths())
                    .list_group_initialization_journals_for_plan(
                        &record.project_id,
                        &record.issue_id,
                        &record.plan_id,
                    );
                match journals {
                    Ok(journals) if journals.len() >= 2 => journals,
                    _ => Vec::new(),
                }
            };
            for group_journal in &split_journals {
                let _ = CodingAttemptStore::new(advance_store.app_paths())
                    .mark_group_initialization_error(group_journal, error);
            }
            if let Ok(Some(journal)) = advance_store.get_advance_initialization(&record) {
                let _ = advance_store.mark_advance_initialization_error(&record, &journal, error);
            } else if let Some(group_journal) = split_journals.first() {
                // 分流早期失败（外层 journal 尚未建）：record 落 attempt 集身份
                // （attempt 集身份保留——per-target journal 的 attempt 全集回落）。
                let mut failed = record.clone();
                failed.status = AdvanceStatus::Failed;
                failed.error = Some(error.clone());
                failed.attempt_id = Some(group_journal.attempt.id.clone());
                failed.target_attempts = split_journals
                    .iter()
                    .filter_map(|journal| {
                        journal.attempt.target_snapshot.as_ref().map(|snapshot| {
                            AdvanceTargetAttemptBinding {
                                target_repository_id: snapshot.logical_repository_id.0.to_string(),
                                attempt_id: journal.attempt.id.clone(),
                            }
                        })
                    })
                    .collect();
                failed.updated_at = chrono::Utc::now().to_rfc3339();
                let _ = advance_store.update_record(&failed);
            } else if let Ok(group_journal) = CodingAttemptStore::new(advance_store.app_paths())
                .get_group_initialization(&record.project_id, &record.issue_id, &record.plan_id)
            {
                let _ = CodingAttemptStore::new(advance_store.app_paths())
                    .mark_group_initialization_error(&group_journal, error);
                let mut failed = record.clone();
                failed.status = AdvanceStatus::Failed;
                failed.error = Some(error.clone());
                failed.attempt_id = Some(group_journal.attempt.id);
                failed.updated_at = chrono::Utc::now().to_rfc3339();
                let _ = advance_store.update_record(&failed);
            } else if record.status == AdvanceStatus::Initializing {
                let mut failed = record.clone();
                failed.status = AdvanceStatus::Failed;
                failed.error = Some(error.clone());
                failed.updated_at = chrono::Utc::now().to_rfc3339();
                let _ = advance_store.update_record(&failed);
            }
        }
        result
    }
}

include!("advance_parts/initialize_inner.inc.rs");

impl WorkspaceEngine {
    fn advance_target_snapshot(
        paths: &crate::product::app_paths::ProductAppPaths,
        input: &AdvanceInput,
        authoritative: &AuthoritativeGroupPlanBinding,
    ) -> Result<Option<crate::product::coding_models::AttemptTargetSnapshot>, String> {
        let Some(logical_id) = authoritative
            .units
            .iter()
            .filter_map(|unit| unit.target_repository_id)
            .next()
        else {
            return Ok(None);
        };
        let lc_id = resolve_issue_logical_codebase_id(paths, &input.project_id, &input.issue_id)
            .map_err(|error| format!("resolve advance target codebase failed: {error}"))?;
        build_attempt_target_snapshot(paths, &input.project_id, logical_id, lc_id.as_deref())
            .map(Some)
            .map_err(|error| format!("capture advance target snapshot failed: {error}"))
    }
    pub(super) fn advance_workspace_entry(
        attempt: &crate::product::coding_models::CodingExecutionAttempt,
    ) -> String {
        attempt
            .worktree_path
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("workspace://{}", attempt.id))
    }

    /// C1 Task 7（REQ-ADV-C1-RETRY）：Failed advance 显式
    /// retry-initialization。独立产品动作、独立 durable retry 事实：
    /// - 普通 `handle_advance` 对 Failed 一律 `Replayed`（原失败事实不动），
    ///   本方法绝不隐式触发；
    /// - 身份链（binding/attempt/checkpoint/plan）过期一律拒绝且 durable
    ///   不变；
    /// - 失败发生在 WorktreeBound 及以后（可能存在 git/worktree 外部副
    ///   作用状态）且未携 `confirm_unknown_side_effect` → 只写 NeedsHuman
    ///   retry 事实，不重跑步骤；
    /// - 确认后续做：CAS 把 Failed record 置回 Initializing（失败审计字段
    ///   保留），再走既有 checkpoint continuation 以同一 command/attempt
    ///   续进到 Ready——不新建 record/attempt，不启动 provider。
    pub async fn retry_initialization(
        &mut self,
        request: &crate::product::models::automation::RetryInitializationRequest,
    ) -> Result<crate::product::models::automation::RetryInitializationResult, String> {
        // D2.1d：独立 engine 路径同 guard——在未持任何 engine/arbitration
        // 锁处取 owning Plan run lease（session 身份）；web retry 入口已在
        // engine.lock 前取得准入时不走本包装（直接调
        // `retry_initialization_with_lease` 消费同一 lease）。
        let paths = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?
            .app_paths();
        let admission = match AdvancePlanAdmission::acquire(
            &paths,
            &self.session.project_id,
            &self.session.issue_id,
            &self.session.entity_id,
        ) {
            Ok(admission) => admission,
            Err(error) => return Err(advance_admission_error(error, &self.session.entity_id)),
        };
        self.retry_initialization_with_lease(request, &admission)
            .await
    }

    /// D2.1d：retry 的 lease 消费实现（入口准入在 engine.lock 前取得）。
    /// purging/deleted 墓碑与执行轮退役围栏先于命令账本幂等与任何 retry
    /// 事实写入——旧轮 retry 不复活。
    pub async fn retry_initialization_with_lease(
        &mut self,
        request: &crate::product::models::automation::RetryInitializationRequest,
        admission: &AdvancePlanAdmission,
    ) -> Result<crate::product::models::automation::RetryInitializationResult, String> {
        use crate::product::models::automation::{
            EnrollmentError, OperationState, RetryInitializationResult,
        };

        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let plan_id = self.session.entity_id.clone();
        let app_paths = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?
            .app_paths();
        let advance_store = AdvanceStore::new(app_paths.clone());
        let automation =
            crate::product::issue_automation_store::IssueAutomationStore::new(app_paths.clone());

        // D2.1d/H3：墓碑/退役围栏先于命令账本幂等与 retry 事实写入。
        if let Err(failure) = admission.guard_round_admission(&app_paths, &request.command_id) {
            return Err(match failure {
                AdvanceRoundGuardFailure::Round(rejection) => {
                    format!(
                        "{}: {}",
                        rejection.stable_kind(),
                        rejection.reason_text(&plan_id)
                    )
                }
                AdvanceRoundGuardFailure::Store(error) => {
                    format!("retry round admission check failed: {error}")
                }
            });
        }

        let record = advance_store
            .get_advance_for_plan(&project_id, &issue_id, &plan_id)
            .map_err(|error| format!("load advance record failed: {error}"))?
            .ok_or_else(|| "no advance record for plan".to_string())?;
        // 退役的原 command 同样先于命令账本幂等/retry 写（旧轮 retry 不
        // 复活）。
        if let Err(failure) = admission.guard_round_admission(&app_paths, &record.command_id) {
            return Err(match failure {
                AdvanceRoundGuardFailure::Round(rejection) => {
                    format!(
                        "{}: {}",
                        rejection.stable_kind(),
                        rejection.reason_text(&plan_id)
                    )
                }
                AdvanceRoundGuardFailure::Store(error) => {
                    format!("retry round admission check failed: {error}")
                }
            });
        }

        // 命令幂等：enrollment 命令账本先判（同 command 异 payload
        // fail-closed；同 payload 命中 → 幂等重放）。
        let digest = request.payload_digest();
        let ledger_hit = automation
            .find_command_result(&project_id, &issue_id, &request.command_id, &digest)
            .map_err(|error| match error {
                EnrollmentError::Conflict { .. } => {
                    "retry command id is already bound to a different payload".to_string()
                }
                other => format!("retry command ledger check failed: {other}"),
            })?;
        if ledger_hit.is_some() {
            let retry = advance_store
                .get_retry_initialization(&project_id, &issue_id, &request.command_id)
                .map_err(|error| format!("load retry record failed: {error}"))?
                .ok_or_else(|| {
                    "retry command ledger entry exists but the durable retry record is missing"
                        .to_string()
                })?;
            // 幂等续做投影：Ready → Replayed；仍 Initializing → 续做；
            // 再次 Failed → Replayed(原失败事实)，不重复建 retry。
            let outcome = self
                .handle_advance_with_start_policy(
                    AdvanceInput {
                        command_id: record.command_id.clone(),
                        project_id,
                        issue_id,
                        plan_id,
                    },
                    crate::product::coding_models::CodingStartRunPolicy::Manual,
                    admission,
                )
                .await?;
            return Ok(RetryInitializationResult {
                command_id: request.command_id.clone(),
                state: OperationState::Replayed,
                retry,
                outcome: Some(outcome),
            });
        }

        // 身份链校验（拒绝路径 durable 全不变）。
        if record.status != AdvanceStatus::Failed {
            return Err(format!(
                "retry_initialization requires a failed advance record, got {:?}",
                record.status
            ));
        }
        let journal = advance_store
            .get_advance_initialization(&record)
            .map_err(|error| format!("load advance journal failed: {error}"))?
            .ok_or_else(|| "failed advance has no initialization journal".to_string())?;
        if journal.error.is_none() {
            return Err("retry_initialization requires a journal failure fact".to_string());
        }
        // record.attempt_id 只在 Ready 完成时写入（单目标早期失败为 None）；
        // 一旦写入必须与 journal 一致。
        if journal.attempt_id != request.expected_attempt_id
            || record
                .attempt_id
                .as_deref()
                .is_some_and(|attempt| attempt != journal.attempt_id)
        {
            return Err(format!(
                "retry attempt identity mismatch: expected {}, journal {}, record {:?}",
                request.expected_attempt_id, journal.attempt_id, record.attempt_id
            ));
        }
        if journal.phase != request.expected_checkpoint {
            return Err(format!(
                "retry checkpoint expired: expected {:?}, durable journal at {:?}",
                request.expected_checkpoint, journal.phase
            ));
        }
        let enrollment = automation
            .get(&project_id, &issue_id)
            .map_err(|error| format!("load enrollment for retry failed: {error}"))?
            .ok_or_else(|| "retry requires a durable automation enrollment".to_string())?;
        if !enrollment.enabled {
            return Err("retry requires an enabled automation enrollment".to_string());
        }
        let binding = enrollment
            .binding_history
            .as_ref()
            .map(|history| history.current.clone())
            .ok_or_else(|| {
                "retry requires a versioned enrollment binding; re-enable with an explicit \
                 target or rebind first"
                    .to_string()
            })?;
        if binding != request.expected_binding {
            return Err(format!(
                "retry binding expired: expected v{}, current v{}",
                request.expected_binding.binding_version, binding.binding_version
            ));
        }
        if !binding.plan_id.is_empty() && binding.plan_id != plan_id {
            return Err(format!(
                "retry binding plan mismatch: binding {}, plan {}",
                binding.plan_id, plan_id
            ));
        }

        // 未知副作用门：失败已在 WorktreeBound 及以后 → 可能存在
        // git/worktree 外部副作用状态；未确认只写 NeedsHuman 事实。
        let side_effect_possible = journal.phase.order_for_engine()
            >= AdvanceInitializationPhase::WorktreeBound.order_for_engine();
        if side_effect_possible && !request.confirm_unknown_side_effect {
            let retry = advance_store
                .create_retry_initialization(&record, request, OperationState::NeedsHuman)
                .map_err(|error| format!("persist retry fact failed: {error}"))?;
            automation
                .append_command_result(
                    &project_id,
                    &issue_id,
                    &request.command_id,
                    &digest,
                    OperationState::NeedsHuman,
                )
                .map_err(|error| format!("persist retry command failed: {error}"))?;
            return Ok(RetryInitializationResult {
                command_id: request.command_id.clone(),
                state: OperationState::NeedsHuman,
                retry,
                outcome: None,
            });
        }

        // 确认续做：写 Accepted retry 事实 → CAS 重开 Failed record（失败
        // 审计字段保留）→ 既有 checkpoint continuation 同 command/attempt。
        let retry = advance_store
            .create_retry_initialization(&record, request, OperationState::Accepted)
            .map_err(|error| format!("persist retry fact failed: {error}"))?;
        automation
            .append_command_result(
                &project_id,
                &issue_id,
                &request.command_id,
                &digest,
                OperationState::Accepted,
            )
            .map_err(|error| format!("persist retry command failed: {error}"))?;
        advance_store
            .reopen_failed_record_for_retry(&record)
            .map_err(|error| format!("reopen failed record for retry failed: {error}"))?;
        let input = AdvanceInput {
            command_id: record.command_id.clone(),
            project_id,
            issue_id,
            plan_id,
        };
        match self
            .handle_advance_with_start_policy(
                input,
                crate::product::coding_models::CodingStartRunPolicy::Manual,
                admission,
            )
            .await
        {
            Ok(outcome @ (AdvanceOutcome::Completed { .. } | AdvanceOutcome::Replayed { .. })) => {
                Ok(RetryInitializationResult {
                    command_id: request.command_id.clone(),
                    state: OperationState::Accepted,
                    retry,
                    outcome: Some(outcome),
                })
            }
            Ok(AdvanceOutcome::Rejected { reason, .. }) => {
                let _ = reason;
                let retry = advance_store
                    .update_retry_initialization_state(
                        &self.session.project_id,
                        &self.session.issue_id,
                        &retry,
                        OperationState::NeedsHuman,
                    )
                    .map_err(|error| format!("mark retry needs human failed: {error}"))?;
                Ok(RetryInitializationResult {
                    command_id: request.command_id.clone(),
                    state: OperationState::NeedsHuman,
                    retry,
                    outcome: None,
                })
            }
            Err(error) => {
                let _ = advance_store.update_retry_initialization_state(
                    &self.session.project_id,
                    &self.session.issue_id,
                    &retry,
                    OperationState::NeedsHuman,
                );
                Err(format!("retry continuation failed: {error}"))
            }
        }
    }

    pub(super) fn current_git_branch(path: &std::path::Path) -> Option<String> {
        let output = Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(path)
            .output()
            .ok()?;
        let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
        (!branch.is_empty()).then_some(branch)
    }

    pub(super) fn advance_provider_config(
        session: &super::types::WorkspaceSession,
        _unit: &crate::product::coding_attempt_store::AuthoritativeCodingUnitBinding,
    ) -> ProviderConfigSnapshot {
        ProviderConfigSnapshot {
            author: session.author_provider.clone(),
            reviewer: session.reviewer_provider.clone(),
            review_rounds: session.review_rounds,
            permission_modes: session.permission_modes.clone(),
        }
    }

    /// REQ-PIB-03（T3.1）：advance 新 attempt 的 fork 基线解析——读
    /// issue.base_branch 并经 `resolve_effective_base_branch`（三面同源唯一
    /// 解析链，先例 `resolve_advance_repository` 同款 IssueStore 读取）：
    /// 显式锁定分支须本地存在，存量 None 走默认链 main→master；不可解析
    /// （分支被删/皆无/仓库不可用）→ Err(diagnosis) fail-closed，不回退当前
    /// 检出或 HEAD，防止 coder worktree 分叉点与 author/C1 核对树错位。
    fn resolve_advance_base_branch(
        paths: &crate::product::app_paths::ProductAppPaths,
        repository_path: &std::path::Path,
        input: &AdvanceInput,
    ) -> Result<String, String> {
        let issue = IssueStore::new(paths.clone())
            .get(&input.project_id, &input.issue_id)
            .map_err(|error| format!("load advance issue failed: {error}"))?;
        crate::product::issue_baseline::resolve_effective_base_branch(
            repository_path,
            issue.base_branch.as_deref(),
        )
        .map_err(|error| format!("resolve advance base branch failed: {error}"))
    }

    fn resolve_advance_repository(
        paths: &crate::product::app_paths::ProductAppPaths,
        input: &AdvanceInput,
        authoritative: &AuthoritativeGroupPlanBinding,
    ) -> Result<crate::product::models::RepositoryRecord, String> {
        let issue = IssueStore::new(paths.clone())
            .get(&input.project_id, &input.issue_id)
            .map_err(|error| format!("load advance issue failed: {error}"))?;
        match RepositoryRouting::load_for_issue(paths, &input.project_id, &input.issue_id)
            .map_err(|error| format!("load advance repository routing failed: {error}"))?
        {
            RepositoryRouting::Legacy { .. } => {
                let repository_id = issue.repo_id.ok_or("advance issue has no repository")?;
                let project = crate::product::project_store::ProjectStore::new(paths.clone())
                    .get(&input.project_id)
                    .map_err(|error| format!("load advance project failed: {error}"))?;
                let store = RepositoryStore::for_project(paths.clone(), &project);
                store
                    .resolve_legacy_physical_repository_if_dual(&input.project_id, &repository_id)
                    .map(|(_, _, repository)| repository)
                    .or_else(|_| {
                        store
                            .list(&input.project_id)
                            .map_err(|error| format!("list advance repositories failed: {error}"))?
                            .into_iter()
                            .find(|repository| repository.id == repository_id)
                            .ok_or_else(|| "advance repository not found".to_string())
                    })
            }
            RepositoryRouting::Logical {
                manifest,
                selection,
            } => {
                let logical_id = authoritative
                    .units
                    .iter()
                    .filter_map(|unit| unit.target_repository_id)
                    .next()
                    .or_else(|| selection.focus_repository_ids.first().copied())
                    .ok_or("advance logical repository target missing")?;
                if !manifest.member_ids.contains(&logical_id) {
                    return Err("advance logical repository target is not selected".to_string());
                }
                let lc_id =
                    resolve_issue_logical_codebase_id(paths, &input.project_id, &input.issue_id)
                        .map_err(|error| format!("resolve advance codebase failed: {error}"))?;
                RepositoryStore::new(paths.clone())
                    .resolve_logical_repository_for_issue_codebase(
                        &input.project_id,
                        lc_id.as_deref(),
                        logical_id,
                    )
                    .map(|(_, _, repository)| repository)
                    .map_err(|error| format!("resolve advance logical repository failed: {error}"))
            }
            RepositoryRouting::FailClosed { reason, .. } => Err(reason),
        }
    }

    #[allow(dead_code)]
    pub(crate) fn advance_record_store(&self) -> Option<AdvanceStore> {
        self.lifecycle_store
            .as_ref()
            .map(|store| AdvanceStore::new(store.app_paths()))
    }
}
