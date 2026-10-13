//! fix-multi-repo-group-coding-start N2（REQ-CS-02/05/07）：plan 显式 Coding
//! 启动授权的冻结 run 记录（`CodingPlanRunStore`）。
//!
//! 只记录显式授权、冻结 target/selection 与暂停事实，不替代 AdvanceRecord/
//! start_claim（各自仍是权威）。磁盘地址冻结：
//! `{workspace_root}/.aria/projects/{project_id}/issues/{issue_id}/coding-plan-runs/{plan_id}.json`，
//! 锁沿 `with_exclusive_lock` 的 target-derived 规则派生为同目录前导点文件
//! `.{plan_id}.json.lock`（路径方法封装在本 store，不改 app_paths.rs）。
//! 所有锁内操作均为同步文件操作，不持锁 await、不持锁调用 provider。

use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::product::advance_store::AdvanceTargetAttemptBinding;
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::with_exclusive_lock;
use crate::product::coding_attempt_store::{CodingAttemptStore, path_is_regular_file};
use crate::product::coding_models::{
    CodingAttemptStatus, CodingExecutionAttempt, CodingExecutionUnitStatus,
};
use crate::product::entity_deletion_store::DeletionEntityKind;
use crate::product::entity_operation_store::EntityDeletionLease;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::IssueWorkItemPlanStatus;
use crate::product::work_item_revision_store::WorkItemRevisionStore;

/// REQ-CS-05：first_ready_unit 首项完成安全暂停的原因码（blocked gate 与
/// run pause 共用同一冻结值）。
pub const PLAN_CODING_CONTINUATION_NOT_ENABLED: &str = "plan_coding_continuation_not_enabled";
/// 今晚唯一模式：一次显式授权只运行全局首个 ready WI。
pub const PLAN_CODING_MODE_FIRST_READY_UNIT: &str = "first_ready_unit";

/// run 相位序列：
/// `prepared→ready_bound→start_requested→started/already_started/needs_human→paused`；
/// 成员删除→`invalidated`（终局，不再可推进）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingPlanRunPhase {
    Prepared,
    ReadyBound,
    StartRequested,
    Started,
    AlreadyStarted,
    NeedsHuman,
    Paused,
    Invalidated,
}

/// 冻结的首启 selection：选中 WI/unit、所属 attempt/target 与确定性首启
/// command 全部在首启调用前持久化（REQ-CS-02）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingPlanSelection {
    pub unit_id: String,
    pub logical_work_item_id: String,
    pub attempt_id: String,
    pub target_repository_id: String,
    pub start_command_id: String,
}

impl CodingPlanSelection {
    /// 确定性首启 command：
    /// `plan_start_{SHA256(JSON数组[project,issue,plan,run.command_id,target_uuid])全64hex}`。
    /// 复用已存在 sha2/hex；产物仅 `[A-Za-z0-9_]`，恒过 `validate_relative_id`。
    pub fn deterministic_start_command_id(
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        command_id: &str,
        target_repository_id: &str,
    ) -> String {
        let payload = serde_json::json!([
            project_id,
            issue_id,
            plan_id,
            command_id,
            target_repository_id,
        ])
        .to_string();
        format!(
            "plan_start_{}",
            hex::encode(Sha256::digest(payload.as_bytes()))
        )
    }
}

/// 首启真实回执（tagged 序列化，不直接序列化 web enum）。`NeedsHuman` 携带
/// reason；副作用不明保持 NeedsHuman，不得由重试隐式清除。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CodingPlanStartOutcome {
    Started { attempt_id: String },
    AlreadyStarted { attempt_id: String },
    NeedsHuman { attempt_id: String, reason: String },
}

/// plan 启动授权的 durable run 记录。`target_attempts` 复用
/// `AdvanceTargetAttemptBinding`（attempt 粒度=逻辑目标仓）；`mode` 冻结为
/// `first_ready_unit`（字符串承载，后续 S/P 包扩展不改旧记录形状）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingPlanRun {
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub command_id: String,
    pub bound_plan_revision_id: String,
    pub dependency_graph_revision_id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_attempts: Vec<AdvanceTargetAttemptBinding>,
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<CodingPlanSelection>,
    pub phase: CodingPlanRunPhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_outcome: Option<CodingPlanStartOutcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pause_reason: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct CodingPlanRunStore {
    paths: ProductAppPaths,
}

