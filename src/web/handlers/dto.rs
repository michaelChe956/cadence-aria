use super::support::{product_execution_workspace_id, product_store_api_error};
use super::*;

pub(crate) fn issue_work_item_plan_detail_dto(
    plan: &IssueWorkItemPlanRecord,
) -> IssueWorkItemPlanDetailDto {
    IssueWorkItemPlanDetailDto {
        id: plan.id.clone(),
        issue_id: plan.issue_id.clone(),
        project_id: plan.project_id.clone(),
        status: issue_work_item_plan_status_text(&plan.status).to_string(),
        source_story_spec_ids: plan.source_story_spec_ids.clone(),
        source_design_spec_ids: plan.source_design_spec_ids.clone(),
        work_item_ids: plan.work_item_ids.clone(),
        verification_plan_ids: plan.verification_plan_ids.clone(),
        dependency_graph: plan
            .dependency_graph
            .iter()
            .map(|edge| IssueWorkItemPlanDependencyEdgeDto {
                from_work_item_id: edge.from_work_item_id.clone(),
                to_work_item_id: edge.to_work_item_id.clone(),
            })
            .collect(),
        repository_profile_ref: plan.repository_profile_ref.clone(),
        options: crate::web::types::WorkItemSplitOptions {
            include_integration_tests: plan.options.include_integration_tests,
            include_e2e_tests: plan.options.include_e2e_tests,
            force_frontend_backend_split: plan.options.force_frontend_backend_split,
            require_execution_plan_confirm: plan.options.require_execution_plan_confirm,
        },
        validator_findings: plan
            .validator_findings
            .iter()
            .map(work_item_split_finding_dto)
            .collect(),
        created_at: plan.created_at.clone(),
        updated_at: plan.updated_at.clone(),
    }
}

pub(crate) fn work_item_split_finding_dto(
    finding: &crate::product::models::WorkItemSplitFinding,
) -> crate::web::types::WorkItemSplitFinding {
    crate::web::types::WorkItemSplitFinding {
        finding_id: finding.code.clone(),
        level: match finding.severity {
            crate::product::models::WorkItemSplitFindingSeverity::Error => "error".to_string(),
            crate::product::models::WorkItemSplitFindingSeverity::Warning => "warning".to_string(),
        },
        message: finding.message.clone(),
        affected_scopes: finding.work_item_ids.clone(),
    }
}

pub(crate) fn issue_work_item_plan_status_text(status: &IssueWorkItemPlanStatus) -> &'static str {
    match status {
        IssueWorkItemPlanStatus::Draft => "draft",
        IssueWorkItemPlanStatus::Confirmed => "confirmed",
        IssueWorkItemPlanStatus::ChangeRequested => "change_requested",
    }
}

pub(crate) fn workspace_dto(record: WorkspaceRecord) -> WorkspaceDto {
    WorkspaceDto {
        workspace_id: record.workspace_id,
        name: record.name,
        path: record.path.to_string_lossy().to_string(),
        default_policy_preset: record.default_policy_preset,
        default_provider_mode: record.default_provider_mode,
        created_at: record.created_at.to_rfc3339(),
        updated_at: record.updated_at.to_rfc3339(),
    }
}

pub(crate) fn project_dto(record: ProjectRecord) -> ProjectDto {
    ProjectDto {
        project_id: record.id,
        name: record.name,
        description: record.description,
        created_at: record.created_at,
        updated_at: record.updated_at,
        last_opened_at: record.last_opened_at,
    }
}

