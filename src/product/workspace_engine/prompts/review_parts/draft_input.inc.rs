impl WorkspaceEngine {
    pub(crate) fn build_work_item_draft_review_input(
        &self,
        draft_candidate: &WorkItemDraftCandidatePayload,
    ) -> Result<StreamingProviderInput, String> {
        let (working_dir, working_directory) = self.review_launch_target_and_cwd()?;
        // C2 Task 5（REQ-CRO-05）：reviewer 缺失（空 effective）fail-closed——
        // 绝不以 Codex 顶替构建 reviewer prompt。
        let provider = self
            .session
            .reviewer_provider
            .clone()
            .ok_or_else(|| "reviewer_configuration_missing".to_string())?;
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
        prompt.push_str(&reviewer_boundary_rules_for(&self.session.workspace_type));
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
                let promised_contracts = record
                    .candidate
                    .canonical_contract_candidate
                    .output_contracts
                    .iter()
                    .map(|contract| contract.contract_id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                prompt.push_str(&format!(
                    "- outline_id: {}\n  draft_id: {}\n  logical_work_item_id: {}\n  title: {}\n  output_contracts: {}\n  exclusive_write_scopes: [{}]\n",
                    record.outline_id,
                    record.draft_id,
                    record.candidate.logical_work_item_id,
                    record.candidate.canonical_contract_candidate.identity.title,
                    promised_contracts,
                    record
                        .candidate
                        .canonical_contract_candidate
                        .write_policy
                        .exclusive_scopes
                        .join(", ")
                ));
            }
        }
        let nonce = structured_output_nonce();
        let structured_output_contract = StructuredOutputContract {
            nonce: nonce.clone(),
            schema_name: "work_item_plan_item_review".to_string(),
        };
        let schema = format!(
            r#"{{"verdict":"pass|revise|needs_human|plan_reopen_required","review_scope":"item","target_outline_id":"{}","generation_round_id":"{}","draft_id":"{}","summary":"一句话摘要","affects_items":[{{"target_outline_id":"{}"}}],"findings":[{{"severity":"blocking|must_fix|suggestion","message":"问题描述（含影响）","evidence":"当前 draft 或依赖上下文中的具体证据","required_action":"需要当前 item author 执行的最小动作"}}]}}"#,
            draft_candidate.draft_record.outline_id,
            draft_candidate.draft_record.generation_round_id,
            draft_candidate.draft_record.draft_id,
            draft_candidate.draft_record.outline_id
        );
        prompt.push_str(&reviewer_output_contract(
            &nonce,
            &schema,
            "\n\n请输出审核意见；可以先输出简短可读说明，最终 JSON 必须放在 nonce sentinel block 中，不得使用 Markdown code fence：\n\
             - `pass`：当前 draft 可进入下一项；只允许没有 blocking/must_fix finding，或只有 suggestion finding。\n\
             - 不要输出 `verdict=pass` 同时给出 blocking/must_fix finding；这类输出会被系统判定为需要返修。\n\
             - `revise`：只允许重写当前 target_outline_id 对应的 draft；如果问题只需当前 item author 修改，必须返回 `revise`。\n\
             - `plan_reopen_required`：需要修改前序 item、拆分边界或 Outline 依赖。\n\
             - `needs_human`：需要用户做范围或产品判断。\n",
            &self.routing_reference_context(),
        ));
        let baseline_tree = self.append_reviewer_baseline_teaching(&mut prompt)?;
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
            structured_output_contract: Some(structured_output_contract),
            env_vars: BTreeMap::new(),
            timeout_secs: DEFAULT_PROVIDER_TIMEOUT_SECS,
        })
    }
}
