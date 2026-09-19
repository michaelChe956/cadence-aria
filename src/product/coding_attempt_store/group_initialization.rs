use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::product::coding_models::{
    CodingAdmissionKind, CodingAttemptPlanBinding, CodingAttemptScope, CodingAttemptStatus,
    CodingExecutionAttempt, CodingExecutionStage, CodingExecutionUnit, CodingExecutionUnitStatus,
    CodingRoleProviderConfigSnapshot,
};
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
    pub fn prepare_group_initialization_with_admission_for_target(
        &self,
        input: &CreateGroupCodingAttemptInput,
        bound_plan_revision_id: &str,
        unit_bindings: &[AuthoritativeCodingUnitBinding],
        admission_kind: CodingAdmissionKind,
        journal_target: Option<crate::product::logical_codebase::LogicalRepositoryId>,
    ) -> Result<CodingGroupInitializationJournal, ProductStoreError> {
        let routing =
            RepositoryRouting::load_for_issue(&self.paths, &input.project_id, &input.issue_id)?;
        let ordered_unit_bindings = if admission_kind == CodingAdmissionKind::ScAdvance {
            topologically_order_unit_bindings(unit_bindings)?
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
            worktree_lease_id: format!("issue_worktree_lease_{}", uuid::Uuid::new_v4().simple()),
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
            if !known.contains(dependency) || !dependencies.insert(dependency.clone()) {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_group_dependency_graph",
                    id: dependency.clone(),
                });
            }
            dependents
                .entry(dependency.clone())
                .or_default()
                .push(binding.logical_work_item_id.clone());
        }
        indegree.insert(binding.logical_work_item_id.clone(), dependencies.len());
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
