impl CodingWorkspaceEngine {
    /// 解析 attempt 当前权威 plan 绑定：活动 coding unit → work item
    /// revision。解析失败一律 fail-closed（验证处理必须绑定可证明的
    /// plan revision）。
    pub(crate) fn verification_triage_current_revision(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<(String, crate::product::models::WorkItemRevision), CodingWorkspaceEngineError>
    {
        let unit = self
            .store
            .get_active_coding_unit(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .ok_or_else(|| {
                CodingWorkspaceEngineError::ProviderStream(
                    "verification_triage_plan_revision_unresolvable".to_string(),
                )
            })?;
        let plan_id = attempt.work_item_group_id.clone().ok_or_else(|| {
            CodingWorkspaceEngineError::ProviderStream(
                "verification_triage_plan_revision_unresolvable".to_string(),
            )
        })?;
        let revision_store =
            crate::product::work_item_revision_store::WorkItemRevisionStore::new(self.store.paths());
        let lineage = revision_store
            .get_plan_lineage(&attempt.project_id, &attempt.issue_id, &plan_id)
            .map_err(|_| {
                CodingWorkspaceEngineError::ProviderStream(
                    "verification_triage_plan_revision_unresolvable".to_string(),
                )
            })?;
        let revision = revision_store
            .get_work_item_revision(&lineage, &unit.logical_work_item_id, &unit.work_item_revision_id)
            .map_err(|_| {
                CodingWorkspaceEngineError::ProviderStream(
                    "verification_triage_plan_revision_unresolvable".to_string(),
                )
            })?;
        Ok((unit.work_item_revision_id, revision))
    }

    /// 解析绑定 check（从 revision 的 verification plan revision 读取，
    /// 不信任调用方自报的 check 语义）。
    pub(crate) fn verification_triage_bound_check(
        &self,
        attempt: &CodingExecutionAttempt,
        revision: &crate::product::models::WorkItemRevision,
        check_id: &str,
    ) -> Result<crate::product::work_item_contract::VerificationCheck, CodingWorkspaceEngineError>
    {
        let revision_store =
            crate::product::work_item_revision_store::WorkItemRevisionStore::new(self.store.paths());
        let lineage = revision_store
            .get_plan_lineage(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt
                    .work_item_group_id
                    .clone()
                    .unwrap_or_default(),
            )
            .map_err(|_| {
                CodingWorkspaceEngineError::ProviderStream(
                    "verification_triage_check_unresolvable".to_string(),
                )
            })?;
        let plan = revision_store
            .get_verification_plan_revision(&lineage, &revision.verification_plan_revision_id)
            .map_err(|_| {
                CodingWorkspaceEngineError::ProviderStream(
                    "verification_triage_check_unresolvable".to_string(),
                )
            })?;
        plan.verification_checks
            .into_iter()
            .find(|check| check.check_id == check_id)
            .ok_or_else(|| {
                CodingWorkspaceEngineError::ProviderStream(
                    "verification_triage_check_unresolvable".to_string(),
                )
            })
    }

    /// C2 oracle C-2b：当前权威 plan revision 的全部绑定 check（并列证据
    /// 数据源；复用 bound_check 同一权威解析链，不新增判定）。
    pub(crate) fn verification_triage_bound_checks(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<Vec<crate::product::work_item_contract::VerificationCheck>, CodingWorkspaceEngineError>
    {
        let (_plan_revision_id, revision) = self.verification_triage_current_revision(attempt)?;
        let revision_store =
            crate::product::work_item_revision_store::WorkItemRevisionStore::new(self.store.paths());
        let lineage = revision_store
            .get_plan_lineage(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt
                    .work_item_group_id
                    .clone()
                    .unwrap_or_default(),
            )
            .map_err(|_| {
                CodingWorkspaceEngineError::ProviderStream(
                    "verification_triage_check_unresolvable".to_string(),
                )
            })?;
        let plan = revision_store
            .get_verification_plan_revision(&lineage, &revision.verification_plan_revision_id)
            .map_err(|_| {
                CodingWorkspaceEngineError::ProviderStream(
                    "verification_triage_check_unresolvable".to_string(),
                )
            })?;
        Ok(plan.verification_checks)
    }

    /// 转入验证处理（门呈现面旁入口，不进动作枚举）：要求存在开放的可
    /// 转入门（coder 输出门或 CR 三门），绑定 finding／check／plan revision
    /// 与全部证据字段；同键未决重入返回既有记录。
    pub async fn enter_verification_triage(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        request: VerificationTriageEntryRequest,
    ) -> Result<
        crate::product::coding_attempt_store::VerificationTriageRecord,
        CodingWorkspaceEngineError,
    > {
        let attempt = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        let open_gates = self.store.list_open_blocked_gates(project_id, issue_id, attempt_id)?;
        let eligible: Vec<CodingGateRequired> = open_gates
            .into_iter()
            .filter(super::provider_failure::is_verification_triage_eligible_gate)
            .collect();
        if eligible.is_empty() {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "verification_triage_no_eligible_gate".to_string(),
            ));
        }
        let (plan_revision_id, revision) = self.verification_triage_current_revision(&attempt)?;
        let check =
            self.verification_triage_bound_check(&attempt, &revision, &request.check_id)?;

        // finding 绑定校验："<report_id>#<index>" 必须命中持久化 code review
        // report 的 finding；coder 输出门路径（开放 coding_output_human_triage
        // 门）绑定该门 plan defect finding 引用；其余 fail-closed。
        let finding_resolvable = request
            .finding_id
            .rsplit_once('#')
            .and_then(|(report_id, index)| {
                let index: usize = index.parse().ok()?;
                let reports = self
                    .store
                    .list_code_review_reports(project_id, issue_id, attempt_id)
                    .ok()?;
                reports
                    .iter()
                    .find(|report| report.id == report_id)
                    .and_then(|report| report.findings.get(index))
                    .map(|_| ())
            })
            .is_some()
            || eligible.iter().any(|gate| {
                gate.reason_code.as_deref() == Some(CODING_OUTPUT_HUMAN_TRIAGE_REASON_CODE)
            });
        if !finding_resolvable {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "verification_triage_finding_unresolvable".to_string(),
            ));
        }