impl CodingPlanRunStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn app_paths(&self) -> ProductAppPaths {
        self.paths.clone()
    }

    fn runs_root(&self, project_id: &str, issue_id: &str) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        Ok(self
            .paths
            .issue_root(project_id, issue_id)
            .join("coding-plan-runs"))
    }

    fn run_path(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(plan_id)?;
        Ok(self
            .runs_root(project_id, issue_id)?
            .join(format!("{plan_id}.json")))
    }

    /// 与 `with_exclusive_lock` 派生命名一致的同目录前导点锁文件
    /// `.{plan_id}.json.lock`（冻结地址；purge 的安全删除窗口才 unlink）。
    fn lock_path(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        let run_path = self.run_path(project_id, issue_id, plan_id)?;
        Ok(run_path.with_file_name(format!(".{plan_id}.json.lock")))
    }

    fn read_run(
        &self,
        path: &Path,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<CodingPlanRun, ProductStoreError> {
        let run: CodingPlanRun = read_json(path)?;
        if run.project_id != project_id || run.issue_id != issue_id || run.plan_id != plan_id {
            // 身份错配不能当 None/缺失处理。
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_plan_run",
                id: plan_id.to_string(),
            });
        }
        Ok(run)
    }

    fn load_run(
        &self,
        path: &Path,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<CodingPlanRun, ProductStoreError> {
        if !path_is_regular_file(path)? {
            return Err(ProductStoreError::NotFound {
                kind: "coding_plan_run",
                id: plan_id.to_string(),
            });
        }
        self.read_run(path, project_id, issue_id, plan_id)
    }

    /// 锁内重读后的身份核对：调用方持有的 run 快照必须与 durable 记录同身份
    /// （project/issue/plan/command/bound revision），否则漂移拒绝。
    fn verify_same_identity(
        &self,
        stored: &CodingPlanRun,
        run: &CodingPlanRun,
    ) -> Result<(), ProductStoreError> {
        if stored.project_id != run.project_id
            || stored.issue_id != run.issue_id
            || stored.plan_id != run.plan_id
            || stored.command_id != run.command_id
            || stored.bound_plan_revision_id != run.bound_plan_revision_id
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_plan_run",
                id: run.plan_id.clone(),
            });
        }
        Ok(())
    }

    /// REQ-CS-02：显式授权冻结入口。confirmed/当前 authoritative 身份校验并
    /// 冻结 graph；同 plan/revision 第二 command 返回原 run（原 command/
    /// selection 不漂移）；异 revision/graph 拒绝；Paused/Invalidated 不再
    /// 返回 active run（无旧数据补授权）。Plan 稳定锁内重读/写入。
    pub fn prepare(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        command_id: &str,
        expected_plan_revision_id: &str,
    ) -> Result<CodingPlanRun, ProductStoreError> {
        for id in [
            project_id,
            issue_id,
            plan_id,
            command_id,
            expected_plan_revision_id,
        ] {
            validate_relative_id(id)?;
        }
        let run_path = self.run_path(project_id, issue_id, plan_id)?;
        with_exclusive_lock(&run_path, || {
            // ① confirmed 权威身份。
            let lifecycle = LifecycleStore::new(self.paths.clone());
            let plan = lifecycle.get_issue_work_item_plan(project_id, issue_id, plan_id)?;
            if plan.status != IssueWorkItemPlanStatus::Confirmed {
                return Err(ProductStoreError::Conflict {
                    kind: "coding_plan_run_plan_not_confirmed",
                    id: plan_id.to_string(),
                });
            }
            // ② 当前 authoritative revision（无在途 amendment）。
            let revision_store = WorkItemRevisionStore::new(self.paths.clone());
            let lineage = revision_store.get_plan_lineage(project_id, issue_id, plan_id)?;
            let active_revision_id = lineage.active_revision_id.clone().ok_or_else(|| {
                ProductStoreError::InvalidRecord {
                    kind: "coding_plan_run_plan_revision",
                    reason: "active plan revision is missing".to_string(),
                }
            })?;
            if lineage.active_amendment_id.is_some() {
                return Err(ProductStoreError::Conflict {
                    kind: "coding_plan_run_active_amendment",
                    id: plan_id.to_string(),
                });
            }
            if active_revision_id != expected_plan_revision_id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_plan_run_revision",
                    id: expected_plan_revision_id.to_string(),
                });
            }
            // ③ 冻结 graph（同源权威绑定校验全图）。
            let attempt_store = CodingAttemptStore::new(self.paths.clone());
            let authoritative = attempt_store
                .resolve_authoritative_group_plan_binding_for_revision(
                    project_id,
                    issue_id,
                    plan_id,
                    &active_revision_id,
                )?;
            // ④ 幂等：已有 run 按冻结身份返回，不写第二 claim 账本。
            if path_is_regular_file(&run_path)? {
                let stored = self.read_run(&run_path, project_id, issue_id, plan_id)?;
                if stored.bound_plan_revision_id != expected_plan_revision_id {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "coding_plan_run_revision",
                        id: expected_plan_revision_id.to_string(),
                    });
                }
                if stored.dependency_graph_revision_id != authoritative.dependency_graph_revision_id
                {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "coding_plan_run_graph",
                        id: authoritative.dependency_graph_revision_id,
                    });
                }
                if matches!(
                    stored.phase,
                    CodingPlanRunPhase::Paused | CodingPlanRunPhase::Invalidated
                ) {
                    return Err(ProductStoreError::Conflict {
                        kind: "coding_plan_run_not_active",
                        id: stored.command_id.clone(),
                    });
                }
                return Ok(stored);
            }
            let now = Utc::now().to_rfc3339();
            let run = CodingPlanRun {
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                plan_id: plan_id.to_string(),
                command_id: command_id.to_string(),
                bound_plan_revision_id: expected_plan_revision_id.to_string(),
                dependency_graph_revision_id: authoritative.dependency_graph_revision_id,
                target_attempts: Vec::new(),
                mode: PLAN_CODING_MODE_FIRST_READY_UNIT.to_string(),
                selection: None,
                phase: CodingPlanRunPhase::Prepared,
                start_outcome: None,
                pause_reason: None,
                created_at: now.clone(),
                updated_at: now,
            };
            write_json(&run_path, &run)?;
            Ok(run)
        })
    }

    /// REQ-CS-02：durable Ready 集绑定与 selection 冻结。无 ready 时仍 durable
    /// 绑定全集但 `selection=None`，保持 `ready_bound` 等待；`ReadyBound(None)`
    /// 后 ready 出现可补冻结 selection（一次冻结，之后不再漂移）。首次绑定
    /// 不收养既有非未首启集合（attempt 必须全部 Created）；重放必须命中同一
    /// 冻结集合/selection，漂移即拒。
    pub fn bind_ready(
        &self,
        run: &CodingPlanRun,
        bindings: &[AdvanceTargetAttemptBinding],
        selection: Option<CodingPlanSelection>,
    ) -> Result<CodingPlanRun, ProductStoreError> {
        for binding in bindings {
            validate_relative_id(&binding.attempt_id)?;
            validate_relative_id(&binding.target_repository_id)?;
        }
        if let Some(selection) = &selection {
            for id in [
                selection.unit_id.as_str(),
                selection.logical_work_item_id.as_str(),
                selection.attempt_id.as_str(),
                selection.target_repository_id.as_str(),
                selection.start_command_id.as_str(),
            ] {
                validate_relative_id(id)?;
            }
        }
        let run_path = self.run_path(&run.project_id, &run.issue_id, &run.plan_id)?;
        with_exclusive_lock(&run_path, || {
            let stored = self.load_run(&run_path, &run.project_id, &run.issue_id, &run.plan_id)?;
            self.verify_same_identity(&stored, run)?;
            if matches!(stored.phase, CodingPlanRunPhase::Invalidated) {
                return Err(ProductStoreError::Conflict {
                    kind: "coding_plan_run_invalidated",
                    id: run.plan_id.clone(),
                });
            }
            if matches!(stored.phase, CodingPlanRunPhase::Paused) {
                return Err(ProductStoreError::Conflict {
                    kind: "coding_plan_run_not_active",
                    id: run.plan_id.clone(),
                });
            }
            let attempt_store = CodingAttemptStore::new(self.paths.clone());
            match stored.phase {
                CodingPlanRunPhase::Prepared => {
                    // 首次绑定：逐 attempt 核 plan 归属、target 桶与未首启状态
                    // （REQ-CS-02：不收养 Running/Waiting/Blocked/Failed/终态集合）。
                    for binding in bindings {
                        let attempt = attempt_store.get_attempt(
                            &run.project_id,
                            &run.issue_id,
                            &binding.attempt_id,
                        )?;
                        if attempt.work_item_group_id.as_deref() != Some(run.plan_id.as_str()) {
                            return Err(ProductStoreError::IdentityMismatch {
                                kind: "coding_plan_run_attempt_binding",
                                id: binding.attempt_id.clone(),
                            });
                        }
                        let attempt_target = attempt
                            .target_snapshot
                            .as_ref()
                            .map(|snapshot| snapshot.logical_repository_id.0.to_string());
                        if attempt_target.as_deref() != Some(binding.target_repository_id.as_str())
                        {
                            return Err(ProductStoreError::IdentityMismatch {
                                kind: "coding_plan_run_attempt_target",
                                id: binding.attempt_id.clone(),
                            });
                        }
                        if attempt.status != CodingAttemptStatus::Created {
                            return Err(ProductStoreError::Conflict {
                                kind: "coding_plan_run_set_not_first_start",
                                id: binding.attempt_id.clone(),
                            });
                        }
                    }
                    if let Some(selection) = &selection {
                        self.verify_selection_locked(run, bindings, selection, &attempt_store)?;
                    }
                    let mut updated = stored.clone();
                    updated.target_attempts = bindings.to_vec();
                    updated.selection = selection;
                    updated.phase = CodingPlanRunPhase::ReadyBound;
                    updated.updated_at = Utc::now().to_rfc3339();
                    write_json(&run_path, &updated)?;
                    Ok(updated)
                }
                CodingPlanRunPhase::ReadyBound => {
                    if stored.target_attempts != bindings {
                        return Err(ProductStoreError::IdentityMismatch {
                            kind: "coding_plan_run_target_attempts",
                            id: run.plan_id.clone(),
                        });
                    }
                    if stored.selection.is_some() && stored.selection != selection {
                        return Err(ProductStoreError::IdentityMismatch {
                            kind: "coding_plan_run_selection",
                            id: run.plan_id.clone(),
                        });
                    }
                    if let Some(selection) = &selection {
                        self.verify_selection_locked(run, bindings, selection, &attempt_store)?;
                    }
                    if stored.selection == selection {
                        return Ok(stored);
                    }
                    let mut updated = stored.clone();
                    updated.selection = selection;
                    updated.updated_at = Utc::now().to_rfc3339();
                    write_json(&run_path, &updated)?;
                    Ok(updated)
                }
                // 中断窗口重放：读同一冻结集合/selection（不重捕、不重发）。
                CodingPlanRunPhase::StartRequested
                | CodingPlanRunPhase::Started
                | CodingPlanRunPhase::AlreadyStarted
                | CodingPlanRunPhase::NeedsHuman => {
                    if stored.target_attempts != bindings || stored.selection != selection {
                        return Err(ProductStoreError::IdentityMismatch {
                            kind: "coding_plan_run_replay_drift",
                            id: run.plan_id.clone(),
                        });
                    }
                    Ok(stored)
                }
                CodingPlanRunPhase::Paused | CodingPlanRunPhase::Invalidated => {
                    // 上面已按 not_active/invalidated 拒绝，此处不可达。
                    Err(ProductStoreError::Conflict {
                        kind: "coding_plan_run_not_active",
                        id: run.plan_id.clone(),
                    })
                }
            }
        })
    }

    /// selection 合法性：attempt 必在冻结绑定集且 target 一致；首启 command
    /// 必须等于确定性派生值；unit 必须存在于该 attempt 且未进入终态——
    /// 桶内首 unit 初始化即 Running 是物化事实而非 provider 执行事实
    /// （N3 DTO execution_state 语义），Pending/Running 均可选；Completed/
    /// Failed/Superseded/Skipped 的 unit 不可被选为首启。
    fn verify_selection_locked(
        &self,
        run: &CodingPlanRun,
        bindings: &[AdvanceTargetAttemptBinding],
        selection: &CodingPlanSelection,
        attempt_store: &CodingAttemptStore,
    ) -> Result<(), ProductStoreError> {
        let Some(binding) = bindings
            .iter()
            .find(|binding| binding.attempt_id == selection.attempt_id)
        else {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_plan_run_selection_attempt",
                id: selection.attempt_id.clone(),
            });
        };
        if binding.target_repository_id != selection.target_repository_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_plan_run_selection_target",
                id: selection.target_repository_id.clone(),
            });
        }
        let expected_command = CodingPlanSelection::deterministic_start_command_id(
            &run.project_id,
            &run.issue_id,
            &run.plan_id,
            &run.command_id,
            &selection.target_repository_id,
        );
        if selection.start_command_id != expected_command {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_plan_run_start_command",
                id: selection.start_command_id.clone(),
            });
        }
        let units = attempt_store.list_coding_units(
            &run.project_id,
            &run.issue_id,
            &selection.attempt_id,
        )?;
        let unit = units
            .iter()
            .find(|unit| unit.id == selection.unit_id)
            .ok_or_else(|| ProductStoreError::InvalidRecord {
                kind: "coding_plan_run_selection_unit",
                reason: format!(
                    "unit {} is not materialized in attempt {}",
                    selection.unit_id, selection.attempt_id
                ),
            })?;
        if unit.logical_work_item_id != selection.logical_work_item_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_plan_run_selection_unit",
                id: selection.unit_id.clone(),
            });
        }
        if matches!(
            unit.status,
            CodingExecutionUnitStatus::Completed
                | CodingExecutionUnitStatus::Failed
                | CodingExecutionUnitStatus::Superseded
                | CodingExecutionUnitStatus::Skipped
        ) {
            return Err(ProductStoreError::Conflict {
                kind: "coding_plan_run_selection_not_ready",
                id: unit.id.clone(),
            });
        }
        Ok(())
    }

    /// REQ-CS-02：首启调用前的 durable 标记。必须有冻结 selection——不能发
    /// 空首启；`start_requested` 之后的重放幂等返回（中断窗口读同一
    /// selection 重放）。
    pub fn mark_start_requested(
        &self,
        run: &CodingPlanRun,
    ) -> Result<CodingPlanRun, ProductStoreError> {
        let run_path = self.run_path(&run.project_id, &run.issue_id, &run.plan_id)?;
        with_exclusive_lock(&run_path, || {
            let stored = self.load_run(&run_path, &run.project_id, &run.issue_id, &run.plan_id)?;
            self.verify_same_identity(&stored, run)?;
            if matches!(stored.phase, CodingPlanRunPhase::Invalidated) {
                return Err(ProductStoreError::Conflict {
                    kind: "coding_plan_run_invalidated",
                    id: run.plan_id.clone(),
                });
            }
            if matches!(stored.phase, CodingPlanRunPhase::Paused) {
                return Err(ProductStoreError::Conflict {
                    kind: "coding_plan_run_not_active",
                    id: run.plan_id.clone(),
                });
            }
            if matches!(
                stored.phase,
                CodingPlanRunPhase::StartRequested
                    | CodingPlanRunPhase::Started
                    | CodingPlanRunPhase::AlreadyStarted
                    | CodingPlanRunPhase::NeedsHuman
            ) {
                return Ok(stored);
            }
            // ReadyBound：必须有冻结 selection。
            if stored.selection.is_none() {
                return Err(ProductStoreError::Conflict {
                    kind: "coding_plan_run_selection_missing",
                    id: run.plan_id.clone(),
                });
            }
            let mut updated = stored.clone();
            updated.phase = CodingPlanRunPhase::StartRequested;
            updated.updated_at = Utc::now().to_rfc3339();
            write_json(&run_path, &updated)?;
            Ok(updated)
        })
    }

    /// REQ-CS-02：记录首启真实回执（Started/AlreadyStarted/NeedsHuman），相位
    /// 随回执落定；回执一旦冻结不再漂移（幂等重放返回原记录）。
    pub fn record_start_outcome(
        &self,
        run: &CodingPlanRun,
        outcome: CodingPlanStartOutcome,
    ) -> Result<CodingPlanRun, ProductStoreError> {
        if let CodingPlanStartOutcome::NeedsHuman { attempt_id, .. } = &outcome {
            validate_relative_id(attempt_id)?;
        }
        let run_path = self.run_path(&run.project_id, &run.issue_id, &run.plan_id)?;
        with_exclusive_lock(&run_path, || {
            let stored = self.load_run(&run_path, &run.project_id, &run.issue_id, &run.plan_id)?;
            self.verify_same_identity(&stored, run)?;
            if matches!(stored.phase, CodingPlanRunPhase::Invalidated) {
                return Err(ProductStoreError::Conflict {
                    kind: "coding_plan_run_invalidated",
                    id: run.plan_id.clone(),
                });
            }
            if matches!(stored.phase, CodingPlanRunPhase::Paused) {
                return Err(ProductStoreError::Conflict {
                    kind: "coding_plan_run_not_active",
                    id: run.plan_id.clone(),
                });
            }
            if stored.start_outcome.as_ref() == Some(&outcome) {
                return Ok(stored);
            }
            if stored.start_outcome.is_some() {
                return Err(ProductStoreError::Conflict {
                    kind: "coding_plan_run_outcome_frozen",
                    id: run.plan_id.clone(),
                });
            }
            if stored.phase != CodingPlanRunPhase::StartRequested {
                return Err(ProductStoreError::Conflict {
                    kind: "coding_plan_run_phase",
                    id: format!("{:?}", stored.phase),
                });
            }
            let phase = match &outcome {
                CodingPlanStartOutcome::Started { .. } => CodingPlanRunPhase::Started,
                CodingPlanStartOutcome::AlreadyStarted { .. } => CodingPlanRunPhase::AlreadyStarted,
                CodingPlanStartOutcome::NeedsHuman { .. } => CodingPlanRunPhase::NeedsHuman,
            };
            let mut updated = stored.clone();
            updated.phase = phase;
            updated.start_outcome = Some(outcome);
            updated.updated_at = Utc::now().to_rfc3339();
            write_json(&run_path, &updated)?;
            Ok(updated)
        })
    }

    pub fn get(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<Option<CodingPlanRun>, ProductStoreError> {
        let run_path = self.run_path(project_id, issue_id, plan_id)?;
        if !path_is_regular_file(&run_path)? {
            return Ok(None);
        }
        Ok(Some(
            self.read_run(&run_path, project_id, issue_id, plan_id)?,
        ))
    }

    /// 按完成/暂停消费面（N5/N2 gate）检索：命中「本 plan 的
    /// first_ready_unit run 且冻结 selection 指向该 attempt」。身份错配与
    /// IO/JSON 错误上抛，不当 None。
    pub fn matching_first_ready_run(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<Option<CodingPlanRun>, ProductStoreError> {
        let Some(plan_id) = attempt.work_item_group_id.clone() else {
            return Ok(None);
        };
        self.scan_runs_for_attempt(attempt, plan_id, RunAttemptMatch::Selection)
    }

    /// REQ-CS-07：冻结集合中任一 attempt 成员被独立删除后，原授权失效
    /// （Invalidated），不再可首启/推进该不完整集合；无匹配 run 直接 OK，
    /// 绝不自动补建。
    pub fn invalidate_for_deleted_attempt(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), ProductStoreError> {
        let Some(plan_id) = attempt.work_item_group_id.clone() else {
            return Ok(());
        };
        let Some(run) = self.scan_runs_for_attempt(attempt, plan_id, RunAttemptMatch::Member)?
        else {
            return Ok(());
        };
        let run_path = self.run_path(&run.project_id, &run.issue_id, &run.plan_id)?;
        with_exclusive_lock(&run_path, || {
            let stored = self.load_run(&run_path, &run.project_id, &run.issue_id, &run.plan_id)?;
            if stored.project_id != attempt.project_id || stored.issue_id != attempt.issue_id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_plan_run",
                    id: run.plan_id.clone(),
                });
            }
            if matches!(stored.phase, CodingPlanRunPhase::Invalidated) {
                return Ok(());
            }
            let mut updated = stored.clone();
            updated.phase = CodingPlanRunPhase::Invalidated;
            updated.updated_at = Utc::now().to_rfc3339();
            write_json(&run_path, &updated)?;
            Ok(())
        })
    }

    /// REQ-CS-05：暂停补记（幂等）。attempt 单文件是停止事实；此处中断可按
    /// matching completed selection 重放恢复。
    pub fn mark_paused(&self, run: &CodingPlanRun) -> Result<CodingPlanRun, ProductStoreError> {
        let run_path = self.run_path(&run.project_id, &run.issue_id, &run.plan_id)?;
        with_exclusive_lock(&run_path, || {
            let stored = self.load_run(&run_path, &run.project_id, &run.issue_id, &run.plan_id)?;
            self.verify_same_identity(&stored, run)?;
            match stored.phase {
                CodingPlanRunPhase::Paused => Ok(stored),
                CodingPlanRunPhase::Invalidated => Err(ProductStoreError::Conflict {
                    kind: "coding_plan_run_invalidated",
                    id: run.plan_id.clone(),
                }),
                _ => {
                    let mut updated = stored.clone();
                    updated.phase = CodingPlanRunPhase::Paused;
                    updated.pause_reason = Some(PLAN_CODING_CONTINUATION_NOT_ENABLED.to_string());
                    updated.updated_at = Utc::now().to_rfc3339();
                    write_json(&run_path, &updated)?;
                    Ok(updated)
                }
            }
        })
    }

    /// REQ-CS-07：现删除分支的精确 purge。核删除/归属（run 身份必须与
    /// (project,issue,plan) 全等）；缺失 OK；只有本确定安全无 writer 的删除
    /// 窗口清 run 锁（不为他处重放 unlink 活锁）。
    pub fn purge_plan(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<(), ProductStoreError> {
        for id in [project_id, issue_id, plan_id] {
            validate_relative_id(id)?;
        }
        let run_path = self.run_path(project_id, issue_id, plan_id)?;
        if !path_is_regular_file(&run_path)? {
            return Ok(());
        }
        let lock_path = self.lock_path(project_id, issue_id, plan_id)?;
        with_exclusive_lock(&run_path, || {
            let _ = self.read_run(&run_path, project_id, issue_id, plan_id)?;
            std::fs::remove_file(&run_path).map_err(|error| {
                ProductStoreError::Io(format!("remove {}: {error}", run_path.display()))
            })
        })?;
        crate::product::coding_attempt_store::remove_file_if_exists(&lock_path)
    }

    /// REQ-CS-07/D2.2：持精确 Plan lease 的删除闭包消费；scope 必须是 Plan。
    pub fn purge_plan_with_lease(
        &self,
        lease: &EntityDeletionLease,
    ) -> Result<(), ProductStoreError> {
        let scope = lease.scope();
        scope.validate()?;
        if scope.kind != DeletionEntityKind::Plan {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_plan_run_purge_scope",
                id: scope.entity_id.clone(),
            });
        }
        self.purge_plan(&scope.project_id, &scope.issue_id, &scope.entity_id)
    }

    fn scan_runs_for_attempt(
        &self,
        attempt: &CodingExecutionAttempt,
        plan_id: String,
        mode: RunAttemptMatch,
    ) -> Result<Option<CodingPlanRun>, ProductStoreError> {
        validate_relative_id(&attempt.project_id)?;
        validate_relative_id(&attempt.issue_id)?;
        validate_relative_id(&attempt.id)?;
        let root = self.runs_root(&attempt.project_id, &attempt.issue_id)?;
        let mut matches = Vec::new();
        for path in crate::product::coding_attempt_store::json_file_paths(&root)? {
            let run: CodingPlanRun = read_json(&path)?;
            if run.project_id != attempt.project_id || run.issue_id != attempt.issue_id {
                // 身份错配/坏数据不能当 None。
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_plan_run",
                    id: run.plan_id.clone(),
                });
            }
            let matched = run.plan_id == plan_id
                && run.mode == PLAN_CODING_MODE_FIRST_READY_UNIT
                && match mode {
                    RunAttemptMatch::Selection => run
                        .selection
                        .as_ref()
                        .is_some_and(|selection| selection.attempt_id == attempt.id),
                    RunAttemptMatch::Member => {
                        run.target_attempts
                            .iter()
                            .any(|binding| binding.attempt_id == attempt.id)
                            || run
                                .selection
                                .as_ref()
                                .is_some_and(|selection| selection.attempt_id == attempt.id)
                    }
                };
            if matched {
                matches.push(run);
            }
        }
        if matches.len() > 1 {
            return Err(ProductStoreError::Ambiguous {
                kind: "coding_plan_run",
                id: attempt.id.clone(),
            });
        }
        Ok(matches.into_iter().next())
    }
}

