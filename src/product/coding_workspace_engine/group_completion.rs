use super::*;
use crate::product::coding_models::{
    CodingAttemptScope, CodingExecutionUnit, CodingExecutionUnitStatus, CodingUnitRun,
    CodingUnitRunStatus,
};
use crate::product::models::{
    HandoffRevision, WorkItemPlanLineage, WorkItemProjectionBundle, WorkItemRevision,
};
use crate::product::work_item_projection::{renderer_for, renderer_version_for_optional_provider};
use crate::product::work_item_revision_store::WorkItemRevisionStore;

use super::runtime_impact::stable_handoff_contract_hash;

/// C2 Task 6（决策 6/#13）：execution 完成时起点缺失的稳定 reason 码。
pub(crate) const UNIT_RUN_START_COMMIT_MISSING: &str = "unit_run_start_commit_missing";

#[derive(Debug, Clone)]
enum GroupUnitCompletionMode {
    Running,
    CompletedRetry { completion_commit: String },
}

#[derive(Debug, Clone)]
struct GroupUnitCompletionFacts {
    active: CodingExecutionUnit,
    run: CodingUnitRun,
    lineage: WorkItemPlanLineage,
    revision: WorkItemRevision,
    bundle: WorkItemProjectionBundle,
    handoff_id: String,
    provided_contracts: Vec<String>,
    provided_capabilities: std::collections::BTreeMap<String, Vec<String>>,
    handoff_contract_hash: String,
    previous_handoff: Option<HandoffRevision>,
    mode: GroupUnitCompletionMode,
}

struct GroupHandoffContractFacts {
    provided_contracts: Vec<String>,
    provided_capabilities: std::collections::BTreeMap<String, Vec<String>>,
    contract_hash: String,
}