pub(crate) fn repository_dto(record: RepositoryRecord) -> RepositoryDto {
    RepositoryDto {
        repository_id: record.id,
        project_id: record.project_id,
        name: record.name,
        path: record.path.to_string_lossy().to_string(),
        repo_hash: record.repo_hash,
        runtime_root: record.runtime_root.to_string_lossy().to_string(),
        default_policy_preset: record.default_policy_preset,
        default_provider_mode: record.default_provider_mode,
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}

pub(crate) fn product_issue_dto_with_binding(
    app_paths: &ProductAppPaths,
    record: ProductIssueRecord,
) -> ApiResult<ProductIssueDto> {
    let active_binding = active_binding_for_issue(app_paths, &record.project_id, &record)?;
    Ok(product_issue_dto(record, active_binding))
}

pub(crate) fn product_issue_dto(
    record: ProductIssueRecord,
    active_binding: Option<IssueRuntimeBindingRecord>,
) -> ProductIssueDto {
    let workspace_id = active_binding
        .as_ref()
        .map(|binding| product_execution_workspace_id(&record.project_id, &binding.repo_id));
    let task_id = active_binding
        .as_ref()
        .and_then(|binding| binding.task_id.clone());
    let session_id = active_binding
        .as_ref()
        .and_then(|binding| binding.session_id.clone());
    let artifacts = active_binding
        .as_ref()
        .map(product_issue_artifacts)
        .unwrap_or_default();
    ProductIssueDto {
        issue_id: record.id,
        project_id: record.project_id,
        repo_id: record.repo_id,
        workspace_id,
        task_id,
        session_id,
        title: record.title,
        description: record.description,
        change_id: record.change_id,
        phase: product_issue_phase_text(&record.phase).to_string(),
        status: product_issue_status_text(&record.status).to_string(),
        active_binding_id: record.active_binding_id,
        artifacts,
        created_at: record.created_at,
        updated_at: record.updated_at,
    }
}

pub(crate) fn latest_workspace_artifact_markdown(
    sessions: &[WorkspaceSessionRecord],
    workspace_type: WorkspaceType,
    entity_id: &str,
) -> Option<String> {
    sessions
        .iter()
        .filter(|session| {
            session.workspace_type == workspace_type && session.entity_id == entity_id
        })
        .flat_map(|session| session.messages.iter())
        .rev()
        .find(|message| matches!(message.role.as_str(), "assistant" | "provider"))
        .map(|message| message.content.clone())
        .filter(|content| !content.trim().is_empty())
}

pub(crate) fn story_spec_dto(
    lifecycle: &LifecycleStore,
    record: &StorySpecRecord,
    session: Option<&WorkspaceSessionRecord>,
) -> ApiResult<StorySpecDto> {
    Ok(StorySpecDto {
        story_spec_id: record.id.clone(),
        issue_id: record.issue_id.clone(),
        repository_id: record.repository_id.clone(),
        title: record.title.clone(),
        current_version: record.current_version,
        current_markdown_preview: current_markdown_preview(lifecycle, record)?,
        confirmation_status: lifecycle_confirmation_status_text(&record.confirmation_status)
            .to_string(),
        artifact_versions: artifact_version_dtos(lifecycle, session)?,
    })
}

pub(crate) fn design_spec_dto(
    lifecycle: &LifecycleStore,
    record: &DesignSpecRecord,
    session: Option<&WorkspaceSessionRecord>,
) -> ApiResult<DesignSpecDto> {
    Ok(DesignSpecDto {
        design_spec_id: record.id.clone(),
        issue_id: record.issue_id.clone(),
        story_spec_ids: record.story_spec_ids.clone(),
        title: record.title.clone(),
        current_version: record.current_version,
        current_markdown_preview: current_markdown_preview(lifecycle, record)?,
        confirmation_status: lifecycle_confirmation_status_text(&record.confirmation_status)
            .to_string(),
        artifact_versions: artifact_version_dtos(lifecycle, session)?,
    })
}

pub(crate) fn workspace_session_for_entity<'a>(
    sessions: &'a [WorkspaceSessionRecord],
    entity_id: &str,
    workspace_type: &WorkspaceType,
) -> Option<&'a WorkspaceSessionRecord> {
    sessions
        .iter()
        .rev()
        .find(|session| session.entity_id == entity_id && &session.workspace_type == workspace_type)
}

pub(crate) fn artifact_version_dtos(
    lifecycle: &LifecycleStore,
    session: Option<&WorkspaceSessionRecord>,
) -> ApiResult<Vec<ArtifactVersionDto>> {
    let Some(session) = session else {
        return Ok(Vec::new());
    };
    lifecycle
        .list_artifact_versions(&session.id)
        .map_err(product_store_api_error)
        .map(|versions| versions.into_iter().map(artifact_version_dto).collect())
}

pub(crate) fn artifact_version_dto(version: ArtifactVersion) -> ArtifactVersionDto {
    ArtifactVersionDto {
        version: version.version,
        markdown: version.to_markdown_string(),
        generated_by: provider_name_text(&version.generated_by).to_string(),
        reviewed_by: version
            .reviewed_by
            .as_ref()
            .map(provider_name_text)
            .map(str::to_string),
        review_verdict: version
            .review_verdict
            .as_ref()
            .map(review_verdict_text)
            .map(str::to_string),
        confirmed_by: version.confirmed_by,
        created_at: version.created_at,
        source_node_id: version.source_node_id,
    }
}