        let input = crate::product::coding_attempt_store::EnterVerificationTriageInput {
            attempt_id: attempt.id.clone(),
            finding_id: request.finding_id,
            check_id: request.check_id,
            plan_revision_id,
            original_command: request.original_command.or(check.command.clone()),
            alternative_command: request.alternative_command,
            cwd: request.cwd,
            outcome: request.outcome,
            test_execution_count: request.test_execution_count,
            environment: request.environment,
            scope: vec![check.check_id],
            expires_at: (chrono::Utc::now() + chrono::Duration::days(7)).to_rfc3339(),
        };
        Ok(self
            .store
            .enter_verification_triage(&attempt, input)?)
    }

    /// 决定验证处理（三类结论均需用户明确批准；拒绝条件 fail-closed 返回
    /// 具体 reason code）。批准"等价证据"／"限域例外"后原门按
    /// manual_continue 语义续跑；批准"计划修订"转 AwaitingPlanAmendment
    /// 由既有 amendment 链接管。
    pub async fn decide_verification_triage(
        &self,
        project_id: &str,
        issue_id: &str,
        attempt_id: &str,
        request: VerificationTriageDecisionRequest,
    ) -> Result<
        crate::product::coding_attempt_store::VerificationTriageRecord,
        CodingWorkspaceEngineError,
    > {
        let attempt = self.store.get_attempt(project_id, issue_id, attempt_id)?;
        let record = self.store.get_verification_triage_record(
            project_id,
            issue_id,
            attempt_id,
            &request.triage_id,
        )?;
        let decided_by = request.decided_by.trim().to_string();
        let reason = request.reason.trim().to_string();
        let decision = crate::product::coding_attempt_store::VerificationTriageDecision::Approve {
            conclusion: request.conclusion,
            decided_by: decided_by.clone(),
            reason: reason.clone(),
        };
        // 幂等重放：已决同义决定直接返回首次 durable 结果（含续跑效果
        // 已落账的场合不再重复校验可能已推进的 plan revision）。
        if record.status != crate::product::coding_attempt_store::VerificationTriageStatus::Pending
        {
            let replayed = self.store.apply_verification_triage_decision(
                &attempt,
                &request.triage_id,
                &decision,
            )?;
            return Ok(replayed);
        }
        if decided_by.is_empty() || reason.is_empty() {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "verification_triage_approval_context_required".to_string(),
            ));
        }
        let expires_at = chrono::DateTime::parse_from_rfc3339(&record.expires_at)
            .map_err(|_| CodingWorkspaceEngineError::ProviderStream(
                "verification_triage_expiry_unreadable".to_string(),
            ))?;
        if chrono::Utc::now() > expires_at {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "verification_triage_expired".to_string(),
            ));
        }
        let (plan_revision_id, revision) = self.verification_triage_current_revision(&attempt)?;
        if plan_revision_id != record.plan_revision_id {
            return Err(CodingWorkspaceEngineError::ProviderStream(
                "verification_triage_plan_revision_expired".to_string(),
            ));
        }
        let check =
            self.verification_triage_bound_check(&attempt, &revision, &record.check_id)?;
        match request.conclusion {
            crate::product::coding_attempt_store::VerificationTriageConclusion::AcceptEquivalentEvidence => {
                let evidence_complete = [
                    record.alternative_command.as_deref(),
                    record.cwd.as_deref(),
                    record.outcome.as_deref(),
                ]
                .into_iter()
                .all(|field| field.is_some_and(|value| !value.trim().is_empty()));
                if !evidence_complete {
                    return Err(CodingWorkspaceEngineError::ProviderStream(
                        "verification_triage_evidence_incomplete".to_string(),
                    ));
                }
                if check.non_zero_test_execution_required
                    && !record
                        .test_execution_count
                        .is_some_and(|count| count > 0)
                {
                    return Err(CodingWorkspaceEngineError::ProviderStream(
                        "verification_triage_non_zero_test_required".to_string(),
                    ));
                }
            }
            crate::product::coding_attempt_store::VerificationTriageConclusion::GrantScopedEnvironmentException => {
                if request
                    .exemption_scope
                    .iter()
                    .any(|scope_entry| !record.scope.contains(scope_entry))
                {
                    return Err(CodingWorkspaceEngineError::ProviderStream(
                        "verification_triage_scope_exceeds_bound_check".to_string(),
                    ));
                }
            }
            crate::product::coding_attempt_store::VerificationTriageConclusion::ApprovePlanRevision => {}
        }

        // durable-first：先落决定（审计随记录），再执行结论续跑。
        let decided = self.store.apply_verification_triage_decision(
            &attempt,
            &request.triage_id,
            &decision,
        )?;
        match request.conclusion {
            crate::product::coding_attempt_store::VerificationTriageConclusion::ApprovePlanRevision => {
                let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
                if current.status != CodingAttemptStatus::AwaitingPlanAmendment {
                    self.store.update_attempt_status(
                        project_id,
                        issue_id,
                        attempt_id,
                        CodingAttemptStatus::AwaitingPlanAmendment,
                    )?;
                }
                // 经既有 amendment 链发起：linked plan repair 存在时幂等开门；
                // 不存在时记录与 AwaitingPlanAmendment 状态即为 durable 等待
                // 事实（Task 12 投影），不强造修订。
                let paused = self.store.get_attempt(project_id, issue_id, attempt_id)?;
                if matches!(
                    self.store.linked_active_plan_repair_snapshot(&paused),
                    Ok(Some(_))
                ) {
                    self.store.reconcile_linked_plan_repair_pause(&paused)?;
                }
            }
            crate::product::coding_attempt_store::VerificationTriageConclusion::AcceptEquivalentEvidence
            | crate::product::coding_attempt_store::VerificationTriageConclusion::GrantScopedEnvironmentException => {
                let current = self.store.get_attempt(project_id, issue_id, attempt_id)?;
                let open_gate = self
                    .store
                    .list_open_blocked_gates(project_id, issue_id, attempt_id)?
                    .into_iter()
                    .find(|gate| {
                        super::provider_failure::is_verification_triage_eligible_gate(gate)
                    });
                if let Some(gate) = open_gate {
                    let operator_context = format!(
                        "验证处理 {} 由 {} 批准：{}",
                        record.triage_id, decided_by, reason
                    );
                    self.continue_attempt_with_manual_context(&current, &gate, operator_context)?;
                    self.store.resolve_blocked_gate_with_action(
                        project_id,
                        issue_id,
                        attempt_id,
                        &gate.gate_id,
                        Some("manual_continue"),
                    )?;
                }
            }
        }
        Ok(decided)
    }

    // ─── C1 Task 6（REQ-WIGA-03）：租约三态判定与确认接管 ───

    /// C2 Task 2（REQ-CRO-02）：engine 侧统一 admission 入口——把 store 的
    /// `admit_coding_run_exclusive`（命令账本 → 租约三态 → 既有
    /// `ensure_provider_run_allowed`）映射为既有错误面；互斥停等以稳定
    /// reason code 呈现（已在运行／需确认接管／租约未知），不启动 provider。
    pub(crate) fn admit_provider_run(
        &self,
        attempt: &CodingExecutionAttempt,
        stage: &CodingExecutionStage,
        purpose: &str,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        use crate::product::coding_attempt_store::CodingRunExclusionDecision;

        let command_id = format!("admit-{}-{:?}-{}", attempt.id, stage, purpose);
        let payload_digest = format!(
            "{}|{}|{}|{:?}|{}",
            attempt.project_id, attempt.issue_id, attempt.id, stage, purpose
        );
        match self
            .store
            .admit_coding_run_exclusive(attempt, &command_id, &payload_digest)?
        {
            CodingRunExclusionDecision::Allowed(authoritative) => Ok(authoritative),
            CodingRunExclusionDecision::AlreadyRunning { lease } => {
                Err(CodingWorkspaceEngineError::ProviderStream(format!(
                    "coding_run_already_running: lease {} 已在运行，请等待",
                    lease.lease_id
                )))
            }
            CodingRunExclusionDecision::TakeoverRequired { lease } => {
                Err(CodingWorkspaceEngineError::ProviderStream(format!(
                    "coding_run_takeover_required: lease {} 持有者已终止，需确认接管后继续",
                    lease.lease_id
                )))
            }
            CodingRunExclusionDecision::LeaseUnknown { lease } => {
                Err(CodingWorkspaceEngineError::ProviderStream(format!(
                    "coding_run_lease_unknown: 租约状态未知，已停等：{}",
                    lease.evidence.join("; ")
                )))
            }
        }
    }

    /// 租约三态判定（应用服务，只读；实现移至 `CodingAttemptStore`，此处
    /// 委托保持既有公共签名，单一实现见 run_exclusion.rs）。只从现有
    /// durable 证据分类：活跃→`ActiveWait`（等待，不抢占）；owner 终态或
    /// 锁已明确释放→`DeadNeedsTakeover`（需确认才接管）；瞬态 owner、
    /// 证据缺失或读失败→`UnknownNeedsHuman`（绝不抢占）。
    pub fn classify_worktree_lease(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> crate::product::models::automation::LeaseDecision {
        self.store.classify_worktree_lease(project_id, issue_id)
    }


    /// 死亡租约确认接管（应用服务）。前置链：命令账本幂等（同 command
    /// 同 payload Replayed、异 payload fail-closed）→ 当前 enrollment
    /// binding 精确校验（过期/异载体 IdentityMismatch）→ 三态判定（活跃
    /// Rejected、未知 NeedsHuman，均不写任何文件）→ expected attempt 终态
    /// 证明 → 既有 worktree 文件锁内 owner CAS 清出死亡 owner（新 owner
    /// 由当前 binding 下的下一次合法 acquire 写入）。接管事实追加命令
    /// 账本；终态 attempt 记录只读保留，不建第二 attempt、不启动 provider。
    pub fn confirm_takeover(
        &self,
        project_id: &str,
        issue_id: &str,
        request: &crate::product::models::automation::LeaseTakeoverRequest,
    ) -> Result<crate::product::models::automation::LeaseTakeoverResult, ProductStoreError> {
        use crate::product::issue_automation_store::IssueAutomationStore;
        use crate::product::json_store::validate_relative_id;
        use crate::product::lifecycle_store::LifecycleStore;
        use crate::product::models::automation::{
            LeaseTakeoverResult, OperationState,
        };

        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let automation = IssueAutomationStore::new(self.store.paths());

        // 命令账本幂等：同 command 同 payload 重放首次 durable 结果。
        let digest = request.payload_digest();
        if automation
            .find_command_result(project_id, issue_id, &request.command_id, &digest)
            .map_err(lease_takeover_store_error)?
            .is_some()
        {
            return Ok(LeaseTakeoverResult {
                command_id: request.command_id.clone(),
                state: OperationState::Replayed,
                lease: self.classify_worktree_lease(project_id, issue_id),
            });
        }

        // 当前 binding 精确校验：过期/异载体/缺 enrollment 一律拒绝。
        let enrollment = automation.get(project_id, issue_id)?;
        let binding = enrollment
            .and_then(|enrollment| enrollment.binding_history)
            .map(|history| history.current)
            .ok_or_else(|| {
                lease_takeover_identity_mismatch("enrollment_binding", &request.command_id)
            })?;
        if binding != request.expected_binding {
            return Err(lease_takeover_identity_mismatch(
                "enrollment_binding",
                &format!(
                    "expected binding v{}, current binding v{}",
                    request.expected_binding.binding_version, binding.binding_version
                ),
            ));
        }

        // 三态判定：活跃只等待（Rejected）、未知停等（NeedsHuman），
        // 都不写任何 durable 文件。
        let decision = self.classify_worktree_lease(project_id, issue_id);
        match decision.disposition {
            crate::product::models::automation::LeaseDisposition::ActiveWait => {
                return Ok(LeaseTakeoverResult {
                    command_id: request.command_id.clone(),
                    state: OperationState::Rejected,
                    lease: decision,
                });
            }
            crate::product::models::automation::LeaseDisposition::UnknownNeedsHuman => {
                return Ok(LeaseTakeoverResult {
                    command_id: request.command_id.clone(),
                    state: OperationState::NeedsHuman,
                    lease: decision,
                });
            }
            crate::product::models::automation::LeaseDisposition::DeadNeedsTakeover => {}
        }

        // 死亡证明：expected attempt 必须存在于本 issue 且已终态；活跃或
        // 缺失均 fail-closed（不猜、不接管）。
        let dead_attempt = self
            .store
            .get_attempt(project_id, issue_id, &request.expected_attempt_id)
            .map_err(|_| {
                lease_takeover_identity_mismatch(
                    "lease_takeover_attempt",
                    &request.expected_attempt_id,
                )
            })?;
        if dead_attempt.status.is_active() {
            let mut decision = decision;
            decision.evidence.push(format!(
                "attempt {} is still active; takeover refused",
                dead_attempt.id
            ));
            return Ok(LeaseTakeoverResult {
                command_id: request.command_id.clone(),
                state: OperationState::Rejected,
                lease: decision,
            });
        }

        let lifecycle = LifecycleStore::new(self.store.paths());
        // 已释放（owner=None）的 Dead 判定无需 CAS 清出；带 owner 的死亡
        // 租约在既有 worktree 文件锁内按 expected_lease_id CAS 清出（issue
        // 维优先，其次按确定性顺序匹配仓维记录；迟到旧 owner 写入统一
        // IdentityMismatch）。
        let mut evidence = decision.evidence.clone();
        if !decision.lease_id.is_empty() {
            if decision.lease_id != request.expected_lease_id {
                return Err(lease_takeover_identity_mismatch(
                    "lease_takeover_owner",
                    &format!(
                        "expected lease {}, classified owner {}",
                        request.expected_lease_id, decision.lease_id
                    ),
                ));
            }
            let issue_record = lifecycle.get_issue_shared_worktree(project_id, issue_id)?;
            if issue_record
                .as_ref()
                .and_then(|record| record.current_lock_owner_id.as_deref())
                == Some(decision.lease_id.as_str())
            {
                lifecycle.takeover_issue_worktree_lock_after_dead_owner(
                    project_id,
                    issue_id,
                    &decision.lease_id,
                )?;
            } else {
                let mut matched = false;
                for repository_id in lifecycle.list_repo_shared_worktrees(project_id, issue_id)? {
                    let record =
                        lifecycle.get_repo_shared_worktree(project_id, issue_id, repository_id)?;
                    if record
                        .as_ref()
                        .and_then(|record| record.current_lock_owner_id.as_deref())
                        == Some(decision.lease_id.as_str())
                    {
                        lifecycle.takeover_repo_worktree_lock_after_dead_owner(
                            project_id,
                            issue_id,
                            repository_id,
                            &decision.lease_id,
                        )?;
                        matched = true;
                        break;
                    }
                }
                if !matched {
                    return Err(lease_takeover_identity_mismatch(
                        "lease_takeover_owner",
                        &decision.lease_id,
                    ));
                }
            }
            evidence.push(format!(
                "takeover_confirmed: dead owner {} cleared under binding v{}",
                decision.lease_id, binding.binding_version
            ));
        } else {
            evidence.push(
                "takeover_confirmed: lock already released; next acquire becomes the owner"
                    .to_string(),
            );
        }

        // 接管事实入命令账本（durable 结果；重放由前置账本查询承担）。
        automation
            .append_command_result(
                project_id,
                issue_id,
                &request.command_id,
                &digest,
                OperationState::Accepted,
            )
            .map_err(lease_takeover_store_error)?;

        let mut lease = self.classify_worktree_lease(project_id, issue_id);
        lease.evidence.extend(evidence);
        Ok(LeaseTakeoverResult {
            command_id: request.command_id.clone(),
            state: OperationState::Accepted,
            lease,
        })
    }
}
