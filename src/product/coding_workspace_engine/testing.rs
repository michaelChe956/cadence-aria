use super::*;

impl CodingWorkspaceEngine {
    pub async fn execute_testing(
        &self,
        attempt: &CodingExecutionAttempt,
        specs: &[TestCommandSpec],
    ) -> Result<TestingReport, CodingWorkspaceEngineError> {
        let Some(worktree_path) = attempt.worktree_path.as_ref() else {
            return Err(CodingWorkspaceEngineError::MissingWorktree(
                attempt.id.clone(),
            ));
        };
        let attempt = self.store.update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::Testing,
        )?;
        let node = self.create_testing_timeline_node(&attempt)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node: node.clone() })
            .await;
        let artifact_output_root = self.store.attempt_test_output_root(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
        );
        let report = run_all_tests(&attempt.id, worktree_path, artifact_output_root, specs).await?;
        self.store.save_testing_report(&report)?;
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
        let completed_at = Utc::now().to_rfc3339();
        self.store.update_timeline_node_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &node.id,
            node_status.clone(),
            summary.clone(),
            Some(completed_at.clone()),
        )?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeUpdated {
                node_id: node.id,
                status: node_status,
                summary,
                completed_at: Some(completed_at),
            })
            .await;
        Ok(report)
    }

    pub async fn create_testing_result_review_gate(
        &self,
        attempt: &CodingExecutionAttempt,
        report: &TestingReport,
    ) -> Result<Option<CodingGateRequired>, CodingWorkspaceEngineError> {
        let current =
            self.store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let open_testing_gate_exists = self
            .store
            .list_open_blocked_gates(&current.project_id, &current.issue_id, &current.id)?
            .into_iter()
            .any(|gate| {
                gate.stage == Some(CodingExecutionStage::Testing)
                    && gate.reason_code.as_deref() != Some(TESTING_RESULT_REVIEW_REASON_CODE)
            });
        if open_testing_gate_exists {
            return Ok(None);
        }

        if current.status != CodingAttemptStatus::Blocked {
            self.store.update_attempt_status(
                &current.project_id,
                &current.issue_id,
                &current.id,
                CodingAttemptStatus::Blocked,
            )?;
            self.release_active_lock_if_shared_worktree_clean(
                &current.project_id,
                &current.issue_id,
                &current.id,
                self.active_work_item_id_for_attempt(&current),
            )
            .await?;
        }

        let node_id = self
            .store
            .latest_role_run(
                &current.project_id,
                &current.issue_id,
                &current.id,
                CodingExecutionStage::Testing,
                CodingProviderRole::Tester,
            )?
            .and_then(|run| run.node_id);
        let gate = self.store.create_blocked_gate(CreateBlockedGateInput {
            attempt_id: current.id.clone(),
            stage: CodingExecutionStage::Testing,
            node_id,
            role: Some(CodingProviderRole::Tester),
            title: "确认 Tester 测试结果".to_string(),
            description: testing_result_review_description(report),
            reason_code: Some(TESTING_RESULT_REVIEW_REASON_CODE.to_string()),
            evidence_refs: vec![format!("{}.json", report.id)],
            raw_provider_output_ref: report.raw_provider_output_ref.clone(),
            available_actions: testing_result_review_gate_actions(),
        })?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingGateRequired { gate: gate.clone() })
            .await;
        Ok(Some(gate))
    }

    pub(crate) async fn save_blocked_testing_report_and_gate(
        &self,
        attempt: &CodingExecutionAttempt,
        node: &CodingTimelineNode,
        mut report: TestingReport,
        gate_context: BlockedTestingGateContext<'_>,
    ) -> Result<TestingReport, CodingWorkspaceEngineError> {
        let BlockedTestingGateContext {
            reason_code,
            description,
            raw_provider_output_ref,
            role_run,
        } = gate_context;
        if let Some(role_run) = role_run {
            bind_testing_report_role_run(&mut report, role_run);
        }
        self.store.save_testing_report(&report)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::TestingReportUpdate {
                report: Box::new(report.clone()),
            })
            .await;
        self.complete_timeline_node(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &node.id,
            CodingTimelineNodeStatus::Blocked,
            Some("测试被阻塞".to_string()),
        )
        .await?;
        if testing_blocked_report_needs_gate(&report, &reason_code) {
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
                self.active_work_item_id_for_attempt(attempt),
            )
            .await?;
            let gate = self.store.create_blocked_gate(CreateBlockedGateInput {
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::Testing,
                node_id: Some(node.id.clone()),
                role: Some(CodingProviderRole::Tester),
                title: "Testing blocked".to_string(),
                description,
                reason_code: Some(reason_code.clone()),
                evidence_refs: vec![format!("{}.json", report.id)],
                raw_provider_output_ref,
                available_actions: testing_blocked_gate_actions(),
            })?;
            let _ = self
                .event_tx
                .send(CodingWsOutMessage::CodingGateRequired { gate })
                .await;
        }
        if let Some(role_run) = role_run {
            self.store.update_role_run_status(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                &role_run.id,
                testing_role_run_status(&report),
                Some(reason_code.clone()),
            )?;
        }
        Ok(report)
    }

    pub(crate) async fn block_provider_driven_testing(
        &self,
        attempt: &CodingExecutionAttempt,
        node: &CodingTimelineNode,
        gate_context: BlockedTestingGateContext<'_>,
    ) -> Result<TestingReport, CodingWorkspaceEngineError> {
        let raw_provider_output_ref = gate_context.raw_provider_output_ref.clone();
        let reason_code = gate_context.reason_code.clone();
        let description = gate_context.description.clone();
        let report_id = next_sequential_id(
            "testing_report",
            self.store
                .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?
                .len(),
        );
        let mut report = build_testing_report(&attempt.id, Vec::new(), "", Some(description));
        report.id = report_id;
        report.overall_status = TestingOverallStatus::Blocked;
        report.raw_provider_output_ref = raw_provider_output_ref.clone();
        report.context_warnings.push(reason_code.to_string());
        self.save_blocked_testing_report_and_gate(attempt, node, report, gate_context)
            .await
    }

    pub(crate) async fn block_invalid_test_plan(
        &self,
        attempt: &CodingExecutionAttempt,
        node: &CodingTimelineNode,
        provider_output: &str,
        error: String,
        gate_context: BlockedTestingGateContext<'_>,
    ) -> Result<TestingReport, CodingWorkspaceEngineError> {
        let raw_provider_output_ref = gate_context.raw_provider_output_ref.clone();
        let reason_code = gate_context.reason_code.clone();
        let report_id = next_sequential_id(
            "testing_report",
            self.store
                .list_testing_reports(&attempt.project_id, &attempt.issue_id, &attempt.id)?
                .len(),
        );
        let mut report = build_testing_report(
            &attempt.id,
            Vec::new(),
            provider_output,
            Some(format!("TestPlan parse failed: {error}")),
        );
        report.id = report_id;
        report.raw_provider_output_ref = raw_provider_output_ref;
        report
            .context_warnings
            .push(format!("{reason_code}:{error}"));
        self.save_blocked_testing_report_and_gate(attempt, node, report, gate_context)
            .await
    }

    pub async fn execute_testing_with_provider(
        &self,
        attempt: &CodingExecutionAttempt,
        provider: &dyn StreamingProviderAdapter,
        context: &CodingExecutionContext,
        specs: &[TestCommandSpec],
        options: TesterAgentOptions,
    ) -> Result<TestingReport, CodingWorkspaceEngineError> {
        let (_command_tx, mut command_rx) = mpsc::channel(1);
        self.execute_testing_with_provider_commands(
            attempt,
            provider,
            context,
            specs,
            options,
            &mut command_rx,
        )
        .await
    }
}
