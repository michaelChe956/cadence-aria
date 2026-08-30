use super::history_compaction::{
    HistoryCompactionInput, HistoryCompactionMode, compact_history,
    render_intermediate_artifact_diffs, render_open_required_findings,
};
use super::review_context::{
    PlanReviewSource, append_review_context_section, load_plan_review_context,
};
use super::reviewer_context_filter::reviewer_context_content;
use super::*;
use crate::cross_cutting::structured_output::StructuredOutputContract;
use crate::product::models::PlanProjectionBundle;
use crate::product::work_item_plan_policy::{ReviewFindingCategory, ReviewInvocationScope};
use crate::product::work_item_plan_source_store::{SourceStoreScope, WorkItemPlanSourceStore};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};

fn single_candidate_dependency_graph(
    items: &[crate::product::work_item_plan_compiler::PlanCandidateItemIr],
) -> Result<serde_json::Value, String> {
    let mut item_ids = BTreeSet::new();
    for item in items {
        let item_id = item.contract.identity.logical_work_item_id.as_str();
        if item_id.trim().is_empty() {
            return Err(
                "single-candidate review IR contains an empty work item identity".to_string(),
            );
        }
        if !item_ids.insert(item_id.to_string()) {
            return Err(format!(
                "single-candidate review IR contains duplicate work item identity `{item_id}`"
            ));
        }
    }
    let mut remaining_dependencies = BTreeMap::new();
    let mut dependents = BTreeMap::<String, BTreeSet<String>>::new();
    let mut edges = Vec::new();
    for item in items {
        let item_id = item.contract.identity.logical_work_item_id.clone();
        let mut dependencies = BTreeSet::new();
        for dependency in &item.contract.depends_on {
            if !item_ids.contains(dependency) {
                return Err(format!(
                    "single-candidate review IR dependency `{dependency}` for `{item_id}` is missing"
                ));
            }
            if dependency == &item_id {
                return Err(format!(
                    "single-candidate review IR work item `{item_id}` depends on itself"
                ));
            }
            if dependencies.insert(dependency.clone()) {
                dependents
                    .entry(dependency.clone())
                    .or_default()
                    .insert(item_id.clone());
                edges.push(format!("{dependency} -> {item_id}"));
            }
        }
        remaining_dependencies.insert(item_id, dependencies);
    }
    edges.sort();
    let mut ready = remaining_dependencies
        .iter()
        .filter_map(|(item_id, dependencies)| dependencies.is_empty().then_some(item_id.clone()))
        .collect::<BTreeSet<_>>();
    let mut topological_order = Vec::with_capacity(items.len());
    while let Some(item_id) = ready.iter().next().cloned() {
        ready.remove(&item_id);
        topological_order.push(item_id.clone());
        for dependent in dependents.get(&item_id).into_iter().flatten() {
            let dependencies = remaining_dependencies
                .get_mut(dependent)
                .expect("dependent must be present in remaining dependency map");
            dependencies.remove(&item_id);
            if dependencies.is_empty() {
                ready.insert(dependent.clone());
            }
        }
    }
    if topological_order.len() != items.len() {
        return Err("single-candidate review IR dependency graph contains a cycle".to_string());
    }
    Ok(json!({
        "topological_order": topological_order,
        "edges": edges,
    }))
}