#[derive(Debug, Clone, Copy)]
enum RunAttemptMatch {
    Selection,
    Member,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::checkpoint_store::CheckpointStore;
    use crate::product::coding_models::{CodingExecutionStage, CodingUnitRun, CodingUnitRunStatus};
    use crate::product::entity_deletion_store::{DeletionEntityKind, EntityScope};
    use crate::product::entity_operation_store::EntityOperationStore;
    use crate::product::logical_codebase::{
        AggregatePolicyArtifactStore, IssueCodebaseSelection, IssueCodebaseSelectionStore,
        LogicalCodebaseFeature, LogicalCodebaseStore,
    };
    use crate::product::models::{
        WorkItemDraftCandidate, WorkItemDraftRecord, WorkItemDraftStatus,
        WorkItemDraftVerificationPlan, WorkItemGenerationMode, WorkspaceType,
    };
    use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
    use crate::product::work_item_plan_store::WorkItemPlanStore;
    use crate::product::workspace_engine::{
        AdvanceInput, AdvanceOutcome, WorkspaceEngine, WorkspaceSession,
    };
    use crate::web::test_controls::{PlanRepairFixtureControl, PlanRepairFixtureRuntime};

    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_plan_0001";
    const PLAN_ID: &str = "work_item_plan_0001";

