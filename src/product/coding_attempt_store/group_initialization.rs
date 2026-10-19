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

impl super::CodingAttemptStore {
    pub fn acquire_group_initialization_arbitration(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<CodingGroupInitializationGuard, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        Ok(CodingGroupInitializationGuard {
            _lock: ExclusiveFileLock::acquire(
                &self.group_initialization_arbitration_path(project_id, issue_id),
            )?,
        })
    }

    pub async fn acquire_group_initialization_arbitration_async(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<CodingGroupInitializationGuard, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        Ok(CodingGroupInitializationGuard {
            _lock: ExclusiveFileLock::acquire_async(
                &self.group_initialization_arbitration_path(project_id, issue_id),
            )
            .await?,
        })
    }

    pub fn prepare_group_initialization(
        &self,
        input: &CreateGroupCodingAttemptInput,
        bound_plan_revision_id: &str,
        unit_bindings: &[AuthoritativeCodingUnitBinding],
    ) -> Result<CodingGroupInitializationJournal, ProductStoreError> {
        self.prepare_group_initialization_with_admission(
            input,
            bound_plan_revision_id,
            unit_bindings,
            CodingAdmissionKind::LegacyGroup,
        )
    }

    pub fn prepare_group_initialization_with_admission(
        &self,
        input: &CreateGroupCodingAttemptInput,
        bound_plan_revision_id: &str,
        unit_bindings: &[AuthoritativeCodingUnitBinding],
        admission_kind: CodingAdmissionKind,
    ) -> Result<CodingGroupInitializationJournal, ProductStoreError> {
        self.prepare_group_initialization_with_admission_for_target(
            input,
            bound_plan_revision_id,
            unit_bindings,
            admission_kind,
            None,
        )
    }

    /// REQ-MTG-02（OQ2 journal 路径规则，WP2 分流创建消费）：多 target 拆分的
    /// per-target journal 变体。`journal_target=Some(t)` 时 journal 落
    /// `group-initializations/{plan_id}/{t}.json` 子目录（单 target 保持原路径
    /// `{plan_id}.json` 零迁移——本函数对既有调用方（`None`）行为零变化）。
    /// attempt 级前置（唯一性/单 active）按输入快照 target 桶细化，与存储路径
    /// 键无关。
    ///
    /// N1 依赖同源（REQ-CS-03/08）：`journal_target=Some(t)` 且 ScAdvance 时
    /// `unit_bindings` 期望是**权威 plan 全 units**——先全图校验（真 unknown/
    /// 重复/self/全局环失败关闭），再仅按桶内边排序 `t` 桶，journal units 保留
    /// 全部原 depends（外部 depends 不删，plan gate 消费）。`None` 保持全集
    /// 拓扑序旧语义。
    pub fn prepare_group_initialization_with_admission_for_target(
        &self,
        input: &CreateGroupCodingAttemptInput,
        bound_plan_revision_id: &str,
        unit_bindings: &[AuthoritativeCodingUnitBinding],
        admission_kind: CodingAdmissionKind,
        journal_target: Option<crate::product::logical_codebase::LogicalRepositoryId>,
    ) -> Result<CodingGroupInitializationJournal, ProductStoreError> {
        // D2.1c：journal 准备/重放入口持 owning Plan scope 共享准入（含读
        // 重放命中路径——删除窗口内的 journal 恢复不得继续走到 ensure）。
        // advance 编排在 arbitration 锁内调用时本 try 准入仍 fail-closed
        // 拒墓碑（D2.1d 将在 advance 入口外层持同 Plan lease）。
        self.with_plan_live_write(&input.project_id, &input.issue_id, &input.plan_id, || {
            self.prepare_group_initialization_with_admission_for_target_inner(
                input,
                bound_plan_revision_id,
                unit_bindings,
                admission_kind,
                journal_target,
            )
        })
    }