pub(crate) trait SpecDtoSource {
    fn project_id(&self) -> &str;
    fn issue_id(&self) -> &str;
    fn entity_id(&self) -> &str;
    fn current_version(&self) -> Option<u32>;
}

impl SpecDtoSource for StorySpecRecord {
    fn project_id(&self) -> &str {
        &self.project_id
    }

    fn issue_id(&self) -> &str {
        &self.issue_id
    }

    fn entity_id(&self) -> &str {
        &self.id
    }

    fn current_version(&self) -> Option<u32> {
        self.current_version
    }
}

impl SpecDtoSource for DesignSpecRecord {
    fn project_id(&self) -> &str {
        &self.project_id
    }

    fn issue_id(&self) -> &str {
        &self.issue_id
    }

    fn entity_id(&self) -> &str {
        &self.id
    }

    fn current_version(&self) -> Option<u32> {
        self.current_version
    }
}

pub(crate) fn current_markdown_preview(
    lifecycle: &LifecycleStore,
    record: &impl SpecDtoSource,
) -> ApiResult<Option<String>> {
    let Some(current_version) = record.current_version() else {
        return Ok(None);
    };
    let versions = lifecycle
        .list_versions(record.project_id(), record.issue_id(), record.entity_id())
        .map_err(product_store_api_error)?;
    Ok(versions
        .into_iter()
        .find(|version| version.version == current_version)
        .map(|version| markdown_preview(&version.markdown)))
}

pub(crate) fn markdown_preview(markdown: &str) -> String {
    let preview = markdown
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    const MAX_PREVIEW_CHARS: usize = 240;
    if preview.chars().count() <= MAX_PREVIEW_CHARS {
        return preview;
    }
    preview.chars().take(MAX_PREVIEW_CHARS).collect()
}

pub(crate) fn lifecycle_work_item_dto(
    lifecycle: &LifecycleStore,
    record: LifecycleWorkItemRecord,
    latest_attempt: Option<CodingAttemptDto>,
    session: Option<&WorkspaceSessionRecord>,
) -> ApiResult<LifecycleWorkItemDto> {
    Ok(LifecycleWorkItemDto {
        work_item_id: record.id,
        issue_id: record.issue_id,
        repository_id: record.repository_id,
        story_spec_ids: record.story_spec_ids,
        design_spec_ids: record.design_spec_ids,
        title: record.title,
        plan_status: work_item_plan_status_text(&record.plan_status).to_string(),
        execution_status: work_item_status_text(&record.execution_status).to_string(),
        latest_attempt,
        artifact_versions: artifact_version_dtos(lifecycle, session)?,
        work_item_set_id: record.work_item_set_id,
        source_work_item_plan_id: record.source_work_item_plan_id,
        source_outline_id: record.source_outline_id,
        source_draft_id: record.source_draft_id,
        planned_implementation_context: record.planned_implementation_context,
        planned_handoff_summary: record.planned_handoff_summary,
        kind: work_item_kind_text(&record.kind).to_string(),
        sequence_hint: record.sequence_hint,
        depends_on: record.depends_on,
        exclusive_write_scopes: record.exclusive_write_scopes,
        forbidden_write_scopes: record.forbidden_write_scopes,
        context_budget: WorkItemContextBudgetDto {
            target_context_k: record.context_budget.target_context_k,
            max_summary_chars: record.context_budget.max_summary_chars,
            max_handoff_chars: record.context_budget.max_handoff_chars,
            max_code_context_chars: record.context_budget.max_code_context_chars,
            max_context_file_refs: record.context_budget.max_context_file_refs,
            max_traceability_refs: record.context_budget.max_traceability_refs,
            max_dependency_handoffs: record.context_budget.max_dependency_handoffs,
        },
        required_handoff_from: record.required_handoff_from,
        verification_plan_ref: record.verification_plan_ref,
        require_execution_plan_confirm: record.require_execution_plan_confirm,
        execution_plan_status: work_item_execution_plan_status_text(&record.execution_plan_status)
            .to_string(),
        handoff_summary_ref: record.handoff_summary_ref,
        completion_commit: record.completion_commit,
        completion_diff_summary_ref: record.completion_diff_summary_ref,
    })
}