    fn run_git(cwd: &std::path::Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .status()
            .expect("start git");
        assert!(status.success(), "git {} failed", args.join(" "));
    }

    fn plan_engine(paths: &ProductAppPaths, checkpoints_root: &std::path::Path) -> WorkspaceEngine {
        let lifecycle = LifecycleStore::new(paths.clone());
        let session_record = lifecycle
            .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .into_iter()
            .find(|session| {
                session.workspace_type == WorkspaceType::WorkItemPlan
                    && session.entity_id == PLAN_ID
            })
            .expect("plan workspace session");
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
        WorkspaceEngine::new_persistent(
            std::sync::Arc::new(CheckpointStore::new(checkpoints_root.join("checkpoints"))),
            lifecycle,
            event_tx,
            WorkspaceSession::from_record(session_record),
        )
    }

    struct N2Fixture {
        _root: tempfile::TempDir,
        paths: ProductAppPaths,
        api: crate::product::logical_codebase::LogicalRepositoryId,
        web: crate::product::logical_codebase::LogicalRepositoryId,
        revision_id: String,
        bindings: Vec<AdvanceTargetAttemptBinding>,
    }

    impl N2Fixture {
        fn coding_store(&self) -> CodingAttemptStore {
            CodingAttemptStore::new(self.paths.clone())
        }

        fn run_store(&self) -> CodingPlanRunStore {
            CodingPlanRunStore::new(self.paths.clone())
        }

