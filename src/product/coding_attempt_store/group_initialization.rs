use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::product::coding_models::{
    CodingAdmissionKind, CodingAttemptPlanBinding, CodingAttemptScope, CodingAttemptStatus,
    CodingExecutionAttempt, CodingExecutionStage, CodingExecutionUnit, CodingExecutionUnitStatus,
    CodingRoleProviderConfigSnapshot,
};
use crate::product::entity_deletion_store::DeletionEntityKind;
use crate::product::entity_operation_store::EntityDeletionLease;
use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::logical_codebase::RepositoryRouting;

use super::group_validation::{
    AuthoritativeCodingUnitBinding, mixed_target_group_rejected, validate_group_single_target,
};
use super::locking::ExclusiveFileLock;
use super::{
    CreateGroupCodingAttemptInput, WorkItemAttemptCreationGuard, incomplete_group_attempt,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingGroupInitializationPhase {
    Prepared,
    AttemptPersisted,
    WorktreeBound,
    PlanBindingSaved,
    UnitsMaterialized,
    Completed,
}

impl CodingGroupInitializationPhase {
    fn order(self) -> u8 {
        match self {
            Self::Prepared => 0,
            Self::AttemptPersisted => 1,
            Self::WorktreeBound => 2,
            Self::PlanBindingSaved => 3,
            Self::UnitsMaterialized => 4,
            Self::Completed => 5,
        }
    }

    pub fn has_reached(self, phase: Self) -> bool {
        self.order() >= phase.order()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingGroupInitializationJournal {
    pub id: String,
    pub project_id: String,
    pub issue_id: String,
    pub plan_id: String,
    pub lock_work_item_id: String,
    pub worktree_lease_id: String,
    pub attempt: CodingExecutionAttempt,
    pub provider_config: CodingRoleProviderConfigSnapshot,
    pub plan_binding: CodingAttemptPlanBinding,
    pub units: Vec<CodingExecutionUnit>,
    pub phase: CodingGroupInitializationPhase,
    pub error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

pub struct CodingGroupInitializationGuard {
    _lock: ExclusiveFileLock,
}

/// H2b（设计 A 节「journal 在而 attempts 亡」）：per-target/legacy group
/// 初始化 journal 的引用分类——前缀（attempt 未物化属预期缺失，续原
/// 协议）/ 绑定存活 / 已越 AttemptPersisted 而 attempt 缺失（执行引用
/// 断裂，零成员闭包清理对象；不得用旧 snapshot 复活旧 ID）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodingGroupJournalReference {
    InitializationPrefix {
        journal_id: String,
        attempt_id: String,
    },
    BoundToAliveAttempt {
        journal_id: String,
        attempt_id: String,
    },
    BoundToDeadAttempt {
        journal_id: String,
        attempt_id: String,
    },
}

impl CodingGroupJournalReference {
    /// journal 稳定 ID（确定性排序辅助）。
    pub fn journal_id(&self) -> &str {
        match self {
            Self::InitializationPrefix { journal_id, .. }
            | Self::BoundToAliveAttempt { journal_id, .. }
            | Self::BoundToDeadAttempt { journal_id, .. } => journal_id,
        }
    }
}

// 物理拆分（large_file_guard 1200 行硬线）：impl 方法体按台账生命周期/日志清理
// 分文件，include! 后与本文件同模块域。
include!("group_initialization_parts/lifecycle_impl.inc.rs");

include!("group_initialization_parts/journal_purge_impl.inc.rs");

fn validate_group_initialization_input(
    input: &CreateGroupCodingAttemptInput,
    bound_plan_revision_id: &str,
    unit_bindings: &[AuthoritativeCodingUnitBinding],
    routing: &RepositoryRouting,
) -> Result<(), ProductStoreError> {
    for id in [
        input.project_id.as_str(),
        input.issue_id.as_str(),
        input.plan_id.as_str(),
        input.current_work_item_id.as_str(),
        bound_plan_revision_id,
    ] {
        validate_relative_id(id)?;
    }
    super::validate_max_auto_rework(input.max_auto_rework)?;
    if unit_bindings.is_empty()
        || unit_bindings[0].logical_work_item_id != input.current_work_item_id
    {
        return Err(ProductStoreError::IdentityMismatch {
            kind: "coding_group_initialization",
            id: input.plan_id.clone(),
        });
    }
    validate_group_single_target(routing, unit_bindings, input.target_snapshot.as_ref())
        .map_err(|_| mixed_target_group_rejected())?;
    for binding in unit_bindings {
        validate_relative_id(&binding.logical_work_item_id)?;
        validate_relative_id(&binding.work_item_revision_id)?;
        validate_relative_id(&binding.verification_plan_revision_id)?;
        validate_relative_id(&binding.projection_bundle_id)?;
        for dependency_id in &binding.dependency_logical_work_item_ids {
            validate_relative_id(dependency_id)?;
        }
    }
    Ok(())
}

fn validate_group_initialization_journal(
    journal: &CodingGroupInitializationJournal,
) -> Result<(), ProductStoreError> {
    for id in [
        journal.id.as_str(),
        journal.project_id.as_str(),
        journal.issue_id.as_str(),
        journal.plan_id.as_str(),
        journal.lock_work_item_id.as_str(),
        journal.worktree_lease_id.as_str(),
        journal.attempt.id.as_str(),
        journal.plan_binding.bound_plan_revision_id.as_str(),
    ] {
        validate_relative_id(id)?;
    }
    let first = journal.units.first().ok_or_else(|| {
        incomplete_group_attempt(&journal.attempt.id, "initialization journal has no units")
    })?;
    let identity_is_valid = journal.attempt.project_id == journal.project_id
        && journal.attempt.issue_id == journal.issue_id
        && journal.attempt.scope == CodingAttemptScope::WorkItemGroup
        && journal.attempt.work_item_group_id.as_deref() == Some(journal.plan_id.as_str())
        && journal.attempt.work_item_id == journal.lock_work_item_id
        && journal.attempt.current_work_item_id.as_deref()
            == Some(journal.lock_work_item_id.as_str())
        && journal.attempt.active_unit_id.as_deref() == Some(first.id.as_str())
        && journal.provider_config
            == CodingRoleProviderConfigSnapshot::from(&journal.attempt.provider_config_snapshot)
        && journal.plan_binding.attempt_id == journal.attempt.id
        && journal.plan_binding.plan_id == journal.plan_id
        && journal.units.iter().enumerate().all(|(index, unit)| {
            unit.id == next_sequential_id("coding_unit", index)
                && unit.attempt_id == journal.attempt.id
                && unit.project_id == journal.project_id
                && unit.issue_id == journal.issue_id
                && unit.plan_id == journal.plan_id
                && unit.order_index == index as u32
                && unit.status
                    == if index == 0 {
                        CodingExecutionUnitStatus::Running
                    } else {
                        CodingExecutionUnitStatus::Pending
                    }
        });
    if !identity_is_valid {
        return Err(incomplete_group_attempt(
            &journal.attempt.id,
            "initialization journal identity is invalid",
        ));
    }
    Ok(())
}

fn journal_matches_request(
    journal: &CodingGroupInitializationJournal,
    input: &CreateGroupCodingAttemptInput,
    bound_plan_revision_id: &str,
    unit_bindings: &[AuthoritativeCodingUnitBinding],
    admission_kind: CodingAdmissionKind,
) -> bool {
    journal.project_id == input.project_id
        && journal.issue_id == input.issue_id
        && journal.plan_id == input.plan_id
        && journal.lock_work_item_id == input.current_work_item_id
        && journal.attempt.base_branch == input.base_branch
        && journal.attempt.branch_name == input.branch_name
        && journal.attempt.worktree_path == input.worktree_path
        && journal.attempt.provider_config_snapshot == input.provider_config_snapshot
        && journal.attempt.target_snapshot == input.target_snapshot
        && journal.attempt.max_auto_rework == input.max_auto_rework
        && journal.attempt.admission_kind == admission_kind
        && journal.plan_binding.bound_plan_revision_id == bound_plan_revision_id
        && journal.units.len() == unit_bindings.len()
        && journal
            .units
            .iter()
            .zip(unit_bindings.iter())
            .all(|(unit, binding)| {
                unit.logical_work_item_id == binding.logical_work_item_id
                    && unit.work_item_revision_id == binding.work_item_revision_id
                    && unit.dependency_logical_work_item_ids
                        == binding.dependency_logical_work_item_ids
            })
}

/// 按依赖边拓扑排序 unit bindings。
///
/// N1 依赖同源（fix-multi-repo-group-coding-start REQ-CS-03/08）：依赖端点不在
/// 输入集内的边是**合法外部边**（跨 target 桶依赖）——保留在返回 binding 的
/// `dependency_logical_work_item_ids`（plan gate 消费），但不参与入度与排序。
/// 重复 logical WI、声明列表内重复依赖、self 边、输入集内部的环仍失败关闭；
/// 「真 unknown 依赖」（连权威 plan 全集都不存在）由
/// [`topologically_order_target_unit_bindings`] 的全图校验与 plan revision
/// 绑定解析（`resolve_authoritative_group_plan_binding_for_revision` 的边端点
/// 校验）失败关闭，本函数对子集输入不做 unknown 判定。
pub(crate) fn topologically_order_unit_bindings(
    bindings: &[AuthoritativeCodingUnitBinding],
) -> Result<Vec<AuthoritativeCodingUnitBinding>, ProductStoreError> {
    use std::collections::{BTreeMap, BTreeSet};

    let known = bindings
        .iter()
        .map(|binding| binding.logical_work_item_id.clone())
        .collect::<BTreeSet<_>>();
    if known.len() != bindings.len() {
        return Err(ProductStoreError::IdentityMismatch {
            kind: "coding_group_dependency_graph",
            id: "duplicate_work_item".to_string(),
        });
    }
    let mut indegree = BTreeMap::new();
    let mut dependents = BTreeMap::<String, Vec<String>>::new();
    for binding in bindings {
        let mut dependencies = BTreeSet::new();
        for dependency in &binding.dependency_logical_work_item_ids {
            if dependency == &binding.logical_work_item_id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_group_dependency_graph",
                    id: binding.logical_work_item_id.clone(),
                });
            }
            if !dependencies.insert(dependency.clone()) {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_group_dependency_graph",
                    id: dependency.clone(),
                });
            }
            if known.contains(dependency) {
                dependents
                    .entry(dependency.clone())
                    .or_default()
                    .push(binding.logical_work_item_id.clone());
            }
        }
        indegree.insert(
            binding.logical_work_item_id.clone(),
            dependencies.intersection(&known).count(),
        );
    }
    let mut ready = indegree
        .iter()
        .filter(|(_, degree)| **degree == 0)
        .map(|(id, _)| id.clone())
        .collect::<Vec<_>>();
    let sort_ready = |ready: &mut Vec<String>| {
        ready.sort();
    };
    sort_ready(&mut ready);
    let mut ordered_ids = Vec::with_capacity(bindings.len());
    while let Some(id) = ready.first().cloned() {
        ready.remove(0);
        ordered_ids.push(id.clone());
        for dependent in dependents.get(&id).into_iter().flatten() {
            let degree = indegree
                .get_mut(dependent)
                .expect("dependency endpoint was registered");
            *degree -= 1;
            if *degree == 0 {
                ready.push(dependent.clone());
            }
        }
        sort_ready(&mut ready);
    }
    if ordered_ids.len() != bindings.len() {
        return Err(ProductStoreError::IdentityMismatch {
            kind: "coding_group_dependency_graph",
            id: "cycle".to_string(),
        });
    }
    let mut by_id = bindings
        .iter()
        .map(|binding| (binding.logical_work_item_id.clone(), binding.clone()))
        .collect::<BTreeMap<_, _>>();
    Ok(ordered_ids
        .into_iter()
        .map(|id| by_id.remove(&id).expect("ordered binding was registered"))
        .collect())
}

