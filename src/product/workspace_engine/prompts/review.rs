use super::*;

impl WorkspaceEngine {
    pub(crate) fn build_review_input(&self) -> Result<StreamingProviderInput, String> {
        if matches!(self.session.workspace_type, WorkspaceType::WorkItemPlan) {
            return self.build_work_item_plan_review_input();
        }

        let working_dir = match &self.session.repository_path {
            Some(path) => path.clone(),
            None => std::env::current_dir()
                .map_err(|error| format!("working directory error: {error}"))?,
        };

        let artifact = self
            .session
            .artifact
            .clone()
            .map(|payload| payload.into_markdown().unwrap_or_default())
            .unwrap_or_default();
        let provider = self
            .session
            .reviewer_provider
            .clone()
            .unwrap_or(ProviderName::Codex);
        let mut prompt = String::new();
        prompt.push_str("请作为 reviewer 审核当前 Workspace 产物。\n\n");
        prompt.push_str(&format!(
            "Workspace 类型: {}\n",
            workspace_type_title(&self.session.workspace_type)
        ));
        prompt.push_str("会话上下文:\n");
        for msg in &self.session.messages {
            if matches!(msg.role.as_str(), "assistant" | "provider") {
                continue;
            }
            prompt.push_str(&format!("[{}]: {}\n", msg.role, msg.content));
        }
        self.append_missing_context_notes_to_prompt(&mut prompt);
        prompt.push_str("\n当前已提取 Artifact Markdown（daemon 已剥离外层 artifact fence）:\n\n");
        prompt.push_str(&artifact);
        prompt.push_str(
            "\n\n审核边界说明：当前 Artifact 是 daemon 从 author 原始输出中提取后的 markdown，外层 artifact fence 已被剥离是正常状态。\
             不要因为当前 Artifact 未包含外层 artifact fence 判定返修；只审核 markdown 内部一级标题、必需 heading、稳定 ID、追踪关系、内容完整性和设计质量。\
             如果 markdown 正文内部的代码块未闭合或内容结构不合规，仍可按实际问题要求返修。\n",
        );
        let nonce = structured_output_nonce();
        prompt.push_str(&reviewer_output_contract(
            &nonce,
            r#"{"verdict":"pass|revise|needs_human","summary":"一句话摘要","findings":[{"severity":"blocking|must_fix|strong_recommend_fix|suggestion|minor|optional","message":"问题描述","evidence":"当前产物中的具体证据","impact":"为什么影响或不影响下一阶段","required_action":"需要作者执行的最小动作"}]}"#,
            "\n\n请输出审核意见；可以先输出简短可读说明，最终 JSON 必须放在 nonce sentinel block 中，不得使用 Markdown code fence：\n\
             - 只有影响下一阶段可用性的 finding 才能标记为 `blocking`、`must_fix` 或 `strong_recommend_fix`。\n\
             - 风格、措辞、文档美化、未来扩展、非必要补充只能标记为 `suggestion`、`minor` 或 `optional`。\n\
             - 没有强返修 finding 时，必须允许用户确认当前版本，不要为了普通建议使用强返修。\n\
             - 如果输出 `verdict=revise`，必须给出至少一个结构化 finding；否则系统会进入人工裁决而不是自动返修。\n\
             - 第二轮及后续 review 只复核上一轮强返修项是否关闭；除非 revision 新引入真正阻塞问题，不得重新发散普通建议。\n\
             - `pass`：产物可进入最终人工确认。\n\
             - `revise`：仅当存在 blocking/must_fix/strong_recommend_fix finding。\n\
             - `needs_human`：没有明确可自动返修内容，需要用户做产品/范围判断。\n",
        ));

        Ok(StreamingProviderInput {
            provider_type: provider_type_for_name(&provider),
            role: AdapterRole::Reviewer,
            prompt,
            working_dir,
            workspace_session_id: Some(self.session.session_id.clone()),
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Supervised,
            env_vars: BTreeMap::new(),
            timeout_secs: DEFAULT_PROVIDER_TIMEOUT_SECS,
        })
    }

