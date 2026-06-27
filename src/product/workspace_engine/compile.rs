use super::*;

pub(crate) fn next_compile_id() -> String {
    format!("compile_{}", chrono::Utc::now().format("%Y%m%d%H%M%S%3f"))
}

pub(crate) fn compile_work_item_id(compile_id: &str, index: usize) -> String {
    format!("work_item_{compile_id}_{:03}", index + 1)
}

pub(crate) fn compile_verification_plan_id(compile_id: &str, index: usize) -> String {
    format!("verification_plan_{compile_id}_{:03}", index + 1)
}

pub(crate) fn parse_compile_verification_scope(value: Option<&str>) -> VerificationScope {
    match value.unwrap_or_default() {
        "unit" => VerificationScope::Unit,
        "integration" => VerificationScope::Integration,
        "e2e" => VerificationScope::E2e,
        "build" => VerificationScope::Build,
        "lint" => VerificationScope::Lint,
        "manual" => VerificationScope::Manual,
        _ => VerificationScope::Custom,
    }
}

pub(crate) fn parse_compile_confidence(value: Option<&str>) -> RepositoryProfileConfidence {
    match value.unwrap_or("high") {
        "low" => RepositoryProfileConfidence::Low,
        "medium" => RepositoryProfileConfidence::Medium,
        _ => RepositoryProfileConfidence::High,
    }
}

pub(crate) fn parse_compile_fallback_policy(value: Option<&str>) -> VerificationFallbackPolicy {
    match value.unwrap_or("manual_gate") {
        "repair_provider_output" => VerificationFallbackPolicy::RepairProviderOutput,
        _ => VerificationFallbackPolicy::ManualGate,
    }
}

pub(crate) fn parse_compile_safety(value: Option<&str>) -> VerificationCommandSafety {
    match value.unwrap_or("approved") {
        "needs_manual_review" => VerificationCommandSafety::NeedsManualReview,
        _ => VerificationCommandSafety::Approved,
    }
}