pub(crate) fn coding_attempt_dto(attempt: &CodingExecutionAttempt) -> CodingAttemptDto {
    CodingAttemptDto {
        attempt_id: attempt.id.clone(),
        work_item_id: attempt.work_item_id.clone(),
        attempt_scope: coding_attempt_scope_text(&attempt.scope).to_string(),
        work_item_group_id: attempt.work_item_group_id.clone(),
        current_work_item_id: attempt.current_work_item_id.clone(),
        active_unit_id: attempt.active_unit_id.clone(),
        attempt_no: attempt.attempt_no,
        status: coding_attempt_status_text(&attempt.status).to_string(),
        stage: coding_execution_stage_text(&attempt.stage).to_string(),
        branch_name: attempt.branch_name.clone(),
        base_branch: attempt.base_branch.clone(),
        worktree_path: attempt
            .worktree_path
            .as_ref()
            .map(|path| path.to_string_lossy().to_string()),
        rework_count: attempt.rework_count,
        head_commit: attempt.head_commit.clone(),
        push_status: attempt
            .pushed_remote
            .as_ref()
            .map(|_| push_status_text(&PushStatus::Pushed).to_string()),
        review_request_url: None,
        created_at: attempt.created_at.clone(),
        updated_at: attempt.updated_at.clone(),
    }
}

pub(crate) fn coding_execution_unit_dto(
    unit: &crate::product::coding_models::CodingExecutionUnit,
) -> CodingExecutionUnitDto {
    CodingExecutionUnitDto {
        unit_id: unit.id.clone(),
        work_item_id: unit.work_item_id.clone(),
        order_index: unit.order_index,
        status: coding_execution_unit_status_text(&unit.status).to_string(),
        summary: unit.summary.clone(),
        handoff_ref: unit.handoff_ref.clone(),
        completion_commit: unit.completion_commit.clone(),
    }
}

pub(crate) fn coding_attempt_scope_text(scope: &CodingAttemptScope) -> &'static str {
    match scope {
        CodingAttemptScope::WorkItem => "work_item",
        CodingAttemptScope::WorkItemGroup => "work_item_group",
    }
}

pub(crate) fn coding_execution_unit_status_text(
    status: &CodingExecutionUnitStatus,
) -> &'static str {
    match status {
        CodingExecutionUnitStatus::Pending => "pending",
        CodingExecutionUnitStatus::Running => "running",
        CodingExecutionUnitStatus::WaitingForHuman => "waiting_for_human",
        CodingExecutionUnitStatus::Completed => "completed",
        CodingExecutionUnitStatus::Failed => "failed",
        CodingExecutionUnitStatus::Blocked => "blocked",
        CodingExecutionUnitStatus::Skipped => "skipped",
    }
}

pub(crate) fn active_coding_timeline_node_id(nodes: &[CodingTimelineNode]) -> Option<String> {
    nodes
        .iter()
        .rev()
        .find(|node| {
            matches!(
                node.status,
                CodingTimelineNodeStatus::Pending
                    | CodingTimelineNodeStatus::Running
                    | CodingTimelineNodeStatus::Blocked
            )
        })
        .map(|node| node.id.clone())
}

pub(crate) fn workspace_session_dto(record: WorkspaceSessionRecord) -> WorkspaceSessionDto {
    WorkspaceSessionDto {
        workspace_session_id: record.id,
        issue_id: record.issue_id,
        entity_id: record.entity_id,
        workspace_type: workspace_type_text(&record.workspace_type).to_string(),
        status: workspace_session_status_text(&record.status).to_string(),
        author_provider: provider_name_text(&record.author_provider).to_string(),
        reviewer_provider: provider_name_text(&record.reviewer_provider).to_string(),
        review_rounds: record.review_rounds,
        superpowers_enabled: record.superpowers_enabled,
        openspec_enabled: record.openspec_enabled,
        messages: record
            .messages
            .into_iter()
            .map(workspace_message_dto)
            .collect(),
    }
}

pub(crate) fn workspace_message_dto(record: WorkspaceMessageRecord) -> WorkspaceMessageDto {
    WorkspaceMessageDto {
        role: record.role,
        content: record.content,
        created_at: record.created_at,
    }
}