    pub(crate) fn build_work_item_plan_review_input(
        &self,
    ) -> Result<StreamingProviderInput, String> {
        if self.active_node_type() == Some(TimelineNodeType::WorkItemBatchReview) {
            return self.build_work_item_batch_review_input();
        }

        if self.active_node_type() == Some(TimelineNodeType::WorkItemDraftReview) {
            let draft_candidate = self.current_work_item_draft_candidate_payload()?;
            return self.build_work_item_draft_review_input(&draft_candidate);
        }

        if let Some(ArtifactPayload::WorkItemPlanOutlineCandidate { outline_candidate }) =
            self.session.artifact.as_ref()
        {
            return self.build_work_item_plan_outline_review_input(outline_candidate);
        }

        let lifecycle = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable for work_item_plan review".to_string())?;
        let candidate = build_work_item_plan_candidate_dto(
            lifecycle,
            &self.session.project_id,
            &self.session.issue_id,
            &self.session.entity_id,
        )
        .map_err(|error| format!("build work_item_plan candidate dto failed: {error}"))?;

        let working_dir = match &self.session.repository_path {
            Some(path) => path.clone(),
            None => std::env::current_dir()
                .map_err(|error| format!("working directory error: {error}"))?,
        };
        let provider = self
            .session
            .reviewer_provider
            .clone()
            .unwrap_or(ProviderName::Codex);

        let mut prompt = String::new();
        prompt
            .push_str("请作为 reviewer 审核当前 WorkItemPlan 候选（整组 WorkItem 拆分计划）。\n\n");
        prompt.push_str(&format!(
            "Workspace 类型: {}\n",
            workspace_type_title(&self.session.workspace_type)
        ));
        prompt.push_str("会话上下文:\n");
        for msg in &self.session.messages {
            if matches!(msg.role.as_str(), "assistant" | "provider") {
                continue;
            }
            prompt.push_str(&format!("[{}]: {}\n", msg.role, msg.content));
        }
        self.append_missing_context_notes_to_prompt(&mut prompt);

        prompt.push_str("\n## 待审核候选\n\n");
        prompt.push_str(&format!(
            "### Plan\n- id: {}\n- status: {}\n",
            candidate.plan.id, candidate.plan.status
        ));
        prompt.push_str(&format!(
            "- options: include_integration_tests={}, include_e2e_tests={}, force_frontend_backend_split={}, require_execution_plan_confirm={}\n",
            candidate.plan.options.include_integration_tests,
            candidate.plan.options.include_e2e_tests,
            candidate.plan.options.force_frontend_backend_split,
            candidate.plan.options.require_execution_plan_confirm,
        ));

        prompt.push_str("\n### WorkItems\n");
        for wi in &candidate.work_items {
            prompt.push_str(&format!(
                "\n- id: {}\n  kind: {}\n  title: {}\n  depends_on: [{}]\n  exclusive_write_scopes: [{}]\n  verification_plan_ref: {}\n",
                wi.id,
                wi.kind,
                wi.title,
                wi.depends_on.join(", "),
                wi.exclusive_write_scopes.join(", "),
                wi.verification_plan_ref.as_deref().unwrap_or("(none)"),
            ));
        }

        prompt.push_str("\n### dependency_graph\n");
        if candidate.plan.dependency_graph.is_empty() {
            prompt.push_str("(empty)\n");
        } else {
            for edge in &candidate.plan.dependency_graph {
                prompt.push_str(&format!(
                    "- {} -> {}\n",
                    edge.from_work_item_id, edge.to_work_item_id
                ));
            }
        }

        prompt.push_str("\n### validator_findings\n");
        if candidate.validator_findings.is_empty() {
            prompt.push_str("(none)\n");
        } else {
            for finding in &candidate.validator_findings {
                prompt.push_str(&format!(
                    "- [{}] {}: {} (work_items: [{}])\n",
                    finding.severity,
                    finding.code,
                    finding.message,
                    finding.work_item_ids.join(", "),
                ));
            }
        }

        prompt.push_str("\n### Repository Profile (trimmed)\n");
        if let Some(rp) = &candidate.repository_profile {
            prompt.push_str(&format!(
                "- confidence: {}\n- detected_layers: [{}]\n",
                rp.confidence,
                rp.detected_layers.join(", "),
            ));
        } else {
            prompt.push_str("(none)\n");
        }

        prompt.push_str("\n### Verification Plans (summary)\n");
        if candidate.verification_plans.is_empty() {
            prompt.push_str("(none)\n");
        } else {
            for vp in &candidate.verification_plans {
                prompt.push_str(&format!(
                    "- plan_ref: {} | scope: {} | commands: {} | manual_checks: {}\n",
                    vp.plan_ref,
                    vp.scope,
                    vp.commands.len(),
                    vp.manual_checks.len(),
                ));
            }
        }

        prompt.push_str(
            "\n\n审核边界说明：本候选是 WorkItemPlan 整组拆分计划，请从以下维度评估：\
             1) 拆分粒度合理性（是否过粗或过细）；\
             2) 依赖完整性（DAG 是否无环、depends_on 指向存在的 work_item）；\
             3) 写入范围互斥（exclusive_write_scopes 之间无重叠）；\
             4) 跨端拆分恰当性（前端/后端/全栈划分是否合理）；\
             5) 验证计划覆盖度（每个 work_item 的 verification_plan_ref 是否存在、scope 是否匹配）。\
             不要因为 verification_plans 摘要未展开 commands 判定返修；只审核上述五个维度。\n",
        );
        let nonce = structured_output_nonce();
        prompt.push_str(&reviewer_output_contract(
            &nonce,
            r#"{"verdict":"pass|revise|needs_human","summary":"一句话摘要","findings":[{"severity":"blocking|must_fix|strong_recommend_fix|suggestion|minor|optional","message":"问题描述","evidence":"当前产物中的具体证据","impact":"为什么影响或不影响下一阶段","required_action":"需要作者执行的最小动作"}]}"#,
            "\n\n请输出审核意见；可以先输出简短可读说明，最终 JSON 必须放在 nonce sentinel block 中，不得使用 Markdown code fence：\n\
             - 只有影响下一阶段可用性的 finding 才能标记为 `blocking`、`must_fix` 或 `strong_recommend_fix`。\n\
             - 风格、措辞、文档美化、未来扩展、非必要补充只能标记为 `suggestion`、`minor` 或 `optional`。\n\
             - 没有强返修 finding 时，必须允许用户确认当前版本，不要为了普通建议使用强返修。\n\
             - 如果输出 `verdict=revise`，必须给出至少一个结构化 finding；否则系统会进入人工裁决而不是自动返修。\n\
             - 第二轮及后续 review 只复核上一轮强返修项是否关闭；除非 revision 新引入真正阻塞问题，不得重新发散普通建议。\n\
             - `pass`：产物可进入最终人工确认。\n\
             - `revise`：仅当存在 blocking/must_fix/strong_recommend_fix finding。\n\
             - `needs_human`：没有明确可自动返修内容，需要用户做产品/范围判断。\n",
        ));

        Ok(StreamingProviderInput {
            provider_type: provider_type_for_name(&provider),
            role: AdapterRole::Reviewer,
            prompt,
            working_dir,
            workspace_session_id: Some(self.session.session_id.clone()),
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Supervised,
            env_vars: BTreeMap::new(),
            timeout_secs: DEFAULT_PROVIDER_TIMEOUT_SECS,
        })
    }

