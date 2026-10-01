impl WorkspaceEngine {
    pub(crate) fn build_single_candidate_plan_review_input(
        &self,
    ) -> Result<StreamingProviderInput, String> {
        let lifecycle = self.lifecycle_store.as_ref().ok_or_else(|| {
            "lifecycle_store unavailable for single-candidate work_item_plan review".to_string()
        })?;
        let missing_refs = [
            self.session
                .plan_candidate_ir_ref
                .as_ref()
                .filter(|value| !value.trim().is_empty())
                .is_none()
                .then_some("plan_candidate_ir_ref"),
            self.session
                .mechanical_report_ref
                .as_ref()
                .filter(|value| !value.trim().is_empty())
                .is_none()
                .then_some("mechanical_report_ref"),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        if !missing_refs.is_empty() {
            return Err(format!(
                "single-candidate review requires durable {}",
                missing_refs.join(" and ")
            ));
        }
        let ir_ref = self
            .session
            .plan_candidate_ir_ref
            .as_deref()
            .expect("missing refs checked above");
        let report_ref = self
            .session
            .mechanical_report_ref
            .as_deref()
            .expect("missing refs checked above");
        let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
        let scope = SourceStoreScope {
            project_id: self.session.project_id.clone(),
            issue_id: self.session.issue_id.clone(),
            plan_id: self.session.entity_id.clone(),
        };
        let ir_record = source_store
            .get_plan_candidate_ir(&scope, ir_ref)
            .map_err(|error| {
                format!(
                    "load single-candidate plan_candidate_ir_ref failed ({}): {error:?}",
                    error.code()
                )
            })?;
        let report_record = source_store
            .get_mechanical_report(&scope, report_ref)
            .map_err(|error| {
                format!(
                    "load single-candidate mechanical_report_ref failed ({}): {error:?}",
                    error.code()
                )
            })?;
        if report_record.ir_id != ir_record.id
            || report_record.report.source_revision_hash != ir_record.ir.source_revision_hash
            || report_record.report.compiler_version != ir_record.ir.compiler_version
        {
            return Err(
                "single-candidate review durable IR and mechanical report bindings mismatch"
                    .to_string(),
            );
        }
        let contract_candidates = ir_record
            .ir
            .items
            .iter()
            .map(|item| {
                json!({
                    "target_repository_id": item.target_repository_id,
                    "identity": item.contract.identity,
                    "goal": item.contract.goal,
                    "tasks": item.contract.tasks,
                    "write_policy": item.contract.write_policy,
                    "acceptance_criteria": item.contract.acceptance_criteria,
                    "verification_checks": item.contract.verification_checks,
                    "depends_on": item.contract.depends_on,
                    "input_contracts": item.contract.input_contracts,
                    "output_contracts": item.contract.output_contracts,
                })
            })
            .collect::<Vec<_>>();
        let dependency_graph = single_candidate_dependency_graph(&ir_record.ir.items)?;
        let reviewer_coverage = single_candidate_reviewer_coverage(&ir_record.ir.items)?;
        let cross_item_contracts = json!({
            "supplies": ir_record.ir.items.iter().map(|item| json!({
                "work_item_id": item.contract.identity.logical_work_item_id,
                "provided_output_contracts": item.contract.output_contracts,
            })).collect::<Vec<_>>(),
            "demands": ir_record.ir.items.iter().map(|item| json!({
                "work_item_id": item.contract.identity.logical_work_item_id,
                "depends_on": item.contract.depends_on,
                "required_input_contracts": item.contract.input_contracts,
            })).collect::<Vec<_>>(),
        });
        let error_count = report_record
            .report
            .findings
            .iter()
            .filter(|finding| {
                finding.severity == crate::product::models::WorkItemSplitFindingSeverity::Error
            })
            .count();
        let warning_count = report_record.report.findings.len() - error_count;
        let mechanical_report = json!({
            "source_revision_hash": report_record.report.source_revision_hash,
            "compiler_version": report_record.report.compiler_version,
            "summary": {
                "error_count": error_count,
                "warning_count": warning_count,
            },
            "findings": report_record.report.findings,
        });
        let mut prompt = String::from(
            "请作为 Plan Reviewer 审核当前 Canonical Contract 与 Projection 候选。\n\n## Plan Review Context\n",
        );
        append_review_context_section(
            &mut prompt,
            "Canonical Contract Candidates",
            &contract_candidates,
        )?;
        append_review_context_section(&mut prompt, "Dependency Contract Graph", &dependency_graph)?;
        append_review_context_section(
            &mut prompt,
            "Reviewer Capability Coverage Projection",
            &reviewer_coverage,
        )?;
        append_review_context_section(
            &mut prompt,
            "Cross-Item Contract Supply / Demand",
            &cross_item_contracts,
        )?;
        append_review_context_section(
            &mut prompt,
            "Projection Validation Report",
            &mechanical_report,
        )?;
        append_review_context_section(
            &mut prompt,
            "Immutable Candidate Artifact Refs",
            &json!({
                "plan_candidate_ir_ref": ir_ref,
                "mechanical_report_ref": report_ref,
            }),
        )?;
        prompt.push_str(
            "\n审核边界：只审核 Plan Review Context 中的权威 canonical contract、依赖拓扑、跨 WorkItem 契约供需与机械校验摘要；不得把 session markdown 或 lifecycle legacy DTO 当作候选事实来源。\n\
             能力覆盖机械规则：当任一 coverage entry 的 `missing_capabilities` 非空时，必须返回 `severity=must_fix`、`category=contract_gap` 的 finding，并按现有 scope 约定提供 `class_hint=repairable`；`evidence` 必须明确指出具体 `from -> to` edge、`contract_id` 与缺失 capability，不得以 `pass` 或 advisory 掩盖该缺口。\n",
        );
        for coverage in reviewer_coverage
            .capability_coverage
            .iter()
            .filter(|coverage| !coverage.missing_capabilities.is_empty())
        {
            prompt.push_str(&format!(
                "- coverage gap evidence required: edge {} -> {}, contract_id `{}`, missing_capabilities [{}]\n",
                coverage.from,
                coverage.to,
                coverage.contract_id,
                coverage.missing_capabilities.join(", "),
            ));
        }
        append_single_candidate_contract_gap_teaching(&mut prompt, &reviewer_coverage);
        prompt.push_str(SINGLE_CANDIDATE_REVIEW_REPETITION_TEACHING);
        // REQ-TOP-04 场景 6（F-52 L2 前轮注入）：复评轮携带上一轮 finding 身份清单。
        if let Some(previous_round) = single_candidate_previous_round_findings_section(self, ir_ref)
        {
            prompt.push_str(&previous_round);
        }
        let nonce = structured_output_nonce();
        let contract = StructuredOutputContract {
            nonce: nonce.clone(),
            schema_name: "single_candidate_work_item_plan_review".to_string(),
        };
        let schema = format!(
            r#"{{"verdict":"pass|revise|needs_human","review_scope":"outline","generation_round_id":"{}","summary":"一句话摘要","findings":[]}}"#,
            ir_record.id
        );
        prompt.push_str(&reviewer_output_contract(
            &nonce,
            &schema,
            "\n只能在契约、依赖、供需匹配或机械校验影响发布时返回 revise；需要产品判断时返回 needs_human。",
            &self.routing_reference_context(),
        ));
        let baseline_tree = self.append_reviewer_baseline_teaching(&mut prompt)?;
        ensure_single_candidate_review_prompt_budget(&prompt)?;
        let (working_dir, working_directory) = self.review_launch_target_and_cwd()?;
        // C2 Task 5（REQ-CRO-05）：reviewer 缺失（空 effective）fail-closed——
        // 绝不以 Codex 顶替构建 reviewer prompt。
        let provider = self
            .session
            .reviewer_provider
            .clone()
            .ok_or_else(|| "reviewer_configuration_missing".to_string())?;
        Ok(StreamingProviderInput {
            working_directory,
            baseline_tree,
            tool_policy: Some(ProviderToolPolicy::deny_file_write_builtins()),
            audit_sink: None,
            provider_type: provider_type_for_name(&provider),
            role: AdapterRole::Reviewer,
            prompt,
            working_dir,
            workspace_session_id: Some(self.session.session_id.clone()),
            resume_provider_session_id: None,
            permission_mode: permission_mode_for_provider(
                &provider,
                self.session.permission_modes.reviewer.clone(),
            ),
            structured_output_contract: Some(contract),
            env_vars: BTreeMap::new(),
            timeout_secs: DEFAULT_PROVIDER_TIMEOUT_SECS,
        })
    }
}