fn review_finding_category_whitelist() -> String {
    [
        ReviewFindingCategory::ContractGap,
        ReviewFindingCategory::SelfContradiction,
        ReviewFindingCategory::ScopeConflict,
        ReviewFindingCategory::VerificationUnattributable,
        ReviewFindingCategory::Completeness,
        ReviewFindingCategory::Other,
    ]
    .into_iter()
    .map(ReviewFindingCategory::as_str)
    .collect::<Vec<_>>()
    .join("、")
}
/// 根据服务端持久化的 invocation scope 生成 reviewer 的范围指令。
///
/// scope 是协议边界的一部分：provider 只能消费这里生成的指令，不能在请求中
/// 自行扩大或替换审核范围。digest 校验失败以及 Verification 缺少机械报告均
/// fail-closed，避免把不完整的范围交给 provider。
pub(crate) fn review_scope_instructions(scope: &ReviewInvocationScope) -> Result<String, String> {
    scope
        .validate_digest()
        .map_err(|error| format!("review invocation scope digest invalid: {error}"))?;
    let category_whitelist = review_finding_category_whitelist();
    match scope {
        ReviewInvocationScope::Initial {
            initial_revision_id,
            scope_digest,
        } if initial_revision_id.trim().is_empty() => {
            Err("initial review scope requires an immutable revision".to_string())
        }
        ReviewInvocationScope::Initial {
            initial_revision_id,
            scope_digest,
        } => Ok(format!(
            "\n## 服务端审核 invocation scope（Initial）\n\
             - immutable initial revision: {initial_revision_id}\n\
             - scope digest: {scope_digest}\n\
             - 只允许一次全候选评估；不得自行增加候选、范围或 provider/campaign 指令。\n\
             - must_fix 仅限机械漏网硬错误或明确自相矛盾；完备度意见只能是 advisory。\n\
             - 每个 finding 必须提供 category 与 class_hint 建议；最终分类由服务端策略层决定。\n\
             - 每个 finding 对象只能包含以下字段：severity、message、evidence（可选）、required_action（可选）、category、class_hint、contract_field（可选）——不得添加 finding_id、code、work_item_ids 或其他字段。\n\
             - category 只能取以上六值之一；无法归类时用 other。合法值：{category_whitelist}。\n\
             - severity 只能取三值之一：blocking（阻断发布）、must_fix（必须修复）、suggestion（建议）——不得使用 error/warning 等其他词。\n\
             - class_hint 只能取三值之一：repairable（可自动返修）、human_required（需人工裁决）、advisory（仅建议）。\n",
        )),
        ReviewInvocationScope::Verification {
            original_fingerprints,
            repaired_revision_id,
            mechanical_report_ref,
            scope_digest,
        } if repaired_revision_id.trim().is_empty() => {
            Err("verification review scope requires an immutable repaired revision".to_string())
        }
        ReviewInvocationScope::Verification {
            original_fingerprints,
            repaired_revision_id,
            mechanical_report_ref,
            scope_digest,
        } if mechanical_report_ref.trim().is_empty() => {
            Err("verification review scope requires a mechanical report".to_string())
        }
        ReviewInvocationScope::Verification {
            original_fingerprints,
            repaired_revision_id,
            mechanical_report_ref,
            scope_digest,
        } => {
            let fingerprints = original_fingerprints
                .iter()
                .map(|fingerprint| fingerprint.0.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            Ok(format!(
                "\n## 服务端审核 invocation scope（Verification）\n\
                 - immutable repaired revision: {repaired_revision_id}\n\
                 - immutable mechanical report: {mechanical_report_ref}\n\
                 - scope digest: {scope_digest}\n\
                 - 仅复核原 fingerprints，不得将新 finding 伪装为原 finding：[{fingerprints}]\n\
                 - mechanical report 是本次 invocation 的唯一机械证据来源。\n\
                 - must_fix 仅限机械漏网硬错误或明确自相矛盾；完备度意见只能是 advisory。\n\
                 - 每个 finding 必须提供 category 与 class_hint 建议；最终分类由服务端策略层决定。\n\
                 - 每个 finding 对象只能包含以下字段：severity、message、evidence（可选）、required_action（可选）、category、class_hint、contract_field（可选）——不得添加 finding_id、code、work_item_ids 或其他字段。\n\
                 - category 只能取以上六值之一；无法归类时用 other。合法值：{category_whitelist}。\n\
                 - severity 只能取三值之一：blocking（阻断发布）、must_fix（必须修复）、suggestion（建议）——不得使用 error/warning 等其他词。\n\
                 - class_hint 只能取三值之一：repairable（可自动返修）、human_required（需人工裁决）、advisory（仅建议）。\n",
            ))
        }
    }
}

impl WorkspaceEngine {
    pub(crate) fn build_review_input(&self) -> Result<StreamingProviderInput, String> {
        if matches!(self.session.workspace_type, WorkspaceType::WorkItemPlan) {
            let mut input = self.build_work_item_plan_review_input()?;
            if self.session.flow_kind
                == crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate
            {
                let scope = self
                    .session
                    .review_invocation_scope
                    .as_ref()
                    .ok_or_else(|| {
                        "single-candidate review invocation scope is not durable".to_string()
                    })?;
                input.prompt.push_str(&review_scope_instructions(scope)?);
            }
            return Ok(input);
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
        prompt.push_str(&reviewer_boundary_rules_for(&self.session.workspace_type));
        if let Some(gate) = reviewer_artifact_schema_gate_for(&self.session.workspace_type) {
            prompt.push_str(&gate);
        }
        prompt.push_str("会话上下文（滑动窗口压缩；最近 2 轮保留原文）:\n");
        prompt.push_str(
            &compact_history(HistoryCompactionInput {
                messages: &self.session.messages,
                artifact_versions: &self.artifact_versions,
                timeline_nodes: &self.timeline_nodes,
                latest_review_verdict: self.latest_review_verdict.as_ref(),
                mode: HistoryCompactionMode::Reviewer,
            })
            .rendered,
        );
        self.append_missing_context_notes_to_prompt(&mut prompt);
        let artifact_diffs = render_intermediate_artifact_diffs(&self.artifact_versions);
        if !artifact_diffs.is_empty() {
            prompt.push_str("\n中间 Artifact 版本（相邻版本 diff 摘要；失败时已保留全文）:\n");
            prompt.push_str(&artifact_diffs);
        }
        let open_required_findings =
            render_open_required_findings(self.latest_review_verdict.as_ref());
        if !open_required_findings.is_empty() {
            prompt.push('\n');
            prompt.push_str(&open_required_findings);
        }
        prompt.push_str("\n当前已提取 Artifact Markdown（daemon 已剥离外层 artifact fence）:\n\n");
        prompt.push_str(&artifact);
        prompt.push_str(
            "\n\n审核边界说明：当前 Artifact 是 daemon 从 author 原始输出中提取后的 markdown，外层 artifact fence 已被剥离是正常状态。\
             不要因为当前 Artifact 未包含外层 artifact fence 判定返修；只审核 markdown 内部一级标题、必需 heading、稳定 ID、追踪关系、内容完整性和设计质量。\
             如果 markdown 正文内部的代码块未闭合或内容结构不合规，仍可按实际问题要求返修。\n",
        );
        if self.session.workspace_type == WorkspaceType::Design {
            prompt.push_str(
                crate::product::workspace_engine::prompts::reviewer_boundary_examples::design_reviewer_boundary_examples(),
            );
        }
        let nonce = structured_output_nonce();
        let structured_output_contract = StructuredOutputContract {
            nonce: nonce.clone(),
            schema_name: "workspace_review".to_string(),
        };
        prompt.push_str(&reviewer_output_contract(
            &nonce,
            r#"{"verdict":"pass|revise|needs_human","summary":"一句话摘要","findings":[{"severity":"blocking|must_fix|suggestion","message":"问题描述（含影响）","evidence":"当前产物中的具体证据","required_action":"需要作者执行的最小动作"}]}"#,
            "\n\n请输出审核意见；可以先输出简短可读说明，最终 JSON 必须放在 nonce sentinel block 中，不得使用 Markdown code fence：\n\
             - 只有影响下一阶段可用性的 finding 才能标记为 `blocking` 或 `must_fix`。\n\
             - 风格、措辞、文档美化、未来扩展、非必要补充只能标记为 `suggestion`。\n\
             - 没有强返修 finding 时，必须允许用户确认当前版本，不要为了普通建议使用强返修。\n\
             - 如果输出 `verdict=revise`，必须给出至少一个结构化 finding；否则系统会进入人工裁决而不是自动返修。\n\
             - 第二轮及后续 review 只复核上一轮强返修项是否关闭；除非 revision 新引入真正阻塞问题，不得重新发散普通建议。\n\
             - `pass`：产物可进入最终人工确认。\n\
             - `revise`：仅当存在 blocking/must_fix finding。\n\
             - `needs_human`：没有明确可自动返修内容，需要用户做产品/范围判断。\n",
            &self.routing_reference_context(),
        ));
        Ok(StreamingProviderInput {
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
        if let Some(ArtifactPayload::WorkItemPlanProjection { projection }) =
            self.session.artifact.as_ref()
        {
            return self.build_projection_plan_review_input(projection);
        }
        if self.session.flow_kind
            == crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate
        {
            return self.build_single_candidate_plan_review_input();
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
        let generation_round_id = self
            .work_item_plan_store()?
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .map(|index| index.current_generation_round_id)
            .unwrap_or_else(|| "legacy_work_item_plan_candidate".to_string());
        let mut prompt = String::new();
        prompt
            .push_str("请作为 reviewer 审核当前 WorkItemPlan 候选（整组 WorkItem 拆分计划）。\n\n");
        prompt.push_str(&format!(
            "Workspace 类型: {}\n",
            workspace_type_title(&self.session.workspace_type)
        ));
        prompt.push_str(&reviewer_boundary_rules_for(&self.session.workspace_type));
        prompt.push_str("会话上下文:\n");
        for msg in &self.session.messages {
            let Some(content) = reviewer_context_content(msg) else {
                continue;
            };
            prompt.push_str(&format!("[{}]: {content}\n", msg.role));
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
        let structured_output_contract = StructuredOutputContract {
            nonce: nonce.clone(),
            schema_name: "work_item_plan_review".to_string(),
        };
        let schema = format!(
            r#"{{"verdict":"pass|revise|needs_human","review_scope":"outline","generation_round_id":"{}","summary":"一句话摘要","findings":[{{"severity":"blocking|must_fix|suggestion","message":"问题描述（含影响）","evidence":"当前产物中的具体证据","required_action":"需要作者执行的最小动作"}}]}}"#,
            generation_round_id
        );
        prompt.push_str(&reviewer_output_contract(
            &nonce,
            &schema,
            "\n\n请输出审核意见；可以先输出简短可读说明，最终 JSON 必须放在 nonce sentinel block 中，不得使用 Markdown code fence：\n\
             - 只有影响下一阶段可用性的 finding 才能标记为 `blocking` 或 `must_fix`。\n\
             - 风格、措辞、文档美化、未来扩展、非必要补充只能标记为 `suggestion`。\n\
             - 没有强返修 finding 时，必须允许用户确认当前版本，不要为了普通建议使用强返修。\n\
             - 如果输出 `verdict=revise`，必须给出至少一个结构化 finding；否则系统会进入人工裁决而不是自动返修。\n\
             - 第二轮及后续 review 只复核上一轮强返修项是否关闭；除非 revision 新引入真正阻塞问题，不得重新发散普通建议。\n\
             - `pass`：产物可进入最终人工确认。\n\
             - `revise`：仅当存在 blocking/must_fix finding；语义为重开 Outline 并重新生成拆分。\n\
             - `needs_human`：没有明确可自动返修内容，需要用户做产品/范围判断。\n",
            &self.routing_reference_context(),
        ));
        Ok(StreamingProviderInput {
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

    fn build_single_candidate_plan_review_input(&self) -> Result<StreamingProviderInput, String> {
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
            "\n审核边界：只审核 Plan Review Context 中的权威 canonical contract、依赖拓扑、跨 WorkItem 契约供需与机械校验摘要；不得把 session markdown 或 lifecycle legacy DTO 当作候选事实来源。\n",
        );
        let nonce = structured_output_nonce();
        let contract = StructuredOutputContract {
            nonce: nonce.clone(),
            schema_name: "work_item_plan_review".to_string(),
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
        let working_dir = self
            .session
            .repository_path
            .clone()
            .map(Ok)
            .unwrap_or_else(|| std::env::current_dir().map_err(|error| error.to_string()))?;
        let provider = self
            .session
            .reviewer_provider
            .clone()
            .unwrap_or(ProviderName::Codex);
        Ok(StreamingProviderInput {
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

    fn build_projection_plan_review_input(
        &self,
        projection: &PlanProjectionBundle,
    ) -> Result<StreamingProviderInput, String> {
        let context =
            load_plan_review_context(self, projection, PlanReviewSource::for_engine(self))?;
        let mut prompt = String::from(
            "请作为 Plan Reviewer 审核当前 Canonical Contract 与 Projection 候选。\n\n## Plan Review Context\n",
        );
        append_review_context_section(
            &mut prompt,
            "Story / Design Traceability",
            &context.story_design_traceability,
        )?;
        append_review_context_section(
            &mut prompt,
            "Canonical Contract Candidates",
            &context.canonical_contract_candidates,
        )?;
        append_review_context_section(
            &mut prompt,
            "Dependency Contract Graph",
            &context.dependency_contract_graph,
        )?;
        append_review_context_section(
            &mut prompt,
            "PlanProjectionBundle Candidate",
            &context.plan_projection_bundle_candidate,
        )?;
        append_review_context_section(
            &mut prompt,
            "WorkItemProjectionBundle Candidates",
            &context.work_item_projection_bundle_candidates,
        )?;
        append_review_context_section(
            &mut prompt,
            "Projection Validation Report",
            &context.projection_validation_report,
        )?;
        append_review_context_section(&mut prompt, "Contract Delta", &context.contract_delta)?;
        append_review_context_section(&mut prompt, "Impact Analysis", &context.impact_analysis)?;
        append_review_context_section(&mut prompt, "Repair Evidence", &context.repair_evidence)?;
        if let Some(fingerprint) = &context.candidate_package_fingerprint {
            append_review_context_section(
                &mut prompt,
                "Candidate Package Fingerprint",
                fingerprint,
            )?;
        }
        if let Some(proposal) = &context.impact_scope_review {
            append_review_context_section(
                &mut prompt,
                "System Minimum Impact Scope",
                &proposal.system_minimum_impact_scope,
            )?;
            append_review_context_section(
                &mut prompt,
                "Proposed Accepted Impact Scope",
                &proposal.proposed_accepted_impact_scope,
            )?;
            append_review_context_section(
                &mut prompt,
                "Risk Acceptance Reason",
                &proposal.risk_acceptance_reason,
            )?;
            append_review_context_section(
                &mut prompt,
                "Candidate Package Fingerprint",
                &proposal.candidate_package_fingerprint,
            )?;
        }
        prompt.push_str(
            "\n审核边界：只审核 Plan Review Context 中的权威契约、依赖图、三 Projection 覆盖与影响范围；不得要求编码执行期差异或复用 Code Reviewer 执行证据。\n",
        );
        let generation_round_id = match &context.impact_scope_review {
            Some(proposal) => proposal.review_generation_round_id.clone(),
            None => self
                .work_item_plan_store()?
                .load_active_index(
                    &self.session.project_id,
                    &self.session.issue_id,
                    &self.session.entity_id,
                )
                .map_err(|error| format!("load work item plan active index failed: {error}"))?
                .map(|index| index.current_generation_round_id)
                .unwrap_or_else(|| projection.plan_revision_id.clone()),
        };
        let nonce = structured_output_nonce();
        let contract = StructuredOutputContract {
            nonce: nonce.clone(),
            schema_name: "work_item_plan_review".to_string(),
        };
        let schema = format!(
            r#"{{"verdict":"pass|revise|needs_human","review_scope":"outline","generation_round_id":"{}","summary":"一句话摘要","findings":[]}}"#,
            generation_round_id
        );
        prompt.push_str(&reviewer_output_contract(
            &nonce,
            &schema,
            "\n只能在契约、依赖或 Projection 覆盖影响发布时返回 revise；需要产品判断时返回 needs_human。",
            &self.routing_reference_context(),
        ));
        let working_dir = self
            .session
            .repository_path
            .clone()
            .map(Ok)
            .unwrap_or_else(|| std::env::current_dir().map_err(|error| error.to_string()))?;
        let provider = self
            .session
            .reviewer_provider
            .clone()
            .unwrap_or(ProviderName::Codex);
        Ok(StreamingProviderInput {
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
            .work_item_plan_store()?
            .load_active_index(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
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
        prompt.push_str(&reviewer_boundary_rules_for(&self.session.workspace_type));
        prompt.push_str("会话上下文:\n");
        for msg in &self.session.messages {
            let Some(content) = reviewer_context_content(msg) else {
                continue;
            };
            prompt.push_str(&format!("[{}]: {content}\n", msg.role));
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
        let outline_json = serde_json::to_string_pretty(outline)
            .map_err(|error| format!("serialize outline candidate error: {error}"))?;
        prompt.push_str("\n## Outline JSON (source of truth)\n");
        prompt.push_str(
            "以下 JSON 是审核事实来源，包含 author/rewriter 产出的完整 Outline 字段；如果后续可读摘要与 JSON 不一致，以 JSON 为准。\n",
        );
        prompt.push_str("<WORK_ITEM_PLAN_OUTLINE_JSON>\n");
        prompt.push_str(&outline_json);
        prompt.push_str("\n</WORK_ITEM_PLAN_OUTLINE_JSON>\n");
        prompt.push_str("\n## Outline summary\n");
        prompt.push_str(&format!(
            "- id: {}\n- source_story_spec_ids: [{}]\n- source_design_spec_ids: [{}]\n- strategy_summary: {}\n- handoff_strategy: {}\n- status: {}\n",
            outline.id,
            outline.source_story_spec_ids.join(", "),
            outline.source_design_spec_ids.join(", "),
            outline.strategy_summary,
            outline.handoff_strategy,
            outline.status
        ));
        prompt.push_str("\n### Work item outlines\n");
        for item in &outline.work_item_outlines {
            let estimated_context_tokens = item
                .estimated_context_tokens
                .map(|value| value.to_string())
                .unwrap_or_else(|| "(missing)".to_string());
            let session_fit = match item.session_fit.as_ref() {
                Some(crate::product::models::WorkItemOutlineSessionFit::FitsSingleAgentSession) => {
                    "fits_single_agent_session"
                }
                Some(crate::product::models::WorkItemOutlineSessionFit::TooLargeMustSplit) => {
                    "too_large_must_split"
                }
                None => "(missing)",
            };
            prompt.push_str(&format!(
                "\n- outline_id: {}\n  title: {}\n  kind: {:?}\n  goal: {}\n  scope: [{}]\n  non_goals: [{}]\n  estimated_context_tokens: {}\n  session_fit: {}\n  source_story_spec_ids: [{}]\n  source_design_spec_ids: [{}]\n  depends_on: [{}]\n  exclusive_write_scopes: [{}]\n  forbidden_write_scopes: [{}]\n  verification_intent: [{}]\n  handoff_notes: {}\n",
                item.outline_id,
                item.title,
                item.kind,
                item.goal,
                item.scope.join(", "),
                item.non_goals.join(", "),
                estimated_context_tokens,
                session_fit,
                item.source_story_spec_ids.join(", "),
                item.source_design_spec_ids.join(", "),
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
             每个 outline 必须能由单个 Claude Code 或 Codex coding 会话可靠完成。estimated_context_tokens 必须存在：不超过 40k 属正常范围，40001..=50000 必须结合目标内聚性、写入范围、编码、测试、返修与验证判断是否能在单 session 闭环，超过 50k 必须返回 `revise` 并要求拆分。\
             同时按最大内聚、最少拆分原则检查过度拆分：在不违反用户显式拆分选项、50k 上限、必要中断点、独立回滚/验收边界和上下文代理指标时，目标一致且可以在同一 session 闭环的 outline 必须合并；发现不必要拆分时返回 `revise`。\
             不要要求 author 在 Outline 阶段输出完整 Work Item 正文、完整 verification plan、required_gates 或 repository_profile。\
             如果问题会影响拆分边界，返回 `revise`；如果需要用户做产品/范围判断，返回 `needs_human`。\n",
        );
        let nonce = structured_output_nonce();
        let structured_output_contract = StructuredOutputContract {
            nonce: nonce.clone(),
            schema_name: "work_item_plan_outline_review".to_string(),
        };
        let schema = format!(
            r#"{{"verdict":"pass|revise|needs_human","review_scope":"outline","generation_round_id":"{}","summary":"一句话摘要","findings":[{{"severity":"blocking|must_fix|suggestion","target_outline_id":"outline id","message":"问题描述（含影响）","evidence":"Outline 中的具体证据","required_action":"需要 Outline author 执行的最小动作"}}]}}"#,
            generation_round_id
        );
        prompt.push_str(&reviewer_output_contract(
            &nonce,
            &schema,
            "\n\n请输出审核意见；可以先输出简短可读说明，最终 JSON 必须放在 nonce sentinel block 中，不得使用 Markdown code fence：\n\
             - `pass`：Outline 可进入生成模式选择。\n\
             - `revise`：Outline 需要返修，且必须给出至少一个 blocking/must_fix finding。\n\
             - `needs_human`：需要用户做产品/范围判断。\n\
             - 每条 finding 如果针对具体 outline，必须填写 `target_outline_id`，且只能引用当前 Outline 中存在的 outline_id。\n\
             - 如果 finding 针对整个 Outline 方案而不是某个具体 outline，可以省略 `target_outline_id`。\n\
             - 发现不必要拆分时必须给出 severity=must_fix 的 finding；message 必须以 [outline_unnecessary_split] 开头，target_outline_id 引用其中一个现有 outline，evidence 列出全部可合并 outline ID，required_action 明确要求合并。\n\
             - 系统会从 findings[].target_outline_id 推导受影响 outline，不要额外输出 affects_items。\n",
            &self.routing_reference_context(),
        ));
        Ok(StreamingProviderInput {
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
        let structured_output_contract = StructuredOutputContract {
            nonce: nonce.clone(),
            schema_name: "work_item_plan_batch_review".to_string(),
        };
        let mut prompt = String::new();
        prompt
            .push_str("请作为 reviewer 审核 WorkItemPlan 自动模式生成的整组 Work Item Draft。\n\n");
        prompt.push_str(&reviewer_boundary_rules_for(&self.session.workspace_type));
        prompt.push_str(&format!(
            "generation_round_id: {}\nbatch_id: {}\n\n",
            batch.generation_round_id, batch.batch_id
        ));
        prompt.push_str("[batch_draft_records]\n");
        prompt.push_str(&draft_json);
        prompt.push_str("\n\n");
        prompt.push_str(&reviewer_output_contract(
            &nonce,
            r#"{"verdict":"pass|revise_batch|needs_human|plan_reopen_required","review_scope":"batch","generation_round_id":"round id","summary":"一句话摘要","affects_items":[{"target_outline_id":"outline id"}],"findings":[{"severity":"blocking|must_fix|suggestion","message":"问题描述（含影响）","evidence":"整组 draft 或依赖上下文中的具体证据","required_action":"需要 batch author 执行的最小动作"}]}"#,
            "\n\n审核规则：自动模式只能整组通过、整组返修或要求重开 Outline；不得要求单项重写。最终 JSON 必须放在 nonce sentinel block 中。\n",
            &self.routing_reference_context(),
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
            permission_mode: permission_mode_for_provider(
                &provider,
                self.session.permission_modes.reviewer.clone(),
            ),
            structured_output_contract: Some(structured_output_contract),
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
        Ok(StreamingProviderInput {
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