    pub(crate) fn build_work_item_plan_outline_review_input(
        &self,
        outline_candidate: &WorkItemPlanOutlineCandidateDto,
    ) -> Result<StreamingProviderInput, String> {
        let working_dir = match &self.session.repository_path {
            Some(path) => path.clone(),
            None => std::env::current_dir()
                .map_err(|error| format!("working directory error: {error}"))?,
        };
        let provider = self
            .session
            .reviewer_provider
            .clone()
            .unwrap_or(ProviderName::Codex);
        let generation_round_id = self
            .work_item_plan_store()
            .ok()
            .and_then(|store| {
                store
                    .load_active_index(
                        &self.session.project_id,
                        &self.session.issue_id,
                        &self.session.entity_id,
                    )
                    .ok()
                    .flatten()
            })
            .map(|index| index.current_generation_round_id)
            .unwrap_or_else(|| "generation_round_unknown".to_string());

        let outline = &outline_candidate.outline;
        let mut prompt = String::new();
        prompt.push_str("请作为 reviewer 审核当前 WorkItemPlan Outline。\n\n");
        prompt.push_str("审核对象只是 Outline 阶段的拆分方案，不是完整 Work Item，不得要求完整 verification plan、required_gates 或 repository_profile。\n\n");
        prompt.push_str(&format!(
            "Workspace 类型: {}\n",
            workspace_type_title(&self.session.workspace_type)
        ));
        prompt.push_str("会话上下文:\n");
        for msg in &self.session.messages {
            if matches!(msg.role.as_str(), "assistant" | "provider") {
                continue;
            }
            prompt.push_str(&format!("[{}]: {}\n", msg.role, msg.content));
        }
        self.append_missing_context_notes_to_prompt(&mut prompt);

        prompt.push_str("\n## Design context gaps\n");
        if outline_candidate.design_context_gaps.is_empty() {
            prompt.push_str("(none)\n");
        } else {
            for gap in &outline_candidate.design_context_gaps {
                prompt.push_str(&format!("- {gap}\n"));
            }
        }

        prompt.push_str("\n## Validator findings\n");
        if outline_candidate.validator_findings.is_empty() {
            prompt.push_str("(none)\n");
        } else {
            for finding in &outline_candidate.validator_findings {
                prompt.push_str(&format!(
                    "- [{}] {}: {}\n",
                    finding.severity, finding.code, finding.message
                ));
            }
        }

        prompt.push_str("\n## Outline\n");
        prompt.push_str(&format!(
            "- id: {}\n- strategy_summary: {}\n- handoff_strategy: {}\n",
            outline.id, outline.strategy_summary, outline.handoff_strategy
        ));
        prompt.push_str("\n### Work item outlines\n");
        for item in &outline.work_item_outlines {
            prompt.push_str(&format!(
                "\n- outline_id: {}\n  title: {}\n  kind: {:?}\n  goal: {}\n  scope: [{}]\n  depends_on: [{}]\n  exclusive_write_scopes: [{}]\n  forbidden_write_scopes: [{}]\n  verification_intent: [{}]\n  handoff_notes: {}\n",
                item.outline_id,
                item.title,
                item.kind,
                item.goal,
                item.scope.join(", "),
                item.depends_on.join(", "),
                item.exclusive_write_scopes.join(", "),
                item.forbidden_write_scopes.join(", "),
                item.verification_intent.join(", "),
                item.handoff_notes,
            ));
        }
        prompt.push_str("\n### Dependency graph\n");
        if outline.dependency_graph.is_empty() {
            prompt.push_str("(empty)\n");
        } else {
            for edge in &outline.dependency_graph {
                prompt.push_str(&format!(
                    "- {} -> {}\n",
                    edge.from_outline_id, edge.to_outline_id
                ));
            }
        }
        prompt.push_str("\n### Risks\n");
        for risk in &outline.risks {
            prompt.push_str(&format!("- {risk}\n"));
        }

        prompt.push_str(
            "\n\n审核边界说明：请只检查拆分策略、覆盖 Story/Design、outline 粒度、依赖图、写入边界、上下文缺口补齐假设与 handoff 策略。\
             不要要求 author 在 Outline 阶段输出完整 Work Item 正文、完整 verification plan、required_gates 或 repository_profile。\
             如果问题会影响拆分边界，返回 `revise`；如果需要用户做产品/范围判断，返回 `needs_human`。\n",
        );
        let nonce = structured_output_nonce();
        let schema = format!(
            r#"{{"verdict":"pass|revise|needs_human","review_scope":"outline","generation_round_id":"{}","summary":"一句话摘要","affects_items":[{{"target_outline_id":"outline id"}}],"findings":[{{"severity":"blocking|must_fix|strong_recommend_fix|suggestion|minor|optional","message":"问题描述","evidence":"Outline 中的具体证据","impact":"为什么影响或不影响 Draft 生成","required_action":"需要 Outline author 执行的最小动作"}}]}}"#,
            generation_round_id
        );
        prompt.push_str(&reviewer_output_contract(
            &nonce,
            &schema,
            "\n\n请输出审核意见；可以先输出简短可读说明，最终 JSON 必须放在 nonce sentinel block 中，不得使用 Markdown code fence：\n\
             - `pass`：Outline 可进入生成模式选择。\n\
             - `revise`：Outline 需要返修，且必须给出至少一个 blocking/must_fix/strong_recommend_fix finding。\n\
             - `needs_human`：需要用户做产品/范围判断。\n\
             - `affects_items.target_outline_id` 只能引用当前 Outline 中存在的 outline_id。\n",
        ));

        Ok(StreamingProviderInput {
            provider_type: provider_type_for_name(&provider),
            role: AdapterRole::Reviewer,
            prompt,
            working_dir,
            workspace_session_id: Some(self.session.session_id.clone()),
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Supervised,
            env_vars: BTreeMap::new(),
            timeout_secs: DEFAULT_PROVIDER_TIMEOUT_SECS,
        })
    }

