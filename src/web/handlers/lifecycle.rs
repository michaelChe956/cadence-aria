use super::dto::*;
use super::support::*;
use super::*;
use crate::product::models::WorkItemRuntimeBinding;
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::product::work_item_runtime_reader::WorkItemRuntimeReader;
use std::collections::{BTreeMap, BTreeSet};

mod deletion;
pub use deletion::{delete_work_item, delete_work_item_plan};

pub async fn issue_lifecycle(
    State(state): State<WebAppState>,
    Path(issue_id): Path<String>,
    Query(query): Query<GateResolveQuery>,
) -> ApiResult<Json<IssueLifecycleResponse>> {
    let project_id = query
        .project_id
        .ok_or_else(|| ApiError::validation("project_required", "project_id is required"))?;
    let app_paths = product_app_paths(&state);
    let issue = IssueStore::new(app_paths.clone())
        .get(&project_id, &issue_id)
        .map_err(product_store_api_error)?;
    let lifecycle = LifecycleStore::new(app_paths.clone());
    backfill_legacy_spec_versions(&lifecycle, &project_id, &issue_id)?;
    let workspace_sessions = lifecycle
        .list_workspace_session_summaries(&project_id, &issue_id)
        .map_err(product_store_api_error)?;
    let story_specs = lifecycle
        .list_story_specs(&project_id, &issue_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .map(|story| {
            let session =
                workspace_session_for_entity(&workspace_sessions, &story.id, &WorkspaceType::Story);
            story_spec_dto(
                &lifecycle,
                &story,
                session.map(|session| session.id.as_str()),
            )
        })
        .collect::<ApiResult<Vec<_>>>()?;
    let design_specs = lifecycle
        .list_design_specs(&project_id, &issue_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .map(|design| {
            let session = workspace_session_for_entity(
                &workspace_sessions,
                &design.id,
                &WorkspaceType::Design,
            );
            design_spec_dto(
                &lifecycle,
                &design,
                session.map(|session| session.id.as_str()),
            )
        })
        .collect::<ApiResult<Vec<_>>>()?;
    let work_item_plan_records = lifecycle
        .list_issue_work_item_plans(&project_id, &issue_id)
        .map_err(product_store_api_error)?;
    let work_item_plans = work_item_plan_records
        .iter()
        .map(issue_work_item_plan_detail_dto)
        .collect::<Vec<_>>();
    let coding_store = CodingAttemptStore::new(app_paths.clone());
    let mut coding_attempts = Vec::new();
    let repository_id = issue.repo_id.clone().ok_or_else(|| {
        product_store_api_error(ProductStoreError::NotFound {
            kind: "issue_repository",
            id: issue.id.clone(),
        })
    })?;
    let revision_store = WorkItemRevisionStore::new(app_paths.clone());
    let runtime_reader = WorkItemRuntimeReader::new(app_paths.clone());
    let mut schema_v2_plan_ids = BTreeSet::new();
    let mut schema_v2_work_item_ids = BTreeSet::new();
    let mut work_items = Vec::new();

    for plan in &work_item_plan_records {
        let lineage = match revision_store.get_plan_lineage(&project_id, &issue_id, &plan.id) {
            Ok(lineage) => lineage,
            Err(ProductStoreError::NotFound {
                kind: "work_item_plan_lineage",
                ..
            }) => {
                continue;
            }
            Err(error) => return Err(product_store_api_error(error)),
        };
        schema_v2_plan_ids.insert(plan.id.clone());
        if plan.status != IssueWorkItemPlanStatus::Confirmed {
            continue;
        }

        let active_revision_id = lineage.active_revision_id.as_deref().ok_or_else(|| {
            product_store_api_error(ProductStoreError::IdentityMismatch {
                kind: "runtime_binding_missing",
                id: plan.id.clone(),
            })
        })?;
        let plan_revision = revision_store
            .get_plan_revision(&project_id, &issue_id, &plan.id, active_revision_id)
            .map_err(product_store_api_error)?;
        let plan_projection = revision_store
            .get_plan_projection_bundle(&lineage, &plan_revision.plan_projection_bundle_id)
            .map_err(product_store_api_error)?;
        if plan_projection.plan_revision_id != plan_revision.id
            || plan_projection.dependency_graph_revision_id
                != plan_revision.dependency_graph_revision_id
            || plan_projection.human_group_projection.plan_id != plan.id
        {
            return Err(product_store_api_error(
                ProductStoreError::IdentityMismatch {
                    kind: "runtime_binding_integrity_mismatch",
                    id: plan.id.clone(),
                },
            ));
        }

        let human_item_ids = plan_projection
            .human_group_projection
            .work_items
            .iter()
            .map(|item| item.logical_work_item_id.clone())
            .collect::<BTreeSet<_>>();
        let revision_item_ids = plan_revision
            .work_item_bindings
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        if human_item_ids.len() != plan_projection.human_group_projection.work_items.len()
            || human_item_ids != revision_item_ids
        {
            return Err(product_store_api_error(
                ProductStoreError::IdentityMismatch {
                    kind: "runtime_binding_integrity_mismatch",
                    id: plan.id.clone(),
                },
            ));
        }

        let group_attempt = coding_store
            .get_attempt_for_work_item_group(&project_id, &issue_id, &plan.id)
            .map_err(product_store_api_error)?;
        let units_by_logical_id = if let Some(attempt) = group_attempt.as_ref() {
            coding_attempts.push(coding_attempt_dto(attempt));
            let units = coding_store
                .list_coding_units(&project_id, &issue_id, &attempt.id)
                .map_err(product_store_api_error)?;
            let mut units_by_logical_id = BTreeMap::new();
            for unit in units {
                if unit.plan_id != plan.id
                    || plan_revision
                        .work_item_bindings
                        .get(&unit.logical_work_item_id)
                        != Some(&unit.work_item_revision_id)
                    || units_by_logical_id
                        .insert(unit.logical_work_item_id.clone(), unit)
                        .is_some()
                {
                    return Err(product_store_api_error(
                        ProductStoreError::IdentityMismatch {
                            kind: "runtime_binding_integrity_mismatch",
                            id: attempt.id.clone(),
                        },
                    ));
                }
            }
            units_by_logical_id
        } else {
            BTreeMap::new()
        };

        for human_projection in &plan_projection.human_group_projection.work_items {
            let work_item_revision_id = plan_revision
                .work_item_bindings
                .get(&human_projection.logical_work_item_id)
                .ok_or_else(|| {
                    product_store_api_error(ProductStoreError::IdentityMismatch {
                        kind: "runtime_binding_integrity_mismatch",
                        id: human_projection.logical_work_item_id.clone(),
                    })
                })?;
            let work_item_revision = revision_store
                .get_work_item_revision(
                    &lineage,
                    &human_projection.logical_work_item_id,
                    work_item_revision_id,
                )
                .map_err(product_store_api_error)?;
            let projection_bundle = revision_store
                .get_work_item_projection_bundle(
                    &lineage,
                    &work_item_revision.work_item_projection_bundle_id,
                )
                .map_err(product_store_api_error)?;
            let binding = WorkItemRuntimeBinding {
                plan_id: plan.id.clone(),
                plan_revision_id: plan_revision.id.clone(),
                logical_work_item_id: human_projection.logical_work_item_id.clone(),
                work_item_revision_id: work_item_revision.id.clone(),
                projection_bundle_id: projection_bundle.id.clone(),
                verification_plan_revision_id: work_item_revision
                    .verification_plan_revision_id
                    .clone(),
                canonical_contract_hash: work_item_revision.canonical_contract_hash.clone(),
                projection_compiler_version: projection_bundle.compiler_version.clone(),
                human_projection_hash: projection_bundle.human_projection_hash.clone(),
                coder_projection_hash: projection_bundle.coder_projection_hash.clone(),
                reviewer_projection_hash: projection_bundle.reviewer_projection_hash.clone(),
            };
            let runtime = runtime_reader
                .resolve_binding(&project_id, &issue_id, &binding)
                .map_err(product_store_api_error)?;
            let session = workspace_session_for_entity(
                &workspace_sessions,
                &human_projection.logical_work_item_id,
                &WorkspaceType::WorkItem,
            );
            let unit = units_by_logical_id.get(&human_projection.logical_work_item_id);
            work_items.push(lifecycle_work_item_runtime_dto(
                &lifecycle,
                LifecycleWorkItemRuntimeDtoInput {
                    repository_id: &repository_id,
                    plan_id: &plan.id,
                    runtime: &runtime,
                    human_projection,
                    latest_attempt: group_attempt.as_ref().map(coding_attempt_dto),
                    unit,
                    session_id: session.map(|session| session.id.as_str()),
                    require_execution_plan_confirm: plan.options.require_execution_plan_confirm,
                },
            )?);
            schema_v2_work_item_ids.insert(human_projection.logical_work_item_id.clone());
        }
    }

    let has_legacy_plan = work_item_plan_records
        .iter()
        .any(|plan| !schema_v2_plan_ids.contains(&plan.id));
    let legacy_work_items = if schema_v2_plan_ids.is_empty() || has_legacy_plan {
        lifecycle
            .list_work_items(&project_id, &issue_id)
            .map_err(product_store_api_error)?
    } else {
        Vec::new()
    };
    let legacy_work_items = legacy_work_items
        .into_iter()
        .filter(|work_item| !schema_v2_work_item_ids.contains(&work_item.id))
        .map(|work_item| {
            let attempts = coding_store
                .list_attempts_for_work_item(&project_id, &issue_id, &work_item.id)
                .map_err(product_store_api_error)?;
            let latest_attempt = attempts.last().map(coding_attempt_dto);
            coding_attempts.extend(attempts.iter().map(coding_attempt_dto));
            let session = workspace_session_for_entity(
                &workspace_sessions,
                &work_item.id,
                &WorkspaceType::WorkItem,
            );
            lifecycle_work_item_dto(
                &lifecycle,
                work_item,
                latest_attempt,
                session.map(|session| session.id.as_str()),
            )
        })
        .collect::<ApiResult<Vec<_>>>()?;
    work_items.extend(legacy_work_items);
    let workspace_sessions = workspace_sessions
        .iter()
        .map(workspace_session_summary_dto)
        .collect();

    Ok(Json(IssueLifecycleResponse {
        issue: product_issue_dto_with_binding(&app_paths, issue)?,
        story_specs,
        design_specs,
        work_item_plans,
        work_items,
        workspace_sessions,
        coding_attempts,
    }))
}

pub async fn generate_story_specs(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Json(request): Json<GenerateStorySpecsRequest>,
) -> ApiResult<Json<GenerateStorySpecsResponse>> {
    let workspace_config = provider_workspace_config(
        request.author_provider.as_deref(),
        request.reviewer_provider.as_deref(),
        request.review_rounds,
        request.superpowers_enabled,
        request.openspec_enabled,
        &*state.provider_availability,
    )?;
    let app_paths = product_app_paths(&state);
    let issue = IssueStore::new(app_paths.clone())
        .get(&project_id, &issue_id)
        .map_err(product_store_api_error)?;
    let repository_id = issue
        .repo_id
        .clone()
        .ok_or_else(|| ApiError::validation("repository_required", "repository_id is required"))?;
    find_repository(&app_paths, &project_id, &repository_id)?;
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            repository_id,
            title: request.title,
            aggregate_codebase: None,
        })
        .map_err(product_store_api_error)?;
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id,
            issue_id,
            entity_id: story.id.clone(),
            workspace_type: WorkspaceType::Story,
            author_provider: workspace_config.author_provider,
            reviewer_provider: workspace_config.reviewer_provider,
            review_rounds: workspace_config.review_rounds,
            superpowers_enabled: workspace_config.superpowers_enabled,
            openspec_enabled: workspace_config.openspec_enabled,
        })
        .map_err(product_store_api_error)?;
    let session = ensure_workspace_context_message(&app_paths, &lifecycle, session)
        .map_err(product_store_api_error)?;

    let story_dto = story_spec_dto(&lifecycle, &story, Some(session.id.as_str()))?;
    Ok(Json(GenerateStorySpecsResponse {
        story_specs: vec![story_dto],
        workspace_session: workspace_session_dto(session),
    }))
}