        fn run_file(&self) -> PathBuf {
            self.paths
                .issue_root(PROJECT_ID, ISSUE_ID)
                .join("coding-plan-runs")
                .join(format!("{PLAN_ID}.json"))
        }

        fn api_attempt(&self) -> CodingExecutionAttempt {
            self.coding_store()
                .get_attempt_for_work_item_group(PROJECT_ID, ISSUE_ID, PLAN_ID, Some(self.api))
                .unwrap()
                .expect("api target attempt")
        }

        fn web_attempt(&self) -> CodingExecutionAttempt {
            self.coding_store()
                .get_attempt_for_work_item_group(PROJECT_ID, ISSUE_ID, PLAN_ID, Some(self.web))
                .unwrap()
                .expect("web target attempt")
        }

        fn core_unit(
            &self,
            attempt: &CodingExecutionAttempt,
        ) -> crate::product::coding_models::CodingExecutionUnit {
            self.coding_store()
                .list_coding_units(PROJECT_ID, ISSUE_ID, &attempt.id)
                .unwrap()
                .into_iter()
                .find(|unit| unit.logical_work_item_id == "wi_core")
                .expect("wi_core unit materialized in the api attempt")
        }

        fn core_selection(&self, command_id: &str) -> CodingPlanSelection {
            let attempt = self.api_attempt();
            let unit = self.core_unit(&attempt);
            CodingPlanSelection {
                unit_id: unit.id,
                logical_work_item_id: "wi_core".to_string(),
                attempt_id: attempt.id,
                target_repository_id: self.api.0.to_string(),
                start_command_id: CodingPlanSelection::deterministic_start_command_id(
                    PROJECT_ID,
                    ISSUE_ID,
                    PLAN_ID,
                    command_id,
                    &self.api.0.to_string(),
                ),
            }
        }
    }

    /// 夹具路线同 workspace_engine::tests::advance_split_targets：PlanRepair
    /// 种子（confirmed plan + 权威 revision 链）+ 两 logical 仓 + per-unit
    /// target 草稿，再 Manual advance 到 durable Ready（per-target attempts
    /// Created、units Pending）。
    async fn n2_fixture() -> N2Fixture {
        let root = tempfile::tempdir().unwrap();
        PlanRepairFixtureRuntime::seed(root.path(), PlanRepairFixtureControl::default())
            .await
            .expect("seed plan repair fixture");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let coding_store = CodingAttemptStore::new(paths.clone());
        let seeded = coding_store
            .get_attempt_for_work_item_group(PROJECT_ID, ISSUE_ID, PLAN_ID, None)
            .unwrap()
            .expect("seeded legacy group attempt");
        coding_store
            .delete_attempt(PROJECT_ID, ISSUE_ID, &seeded.id)
            .unwrap();

        let mut targets = Vec::new();
        for name in ["n2-api", "n2-web"] {
            let canonical_path = root.path().join(name);
            std::fs::create_dir_all(&canonical_path).unwrap();
            run_git(&canonical_path, &["init", "--quiet"]);
            run_git(
                &canonical_path,
                &["config", "user.email", "n2@example.test"],
            );
            run_git(&canonical_path, &["config", "user.name", "N2 Plan Run"]);
            std::fs::write(canonical_path.join("README.md"), format!("# {name}\n")).unwrap();
            run_git(&canonical_path, &["add", "README.md"]);
            run_git(
                &canonical_path,
                &["commit", "--quiet", "-m", "initial commit"],
            );
            let repository = RepositoryStore::with_logical_codebase_feature(
                paths.clone(),
                LogicalCodebaseFeature::enabled(),
            )
            .create(CreateRepositoryInput {
                project_id: PROJECT_ID.to_string(),
                name: name.to_string(),
                path: canonical_path,
                default_policy_preset: None,
                default_provider_mode: None,
                idempotency_key: format!("n2-plan-run-{name}"),
            })
            .unwrap();
            targets.push(
                repository
                    .logical_repository_id
                    .expect("logical repository ID"),
            );
        }
        let [api, web] = [targets[0], targets[1]];
        let manifest = LogicalCodebaseStore::new(paths.clone())
            .load_manifest(PROJECT_ID)
            .unwrap()
            .expect("manifest");
        AggregatePolicyArtifactStore::new(paths.clone())
            .ensure_bootstrap(&manifest)
            .unwrap();
        IssueCodebaseSelectionStore::new(paths.clone())
            .save(&IssueCodebaseSelection::explicit(
                PROJECT_ID,
                ISSUE_ID,
                targets.clone(),
                Vec::new(),
                Vec::new(),
                None,
            ))
            .unwrap();

        let revision_store = WorkItemRevisionStore::new(paths.clone());
        let lineage = revision_store
            .get_plan_lineage(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap();
        let plan_store = WorkItemPlanStore::new(paths.clone());
        for (logical_id, revision_id, target) in [
            ("wi_core", "work_item_revision_wi_core_0001", api),
            (
                "wi_registration",
                "work_item_revision_wi_registration_0001",
                api,
            ),
            ("wi_unrelated", "work_item_revision_wi_unrelated_0001", web),
        ] {
            let revision = revision_store
                .get_work_item_revision(&lineage, logical_id, revision_id)
                .unwrap();
            plan_store
                .put_draft_record(&WorkItemDraftRecord {
                    project_id: PROJECT_ID.to_string(),
                    issue_id: ISSUE_ID.to_string(),
                    plan_id: PLAN_ID.to_string(),
                    draft_id: revision.source_draft_revision_id.clone(),
                    outline_id: format!("outline_{logical_id}"),
                    generation_round_id: "round_n2_plan_run".to_string(),
                    batch_id: None,
                    attempt_index: 1,
                    outline_version_ref: "outline_version_n2".to_string(),
                    generation_mode: WorkItemGenerationMode::Serial,
                    generation_diagnostics: None,
                    candidate: WorkItemDraftCandidate {
                        target_repository_id: Some(target),
                        outline_id: format!("outline_{logical_id}"),
                        logical_work_item_id: logical_id.to_string(),
                        canonical_contract_candidate: revision.canonical_contract,
                        verification_plan: WorkItemDraftVerificationPlan { checks: Vec::new() },
                    },
                    status: WorkItemDraftStatus::Accepted,
                    active: true,
                    superseded_by_draft_id: None,
                    supersede_reason: None,
                    copied_from_draft_id: None,
                    review_node_id: None,
                    review_verdict_ref: None,
                    generated_from_node_id: "n2_plan_run_fixture".to_string(),
                    accepted_at: Some("2026-10-13T00:00:00Z".to_string()),
                    superseded_at: None,
                    created_at: "2026-10-13T00:00:00Z".to_string(),
                    updated_at: "2026-10-13T00:00:00Z".to_string(),
                })
                .unwrap();
        }

        let revision_id = lineage
            .active_revision_id
            .clone()
            .expect("active plan revision");

        let mut engine = plan_engine(&paths, root.path());
        let outcome = engine
            .handle_advance(AdvanceInput {
                command_id: "cmd_n2_advance_seed".to_string(),
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                plan_id: PLAN_ID.to_string(),
            })
            .await
            .expect("manual advance reaches durable Ready");
        let AdvanceOutcome::Completed {
            target_attempts, ..
        } = outcome
        else {
            panic!("expected advance Completed, got {outcome:?}");
        };
        assert_eq!(target_attempts.len(), 2, "one attempt per logical target");

        N2Fixture {
            _root: root,
            paths,
            api,
            web,
            revision_id,
            bindings: target_attempts,
        }
    }

    /// 同 command/revision 重放冻结身份；异 revision/graph 拒绝且不写。
    #[tokio::test]
    async fn plan_coding_n2_command_replay_freezes_identity() {
        let fixture = n2_fixture().await;
        let run_store = fixture.run_store();
        let selection = fixture.core_selection("cmd_n2_user_0001");

        let run = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_n2_user_0001",
                &fixture.revision_id,
            )
            .unwrap();
        assert_eq!(run.phase, CodingPlanRunPhase::Prepared);
        assert_eq!(run.command_id, "cmd_n2_user_0001");
        assert_eq!(run.mode, PLAN_CODING_MODE_FIRST_READY_UNIT);
        assert!(run.target_attempts.is_empty());
        assert!(!run.dependency_graph_revision_id.is_empty());

        let bound = run_store
            .bind_ready(&run, &fixture.bindings, Some(selection.clone()))
            .unwrap();
        assert_eq!(bound.phase, CodingPlanRunPhase::ReadyBound);
        assert_eq!(bound.selection.as_ref(), Some(&selection));

        // 同 command/revision 重放：原 run 原样返回（冻结不漂移）。
        let replay = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_n2_user_0001",
                &fixture.revision_id,
            )
            .unwrap();
        assert_eq!(replay.command_id, "cmd_n2_user_0001");
        assert_eq!(replay.created_at, bound.created_at);
        assert_eq!(replay.selection.as_ref(), Some(&selection));
        assert_eq!(replay.phase, CodingPlanRunPhase::ReadyBound);