    pub(crate) fn build_work_item_batch_review_input(
        &self,
    ) -> Result<StreamingProviderInput, String> {
        let store = self.work_item_plan_store()?;
        let index = store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        let batch = current_work_item_batch(&index)?;
        let draft_records =
            self.batch_work_item_plan_draft_records(&store, &index, &batch.batch_id)?;
        let draft_json =
            serde_json::to_string_pretty(&draft_records).unwrap_or_else(|_| "[]".to_string());
        let outline_ids = self.current_work_item_plan_outline_ids();
        let provider = self
            .session
            .reviewer_provider
            .clone()
            .unwrap_or(ProviderName::Codex);
        let nonce = structured_output_nonce();
        let mut prompt = String::new();
        prompt
            .push_str("请作为 reviewer 审核 WorkItemPlan 自动模式生成的整组 Work Item Draft。\n\n");
        prompt.push_str(&format!(
            "generation_round_id: {}\nbatch_id: {}\n\n",
            batch.generation_round_id, batch.batch_id
        ));
        prompt.push_str("[batch_draft_records]\n");
        prompt.push_str(&draft_json);
        prompt.push_str("\n\n");
        prompt.push_str(&reviewer_output_contract(
            &nonce,
            r#"{"verdict":"pass|revise_batch|needs_human|plan_reopen_required","review_scope":"batch","generation_round_id":"round id","summary":"一句话摘要","affects_items":[{"target_outline_id":"outline id"}],"findings":[{"severity":"blocking|must_fix|strong_recommend_fix|suggestion|minor|optional","message":"问题描述","evidence":"整组 draft 或依赖上下文中的具体证据","impact":"为什么影响或不影响 final compile","required_action":"需要 batch author 执行的最小动作"}]}"#,
            "\n\n审核规则：自动模式只能整组通过、整组返修或要求重开 Outline；不得要求单项重写。最终 JSON 必须放在 nonce sentinel block 中。\n",
        ));
        prompt.push_str(&format!(
            "\n[valid_outline_ids]\n{}\n",
            outline_ids.join("\n")
        ));
        let working_dir = self
            .session
            .repository_path
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .ok_or_else(|| "working directory unavailable".to_string())?;
        Ok(StreamingProviderInput {
            provider_type: provider_type_for_name(&provider),
            role: AdapterRole::Reviewer,
            prompt,
            working_dir,
            workspace_session_id: Some(self.session.session_id.clone()),
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Supervised,
            env_vars: BTreeMap::new(),
            timeout_secs: DEFAULT_PROVIDER_TIMEOUT_SECS,
        })
    }

