use super::execution::ProviderTestingExecutionPhase;
use super::*;

pub(crate) struct ProviderTestingReportInput<'a> {
    pub(crate) attempt: CodingExecutionAttempt,
    pub(crate) node: CodingTimelineNode,
    pub(crate) role_run: CodingRoleRun,
    pub(crate) provider: &'a dyn StreamingProviderAdapter,
    pub(crate) worktree_path: PathBuf,
    pub(crate) tester_provider: ProviderName,
    pub(crate) plan: TestPlan,
    pub(crate) options: &'a TesterAgentOptions,
    pub(crate) command_rx: &'a mut mpsc::Receiver<CodingRunnerCommand>,
    pub(crate) phase: ProviderTestingExecutionPhase,
}

impl CodingWorkspaceEngine {
    pub(crate) async fn finalize_provider_testing_report_phase(
        &self,
        input: ProviderTestingReportInput<'_>,
    ) -> Result<TestingReport, CodingWorkspaceEngineError> {
        let ProviderTestingReportInput {
            attempt,
            node,
            role_run,
            provider,
            worktree_path,
            tester_provider,
            plan,
            options,
            command_rx,
            phase,
        } = input;
        let ProviderTestingExecutionPhase {
            full_output,
            mut step_results,
            unplanned_commands,
            unplanned_evidence,
            context_warnings,
            blocked_summary,
            blocked_reason_code,
            mut chat_entry_sequence,
        } = phase;
        let execute_raw_ref = self.store.save_provider_raw_output(
            &attempt.id,
            CodingExecutionStage::Testing,
            "execute_test_plan",
            &full_output,
        )?;
        for provider_step_result in parse_testing_step_results_from_provider_output(&full_output) {
            if !step_results
                .iter()
                .any(|existing| existing.step_id == provider_step_result.step_id)
            {
                step_results.push(provider_step_result);
            }
        }
        let mut report_plan = plan.clone();
        for warning in context_warnings {
            if !report_plan
                .context_warnings
                .iter()
                .any(|existing| existing == &warning)
            {
                report_plan.context_warnings.push(warning);
            }
        }
        let report_id = next_sequential_id(
            "testing_report",
            self.store
                .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?
                .len(),
        );
        let mut report_raw_ref = execute_raw_ref.clone();
        let provider_claim = serde_json::from_str(&full_output).ok();
        let mut report = build_plan_based_testing_report(
            &report_id,
            &attempt.id,
            &report_plan,
            step_results.clone(),
            unplanned_commands.clone(),
            provider_claim,
            Some(report_raw_ref.clone()),
        );
        report.unplanned_evidence = unplanned_evidence.clone();
        if !report.missing_required_steps.is_empty() && blocked_summary.is_none() {
            let repair_prompt =
                build_tester_execute_repair_prompt(&full_output, &report.missing_required_steps);
            let repair_adapter_input = AdapterInput {
                provider_type: provider_type_for_name(&tester_provider),
                role: AdapterRole::Reviewer,
                worktree_path: Some(worktree_path.to_string_lossy().to_string()),
                prompt: repair_prompt,
                context_files: Vec::new(),
                output_schema: "coding_workspace_test_execution_json".to_string(),
                timeout: options.timeout.as_secs().max(1),
                max_retries: 0,
            };
            let repair_input = StreamingProviderInput {
                provider_type: repair_adapter_input.provider_type.clone(),
                role: repair_adapter_input.role.clone(),
                prompt: repair_adapter_input.prompt.clone(),
                working_dir: worktree_path.clone(),
                workspace_session_id: Some(attempt.id.clone()),
                resume_provider_session_id: None,
                permission_mode: role_permission_mode_for_attempt(
                    &self.store,
                    &attempt,
                    CodingProviderRole::Tester,
                )?,
                env_vars: BTreeMap::new(),
                timeout_secs: repair_adapter_input.timeout,
            };
            let repair_output = self
                .run_provider_stream_to_completion(CodingProviderStreamRun {
                    attempt: &attempt,
                    node_id: &node.id,
                    role_run: Some(&role_run),
                    provider,
                    legacy_input: &repair_adapter_input,
                    input: repair_input,
                    provider_name: &tester_provider,
                    provider_role: CodingProviderRole::Tester,
                    command_rx,
                    allow_legacy_stream_fallback: false,
                    timeout: None,
                    timeout_reason_code: None,
                })
                .await?;
            let repair_raw_ref = self.store.save_provider_raw_output(
                &attempt.id,
                CodingExecutionStage::Testing,
                "execute_test_plan_repair",
                &repair_output,
            )?;
            report_raw_ref = repair_raw_ref;
            for provider_step_result in
                parse_testing_step_results_from_provider_output(&repair_output)
            {
                if !step_results
                    .iter()
                    .any(|existing| existing.step_id == provider_step_result.step_id)
                {
                    step_results.push(provider_step_result);
                }
            }
            let repair_provider_claim = serde_json::from_str(&repair_output).ok();
            report = build_plan_based_testing_report(
                &report_id,
                &attempt.id,
                &report_plan,
                step_results.clone(),
                unplanned_commands.clone(),
                repair_provider_claim,
                Some(report_raw_ref.clone()),
            );
            report.unplanned_evidence = unplanned_evidence.clone();
        }
        if let Some(summary) = blocked_summary {
            report.overall_status = TestingOverallStatus::Blocked;
            report.context_warnings.push(summary);
        }
        bind_testing_report_role_run(&mut report, &role_run);
        self.store.save_testing_report(&report)?;
        let entry = tester_chat_entry(
            &attempt,
            &node.id,
            &mut chat_entry_sequence,
            CodingEntryType::AssistantMessage,
            Some(format_testing_report_chat_summary(&report)),
            Some(serde_json::json!({
                "phase": "testing_result",
                "testing_report_id": report.id.clone(),
                "role_run_id": role_run.id.clone(),
                "run_no": role_run.run_no,
                "raw_provider_output_ref": report.raw_provider_output_ref.clone()
            })),
        );
        self.save_and_emit_chat_entry(entry).await;
        self.store.update_role_run_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &role_run.id,
            testing_role_run_status(&report),
            derive_testing_role_run_reason(&report),
        )?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::TestingReportUpdate {
                report: Box::new(report.clone()),
            })
            .await;

        let (node_status, summary) = match report.overall_status {
            TestingOverallStatus::Passed => (
                CodingTimelineNodeStatus::Completed,
                Some("测试通过".to_string()),
            ),
            TestingOverallStatus::PassedWithWarnings => (
                CodingTimelineNodeStatus::Completed,
                Some("测试通过但有警告".to_string()),
            ),
            TestingOverallStatus::Failed => (
                CodingTimelineNodeStatus::Failed,
                Some("测试失败".to_string()),
            ),
            TestingOverallStatus::SkippedByUserDecision => (
                CodingTimelineNodeStatus::Completed,
                Some("测试由用户决策跳过".to_string()),
            ),
            TestingOverallStatus::Blocked => (
                CodingTimelineNodeStatus::Blocked,
                Some("测试被阻塞".to_string()),
            ),
        };
        if matches!(
            report.overall_status,
            TestingOverallStatus::Failed | TestingOverallStatus::Blocked
        ) && !testing_report_should_enter_analyst(&report)
        {
            self.store.update_attempt_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                CodingAttemptStatus::Blocked,
            )?;
            self.release_active_lock_if_shared_worktree_clean(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                self.active_work_item_id_for_attempt(&attempt),
            )
            .await?;
        }
        if report.overall_status == TestingOverallStatus::Blocked
            && !testing_report_should_enter_analyst(&report)
        {
            let gate = self.store.create_blocked_gate(CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::Testing,
                node_id: Some(node.id.clone()),
                role: Some(CodingProviderRole::Tester),
                title: "Testing blocked".to_string(),
                description: "Required testing steps are missing or blocked".to_string(),
                reason_code: Some(derive_testing_blocked_reason_code(
                    blocked_reason_code,
                    &report,
                )),
                evidence_refs: vec![format!("{}.json", report.id)],
                raw_provider_output_ref: Some(report_raw_ref),
                available_actions: testing_blocked_gate_actions(),
            })?;
            let _ = self
                .event_tx
                .send(CodingWsOutMessage::CodingGateRequired { gate })
                .await;
        }
        self.complete_timeline_node(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &node.id,
            node_status,
            summary,
        )
        .await?;
        Ok(report)
    }
}
