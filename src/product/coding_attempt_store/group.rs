use chrono::Utc;

use crate::product::coding_models::{
    CodingAdmissionKind, CodingAttemptScope, CodingAttemptStatus, CodingExecutionAttempt,
    CodingExecutionStage, CodingExecutionUnit, CodingExecutionUnitStatus,
    CodingRoleProviderConfigSnapshot, CodingUnitRunStatus,
};
use crate::product::id::next_sequential_id_in_directory;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{AmendmentResumeMode, PlanAmendmentManifest};
use std::collections::HashSet;

use super::locking::with_exclusive_lock;
use super::{CreateCodingExecutionUnitInput, CreateGroupCodingAttemptInput};

impl super::CodingAttemptStore {
    pub fn set_resume_target_from_manifest(
        &self,
        attempt: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
    ) -> Result<CodingExecutionUnit, ProductStoreError> {
        let current = self.validate_attempt_lineage(attempt)?;
        let mut units = self
            .list_coding_units(&current.project_id, &current.issue_id, &current.id)?
            .into_iter()
            .filter(|unit| {
                unit.logical_work_item_id == manifest.resume_target.logical_work_item_id
            });
        let target = units.next().ok_or_else(|| ProductStoreError::NotFound {
            kind: "coding_amendment_resume_target",
            id: manifest.resume_target.logical_work_item_id.clone(),
        })?;
        if units.next().is_some() {
            return Err(ProductStoreError::Ambiguous {
                kind: "coding_amendment_resume_target",
                id: manifest.resume_target.logical_work_item_id.clone(),
            });
        }
        let (unit_status, run_status) = match manifest.resume_target.mode {
            AmendmentResumeMode::Reexecute => (
                CodingExecutionUnitStatus::Running,
                CodingUnitRunStatus::Running,
            ),
            AmendmentResumeMode::Revalidate => (
                CodingExecutionUnitStatus::NeedsRevalidation,
                CodingUnitRunStatus::NeedsRevalidation,
            ),
            AmendmentResumeMode::AwaitHandoff => (
                CodingExecutionUnitStatus::AwaitingAmendment,
                CodingUnitRunStatus::AwaitingAmendment,
            ),
        };
        let target = self.update_coding_unit_status(
            &current.project_id,
            &current.issue_id,
            &current.id,
            &target.id,
            unit_status,
            Some(format!("Resume after Plan Amendment {}", manifest.id)),
        )?;
        self.set_materialized_amendment_unit_run_status(
            &current,
            manifest,
            &target.logical_work_item_id,
            run_status,
        )?;
        Ok(target)
    }