impl CodingWorkspaceEngine {
    pub async fn complete_group_unit_after_code_review(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let attempt =
            self.store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        self.validate_attempt_issue_shared_worktree_lock_if_present(&attempt)?;
        // N5（REQ-CS-05）：CompletedRetry 入口先核冻结完成 pause 身份——
        // pause 写回（清指针+Blocked）后 preflight 会按非 Running 误拒；
        // 此处以幂等 pause 原语对齐（含 run Paused 补记中断窗口）并保持
        // 暂停形状返回，不重进完成链。
        if let Some(paused) = self.reconcile_paused_plan_first_unit(&attempt)? {
            return Ok(paused);
        }
        let facts = self.preflight_group_unit_completion(&attempt)?;
        #[cfg(test)]
        crate::product::coding_workspace_engine::mutation_test_pause::pause_coding_mutation_for_test(
            self.store.paths().root(),
            match facts.mode {
                GroupUnitCompletionMode::Running => crate::product::coding_workspace_engine::mutation_test_pause::CodingMutationTestPoint::GroupCompletionRunning,
                GroupUnitCompletionMode::CompletedRetry { .. } => crate::product::coding_workspace_engine::mutation_test_pause::CodingMutationTestPoint::GroupCompletionCompletedRetry,
            },
        )
        .await;
        let attempt =
            self.store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        self.validate_attempt_issue_shared_worktree_lock_if_present(&attempt)?;
        validate_group_completion_attempt_state(&attempt)?;
        // C2 Task 6（决策 6/#13）：完成时起点缺失 MUST NOT 按 base_branch
        // HEAD 回填——在任何完成写入前持久化诊断并停等（人工恢复后原链
        // 继续补齐事实）。历史 UnitRun 已有 start_commit 不重算不改写。
        if facts.run.start_commit.is_none() {
            self.store
                .transition_to_awaiting_manual_recovery(&attempt.id, UNIT_RUN_START_COMMIT_MISSING)
                .map_err(|error| CodingWorkspaceEngineError::ProviderProtocol(error.to_string()))?;
            self.store.append_manual_recovery_diagnostic(
                &attempt,
                UNIT_RUN_START_COMMIT_MISSING,
                &format!(
                    "unit run {} 完成时缺少认领前冻结的 start_commit，拒绝按 base HEAD 回填",
                    facts.run.id
                ),
            )?;
            return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                "{UNIT_RUN_START_COMMIT_MISSING}: {}",
                facts.run.id
            )));
        }
        let attempt = match &facts.mode {
            GroupUnitCompletionMode::Running => {
                self.record_current_group_unit_completion_head(&attempt, &facts.active)
                    .await?
            }
            GroupUnitCompletionMode::CompletedRetry { completion_commit } => {
                let worktree_path = attempt.worktree_path.as_ref().ok_or_else(|| {
                    CodingWorkspaceEngineError::MissingWorktree(attempt.id.clone())
                })?;
                self.ensure_worktree_clean_with_manual_gate(
                    &attempt,
                    worktree_path,
                    CodingExecutionStage::ReviewRequest,
                )
                .await?;
                self.recover_completed_group_unit_commit(&attempt, &facts.active, completion_commit)
                    .await?
            }
        };
        let completion_commit = attempt.head_commit.as_deref().ok_or_else(|| {
            CodingWorkspaceEngineError::CompletionCommitMissing(attempt.id.clone())
        })?;
        let completed_run =
            self.store
                .complete_coding_unit_run(&attempt, &facts.run.id, completion_commit)?;
        let handoff = self.publish_group_unit_handoff_revision(&attempt, &facts, &completed_run)?;
        let transition = self.authoritative_handoff_transition(
            &attempt,
            facts.previous_handoff.clone(),
            handoff,
        )?;
        self.apply_authoritative_handoff_transition(&attempt, transition)
            .await?;
        // N5（REQ-CS-05）：完成事实（completion commit/权威 run/handoff）落定后、
        // advance_to_next_group_unit 及其 SC selector 之前先尝试 plan-run
        // first_ready 安全暂停。Running 分支在此显式落定 selected unit
        // Completed（原 complete_current_group_unit 的前半段），暂停命中即
        // 短路返回：不进入下一 unit 选择/推进，也不转 ReviewRequest/push。
        let attempt = if facts.active.status == CodingExecutionUnitStatus::Completed {
            attempt
        } else {
            self.store
                .update_coding_unit_status(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    &facts.active.id,
                    CodingExecutionUnitStatus::Completed,
                    Some("当前 Work Item 已完成".to_string()),
                )
                .map(|_| ())?;
            self.store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?
        };
        if let Some(paused) = self
            .store
            .pause_completed_plan_first_unit(&attempt, &facts.active.id)?
        {
            return Ok(paused);
        }
        self.advance_to_next_group_unit(&attempt).await
    }

    /// N5（REQ-CS-05）：CompletedRetry/恢复入口的暂停身份识别。仅当 attempt
    /// 已处于 N2 安全暂停形状（Blocked+CodeReview+清指针）且 matching
    /// first_ready run 未 Invalidated 时，用幂等 pause 原语对齐并返回——
    /// 否则清指针后的 preflight 会按非 Running 误拒，且 run Paused 补记
    /// 中断窗口无法恢复。Running 完成链的暂停写在尾部完成事实之后。
    fn reconcile_paused_plan_first_unit(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<Option<CodingExecutionAttempt>, CodingWorkspaceEngineError> {
        if !plan_coding_pause_shape(attempt) {
            return Ok(None);
        }
        let run_store =
            crate::product::coding_plan_run::CodingPlanRunStore::new(self.store.paths());
        let Some(run) = run_store.matching_first_ready_run(attempt)? else {
            return Ok(None);
        };
        if matches!(
            run.phase,
            crate::product::coding_plan_run::CodingPlanRunPhase::Invalidated
        ) {
            return Ok(None);
        }
        let Some(selection) = run.selection.clone() else {
            return Ok(None);
        };
        Ok(self
            .store
            .pause_completed_plan_first_unit(attempt, &selection.unit_id)?)
    }

    /// N5（REQ-CS-05）：runner 的第二道停止保险——识别「本 plan first_ready
    /// run 的真实安全暂停」：attempt 处于暂停形状且 matching run phase=Paused
    /// （而非任意 Blocked 文本）；无 run/未暂停/Invalidated 均为 false。
    pub fn plan_first_unit_safely_paused(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<bool, CodingWorkspaceEngineError> {
        if !plan_coding_pause_shape(attempt) {
            return Ok(false);
        }
        let run_store =
            crate::product::coding_plan_run::CodingPlanRunStore::new(self.store.paths());
        let Some(run) = run_store.matching_first_ready_run(attempt)? else {
            return Ok(false);
        };
        Ok(matches!(
            run.phase,
            crate::product::coding_plan_run::CodingPlanRunPhase::Paused
        ))
    }
    fn publish_group_unit_handoff_revision(
        &self,
        attempt: &CodingExecutionAttempt,
        facts: &GroupUnitCompletionFacts,
        completed_run: &CodingUnitRun,
    ) -> Result<HandoffRevision, CodingWorkspaceEngineError> {
        if completed_run.id != facts.run.id
            || completed_run.unit_id != facts.active.id
            || completed_run.work_item_revision_id != facts.revision.id
            || completed_run.projection_bundle_id != facts.bundle.id
            || completed_run.status != CodingUnitRunStatus::Completed
        {
            return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                "group_completion_completed_run_mismatch: {}",
                completed_run.id
            )));
        }
        let completion_commit = completed_run.completion_commit.as_deref().ok_or_else(|| {
            CodingWorkspaceEngineError::CompletionCommitMissing(completed_run.id.clone())
        })?;
        let handoff_id = facts.handoff_id.clone();
        let build_handoff = |created_at: String| {
            build_group_handoff_revision(facts, completed_run, completion_commit, created_at)
        };
        let revision_store = WorkItemRevisionStore::new(self.store.paths());
        let handoff = match revision_store.get_handoff_revision(
            &facts.lineage,
            &facts.active.logical_work_item_id,
            &handoff_id,
        ) {
            Ok(existing) => {
                if existing != build_handoff(existing.created_at.clone()) {
                    return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                        "group_completion_handoff_revision_mismatch: {handoff_id}"
                    )));
                }
                existing
            }
            Err(ProductStoreError::NotFound {
                kind: "handoff_revision",
                ..
            }) => {
                let handoff = build_handoff(Utc::now().to_rfc3339());
                revision_store.put_handoff_revision(&facts.lineage, &handoff)?;
                handoff
            }
            Err(error) => return Err(error.into()),
        };
        if facts.active.latest_handoff_revision_id.as_deref() != Some(handoff.id.as_str()) {
            self.store.update_coding_unit_latest_handoff_revision_id(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &facts.active.id,
                Some(handoff.id.clone()),
            )?;
        }
        Ok(handoff)
    }

    fn preflight_group_unit_completion(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<GroupUnitCompletionFacts, CodingWorkspaceEngineError> {
        if attempt.scope != CodingAttemptScope::WorkItemGroup {
            return Err(CodingWorkspaceEngineError::FinalConfirmNotReady(
                attempt.id.clone(),
            ));
        }
        validate_group_completion_attempt_state(attempt)?;
        let (active, recovering_cleared_active) = match self.store.get_active_coding_unit(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )? {
            Some(active) => (active, false),
            None => (self.recoverable_completed_group_unit(attempt)?, true),
        };
        let runs = self.store.list_coding_unit_runs(attempt, &active.id)?;
        let run = runs
            .iter()
            .max_by_key(|run| run.execution_no)
            .cloned()
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "coding_unit_run",
                id: active.id.clone(),
            })?;
        let active_run_count = runs.iter().filter(|run| run.status.is_active()).count();
        let mode = match run.status {
            CodingUnitRunStatus::Running | CodingUnitRunStatus::NeedsRevalidation
                if active_run_count == 1 =>
            {
                GroupUnitCompletionMode::Running
            }
            CodingUnitRunStatus::Completed if active_run_count == 0 => {
                let completion_commit = run.completion_commit.clone().ok_or_else(|| {
                    CodingWorkspaceEngineError::CompletionCommitMissing(run.id.clone())
                })?;
                GroupUnitCompletionMode::CompletedRetry { completion_commit }
            }
            _ if active_run_count > 1 => {
                return Err(ProductStoreError::Ambiguous {
                    kind: "coding_unit_run",
                    id: active.id,
                }
                .into());
            }
            _ => {
                return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                    "group_completion_unit_run_not_authoritative: {}",
                    run.id
                )));
            }
        };
        if recovering_cleared_active {
            let GroupUnitCompletionMode::CompletedRetry { completion_commit } = &mode else {
                return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                    "group_completion_cleared_active_not_completed: {}",
                    active.id
                )));
            };
            if active.status != CodingExecutionUnitStatus::Completed
                || active.completion_commit.as_deref() != Some(completion_commit.as_str())
            {
                return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                    "group_completion_cleared_active_commit_mismatch: {}",
                    active.id
                )));
            }
        }
        let plan_id = attempt.work_item_group_id.as_deref().ok_or_else(|| {
            CodingWorkspaceEngineError::ProviderStream(
                "group_completion_plan_binding_missing".to_string(),
            )
        })?;
        let revision_store = WorkItemRevisionStore::new(self.store.paths());
        let lineage =
            revision_store.get_plan_lineage(&attempt.project_id, &attempt.issue_id, plan_id)?;
        let revision = revision_store.get_work_item_revision(
            &lineage,
            &active.logical_work_item_id,
            &run.work_item_revision_id,
        )?;
        let bundle =
            revision_store.get_work_item_projection_bundle(&lineage, &run.projection_bundle_id)?;
        validate_projection_bundle(&run.work_item_revision_id, &revision, &bundle)?;
        let resolved_handoff_revision_ids =
            self.authoritative_resolved_handoff_revision_ids(attempt, &active, &lineage)?;
        let providers = self.store.get_role_provider_config_snapshot(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        )?;
        if run.resolved_handoff_revision_ids != resolved_handoff_revision_ids {
            return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                "group_completion_handoff_binding_mismatch: {}",
                run.id
            )));
        }
        if run.unit_id != active.id
            || run.work_item_revision_id != active.work_item_revision_id
            || run.work_item_revision_id != revision.id
            || run.canonical_contract_hash != revision.canonical_contract_hash
            || run.projection_bundle_id != bundle.id
            || run.projection_compiler_version != bundle.compiler_version
            || run.coder_projection_hash != bundle.coder_projection_hash
            || run.reviewer_projection_hash != bundle.reviewer_projection_hash
            || run.coder_provider_renderer_version
                != renderer_for(&providers.coder).renderer_version()
            || run.reviewer_provider_renderer_version
                != renderer_version_for_optional_provider(providers.code_reviewer.as_ref())
        {
            return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                "group_completion_unit_run_binding_mismatch: {}",
                run.id
            )));
        }
        let handoff_contract = group_handoff_contract_facts(&revision)?;
        let mut facts = GroupUnitCompletionFacts {
            handoff_id: format!("handoff_revision_{}", run.id),
            active,
            run,
            lineage,
            revision,
            bundle,
            provided_contracts: handoff_contract.provided_contracts,
            provided_capabilities: handoff_contract.provided_capabilities,
            handoff_contract_hash: handoff_contract.contract_hash,
            previous_handoff: None,
            mode,
        };
        facts.previous_handoff =
            self.preflight_existing_group_handoff(attempt, &facts, recovering_cleared_active)?;
        Ok(facts)
    }

    fn recoverable_completed_group_unit(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<CodingExecutionUnit, CodingWorkspaceEngineError> {
        let units =
            self.store
                .list_coding_units(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let pending_boundary = units
            .iter()
            .position(|unit| unit.status == CodingExecutionUnitStatus::Pending)
            .unwrap_or(units.len());
        if pending_boundary == 0
            || units[..pending_boundary]
                .iter()
                .any(|unit| unit.status != CodingExecutionUnitStatus::Completed)
            || units[pending_boundary..]
                .iter()
                .any(|unit| unit.status != CodingExecutionUnitStatus::Pending)
        {
            return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                "group_completion_unit_sequence_invalid: {}",
                attempt.id
            )));
        }
        Ok(units[pending_boundary - 1].clone())
    }

    fn preflight_existing_group_handoff(
        &self,
        attempt: &CodingExecutionAttempt,
        facts: &GroupUnitCompletionFacts,
        require_existing: bool,
    ) -> Result<Option<HandoffRevision>, CodingWorkspaceEngineError> {
        if require_existing
            && facts.active.latest_handoff_revision_id.as_deref() != Some(facts.handoff_id.as_str())
        {
            return Err(group_handoff_revision_conflict(&facts.handoff_id));
        }
        let revision_store = WorkItemRevisionStore::new(self.store.paths());
        let pointer_is_current =
            facts.active.latest_handoff_revision_id.as_deref() == Some(facts.handoff_id.as_str());
        let mut captured_previous = None;
        if let Some(previous_id) = facts
            .active
            .latest_handoff_revision_id
            .as_deref()
            .filter(|pointer| *pointer != facts.handoff_id)
        {
            let previous = match revision_store.get_handoff_revision(
                &facts.lineage,
                &facts.active.logical_work_item_id,
                previous_id,
            ) {
                Ok(previous) => previous,
                Err(ProductStoreError::NotFound {
                    kind: "handoff_revision",
                    ..
                }) => return Err(group_handoff_revision_conflict(&facts.handoff_id)),
                Err(error) => return Err(error.into()),
            };
            if previous.coding_unit_run_id == facts.run.id {
                return Err(group_handoff_revision_conflict(&facts.handoff_id));
            }
            let previous_run = self
                .authoritative_handoff_run(attempt, &previous)
                .map_err(|_| group_handoff_revision_conflict(&facts.handoff_id))?;
            if previous_run.execution_no >= facts.run.execution_no {
                return Err(group_handoff_revision_conflict(&facts.handoff_id));
            }
            captured_previous = Some(previous);
        }
        let existing = match revision_store.get_handoff_revision(
            &facts.lineage,
            &facts.active.logical_work_item_id,
            &facts.handoff_id,
        ) {
            Ok(existing) => Some(existing),
            Err(ProductStoreError::NotFound {
                kind: "handoff_revision",
                ..
            }) => None,
            Err(error) => return Err(error.into()),
        };
        let Some(existing) = existing else {
            if require_existing || pointer_is_current {
                return Err(group_handoff_revision_conflict(&facts.handoff_id));
            }
            return Ok(captured_previous);
        };
        let GroupUnitCompletionMode::CompletedRetry { completion_commit } = &facts.mode else {
            return Err(group_handoff_revision_conflict(&facts.handoff_id));
        };
        let expected = build_group_handoff_revision(
            facts,
            &facts.run,
            completion_commit,
            existing.created_at.clone(),
        );
        if existing != expected {
            return Err(group_handoff_revision_conflict(&facts.handoff_id));
        }
        let next_run = self
            .authoritative_handoff_run(attempt, &existing)
            .map_err(|_| group_handoff_revision_conflict(&facts.handoff_id))?;
        if pointer_is_current {
            return self
                .authoritative_previous_handoff_for_run(
                    attempt,
                    &facts.lineage,
                    &existing,
                    &next_run,
                )
                .map_err(|_| group_handoff_revision_conflict(&facts.handoff_id));
        }
        Ok(captured_previous)
    }

    async fn record_current_group_unit_completion_head(
        &self,
        attempt: &CodingExecutionAttempt,
        active: &CodingExecutionUnit,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let Some(worktree_path) = attempt.worktree_path.as_ref() else {
            return Err(CodingWorkspaceEngineError::MissingWorktree(
                attempt.id.clone(),
            ));
        };
        let completion_commit = self._git_service.git_current_head(worktree_path).await?;
        self.persist_group_unit_completion_commit(attempt, active, &completion_commit)
    }

    async fn recover_completed_group_unit_commit(
        &self,
        attempt: &CodingExecutionAttempt,
        active: &CodingExecutionUnit,
        completion_commit: &str,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let worktree_path = attempt
            .worktree_path
            .as_ref()
            .ok_or_else(|| CodingWorkspaceEngineError::MissingWorktree(attempt.id.clone()))?;
        let current_head = self._git_service.git_current_head(worktree_path).await?;
        if current_head != completion_commit {
            return Err(CodingWorkspaceEngineError::ProviderStream(format!(
                "group_completion_commit_mismatch: {}",
                active.id
            )));
        }
        self.persist_group_unit_completion_commit(attempt, active, completion_commit)
    }

    fn persist_group_unit_completion_commit(
        &self,
        attempt: &CodingExecutionAttempt,
        active: &CodingExecutionUnit,
        completion_commit: &str,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let updated = self.store.update_attempt_head_commit(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            Some(completion_commit.to_string()),
        )?;
        self.store.update_coding_unit_completion_commit(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &active.id,
            Some(completion_commit.to_string()),
        )?;
        Ok(updated)
    }
}