pub(crate) fn product_issue_artifacts(
    binding: &IssueRuntimeBindingRecord,
) -> Vec<ProductIssueArtifactDto> {
    let Some(task_id) = binding.task_id.as_deref() else {
        return Vec::new();
    };
    let Some(workspace_root) = workspace_root_for_binding(binding) else {
        return Vec::new();
    };
    WebRuntime::projection_for_workspace(&workspace_root, Some(task_id), None)
        .map(|projection| {
            projection
                .artifact_index
                .into_iter()
                .map(|artifact| ProductIssueArtifactDto {
                    stage: artifact_stage(
                        &artifact.artifact_kind,
                        artifact.producer_node.as_deref(),
                    )
                    .to_string(),
                    artifact_ref: artifact.artifact_ref,
                    artifact_kind: artifact.artifact_kind,
                    producer_node: artifact.producer_node,
                    path: artifact.path,
                    summary: artifact.summary,
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn workspace_root_for_binding(binding: &IssueRuntimeBindingRecord) -> Option<PathBuf> {
    binding.runtime_root.parent()?.parent().map(PathBuf::from)
}

pub(crate) fn artifact_stage(artifact_kind: &str, producer_node: Option<&str>) -> &'static str {
    match producer_node {
        Some("N04" | "N05" | "N06" | "N07") => return "story_spec",
        Some("N08" | "N09" | "N10" | "N11" | "N12") => return "design_spec",
        Some("N27") => return "done",
        Some(_) => return "work_item",
        None => {}
    }
    if artifact_kind.contains("clarification")
        || artifact_kind == "spec"
        || artifact_kind == "openspec_spec"
        || artifact_kind == "openspec_proposal"
    {
        "story_spec"
    } else if artifact_kind.contains("design") {
        "design_spec"
    } else if artifact_kind.contains("final") {
        "done"
    } else {
        "work_item"
    }
}

pub(crate) fn active_binding_for_issue(
    app_paths: &ProductAppPaths,
    project_id: &str,
    issue: &ProductIssueRecord,
) -> ApiResult<Option<IssueRuntimeBindingRecord>> {
    let Some(active_binding_id) = issue.active_binding_id.as_deref() else {
        return Ok(None);
    };
    Ok(RuntimeBindingStore::new(app_paths.clone())
        .list(project_id, &issue.id)
        .map_err(product_store_api_error)?
        .into_iter()
        .find(|binding| binding.id == active_binding_id))
}

pub(crate) fn issue_dto(record: IssueRecord) -> IssueDto {
    IssueDto {
        issue_id: record.issue_id,
        title: record.title,
        description: record.description,
        status: issue_status_text(&record.status).to_string(),
        workspace_id: record.workspace_id,
        task_id: record.task_id,
        session_id: record.session_id,
        change_id: record.change_id,
        created_at: record.created_at.to_rfc3339(),
        updated_at: record.updated_at.to_rfc3339(),
    }
}

pub(crate) fn product_issue_phase_text(phase: &ProductIssuePhase) -> &'static str {
    match phase {
        ProductIssuePhase::Clarification => "clarification",
        ProductIssuePhase::Development => "development",
        ProductIssuePhase::Acceptance => "acceptance",
    }
}

pub(crate) fn product_issue_status_text(status: &ProductIssueStatus) -> &'static str {
    match status {
        ProductIssueStatus::Draft => "draft",
        ProductIssueStatus::InProgress => "in_progress",
        ProductIssueStatus::Completed => "completed",
        ProductIssueStatus::Blocked => "blocked",
    }
}

pub(crate) fn lifecycle_confirmation_status_text(
    status: &LifecycleConfirmationStatus,
) -> &'static str {
    match status {
        LifecycleConfirmationStatus::Draft => "draft",
        LifecycleConfirmationStatus::InReview => "in_review",
        LifecycleConfirmationStatus::Confirmed => "confirmed",
        LifecycleConfirmationStatus::ChangeRequested => "change_requested",
        LifecycleConfirmationStatus::Blocked => "blocked",
    }
}

pub(crate) fn work_item_plan_status_text(status: &WorkItemPlanStatus) -> &'static str {
    match status {
        WorkItemPlanStatus::NotStarted => "not_started",
        WorkItemPlanStatus::Draft => "draft",
        WorkItemPlanStatus::Confirmed => "confirmed",
        WorkItemPlanStatus::ChangeRequested => "change_requested",
    }
}

