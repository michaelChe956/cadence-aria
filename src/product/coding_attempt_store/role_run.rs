use chrono::Utc;

use crate::product::coding_models::{
    CodingExecutionAttempt, CodingExecutionStage, CodingProviderRole, CodingRoleRun,
    CodingRoleRunRetryMetadata, CodingRoleRunStatus, CodingRoleRunTrigger,
};
use crate::product::id::next_sequential_id_in_directory;
use crate::product::json_store::{
    ProductStoreError, read_json, validate_relative_artifact_ref, validate_relative_id, write_json,
};

impl super::CodingAttemptStore {
    pub fn create_role_run(
        &self,
        attempt: &CodingExecutionAttempt,
        stage: CodingExecutionStage,
        role: CodingProviderRole,
        trigger: CodingRoleRunTrigger,
        node_id: Option<String>,
    ) -> Result<CodingRoleRun, ProductStoreError> {
        validate_relative_id(&attempt.project_id)?;
        validate_relative_id(&attempt.issue_id)?;
        validate_relative_id(&attempt.id)?;
        if let Some(node_id) = &node_id {
            validate_relative_id(node_id)?;
        }
        let role_runs_root =
            self.role_runs_root(&attempt.project_id, &attempt.issue_id, &attempt.id);
        let existing = self.list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let id = next_sequential_id_in_directory("coding_role_run", &role_runs_root).map_err(
            |error| ProductStoreError::Io(format!("read {}: {error}", role_runs_root.display())),
        )?;
        let run_no = existing
            .iter()
            .filter(|run| run.stage == stage && run.role == role)
            .map(|run| run.run_no)
            .max()
            .unwrap_or(0)
            + 1;
        let run = CodingRoleRun {
            id: id.clone(),
            attempt_id: attempt.id.clone(),
            stage,
            role,
            run_no,
            status: CodingRoleRunStatus::Running,
            trigger,
            retry_metadata: Some(CodingRoleRunRetryMetadata {
                cycle_id: id.clone(),
                attempt_no: 1,
                prior_run_id: None,
            }),
            node_id,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            supersedes_run_id: None,
            superseded_by_run_id: None,
            reason_code: None,
            raw_provider_output_refs: Vec::new(),
            artifact_refs: Vec::new(),
        };
        self.save_role_run(&attempt.project_id, &attempt.issue_id, &run)?;
        Ok(run)
    }

    pub fn create_retry_role_run(
        &self,
        attempt: &CodingExecutionAttempt,
        stage: CodingExecutionStage,
        role: CodingProviderRole,
        trigger: CodingRoleRunTrigger,
        node_id: Option<String>,
        retry: CodingRoleRunRetryMetadata,
    ) -> Result<CodingRoleRun, ProductStoreError> {
        validate_relative_id(&attempt.project_id)?;
        validate_relative_id(&attempt.issue_id)?;
        validate_relative_id(&attempt.id)?;
        validate_retry_metadata(&retry)?;
        if let Some(node_id) = &node_id {
            validate_relative_id(node_id)?;
        }

        let prior_run_id = retry
            .prior_run_id
            .as_deref()
            .ok_or_else(|| invalid_retry_metadata("missing_prior_run_id"))?;
        let prior = self.get_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            prior_run_id,
        )?;
        let Some(prior_retry) = prior.retry_metadata.as_ref() else {
            return Err(invalid_retry_metadata("prior_run_missing_retry_metadata"));
        };
        if prior.status != CodingRoleRunStatus::Failed
            || prior.stage != stage
            || prior.role != role
            || prior_retry.cycle_id != retry.cycle_id
            || prior_retry.attempt_no != retry.attempt_no - 1
        {
            return Err(invalid_retry_metadata(prior.id));
        }

