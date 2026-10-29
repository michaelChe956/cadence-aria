#[derive(Debug, Clone)]
pub struct AggregateInitializationOperationStore {
    paths: ProductAppPaths,
    lc_id: Option<String>,
}

impl AggregateInitializationOperationStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths, lc_id: None }
    }

    /// Scopes operation records to one logical codebase subtree
    /// (`aggregate-initializations/` under the v1.3 per-LC layout; the legacy
    /// alias codebase keeps the legacy project-scoped root).
    pub fn for_lc(paths: ProductAppPaths, lc_id: impl Into<String>) -> Self {
        Self {
            paths,
            lc_id: Some(lc_id.into()),
        }
    }

    fn operations_root(&self, project_id: &str) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        Ok(
            crate::product::logical_codebase::lc_scope_root(&self.paths, project_id, &self.lc_id)?
                .join("aggregate-initializations"),
        )
    }

    /// Create the operation idempotently. A retried create with the same
    /// idempotency identity (project, key, manifest revision, policy digest,
    /// profile/evidence digest) returns the existing record; any drift yields
    /// a conflict.
    pub fn create_idempotent(
        &self,
        operation: AggregateInitializationOperation,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        validate_initial_operation(&operation)?;
        let path = self.operation_path(&operation.project_id, &operation.operation_id)?;
        if path.exists() {
            let existing: AggregateInitializationOperation = read_json(&path)?;
            ensure_identity(&existing, &operation.project_id, &operation.operation_id)?;
            validate_record_shape(&existing)?;
            if existing.idempotency_identity() == operation.idempotency_identity()
                && existing == operation
            {
                return Ok(existing);
            }
            return Err(conflict(&operation.operation_id));
        }
        write_json(&path, &operation)?;
        Ok(operation)
    }

    /// C4 Task 3：只读列出当前 scope 的全部 operation，按 `updated_at` 降序
    ///（最新在前）供 bootstrap 投影组合；不写任何文件。
    pub fn list(
        &self,
        project_id: &str,
    ) -> Result<Vec<AggregateInitializationOperation>, ProductStoreError> {
        let root = self.operations_root(project_id)?;
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut operations = Vec::new();
        for entry in std::fs::read_dir(&root)
            .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", root.display())))?
        {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
            })?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let operation: AggregateInitializationOperation = read_json(&path)?;
            ensure_identity(&operation, project_id, &operation.operation_id)?;
            validate_record_shape(&operation)?;
            operations.push(operation);
        }
        operations.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
        Ok(operations)
    }

    pub fn get(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        let path = self.operation_path(project_id, operation_id)?;
        if !path.exists() {
            return Err(not_found(operation_id));
        }
        let operation: AggregateInitializationOperation = read_json(&path)?;
        ensure_identity(&operation, project_id, operation_id)?;
        validate_record_shape(&operation)?;
        Ok(operation)
    }

    pub fn mark_running(
        &self,
        project_id: &str,
        operation_id: &str,
        updated_at: String,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation.status != AggregateInitializationOperationStatus::Created {
                return Err(identity_mismatch(operation_id));
            }
            operation.status = AggregateInitializationOperationStatus::Running;
            operation.updated_at = updated_at;
            Ok(())
        })
    }

    /// Start a step. Requires the operation to be `Running`, all preceding
    /// steps to be `Completed`, and the target step to be `Pending`. The
    /// `input_digest` is captured on the step record so a replayed checkpoint
    /// can be matched.
    pub fn mark_step_running(
        &self,
        project_id: &str,
        operation_id: &str,
        step_id: AggregateInitializationStepKind,
        input_digest: String,
        updated_at: String,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation.status != AggregateInitializationOperationStatus::Running {
                return Err(identity_mismatch(operation_id));
            }
            let step_index = step_id.index();
            if operation.steps[..step_index]
                .iter()
                .any(|step| !is_completed_or_skipped_step(step))
            {
                return Err(identity_mismatch(operation_id));
            }
            let step = operation
                .steps
                .get(step_index)
                .ok_or_else(|| identity_mismatch(operation_id))?;
            if step.status == AggregateInitializationStepStatus::Running {
                return Ok(());
            }
            if step.status != AggregateInitializationStepStatus::Pending {
                return Err(identity_mismatch(operation_id));
            }
            if operation.current_step.is_some() {
                return Err(identity_mismatch(operation_id));
            }

            let step = operation
                .steps
                .get_mut(step_index)
                .ok_or_else(|| identity_mismatch(operation_id))?;
            step.status = AggregateInitializationStepStatus::Running;
            step.started_at = Some(updated_at.clone());
            step.completed_at = None;
            step.input_digest = Some(input_digest);
            operation.current_step = Some(step_id);
            operation.updated_at = updated_at;
            Ok(())
        })
    }

    /// Capture a step's output artifact reference before the step is completed.
    /// Required before `mark_step_completed` for every step.
    pub fn checkpoint_step_output(
        &self,
        project_id: &str,
        operation_id: &str,
        step_id: AggregateInitializationStepKind,
        output_artifact_ref: String,
        updated_at: String,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation.status != AggregateInitializationOperationStatus::Running
                || operation.current_step != Some(step_id)
            {
                return Err(identity_mismatch(operation_id));
            }
            let step_index = step_id.index();
            let step = operation
                .steps
                .get_mut(step_index)
                .ok_or_else(|| identity_mismatch(operation_id))?;
            if step.status != AggregateInitializationStepStatus::Running
                || step.output_artifact_ref.is_some()
            {
                return Err(identity_mismatch(operation_id));
            }
            step.output_artifact_ref = Some(output_artifact_ref);
            operation.updated_at = updated_at;
            Ok(())
        })
    }

    pub fn mark_step_completed(
        &self,
        project_id: &str,
        operation_id: &str,
        step_id: AggregateInitializationStepKind,
        updated_at: String,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation.status != AggregateInitializationOperationStatus::Running
                || operation.current_step != Some(step_id)
            {
                return Err(identity_mismatch(operation_id));
            }
            let step_index = step_id.index();
            let step = operation
                .steps
                .get_mut(step_index)
                .ok_or_else(|| identity_mismatch(operation_id))?;
            if step.status != AggregateInitializationStepStatus::Running
                || step.output_artifact_ref.is_none()
            {
                return Err(identity_mismatch(operation_id));
            }
            step.status = AggregateInitializationStepStatus::Completed;
            step.completed_at = Some(updated_at.clone());
            operation.current_step = None;
            operation.updated_at = updated_at;
            Ok(())
        })
    }

    /// Mark the operation completed once every step is `Completed`. Requires
    /// the operation to be `Running` with no active step, failure or
    /// cancellation.
    pub fn finish_completed(
        &self,
        project_id: &str,
        operation_id: &str,
        completed_at: String,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation.status != AggregateInitializationOperationStatus::Running
                || operation.current_step.is_some()
                || operation.failed_step.is_some()
                || operation.cancellation.is_some()
                || operation.error.is_some()
                || operation.completed_at.is_some()
                || !operation.steps.iter().all(is_completed_or_skipped_step)
            {
                return Err(identity_mismatch(operation_id));
            }
            operation.status = AggregateInitializationOperationStatus::Completed;
            operation.updated_at = completed_at.clone();
            operation.completed_at = Some(completed_at);
            Ok(())
        })
    }

    pub fn finish_failed(
        &self,
        project_id: &str,
        operation_id: &str,
        failed_step: Option<AggregateInitializationStepKind>,
        error: AggregateInitializationErrorRecord,
        completed_at: String,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation.status != AggregateInitializationOperationStatus::Running {
                return Err(identity_mismatch(operation_id));
            }
            if let Some(step_id) = failed_step {
                if operation.current_step != Some(step_id) {
                    return Err(identity_mismatch(operation_id));
                }
                let step_index = step_id.index();
                let step = operation
                    .steps
                    .get_mut(step_index)
                    .ok_or_else(|| identity_mismatch(operation_id))?;
                if step.status != AggregateInitializationStepStatus::Running {
                    return Err(identity_mismatch(operation_id));
                }
                step.status = AggregateInitializationStepStatus::Failed;
                step.completed_at = Some(completed_at.clone());
            } else if operation.current_step.is_some() {
                return Err(identity_mismatch(operation_id));
            }

            operation.status = AggregateInitializationOperationStatus::Failed;
            operation.failed_step = failed_step;
            operation.current_step = None;
            operation.cancellation = None;
            operation.error = Some(error);
            operation.updated_at = completed_at.clone();
            operation.completed_at = Some(completed_at);
            Ok(())
        })
    }

    pub fn cancel(
        &self,
        project_id: &str,
        operation_id: &str,
        cancellation: AggregateCancellationRecord,
        updated_at: String,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        let operation = self.update(project_id, operation_id, |operation| {
            if !matches!(
                operation.status,
                AggregateInitializationOperationStatus::Created
                    | AggregateInitializationOperationStatus::Running
            ) {
                return Err(identity_mismatch(operation_id));
            }
            if operation.cancellation.is_some() {
                return Err(identity_mismatch(operation_id));
            }
            if let Some(step_id) = operation.current_step {
                let step_index = step_id.index();
                let step = operation
                    .steps
                    .get_mut(step_index)
                    .ok_or_else(|| identity_mismatch(operation_id))?;
                if step.status == AggregateInitializationStepStatus::Running {
                    step.status = AggregateInitializationStepStatus::Failed;
                    step.completed_at = Some(updated_at.clone());
                    operation.failed_step = Some(step_id);
                }
            }
            operation.status = AggregateInitializationOperationStatus::Cancelled;
            operation.current_step = None;
            operation.cancellation = Some(cancellation);
            operation.updated_at = updated_at.clone();
            operation.completed_at = Some(updated_at);
            Ok(())
        })?;

        // Persisted cancellation is the source of truth. Drop any
        // operation-owned staging so a later explicit resume starts clean;
        // already-atomic-published digests are never guessed-rolled-back.
        let staging_root = self.staging_path(project_id, operation_id)?;
        if staging_root.exists() {
            std::fs::remove_dir_all(&staging_root).map_err(|error| {
                ProductStoreError::Io(format!(
                    "cancel could not delete staging {}: {error}",
                    staging_root.display()
                ))
            })?;
        }
        Ok(operation)
    }

    /// 记录「四条 Cadence-skills 命令是否必要」的判定依据（owner
    /// 2026-10-29）。只用于不改变步骤状态的判定（`forced`：显式重新
    /// 初始化）；`skipped` 判定必须走 [`Self::skip_provider_turn_steps`]，
    /// 让判定与步骤状态在同一次 durable 写里互相可证。已记录判定重放时
    /// 幂等返回，绝不改写冻结依据。
    pub fn record_skills_initialization(
        &self,
        project_id: &str,
        operation_id: &str,
        decision: AggregateSkillsInitialization,
        updated_at: String,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation.status != AggregateInitializationOperationStatus::Running {
                return Err(identity_mismatch(operation_id));
            }
            if decision.kind == AggregateSkillsInitializationKind::Skipped {
                return Err(identity_mismatch(operation_id));
            }
            match operation.skills_initialization.as_ref() {
                None => {
                    operation.skills_initialization = Some(decision);
                    operation.updated_at = updated_at;
                    Ok(())
                }
                Some(existing) if *existing == decision => Ok(()),
                Some(_) => Err(identity_mismatch(operation_id)),
            }
        })
    }

    /// 跳过三个 provider-turn 步骤（`PreCheck`/`RuleAndMcpConfig`/
    /// `OpenspecAndExamples`）：在**一次** durable 写里同时落盘判定依据与
    /// 三个 `Skipped` 步骤，绝不出现「判定跳过但没有跳过痕迹」或反之的
    /// 中间态。要求 operation Running、无进行中的步骤、两个确定性步骤
    /// 已 `Completed`、三个 provider-turn 步骤都还是 `Pending`，且此前没有
    /// 任何判定。判定后不可重判（重放幂等）。
    pub fn skip_provider_turn_steps(
        &self,
        project_id: &str,
        operation_id: &str,
        decision: AggregateSkillsInitialization,
        updated_at: String,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation.status != AggregateInitializationOperationStatus::Running
                || operation.current_step.is_some()
                || decision.kind != AggregateSkillsInitializationKind::Skipped
                || decision.markers.is_empty()
            {
                return Err(identity_mismatch(operation_id));
            }
            match operation.skills_initialization.as_ref() {
                Some(existing) if *existing == decision => return Ok(()),
                Some(_) => return Err(identity_mismatch(operation_id)),
                None => {}
            }
            // 防御：判定前绝不允许已有 Skipped 步骤（正常走不到，但形状
            // 不变量在这里再兜一次）。
            if operation
                .steps
                .iter()
                .any(|step| step.status == AggregateInitializationStepStatus::Skipped)
            {
                return Err(identity_mismatch(operation_id));
            }
            for step in [
                AggregateInitializationStepKind::PreCheck,
                AggregateInitializationStepKind::RuleAndMcpConfig,
                AggregateInitializationStepKind::OpenspecAndExamples,
            ] {
                let record = &operation.steps[step.index()];
                if record.status != AggregateInitializationStepStatus::Pending {
                    return Err(identity_mismatch(operation_id));
                }
            }
            if !operation.steps[..AggregateInitializationStepKind::PreCheck.index()]
                .iter()
                .all(is_completed_step)
            {
                return Err(identity_mismatch(operation_id));
            }
            let reason_code = decision.reason_code.clone();
            operation.skills_initialization = Some(decision);
            for step in [
                AggregateInitializationStepKind::PreCheck,
                AggregateInitializationStepKind::RuleAndMcpConfig,
                AggregateInitializationStepKind::OpenspecAndExamples,
            ] {
                let record = &mut operation.steps[step.index()];
                record.status = AggregateInitializationStepStatus::Skipped;
                record.started_at = None;
                record.completed_at = Some(updated_at.clone());
                record.input_digest = None;
                record.output_artifact_ref = None;
                record.skip_reason = Some(reason_code.clone());
            }
            operation.updated_at = updated_at;
            Ok(())
        })
    }

    /// C4 Task 4：显式续跑前的 reopen——Failed operation 回到 Running，
    /// 仅把 Failed 状态的步骤重置为 Pending；Completed 步骤（含其
    /// checkpoint/output artifact ref）原样保留，使续跑不重复已完成
    /// provider turn。Cancelled/Completed/非 Failed 一律拒绝。
    pub fn reopen_for_resume(
        &self,
        project_id: &str,
        operation_id: &str,
        updated_at: String,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation.status != AggregateInitializationOperationStatus::Failed {
                return Err(identity_mismatch(operation_id));
            }
            for step in operation.steps.iter_mut() {
                if step.status == AggregateInitializationStepStatus::Failed {
                    step.status = AggregateInitializationStepStatus::Pending;
                    step.started_at = None;
                    step.completed_at = None;
                    // Pending 步骤的 durable 形状不变量不含部分 checkpoint
                    //（`is_pending_step` 要求 input_digest/output_ref 为空）；
                    // 重跑时由 mark_step_running 重新落盘。不清除会让真实
                    // 协调器产生的 Failed operation（失败步必带 input_digest）
                    // 在重开后的形状校验上 fail-closed（C4 Task 10 红灯发现）。
                    step.input_digest = None;
                    step.output_artifact_ref = None;
                }
            }
            operation.status = AggregateInitializationOperationStatus::Running;
            operation.failed_step = None;
            operation.current_step = None;
            operation.error = None;
            operation.completed_at = None;
            operation.updated_at = updated_at;
            Ok(())
        })
    }

    /// C4 Task 6：把显式 bootstrap 动作的命令审计事实追加到既有
    /// operation 记录（同 command 重放的判定来源）。不改变步骤状态。
    pub fn record_action(
        &self,
        project_id: &str,
        operation_id: &str,
        record: crate::product::logical_codebase::aggregate_initialization::AggregateInitializationActionRecord,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        self.update(project_id, operation_id, |operation| {
            if operation
                .action_records
                .iter()
                .any(|existing| existing.command_id == record.command_id)
            {
                return Ok(());
            }
            operation.action_records.push(record);
            Ok(())
        })
    }

    pub fn recover_interrupted(
        &self,
        project_id: &str,
        operation_id: &str,
        completed_at: String,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        let operation = self.update(project_id, operation_id, |operation| {
            if matches!(
                operation.status,
                AggregateInitializationOperationStatus::Completed
                    | AggregateInitializationOperationStatus::Failed
                    | AggregateInitializationOperationStatus::Cancelled
            ) {
                return Ok(());
            }

            let failed_step = operation
                .steps
                .iter_mut()
                .find(|step| step.status == AggregateInitializationStepStatus::Running)
                .map(|step| {
                    step.status = AggregateInitializationStepStatus::Failed;
                    step.completed_at = Some(completed_at.clone());
                    step.step_id
                });
            operation.status = AggregateInitializationOperationStatus::Failed;
            operation.failed_step = failed_step;
            operation.current_step = None;
            operation.cancellation = None;
            operation.error = Some(AggregateInitializationErrorRecord::interrupted());
            operation.updated_at = completed_at.clone();
            operation.completed_at = Some(completed_at);
            Ok(())
        })?;

        // Recovery never auto-restarts the provider. Any operation-owned
        // staging left behind by the interrupted step is deleted so a later
        // explicit resume starts from a clean slate; the persisted record
        // above is the only source of truth. A missing staging directory is
        // not an error (the step may not have written any yet).
        let staging_root = self.staging_path(project_id, operation_id)?;
        if staging_root.exists() {
            std::fs::remove_dir_all(&staging_root).map_err(|error| {
                ProductStoreError::Io(format!(
                    "recover_interrupted could not delete staging {}: {error}",
                    staging_root.display()
                ))
            })?;
        }
        Ok(operation)
    }

    fn operation_path(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(operation_id)?;
        Ok(self
            .operations_root(project_id)?
            .join(format!("{operation_id}.json")))
    }

    /// The operation-owned staging directory used by an in-flight step to
    /// buffer partial output before it is checkpointed. It lives next to the
    /// operation record under the aggregate-initializations root and is
    /// deleted on cancel/recover so a later explicit resume starts clean.
    pub fn staging_path(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(operation_id)?;
        Ok(self
            .operations_root(project_id)?
            .join(operation_id)
            .join("staging"))
    }

    fn update(
        &self,
        project_id: &str,
        operation_id: &str,
        update: impl FnOnce(&mut AggregateInitializationOperation) -> Result<(), ProductStoreError>,
    ) -> Result<AggregateInitializationOperation, ProductStoreError> {
        let path = self.operation_path(project_id, operation_id)?;
        if !path.exists() {
            return Err(not_found(operation_id));
        }
        let mut operation: AggregateInitializationOperation = read_json(&path)?;
        ensure_identity(&operation, project_id, operation_id)?;
        validate_record_shape(&operation)?;
        update(&mut operation)?;
        validate_record_shape(&operation)?;
        write_json(&path, &operation)?;
        Ok(operation)
    }
}
