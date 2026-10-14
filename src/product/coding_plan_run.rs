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
use crate::product::entity_deletion_store::{DeletionEntityKind, EntityDeletionStore, EntityScope};
use crate::product::entity_operation_store::{EntityDeletionLease, EntityOperationStore};
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::IssueWorkItemPlanStatus;
use crate::product::work_item_revision_store::WorkItemRevisionStore;

/// REQ-CS-05：first_ready_unit 首项完成安全暂停的原因码（blocked gate 与
/// run pause 共用同一冻结值）。
pub const PLAN_CODING_CONTINUATION_NOT_ENABLED: &str = "plan_coding_continuation_not_enabled";
/// 今晚唯一模式：一次显式授权只运行全局首个 ready WI。
pub const PLAN_CODING_MODE_FIRST_READY_UNIT: &str = "first_ready_unit";

/// H2a（设计 2026-10-14 A 节）：plan 执行轮生命周期元数据目录（冻结地址）
/// `{issue_root}/coding-plan-workspace-lifecycles/`。其锁为同目录前导点
/// `.{plan_id}.json.lock`（稳定 inode）：workspace 重置清理绝不删除；仅
/// 真实 Plan 删除闭包（D2.2/H4）负责纳入清理。
pub const CODING_PLAN_WORKSPACE_LIFECYCLES_DIR: &str = "coding-plan-workspace-lifecycles";

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

/// H2a（设计 2026-10-14 A 节）：clearing 中的纯 ID pending 清单——冻结的
/// 本轮清理对象（attempt/advance/retry/target），失败保持可续清的事实源。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingPlanWorkspaceClearing {
    pub command_id: String,
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_attempt_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_advance_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_retry_command_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_target_repository_ids: Vec<String>,
    pub started_at: String,
}

/// H2a：重置回执（纯 ID/状态）——同 command 重试据此返回/续做这一次操作，
/// 不再次清新轮。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingPlanWorkspaceResetReceipt {
    pub command_id: String,
    pub from_generation: u64,
    pub to_generation: u64,
    pub completed_at: String,
}

/// H2a：plan 执行轮生命周期元数据（`{issue_root}/coding-plan-workspace-lifecycles/
/// {plan_id}.json`）。只有身份/状态：单调 generation、已退役 start/advance
/// command 与 attempt/advance ID、clearing pending 清单与操作回执；无旧日志、
/// unit 内容、完整 run 归档；不是第二 claim/scheduler 账本。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingPlanWorkspaceLifecycle {
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retired_command_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retired_attempt_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retired_advance_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clearing: Option<CodingPlanWorkspaceClearing>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub receipts: Vec<CodingPlanWorkspaceResetReceipt>,
    pub created_at: String,
    pub updated_at: String,
}