        let role_runs_root =
            self.role_runs_root(&attempt.project_id, &attempt.issue_id, &attempt.id);
        let existing = self.list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        if existing.iter().any(|run| {
            run.retry_metadata.as_ref().is_some_and(|existing_retry| {
                existing_retry.cycle_id == retry.cycle_id
                    && existing_retry.attempt_no == retry.attempt_no
            })
        }) {
            return Err(invalid_retry_metadata(format!(
                "{}:{}",
                retry.cycle_id, retry.attempt_no
            )));
        }
        let id = next_sequential_id_in_directory("coding_role_run", &role_runs_root).map_err(
            |error| ProductStoreError::Io(format!("read {}: {error}", role_runs_root.display())),
        )?;
        let run_no = existing
            .iter()
            .filter(|run| run.stage == stage && run.role == role)
            .map(|run| run.run_no)
            .max()
            .unwrap_or(0)
            + 1;
        let run = CodingRoleRun {
            id,
            attempt_id: attempt.id.clone(),
            stage,
            role,
            run_no,
            status: CodingRoleRunStatus::Running,
            trigger,
            retry_metadata: Some(retry),
            node_id,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            supersedes_run_id: None,
            superseded_by_run_id: None,
            reason_code: None,
            raw_provider_output_refs: Vec::new(),
            artifact_refs: Vec::new(),
        };
        self.save_role_run(&attempt.project_id, &attempt.issue_id, &run)?;
        Ok(run)
    }

    pub fn create_manual_retry_role_run(
        &self,
        attempt: &CodingExecutionAttempt,
        stage: CodingExecutionStage,
        role: CodingProviderRole,
        prior: &CodingRoleRun,
        reason_code: Option<String>,
    ) -> Result<CodingRoleRun, ProductStoreError> {
        validate_relative_id(&attempt.project_id)?;
        validate_relative_id(&attempt.issue_id)?;
        validate_relative_id(&attempt.id)?;
        validate_relative_id(&prior.id)?;
        let role_runs_root =
            self.role_runs_root(&attempt.project_id, &attempt.issue_id, &attempt.id);
        let existing = self.list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let latest = existing
            .iter()
            .rev()
            .find(|run| run.stage == stage && run.role == role)
            .ok_or_else(|| invalid_retry_metadata("missing_prior_run"))?;
        if latest.id != prior.id
            || latest.attempt_id != attempt.id
            || latest.status != CodingRoleRunStatus::Failed
        {
            return Err(invalid_retry_metadata(prior.id.clone()));
        }

        let id = next_sequential_id_in_directory("coding_role_run", &role_runs_root).map_err(
            |error| ProductStoreError::Io(format!("read {}: {error}", role_runs_root.display())),
        )?;
        let run_no = existing
            .iter()
            .filter(|run| run.stage == stage && run.role == role)
            .map(|run| run.run_no)
            .max()
            .unwrap_or(0)
            + 1;
        let run = CodingRoleRun {
            id: id.clone(),
            attempt_id: attempt.id.clone(),
            stage,
            role,
            run_no,
            status: CodingRoleRunStatus::Running,
            trigger: CodingRoleRunTrigger::ManualRetry,
            retry_metadata: Some(CodingRoleRunRetryMetadata {
                cycle_id: id,
                attempt_no: 1,
                prior_run_id: Some(prior.id.clone()),
            }),
            node_id: None,
            started_at: Utc::now().to_rfc3339(),
            completed_at: None,
            supersedes_run_id: None,
            superseded_by_run_id: None,
            reason_code,
            raw_provider_output_refs: Vec::new(),
            artifact_refs: Vec::new(),
        };
        self.save_role_run(&attempt.project_id, &attempt.issue_id, &run)?;
        Ok(run)
    }

    pub fn list_role_runs(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
    ) -> Result<Vec<CodingRoleRun>, ProductStoreError> {
        let mut runs: Vec<CodingRoleRun> =
            super::list_json_records(&self.role_runs_root(project_id, issue_id, attempt_id))?;
        runs.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(runs)
    }

    pub fn latest_role_run(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        stage: CodingExecutionStage,
        role: CodingProviderRole,
    ) -> Result<Option<CodingRoleRun>, ProductStoreError> {
        Ok(self
            .list_role_runs(project_id, issue_id, attempt_id)?
            .into_iter()
            .rev()
            .find(|run| run.stage == stage && run.role == role))
    }

    pub fn update_role_run_status(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_run_id: &str,
        status: CodingRoleRunStatus,
        reason_code: Option<String>,
    ) -> Result<CodingRoleRun, ProductStoreError> {
        let mut run = self.get_role_run(project_id, issue_id, attempt_id, role_run_id)?;
        run.status = status;
        run.reason_code = reason_code;
        run.completed_at = Some(Utc::now().to_rfc3339());
        self.save_role_run(project_id, issue_id, &run)?;
        Ok(run)
    }

    pub fn attach_role_run_node(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_run_id: &str,
        node_id: String,
    ) -> Result<CodingRoleRun, ProductStoreError> {
        validate_relative_id(&node_id)?;
        let mut run = self.get_role_run(project_id, issue_id, attempt_id, role_run_id)?;
        run.node_id = Some(node_id);
        self.save_role_run(project_id, issue_id, &run)?;
        Ok(run)
    }

    pub fn supersede_latest_role_run_and_create(
        &self,
        attempt: &CodingExecutionAttempt,
        stage: CodingExecutionStage,
        role: CodingProviderRole,
        trigger: CodingRoleRunTrigger,
        node_id: Option<String>,
        reason_code: Option<String>,
    ) -> Result<CodingRoleRun, ProductStoreError> {
        let previous = self.latest_role_run(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            stage.clone(),
            role.clone(),
        )?;
        let mut next = self.create_role_run(attempt, stage, role, trigger, node_id)?;
        next.supersedes_run_id = previous.as_ref().map(|run| run.id.clone());
        next.reason_code = reason_code;
        self.save_role_run(&attempt.project_id, &attempt.issue_id, &next)?;
        if let Some(mut previous_run) = previous {
            previous_run.status = CodingRoleRunStatus::Superseded;
            previous_run.superseded_by_run_id = Some(next.id.clone());
            previous_run.completed_at = Some(Utc::now().to_rfc3339());
            self.save_role_run(&attempt.project_id, &attempt.issue_id, &previous_run)?;
        }
        Ok(next)
    }

    /// S4：runner 死亡转 AwaitingManualRecovery 时的同步收尾——该 attempt
    /// 名下仍处于 Running 的 role run 的唯一终态写方（runner）已死，不收尾
    /// 会让 RoleRunHistoryPanel 对已 fail-closed 的 attempt 永久投影「运行
    /// 中」。统一收尾为 Failed，并携带 attempt 同款稳定 reason 码；恢复
    /// journal 持有的 retry run 豁免（见 `recovery_journal_retry_exemption`）。
    /// 返回收尾条数（0 表示本无残留）。
    pub fn fail_running_role_runs_for_attempt(
        &self,
        attempt: &CodingExecutionAttempt,
        reason_code: &str,
    ) -> Result<usize, ProductStoreError> {
        let (exempt_run_id, exempt_reason) = self.recovery_journal_retry_exemption(attempt)?;
        let mut finalized = 0usize;
        for mut run in self.list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)? {
            if run.status != CodingRoleRunStatus::Running {
                continue;
            }
            if is_journal_held_retry_run(&run, exempt_run_id.as_deref(), exempt_reason.as_deref()) {
                continue;
            }
            run.status = CodingRoleRunStatus::Failed;
            run.reason_code = Some(reason_code.to_string());
            run.completed_at = Some(Utc::now().to_rfc3339());
            self.save_role_run(&attempt.project_id, &attempt.issue_id, &run)?;
            finalized += 1;
        }
        Ok(finalized)
    }

    /// S4：RecoverCoding 显式恢复时的清场——修复落地前死亡路径不收尾遗留
    /// 的残留 Running role run 统一标 Superseded（先例
    /// `ensure_failed_code_review_retry_role_run` 的 stale 收尾）。恢复瞬间
    /// 旧 runner 已死、新 run 由重启后的 runner 另行创建（各阶段重入处仅
    /// 复用 `Running && node_id.is_none()` 的 run），此刻仍 Running 的只能是
    /// 残留脏数据；恢复 journal 持有的 retry run 同样豁免。返回收尾条数
    /// （0 表示本无残留）。
    pub fn supersede_residual_running_role_runs_for_attempt(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<usize, ProductStoreError> {
        let (exempt_run_id, exempt_reason) = self.recovery_journal_retry_exemption(attempt)?;
        let mut superseded = 0usize;
        for mut run in self.list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)? {
            if run.status != CodingRoleRunStatus::Running {
                continue;
            }
            if is_journal_held_retry_run(&run, exempt_run_id.as_deref(), exempt_reason.as_deref()) {
                continue;
            }
            run.status = CodingRoleRunStatus::Superseded;
            run.completed_at = Some(Utc::now().to_rfc3339());
            self.save_role_run(&attempt.project_id, &attempt.issue_id, &run)?;
            superseded += 1;
        }
        Ok(superseded)
    }

    /// S4 豁免面：failed-code-review 恢复 journal 持有的 retry role run
    /// 必须保持 Running+reason=recovery_key，才能被恢复链识别复用；死亡
    /// 收尾/恢复清场打穿它会让 retry_review 通道永久关死（gate 悬挂无
    /// 动作）。覆盖两个窗口：
    ///
    /// 1. journal 未 completed：`ensure_failed_code_review_retry_role_run`
    ///    的 reason 匹配与 `validate_retry_role_run` 的复用凭据。识别与
    ///    store 自身恢复/回滚逻辑同款：`journal.retry_role_run_id` 或
    ///    `reason_code == recovery_key` 二者其一（后者覆盖 retry run 已建
    ///    而 journal 尚未登记 id 的窗口）。
    /// 2. journal 已 completed 但 retry 节点尚未 spawn（reserved spawn 在
    ///    provider 进入前完成 journal 后、retry run attach node 前的死亡
    ///    窗口）：`completed_journal_waits_for_retry_node` 要求 retry run
    ///    仍 Running+node_id.is_none()，打穿则该判定永久 false。已 spawn
    ///    （node_id 有值）的 retry run 按普通 run 收尾，不豁免。
    ///
    /// 裁定取舍：接受豁免 run 在 AMR 态投影为 running 的已知单例投影瑕疵
    /// （<恢复通道永久死）；若要消此瑕疵，走「恢复流程重开 journal run」
    /// 另立切片。
    fn recovery_journal_retry_exemption(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(Option<String>, Option<String>), ProductStoreError> {
        let Some(journal) = self.get_failed_code_review_recovery_journal(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?
        else {
            return Ok((None, None));
        };
        if !journal.is_completed() {
            return Ok((journal.retry_role_run_id, Some(journal.recovery_key)));
        }
        let Some(retry_role_run_id) = journal.retry_role_run_id.as_deref() else {
            return Ok((None, None));
        };
        let retry_awaits_spawn = matches!(
            self.get_role_run(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                retry_role_run_id,
            ),
            Ok(run) if run.node_id.is_none()
        );
        if retry_awaits_spawn {
            Ok((
                Some(retry_role_run_id.to_string()),
                Some(journal.recovery_key),
            ))
        } else {
            Ok((None, None))
        }
    }

    pub fn update_role_run_refs(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_run_id: &str,
        raw_provider_output_refs: Vec<String>,
        artifact_refs: Vec<String>,
    ) -> Result<CodingRoleRun, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(attempt_id)?;
        validate_relative_id(role_run_id)?;
        let mut run = self.get_role_run(project_id, issue_id, attempt_id, role_run_id)?;
        for reference in raw_provider_output_refs {
            validate_relative_artifact_ref(&reference)?;
            if !run
                .raw_provider_output_refs
                .iter()
                .any(|existing| existing == &reference)
            {
                run.raw_provider_output_refs.push(reference);
            }
        }
        for reference in artifact_refs {
            validate_relative_artifact_ref(&reference)?;
            if !run
                .artifact_refs
                .iter()
                .any(|existing| existing == &reference)
            {
                run.artifact_refs.push(reference);
            }
        }
        self.save_role_run(project_id, issue_id, &run)?;
        Ok(run)
    }

    pub(crate) fn save_role_run(
        &self,
        project_id: &str,
        issue_id: &str,
        run: &CodingRoleRun,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&run.id)?;
        write_json(&self.role_run_path(project_id, issue_id, run), run)
    }

    pub(crate) fn get_role_run(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        role_run_id: &str,
    ) -> Result<CodingRoleRun, ProductStoreError> {
        validate_relative_id(role_run_id)?;
        read_json(
            &self
                .role_runs_root(project_id, issue_id, attempt_id)
                .join(format!("{role_run_id}.json")),
        )
    }
}