/// N1 依赖同源（REQ-CS-03/08）：按**已验证全局图**的 target 桶排序。
///
/// 先对 `all`（权威 plan 全 units）做全图校验——真 unknown 依赖（端点不在全集
/// 内）、重复 WI、self 边、全局环（含跨桶环，不按仓缩图）失败关闭；再仅以
/// 桶内边排序 `target` 桶（复用 [`topologically_order_unit_bindings`]，同一
/// 算法与并列打破规则），返回的 binding 保留全部原
/// `dependency_logical_work_item_ids`（外部 depends 不删，plan gate 消费）。
/// 空 target 桶失败关闭。完整性/integrity 恢复面消费同一排序语义。
pub(crate) fn topologically_order_target_unit_bindings(
    all: &[AuthoritativeCodingUnitBinding],
    target: crate::product::logical_codebase::LogicalRepositoryId,
) -> Result<Vec<AuthoritativeCodingUnitBinding>, ProductStoreError> {
    let known = all
        .iter()
        .map(|binding| binding.logical_work_item_id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    for binding in all {
        for dependency in &binding.dependency_logical_work_item_ids {
            if !known.contains(dependency) {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_group_dependency_graph",
                    id: dependency.clone(),
                });
            }
        }
    }
    // 全图结构校验（重复/self/全局环；unknown 已核，全部边在集内）。
    topologically_order_unit_bindings(all)?;
    let bucket = all
        .iter()
        .filter(|binding| binding.target_repository_id == Some(target))
        .cloned()
        .collect::<Vec<_>>();
    if bucket.is_empty() {
        return Err(ProductStoreError::IdentityMismatch {
            kind: "coding_group_dependency_graph",
            id: format!("empty_target_bucket_{}", target.0),
        });
    }
    // 桶内排序：同一 helper，外部边保留不计入。
    topologically_order_unit_bindings(&bucket)
}
fn same_group_initialization_identity(
    left: &CodingGroupInitializationJournal,
    right: &CodingGroupInitializationJournal,
) -> bool {
    left.id == right.id
        && left.project_id == right.project_id
        && left.issue_id == right.issue_id
        && left.plan_id == right.plan_id
        && left.lock_work_item_id == right.lock_work_item_id
        && left.worktree_lease_id == right.worktree_lease_id
        && left.attempt == right.attempt
        && left.provider_config == right.provider_config
        && left.plan_binding == right.plan_binding
        && left.units == right.units
        && left.created_at == right.created_at
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::coding_attempt_store::CodingAttemptStore;
    use crate::product::entity_deletion_store::{DeletionEntityKind, EntityScope};
    use crate::product::entity_operation_store::EntityOperationStore;
    use crate::product::logical_codebase::{LogicalRepositoryId, RepositoryCheckoutId};
    use crate::product::models::ProviderName;
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;
    use tempfile::TempDir;

    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_0001";

    fn plain_store() -> (TempDir, ProductAppPaths, CodingAttemptStore) {
        let tmp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        let store = CodingAttemptStore::new(paths.clone());
        (tmp, paths, store)
    }

    fn provider_snapshot() -> ProviderConfigSnapshot {
        ProviderConfigSnapshot {
            author: ProviderName::Codex,
            reviewer: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
            permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        }
    }

    fn legacy_group_input(plan_id: &str) -> CreateGroupCodingAttemptInput {
        CreateGroupCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            plan_id: plan_id.to_string(),
            current_work_item_id: "work_item_0001".to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            target_snapshot: None,
            max_auto_rework: 2,
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
        }
    }

    fn legacy_unit(logical_id: &str) -> AuthoritativeCodingUnitBinding {
        AuthoritativeCodingUnitBinding {
            logical_work_item_id: logical_id.to_string(),
            work_item_revision_id: format!("revision_{logical_id}"),
            verification_plan_revision_id: format!("verification_{logical_id}"),
            projection_bundle_id: format!("projection_{logical_id}"),
            target_repository_id: None,
            source_draft_error: None,
            dependency_logical_work_item_ids: Vec::new(),
        }
    }

    /// Logical 路由 fixture：project + manifest（targets）+ issue + explicit
    /// selection（同 group_split_target 惯例），使 per-target journal 可经
    /// `prepare_group_initialization_with_admission_for_target` 准备。
    fn routing_store(
        targets: Vec<LogicalRepositoryId>,
    ) -> (TempDir, ProductAppPaths, CodingAttemptStore, String, String) {
        let tmp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        let project = crate::product::project_store::ProjectStore::new(paths.clone())
            .create(crate::product::project_store::CreateProjectInput {
                name: "h2b group initialization".to_string(),
                description: None,
            })
            .unwrap();
        crate::product::logical_codebase::LogicalCodebaseStore::new(paths.clone())
            .save_manifest(
                &project.id,
                &crate::product::logical_codebase::LogicalCodebaseManifest::new(
                    &project.id,
                    tmp.path().join("logical-root"),
                    targets.clone(),
                ),
            )
            .unwrap();
        let issue = crate::product::issue_store::IssueStore::new(paths.clone())
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: project.id.clone(),
                repo_id: None,
                logical_codebase_id: crate::product::logical_codebase::legacy_logical_codebase_id(
                    &project.id,
                ),
                title: "h2b group initialization issue".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .unwrap();
        crate::product::logical_codebase::IssueCodebaseSelectionStore::new(paths.clone())
            .save(
                &crate::product::logical_codebase::IssueCodebaseSelection::explicit(
                    &project.id,
                    &issue.id,
                    targets,
                    Vec::new(),
                    Vec::new(),
                    None,
                ),
            )
            .unwrap();
        let store = CodingAttemptStore::new(paths.clone());
        (tmp, paths, store, project.id, issue.id)
    }

    fn target_snapshot(
        logical_id: LogicalRepositoryId,
    ) -> crate::product::coding_models::AttemptTargetSnapshot {
        crate::product::coding_models::AttemptTargetSnapshot {
            logical_repository_id: logical_id,
            checkout_id: RepositoryCheckoutId(uuid::Uuid::nil()),
            physical_repository_id: "repository_h2b".to_string(),
            canonical_path: std::path::PathBuf::from("/h2b/repository"),
            git_dir_identity: "h2b-git-dir".to_string(),
            revision: Some("h2b-revision".to_string()),
            policy_digest: "h2b-policy".to_string(),
            membership_revision: 1,
            captured_at: "2026-10-14T00:00:00Z".to_string(),
            capture_source: "test".to_string(),
        }
    }

    fn split_unit(target: LogicalRepositoryId, logical_id: &str) -> AuthoritativeCodingUnitBinding {
        AuthoritativeCodingUnitBinding {
            target_repository_id: Some(target),
            ..legacy_unit(logical_id)
        }
    }

    fn prepare_for_target(
        store: &CodingAttemptStore,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        work_item_id: &str,
        target: LogicalRepositoryId,
    ) -> CodingGroupInitializationJournal {
        let input = CreateGroupCodingAttemptInput {
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            plan_id: plan_id.to_string(),
            current_work_item_id: work_item_id.to_string(),
            base_branch: "main".to_string(),
            branch_name: format!("aria/issues/{issue_id}/{}", target.0),
            worktree_path: None,
            provider_config_snapshot: provider_snapshot(),
            target_snapshot: Some(target_snapshot(target)),
            max_auto_rework: 2,
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
        };
        store
            .prepare_group_initialization_with_admission_for_target(
                &input,
                "plan_revision_h2b",
                &[split_unit(target, work_item_id)],
                CodingAdmissionKind::ScAdvance,
                Some(target),
            )
            .expect("prepare per-target journal")
    }

    #[test]
    fn h2b_beyond_attempt_persisted_journal_must_not_revive_dead_attempt_id() {
        // 设计 A 节「journal 在而 attempts 亡」：journal 已达
        // AttemptPersisted 而绑定 attempt 已删（整组删除残留）——ensure 不得
        // 用旧 snapshot 重建该 attempt（复活旧 ID），fail-closed。
        let (_tmp, _paths, store) = plain_store();
        let journal = store
            .prepare_group_initialization(
                &legacy_group_input("work_item_plan_0001"),
                "plan_revision_0001",
                &[legacy_unit("work_item_0001")],
            )
            .expect("prepare legacy journal");
        let guard = store
            .acquire_work_item_attempt_creation(PROJECT_ID, ISSUE_ID, &journal.lock_work_item_id)
            .unwrap();
        let attempt = store
            .ensure_group_initialization_attempt(&journal, &guard)
            .expect("prefix first materialization");
        let persisted = store
            .advance_group_initialization_phase(
                &journal,
                CodingGroupInitializationPhase::AttemptPersisted,
            )
            .expect("checkpoint attempt persistence");

        // 模拟整组删除只删了 attempt 而 journal 残留。
        let attempt_path = store.attempt_path(PROJECT_ID, ISSUE_ID, &attempt.id);
        crate::product::coding_attempt_store::remove_file_if_exists(&attempt_path).unwrap();

        let error = store
            .ensure_group_initialization_attempt(&persisted, &guard)
            .expect_err("must not rebuild the dead attempt from the stale snapshot");
        assert!(
            error.to_string().contains("must not be revived"),
            "unexpected error: {error}"
        );
        assert!(
            !attempt_path.is_file(),
            "the dead attempt id must stay dead"
        );
    }

    #[test]
    fn h2b_prefix_journal_continues_first_materialization_protocol() {
        // 设计 A 节：journal 未达 AttemptPersisted、attempt 缺失属既有初始化
        // 中断前缀——ensure 以原 snapshot 首物化续做（原协议零变化），不得
        // 误判为删除。
        let (_tmp, _paths, store) = plain_store();
        let journal = store
            .prepare_group_initialization(
                &legacy_group_input("work_item_plan_0001"),
                "plan_revision_0001",
                &[legacy_unit("work_item_0001")],
            )
            .expect("prepare legacy journal");
        let attempt_path = store.attempt_path(PROJECT_ID, ISSUE_ID, &journal.attempt.id);
        assert!(!attempt_path.is_file());

        let guard = store
            .acquire_work_item_attempt_creation(PROJECT_ID, ISSUE_ID, &journal.lock_work_item_id)
            .unwrap();
        let attempt = store
            .ensure_group_initialization_attempt(&journal, &guard)
            .expect("prefix journal first-materializes the attempt");
        assert_eq!(attempt.id, journal.attempt.id);
        assert!(attempt_path.is_file());
    }

    #[test]
    fn h2b_classify_group_initialization_references_three_forms() {
        // 三形态：前缀（Prepared，attempt 未物化属预期）/ 绑定存活 /
        // 已越 AttemptPersisted 而 attempt 缺失（断裂，交零成员闭包）。
        let targets = vec![
            LogicalRepositoryId(uuid::Uuid::nil()),
            LogicalRepositoryId(uuid::Uuid::from_u64_pair(
                0x2222_2222_2222_2222,
                0x2222_2222_2222_2222,
            )),
            LogicalRepositoryId(uuid::Uuid::from_u64_pair(
                0x3333_3333_3333_3333,
                0x3333_3333_3333_3333,
            )),
        ];
        let (_tmp, _paths, store, project_id, issue_id) = routing_store(targets.clone());
        let plan_id = "work_item_plan_h2b_0001";

        // A：前缀——只 prepare，不物化。
        let journal_a = prepare_for_target(
            &store,
            &project_id,
            &issue_id,
            plan_id,
            "wi_h2b_a",
            targets[0],
        );
        // B：物化 + AttemptPersisted，存活。
        let journal_b = prepare_for_target(
            &store,
            &project_id,
            &issue_id,
            plan_id,
            "wi_h2b_b",
            targets[1],
        );
        let guard_b = store
            .acquire_work_item_attempt_creation(
                &project_id,
                &issue_id,
                &journal_b.lock_work_item_id,
            )
            .unwrap();
        store
            .ensure_group_initialization_attempt(&journal_b, &guard_b)
            .expect("materialize B");
        let persisted_b = store
            .advance_group_initialization_phase(
                &journal_b,
                CodingGroupInitializationPhase::AttemptPersisted,
            )
            .expect("checkpoint B");
        // C：物化 + AttemptPersisted 后 attempt 被删（残留 journal）。
        let journal_c = prepare_for_target(
            &store,
            &project_id,
            &issue_id,
            plan_id,
            "wi_h2b_c",
            targets[2],
        );
        let guard_c = store
            .acquire_work_item_attempt_creation(
                &project_id,
                &issue_id,
                &journal_c.lock_work_item_id,
            )
            .unwrap();
        store
            .ensure_group_initialization_attempt(&journal_c, &guard_c)
            .expect("materialize C");
        let persisted_c = store
            .advance_group_initialization_phase(
                &journal_c,
                CodingGroupInitializationPhase::AttemptPersisted,
            )
            .expect("checkpoint C");
        crate::product::coding_attempt_store::remove_file_if_exists(&store.attempt_path(
            &project_id,
            &issue_id,
            &journal_c.attempt.id,
        ))
        .unwrap();

        let existing = vec![persisted_b.attempt.id.clone()];
        let mut references = store
            .classify_group_initialization_references(&project_id, &issue_id, plan_id, &existing)
            .unwrap();
        references.sort_by(|left, right| left.journal_id().cmp(right.journal_id()));
        let mut expected = vec![
            CodingGroupJournalReference::InitializationPrefix {
                journal_id: journal_a.id.clone(),
                attempt_id: journal_a.attempt.id.clone(),
            },
            CodingGroupJournalReference::BoundToAliveAttempt {
                journal_id: persisted_b.id.clone(),
                attempt_id: persisted_b.attempt.id.clone(),
            },
            CodingGroupJournalReference::BoundToDeadAttempt {
                journal_id: persisted_c.id.clone(),
                attempt_id: persisted_c.attempt.id.clone(),
            },
        ];
        expected.sort_by(|left, right| left.journal_id().cmp(right.journal_id()));
        assert_eq!(references, expected);
    }

    #[test]
    fn h2b_purge_journals_with_lease_clears_legacy_path_and_keeps_other_plans() {
        // 零成员闭包 journal 半边：Plan 独占 lease 内清 legacy 原路径
        // journal；他 Plan journal 不动；幂等；非 Plan scope 拒绝。
        let (_tmp, paths, store) = plain_store();
        store
            .prepare_group_initialization(
                &legacy_group_input("work_item_plan_0001"),
                "plan_revision_0001",
                &[legacy_unit("work_item_0001")],
            )
            .unwrap();
        store
            .prepare_group_initialization(
                &legacy_group_input("work_item_plan_0002"),
                "plan_revision_0002",
                &[legacy_unit("work_item_0001")],
            )
            .unwrap();

        let scope = EntityScope::new(
            PROJECT_ID,
            ISSUE_ID,
            DeletionEntityKind::Plan,
            "work_item_plan_0001",
        );
        let operations = EntityOperationStore::new(paths.clone());
        let lease = operations.try_acquire_delete(&scope).unwrap();
        store
            .purge_group_initialization_journals_with_lease(&lease)
            .unwrap();
        store
            .purge_group_initialization_journals_with_lease(&lease)
            .unwrap();
        drop(lease);

        assert!(
            store
                .get_group_initialization(PROJECT_ID, ISSUE_ID, "work_item_plan_0001")
                .is_err(),
            "purged plan journal must be gone"
        );
        assert_eq!(
            store
                .list_group_initialization_journals_for_plan(
                    PROJECT_ID,
                    ISSUE_ID,
                    "work_item_plan_0002"
                )
                .unwrap()
                .len(),
            1,
            "other plan journals must survive"
        );

        let story_scope = EntityScope::new(
            PROJECT_ID,
            ISSUE_ID,
            DeletionEntityKind::Story,
            "story_0001",
        );
        let wrong = operations.try_acquire_delete(&story_scope).unwrap();
        let error = store
            .purge_group_initialization_journals_with_lease(&wrong)
            .expect_err("non-Plan scope must be rejected");
        assert!(
            matches!(
                error,
                ProductStoreError::IdentityMismatch { kind, .. }
                    if kind == "coding_group_initialization_purge_scope"
            ),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn h2b_purge_journals_clears_per_target_subdir_scoped_to_plan() {
        // per-target 子目录形态：清 {plan}/{target}.json 全部 journal、不动
        // 他 Plan；幂等；子目录 journal 文件确实消失。
        let targets = vec![
            LogicalRepositoryId(uuid::Uuid::nil()),
            LogicalRepositoryId(uuid::Uuid::from_u64_pair(
                0x2222_2222_2222_2222,
                0x2222_2222_2222_2222,
            )),
        ];
        let (_tmp, paths, store, project_id, issue_id) = routing_store(targets.clone());
        let journal_a = prepare_for_target(
            &store,
            &project_id,
            &issue_id,
            "work_item_plan_h2b_purge",
            "wi_purge_a",
            targets[0],
        );
        let journal_b = prepare_for_target(
            &store,
            &project_id,
            &issue_id,
            "work_item_plan_h2b_purge",
            "wi_purge_b",
            targets[1],
        );
        let bystander = prepare_for_target(
            &store,
            &project_id,
            &issue_id,
            "work_item_plan_h2b_keep",
            "wi_keep_a",
            targets[0],
        );

        let scope = EntityScope::new(
            &project_id,
            &issue_id,
            DeletionEntityKind::Plan,
            "work_item_plan_h2b_purge",
        );
        let lease = EntityOperationStore::new(paths.clone())
            .try_acquire_delete(&scope)
            .unwrap();
        store
            .purge_group_initialization_journals_with_lease(&lease)
            .unwrap();
        store
            .purge_group_initialization_journals_with_lease(&lease)
            .unwrap();
        drop(lease);

        assert!(
            store
                .list_group_initialization_journals_for_plan(
                    &project_id,
                    &issue_id,
                    "work_item_plan_h2b_purge"
                )
                .unwrap()
                .is_empty()
        );
        assert!(
            !store
                .group_initialization_journal_path_for_target(
                    &project_id,
                    &issue_id,
                    "work_item_plan_h2b_purge",
                    &targets[0]
                )
                .is_file()
        );
        let kept = store
            .list_group_initialization_journals_for_plan(
                &project_id,
                &issue_id,
                "work_item_plan_h2b_keep",
            )
            .unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].id, bystander.id);
        assert_ne!(kept[0].attempt.id, journal_a.attempt.id);
        assert_ne!(kept[0].attempt.id, journal_b.attempt.id);
    }

    #[test]
    fn h2b_purge_journals_deletes_unparseable_residue() {
        // 半残现场：journal 内容损坏（`{}`）时归属已由路径界定（文件名/父目录
        // 即 plan_id），删除侧必须直接清掉并继续——残留不可解析不是删除失败
        // （REQ-WPS-03/04）。他 Plan 的完好 journal 不受影响。
        let target = LogicalRepositoryId(uuid::Uuid::from_u64_pair(
            0x3333_3333_3333_3333,
            0x3333_3333_3333_3333,
        ));
        let (_tmp, paths, store, project_id, issue_id) = routing_store(vec![target]);
        let _damaged = prepare_for_target(
            &store,
            &project_id,
            &issue_id,
            "work_item_plan_h2b_damaged",
            "wi_damaged",
            target,
        );
        let bystander = prepare_for_target(
            &store,
            &project_id,
            &issue_id,
            "work_item_plan_h2b_damaged_keep",
            "wi_keep",
            target,
        );
        let damaged_target_path = store.group_initialization_journal_path_for_target(
            &project_id,
            &issue_id,
            "work_item_plan_h2b_damaged",
            &target,
        );
        assert!(damaged_target_path.is_file());
        std::fs::write(&damaged_target_path, "{}").unwrap();
        // legacy 原路径同样损坏（半残现场两种形态并存）。
        let damaged_legacy = store.group_initialization_journal_path(
            &project_id,
            &issue_id,
            "work_item_plan_h2b_damaged",
        );
        std::fs::write(&damaged_legacy, "{}").unwrap();

        let scope = EntityScope::new(
            &project_id,
            &issue_id,
            DeletionEntityKind::Plan,
            "work_item_plan_h2b_damaged",
        );
        let lease = EntityOperationStore::new(paths.clone())
            .try_acquire_delete(&scope)
            .unwrap();
        store
            .purge_group_initialization_journals_with_lease(&lease)
            .expect("unparseable journal residue must not fail the purge");
        drop(lease);

        assert!(
            !damaged_target_path.exists(),
            "damaged per-target journal residue must be purged"
        );
        assert!(
            !damaged_legacy.exists(),
            "damaged legacy journal residue must be purged"
        );
        let kept = store
            .list_group_initialization_journals_for_plan(
                &project_id,
                &issue_id,
                "work_item_plan_h2b_damaged_keep",
            )
            .unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].id, bystander.id);
    }
}