/// N5（REQ-CS-05）：N2 安全暂停落定后的 attempt 持久化形状——Blocked+
/// CodeReview、active/current 指针清空。仅形状本身不构成暂停证据，还须
/// matching first_ready run 的相位核验（见 engine 侧两个识别入口）。
fn plan_coding_pause_shape(attempt: &CodingExecutionAttempt) -> bool {
    attempt.scope == CodingAttemptScope::WorkItemGroup
        && attempt.status == CodingAttemptStatus::Blocked
        && attempt.stage == CodingExecutionStage::CodeReview
        && attempt.active_unit_id.is_none()
        && attempt.current_work_item_id.is_none()
}
fn validate_group_completion_attempt_state(
    attempt: &CodingExecutionAttempt,
) -> Result<(), CodingWorkspaceEngineError> {
    if attempt.stage != CodingExecutionStage::ReviewRequest {
        return Err(CodingWorkspaceEngineError::ProviderStream(format!(
            "group_completion_stage_not_ready: {}",
            attempt.id
        )));
    }
    if attempt.status != CodingAttemptStatus::Running {
        return Err(CodingWorkspaceEngineError::ProviderStream(format!(
            "group_completion_status_not_ready: {}",
            attempt.id
        )));
    }
    Ok(())
}

fn group_handoff_contract_facts(
    revision: &WorkItemRevision,
) -> Result<GroupHandoffContractFacts, CodingWorkspaceEngineError> {
    let mut provided_contracts = revision
        .canonical_contract
        .handoff_contract
        .provided_contract_refs
        .clone();
    provided_contracts.sort();
    provided_contracts.dedup();
    let mut provided_capabilities = std::collections::BTreeMap::new();
    for output in &revision.canonical_contract.output_contracts {
        if !provided_contracts.contains(&output.contract_id) {
            continue;
        }
        provided_capabilities
            .entry(output.contract_id.clone())
            .or_insert_with(Vec::new)
            .extend(output.capabilities.iter().cloned());
    }
    for capabilities in provided_capabilities.values_mut() {
        capabilities.sort();
        capabilities.dedup();
    }
    let contract_hash = stable_handoff_contract_hash(&provided_contracts, &provided_capabilities)?;
    Ok(GroupHandoffContractFacts {
        provided_contracts,
        provided_capabilities,
        contract_hash,
    })
}

fn build_group_handoff_revision(
    facts: &GroupUnitCompletionFacts,
    completed_run: &CodingUnitRun,
    completion_commit: &str,
    created_at: String,
) -> HandoffRevision {
    HandoffRevision {
        id: facts.handoff_id.clone(),
        logical_work_item_id: facts.active.logical_work_item_id.clone(),
        work_item_revision_id: completed_run.work_item_revision_id.clone(),
        coding_unit_run_id: completed_run.id.clone(),
        provided_contracts: facts.provided_contracts.clone(),
        provided_capabilities: facts.provided_capabilities.clone(),
        contract_hash: facts.handoff_contract_hash.clone(),
        commit_sha: completion_commit.to_string(),
        created_at,
    }
}

fn group_handoff_revision_conflict(handoff_id: &str) -> CodingWorkspaceEngineError {
    CodingWorkspaceEngineError::ProviderStream(format!(
        "group_completion_handoff_revision_conflict: {handoff_id}"
    ))
}