pub(crate) fn work_item_status_text(status: &WorkItemStatus) -> &'static str {
    match status {
        WorkItemStatus::Pending => "pending",
        WorkItemStatus::Planning => "planning",
        WorkItemStatus::Coding => "coding",
        WorkItemStatus::Completed => "completed",
        WorkItemStatus::Blocked => "blocked",
    }
}

pub(crate) fn work_item_kind_text(kind: &WorkItemKind) -> &'static str {
    match kind {
        WorkItemKind::Backend => "backend",
        WorkItemKind::Frontend => "frontend",
        WorkItemKind::Integration => "integration",
        WorkItemKind::E2e => "e2e",
        WorkItemKind::Docs => "docs",
        WorkItemKind::Infra => "infra",
        WorkItemKind::Other => "other",
    }
}

pub(crate) fn work_item_execution_plan_status_text(
    status: &WorkItemExecutionPlanStatus,
) -> &'static str {
    match status {
        WorkItemExecutionPlanStatus::NotStarted => "not_started",
        WorkItemExecutionPlanStatus::Draft => "draft",
        WorkItemExecutionPlanStatus::Confirmed => "confirmed",
        WorkItemExecutionPlanStatus::ChangeRequested => "change_requested",
    }
}

pub(crate) fn coding_attempt_status_text(status: &CodingAttemptStatus) -> &'static str {
    match status {
        CodingAttemptStatus::Created => "created",
        CodingAttemptStatus::Running => "running",
        CodingAttemptStatus::WaitingForHuman => "waiting_for_human",
        CodingAttemptStatus::Blocked => "blocked",
        CodingAttemptStatus::Completed => "completed",
        CodingAttemptStatus::Failed => "failed",
        CodingAttemptStatus::Aborted => "aborted",
    }
}

pub(crate) fn coding_execution_stage_text(stage: &CodingExecutionStage) -> &'static str {
    match stage {
        CodingExecutionStage::PrepareContext => "prepare_context",
        CodingExecutionStage::WorktreePrepare => "worktree_prepare",
        CodingExecutionStage::Coding => "coding",
        CodingExecutionStage::Testing => "testing",
        CodingExecutionStage::CodeReview => "code_review",
        CodingExecutionStage::Rework => "rework",
        CodingExecutionStage::ReviewRequest => "review_request",
        CodingExecutionStage::InternalPrReview => "internal_pr_review",
        CodingExecutionStage::FinalConfirm => "final_confirm",
    }
}

pub(crate) fn push_status_text(status: &PushStatus) -> &'static str {
    match status {
        PushStatus::NotPushed => "not_pushed",
        PushStatus::Pushed => "pushed",
        PushStatus::Failed => "failed",
    }
}

pub(crate) fn workspace_type_text(workspace_type: &WorkspaceType) -> &'static str {
    match workspace_type {
        WorkspaceType::Story => "story",
        WorkspaceType::Design => "design",
        WorkspaceType::WorkItem => "work_item",
        WorkspaceType::WorkItemPlan => "work_item_plan",
    }
}

pub(crate) fn workspace_session_status_text(status: &WorkspaceSessionStatus) -> &'static str {
    match status {
        WorkspaceSessionStatus::Open => "open",
        WorkspaceSessionStatus::Running => "running",
        WorkspaceSessionStatus::WaitingForHuman => "waiting_for_human",
        WorkspaceSessionStatus::Confirmed => "confirmed",
        WorkspaceSessionStatus::ChangeRequested => "change_requested",
        WorkspaceSessionStatus::BlockedProviderUnavailable => "blocked_provider_unavailable",
        WorkspaceSessionStatus::Terminated => "terminated",
    }
}

pub(crate) fn provider_name_text(provider: &ProviderName) -> &'static str {
    match provider {
        ProviderName::ClaudeCode => "claude_code",
        ProviderName::Codex => "codex",
        ProviderName::Fake => "fake",
    }
}

pub(crate) fn review_verdict_text(verdict: &ReviewVerdictType) -> &'static str {
    match verdict {
        ReviewVerdictType::Pass => "pass",
        ReviewVerdictType::Revise => "revise",
        ReviewVerdictType::NeedsHuman => "needs_human",
    }
}

pub(crate) fn issue_status_text(status: &IssueStatus) -> &'static str {
    match status {
        IssueStatus::Draft => "draft",
        IssueStatus::Started => "started",
        IssueStatus::Running => "running",
        IssueStatus::Completed => "completed",
        IssueStatus::Blocked => "blocked",
    }
}