pub(crate) fn json_string_array(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn parse_compile_verification_plan(
    value: &serde_json::Value,
    id: String,
    project_id: String,
    issue_id: String,
    work_item_id: String,
    now: String,
) -> VerificationPlan {
    let commands = value
        .get("commands")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .enumerate()
                .map(|(index, command)| VerificationCommand {
                    id: command
                        .get("id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("cmd_{:03}", index + 1)),
                    label: command
                        .get("label")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("验证命令")
                        .to_string(),
                    command: command
                        .get("command")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    cwd: command
                        .get("cwd")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    purpose: command
                        .get("purpose")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    required: command
                        .get("required")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                    timeout_seconds: command
                        .get("timeout_seconds")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(120),
                    source: VerificationCommandSource::Provider,
                    safety: parse_compile_safety(
                        command.get("safety").and_then(serde_json::Value::as_str),
                    ),
                })
                .collect()
        })
        .unwrap_or_default();
    let manual_checks = value
        .get("manual_checks")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .enumerate()
                .map(|(index, check)| VerificationManualCheck {
                    id: check
                        .get("id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("manual_{:03}", index + 1)),
                    label: check
                        .get("label")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("人工检查")
                        .to_string(),
                    instructions: check
                        .get("instructions")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    required: check
                        .get("required")
                        .and_then(serde_json::Value::as_bool)
                        .unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default();

    VerificationPlan {
        id,
        project_id,
        issue_id,
        work_item_id,
        repository_profile_ref: None,
        provider_run_ref: None,
        scope: parse_compile_verification_scope(
            value.get("scope").and_then(serde_json::Value::as_str),
        ),
        commands,
        manual_checks,
        required_gates: json_string_array(value.get("required_gates")),
        risk_notes: json_string_array(value.get("risk_notes")),
        confidence: parse_compile_confidence(
            value.get("confidence").and_then(serde_json::Value::as_str),
        ),
        fallback_policy: parse_compile_fallback_policy(
            value
                .get("fallback_policy")
                .and_then(serde_json::Value::as_str),
        ),
        created_at: now.clone(),
        updated_at: now,
    }
}

impl WorkspaceEngine {
    pub(crate) async fn enter_work_item_plan_compile(&mut self) {
        self.transition_stage(WorkspaceStage::Running).await;
        self.create_timeline_node(TimelineNodeDraft {
            node_type: TimelineNodeType::WorkItemPlanCompile,
            agent: None,
            stage: WorkspaceStage::Running,
            round: None,
            title: "WorkItemPlan Final Compile".to_string(),
            summary: Some("编译已确认 Draft 并写入真实 Work Item".to_string()),
            status: TimelineNodeStatus::Active,
        })
        .await;

        match self.run_work_item_plan_compile().await {
            Ok(report) => {
                let work_item_count = report.work_item_ids.len();
                self.update_artifact(ArtifactPayload::WorkItemPlanCompileReport {
                    compile_report: Box::new(report),
                })
                .await;
                self.complete_active_node(Some(format!(
                    "Final Compile 完成，已创建 {work_item_count} 个 Work Item"
                )))
                .await;
                self.enter_human_confirm(Some(format!(
                    "Final Compile 完成，已创建 {work_item_count} 个 Work Item，等待最终确认"
                )))
                .await;
            }
            Err(message) => {
                self.complete_active_node(Some(format!("Final Compile 失败：{message}")))
                    .await;
                if self.mark_latest_compile_transaction_recovery_required(&message) {
                    self.enter_work_item_plan_compile_recovery(Some(format!(
                        "Final Compile 需要恢复：{message}"
                    )))
                    .await;
                } else if self.is_current_work_item_plan_batch_mode() {
                    self.enter_work_item_batch_confirm(Some(format!(
                        "Final Compile strict validator 失败：{message}"
                    )))
                    .await;
                } else {
                    self.enter_human_confirm(Some(format!("Final Compile 失败：{message}")))
                        .await;
                }
            }
        }
    }

    pub(crate) async fn enter_work_item_plan_compile_recovery(&mut self, summary: Option<String>) {
        self.transition_stage(WorkspaceStage::HumanConfirm).await;
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::WorkItemPlanCompileRecovery,
                agent: None,
                stage: WorkspaceStage::HumanConfirm,
                round: None,
                title: "WorkItemPlan Compile Recovery".to_string(),
                summary,
                status: TimelineNodeStatus::Active,
            })
            .await;
        if let Some(store) = &self.lifecycle_store {
            let _ = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::WaitingForHuman,
            );
        }
    }

    pub(crate) fn mark_latest_compile_transaction_recovery_required(&self, message: &str) -> bool {
        let Ok(store) = self.work_item_plan_store() else {
            return false;
        };
        let Ok(Some(mut tx)) = store
            .list_compile_transactions(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map(|transactions| {
                transactions
                    .into_iter()
                    .filter(|tx| {
                        matches!(
                            tx.status,
                            WorkItemPlanCompileStatus::Preparing
                                | WorkItemPlanCompileStatus::Validating
                                | WorkItemPlanCompileStatus::Committing
                                | WorkItemPlanCompileStatus::RecoveryRequired
                        )
                    })
                    .max_by(|left, right| left.created_at.cmp(&right.created_at))
            })
        else {
            return false;
        };
        tx.status = WorkItemPlanCompileStatus::RecoveryRequired;
        tx.failure_reason = Some(message.to_string());
        tx.updated_at = chrono::Utc::now().to_rfc3339();
        store.put_compile_transaction(&tx).is_ok()
    }

    pub(crate) async fn run_work_item_plan_compile(
        &mut self,
    ) -> Result<WorkItemPlanCompileReportPayload, String> {
        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let store = self.work_item_plan_store()?;
        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let plan_id = self.session.entity_id.clone();
        let previous_plan = lifecycle
            .get_issue_work_item_plan(&project_id, &issue_id, &plan_id)
            .map_err(|error| format!("load issue work item plan failed: {error}"))?;
        let index = store
            .load_active_index(&project_id, &issue_id, &plan_id)
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
        let outline_order = work_item_plan_outline_topological_order(&outline_candidate.outline)?;
        let draft_records =
            self.accepted_active_draft_records_for_compile(&store, &index, &outline_order)?;
        let active_draft_ids: Vec<String> = draft_records
            .iter()
            .map(|record| record.draft_id.clone())
            .collect();
        let compile_id = next_compile_id();
        let now = chrono::Utc::now().to_rfc3339();
        let outline_to_work_item_id: BTreeMap<String, String> = outline_order
            .iter()
            .enumerate()
            .map(|(index, outline_id)| {
                (outline_id.clone(), compile_work_item_id(&compile_id, index))
            })
            .collect();
        let outline_to_verification_plan_id: BTreeMap<String, String> = outline_order
            .iter()
            .enumerate()
            .map(|(index, outline_id)| {
                (
                    outline_id.clone(),
                    compile_verification_plan_id(&compile_id, index),
                )
            })
            .collect();
        let mut tx = WorkItemPlanCompileTransaction {
            compile_id: compile_id.clone(),
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            plan_id: plan_id.clone(),
            generation_round_id: index.current_generation_round_id.clone(),
            outline_version_ref: outline_candidate.outline.id.clone(),
            active_draft_ids,
            status: WorkItemPlanCompileStatus::Preparing,
            plan_commit_state: WorkItemPlanCommitState::NotStarted,
            step_cursor: "preparing".to_string(),
            outline_to_work_item_id: BTreeMap::new(),
            outline_to_verification_plan_id: BTreeMap::new(),
            created_work_item_ids: Vec::new(),
            created_verification_plan_ids: Vec::new(),
            child_session_ids: Vec::new(),
            validator_findings: Vec::new(),
            abort_requested_at: None,
            failure_reason: None,
            previous_plan_snapshot: previous_plan.clone(),
            created_at: now.clone(),
            updated_at: now.clone(),
            committed_at: None,
        };
        store
            .put_compile_transaction(&tx)
            .map_err(|error| format!("save compile transaction failed: {error}"))?;

        let repository_id = self.work_item_plan_repository_id(&lifecycle, &previous_plan)?;
        let (mut compiled_plan, work_items, verification_plans) = self
            .project_work_item_plan_drafts_for_compile(
                &previous_plan,
                &draft_records,
                WorkItemPlanCompileProjectionContext {
                    outline_order: &outline_order,
                    outline_to_work_item_id: &outline_to_work_item_id,
                    outline_to_verification_plan_id: &outline_to_verification_plan_id,
                    repository_id: &repository_id,
                    now: &now,
                },
            )?;
        tx.status = WorkItemPlanCompileStatus::Validating;
        tx.step_cursor = "validating".to_string();
        tx.outline_to_work_item_id = outline_to_work_item_id;
        tx.outline_to_verification_plan_id = outline_to_verification_plan_id;
        tx.updated_at = chrono::Utc::now().to_rfc3339();
        store
            .put_compile_transaction(&tx)
            .map_err(|error| format!("save validating compile transaction failed: {error}"))?;

        let report = WorkItemSplitValidator::validate(
            &compiled_plan,
            &work_items,
            None,
            &verification_plans,
        );
        tx.validator_findings = report.findings.clone();
        if report.has_errors() {
            let failure_report = WorkItemPlanCompileReportPayload {
                compile_id: compile_id.clone(),
                generation_round_id: index.current_generation_round_id.clone(),
                status: WorkItemPlanCompileStatus::Failed,
                plan_commit_state: WorkItemPlanCommitState::NotStarted,
                work_item_ids: Vec::new(),
                verification_plan_ids: Vec::new(),
                child_session_ids: Vec::new(),
                validator_findings: work_item_split_findings_to_dto(&tx.validator_findings),
            };
            tx.status = WorkItemPlanCompileStatus::Failed;
            tx.failure_reason = Some(work_item_plan_findings_summary(
                "Final Compile strict validator failed",
                &report.findings,
            ));
            tx.updated_at = chrono::Utc::now().to_rfc3339();
            store
                .put_compile_transaction(&tx)
                .map_err(|error| format!("save failed compile transaction failed: {error}"))?;
            self.update_artifact(ArtifactPayload::WorkItemPlanCompileReport {
                compile_report: Box::new(failure_report),
            })
            .await;
            return Err(work_item_plan_findings_summary(
                "Final Compile strict validator failed",
                &report.findings,
            ));
        }
        compiled_plan.validator_findings = report.findings.clone();

        tx.status = WorkItemPlanCompileStatus::Committing;
        tx.step_cursor = "committing".to_string();
        tx.updated_at = chrono::Utc::now().to_rfc3339();
        store
            .put_compile_transaction(&tx)
            .map_err(|error| format!("save committing compile transaction failed: {error}"))?;

        for (work_item, verification_plan) in work_items.iter().zip(verification_plans.iter()) {
            if !tx.created_work_item_ids.contains(&work_item.id) {
                lifecycle
                    .create_work_item(CreateWorkItemInput {
                        id: Some(work_item.id.clone()),
                        project_id: work_item.project_id.clone(),
                        issue_id: work_item.issue_id.clone(),
                        repository_id: work_item.repository_id.clone(),
                        story_spec_ids: work_item.story_spec_ids.clone(),
                        design_spec_ids: work_item.design_spec_ids.clone(),
                        title: work_item.title.clone(),
                        work_item_set_id: work_item.work_item_set_id.clone(),
                        kind: work_item.kind.clone(),
                        sequence_hint: work_item.sequence_hint,
                        depends_on: work_item.depends_on.clone(),
                        exclusive_write_scopes: work_item.exclusive_write_scopes.clone(),
                        forbidden_write_scopes: work_item.forbidden_write_scopes.clone(),
                        context_budget: work_item.context_budget.clone(),
                        required_handoff_from: work_item.required_handoff_from.clone(),
                        verification_plan_ref: work_item.verification_plan_ref.clone(),
                        require_execution_plan_confirm: work_item.require_execution_plan_confirm,
                        plan_status: WorkItemPlanStatus::Confirmed,
                    })
                    .map_err(|error| format!("create work item failed: {error}"))?;
                tx.created_work_item_ids.push(work_item.id.clone());
            }
            if !tx
                .created_verification_plan_ids
                .contains(&verification_plan.id)
            {
                lifecycle
                    .create_verification_plan(CreateVerificationPlanInput {
                        id: Some(verification_plan.id.clone()),
                        project_id: verification_plan.project_id.clone(),
                        issue_id: verification_plan.issue_id.clone(),
                        work_item_id: verification_plan.work_item_id.clone(),
                        repository_profile_ref: verification_plan.repository_profile_ref.clone(),
                        provider_run_ref: verification_plan.provider_run_ref.clone(),
                        scope: verification_plan.scope.clone(),
                        commands: verification_plan.commands.clone(),
                        manual_checks: verification_plan.manual_checks.clone(),
                        required_gates: verification_plan.required_gates.clone(),
                        risk_notes: verification_plan.risk_notes.clone(),
                        confidence: verification_plan.confidence.clone(),
                        fallback_policy: verification_plan.fallback_policy.clone(),
                    })
                    .map_err(|error| format!("create verification plan failed: {error}"))?;
                tx.created_verification_plan_ids
                    .push(verification_plan.id.clone());
            }
            let child_session = lifecycle
                .create_workspace_session(CreateWorkspaceSessionInput {
                    project_id: project_id.clone(),
                    issue_id: issue_id.clone(),
                    entity_id: work_item.id.clone(),
                    workspace_type: WorkspaceType::WorkItem,
                    author_provider: self.session.author_provider.clone(),
                    reviewer_provider: self
                        .session
                        .reviewer_provider
                        .clone()
                        .unwrap_or(ProviderName::Codex),
                    review_rounds: self.session.review_rounds,
                    superpowers_enabled: self.session.superpowers_enabled,
                    openspec_enabled: self.session.openspec_enabled,
                })
                .map_err(|error| format!("create child work item workspace failed: {error}"))?;
            tx.child_session_ids.push(child_session.id);
            tx.updated_at = chrono::Utc::now().to_rfc3339();
            store
                .put_compile_transaction(&tx)
                .map_err(|error| format!("save compile step cursor failed: {error}"))?;
        }

        tx.plan_commit_state = WorkItemPlanCommitState::Committed;
        tx.committed_at = Some(chrono::Utc::now().to_rfc3339());
        tx.step_cursor = "plan_commit_marker_written".to_string();
        tx.updated_at = chrono::Utc::now().to_rfc3339();
        store
            .put_compile_transaction(&tx)
            .map_err(|error| format!("save compile committed marker failed: {error}"))?;

        lifecycle
            .commit_issue_work_item_plan(
                &project_id,
                &issue_id,
                &plan_id,
                IssueWorkItemPlanUpdate {
                    work_item_ids: compiled_plan.work_item_ids.clone(),
                    verification_plan_ids: compiled_plan.verification_plan_ids.clone(),
                    repository_profile_ref: None,
                    dependency_graph: compiled_plan.dependency_graph.clone(),
                    created_from_provider_run: compiled_plan.created_from_provider_run.clone(),
                    validator_findings: compiled_plan.validator_findings.clone(),
                },
            )
            .map_err(|error| format!("commit issue work item plan failed: {error}"))?;

        tx.status = WorkItemPlanCompileStatus::Committed;
        tx.step_cursor = "committed".to_string();
        tx.updated_at = chrono::Utc::now().to_rfc3339();
        store
            .put_compile_transaction(&tx)
            .map_err(|error| format!("save committed compile transaction failed: {error}"))?;

        Ok(WorkItemPlanCompileReportPayload {
            compile_id,
            generation_round_id: index.current_generation_round_id,
            status: WorkItemPlanCompileStatus::Committed,
            plan_commit_state: WorkItemPlanCommitState::Committed,
            work_item_ids: compiled_plan.work_item_ids,
            verification_plan_ids: compiled_plan.verification_plan_ids,
            child_session_ids: tx.child_session_ids,
            validator_findings: work_item_split_findings_to_dto(&tx.validator_findings),
        })
    }

    pub async fn handle_work_item_plan_compile_recovery_action(
        &mut self,
        action: WorkItemPlanCompileRecoveryActionDto,
        reason: Option<String>,
    ) -> Result<WorkItemPlanCompileRecoveryOutcome, String> {
        if self.session.workspace_type != WorkspaceType::WorkItemPlan
            || self.active_node_type() != Some(TimelineNodeType::WorkItemPlanCompileRecovery)
        {
            return Err(
                "work_item_plan_compile_recovery_action requires active work_item_plan_compile_recovery node"
                    .to_string(),
            );
        }

        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let store = self.work_item_plan_store()?;
        let mut tx = self.latest_work_item_plan_recovery_transaction(&store)?;

        match action {
            WorkItemPlanCompileRecoveryActionDto::AbortAndRollback => {
                if tx.plan_commit_state == WorkItemPlanCommitState::Committed {
                    return Err(
                        "abort_and_rollback is not allowed when plan_commit_state=committed"
                            .to_string(),
                    );
                }

                for verification_plan_id in tx.created_verification_plan_ids.clone() {
                    lifecycle
                        .delete_verification_plan(
                            &tx.project_id,
                            &tx.issue_id,
                            &verification_plan_id,
                        )
                        .map_err(|error| {
                            format!("delete verification plan during rollback failed: {error}")
                        })?;
                }
                for work_item_id in tx.created_work_item_ids.clone() {
                    lifecycle
                        .delete_work_item(&tx.project_id, &tx.issue_id, &work_item_id)
                        .map_err(|error| {
                            format!("delete work item during rollback failed: {error}")
                        })?;
                }
                lifecycle
                    .restore_issue_work_item_plan_snapshot(
                        &tx.project_id,
                        &tx.issue_id,
                        &tx.plan_id,
                        &tx.previous_plan_snapshot,
                    )
                    .map_err(|error| format!("restore previous WorkItemPlan failed: {error}"))?;

                tx.status = WorkItemPlanCompileStatus::Failed;
                tx.created_work_item_ids.clear();
                tx.created_verification_plan_ids.clear();
                tx.child_session_ids.clear();
                tx.failure_reason = Some(
                    reason
                        .unwrap_or_else(|| "compile recovery aborted and rolled back".to_string()),
                );
                tx.step_cursor = "rolled_back".to_string();
                tx.updated_at = chrono::Utc::now().to_rfc3339();
                store.put_compile_transaction(&tx).map_err(|error| {
                    format!("save rolled back compile transaction failed: {error}")
                })?;

                self.complete_active_node(Some(
                    "已放弃本次 Final Compile 并恢复旧 Plan".to_string(),
                ))
                .await;
                self.enter_human_confirm(Some(
                    "Final Compile 已回滚，等待人工确认下一步".to_string(),
                ))
                .await;
                Ok(WorkItemPlanCompileRecoveryOutcome::HumanConfirm)
            }
            WorkItemPlanCompileRecoveryActionDto::Continue => {
                if tx.plan_commit_state == WorkItemPlanCommitState::Committed {
                    self.commit_recovered_work_item_plan_after_marker(&lifecycle, &tx)?;
                    tx.status = WorkItemPlanCompileStatus::Committed;
                    tx.failure_reason = reason.or(tx.failure_reason);
                    tx.step_cursor = "committed".to_string();
                    tx.updated_at = chrono::Utc::now().to_rfc3339();
                    store.put_compile_transaction(&tx).map_err(|error| {
                        format!("save continued compile transaction failed: {error}")
                    })?;
                    self.complete_active_node(Some(
                        "Final Compile 已从 committed marker 恢复".to_string(),
                    ))
                    .await;
                    self.enter_human_confirm(Some(
                        "Final Compile 已提交，等待最终确认".to_string(),
                    ))
                    .await;
                    return Ok(WorkItemPlanCompileRecoveryOutcome::HumanConfirm);
                }

                self.complete_active_node(Some("继续 Final Compile".to_string()))
                    .await;
                self.enter_work_item_plan_compile().await;
                Ok(WorkItemPlanCompileRecoveryOutcome::Continue)
            }
            WorkItemPlanCompileRecoveryActionDto::HumanTriage => {
                tx.failure_reason = reason.or(tx.failure_reason);
                tx.updated_at = chrono::Utc::now().to_rfc3339();
                store.put_compile_transaction(&tx).map_err(|error| {
                    format!("save human triage compile transaction failed: {error}")
                })?;
                self.complete_active_node(Some("Final Compile 转人工处理".to_string()))
                    .await;
                self.enter_human_confirm(Some("Final Compile 需要人工整理".to_string()))
                    .await;
                Ok(WorkItemPlanCompileRecoveryOutcome::HumanConfirm)
            }
        }
    }

    pub(crate) fn latest_work_item_plan_recovery_transaction(
        &self,
        store: &WorkItemPlanStore,
    ) -> Result<WorkItemPlanCompileTransaction, String> {
        store
            .list_compile_transactions(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("list compile transactions failed: {error}"))?
            .into_iter()
            .filter(|tx| tx.status == WorkItemPlanCompileStatus::RecoveryRequired)
            .max_by(|left, right| left.created_at.cmp(&right.created_at))
            .ok_or_else(|| "work item plan compile recovery transaction is missing".to_string())
    }

    pub(crate) fn commit_recovered_work_item_plan_after_marker(
        &self,
        lifecycle: &LifecycleStore,
        tx: &WorkItemPlanCompileTransaction,
    ) -> Result<(), String> {
        let work_items = lifecycle
            .list_work_items(&tx.project_id, &tx.issue_id)
            .map_err(|error| format!("list work items during compile recovery failed: {error}"))?;
        let created_work_item_ids: HashSet<&str> = tx
            .created_work_item_ids
            .iter()
            .map(String::as_str)
            .collect();
        let work_items_by_id: HashMap<&str, &LifecycleWorkItemRecord> = work_items
            .iter()
            .filter(|item| created_work_item_ids.contains(item.id.as_str()))
            .map(|item| (item.id.as_str(), item))
            .collect();
        for work_item_id in &tx.created_work_item_ids {
            if !work_items_by_id.contains_key(work_item_id.as_str()) {
                return Err(format!(
                    "created work item `{work_item_id}` missing during compile recovery"
                ));
            }
        }

        let dependency_graph = tx
            .created_work_item_ids
            .iter()
            .filter_map(|work_item_id| work_items_by_id.get(work_item_id.as_str()).copied())
            .flat_map(|work_item| {
                work_item
                    .depends_on
                    .iter()
                    .cloned()
                    .map(|from_work_item_id| IssueWorkItemDependencyEdge {
                        from_work_item_id,
                        to_work_item_id: work_item.id.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        lifecycle
            .commit_issue_work_item_plan(
                &tx.project_id,
                &tx.issue_id,
                &tx.plan_id,
                IssueWorkItemPlanUpdate {
                    work_item_ids: tx.created_work_item_ids.clone(),
                    verification_plan_ids: tx.created_verification_plan_ids.clone(),
                    repository_profile_ref: None,
                    dependency_graph,
                    created_from_provider_run: tx
                        .previous_plan_snapshot
                        .created_from_provider_run
                        .clone(),
                    validator_findings: tx.validator_findings.clone(),
                },
            )
            .map_err(|error| format!("commit recovered WorkItemPlan failed: {error}"))?;
        Ok(())
    }

    pub(crate) fn accepted_active_draft_records_for_compile(
        &self,
        store: &WorkItemPlanStore,
        index: &WorkItemPlanDraftActiveIndex,
        outline_order: &[String],
    ) -> Result<Vec<WorkItemDraftRecord>, String> {
        let mut records = Vec::with_capacity(outline_order.len());
        for outline_id in outline_order {
            let draft_id = index
                .outline_to_current_draft_id
                .get(outline_id)
                .ok_or_else(|| format!("outline `{outline_id}` has no active draft"))?;
            if index.draft_statuses.get(draft_id) != Some(&WorkItemDraftStatus::Accepted) {
                return Err(format!("draft `{draft_id}` is not accepted"));
            }
            let record = store
                .get_draft_record(
                    &index.project_id,
                    &index.issue_id,
                    &index.plan_id,
                    &index.current_generation_round_id,
                    draft_id,
                )
                .map_err(|error| format!("load active draft `{draft_id}` failed: {error}"))?;
            if !record.active || record.status != WorkItemDraftStatus::Accepted {
                return Err(format!(
                    "draft `{draft_id}` is not an accepted active draft"
                ));
            }
            if record.superseded_by_draft_id.is_some()
                || record.supersede_reason.is_some()
                || record.superseded_at.is_some()
            {
                return Err(format!("draft `{draft_id}` has been superseded"));
            }
            records.push(record);
        }
        Ok(records)
    }
}