    pub(crate) fn build_work_item_draft_review_input(
        &self,
        draft_candidate: &WorkItemDraftCandidatePayload,
    ) -> Result<StreamingProviderInput, String> {
        let working_dir = match &self.session.repository_path {
            Some(path) => path.clone(),
            None => std::env::current_dir()
                .map_err(|error| format!("working directory error: {error}"))?,
        };
        let provider = self
            .session
            .reviewer_provider
            .clone()
            .unwrap_or(ProviderName::Codex);
        let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
        let current_outline = outline_candidate
            .outline
            .work_item_outlines
            .iter()
            .find(|outline| outline.outline_id == draft_candidate.draft_record.outline_id)
            .ok_or_else(|| {
                format!(
                    "outline {} not found for draft review",
                    draft_candidate.draft_record.outline_id
                )
            })?;
        let store = self.work_item_plan_store()?;
        let index = store
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        let accepted_drafts = self.accepted_work_item_plan_draft_records(&store, &index)?;

        let mut prompt = String::new();
        prompt.push_str("请作为 reviewer 审核当前单个 Work Item Draft。\n\n");
        prompt.push_str("审核边界：只能审核当前 draft 是否符合对应 outline 以及是否正确消费已接受依赖。若需要修改当前 item，返回 `revise`；若需要修改前序 item 或拆分边界，必须返回 `plan_reopen_required`；不得用 `revise` 修改非当前 item。\n\n");
        prompt.push_str(&format!(
            "generation_round_id: {}\ndraft_id: {}\ntarget_outline_id: {}\n\n",
            draft_candidate.draft_record.generation_round_id,
            draft_candidate.draft_record.draft_id,
            draft_candidate.draft_record.outline_id
        ));
        prompt.push_str("## Current outline\n");
        prompt.push_str(
            &serde_json::to_string_pretty(current_outline)
                .map_err(|error| format!("serialize current outline failed: {error}"))?,
        );
        prompt.push_str("\n\n## Current draft\n");
        prompt.push_str(
            &serde_json::to_string_pretty(&draft_candidate.draft_record.candidate)
                .map_err(|error| format!("serialize current draft failed: {error}"))?,
        );
        prompt.push_str("\n\n## Local validator findings\n");
        if draft_candidate.validator_findings.is_empty() {
            prompt.push_str("(none)\n");
        } else {
            for finding in &draft_candidate.validator_findings {
                prompt.push_str(&format!(
                    "- [{}] {}: {}\n",
                    finding.severity, finding.code, finding.message
                ));
            }
        }
        prompt.push_str("\n## Accepted previous drafts\n");
        if accepted_drafts.is_empty() {
            prompt.push_str("(none)\n");
        } else {
            for record in &accepted_drafts {
                prompt.push_str(&format!(
                    "- outline_id: {}\n  draft_id: {}\n  title: {}\n  handoff_summary: {}\n  exclusive_write_scopes: [{}]\n",
                    record.outline_id,
                    record.draft_id,
                    record.candidate.title,
                    record.candidate.handoff_summary,
                    record.candidate.exclusive_write_scopes.join(", ")
                ));
            }
        }

        let nonce = structured_output_nonce();
        let schema = format!(
            r#"{{"verdict":"pass|revise|needs_human|plan_reopen_required","review_scope":"item","target_outline_id":"{}","generation_round_id":"{}","draft_id":"{}","summary":"一句话摘要","affects_items":[{{"target_outline_id":"{}"}}],"findings":[{{"severity":"blocking|must_fix|strong_recommend_fix|suggestion|minor|optional","message":"问题描述","evidence":"当前 draft 或依赖上下文中的具体证据","impact":"为什么影响或不影响后续生成","required_action":"需要当前 item author 执行的最小动作"}}]}}"#,
            draft_candidate.draft_record.outline_id,
            draft_candidate.draft_record.generation_round_id,
            draft_candidate.draft_record.draft_id,
            draft_candidate.draft_record.outline_id
        );
        prompt.push_str(&reviewer_output_contract(
            &nonce,
            &schema,
            "\n\n请输出审核意见；可以先输出简短可读说明，最终 JSON 必须放在 nonce sentinel block 中，不得使用 Markdown code fence：\n\
             - `pass`：当前 draft 可进入下一项；只允许没有 blocking/must_fix/strong_recommend_fix finding，或只有 suggestion/minor/optional finding。\n\
             - 不要输出 `verdict=pass` 同时给出 blocking/must_fix/strong_recommend_fix finding；这类输出会被系统判定为需要返修。\n\
             - `revise`：只允许重写当前 target_outline_id 对应的 draft；如果问题只需当前 item author 修改，必须返回 `revise`。\n\
             - `plan_reopen_required`：需要修改前序 item、拆分边界或 Outline 依赖。\n\
             - `needs_human`：需要用户做范围或产品判断。\n",
        ));

        Ok(StreamingProviderInput {
            provider_type: provider_type_for_name(&provider),
            role: AdapterRole::Reviewer,
            prompt,
            working_dir,
            workspace_session_id: Some(self.session.session_id.clone()),
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Supervised,
            env_vars: BTreeMap::new(),
            timeout_secs: DEFAULT_PROVIDER_TIMEOUT_SECS,
        })
    }
}