pub async fn generate_design_specs(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Json(request): Json<GenerateDesignSpecsRequest>,
) -> ApiResult<Json<GenerateDesignSpecsResponse>> {
    let workspace_config = provider_workspace_config(
        request.author_provider.as_deref(),
        request.reviewer_provider.as_deref(),
        request.review_rounds,
        request.superpowers_enabled,
        request.openspec_enabled,
        &*state.provider_availability,
    )?;
    let app_paths = product_app_paths(&state);
    IssueStore::new(app_paths.clone())
        .get(&project_id, &issue_id)
        .map_err(product_store_api_error)?;
    let lifecycle = LifecycleStore::new(app_paths.clone());
    validate_confirmed_story_specs(&lifecycle, &project_id, &issue_id, &request.story_spec_ids)?;
    let design = lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            story_spec_ids: request.story_spec_ids,
            title: request.title,
        })
        .map_err(product_store_api_error)?;
    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id,
            issue_id,
            entity_id: design.id.clone(),
            workspace_type: WorkspaceType::Design,
            author_provider: workspace_config.author_provider,
            reviewer_provider: workspace_config.reviewer_provider,
            review_rounds: workspace_config.review_rounds,
            superpowers_enabled: workspace_config.superpowers_enabled,
            openspec_enabled: workspace_config.openspec_enabled,
        })
        .map_err(product_store_api_error)?;
    let session = ensure_workspace_context_message(&app_paths, &lifecycle, session)
        .map_err(product_store_api_error)?;

    let design_dto = design_spec_dto(&lifecycle, &design, Some(session.id.as_str()))?;
    Ok(Json(GenerateDesignSpecsResponse {
        design_specs: vec![design_dto],
        workspace_session: workspace_session_dto(session),
    }))
}

