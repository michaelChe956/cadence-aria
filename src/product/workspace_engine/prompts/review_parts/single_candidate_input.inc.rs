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

        // SG3a.1（plan §0.6.2）：同代结构报告 join——Some payload 对 live
        // Design/member/options/baseline 事实复算同 context digest/rule，
        // missing/漂移即旧 context 拒发（不用另一个报告补「合法」；
        // store 已在 get 时核过 source/IR/compiler/report_digest 绑定）。
        // None 是旧记录参考形态（可展示/参考，不冒新合法授权），
        // 不注入三可信 section，保持既有输入面零变化。
        if let Some(structure) = report_record.structure_report.as_ref() {
            let source_ref_binding = self
                .session
                .work_item_plan_source_revision_ref
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    "single-candidate review structure context missing: durable work_item_plan_source_revision_ref is required to verify the same-generation structure report"
                        .to_string()
                })?;
            let live_context = self
                .build_single_candidate_structure_context(&ir_record.ir, source_ref_binding, ir_ref)
                .map_err(|error| {
                    format!("single-candidate review structure context unavailable: {error}")
                })?;
            if structure.context_digest.trim().is_empty() {
                return Err(format!(
                    "single-candidate review structure context missing: payload (report_ref {report_ref}) carries no context_digest; a launch without same-generation structure evidence is refused"
                ));
            }
            if structure.rule_version != live_context.rule_version {
                return Err(format!(
                    "single-candidate review structure rule stale: payload {} != current {}; refs must be re-frozen by a new legal candidate round",
                    structure.rule_version, live_context.rule_version
                ));
            }
            if structure.context_digest != live_context.context_digest {
                return Err(format!(
                    "single-candidate review structure context stale: payload context_digest {} != current context_digest {}; another report cannot re-authorize these refs",
                    structure.context_digest, live_context.context_digest
                ));
            }
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

        // SG3a.1 三可信 section（plan §0.6.2）：只序列化同一 record 的 Some
        // 结构 payload——实际来源边/仓序/target、身份绑定与修复审计。
        // 不新增 LLM 拓扑计算权威（reviewer 不重算图）；None 旧记录跳过。
        if let Some(structure) = &report_record.structure_report {
            let source_ref_identity = self
                .session
                .work_item_plan_source_revision_ref
                .clone()
                .unwrap_or_default();
            append_review_context_section(
                &mut prompt,
                "Deterministic Structure Report",
                &json!({
                    "effective_edges": structure.effective_edges,
                    "cycle_witness": structure.cycle_witness,
                    "findings": structure.findings,
                    "structurally_valid": structure.structurally_valid,
                    "protected_predicates": structure.protected_predicates,
                }),
            )?;
            append_review_context_section(
                &mut prompt,
                "Structure Context Identity",
                &json!({
                    "source_revision_ref": source_ref_identity,
                    "plan_candidate_ir_ref": ir_ref,
                    "mechanical_report_ref": report_ref,
                    "source_hash": structure.source_hash,
                    "context_digest": structure.context_digest,
                    "rule_version": structure.rule_version,
                    "report_digest": structure.report_digest,
                }),
            )?;
            append_review_context_section(
                &mut prompt,
                "Structural Repair Audit",
                &json!({
                    "instruction_refs": structure.instruction_refs,
                    "repair_audit_refs": structure.repair_audit_refs,
                }),
            )?;
            prompt.push_str(
                "结构修复审计说明：初轮合法候选无修复指令与审计引用（初轮无修复明确空，不伪模型许可）。\n",
            );
        }
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
            r#"{{"verdict":"pass|revise|needs_human","review_scope":"outline","generation_round_id":"{}","summary":"一句话摘要","findings":[{{"severity":"blocking|must_fix|suggestion","message":"问题描述（含影响）","evidence":"具体证据","required_action":"需要作者执行的最小动作","owner_question":"面向用户的一句话提问（仅 needs_human 必填）","owner_options":[{{"label":"选项名（人话）","consequence":"选它之后会发生什么"}}]}}]}}"#,
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
            // 批2 Task 11(T20 前中间态):Reviewer wire 面全开(无 deny);
            // 纪律=ReviewerVerify+窗口合同,真窗归 T20;执行审计按角色保留。
            // 批2 Task 20b(决策项⑦):Reviewer 输入铸造 ReviewerVerify 窗口
            // 合同(活动审查窗的窗口 id;无活动窗=None,由四家启动检查/
            // 网关复验拒绝,不 fallback)。
            tool_policy: self.reviewer_verify_policy(),
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
