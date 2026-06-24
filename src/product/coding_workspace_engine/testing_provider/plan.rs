use super::*;

pub(crate) struct ProviderTestingPlanPhase {
    pub(crate) tester_provider: ProviderName,
    pub(crate) evaluation_context_json: String,
    pub(crate) plan: TestPlan,
    pub(crate) chat_entry_sequence: usize,
}

pub(crate) struct ProviderTestingPlanInput<'a> {
    pub(crate) attempt: CodingExecutionAttempt,
    pub(crate) node: CodingTimelineNode,
    pub(crate) role_run: CodingRoleRun,
    pub(crate) provider: &'a dyn StreamingProviderAdapter,
    pub(crate) worktree_path: PathBuf,
    pub(crate) options: &'a TesterAgentOptions,
    pub(crate) command_rx: &'a mut mpsc::Receiver<CodingRunnerCommand>,
}

pub(crate) enum ProviderTestingPlanOutcome {
    EarlyReport(TestingReport),
    Completed(ProviderTestingPlanPhase),
}

impl CodingWorkspaceEngine {
    pub(crate) async fn run_provider_testing_plan_phase(
        &self,
        input: ProviderTestingPlanInput<'_>,
    ) -> Result<ProviderTestingPlanOutcome, CodingWorkspaceEngineError> {
        let ProviderTestingPlanInput {
            attempt,
            node,
            role_run,
            provider,
            worktree_path,
            options,
            command_rx,
        } = input;
        let tester_provider = self
            .store
            .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)?
            .tester;
        let evaluation_context = build_evaluation_context_pack(
            self.store.paths(),
            &attempt,
            EvaluationContextRole::Tester,
        )?;
        let evaluation_context_json =
            serde_json::to_string_pretty(&evaluation_context).map_err(|error| {
                CodingWorkspaceEngineError::ProviderStream(format!(
                    "serialize_evaluation_context_failed: {error}"
                ))
            })?;
        let retry_diagnostic = self.retry_diagnostic_for_previous_run(&attempt, &role_run)?;
        let plan_prompt = build_tester_plan_prompt(
            &attempt,
            &evaluation_context_json,
            retry_diagnostic.as_deref(),
        );
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingExecutionEvent {
                event: provider_prompt_event(
                    &node.id,
                    &tester_provider,
                    plan_prompt.clone(),
                    "plan_tests",
                ),
            })
            .await;
        let resume_provider_session_id = self.provider_resume_session_id_for_attempt(
            &attempt,
            &CodingProviderRole::Tester,
            &tester_provider,
        );
        let mut chat_entry_sequence = 1usize;
        let plan_adapter_input = AdapterInput {
            provider_type: provider_type_for_name(&tester_provider),
            role: AdapterRole::Reviewer,
            worktree_path: Some(worktree_path.to_string_lossy().to_string()),
            prompt: plan_prompt,
            context_files: Vec::new(),
            output_schema: "coding_workspace_test_plan_json".to_string(),
            timeout: options.timeout.as_secs().max(1),
            max_retries: 0,
        };
        let plan_input = StreamingProviderInput {
            provider_type: plan_adapter_input.provider_type.clone(),
            role: plan_adapter_input.role.clone(),
            prompt: plan_adapter_input.prompt.clone(),
            working_dir: worktree_path.clone(),
            workspace_session_id: Some(attempt.id.clone()),
            resume_provider_session_id,
            permission_mode: role_permission_mode_for_attempt(
                &self.store,
                &attempt,
                CodingProviderRole::Tester,
            )?,
            env_vars: BTreeMap::new(),
            timeout_secs: plan_adapter_input.timeout,
        };
        let plan_output = match self
            .run_provider_stream_to_completion(CodingProviderStreamRun {
                attempt: &attempt,
                node_id: &node.id,
                role_run: Some(&role_run),
                provider,
                legacy_input: &plan_adapter_input,
                input: plan_input,
                provider_name: &tester_provider,
                provider_role: CodingProviderRole::Tester,
                command_rx,
                allow_legacy_stream_fallback: false,
                timeout: Some(options.timeout),
                timeout_reason_code: Some("plan_tests_timeout"),
            })
            .await
        {
            Ok(output) => output,
            Err(error) => {
                let reason_code = if error.to_string().contains("plan_tests_timeout") {
                    "plan_tests_timeout"
                } else {
                    "provider_start_failed"
                };
                return self
                    .block_provider_driven_testing(
                        &attempt,
                        &node,
                        BlockedTestingGateContext {
                            reason_code: reason_code.to_string(),
                            description: format!(
                                "Tester provider failed during plan_tests: {error}"
                            ),
                            raw_provider_output_ref: None,
                            role_run: Some(&role_run),
                        },
                    )
                    .await
                    .map(ProviderTestingPlanOutcome::EarlyReport);
            }
        };
        let plan_raw_ref = self.store.save_provider_raw_output(
            &attempt.id,
            CodingExecutionStage::Testing,
            "plan_tests",
            &plan_output,
        )?;
        let plan_id = next_sequential_id(
            "test_plan",
            self.store
                .list_test_plans(&attempt.project_id, &attempt.issue_id, &attempt.id)?
                .len(),
        );
        let plan = match parse_test_plan_payload(
            &attempt.id,
            &plan_id,
            &plan_output,
            Some(plan_raw_ref.clone()),
        ) {
            Ok(mut plan) => {
                bind_test_plan_role_run(&mut plan, &role_run);
                self.store.save_test_plan(&plan)?;
                plan
            }
            Err(first_error) => {
                let repair_prompt =
                    build_tester_plan_repair_prompt(&plan_output, &first_error.to_string());
                let repair_adapter_input = AdapterInput {
                    provider_type: provider_type_for_name(&tester_provider),
                    role: AdapterRole::Reviewer,
                    worktree_path: Some(worktree_path.to_string_lossy().to_string()),
                    prompt: repair_prompt,
                    context_files: Vec::new(),
                    output_schema: "coding_workspace_test_plan_json".to_string(),
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
                let repair_output = match self
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
                        timeout: Some(options.timeout),
                        timeout_reason_code: Some("plan_tests_timeout"),
                    })
                    .await
                {
                    Ok(output) => output,
                    Err(error) => {
                        let reason_code = if error.to_string().contains("plan_tests_timeout") {
                            "plan_tests_timeout"
                        } else {
                            "provider_start_failed"
                        };
                        return self
                            .block_provider_driven_testing(
                                &attempt,
                                &node,
                                BlockedTestingGateContext {
                                    reason_code: reason_code.to_string(),
                                    description: format!(
                                        "Tester provider failed during plan_tests_repair: {error}"
                                    ),
                                    raw_provider_output_ref: None,
                                    role_run: Some(&role_run),
                                },
                            )
                            .await
                            .map(ProviderTestingPlanOutcome::EarlyReport);
                    }
                };
                let repair_raw_ref = self.store.save_provider_raw_output(
                    &attempt.id,
                    CodingExecutionStage::Testing,
                    "plan_tests_repair",
                    &repair_output,
                )?;
                match parse_test_plan_payload(
                    &attempt.id,
                    &plan_id,
                    &repair_output,
                    Some(repair_raw_ref.clone()),
                ) {
                    Ok(mut plan) => {
                        bind_test_plan_role_run(&mut plan, &role_run);
                        self.store.save_test_plan(&plan)?;
                        plan
                    }
                    Err(repair_error) => {
                        return self
                            .block_invalid_test_plan(
                                &attempt,
                                &node,
                                &repair_output,
                                repair_error.to_string(),
                                BlockedTestingGateContext {
                                    reason_code: "test_plan_repair_failed".to_string(),
                                    description: "TestPlan parse failed".to_string(),
                                    raw_provider_output_ref: Some(repair_raw_ref),
                                    role_run: Some(&role_run),
                                },
                            )
                            .await
                            .map(ProviderTestingPlanOutcome::EarlyReport);
                    }
                }
            }
        };
        let entry = tester_chat_entry(
            &attempt,
            &node.id,
            &mut chat_entry_sequence,
            CodingEntryType::AssistantMessage,
            Some(format_test_plan_chat_summary(&plan)),
            Some(serde_json::json!({
                "phase": "test_plan",
                "test_plan_id": plan.id.clone(),
                "role_run_id": role_run.id.clone(),
                "run_no": role_run.run_no,
                "raw_provider_output_ref": plan.raw_provider_output_ref.clone()
            })),
        );
        self.save_and_emit_chat_entry(entry).await;

        Ok(ProviderTestingPlanOutcome::Completed(
            ProviderTestingPlanPhase {
                tester_provider,
                evaluation_context_json,
                plan,
                chat_entry_sequence,
            },
        ))
    }
}