pub async fn prepare_work_item_plan(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Json(request): Json<PrepareWorkItemPlanRequest>,
) -> ApiResult<Json<PrepareWorkItemPlanResponse>> {
    let workspace_config = provider_workspace_config(
        request.author_provider.as_deref(),
        request.reviewer_provider.as_deref(),
        request.review_rounds,
        request.superpowers_enabled,
        request.openspec_enabled,
        &*state.provider_availability,
    )?;
    let app_paths = product_app_paths(&state);
    let issue = IssueStore::new(app_paths.clone())
        .get(&project_id, &issue_id)
        .map_err(product_store_api_error)?;
    let repository_id = issue
        .repo_id
        .clone()
        .ok_or_else(|| ApiError::validation("repository_required", "repository_id is required"))?;
    find_repository(&app_paths, &project_id, &repository_id)?;
    let lifecycle = LifecycleStore::new(app_paths.clone());
    validate_confirmed_story_specs(&lifecycle, &project_id, &issue_id, &request.story_spec_ids)?;
    validate_confirmed_design_specs(&lifecycle, &project_id, &issue_id, &request.design_spec_ids)?;

    let plan = lifecycle
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: None,
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            source_story_spec_ids: request.story_spec_ids,
            source_design_spec_ids: request.design_spec_ids,
            options: crate::product::models::IssueWorkItemPlanOptions {
                include_integration_tests: request.include_integration_tests.unwrap_or(true),
                include_e2e_tests: request.include_e2e_tests.unwrap_or(false),
                force_frontend_backend_split: request.force_frontend_backend_split.unwrap_or(false),
                require_execution_plan_confirm: request
                    .require_execution_plan_confirm
                    .unwrap_or(false),
            },
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: Vec::new(),
            repository_profile_ref: None,
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
        })
        .map_err(product_store_api_error)?;

    let session = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id,
            issue_id,
            entity_id: plan.id.clone(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: workspace_config.author_provider,
            reviewer_provider: workspace_config.reviewer_provider,
            review_rounds: workspace_config.review_rounds,
            superpowers_enabled: workspace_config.superpowers_enabled,
            openspec_enabled: workspace_config.openspec_enabled,
        })
        .map_err(product_store_api_error)?;
    let session = ensure_workspace_context_message(&app_paths, &lifecycle, session)
        .map_err(product_store_api_error)?;

    Ok(Json(PrepareWorkItemPlanResponse {
        work_item_plan: issue_work_item_plan_detail_dto(&plan),
        workspace_session: workspace_session_dto(session),
    }))
}