    /// REQ-MTG-02（WP2 检索细化，D2.1 A2）：per-`(plan, target)` 检索。
    ///
    /// - `Some(target)`：仅命中该 target 的 attempt（按 attempt 取最早，确定性
    ///   兜底）；无快照 attempt 不入任何 target 桶，永不作为某 target 的命中
    ///   返回（「取最早」歧义由 per-(plan,target) 检索+增殖审计面取代）。
    /// - `None`：无快照桶（legacy 语义保持——单 target/legacy 场景行为零变化；
    ///   routing 为 Legacy 时所有 attempt 均无快照）。
    pub fn get_attempt_for_work_item_group(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        target: Option<crate::product::logical_codebase::LogicalRepositoryId>,
    ) -> Result<Option<CodingExecutionAttempt>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(plan_id)?;
        let mut attempts: Vec<CodingExecutionAttempt> =
            super::list_json_records(&self.coding_attempts_root(project_id, issue_id))?
                .into_iter()
                .filter(|attempt: &CodingExecutionAttempt| {
                    attempt.work_item_group_id.as_deref() == Some(plan_id)
                        && Self::attempt_target_bucket(attempt) == target
                })
                .collect();
        attempts.sort_by(|left, right| {
            left.attempt_no
                .cmp(&right.attempt_no)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(attempts.into_iter().next())
    }

    /// REQ-MTG-02（WP2）：plan 级全量列表（消费面迭代形态——删除门禁/投影/
    /// amendment 路由用；按 attempt_no 升序）。单 target 场景与
    /// `get_attempt_for_work_item_group(..., None)` 的「取最早」结果一致。
    pub fn list_attempts_for_work_item_group(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<Vec<CodingExecutionAttempt>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(plan_id)?;
        let mut attempts: Vec<CodingExecutionAttempt> =
            super::list_json_records(&self.coding_attempts_root(project_id, issue_id))?
                .into_iter()
                .filter(|attempt: &CodingExecutionAttempt| {
                    attempt.work_item_group_id.as_deref() == Some(plan_id)
                })
                .collect();
        attempts.sort_by(|left, right| {
            left.attempt_no
                .cmp(&right.attempt_no)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(attempts)
    }

    /// attempt 的 target 桶键：快照在场取 `logical_repository_id`，否则 `None`
    /// （D2.1 A2/A4：无快照 attempt 不计入任何 target 桶）。
    pub(crate) fn attempt_target_bucket(
        attempt: &CodingExecutionAttempt,
    ) -> Option<crate::product::logical_codebase::LogicalRepositoryId> {
        attempt
            .target_snapshot
            .as_ref()
            .map(|snapshot| snapshot.logical_repository_id)
    }

    /// 建组输入的 target 桶键（输入快照在场取其 logical_repository_id）。
    pub(crate) fn attempt_target_bucket_from_input(
        input: &CreateGroupCodingAttemptInput,
    ) -> Option<crate::product::logical_codebase::LogicalRepositoryId> {
        input
            .target_snapshot
            .as_ref()
            .map(|snapshot| snapshot.logical_repository_id)
    }

    #[cfg(test)]
    pub fn ensure_group_attempt_for_advance(
        &self,
        input: &CreateGroupCodingAttemptInput,
        plan_revision_id: &str,
        unit_bindings: &[super::group_validation::AuthoritativeCodingUnitBinding],
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        validate_relative_id(plan_revision_id)?;
        let _initialization_guard =
            self.acquire_group_initialization_arbitration(&input.project_id, &input.issue_id)?;
        let creation_guard = self.acquire_work_item_attempt_creation(
            &input.project_id,
            &input.issue_id,
            &input.current_work_item_id,
        )?;
        if let Some(existing) = self.get_attempt_for_work_item_group(
            &input.project_id,
            &input.issue_id,
            &input.plan_id,
            Self::attempt_target_bucket_from_input(input),
        )? {
            if existing.admission_kind != CodingAdmissionKind::ScAdvance {
                return Err(ProductStoreError::Conflict {
                    kind: "coding_attempt_admission_kind",
                    id: existing.id,
                });
            }
            let binding = self.get_plan_binding(&existing)?;
            if binding.plan_id != input.plan_id
                || binding.bound_plan_revision_id != plan_revision_id
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "coding_attempt_plan_revision",
                    id: existing.id,
                });
            }
            return Ok(existing);
        }
        let journal = self.prepare_group_initialization_with_admission(
            input,
            plan_revision_id,
            unit_bindings,
            CodingAdmissionKind::ScAdvance,
        )?;
        self.ensure_group_initialization_attempt(&journal, &creation_guard)
    }