    fn prepare_group_initialization_with_admission_for_target_inner(
        &self,
        input: &CreateGroupCodingAttemptInput,
        bound_plan_revision_id: &str,
        unit_bindings: &[AuthoritativeCodingUnitBinding],
        admission_kind: CodingAdmissionKind,
        journal_target: Option<crate::product::logical_codebase::LogicalRepositoryId>,
    ) -> Result<CodingGroupInitializationJournal, ProductStoreError> {
        let routing =
            // C4 Task 2：issue 身份先经唯一 authority resolver 冻结（conflict
            // fail-closed）；旧数据无归属/无别名 record 时回退 Legacy。
            match crate::product::logical_codebase::RepositoryAuthorityResolver::new(
                self.paths.clone(),
            )
            .resolve_for_issue(&input.project_id, &input.issue_id)?
            {
                None => RepositoryRouting::Legacy {
                    repository_id: String::new(),
                },
                Some(resolution) => RepositoryRouting::classify(
                    resolution.manifest,
                    resolution.selection,
                ),
            };
        let ordered_unit_bindings = if admission_kind == CodingAdmissionKind::ScAdvance {
            match journal_target {
                // N1：全图校验 → 桶内边排序，外部 depends 保留。
                Some(target) => topologically_order_target_unit_bindings(unit_bindings, target)?,
                None => topologically_order_unit_bindings(unit_bindings)?,
            }
        } else {
            unit_bindings.to_vec()
        };
        validate_group_initialization_input(
            input,
            bound_plan_revision_id,
            &ordered_unit_bindings,
            &routing,
        )?;
        let path = match journal_target {
            None => self.group_initialization_journal_path(
                &input.project_id,
                &input.issue_id,
                &input.plan_id,
            ),
            Some(target) => self.group_initialization_journal_path_for_target(
                &input.project_id,
                &input.issue_id,
                &input.plan_id,
                &target,
            ),
        };
        if super::path_is_regular_file(&path)? {
            let journal: CodingGroupInitializationJournal = read_json(&path)?;
            validate_group_initialization_journal(&journal)?;
            if journal_matches_request(
                &journal,
                input,
                bound_plan_revision_id,
                &ordered_unit_bindings,
                admission_kind,
            ) {
                return Ok(journal);
            }
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "initialization journal identity differs from the authoritative request",
            ));
        }

        let input_target = super::CodingAttemptStore::attempt_target_bucket_from_input(input);
        // REQ-MTG-02（2.2 唯一性细化，D2.1 A1）：per-(plan,target) 前置——无快照
        // attempt 不参与唯一性判定（不阻塞 target-attempt 的 journal 准备）。
        if let Some(existing) = self.get_attempt_for_work_item_group(
            &input.project_id,
            &input.issue_id,
            &input.plan_id,
            input_target,
        )? {
            return Err(incomplete_group_attempt(
                &existing.id,
                "group attempt exists without an initialization journal",
            ));
        }
        let existing_attempts: Vec<CodingExecutionAttempt> = super::list_json_records(
            &self.coding_attempts_root(&input.project_id, &input.issue_id),
        )?;
        // REQ-MTG-02（2.2 单 active 细化，D2.1 A4）：per-(issue,target)——无快照
        // active attempt 不计入任何桶。
        if let Some(active) = existing_attempts.into_iter().find(|attempt| {
            attempt.status.is_active()
                && super::CodingAttemptStore::attempt_target_bucket(attempt) == input_target
        }) {
            return Err(ProductStoreError::Io(format!(
                "active_coding_attempt_exists: {}",
                active.id
            )));
        }

        let journal = self.build_group_initialization_journal(
            input,
            bound_plan_revision_id,
            &ordered_unit_bindings,
            admission_kind,
            journal_target,
        )?;
        write_json(&path, &journal)?;
        Ok(journal)
    }

    pub fn get_group_initialization(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<CodingGroupInitializationJournal, ProductStoreError> {
        for id in [project_id, issue_id, plan_id] {
            validate_relative_id(id)?;
        }
        let path = self.group_initialization_journal_path(project_id, issue_id, plan_id);
        if !super::path_is_regular_file(&path)? {
            return Err(ProductStoreError::NotFound {
                kind: "coding_group_initialization_journal",
                id: plan_id.to_string(),
            });
        }
        let journal: CodingGroupInitializationJournal = read_json(&path)?;
        validate_group_initialization_journal(&journal)?;
        if journal.project_id != project_id
            || journal.issue_id != issue_id
            || journal.plan_id != plan_id
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_group_initialization_journal",
                id: plan_id.to_string(),
            });
        }
        Ok(journal)
    }

    /// REQ-MTG-02（OQ2 读取规则，WP2）：plan 的全部初始化 journal——先试原路径
    /// （存量单 target journal 零迁移），原路径不存在再列 `{plan_id}/` 子目录
    /// （按文件名=target UUID 升序，确定性）。
    pub fn list_group_initialization_journals_for_plan(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<Vec<CodingGroupInitializationJournal>, ProductStoreError> {
        for id in [project_id, issue_id, plan_id] {
            validate_relative_id(id)?;
        }
        let original = self.group_initialization_journal_path(project_id, issue_id, plan_id);
        if super::path_is_regular_file(&original)? {
            return Ok(vec![
                self.get_group_initialization(project_id, issue_id, plan_id)?,
            ]);
        }
        let directory = self
            .coding_attempts_root(project_id, issue_id)
            .join("group-initializations")
            .join(plan_id);
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Vec::new());
            }
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read {}: {error}",
                    directory.display()
                )));
            }
        };
        let mut paths = Vec::new();
        for entry in entries {
            let path = entry
                .map_err(|error| {
                    ProductStoreError::Io(format!("read {} entry: {error}", directory.display()))
                })?
                .path();
            if path.extension().and_then(|value| value.to_str()) == Some("json") {
                paths.push(path);
            }
        }
        paths.sort();
        let mut journals = Vec::with_capacity(paths.len());
        for path in paths {
            let journal: CodingGroupInitializationJournal = read_json(&path)?;
            validate_group_initialization_journal(&journal)?;
            if journal.project_id != project_id
                || journal.issue_id != issue_id
                || journal.plan_id != plan_id
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_group_initialization_journal",
                    id: plan_id.to_string(),
                });
            }
            journals.push(journal);
        }
        Ok(journals)
    }

    pub fn ensure_group_initialization_attempt(
        &self,
        journal: &CodingGroupInitializationJournal,
        guard: &WorkItemAttemptCreationGuard,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        validate_group_initialization_journal(journal)?;
        guard.validate_identity(
            self,
            &journal.project_id,
            &journal.issue_id,
            &journal.lock_work_item_id,
        )?;
        // D2.1c：attempt 持久化消费 guard 已持的 owning Plan 共享准入
        //（scope 在 creation 锁外层取得，不嵌套重取）；guard 无归属
        //（legacy lock WI）或归属与 journal plan 不一致时，以 journal.plan_id
        // 自身的准入兜底——purging/deleted 墓碑后的 journal 恢复创建稳定
        // 拒绝（REQ-DEL-05：不能检查后另一入口再创建）。
        let guard_admits_journal_plan = guard.admitted_plan_scope().is_some_and(|scope| {
            scope.kind == crate::product::entity_deletion_store::DeletionEntityKind::Plan
                && scope.project_id == journal.project_id
                && scope.issue_id == journal.issue_id
                && scope.entity_id == journal.plan_id
        });
        if guard_admits_journal_plan {
            self.ensure_group_initialization_attempt_inner(journal)
        } else {
            self.with_plan_live_write(
                &journal.project_id,
                &journal.issue_id,
                &journal.plan_id,
                || self.ensure_group_initialization_attempt_inner(journal),
            )
        }
    }

    fn ensure_group_initialization_attempt_inner(
        &self,
        journal: &CodingGroupInitializationJournal,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        // REQ-MTG-02（2.2 异 target 并行解禁）：active 检查按 target 桶细化——
        // 本 journal 的桶内必须恰为自身；异 target active（D2.1 A4 无快照亦同）
        // 不阻塞。
        let journal_target = Self::attempt_target_bucket(&journal.attempt);
        let active_attempts = super::list_json_records::<CodingExecutionAttempt>(
            &self.coding_attempts_root(&journal.project_id, &journal.issue_id),
        )?
        .into_iter()
        .filter(|attempt| {
            attempt.status.is_active() && Self::attempt_target_bucket(attempt) == journal_target
        })
        .collect::<Vec<_>>();
        let attempt_path =
            self.attempt_path(&journal.project_id, &journal.issue_id, &journal.attempt.id);
        if super::path_is_regular_file(&attempt_path)? {
            let existing: CodingExecutionAttempt = read_json(&attempt_path)?;
            if existing != journal.attempt {
                return Err(incomplete_group_attempt(
                    &journal.attempt.id,
                    "persisted attempt differs from initialization journal",
                ));
            }
            if active_attempts.len() != 1 || active_attempts[0].id != journal.attempt.id {
                return Err(incomplete_group_attempt(
                    &journal.attempt.id,
                    "another active attempt exists during initialization replay",
                ));
            }
        } else {
            // H2b（设计 A 节「journal 在而 attempts 亡」）：已越
            // AttemptPersisted 的 journal，其 attempt 曾被耐久物化过——
            // 此时缺失＝已被删除，不得用旧 snapshot 重建（复活旧 attempt
            // ID）；未达 AttemptPersisted 的缺失属既有初始化中断前缀，
            // 保持首物化续做，原协议零变化。
            if journal
                .phase
                .has_reached(CodingGroupInitializationPhase::AttemptPersisted)
            {
                return Err(incomplete_group_attempt(
                    &journal.attempt.id,
                    "persisted attempt is missing beyond AttemptPersisted; the old attempt id must not be revived",
                ));
            }
            if let Some(active) = active_attempts.first() {
                return Err(incomplete_group_attempt(
                    &journal.attempt.id,
                    &format!(
                        "active attempt {} differs from initialization journal",
                        active.id
                    ),
                ));
            }
            write_json(&attempt_path, &journal.attempt)?;
        }

        let provider_path = self.role_provider_config_path(
            &journal.project_id,
            &journal.issue_id,
            &journal.attempt.id,
        );
        if super::path_is_regular_file(&provider_path)? {
            let existing: CodingRoleProviderConfigSnapshot = read_json(&provider_path)?;
            if existing != journal.provider_config {
                return Err(incomplete_group_attempt(
                    &journal.attempt.id,
                    "provider config differs from initialization journal",
                ));
            }
        } else {
            write_json(&provider_path, &journal.provider_config)?;
        }
        Ok(journal.attempt.clone())
    }

    pub fn validate_materialized_group_initialization_attempt(
        &self,
        journal: &CodingGroupInitializationJournal,
        guard: &WorkItemAttemptCreationGuard,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        validate_group_initialization_journal(journal)?;
        guard.validate_identity(
            self,
            &journal.project_id,
            &journal.issue_id,
            &journal.lock_work_item_id,
        )?;
        let attempt_path =
            self.attempt_path(&journal.project_id, &journal.issue_id, &journal.attempt.id);
        if !super::path_is_regular_file(&attempt_path)? {
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "persisted attempt is missing during bound replay",
            ));
        }
        let persisted_attempt: CodingExecutionAttempt = read_json(&attempt_path)?;
        if persisted_attempt != journal.attempt {
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "persisted attempt differs from initialization journal",
            ));
        }
        // REQ-MTG-02（2.2）：同 ensure——active 检查按 target 桶细化。
        let journal_target = Self::attempt_target_bucket(&journal.attempt);
        let active_attempts = super::list_json_records::<CodingExecutionAttempt>(
            &self.coding_attempts_root(&journal.project_id, &journal.issue_id),
        )?
        .into_iter()
        .filter(|attempt| {
            attempt.status.is_active() && Self::attempt_target_bucket(attempt) == journal_target
        })
        .collect::<Vec<_>>();
        if active_attempts.len() != 1 || active_attempts[0].id != journal.attempt.id {
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "another active attempt exists during bound replay",
            ));
        }
        let provider_path = self.role_provider_config_path(
            &journal.project_id,
            &journal.issue_id,
            &journal.attempt.id,
        );
        if !super::path_is_regular_file(&provider_path)? {
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "provider config is missing during bound replay",
            ));
        }
        let provider_config: CodingRoleProviderConfigSnapshot = read_json(&provider_path)?;
        if provider_config != journal.provider_config {
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "provider config differs from initialization journal",
            ));
        }
        Ok(persisted_attempt)
    }

    pub fn ensure_group_initialization_plan_binding(
        &self,
        journal: &CodingGroupInitializationJournal,
    ) -> Result<(), ProductStoreError> {
        validate_group_initialization_journal(journal)?;
        self.save_plan_binding(&journal.attempt, &journal.plan_binding)
    }

    pub fn ensure_group_initialization_unit(
        &self,
        journal: &CodingGroupInitializationJournal,
        index: usize,
    ) -> Result<CodingExecutionUnit, ProductStoreError> {
        validate_group_initialization_journal(journal)?;
        let expected =
            journal
                .units
                .get(index)
                .ok_or_else(|| ProductStoreError::IdentityMismatch {
                    kind: "coding_group_initialization_unit",
                    id: index.to_string(),
                })?;
        let materialized =
            self.list_coding_units(&journal.project_id, &journal.issue_id, &journal.attempt.id)?;
        if materialized.len() > journal.units.len()
            || materialized
                .iter()
                .zip(journal.units.iter())
                .any(|(existing, expected)| existing != expected)
        {
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "materialized units differ from initialization journal",
            ));
        }
        if let Some(existing) = materialized.get(index) {
            return Ok(existing.clone());
        }
        if materialized.len() != index {
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "materialized units are not a contiguous journal prefix",
            ));
        }
        write_json(
            &self.coding_unit_path(
                &expected.project_id,
                &expected.issue_id,
                &expected.attempt_id,
                &expected.id,
            ),
            expected,
        )?;
        Ok(expected.clone())
    }

    pub fn mark_group_initialization_error(
        &self,
        journal: &CodingGroupInitializationJournal,
        error: &str,
    ) -> Result<CodingGroupInitializationJournal, ProductStoreError> {
        // D2.1c：失败标记是 journal 写——删除窗口内不得复活已 purge 的
        // journal（purging/deleted 墓碑稳定拒绝）。
        self.with_plan_live_write(
            &journal.project_id,
            &journal.issue_id,
            &journal.plan_id,
            || self.mark_group_initialization_error_inner(journal, error),
        )
    }
    fn mark_group_initialization_error_inner(
        &self,
        journal: &CodingGroupInitializationJournal,
        error: &str,
    ) -> Result<CodingGroupInitializationJournal, ProductStoreError> {
        validate_group_initialization_journal(journal)?;
        let path = self.locate_group_initialization_journal_path(journal)?;
        let mut current: CodingGroupInitializationJournal = read_json(&path)?;
        validate_group_initialization_journal(&current)?;
        if !same_group_initialization_identity(&current, journal) {
            return Err(incomplete_group_attempt(
                &journal.attempt.id,
                "initialization journal changed while recording failure",
            ));
        }
        current.error = Some(error.to_string());
        current.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &current)?;
        Ok(current)
    }
    pub fn advance_group_initialization_phase(
        &self,
        expected: &CodingGroupInitializationJournal,
        next: CodingGroupInitializationPhase,
    ) -> Result<CodingGroupInitializationJournal, ProductStoreError> {
        // D2.1c：相位推进是 journal 写——删除窗口内不得复活已 purge 的
        // journal（purging/deleted 墓碑稳定拒绝）。
        self.with_plan_live_write(
            &expected.project_id,
            &expected.issue_id,
            &expected.plan_id,
            || self.advance_group_initialization_phase_inner(expected, next),
        )
    }
    fn advance_group_initialization_phase_inner(
        &self,
        expected: &CodingGroupInitializationJournal,
        next: CodingGroupInitializationPhase,
    ) -> Result<CodingGroupInitializationJournal, ProductStoreError> {
        validate_group_initialization_journal(expected)?;
        let path = self.locate_group_initialization_journal_path(expected)?;
        let mut current: CodingGroupInitializationJournal = read_json(&path)?;
        validate_group_initialization_journal(&current)?;
        if !same_group_initialization_identity(&current, expected) {
            return Err(incomplete_group_attempt(
                &expected.attempt.id,
                "initialization journal changed during replay",
            ));
        }
        if current.phase.order() >= next.order() {
            return Ok(current);
        }
        if next.order() != current.phase.order() + 1 {
            return Err(incomplete_group_attempt(
                &expected.attempt.id,
                "initialization phase advance is not contiguous",
            ));
        }
        current.phase = next;
        current.error = None;
        current.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &current)?;
        Ok(current)
    }

    pub(crate) fn delete_group_initialization_for_attempt(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(), ProductStoreError> {
        let Some(_plan_id) = attempt.work_item_group_id.as_deref() else {
            return Ok(());
        };
        // 先查原路径（单 target journal——mismatch 保持原 fail-closed 报错），
        // 再查 per-target 子目录（异 target 的 sibling journal 跳过不误删）。
        let original = self.group_initialization_journal_path(
            &attempt.project_id,
            &attempt.issue_id,
            _plan_id,
        );
        if super::path_is_regular_file(&original)? {
            let journal: CodingGroupInitializationJournal = read_json(&original)?;
            validate_group_initialization_journal(&journal)?;
            if journal.attempt.id != attempt.id {
                return Err(incomplete_group_attempt(
                    &attempt.id,
                    "delete target differs from initialization journal",
                ));
            }
            return super::remove_file_if_exists(&original);
        }
        if let Some(target) = Self::attempt_target_bucket(attempt) {
            let target_path = self.group_initialization_journal_path_for_target(
                &attempt.project_id,
                &attempt.issue_id,
                _plan_id,
                &target,
            );
            if super::path_is_regular_file(&target_path)? {
                let journal: CodingGroupInitializationJournal = read_json(&target_path)?;
                validate_group_initialization_journal(&journal)?;
                if journal.attempt.id == attempt.id {
                    return super::remove_file_if_exists(&target_path);
                }
            }
        }
        Ok(())
    }

    // ─── H2b（设计 A 节，整组删除与重启）：journal 引用分类与闭包清理 ───

    /// 按调用方枚举的真实 attempt 存活集分类该 plan 全部初始化 journal
    ///（原路径 + per-target 子目录；坏 JSON/身份错配 fail-closed 同
    /// `list_group_initialization_journals_for_plan`）。判定只看 journal
    /// 相位与存活集成员关系，不读 attempt 文件（权威存活集归闭包/调用方，
    /// 不复制第二套扫描器）：未达 AttemptPersisted 的 journal 缺 attempt
    /// 属既有初始化中断前缀（`InitializationPrefix`，续原协议，不当残留
    /// 清）；已越 AttemptPersisted 按 attempt 存活分 `BoundToAlive` /
    /// `BoundToDead`（后者为执行引用断裂，零成员闭包清理对象）。
    pub fn classify_group_initialization_references(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        existing_attempt_ids: &[String],
    ) -> Result<Vec<CodingGroupJournalReference>, ProductStoreError> {
        for id in [project_id, issue_id, plan_id] {
            validate_relative_id(id)?;
        }
        let existing: std::collections::BTreeSet<String> =
            existing_attempt_ids.iter().cloned().collect();
        Ok(self
            .list_group_initialization_journals_for_plan(project_id, issue_id, plan_id)?
            .into_iter()
            .map(|journal| {
                let journal_id = journal.id.clone();
                let attempt_id = journal.attempt.id.clone();
                if journal
                    .phase
                    .has_reached(CodingGroupInitializationPhase::AttemptPersisted)
                {
                    if existing.contains(&attempt_id) {
                        CodingGroupJournalReference::BoundToAliveAttempt {
                            journal_id,
                            attempt_id,
                        }
                    } else {
                        CodingGroupJournalReference::BoundToDeadAttempt {
                            journal_id,
                            attempt_id,
                        }
                    }
                } else {
                    CodingGroupJournalReference::InitializationPrefix {
                        journal_id,
                        attempt_id,
                    }
                }
            })
            .collect())
    }

    /// H2b（设计 A 节完整重置闭包第 4 步）：清该 plan 的 group 初始化
    /// journal 残留（legacy 原路径 `{plan_id}.json` + per-target 子目录
    /// `{plan_id}/{target}.json`，含 attempts 已亡的 journal）。持精确
    /// Plan 独占 lease 消费（同 `coding_plan_run::purge_plan_with_lease`
    /// 模式）——租约即安全清理窗口，不嵌套重取 scope；missing 幂等；
    /// journal 身份/归属错配 fail-closed 不误删他 Plan 对象；不删
    /// `group-initializations` 根与 arbitration 公共锁。
    pub fn purge_group_initialization_journals_with_lease(
        &self,
        lease: &EntityDeletionLease,
    ) -> Result<(), ProductStoreError> {
        let scope = lease.scope();
        scope.validate()?;
        if scope.kind != DeletionEntityKind::Plan {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_group_initialization_purge_scope",
                id: scope.entity_id.clone(),
            });
        }
        let (project_id, issue_id, plan_id) = (
            scope.project_id.as_str(),
            scope.issue_id.as_str(),
            scope.entity_id.as_str(),
        );
        for id in [project_id, issue_id, plan_id] {
            validate_relative_id(id)?;
        }
        // ① legacy 原路径 `{plan_id}.json`。
        let original = self.group_initialization_journal_path(project_id, issue_id, plan_id);
        if super::path_is_regular_file(&original)? {
            let journal: CodingGroupInitializationJournal = read_json(&original)?;
            validate_group_initialization_journal(&journal)?;
            if journal.project_id != project_id
                || journal.issue_id != issue_id
                || journal.plan_id != plan_id
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_group_initialization_journal",
                    id: plan_id.to_string(),
                });
            }
            super::remove_file_if_exists(&original)?;
        }
        // ② per-target 子目录 `{plan_id}/{target}.json`（逐一身份核对；非
        //    json 条目跳过，坏 JSON/归属错配 fail-closed）。
        let directory = self
            .coding_attempts_root(project_id, issue_id)
            .join("group-initializations")
            .join(plan_id);
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read {}: {error}",
                    directory.display()
                )));
            }
        };
        for entry in entries {
            let path = entry
                .map_err(|error| {
                    ProductStoreError::Io(format!("read {} entry: {error}", directory.display()))
                })?
                .path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let journal: CodingGroupInitializationJournal = read_json(&path)?;
            validate_group_initialization_journal(&journal)?;
            if journal.project_id != project_id
                || journal.issue_id != issue_id
                || journal.plan_id != plan_id
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_group_initialization_journal",
                    id: plan_id.to_string(),
                });
            }
            super::remove_file_if_exists(&path)?;
        }
        Ok(())
    }

    /// OQ2 定位规则（WP2）：mark/advance 相位函数据此找到 journal 的实际存储
    /// 位置——先试原路径（存量单 target journal 零迁移），再试 attempt 快照
    /// 对应的 per-target 子目录；两处 id 均不匹配时回落原路径（由调用方的
    /// 既有校验产出 fail-closed 错误）。
    fn locate_group_initialization_journal_path(
        &self,
        journal: &CodingGroupInitializationJournal,
    ) -> Result<std::path::PathBuf, ProductStoreError> {
        let original = self.group_initialization_journal_path(
            &journal.project_id,
            &journal.issue_id,
            &journal.plan_id,
        );
        if super::path_is_regular_file(&original)? {
            let existing: CodingGroupInitializationJournal = read_json(&original)?;
            if existing.id == journal.id {
                return Ok(original);
            }
        }
        if let Some(target) = Self::attempt_target_bucket(&journal.attempt) {
            let target_path = self.group_initialization_journal_path_for_target(
                &journal.project_id,
                &journal.issue_id,
                &journal.plan_id,
                &target,
            );
            if super::path_is_regular_file(&target_path)? {
                let existing: CodingGroupInitializationJournal = read_json(&target_path)?;
                if existing.id == journal.id {
                    return Ok(target_path);
                }
            }
        }
        Ok(original)
    }

    fn build_group_initialization_journal(
        &self,
        input: &CreateGroupCodingAttemptInput,
        bound_plan_revision_id: &str,
        unit_bindings: &[AuthoritativeCodingUnitBinding],
        admission_kind: CodingAdmissionKind,
        journal_target: Option<crate::product::logical_codebase::LogicalRepositoryId>,
    ) -> Result<CodingGroupInitializationJournal, ProductStoreError> {
        let id = self.allocate_coding_attempt_id();
        let attempt_no = self
            .list_attempts_for_work_item(
                &input.project_id,
                &input.issue_id,
                &input.current_work_item_id,
            )?
            .iter()
            .map(|attempt| attempt.attempt_no)
            .max()
            .unwrap_or(0)
            + 1;
        let now = Utc::now().to_rfc3339();
        let first_unit_id = next_sequential_id("coding_unit", 0);
        let attempt = CodingExecutionAttempt {
            id: id.clone(),
            project_id: input.project_id.clone(),
            issue_id: input.issue_id.clone(),
            work_item_id: input.current_work_item_id.clone(),
            attempt_no,
            scope: CodingAttemptScope::WorkItemGroup,
            status: CodingAttemptStatus::Created,
            version: 0,
            manual_recovery_reason: None,
            admission_ticket_consumed_at: None,
            admission_kind,
            stage: CodingExecutionStage::PrepareContext,
            base_branch: input.base_branch.clone(),
            branch_name: input.branch_name.clone(),
            worktree_path: input.worktree_path.clone(),
            provider_config_snapshot: input.provider_config_snapshot.clone(),
            rework_count: 0,
            max_auto_rework: input.max_auto_rework,
            work_item_group_id: Some(input.plan_id.clone()),
            current_work_item_id: Some(input.current_work_item_id.clone()),
            active_unit_id: Some(first_unit_id),
            head_commit: None,
            pushed_remote: None,
            review_request_id: None,
            provider_conversations: Vec::new(),
            created_at: now.clone(),
            updated_at: now.clone(),
            target_snapshot: input.target_snapshot.clone(),
            start_run_policy: input.start_run_policy.clone(),
            start_claim: None,
            completed_at: None,
        };
        let units = unit_bindings
            .iter()
            .enumerate()
            .map(|(index, binding)| CodingExecutionUnit {
                id: next_sequential_id("coding_unit", index),
                attempt_id: id.clone(),
                project_id: input.project_id.clone(),
                issue_id: input.issue_id.clone(),
                plan_id: input.plan_id.clone(),
                logical_work_item_id: binding.logical_work_item_id.clone(),
                work_item_revision_id: binding.work_item_revision_id.clone(),
                dependency_logical_work_item_ids: binding.dependency_logical_work_item_ids.clone(),
                order_index: index as u32,
                status: if index == 0 {
                    CodingExecutionUnitStatus::Running
                } else {
                    CodingExecutionUnitStatus::Pending
                },
                started_at: (index == 0).then(|| now.clone()),
                completed_at: None,
                latest_handoff_revision_id: None,
                completion_commit: None,
                summary: None,
                created_at: now.clone(),
                updated_at: now.clone(),
            })
            .collect::<Vec<_>>();
        let journal = CodingGroupInitializationJournal {
            id: match journal_target {
                // OQ2：单 target journal id 保持原形态（存量零迁移）；多 target
                // per-target journal 以 target UUID 限定（同 plan 各 journal 身份
                // 可区分，定位规则按 id 匹配）。
                None => format!("coding_group_initialization_{}", input.plan_id),
                Some(target) => {
                    format!("coding_group_initialization_{}_{}", input.plan_id, target.0)
                }
            },
            project_id: input.project_id.clone(),
            issue_id: input.issue_id.clone(),
            plan_id: input.plan_id.clone(),
            lock_work_item_id: input.current_work_item_id.clone(),
            // 缺陷 #13 层2（组入口路由分流配套）：仓维锁 bind 只接受
            // `repo_worktree_lease_` 前缀租约；Logical 路由（journal_target
            // 存在）必须派生仓维前缀，Legacy 路由维持 issue 维前缀不变。
            worktree_lease_id: format!(
                "{}{}",
                if journal_target.is_some() {
                    "repo_worktree_lease_"
                } else {
                    "issue_worktree_lease_"
                },
                uuid::Uuid::new_v4().simple()
            ),
            provider_config: CodingRoleProviderConfigSnapshot::from(
                &input.provider_config_snapshot,
            ),
            plan_binding: CodingAttemptPlanBinding {
                attempt_id: id,
                plan_id: input.plan_id.clone(),
                bound_plan_revision_id: bound_plan_revision_id.to_string(),
                applied_amendment_ids: Vec::new(),
                updated_at: now.clone(),
            },
            attempt,
            units,
            phase: CodingGroupInitializationPhase::Prepared,
            error: None,
            created_at: now.clone(),
            updated_at: now,
        };
        validate_group_initialization_journal(&journal)?;
        Ok(journal)
    }
}

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
                logical_codebase_id: None,
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
}