pub async fn delete_story_spec(
    State(state): State<WebAppState>,
    Path((project_id, issue_id, story_spec_id)): Path<(String, String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let store = LifecycleStore::new(product_app_paths(&state));
    store
        .delete_story_spec(&project_id, &issue_id, &story_spec_id)
        .map_err(product_store_api_error)?;
    Ok(Json(json!({"status":"deleted"})))
}

pub async fn delete_design_spec(
    State(state): State<WebAppState>,
    Path((project_id, issue_id, design_spec_id)): Path<(String, String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let store = LifecycleStore::new(product_app_paths(&state));
    store
        .delete_design_spec(&project_id, &issue_id, &design_spec_id)
        .map_err(product_store_api_error)?;
    Ok(Json(json!({"status":"deleted"})))
}

pub async fn confirm_gate(
    State(state): State<WebAppState>,
    Path((issue_id, gate_id)): Path<(String, String)>,
    Query(query): Query<GateResolveQuery>,
    Json(request): Json<ResolveGateRequest>,
) -> ApiResult<Json<ResolveGateResponse>> {
    resolve_gate(
        &state,
        issue_id,
        gate_id,
        query.project_id,
        GateStatus::Confirmed,
        "confirmed",
        request,
    )
}

pub async fn request_gate_change(
    State(state): State<WebAppState>,
    Path((issue_id, gate_id)): Path<(String, String)>,
    Query(query): Query<GateResolveQuery>,
    Json(request): Json<ResolveGateRequest>,
) -> ApiResult<Json<ResolveGateResponse>> {
    resolve_gate(
        &state,
        issue_id,
        gate_id,
        query.project_id,
        GateStatus::ChangeRequested,
        "change_requested",
        request,
    )
}

pub async fn terminate_gate(
    State(state): State<WebAppState>,
    Path((issue_id, gate_id)): Path<(String, String)>,
    Query(query): Query<GateResolveQuery>,
    Json(request): Json<ResolveGateRequest>,
) -> ApiResult<Json<ResolveGateResponse>> {
    resolve_gate(
        &state,
        issue_id,
        gate_id,
        query.project_id,
        GateStatus::Terminated,
        "terminated",
        request,
    )
}

pub(crate) fn resolve_gate(
    state: &WebAppState,
    issue_id: String,
    gate_id: String,
    project_id: Option<String>,
    status: GateStatus,
    decision: &str,
    request: ResolveGateRequest,
) -> ApiResult<Json<ResolveGateResponse>> {
    let store = GateStore::new(product_app_paths(state));
    let ResolveGateRequest {
        comment,
        requested_change,
    } = request;
    let gate = match project_id {
        Some(project_id) => store
            .resolve(
                &project_id,
                &issue_id,
                &gate_id,
                status,
                comment,
                requested_change,
            )
            .map_err(product_store_api_error)?,
        None => {
            let project_ids = store
                .project_ids_for_gate(&issue_id, &gate_id)
                .map_err(product_store_api_error)?;
            match project_ids.as_slice() {
                [project_id] => store
                    .resolve(
                        project_id,
                        &issue_id,
                        &gate_id,
                        status,
                        comment,
                        requested_change,
                    )
                    .map_err(product_store_api_error)?,
                [] => {
                    return Err(product_store_api_error(ProductStoreError::NotFound {
                        kind: "gate",
                        id: gate_id,
                    }));
                }
                _ => {
                    return Err(ApiError::runtime(
                        "gate_ambiguous",
                        "gate matches multiple projects",
                        json!({}),
                    ));
                }
            }
        }
    };
    Ok(Json(ResolveGateResponse {
        issue_id: gate.issue_id,
        gate_id: gate.id,
        node_id: gate.node_id,
        decision: decision.to_string(),
        next_node: None,
    }))
}

pub(crate) fn backfill_legacy_spec_versions(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
) -> ApiResult<()> {
    let sessions = lifecycle
        .list_workspace_sessions(project_id, issue_id)
        .map_err(product_store_api_error)?;
    for story in lifecycle
        .list_story_specs(project_id, issue_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .filter(|story| story.current_version.is_none())
    {
        if lifecycle
            .list_versions(project_id, issue_id, &story.id)
            .map_err(product_store_api_error)?
            .is_empty()
            && let Some(markdown) =
                latest_workspace_artifact_markdown(&sessions, WorkspaceType::Story, &story.id)
        {
            lifecycle
                .append_version(AppendSpecVersionInput {
                    project_id: project_id.to_string(),
                    issue_id: issue_id.to_string(),
                    entity_id: story.id,
                    markdown,
                    provider_run_refs: Vec::new(),
                    review_refs: Vec::new(),
                    confirmed_by: None,
                })
                .map_err(product_store_api_error)?;
        }
    }

    for design in lifecycle
        .list_design_specs(project_id, issue_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .filter(|design| design.current_version.is_none())
    {
        if lifecycle
            .list_versions(project_id, issue_id, &design.id)
            .map_err(product_store_api_error)?
            .is_empty()
            && let Some(markdown) =
                latest_workspace_artifact_markdown(&sessions, WorkspaceType::Design, &design.id)
        {
            lifecycle
                .append_version(AppendSpecVersionInput {
                    project_id: project_id.to_string(),
                    issue_id: issue_id.to_string(),
                    entity_id: design.id,
                    markdown,
                    provider_run_refs: Vec::new(),
                    review_refs: Vec::new(),
                    confirmed_by: None,
                })
                .map_err(product_store_api_error)?;
        }
    }

    Ok(())
}

pub(crate) fn validate_confirmed_story_specs(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    story_spec_ids: &[String],
) -> ApiResult<()> {
    if story_spec_ids.is_empty() {
        return Err(ApiError::validation(
            "story_spec_required",
            "story_spec_ids is required",
        ));
    }

    let stories = lifecycle
        .list_story_specs(project_id, issue_id)
        .map_err(product_store_api_error)?;
    for story_id in story_spec_ids {
        let Some(story) = stories.iter().find(|story| story.id == *story_id) else {
            return Err(ApiError::runtime(
                "story_spec_not_found",
                "story spec not found",
                json!({}),
            ));
        };
        if story.confirmation_status != LifecycleConfirmationStatus::Confirmed {
            return Err(ApiError::validation(
                "story_spec_not_confirmed",
                "story spec must be confirmed before generating downstream artifacts",
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_confirmed_design_specs(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    design_spec_ids: &[String],
) -> ApiResult<()> {
    if design_spec_ids.is_empty() {
        return Err(ApiError::validation(
            "design_spec_required",
            "design_spec_ids is required",
        ));
    }

    let designs = lifecycle
        .list_design_specs(project_id, issue_id)
        .map_err(product_store_api_error)?;
    for design_id in design_spec_ids {
        let Some(design) = designs.iter().find(|design| design.id == *design_id) else {
            return Err(ApiError::runtime(
                "design_spec_not_found",
                "design spec not found",
                json!({}),
            ));
        };
        if design.confirmation_status != LifecycleConfirmationStatus::Confirmed {
            return Err(ApiError::validation(
                "design_spec_not_confirmed",
                "design spec must be confirmed before generating work items",
            ));
        }
    }
    Ok(())
}

pub(crate) fn confirm_workspace_entity(
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
) -> ApiResult<()> {
    match session.workspace_type {
        WorkspaceType::Story | WorkspaceType::Design => lifecycle
            .update_spec_confirmation_status(
                &session.project_id,
                &session.issue_id,
                &session.entity_id,
                LifecycleConfirmationStatus::Confirmed,
            )
            .map_err(product_store_api_error),
        WorkspaceType::WorkItem => lifecycle
            .update_work_item_plan_status(
                &session.project_id,
                &session.issue_id,
                &session.entity_id,
                WorkItemPlanStatus::Confirmed,
            )
            .map(|_| ())
            .map_err(product_store_api_error),
        WorkspaceType::WorkItemPlan => Err(ApiError::runtime(
            "work_item_plan_confirm_not_supported",
            "confirm is not yet supported for work item plan sessions",
            json!({}),
        )),
    }
}