    pub fn create_group_attempt(
        &self,
        input: CreateGroupCodingAttemptInput,
    ) -> Result<CodingExecutionAttempt, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.plan_id)?;
        validate_relative_id(&input.current_work_item_id)?;
        super::validate_max_auto_rework(input.max_auto_rework)?;
        let input_target = Self::attempt_target_bucket_from_input(&input);
        // REQ-MTG-02（2.2 唯一性细化）：per-(plan,target) 第二 attempt 拒——
        // D2.1 A1：无快照 attempt 不参与 per-(plan,target) 唯一性判定。
        if let Some(existing) = self.get_attempt_for_work_item_group(
            &input.project_id,
            &input.issue_id,
            &input.plan_id,
            input_target,
        )? {
            return Err(ProductStoreError::Io(format!(
                "coding_attempt_group_already_exists: {}",
                existing.id
            )));
        }

        // REQ-MTG-02（2.2 单 active 细化）：issue 级单 active 细化为
        // per-(issue,target)——同 target 串行保留、异 target 并行解禁；
        // D2.1 A4：无快照 active attempt 不计入任何桶（物理互斥由三元键
        // worktree lock 兜底）。
        let existing_attempts: Vec<CodingExecutionAttempt> = super::list_json_records(
            &self.coding_attempts_root(&input.project_id, &input.issue_id),
        )?;
        if let Some(active) = existing_attempts.into_iter().find(|attempt| {
            attempt.status.is_active() && Self::attempt_target_bucket(attempt) == input_target
        }) {
            return Err(ProductStoreError::Io(format!(
                "active_coding_attempt_exists: {}",
                active.id
            )));
        }

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
        let attempt = CodingExecutionAttempt {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            work_item_id: input.current_work_item_id.clone(),
            attempt_no,
            scope: CodingAttemptScope::WorkItemGroup,
            status: CodingAttemptStatus::Created,
            version: 0,
            manual_recovery_reason: None,
            admission_ticket_consumed_at: None,
            admission_kind: CodingAdmissionKind::LegacyGroup,
            stage: CodingExecutionStage::PrepareContext,
            base_branch: input.base_branch,
            branch_name: input.branch_name,
            worktree_path: input.worktree_path,
            provider_config_snapshot: input.provider_config_snapshot,
            rework_count: 0,
            max_auto_rework: input.max_auto_rework,
            work_item_group_id: Some(input.plan_id),
            current_work_item_id: Some(input.current_work_item_id),
            active_unit_id: None,
            head_commit: None,
            pushed_remote: None,
            review_request_id: None,
            provider_conversations: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
            target_snapshot: input.target_snapshot,
            completed_at: None,
        };

        write_json(
            &self.attempt_path(&attempt.project_id, &attempt.issue_id, &id),
            &attempt,
        )?;
        write_json(
            &self.role_provider_config_path(&attempt.project_id, &attempt.issue_id, &id),
            &CodingRoleProviderConfigSnapshot::from(&attempt.provider_config_snapshot),
        )?;
        Ok(attempt)
    }

    pub fn create_coding_unit(
        &self,
        input: CreateCodingExecutionUnitInput,
    ) -> Result<CodingExecutionUnit, ProductStoreError> {
        validate_relative_id(&input.attempt_id)?;
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.plan_id)?;
        validate_relative_id(&input.logical_work_item_id)?;
        validate_relative_id(&input.work_item_revision_id)?;
        for dependency_id in &input.dependency_logical_work_item_ids {
            validate_relative_id(dependency_id)?;
        }
        let dependencies = input
            .dependency_logical_work_item_ids
            .iter()
            .collect::<HashSet<_>>();
        if input.logical_work_item_id == input.work_item_revision_id
            || dependencies.contains(&input.logical_work_item_id)
            || dependencies.len() != input.dependency_logical_work_item_ids.len()
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "coding_execution_unit_binding",
                id: input.logical_work_item_id.clone(),
            });
        }

        let attempt = self.get_attempt(&input.project_id, &input.issue_id, &input.attempt_id)?;
        if attempt.scope != CodingAttemptScope::WorkItemGroup {
            return Err(ProductStoreError::Io(format!(
                "coding_attempt_scope_invalid: {}",
                attempt.id
            )));
        }
        if attempt.work_item_group_id.as_deref() != Some(input.plan_id.as_str()) {
            return Err(ProductStoreError::Io(format!(
                "coding_attempt_plan_mismatch: {}",
                attempt.id
            )));
        }
        if input.status.is_active()
            && self
                .list_coding_units(&input.project_id, &input.issue_id, &input.attempt_id)?
                .into_iter()
                .any(|unit| unit.status.is_active())
        {
            return Err(ProductStoreError::Io(format!(
                "active_coding_unit_exists: {}",
                input.attempt_id
            )));
        }

        let root = self.coding_units_root(&input.project_id, &input.issue_id, &input.attempt_id);
        let id = next_sequential_id_in_directory("coding_unit", &root)
            .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", root.display())))?;
        let now = Utc::now().to_rfc3339();
        let started_at = if matches!(input.status, CodingExecutionUnitStatus::Running) {
            Some(now.clone())
        } else {
            None
        };
        let unit = CodingExecutionUnit {
            id: id.clone(),
            attempt_id: input.attempt_id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            plan_id: input.plan_id,
            logical_work_item_id: input.logical_work_item_id,
            work_item_revision_id: input.work_item_revision_id,
            dependency_logical_work_item_ids: input.dependency_logical_work_item_ids,
            order_index: input.order_index,
            status: input.status,
            started_at,
            completed_at: None,
            latest_handoff_revision_id: None,
            completion_commit: None,
            summary: None,
            created_at: now.clone(),
            updated_at: now,
        };
        write_json(
            &self.coding_unit_path(&unit.project_id, &unit.issue_id, &unit.attempt_id, &unit.id),
            &unit,
        )?;

        if unit.status.is_active() {
            let mut attempt = attempt;
            attempt.active_unit_id = Some(unit.id.clone());
            attempt.current_work_item_id = Some(unit.logical_work_item_id.clone());
            attempt.updated_at = Utc::now().to_rfc3339();
            self.update_attempt_non_status_fields(&attempt)?;
        }

        Ok(unit)
    }

    pub fn list_coding_units(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<CodingExecutionUnit>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        let mut units: Vec<CodingExecutionUnit> =
            super::list_json_records(&self.coding_units_root(project_id, issue_id, attempt_id))?;
        units.sort_by(|left, right| {
            left.order_index
                .cmp(&right.order_index)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(units)
    }

    pub fn get_active_coding_unit(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Option<CodingExecutionUnit>, ProductStoreError> {
        let mut active_units = self
            .list_coding_units(project_id, issue_id, attempt_id)?
            .into_iter()
            .filter(|unit| unit.status.is_active());
        let first = active_units.next();
        if let Some(extra) = active_units.next() {
            return Err(ProductStoreError::Io(format!(
                "active_coding_unit_ambiguous: {}",
                extra.id
            )));
        }
        Ok(first)
    }

    pub fn update_coding_unit_status(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        unit_id: &str,
        status: CodingExecutionUnitStatus,
        summary: Option<String>,
    ) -> Result<CodingExecutionUnit, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        validate_relative_id(unit_id)?;
        let attempt_path = self.attempt_path(project_id, issue_id, attempt_id);
        with_exclusive_lock(&attempt_path, || {
            let path = self.coding_unit_path(project_id, issue_id, attempt_id, unit_id);
            let mut unit: CodingExecutionUnit = read_json(&path)?;
            if status.is_active()
                && self
                    .list_coding_units(project_id, issue_id, attempt_id)?
                    .into_iter()
                    .any(|existing| existing.id != unit_id && existing.status.is_active())
            {
                return Err(ProductStoreError::Io(format!(
                    "active_coding_unit_exists: {}",
                    attempt_id
                )));
            }
            let now = Utc::now().to_rfc3339();
            if matches!(status, CodingExecutionUnitStatus::Running) && unit.started_at.is_none() {
                unit.started_at = Some(now.clone());
            }
            if matches!(
                status,
                CodingExecutionUnitStatus::Completed
                    | CodingExecutionUnitStatus::Failed
                    | CodingExecutionUnitStatus::Superseded
                    | CodingExecutionUnitStatus::Skipped
            ) {
                unit.completed_at = Some(now.clone());
            } else {
                // F-44：离开终态必须清除终态时间戳，否则留下「Running + completed_at」
                // 的矛盾记录。触发方有三类：①终态重开把中止时归一的 Skipped/Failed
                // unit 复位为 resume target（本批新增）；②`resume_attempt_after_amendment`
                // 的 `AmendmentResumeMode::Reexecute` 把 `Superseded` unit 置回 Running
                // （既有行为，此前会遗留 stale `completed_at`，现在一并清除——良性修正）；
                // ③ProviderFailure 把 `Blocked` unit 重新置为 Running（`Blocked` 本就
                // active、无 `completed_at`，等价空操作）。即：进终态写戳、离开终态清戳，
                // 与 attempt record 的 completed_at 语义对齐。
                unit.completed_at = None;
            }
            unit.status = status;
            unit.summary = summary;
            unit.updated_at = now;
            write_json(&path, &unit)?;

            let mut attempt = self.get_attempt(project_id, issue_id, attempt_id)?;
            match self.get_active_coding_unit(project_id, issue_id, attempt_id)? {
                Some(active) => {
                    attempt.active_unit_id = Some(active.id.clone());
                    attempt.current_work_item_id = Some(active.logical_work_item_id.clone());
                }
                None => {
                    attempt.active_unit_id = None;
                    attempt.current_work_item_id = None;
                }
            }
            attempt.updated_at = Utc::now().to_rfc3339();
            self.save_coding_attempt_with_status(&attempt)?;

            Ok(unit)
        })
    }

    pub fn update_coding_unit_latest_handoff_revision_id(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        unit_id: &str,
        latest_handoff_revision_id: Option<String>,
    ) -> Result<CodingExecutionUnit, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        validate_relative_id(unit_id)?;
        if let Some(handoff_revision_id) = latest_handoff_revision_id.as_deref() {
            validate_relative_id(handoff_revision_id)?;
        }
        let path = self.coding_unit_path(project_id, issue_id, attempt_id, unit_id);
        let mut unit: CodingExecutionUnit = read_json(&path)?;
        unit.latest_handoff_revision_id = latest_handoff_revision_id;
        unit.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &unit)?;
        Ok(unit)
    }

    pub fn update_coding_unit_completion_commit(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        unit_id: &str,
        completion_commit: Option<String>,
    ) -> Result<CodingExecutionUnit, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        validate_relative_id(unit_id)?;
        let path = self.coding_unit_path(project_id, issue_id, attempt_id, unit_id);
        let mut unit: CodingExecutionUnit = read_json(&path)?;
        unit.completion_commit = completion_commit;
        unit.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &unit)?;
        Ok(unit)
    }
}