        // 异 revision：拒绝且不改写 run。
        let error = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_n2_user_0001",
                "work_item_plan_revision_drift",
            )
            .unwrap_err();
        assert!(
            matches!(error, ProductStoreError::IdentityMismatch { ref kind, .. } if *kind == "coding_plan_run_revision"),
            "unexpected error: {error}"
        );
        assert_eq!(
            run_store
                .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
                .unwrap()
                .expect("run survives drift rejection")
                .command_id,
            "cmd_n2_user_0001"
        );

        // graph 漂移：篡改 run 文件 graph revision 后 prepare 拒绝且不复活。
        let mut tampered = bound.clone();
        tampered.dependency_graph_revision_id = "dependency_graph_revision_tampered".to_string();
        write_json(&fixture.run_file(), &tampered).unwrap();
        let error = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_n2_user_0001",
                &fixture.revision_id,
            )
            .unwrap_err();
        assert!(
            matches!(error, ProductStoreError::IdentityMismatch { ref kind, .. } if *kind == "coding_plan_run_graph"),
            "unexpected error: {error}"
        );
        let persisted: CodingPlanRun = read_json(&fixture.run_file()).unwrap();
        assert_eq!(
            persisted.dependency_graph_revision_id, "dependency_graph_revision_tampered",
            "drift rejection must not rewrite the run file"
        );
    }

    /// 同 plan/revision 第二 command 返回原 active run（原 command/selection）。
    #[tokio::test]
    async fn plan_coding_n2_second_command_returns_active_run() {
        let fixture = n2_fixture().await;
        let run_store = fixture.run_store();
        let selection = fixture.core_selection("cmd_n2_user_first");

        let run = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_n2_user_first",
                &fixture.revision_id,
            )
            .unwrap();
        let bound = run_store
            .bind_ready(&run, &fixture.bindings, Some(selection))
            .unwrap();
        let requested = run_store.mark_start_requested(&bound).unwrap();
        assert_eq!(requested.phase, CodingPlanRunPhase::StartRequested);

        // 第二 command（重复点击）：返回原 run，不建第二 claim 账本。
        let second = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_n2_user_second",
                &fixture.revision_id,
            )
            .unwrap();
        assert_eq!(second.command_id, "cmd_n2_user_first");
        assert_eq!(second.phase, CodingPlanRunPhase::StartRequested);
        let selection = second.selection.expect("selection frozen");
        assert_eq!(
            selection.start_command_id,
            CodingPlanSelection::deterministic_start_command_id(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_n2_user_first",
                &fixture.api.0.to_string(),
            ),
            "start command stays derived from the original command"
        );
        // 仍只有一个 run 文件（每 plan 至多一个 run 记录）。
        let runs_root = fixture
            .paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("coding-plan-runs");
        assert_eq!(
            crate::product::coding_attempt_store::json_file_paths(&runs_root)
                .unwrap()
                .len(),
            1
        );
    }

    /// 异身份不写；首次绑定不收养已有非未首启集合；target/selection 冻结后
    /// 漂移拒绝。
    #[tokio::test]
    async fn plan_coding_n2_identity_drift_and_existing_started_set_rejected() {
        let fixture = n2_fixture().await;
        let run_store = fixture.run_store();
        let coding_store = fixture.coding_store();

        // 异 revision：直接拒绝，不落 run 文件。
        let error = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_n2_drift",
                "work_item_plan_revision_missing",
            )
            .unwrap_err();
        assert!(matches!(error, ProductStoreError::IdentityMismatch { .. }));
        assert!(
            run_store
                .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
                .unwrap()
                .is_none()
        );

        let run = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_n2_drift",
                &fixture.revision_id,
            )
            .unwrap();

        // 已有非未首启集合：web attempt 已 Running → 首次绑定拒绝（不收养）。
        let web_attempt = fixture.web_attempt();
        coding_store
            .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &web_attempt.id)
            .unwrap();
        let error = run_store
            .bind_ready(&run, &fixture.bindings, None)
            .unwrap_err();
        assert!(
            matches!(error, ProductStoreError::Conflict { ref kind, .. } if *kind == "coding_plan_run_set_not_first_start"),
            "unexpected error: {error}"
        );
        assert!(
            run_store
                .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
                .unwrap()
                .expect("run still prepared")
                .target_attempts
                .is_empty(),
            "rejected first bind must not durably adopt the started set"
        );

        // 归还未首启状态后绑定成功。
        let mut restored = coding_store
            .get_attempt(PROJECT_ID, ISSUE_ID, &web_attempt.id)
            .unwrap();
        restored.status = CodingAttemptStatus::Created;
        restored.admission_ticket_consumed_at = None;
        coding_store
            .write_coding_attempt_for_test(&restored)
            .unwrap();
        let bound = run_store.bind_ready(&run, &fixture.bindings, None).unwrap();
        assert_eq!(bound.phase, CodingPlanRunPhase::ReadyBound);
        assert!(bound.selection.is_none());

        // target 漂移：绑定集缩水拒绝，冻结集合不变。
        let shrunk: Vec<_> = fixture
            .bindings
            .iter()
            .filter(|binding| binding.attempt_id != web_attempt.id)
            .cloned()
            .collect();
        let error = run_store.bind_ready(&bound, &shrunk, None).unwrap_err();
        assert!(
            matches!(error, ProductStoreError::IdentityMismatch { ref kind, .. } if *kind == "coding_plan_run_target_attempts"),
            "unexpected error: {error}"
        );
        assert_eq!(
            run_store
                .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
                .unwrap()
                .unwrap()
                .target_attempts,
            fixture.bindings
        );

        // selection 冻结后漂移拒绝：先冻结 wi_core，再用 web 首项尝试改选。
        let core_selection = fixture.core_selection("cmd_n2_drift");
        let frozen = run_store
            .bind_ready(&bound, &fixture.bindings, Some(core_selection.clone()))
            .unwrap();
        assert_eq!(frozen.selection.as_ref(), Some(&core_selection));
        let web_attempt = fixture.web_attempt();
        let web_unit = coding_store
            .list_coding_units(PROJECT_ID, ISSUE_ID, &web_attempt.id)
            .unwrap()
            .into_iter()
            .find(|unit| unit.logical_work_item_id == "wi_unrelated")
            .expect("web unit");
        let other_selection = CodingPlanSelection {
            unit_id: web_unit.id,
            logical_work_item_id: "wi_unrelated".to_string(),
            attempt_id: web_attempt.id,
            target_repository_id: fixture.web.0.to_string(),
            start_command_id: CodingPlanSelection::deterministic_start_command_id(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_n2_drift",
                &fixture.web.0.to_string(),
            ),
        };
        let error = run_store
            .bind_ready(&frozen, &fixture.bindings, Some(other_selection))
            .unwrap_err();
        assert!(
            matches!(error, ProductStoreError::IdentityMismatch { ref kind, .. } if *kind == "coding_plan_run_selection"),
            "unexpected error: {error}"
        );
        assert_eq!(
            run_store
                .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
                .unwrap()
                .unwrap()
                .selection,
            Some(core_selection),
            "selection stays frozen"
        );
    }

    /// ready_bound 重启保留 selection；无 ready 等待（selection=None）不能发
    /// 空首启；ready 出现后补冻结 selection。
    #[tokio::test]
    async fn plan_coding_n2_readybound_restart_keeps_selection() {
        let fixture = n2_fixture().await;
        let run_store = fixture.run_store();

        let run = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_n2_restart",
                &fixture.revision_id,
            )
            .unwrap();
        // 无 ready：仍 durable 绑定全集，selection=None，保持 ready_bound 等待。
        let waiting = run_store.bind_ready(&run, &fixture.bindings, None).unwrap();
        assert_eq!(waiting.phase, CodingPlanRunPhase::ReadyBound);
        assert_eq!(waiting.target_attempts, fixture.bindings);
        assert!(waiting.selection.is_none());

        // 空首启拒绝：mark_start_requested 必须有冻结 selection。
        let error = run_store.mark_start_requested(&waiting).unwrap_err();
        assert!(
            matches!(error, ProductStoreError::Conflict { ref kind, .. } if *kind == "coding_plan_run_selection_missing"),
            "unexpected error: {error}"
        );

        // 重启（新 store 实例）后状态与等待事实保留。
        let restarted = CodingPlanRunStore::new(fixture.paths.clone());
        let reloaded = restarted
            .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap()
            .expect("run survives restart");
        assert_eq!(reloaded.phase, CodingPlanRunPhase::ReadyBound);
        assert!(reloaded.selection.is_none());

        // ready 出现：补冻结 selection（一次冻结）。
        let core_selection = fixture.core_selection("cmd_n2_restart");
        let selected = restarted
            .bind_ready(&reloaded, &fixture.bindings, Some(core_selection.clone()))
            .unwrap();
        assert_eq!(selected.phase, CodingPlanRunPhase::ReadyBound);
        assert_eq!(selected.selection.as_ref(), Some(&core_selection));

        // 再次重启：selection（含确定性首启 command）保留，可推进 start_requested。
        let restarted_again = CodingPlanRunStore::new(fixture.paths.clone());
        let reselected = restarted_again
            .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap()
            .expect("run survives second restart");
        let selection = reselected
            .selection
            .clone()
            .expect("selection kept across restart");
        assert_eq!(selection.start_command_id, core_selection.start_command_id);
        validate_relative_id(&selection.start_command_id).unwrap();

        let requested = restarted_again.mark_start_requested(&reselected).unwrap();
        assert_eq!(requested.phase, CodingPlanRunPhase::StartRequested);
        // start_requested 后重放幂等（中断窗口读同一 selection）。
        let replayed = restarted_again.mark_start_requested(&requested).unwrap();
        assert_eq!(replayed.phase, CodingPlanRunPhase::StartRequested);
        assert_eq!(replayed.selection.as_ref(), Some(&core_selection));
    }

    /// 匹配 Completed 首项可 Blocked+CodeReview 空指针/空 actions，integrity
    /// 认可合法 pause 且 provider 拒绝；无本 run 的同形状坏指针仍拒绝。
    #[tokio::test]
    async fn plan_coding_n2_pause_pointer_and_provider_admission() {
        let fixture = n2_fixture().await;
        let run_store = fixture.run_store();
        let coding_store = fixture.coding_store();

        let selection = fixture.core_selection("cmd_n2_pause");
        let run = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_n2_pause",
                &fixture.revision_id,
            )
            .unwrap();
        let bound = run_store
            .bind_ready(&run, &fixture.bindings, Some(selection.clone()))
            .unwrap();
        let requested = run_store.mark_start_requested(&bound).unwrap();
        let started = run_store
            .record_start_outcome(
                &requested,
                CodingPlanStartOutcome::Started {
                    attempt_id: selection.attempt_id.clone(),
                },
            )
            .unwrap();
        assert_eq!(started.phase, CodingPlanRunPhase::Started);

        // 执行事实：attempt Running（含 admission 标记）+ 首项 Completed 的
        // completion commit/权威 unit run/handoff。
        coding_store
            .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &selection.attempt_id)
            .unwrap();
        let attempt = coding_store
            .get_attempt(PROJECT_ID, ISSUE_ID, &selection.attempt_id)
            .unwrap();
        let unit = fixture.core_unit(&attempt);
        let now = Utc::now().to_rfc3339();
        let unit_run = CodingUnitRun {
            id: "coding_unit_run_n2_0001".to_string(),
            unit_id: unit.id.clone(),
            execution_no: 1,
            work_item_revision_id: unit.work_item_revision_id.clone(),
            resolved_handoff_revision_ids: Vec::new(),
            canonical_contract_hash: "n2_canonical_hash".to_string(),
            projection_bundle_id: "projection_bundle_n2_0001".to_string(),
            projection_compiler_version: "n2-compiler".to_string(),
            coder_provider_renderer_version: "n2-coder".to_string(),
            reviewer_provider_renderer_version: "n2-reviewer".to_string(),
            internal_reviewer_provider_renderer_version: None,
            coder_projection_hash: "n2-coder-hash".to_string(),
            reviewer_projection_hash: "n2-reviewer-hash".to_string(),
            coder_execution_context_hash: None,
            reviewer_execution_context_hash: None,
            internal_reviewer_execution_context_hash: None,
            status: CodingUnitRunStatus::Running,
            unit_rework_count: 0,
            verification_retry_count: 0,
            operational_retry_count: 0,
            plan_repair_count: 0,
            start_commit: None,
            completion_commit: None,
            created_at: now.clone(),
            updated_at: now,
        };
        coding_store
            .create_coding_unit_run(&attempt, &unit_run)
            .unwrap();
        coding_store
            .complete_coding_unit_run(&attempt, &unit_run.id, "n2_commit_0001")
            .unwrap();
        coding_store
            .update_coding_unit_status(
                PROJECT_ID,
                ISSUE_ID,
                &attempt.id,
                &unit.id,
                crate::product::coding_models::CodingExecutionUnitStatus::Completed,
                Some("wi_core completed".to_string()),
            )
            .unwrap();
        coding_store
            .update_coding_unit_latest_handoff_revision_id(
                PROJECT_ID,
                ISSUE_ID,
                &attempt.id,
                &unit.id,
                Some("handoff_revision_n2_0001".to_string()),
            )
            .unwrap();
        coding_store
            .update_coding_unit_completion_commit(
                PROJECT_ID,
                ISSUE_ID,
                &attempt.id,
                &unit.id,
                Some("n2_commit_0001".to_string()),
            )
            .unwrap();
        let running = coding_store
            .get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id)
            .unwrap();
        assert!(
            running.active_unit_id.is_none(),
            "completion cleared the pointer"
        );

        // 清指针后、pause 写回前的中断窗口：provider 必须拒绝（不进 selector/
        // provider/push）。
        let error = coding_store
            .ensure_provider_run_allowed(&running)
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("plan_coding_first_unit_completed"),
            "unexpected admission result: {error}"
        );

        // 首项完成安全暂停：Blocked+CodeReview/清指针/空 actions gate/run Paused。
        let paused = coding_store
            .pause_completed_plan_first_unit(&running, &unit.id)
            .unwrap()
            .expect("matched first ready unit pauses");
        assert_eq!(paused.status, CodingAttemptStatus::Blocked);
        assert_eq!(paused.stage, CodingExecutionStage::CodeReview);
        assert!(paused.active_unit_id.is_none());
        assert!(paused.current_work_item_id.is_none());

        let gates = coding_store
            .list_open_blocked_gates(PROJECT_ID, ISSUE_ID, &paused.id)
            .unwrap();
        assert_eq!(gates.len(), 1, "exactly one pause gate");
        assert_eq!(
            gates[0].reason_code.as_deref(),
            Some(PLAN_CODING_CONTINUATION_NOT_ENABLED)
        );
        assert!(
            gates[0].available_actions.is_empty(),
            "pause gate has no actions"
        );

        let run = run_store
            .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap()
            .expect("run present");
        assert_eq!(run.phase, CodingPlanRunPhase::Paused);
        assert_eq!(
            run.pause_reason.as_deref(),
            Some(PLAN_CODING_CONTINUATION_NOT_ENABLED)
        );

        // 合法 pause 分支：integrity 认可（严格匹配 first_ready selected 完成身份）。
        coding_store
            .validate_group_attempt_integrity(&paused)
            .expect("legal plan-coding pause passes pointer validation");

        // provider 准入拒绝（暂停不可被普通 retry/StartCoding/gate response 绕行）。
        let error = coding_store
            .ensure_provider_run_allowed(&paused)
            .unwrap_err();
        assert!(
            error.to_string().contains("plan_coding_paused"),
            "unexpected admission result: {error}"
        );

        // 幂等：重复 pause 不再新增 gate、不改 run。
        let paused_again = coding_store
            .pause_completed_plan_first_unit(&paused, &unit.id)
            .unwrap()
            .expect("idempotent pause");
        assert_eq!(paused_again.status, CodingAttemptStatus::Blocked);
        assert_eq!(
            coding_store
                .list_open_blocked_gates(PROJECT_ID, ISSUE_ID, &paused.id)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            run_store
                .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
                .unwrap()
                .unwrap()
                .phase,
            CodingPlanRunPhase::Paused
        );

        // 无本 run 的同形状仍是坏指针：purge 后 integrity 回到失败关闭。
        run_store.purge_plan(PROJECT_ID, ISSUE_ID, PLAN_ID).unwrap();
        let error = coding_store
            .validate_group_attempt_integrity(&paused)
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("attempt has no legal active or resume target"),
            "bad pointer without a matching run must stay rejected: {error}"
        );
    }

    /// 单成员删除使授权 Invalidated；迟到写不复活；purge 清 run 与锁、保留
    /// 他 plan；lease scope 校验；NotFound/None 分类不吞。
    #[tokio::test]
    async fn plan_coding_n2_late_write_and_deleted_member_invalidates() {
        let fixture = n2_fixture().await;
        let run_store = fixture.run_store();
        let coding_store = fixture.coding_store();

        let selection = fixture.core_selection("cmd_n2_delete");
        let run = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_n2_delete",
                &fixture.revision_id,
            )
            .unwrap();
        let bound = run_store
            .bind_ready(&run, &fixture.bindings, Some(selection.clone()))
            .unwrap();
        let requested = run_store.mark_start_requested(&bound).unwrap();

        // 成员删除（非 selected 的 web attempt）→ 授权 Invalidated。
        let web_attempt = fixture.web_attempt();
        run_store
            .invalidate_for_deleted_attempt(&web_attempt)
            .unwrap();
        let invalidated = run_store
            .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap()
            .expect("run present");
        assert_eq!(invalidated.phase, CodingPlanRunPhase::Invalidated);
        assert!(
            !invalidated.target_attempts.is_empty(),
            "invalidation keeps the frozen set for forensics"
        );

        // 迟到写全部拒绝：不复活授权、不自动补建。
        let error = run_store.mark_start_requested(&requested).unwrap_err();
        assert!(
            matches!(error, ProductStoreError::Conflict { ref kind, .. } if *kind == "coding_plan_run_invalidated"),
            "unexpected error: {error}"
        );
        let error = run_store
            .record_start_outcome(
                &requested,
                CodingPlanStartOutcome::Started {
                    attempt_id: selection.attempt_id.clone(),
                },
            )
            .unwrap_err();
        assert!(matches!(error, ProductStoreError::Conflict { .. }));
        let error = run_store
            .bind_ready(&bound, &fixture.bindings, Some(selection.clone()))
            .unwrap_err();
        assert!(
            matches!(error, ProductStoreError::Conflict { ref kind, .. } if *kind == "coding_plan_run_invalidated"),
            "unexpected error: {error}"
        );
        let error = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_n2_delete",
                &fixture.revision_id,
            )
            .unwrap_err();
        assert!(
            matches!(error, ProductStoreError::Conflict { ref kind, .. } if *kind == "coding_plan_run_not_active"),
            "no re-authorization over an invalidated run: {error}"
        );
        let error = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_n2_delete_late",
                &fixture.revision_id,
            )
            .unwrap_err();
        assert!(matches!(error, ProductStoreError::Conflict { .. }));

        // 幂等：重复 invalidate OK，冻结 selection 不被抹除。
        run_store
            .invalidate_for_deleted_attempt(&web_attempt)
            .unwrap();
        assert_eq!(
            run_store
                .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
                .unwrap()
                .unwrap()
                .phase,
            CodingPlanRunPhase::Invalidated
        );
        assert_eq!(
            run_store
                .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
                .unwrap()
                .unwrap()
                .selection
                .expect("selection retained"),
            selection,
            "late writes must not resurrect but history stays"
        );

        // 无匹配 run 的 attempt：直接 OK（不自动补建）。
        let mut foreign = fixture.api_attempt();
        foreign.work_item_group_id = Some("work_item_plan_other".to_string());
        run_store.invalidate_for_deleted_attempt(&foreign).unwrap();
        assert!(
            run_store
                .get(PROJECT_ID, ISSUE_ID, "work_item_plan_other")
                .unwrap()
                .is_none(),
            "no run is auto-created for unrelated attempts"
        );

        // purge：清本 plan run 与锁、保留他 plan 的 run；缺失幂等 OK。
        let other_run = CodingPlanRun {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: "work_item_plan_other".to_string(),
            command_id: "cmd_n2_other".to_string(),
            bound_plan_revision_id: fixture.revision_id.clone(),
            dependency_graph_revision_id: "dependency_graph_revision_other".to_string(),
            target_attempts: Vec::new(),
            mode: PLAN_CODING_MODE_FIRST_READY_UNIT.to_string(),
            selection: None,
            phase: CodingPlanRunPhase::Prepared,
            start_outcome: None,
            pause_reason: None,
            created_at: "2026-10-13T00:00:00Z".to_string(),
            updated_at: "2026-10-13T00:00:00Z".to_string(),
        };
        let other_path = fixture
            .paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("coding-plan-runs")
            .join("work_item_plan_other.json");
        write_json(&other_path, &other_run).unwrap();

        run_store.purge_plan(PROJECT_ID, ISSUE_ID, PLAN_ID).unwrap();
        assert!(
            run_store
                .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
                .unwrap()
                .is_none()
        );
        assert!(
            run_store
                .get(PROJECT_ID, ISSUE_ID, "work_item_plan_other")
                .unwrap()
                .is_some(),
            "other plan's run survives the purge"
        );
        let lock_path = fixture
            .paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("coding-plan-runs")
            .join(format!(".{PLAN_ID}.json.lock"));
        assert!(
            !lock_path.exists(),
            "run lock is cleared in the deletion window"
        );
        // 缺失幂等 OK；对缺失 run 的迟到写是 NotFound（不当 None 吞掉）。
        run_store.purge_plan(PROJECT_ID, ISSUE_ID, PLAN_ID).unwrap();
        let error = run_store
            .bind_ready(&invalidated, &fixture.bindings, None)
            .unwrap_err();
        assert!(
            matches!(error, ProductStoreError::NotFound { ref kind, .. } if *kind == "coding_plan_run"),
            "missing run must surface NotFound, not None: {error}"
        );

        // purge_plan_with_lease：Plan scope 精确消费；非 Plan scope 拒绝。
        let operations = EntityOperationStore::new(fixture.paths.clone());
        let scope = EntityScope::new(PROJECT_ID, ISSUE_ID, DeletionEntityKind::Plan, PLAN_ID);
        let lease = operations.try_acquire_delete(&scope).unwrap();
        run_store.purge_plan_with_lease(&lease).unwrap();
        let wrong_scope = EntityScope::new(
            PROJECT_ID,
            ISSUE_ID,
            DeletionEntityKind::Story,
            "story_spec_0001",
        );
        let wrong_lease = operations.try_acquire_delete(&wrong_scope).unwrap();
        let error = run_store.purge_plan_with_lease(&wrong_lease).unwrap_err();
        assert!(
            matches!(error, ProductStoreError::IdentityMismatch { ref kind, .. } if *kind == "coding_plan_run_purge_scope"),
            "unexpected error: {error}"
        );
    }
}
