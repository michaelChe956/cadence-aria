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
        // S2b（N5 步暂停移除）：完成链不再识别/补写 plan-run 暂停身份——
        // legacy 暂停形状（Blocked+清指针）重入由 preflight 失败关闭，恢复
        // 走入口面 resume 语义；新数据完成即自动推进。
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
        let handoff_id = handoff.id.clone();
        let transition = self.authoritative_handoff_transition(
            &attempt,
            facts.previous_handoff.clone(),
            handoff,
        )?;
        self.apply_authoritative_handoff_transition(&attempt, transition)
            .await?;
        self.emit_group_unit_delivery_facts(&attempt, &facts, &completed_run, &handoff_id)
            .await;
        // S2b（N5 步暂停移除）：完成事实（completion commit/权威 run/handoff）
        // 落定后显式落定当前 unit Completed（原 complete_current_group_unit
        // 的前半段），随即自动推进——不写默认步暂停；推进保持独立 pub 入口
        // （advance_to_next_group_unit，per-unit/per-plan 粒度，无跨 plan
        // 串行假设），供自动化编排器（入口面 drive 桥）复用同一推进语义。
        if facts.active.status != CodingExecutionUnitStatus::Completed {
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
        }
        // P2（审查修复）：advance 一律传**更新前**的内存 attempt——
        // `update_coding_unit_status` 会同步清空 durable active/current 指针，
        // 重读会使 advance 的锁转移 from 身份回退到创建期 work_item_id，
        // 非首 unit 完成时 from≠锁 owner 而 `issue_worktree_active` 失败。
        // 旧路径（complete_current_group_unit→advance）同样传调用方内存对象。
        self.advance_to_next_group_unit(&attempt).await
    }

    /// S7 平台确定性交付摘要(Q4③):unit 完成后用平台事实生成「交付事实(平台生成)」
    /// chat 尾帧——completion_commit、`start..completion` 变更文件、验证证据、handoff id。
    /// 帧内容只来自平台记录,不依赖 agent 汇报 prose;帧 id 取自 unit run,
    /// CompletedRetry 重入时按同一 id 覆写,不产生重复帧。写入失败不阻断完成主链路
    /// (与 `emit_code_review_chat_entry` 同语义)。
    async fn emit_group_unit_delivery_facts(
        &self,
        attempt: &CodingExecutionAttempt,
        facts: &GroupUnitCompletionFacts,
        completed_run: &CodingUnitRun,
        handoff_id: &str,
    ) {
        let Some(completion_commit) = completed_run.completion_commit.as_deref() else {
            return;
        };
        let mut changed_files: Vec<String> = Vec::new();
        let mut changed_files_note: Option<String> = None;
        match (
            attempt.worktree_path.as_deref(),
            completed_run.start_commit.as_deref(),
        ) {
            (Some(worktree_path), Some(start_commit)) => {
                match self
                    ._git_service
                    .git_commit_range_changed_files(worktree_path, start_commit, completion_commit)
                    .await
                {
                    Ok(files) => changed_files = files,
                    Err(error) => changed_files_note = Some(error.to_string()),
                }
            }
            (None, _) => changed_files_note = Some("worktree_missing".to_string()),
            (Some(_), None) => changed_files_note = Some("start_commit_missing".to_string()),
        }
        let report = self
            .store
            .list_code_review_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .ok()
            .and_then(|reports| {
                reports
                    .into_iter()
                    .filter(|report| {
                        report.unit_run_id.as_deref() == Some(completed_run.id.as_str())
                    })
                    .max_by(|left, right| {
                        left.created_at
                            .cmp(&right.created_at)
                            .then_with(|| left.round.cmp(&right.round))
                            .then_with(|| left.id.cmp(&right.id))
                    })
            });
        let verification = match &report {
            Some(report) => json!({
                "review_report_id": report.id,
                "verdict": report.verdict,
                "tested_evidence_refs": report.tested_evidence_refs,
            }),
            None => json!({
                "review_report_id": serde_json::Value::Null,
                "verdict": serde_json::Value::Null,
                "tested_evidence_refs": Vec::<String>::new(),
            }),
        };
        let verdict_label = match report.as_ref().map(|report| &report.verdict) {
            Some(ReviewVerdict::Approve) => "approve",
            Some(ReviewVerdict::RequestChanges) => "request_changes",
            Some(ReviewVerdict::Blocked) => "blocked",
            None => "无记录",
        };
        let content = format!(
            "Work Item {} 交付完成(平台生成)\n提交: {}\n变更文件({}): {}\n验证证据: {}\nhandoff: {}",
            facts.active.logical_work_item_id,
            completion_commit,
            changed_files.len(),
            if changed_files.is_empty() {
                "无".to_string()
            } else {
                changed_files.join(", ")
            },
            match &report {
                Some(report) => format!(
                    "code review {} · {} 条证据({})",
                    verdict_label,
                    report.tested_evidence_refs.len(),
                    report.id
                ),
                None => "无 code review 记录".to_string(),
            },
            handoff_id,
        );
        let mut metadata = json!({
            "source": "platform_delivery_facts",
            "unit_id": facts.active.id,
            "unit_run_id": completed_run.id,
            "logical_work_item_id": facts.active.logical_work_item_id,
            "start_commit": completed_run.start_commit,
            "completion_commit": completion_commit,
            "changed_files": changed_files,
            "verification": verification,
            "handoff_revision_id": handoff_id,
        });
        if let Some(note) = changed_files_note {
            metadata["changed_files_unavailable_reason"] = json!(note);
        }
        let entry = CodingChatEntry {
            id: format!("{}_delivery_facts", completed_run.id),
            attempt_id: attempt.id.clone(),
            node_id: None,
            role: CodingAgentRole::System,
            entry_type: CodingEntryType::SystemEvent {
                event_type: "delivery_facts".to_string(),
                message: format!(
                    "Work Item {} 交付事实(平台生成)",
                    facts.active.logical_work_item_id
                ),
            },
            content: Some(content),
            metadata: Some(metadata),
            created_at: Utc::now().to_rfc3339(),
        };
        self.save_and_emit_chat_entry(attempt, entry).await;
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
    if provided_contracts.is_empty() {
        // P3-c（REQ-GCE-07）：refs 为空时从冻结的 output_contracts 登记——
        // 平台在完成发布时登记契约，不依赖 coder 自报；无 output 的非接口
        // WI 仍允许为空。
        provided_contracts = revision
            .canonical_contract
            .output_contracts
            .iter()
            .map(|output| output.contract_id.clone())
            .collect();
    }
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

#[cfg(test)]
mod handoff_contract_registration_tests {
    use super::*;
    use crate::product::work_item_contract::{
        CanonicalWorkItemContract, HandoffContract, PromisedOutputContract,
        WorkItemContractIdentity, WorkItemGoal, WorkItemWritePolicy,
    };
    use std::collections::BTreeMap;

    fn revision_with(
        provided_contract_refs: Vec<&str>,
        output_contracts: Vec<(&str, Vec<&str>)>,
    ) -> WorkItemRevision {
        WorkItemRevision {
            id: "work_item_revision_0001".to_string(),
            logical_work_item_id: "work_item_0001".to_string(),
            source_draft_revision_id: "draft_revision_0001".to_string(),
            canonical_contract: CanonicalWorkItemContract {
                intent_contract: None,
                schema_version: 1,
                identity: WorkItemContractIdentity {
                    logical_work_item_id: "work_item_0001".to_string(),
                    title: "upstream work item".to_string(),
                    kind: "implementation".to_string(),
                },
                goal: WorkItemGoal {
                    summary: "upstream work item".to_string(),
                },
                non_goals: Vec::new(),
                depends_on: Vec::new(),
                input_contracts: Vec::new(),
                output_contracts: output_contracts
                    .into_iter()
                    .map(|(contract_id, capabilities)| PromisedOutputContract {
                        contract_id: contract_id.to_string(),
                        capabilities: capabilities.into_iter().map(str::to_string).collect(),
                    })
                    .collect(),
                tasks: Vec::new(),
                write_policy: WorkItemWritePolicy {
                    exclusive_scopes: Vec::new(),
                    forbidden_scopes: Vec::new(),
                },
                acceptance_criteria: Vec::new(),
                verification_checks: Vec::new(),
                handoff_contract: HandoffContract {
                    required_fields: Vec::new(),
                    provided_contract_refs: provided_contract_refs
                        .into_iter()
                        .map(str::to_string)
                        .collect(),
                    reviewer_check_refs: Vec::new(),
                },
                blocker_rules: Vec::new(),
                design_traceability: Vec::new(),
            },
            canonical_contract_hash: "canonical_hash".to_string(),
            work_item_projection_bundle_id: "projection_bundle_0001".to_string(),
            verification_plan_revision_id: "verification_revision_0001".to_string(),
            created_at: "2026-10-15T00:00:00Z".to_string(),
        }
    }

    /// P3-c（REQ-GCE-07）①：refs 空 + output 声明契约 → 完成 handoff 从冻结
    /// output_contracts 登记 refs/能力/hash 齐（接既有 hash/build/publish 链）；
    /// 重复 contract_id 合并去重，不依赖 coder 自报。
    #[test]
    fn registers_contract_refs_from_frozen_outputs_when_refs_empty() {
        let revision = revision_with(
            Vec::new(),
            vec![
                ("CT-002", vec!["cap_shared", "cap_only_b"]),
                ("CT-001", vec!["cap_shared", "cap_only_a"]),
                ("CT-001", vec!["cap_only_a", "cap_dup"]),
            ],
        );
        let facts = group_handoff_contract_facts(&revision).expect("contract facts");
        assert_eq!(facts.provided_contracts, vec!["CT-001", "CT-002"]);
        let expected_capabilities = BTreeMap::from([
            (
                "CT-001".to_string(),
                vec!["cap_dup".to_string(), "cap_only_a".to_string(), "cap_shared".to_string()],
            ),
            (
                "CT-002".to_string(),
                vec!["cap_only_b".to_string(), "cap_shared".to_string()],
            ),
        ]);
        assert_eq!(facts.provided_capabilities, expected_capabilities);
        assert_eq!(
            facts.contract_hash,
            stable_handoff_contract_hash(
                &["CT-001".to_string(), "CT-002".to_string()],
                &expected_capabilities
            )
            .expect("expected hash")
        );
    }

    /// P3-c（REQ-GCE-07）②：refs 非空保持原选择——不因 output 声明扩集。
    #[test]
    fn keeps_explicit_refs_selection_when_non_empty() {
        let revision = revision_with(
            vec!["CT-001"],
            vec![("CT-001", vec!["cap_a"]), ("CT-002", vec!["cap_b"])],
        );
        let facts = group_handoff_contract_facts(&revision).expect("contract facts");
        assert_eq!(facts.provided_contracts, vec!["CT-001"]);
        assert_eq!(
            facts.provided_capabilities,
            BTreeMap::from([("CT-001".to_string(), vec!["cap_a".to_string()])])
        );
    }

    /// P3-c（REQ-GCE-07）③：无 output 的非接口 WI 允许为空（空 hash 仍稳定）。
    #[test]
    fn allows_empty_registration_without_outputs() {
        let revision = revision_with(Vec::new(), Vec::new());
        let facts = group_handoff_contract_facts(&revision).expect("contract facts");
        assert!(facts.provided_contracts.is_empty());
        assert!(facts.provided_capabilities.is_empty());
        assert_eq!(
            facts.contract_hash,
            stable_handoff_contract_hash(&[], &BTreeMap::new()).expect("empty hash")
        );
    }
}