fn validate_retry_metadata(retry: &CodingRoleRunRetryMetadata) -> Result<(), ProductStoreError> {
    validate_relative_id(&retry.cycle_id)?;
    let Some(prior_run_id) = retry.prior_run_id.as_deref() else {
        return Err(invalid_retry_metadata("missing_prior_run_id"));
    };
    validate_relative_id(prior_run_id)?;
    if !(2..=3).contains(&retry.attempt_no) {
        return Err(invalid_retry_metadata(retry.attempt_no.to_string()));
    }
    Ok(())
}

/// S4 豁免判定：run 是否为未 completed 恢复 journal 持有的 retry run——
/// id 已登记（`journal.retry_role_run_id`）或 reason 等于 journal 的
/// recovery_key（覆盖 run 已建而 id 尚未登记的窗口）二者其一。
fn is_journal_held_retry_run(
    run: &CodingRoleRun,
    exempt_run_id: Option<&str>,
    exempt_reason: Option<&str>,
) -> bool {
    exempt_run_id == Some(run.id.as_str())
        || exempt_reason.is_some_and(|key| run.reason_code.as_deref() == Some(key))
}

fn invalid_retry_metadata(id: impl Into<String>) -> ProductStoreError {
    ProductStoreError::Conflict {
        kind: "coding_role_run_retry_metadata",
        id: id.into(),
    }
}