/// H2a：闭包在 Plan 独占租约内采集后传入 `retire_workspace_round_with_lease`
/// 的冻结清单（纯 ID）：retired_* 为退役围栏事实，pending_* 为清理对象。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingPlanWorkspaceResetChecklist {
    pub command_id: String,
    pub retired_command_ids: Vec<String>,
    pub retired_attempt_ids: Vec<String>,
    pub retired_advance_ids: Vec<String>,
    pub pending_attempt_ids: Vec<String>,
    pub pending_advance_ids: Vec<String>,
    pub pending_retry_command_ids: Vec<String>,
    pub pending_target_repository_ids: Vec<String>,
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

    fn plan_scope(
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<EntityScope, ProductStoreError> {
        let scope = EntityScope::new(project_id, issue_id, DeletionEntityKind::Plan, plan_id);
        scope.validate()?;
        Ok(scope)
    }

    /// plan §0.2 owner 写准入（同 `AdvanceStore::with_plan_live_write` 惯例，
    /// 审查修复轮 P2）：scope try-shared 外层——争用/墓碑一律立即
    /// `Conflict{entity_deleting}`（零阻塞等待），再进 run 文件锁内层；锁内
    /// 重读/CAS 语义不变。迟到 coding-start 在删除窗口被拒，不重建可重放
    /// 孤儿 run（REQ-CS-07）。
    fn with_plan_live_write<T>(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        operation: impl FnOnce() -> Result<T, ProductStoreError>,
    ) -> Result<T, ProductStoreError> {
        let scope = Self::plan_scope(project_id, issue_id, plan_id)?;
        EntityOperationStore::new(self.paths.clone()).with_live_write(&scope, operation)
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
        self.with_plan_live_write(project_id, issue_id, plan_id, || {
            with_exclusive_lock(&run_path, || {
                // H2a 退役围栏先行：clearing/退役 command 不能准备新轮
                // （不能因活动 run 文件缺失重新准备）。
                self.verify_workspace_round_admission(project_id, issue_id, plan_id, command_id)?;
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
                    if stored.dependency_graph_revision_id
                        != authoritative.dependency_graph_revision_id
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
        self.with_plan_live_write(&run.project_id, &run.issue_id, &run.plan_id, || {
            with_exclusive_lock(&run_path, || {
                // H2a 退役围栏：clearing/退役 command 的 run 更新一律拒绝。
                self.verify_workspace_round_admission(
                    &run.project_id,
                    &run.issue_id,
                    &run.plan_id,
                    &run.command_id,
                )?;
                let stored =
                    self.load_run(&run_path, &run.project_id, &run.issue_id, &run.plan_id)?;
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
                            if attempt_target.as_deref()
                                != Some(binding.target_repository_id.as_str())
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
        self.with_plan_live_write(&run.project_id, &run.issue_id, &run.plan_id, || {
            with_exclusive_lock(&run_path, || {
                // H2a 退役围栏：clearing/退役 command 的 run 更新一律拒绝。
                self.verify_workspace_round_admission(
                    &run.project_id,
                    &run.issue_id,
                    &run.plan_id,
                    &run.command_id,
                )?;
                let stored =
                    self.load_run(&run_path, &run.project_id, &run.issue_id, &run.plan_id)?;
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
        self.with_plan_live_write(&run.project_id, &run.issue_id, &run.plan_id, || {
            with_exclusive_lock(&run_path, || {
                // H2a 退役围栏：clearing/退役 command 的 run 更新一律拒绝。
                self.verify_workspace_round_admission(
                    &run.project_id,
                    &run.issue_id,
                    &run.plan_id,
                    &run.command_id,
                )?;
                let stored =
                    self.load_run(&run_path, &run.project_id, &run.issue_id, &run.plan_id)?;
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
                    CodingPlanStartOutcome::AlreadyStarted { .. } => {
                        CodingPlanRunPhase::AlreadyStarted
                    }
                    CodingPlanStartOutcome::NeedsHuman { .. } => CodingPlanRunPhase::NeedsHuman,
                };
                let mut updated = stored.clone();
                updated.phase = phase;
                updated.start_outcome = Some(outcome);
                updated.updated_at = Utc::now().to_rfc3339();
                write_json(&run_path, &updated)?;
                Ok(updated)
            })
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
        self.with_plan_live_write(&run.project_id, &run.issue_id, &run.plan_id, || {
            with_exclusive_lock(&run_path, || {
                self.invalidate_run_file_locked(
                    &run_path,
                    &attempt.project_id,
                    &attempt.issue_id,
                    &run.plan_id,
                )
            })
        })
    }

    /// H2a（设计 2026-10-14 A 节）：消费已持 Plan 独占租约的成员删除失效。
    /// 持租约清理闭包内调用，不二次 try-shared——原公开路径在独占租约下会
    /// 以 `entity_deleting` 自拒；与公开闭包共用 `invalidate_run_file_locked`。
    pub fn invalidate_for_deleted_attempt_with_lease(
        &self,
        lease: &EntityDeletionLease,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), ProductStoreError> {
        let Some(plan_id) = attempt.work_item_group_id.clone() else {
            return Ok(());
        };
        Self::verify_plan_lease_identity(lease, &attempt.project_id, &attempt.issue_id, &plan_id)?;
        let Some(run) = self.scan_runs_for_attempt(attempt, plan_id, RunAttemptMatch::Member)?
        else {
            return Ok(());
        };
        let run_path = self.run_path(&run.project_id, &run.issue_id, &run.plan_id)?;
        with_exclusive_lock(&run_path, || {
            self.invalidate_run_file_locked(
                &run_path,
                &attempt.project_id,
                &attempt.issue_id,
                &run.plan_id,
            )
        })
    }

    /// H2a：闭包冻结清单时把现行 run 标 Invalidated（消费已持 Plan 独占
    /// 租约；幂等；原授权 Invalidated 阻断新的本轮首启）。
    pub fn invalidate_run_for_reset_with_lease(
        &self,
        lease: &EntityDeletionLease,
        run: &CodingPlanRun,
    ) -> Result<(), ProductStoreError> {
        Self::verify_plan_lease_identity(lease, &run.project_id, &run.issue_id, &run.plan_id)?;
        let run_path = self.run_path(&run.project_id, &run.issue_id, &run.plan_id)?;
        with_exclusive_lock(&run_path, || {
            self.invalidate_run_file_locked(&run_path, &run.project_id, &run.issue_id, &run.plan_id)
        })
    }

    /// 锁内失效核体：重读 run（NotFound 上抛不吞）、核 (project,issue) 身份、
    /// 幂等置 Invalidated（终局，冻结集合/selection 保留不抹除）。
    fn invalidate_run_file_locked(
        &self,
        run_path: &Path,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<(), ProductStoreError> {
        let stored = self.load_run(run_path, project_id, issue_id, plan_id)?;
        if stored.project_id != project_id || stored.issue_id != issue_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_plan_run",
                id: plan_id.to_string(),
            });
        }
        if matches!(stored.phase, CodingPlanRunPhase::Invalidated) {
            return Ok(());
        }
        let mut updated = stored.clone();
        updated.phase = CodingPlanRunPhase::Invalidated;
        updated.updated_at = Utc::now().to_rfc3339();
        write_json(run_path, &updated)
    }

    /// Plan 独占租约身份核对：scope 必须与 (project,issue,plan) 全等。
    fn verify_plan_lease_identity(
        lease: &EntityDeletionLease,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<(), ProductStoreError> {
        let scope = lease.scope();
        scope.validate()?;
        if scope.kind != DeletionEntityKind::Plan
            || scope.project_id != project_id
            || scope.issue_id != issue_id
            || scope.entity_id != plan_id
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_plan_run_lease_scope",
                id: plan_id.to_string(),
            });
        }
        Ok(())
    }

    /// REQ-CS-05：暂停补记（幂等）。attempt 单文件是停止事实；此处中断可按
    /// matching completed selection 重放恢复。
    pub fn mark_paused(&self, run: &CodingPlanRun) -> Result<CodingPlanRun, ProductStoreError> {
        let run_path = self.run_path(&run.project_id, &run.issue_id, &run.plan_id)?;
        self.with_plan_live_write(&run.project_id, &run.issue_id, &run.plan_id, || {
            with_exclusive_lock(&run_path, || {
                // H2a 退役围栏：clearing/退役 command 的 run 更新一律拒绝。
                self.verify_workspace_round_admission(
                    &run.project_id,
                    &run.issue_id,
                    &run.plan_id,
                    &run.command_id,
                )?;
                let stored =
                    self.load_run(&run_path, &run.project_id, &run.issue_id, &run.plan_id)?;
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
                        updated.pause_reason =
                            Some(PLAN_CODING_CONTINUATION_NOT_ENABLED.to_string());
                        updated.updated_at = Utc::now().to_rfc3339();
                        write_json(&run_path, &updated)?;
                        Ok(updated)
                    }
                }
            })
        })
    }

    /// REQ-CS-07：现删除分支的精确 purge。核删除/归属（run 身份必须与
    /// (project,issue,plan) 全等）；缺失 OK。run 文件删除恒受 run 锁互斥；
    /// 锁文件 unlink 仅在 Plan scope 墓碑在位的删除窗口执行——墓碑在位后
    /// 任何 run writer 都被 `with_plan_live_write` 以 `entity_deleting` 拒，
    /// 不再有新 writer 进入，unlink 无 inode 双 writer 风险；非窗口调用不
    /// 清锁（残留锁文件无授权语义，留待真正删除窗口清理，不为他处重放
    /// unlink 活锁）。
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
        if path_is_regular_file(&run_path)? {
            with_exclusive_lock(&run_path, || {
                let _ = self.read_run(&run_path, project_id, issue_id, plan_id)?;
                std::fs::remove_file(&run_path).map_err(|error| {
                    ProductStoreError::Io(format!("remove {}: {error}", run_path.display()))
                })
            })?;
        }
        // run 缺失不短路锁清理：删除窗口重入（run 已清、墓碑在位）仍需
        // 幂等清锁；无墓碑则保持只读退出（Ok）。
        let scope = Self::plan_scope(project_id, issue_id, plan_id)?;
        if EntityDeletionStore::new(self.paths.clone())
            .get(&scope)?
            .is_some()
        {
            let lock_path = self.lock_path(project_id, issue_id, plan_id)?;
            return crate::product::coding_attempt_store::remove_file_if_exists(&lock_path);
        }
        Ok(())
    }

    /// REQ-CS-07/D2.2：持精确 Plan lease 的删除闭包消费；scope 必须是 Plan。
    /// 租约本身（scope try-exclusive）即安全删除窗口——租约持有期间任何
    /// run writer 的 try-shared 准入全部 Busy 拒，锁 unlink 在租约保护内
    /// 强制执行（不依赖墓碑时序）。
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
        let (project_id, issue_id, plan_id) = (
            scope.project_id.as_str(),
            scope.issue_id.as_str(),
            scope.entity_id.as_str(),
        );
        let run_path = self.run_path(project_id, issue_id, plan_id)?;
        if path_is_regular_file(&run_path)? {
            with_exclusive_lock(&run_path, || {
                let _ = self.read_run(&run_path, project_id, issue_id, plan_id)?;
                std::fs::remove_file(&run_path).map_err(|error| {
                    ProductStoreError::Io(format!("remove {}: {error}", run_path.display()))
                })
            })?;
        }
        let lock_path = self.lock_path(project_id, issue_id, plan_id)?;
        crate::product::coding_attempt_store::remove_file_if_exists(&lock_path)
    }

    /// H4（设计 2026-10-14 A 节，H2a 移交项）：真实 Plan 删除闭包消费——
    /// purge 生命周期元数据 `{plan_id}.json` 与同目录稳定前导点锁
    /// `.{plan_id}.json.lock`。workspace 重置清理**绝不**删除该锁（稳定
    /// inode）；仅 Plan 真删除的安全删除窗口 unlink。缺失=已清（幂等重入）；
    /// 元数据在位时先读校验身份（坏 JSON/身份错配 fail-closed，不当空强删）。
    pub fn purge_workspace_lifecycle_with_lease(
        &self,
        lease: &EntityDeletionLease,
    ) -> Result<(), ProductStoreError> {
        let scope = lease.scope();
        scope.validate()?;
        if scope.kind != DeletionEntityKind::Plan {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_plan_workspace_lifecycle_purge_scope",
                id: scope.entity_id.clone(),
            });
        }
        let (project_id, issue_id, plan_id) = (
            scope.project_id.as_str(),
            scope.issue_id.as_str(),
            scope.entity_id.as_str(),
        );
        let path = self.lifecycle_path(project_id, issue_id, plan_id)?;
        if path_is_regular_file(&path)? {
            with_exclusive_lock(&path, || {
                let _ = self.read_workspace_lifecycle(project_id, issue_id, plan_id)?;
                std::fs::remove_file(&path).map_err(|error| {
                    ProductStoreError::Io(format!("remove {}: {error}", path.display()))
                })
            })?;
        }
        // 与 run 前导点锁同约定：`.{plan_id}.json.lock`（lifecycle 文件名为
        // `{plan_id}.json`）。
        let lock_path = path.with_file_name(format!(".{plan_id}.json.lock"));
        crate::product::coding_attempt_store::remove_file_if_exists(&lock_path)
    }

    // ─── H2a（设计 2026-10-14 A/C 节）：plan 执行轮生命周期元数据 ───

    /// 生命周期元数据目录（冻结地址）：
    /// `{issue_root}/coding-plan-workspace-lifecycles/`（模块级
    /// [`CODING_PLAN_WORKSPACE_LIFECYCLES_DIR`]）。

    fn lifecycles_root(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        Ok(self
            .paths
            .issue_root(project_id, issue_id)
            .join(CODING_PLAN_WORKSPACE_LIFECYCLES_DIR))
    }

    fn lifecycle_path(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(plan_id)?;
        Ok(self
            .lifecycles_root(project_id, issue_id)?
            .join(format!("{plan_id}.json")))
    }

    /// 身份核对的元数据读取（无锁读：`write_json` 为 temp+rename 原子替换，
    /// 读见旧或新完整 JSON；坏 JSON/身份错配/IO 错误上抛，不当 None）。
    fn read_workspace_lifecycle(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<Option<CodingPlanWorkspaceLifecycle>, ProductStoreError> {
        let path = self.lifecycle_path(project_id, issue_id, plan_id)?;
        if !path_is_regular_file(&path)? {
            return Ok(None);
        }
        let lifecycle: CodingPlanWorkspaceLifecycle = read_json(&path)?;
        if lifecycle.project_id != project_id
            || lifecycle.issue_id != issue_id
            || lifecycle.plan_id != plan_id
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_plan_workspace_lifecycle",
                id: plan_id.to_string(),
            });
        }
        Ok(Some(lifecycle))
    }

    /// H2a：生命周期读取（H3/H4 GET/准入消费）。缺失＝从未重置（generation 1）。
    pub fn get_workspace_lifecycle(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<Option<CodingPlanWorkspaceLifecycle>, ProductStoreError> {
        self.read_workspace_lifecycle(project_id, issue_id, plan_id)
    }

    /// H2a：当前执行轮 generation（未创建过生命周期元数据＝1；单调不减）。
    pub fn current_workspace_generation(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<u64, ProductStoreError> {
        Ok(self
            .read_workspace_lifecycle(project_id, issue_id, plan_id)?
            .map_or(1, |lifecycle| lifecycle.generation))
    }

    /// H2a 退役围栏（设计 A 节「必需的退役/中断围栏」）：clearing 进行中或
    /// 退役 command 一律拒绝（`coding_plan_workspace_clearing` /
    /// `coding_plan_run_retired`）；IO/JSON 错误上抛 fail-closed。必须在
    /// `with_plan_live_write` 共享区内调用——生命周期写方（retire/complete）
    /// 持 Plan 独占租约，与共享区互斥，围栏读与生命周期写不可能并发。
    pub fn verify_workspace_round_admission(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        command_id: &str,
    ) -> Result<(), ProductStoreError> {
        let Some(lifecycle) = self.read_workspace_lifecycle(project_id, issue_id, plan_id)? else {
            return Ok(());
        };
        if lifecycle.clearing.is_some() {
            return Err(ProductStoreError::Conflict {
                kind: "coding_plan_workspace_clearing",
                id: plan_id.to_string(),
            });
        }
        if lifecycle
            .retired_command_ids
            .iter()
            .any(|id| id == command_id)
        {
            return Err(ProductStoreError::Conflict {
                kind: "coding_plan_run_retired",
                id: command_id.to_string(),
            });
        }
        Ok(())
    }

    /// H2a：消费已持 Plan 独占租约的清单冻结——耐久 clearing（纯 ID pending
    /// 清单）＋合并退役事实（旧 start/advance command、attempt/advance ID）。
    /// 同 command 重入刷新同一 clearing（续清）；异 command 且 clearing 在位
    /// 拒绝；clearing 缺失但同 command 回执已存在＝幂等重放，零写入。
    pub fn retire_workspace_round_with_lease(
        &self,
        lease: &EntityDeletionLease,
        checklist: &CodingPlanWorkspaceResetChecklist,
    ) -> Result<CodingPlanWorkspaceLifecycle, ProductStoreError> {
        let scope = lease.scope();
        scope.validate()?;
        if scope.kind != DeletionEntityKind::Plan {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_plan_workspace_lease_scope",
                id: scope.entity_id.clone(),
            });
        }
        let (project_id, issue_id, plan_id) = (
            scope.project_id.as_str(),
            scope.issue_id.as_str(),
            scope.entity_id.as_str(),
        );
        for id in std::iter::once(&checklist.command_id)
            .chain(checklist.retired_command_ids.iter())
            .chain(checklist.retired_attempt_ids.iter())
            .chain(checklist.retired_advance_ids.iter())
            .chain(checklist.pending_attempt_ids.iter())
            .chain(checklist.pending_advance_ids.iter())
            .chain(checklist.pending_retry_command_ids.iter())
            .chain(checklist.pending_target_repository_ids.iter())
        {
            validate_relative_id(id)?;
        }
        let path = self.lifecycle_path(project_id, issue_id, plan_id)?;
        with_exclusive_lock(&path, || {
            let now = Utc::now().to_rfc3339();
            let mut lifecycle = self
                .read_workspace_lifecycle(project_id, issue_id, plan_id)?
                .unwrap_or_else(|| CodingPlanWorkspaceLifecycle {
                    project_id: project_id.to_string(),
                    issue_id: issue_id.to_string(),
                    plan_id: plan_id.to_string(),
                    generation: 1,
                    retired_command_ids: Vec::new(),
                    retired_attempt_ids: Vec::new(),
                    retired_advance_ids: Vec::new(),
                    clearing: None,
                    receipts: Vec::new(),
                    created_at: now.clone(),
                    updated_at: now.clone(),
                });
            if let Some(clearing) = &lifecycle.clearing
                && clearing.command_id != checklist.command_id
            {
                return Err(ProductStoreError::Conflict {
                    kind: "coding_plan_workspace_clearing",
                    id: plan_id.to_string(),
                });
            }
            if lifecycle.clearing.is_none()
                && lifecycle
                    .receipts
                    .iter()
                    .any(|receipt| receipt.command_id == checklist.command_id)
            {
                // 同 command 已完整重置过的幂等重放：不再次冻结清新轮。
                return Ok(lifecycle);
            }
            let merge = |existing: &mut Vec<String>, additions: &[String]| {
                existing.extend(additions.iter().cloned());
                existing.sort();
                existing.dedup();
            };
            merge(
                &mut lifecycle.retired_command_ids,
                &checklist.retired_command_ids,
            );
            merge(
                &mut lifecycle.retired_attempt_ids,
                &checklist.retired_attempt_ids,
            );
            merge(
                &mut lifecycle.retired_advance_ids,
                &checklist.retired_advance_ids,
            );
            let sorted = |mut values: Vec<String>| {
                values.sort();
                values.dedup();
                values
            };
            lifecycle.clearing = Some(CodingPlanWorkspaceClearing {
                command_id: checklist.command_id.clone(),
                generation: lifecycle.generation,
                pending_attempt_ids: sorted(checklist.pending_attempt_ids.clone()),
                pending_advance_ids: sorted(checklist.pending_advance_ids.clone()),
                pending_retry_command_ids: sorted(checklist.pending_retry_command_ids.clone()),
                pending_target_repository_ids: sorted(
                    checklist.pending_target_repository_ids.clone(),
                ),
                started_at: lifecycle
                    .clearing
                    .as_ref()
                    .map(|clearing| clearing.started_at.clone())
                    .unwrap_or(now.clone()),
            });
            lifecycle.updated_at = now;
            write_json(&path, &lifecycle)?;
            Ok(lifecycle)
        })
    }

    /// H2a：消费已持 Plan 独占租约的轮次切换收口——复核归零由编排层（同一
    /// 受保护操作内）负责，此处完成 generation 单调 +1、清 pending、记纯 ID
    /// 回执。clearing 缺失时同 command 回执幂等重放；异 command 拒绝。
    pub fn complete_workspace_reset_with_lease(
        &self,
        lease: &EntityDeletionLease,
        command_id: &str,
    ) -> Result<CodingPlanWorkspaceLifecycle, ProductStoreError> {
        let scope = lease.scope();
        scope.validate()?;
        if scope.kind != DeletionEntityKind::Plan {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_plan_workspace_lease_scope",
                id: scope.entity_id.clone(),
            });
        }
        validate_relative_id(command_id)?;
        let (project_id, issue_id, plan_id) = (
            scope.project_id.as_str(),
            scope.issue_id.as_str(),
            scope.entity_id.as_str(),
        );
        let path = self.lifecycle_path(project_id, issue_id, plan_id)?;
        with_exclusive_lock(&path, || {
            let now = Utc::now().to_rfc3339();
            let mut lifecycle = self
                .read_workspace_lifecycle(project_id, issue_id, plan_id)?
                .ok_or_else(|| ProductStoreError::InvalidRecord {
                    kind: "coding_plan_workspace_lifecycle",
                    reason: "cannot complete a workspace reset without lifecycle metadata"
                        .to_string(),
                })?;
            match lifecycle.clearing.clone() {
                None => {
                    if lifecycle
                        .receipts
                        .iter()
                        .any(|receipt| receipt.command_id == command_id)
                    {
                        return Ok(lifecycle);
                    }
                    Err(ProductStoreError::Conflict {
                        kind: "coding_plan_workspace_clearing",
                        id: plan_id.to_string(),
                    })
                }
                Some(clearing) => {
                    if clearing.command_id != command_id {
                        return Err(ProductStoreError::Conflict {
                            kind: "coding_plan_workspace_clearing",
                            id: plan_id.to_string(),
                        });
                    }
                    let from_generation = lifecycle.generation;
                    lifecycle.generation += 1;
                    lifecycle.receipts.push(CodingPlanWorkspaceResetReceipt {
                        command_id: command_id.to_string(),
                        from_generation,
                        to_generation: lifecycle.generation,
                        completed_at: now.clone(),
                    });
                    lifecycle.clearing = None;
                    lifecycle.updated_at = now;
                    write_json(&path, &lifecycle)?;
                    Ok(lifecycle)
                }
            }
        })
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

/// H2a 共享测试夹具（web 层零成员闭包测试与本地 store 测试同源，避免第二
/// 份 fixture）：PlanRepair 种子 + 两 logical 仓 + per-unit target 草稿 +
/// Manual advance 到 durable Ready（per-target attempts Created、units
/// Pending、advance record/外层 journal/per-target journals 在场）。
#[cfg(test)]
pub(crate) mod workspace_reset_fixtures {
    use super::*;
    use crate::product::checkpoint_store::CheckpointStore;
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

    pub(crate) const PROJECT_ID: &str = "project_0001";
    pub(crate) const ISSUE_ID: &str = "issue_plan_0001";
    pub(crate) const PLAN_ID: &str = "work_item_plan_0001";

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

    pub(crate) struct N2Fixture {
        pub(crate) _root: tempfile::TempDir,
        pub(crate) paths: ProductAppPaths,
        pub(crate) api: crate::product::logical_codebase::LogicalRepositoryId,
        pub(crate) web: crate::product::logical_codebase::LogicalRepositoryId,
        pub(crate) revision_id: String,
        pub(crate) bindings: Vec<AdvanceTargetAttemptBinding>,
    }

    impl N2Fixture {
        pub(crate) fn coding_store(&self) -> CodingAttemptStore {
            CodingAttemptStore::new(self.paths.clone())
        }

        pub(crate) fn run_store(&self) -> CodingPlanRunStore {
            CodingPlanRunStore::new(self.paths.clone())
        }

        pub(crate) fn run_file(&self) -> PathBuf {
            self.paths
                .issue_root(PROJECT_ID, ISSUE_ID)
                .join("coding-plan-runs")
                .join(format!("{PLAN_ID}.json"))
        }

        pub(crate) fn api_attempt(&self) -> CodingExecutionAttempt {
            self.coding_store()
                .get_attempt_for_work_item_group(PROJECT_ID, ISSUE_ID, PLAN_ID, Some(self.api))
                .unwrap()
                .expect("api target attempt")
        }

        pub(crate) fn web_attempt(&self) -> CodingExecutionAttempt {
            self.coding_store()
                .get_attempt_for_work_item_group(PROJECT_ID, ISSUE_ID, PLAN_ID, Some(self.web))
                .unwrap()
                .expect("web target attempt")
        }

        pub(crate) fn core_unit(
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

        pub(crate) fn core_selection(&self, command_id: &str) -> CodingPlanSelection {
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

    pub(crate) async fn n2_fixture() -> N2Fixture {
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
}

#[cfg(test)]
mod tests {
    use super::workspace_reset_fixtures::{ISSUE_ID, PLAN_ID, PROJECT_ID, n2_fixture};
    use super::*;
    use crate::product::coding_models::{CodingExecutionStage, CodingUnitRun, CodingUnitRunStatus};
    use crate::product::entity_deletion_store::{DeletionEntityKind, EntityScope};
    use crate::product::entity_operation_store::EntityOperationStore;

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
        // ① 无墓碑窗口 purge：只清 run 文件不清锁（防 inode 双 writer）。
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
        // 对缺失 run 的迟到写是 NotFound（live-write 放行后 run 锁内判缺失，
        // 不当 None 吞掉）。
        let error = run_store
            .bind_ready(&invalidated, &fixture.bindings, None)
            .unwrap_err();
        assert!(
            matches!(error, ProductStoreError::NotFound { ref kind, .. } if *kind == "coding_plan_run"),
            "missing run must surface NotFound, not None: {error}"
        );
        assert!(
            lock_path.exists(),
            "purge outside the deletion window keeps the lock file"
        );

        // ② 真实删除闭包形态：Plan scope 墓碑在位后 purge 幂等清文件与锁。
        let tombstone_scope =
            EntityScope::new(PROJECT_ID, ISSUE_ID, DeletionEntityKind::Plan, PLAN_ID);
        let operations = EntityOperationStore::new(fixture.paths.clone());
        let deletions =
            crate::product::entity_deletion_store::EntityDeletionStore::new(fixture.paths.clone());
        let tombstone_lease = operations.try_acquire_delete(&tombstone_scope).unwrap();
        deletions.begin(&tombstone_lease).unwrap();
        drop(tombstone_lease);
        run_store.purge_plan(PROJECT_ID, ISSUE_ID, PLAN_ID).unwrap();
        assert!(
            !lock_path.exists(),
            "run lock is cleared inside the deletion window"
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

    /// P1（审查修复轮）：Invalidated run 不构成暂停身份——完成首项在场也
    /// 不得写 attempt（半执行：attempt Blocked 后 run 补记必败）。
    #[tokio::test]
    async fn plan_coding_n2_pause_rejects_invalidated_run() {
        let fixture = n2_fixture().await;
        let run_store = fixture.run_store();
        let coding_store = fixture.coding_store();

        let selection = fixture.core_selection("cmd_n2_pause_invalid");
        let run = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_n2_pause_invalid",
                &fixture.revision_id,
            )
            .unwrap();
        let bound = run_store
            .bind_ready(&run, &fixture.bindings, Some(selection.clone()))
            .unwrap();
        let requested = run_store.mark_start_requested(&bound).unwrap();
        run_store
            .record_start_outcome(
                &requested,
                CodingPlanStartOutcome::Started {
                    attempt_id: selection.attempt_id.clone(),
                },
            )
            .unwrap();

        coding_store
            .seed_running_attempt_for_test(PROJECT_ID, ISSUE_ID, &selection.attempt_id)
            .unwrap();
        let attempt = coding_store
            .get_attempt(PROJECT_ID, ISSUE_ID, &selection.attempt_id)
            .unwrap();
        let unit = fixture.core_unit(&attempt);
        let now = Utc::now().to_rfc3339();
        coding_store
            .create_coding_unit_run(
                &attempt,
                &CodingUnitRun {
                    id: "coding_unit_run_n2_invalid_0001".to_string(),
                    unit_id: unit.id.clone(),
                    execution_no: 1,
                    work_item_revision_id: unit.work_item_revision_id.clone(),
                    resolved_handoff_revision_ids: Vec::new(),
                    canonical_contract_hash: "n2_invalid_hash".to_string(),
                    projection_bundle_id: "projection_bundle_n2_invalid".to_string(),
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
                },
            )
            .unwrap();
        coding_store
            .complete_coding_unit_run(
                &attempt,
                "coding_unit_run_n2_invalid_0001",
                "n2_invalid_commit",
            )
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
                Some("handoff_revision_n2_invalid".to_string()),
            )
            .unwrap();
        coding_store
            .update_coding_unit_completion_commit(
                PROJECT_ID,
                ISSUE_ID,
                &attempt.id,
                &unit.id,
                Some("n2_invalid_commit".to_string()),
            )
            .unwrap();
        let running = coding_store
            .get_attempt(PROJECT_ID, ISSUE_ID, &attempt.id)
            .unwrap();

        // 成员删除使 run Invalidated：pause 不匹配（None），不得触碰 attempt。
        run_store
            .invalidate_for_deleted_attempt(&fixture.web_attempt())
            .unwrap();
        let outcome = coding_store
            .pause_completed_plan_first_unit(&running, &unit.id)
            .unwrap();
        assert!(
            outcome.is_none(),
            "invalidated run must not be treated as a pause identity"
        );

        let untouched = coding_store
            .get_attempt(PROJECT_ID, ISSUE_ID, &running.id)
            .unwrap();
        assert_eq!(untouched.status, CodingAttemptStatus::Running);
        assert!(
            coding_store
                .list_open_blocked_gates(PROJECT_ID, ISSUE_ID, &running.id)
                .unwrap()
                .is_empty(),
            "no pause gate may be created for an invalidated run"
        );
        assert_eq!(
            run_store
                .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
                .unwrap()
                .unwrap()
                .phase,
            CodingPlanRunPhase::Invalidated,
            "run stays invalidated, never paused"
        );
    }

    /// P2（审查修复轮）：Plan scope live-write 准入——墓碑在位时全部 run
    /// 写点以 entity_deleting 拒（不重建可重放孤儿）；读路径不受阻；删除
    /// 窗口内 purge 清文件与锁。
    #[tokio::test]
    async fn plan_coding_n2_writers_reject_entity_deleting_tombstone() {
        let fixture = n2_fixture().await;
        let run_store = fixture.run_store();

        let selection = fixture.core_selection("cmd_n2_tombstone");
        let run = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_n2_tombstone",
                &fixture.revision_id,
            )
            .unwrap();
        let bound = run_store
            .bind_ready(&run, &fixture.bindings, Some(selection.clone()))
            .unwrap();
        let requested = run_store.mark_start_requested(&bound).unwrap();

        // 放置 purging 墓碑（真实删除闭包形态：lease→begin→释放）。
        let scope = EntityScope::new(PROJECT_ID, ISSUE_ID, DeletionEntityKind::Plan, PLAN_ID);
        let operations = EntityOperationStore::new(fixture.paths.clone());
        let deletions =
            crate::product::entity_deletion_store::EntityDeletionStore::new(fixture.paths.clone());
        let lease = operations.try_acquire_delete(&scope).unwrap();
        deletions.begin(&lease).unwrap();
        drop(lease);

        // 迟到写点全部被 entity_deleting 拒。
        for name in [
            "prepare",
            "bind_ready",
            "mark_start_requested",
            "mark_paused",
        ] {
            let result = match name {
                "prepare" => run_store.prepare(
                    PROJECT_ID,
                    ISSUE_ID,
                    PLAN_ID,
                    "cmd_n2_tombstone_late",
                    &fixture.revision_id,
                ),
                "bind_ready" => {
                    run_store.bind_ready(&bound, &fixture.bindings, Some(selection.clone()))
                }
                "mark_start_requested" => run_store.mark_start_requested(&requested),
                _ => run_store.mark_paused(&requested),
            };
            let error = result.unwrap_err();
            assert!(
                matches!(error, ProductStoreError::Conflict { ref kind, .. } if *kind == "entity_deleting"),
                "{name}: unexpected error: {error}"
            );
        }
        let error = run_store
            .record_start_outcome(
                &requested,
                CodingPlanStartOutcome::Started {
                    attempt_id: selection.attempt_id.clone(),
                },
            )
            .unwrap_err();
        assert!(
            matches!(error, ProductStoreError::Conflict { ref kind, .. } if *kind == "entity_deleting"),
            "record_start_outcome: unexpected error: {error}"
        );

        // 读路径不受阻：run 事实仍可观察。
        assert_eq!(
            run_store
                .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
                .unwrap()
                .unwrap()
                .phase,
            CodingPlanRunPhase::StartRequested
        );

        // 删除窗口（墓碑在位）purge：清 run 文件与锁，不留可重放孤儿。
        run_store.purge_plan(PROJECT_ID, ISSUE_ID, PLAN_ID).unwrap();
        assert!(
            run_store
                .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
                .unwrap()
                .is_none()
        );
        let lock_path = fixture
            .paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("coding-plan-runs")
            .join(format!(".{PLAN_ID}.json.lock"));
        assert!(
            !lock_path.exists(),
            "lock is cleared inside the deletion window"
        );
        // 墓碑仍在：purge 后迟到 prepare 依旧拒，不重建孤儿。
        let error = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_n2_tombstone_replay",
                &fixture.revision_id,
            )
            .unwrap_err();
        assert!(
            matches!(error, ProductStoreError::Conflict { ref kind, .. } if *kind == "entity_deleting"),
            "no orphan run may be rebuilt after purge: {error}"
        );
    }

    // ─── H2a（设计 2026-10-14 A 节）：退役围栏/生命周期元数据/持租约失效 ───

    fn h2a_checklist(
        command_id: &str,
        run: Option<&CodingPlanRun>,
    ) -> CodingPlanWorkspaceResetChecklist {
        let mut retired_command_ids = vec![command_id.to_string()];
        let mut retired_attempt_ids = Vec::new();
        if let Some(run) = run {
            retired_command_ids.push(run.command_id.clone());
            for binding in &run.target_attempts {
                retired_attempt_ids.push(binding.attempt_id.clone());
            }
        }
        CodingPlanWorkspaceResetChecklist {
            command_id: command_id.to_string(),
            retired_command_ids,
            retired_attempt_ids: retired_attempt_ids.clone(),
            retired_advance_ids: Vec::new(),
            pending_attempt_ids: retired_attempt_ids,
            pending_advance_ids: Vec::new(),
            pending_retry_command_ids: Vec::new(),
            pending_target_repository_ids: Vec::new(),
        }
    }

    /// H2a T1：全删关闭旧执行轮后，退役 command 不能因 run 文件缺失重新
    /// 准备（清 run 绝不是完整重置）；旧 run 快照迟到写被拒；新 command
    /// 正常开新轮；generation 单调 +1 且回执/退役事实在位。
    #[tokio::test]
    async fn plan_workspace_h2a_retired_command_cannot_reprepare() {
        let fixture = n2_fixture().await;
        let run_store = fixture.run_store();
        let run = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_h2a_round1",
                &fixture.revision_id,
            )
            .unwrap();
        let bound = run_store.bind_ready(&run, &fixture.bindings, None).unwrap();

        let scope = EntityScope::new(PROJECT_ID, ISSUE_ID, DeletionEntityKind::Plan, PLAN_ID);
        let lease = EntityOperationStore::new(fixture.paths.clone())
            .try_acquire_delete(&scope)
            .unwrap();
        run_store
            .retire_workspace_round_with_lease(
                &lease,
                &h2a_checklist("cmd_h2a_reset_0001", Some(&bound)),
            )
            .unwrap();
        run_store.purge_plan_with_lease(&lease).unwrap();
        let completed = run_store
            .complete_workspace_reset_with_lease(&lease, "cmd_h2a_reset_0001")
            .unwrap();
        drop(lease);
        assert_eq!(completed.generation, 2);
        assert!(completed.clearing.is_none());
        assert_eq!(completed.receipts.len(), 1);
        assert_eq!(completed.receipts[0].from_generation, 1);
        assert_eq!(completed.receipts[0].to_generation, 2);
        assert!(
            completed
                .retired_command_ids
                .contains(&"cmd_h2a_round1".to_string())
        );

        // 旧 command 复跑必须被退役围栏拒绝（run 文件已清也不得重建孤儿）。
        let error = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_h2a_round1",
                &fixture.revision_id,
            )
            .unwrap_err();
        assert!(
            matches!(error, ProductStoreError::Conflict { ref kind, .. } if *kind == "coding_plan_run_retired"),
            "retired command must not re-prepare: {error}"
        );
        // 旧 run 快照的迟到 run 更新同样被拒。
        let error = run_store.mark_start_requested(&bound).unwrap_err();
        assert!(
            matches!(error, ProductStoreError::Conflict { ref kind, .. } if *kind == "coding_plan_run_retired"),
            "late run writes must hit the retirement fence: {error}"
        );
        // 新 command 在新 generation 上正常开轮。
        let fresh = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_h2a_round2",
                &fixture.revision_id,
            )
            .unwrap();
        assert_eq!(fresh.command_id, "cmd_h2a_round2");
        assert!(matches!(fresh.phase, CodingPlanRunPhase::Prepared));
    }

    /// H2a T2：clearing 耐久阻断本轮首启/run 更新；异 command retire/
    /// complete 拒绝穿越；同 command 续清与幂等重放零副作用。
    #[tokio::test]
    async fn plan_workspace_h2a_clearing_fences_and_completes() {
        let fixture = n2_fixture().await;
        let run_store = fixture.run_store();
        let run = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_h2a_c_round1",
                &fixture.revision_id,
            )
            .unwrap();

        let scope = EntityScope::new(PROJECT_ID, ISSUE_ID, DeletionEntityKind::Plan, PLAN_ID);
        let lease = EntityOperationStore::new(fixture.paths.clone())
            .try_acquire_delete(&scope)
            .unwrap();
        run_store
            .retire_workspace_round_with_lease(
                &lease,
                &h2a_checklist("cmd_h2a_c_reset", Some(&run)),
            )
            .unwrap();
        drop(lease); // 模拟清理中断：clearing 耐久、租约已释放。

        // clearing 阻断新首启与 run 更新。
        for error in [
            run_store
                .prepare(
                    PROJECT_ID,
                    ISSUE_ID,
                    PLAN_ID,
                    "cmd_h2a_c_new",
                    &fixture.revision_id,
                )
                .unwrap_err(),
            run_store
                .bind_ready(&run, &fixture.bindings, None)
                .unwrap_err(),
            run_store.mark_paused(&run).unwrap_err(),
        ] {
            assert!(
                matches!(error, ProductStoreError::Conflict { ref kind, .. } if *kind == "coding_plan_workspace_clearing"),
                "clearing must fence run writes: {error}"
            );
        }
        // GET 可投影 clearing 事实。
        let lifecycle = run_store
            .get_workspace_lifecycle(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap()
            .expect("durable clearing");
        assert_eq!(
            lifecycle.clearing.as_ref().unwrap().command_id,
            "cmd_h2a_c_reset"
        );

        let lease = EntityOperationStore::new(fixture.paths.clone())
            .try_acquire_delete(&scope)
            .unwrap();
        // 异 command retire/complete 拒绝穿越。
        assert!(matches!(
            run_store
                .retire_workspace_round_with_lease(&lease, &h2a_checklist("cmd_h2a_c_other", None))
                .unwrap_err(),
            ProductStoreError::Conflict { ref kind, .. }
                if *kind == "coding_plan_workspace_clearing"
        ));
        assert!(matches!(
            run_store
                .complete_workspace_reset_with_lease(&lease, "cmd_h2a_c_other")
                .unwrap_err(),
            ProductStoreError::Conflict { ref kind, .. }
                if *kind == "coding_plan_workspace_clearing"
        ));
        // 同 command 重入 retire 刷新同一 clearing（续清），complete 完成轮次切换。
        run_store
            .retire_workspace_round_with_lease(
                &lease,
                &h2a_checklist("cmd_h2a_c_reset", Some(&run)),
            )
            .unwrap();
        let completed = run_store
            .complete_workspace_reset_with_lease(&lease, "cmd_h2a_c_reset")
            .unwrap();
        assert_eq!(completed.generation, 2);
        assert!(completed.clearing.is_none());
        // complete 幂等重放（clearing 已清、同 command 回执在）。
        let replay = run_store
            .complete_workspace_reset_with_lease(&lease, "cmd_h2a_c_reset")
            .unwrap();
        assert_eq!(replay, completed);
        // retire 幂等重放：同 command 回执在，不再冻结新 clearing。
        let after = run_store
            .retire_workspace_round_with_lease(&lease, &h2a_checklist("cmd_h2a_c_reset", None))
            .unwrap();
        assert!(after.clearing.is_none());
        assert_eq!(after.receipts.len(), 1);
    }

    /// H2a T3：持 Plan 独占租约时公开成员失效路径自拒 `entity_deleting`；
    /// 消费已持租约的内部入口与原闭包共用实现，正常失效且冻结集保留；
    /// 租约身份错配拒绝。
    #[tokio::test]
    async fn plan_workspace_h2a_invalidate_consumes_held_lease() {
        let fixture = n2_fixture().await;
        let run_store = fixture.run_store();
        let run = run_store
            .prepare(
                PROJECT_ID,
                ISSUE_ID,
                PLAN_ID,
                "cmd_h2a_l_round1",
                &fixture.revision_id,
            )
            .unwrap();
        let bound = run_store.bind_ready(&run, &fixture.bindings, None).unwrap();
        let attempt = fixture.api_attempt();

        let scope = EntityScope::new(PROJECT_ID, ISSUE_ID, DeletionEntityKind::Plan, PLAN_ID);
        let lease = EntityOperationStore::new(fixture.paths.clone())
            .try_acquire_delete(&scope)
            .unwrap();
        // 公开路径在独占租约下自拒（try-shared → entity_deleting）。
        let error = run_store
            .invalidate_for_deleted_attempt(&attempt)
            .unwrap_err();
        assert!(
            matches!(error, ProductStoreError::Conflict { ref kind, .. } if *kind == "entity_deleting"),
            "public path must self-reject under the exclusive lease: {error}"
        );
        // 持租约内部入口不二次取 scope：成员失效成功且幂等。
        run_store
            .invalidate_for_deleted_attempt_with_lease(&lease, &attempt)
            .unwrap();
        run_store
            .invalidate_for_deleted_attempt_with_lease(&lease, &attempt)
            .unwrap();
        let stored = run_store
            .get(PROJECT_ID, ISSUE_ID, PLAN_ID)
            .unwrap()
            .expect("run still on disk");
        assert!(matches!(stored.phase, CodingPlanRunPhase::Invalidated));
        assert_eq!(stored.target_attempts, bound.target_attempts);
        // run 维度重置失效入口同核（已 Invalidated 幂等）。
        run_store
            .invalidate_run_for_reset_with_lease(&lease, &bound)
            .unwrap();
        // 租约身份错配拒绝（异 plan scope）。
        let other_scope = EntityScope::new(
            PROJECT_ID,
            ISSUE_ID,
            DeletionEntityKind::Plan,
            "work_item_plan_9999",
        );
        let other_lease = EntityOperationStore::new(fixture.paths.clone())
            .try_acquire_delete(&other_scope)
            .unwrap();
        assert!(matches!(
            run_store
                .invalidate_run_for_reset_with_lease(&other_lease, &bound)
                .unwrap_err(),
            ProductStoreError::IdentityMismatch { ref kind, .. }
                if *kind == "coding_plan_run_lease_scope"
        ));
    }
}
