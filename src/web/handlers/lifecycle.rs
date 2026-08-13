use super::dto::*;
use super::support::*;
use super::*;
use crate::product::lifecycle_store::{
    AggregateDesignSpecScope, AggregateStorySpecScope, ConfirmAggregateGateError,
};
use crate::product::logical_codebase::{
    LogicalRepositoryId, PlanningContextResolver, PlanningContextSetResolver, RepositoryRouting,
};
use crate::product::models::WorkItemRuntimeBinding;
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::product::work_item_runtime_reader::WorkItemRuntimeReader;
use crate::product::workspace_engine::group_work_items_by_target;
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
    // 方案 X 阶段1：按 RepositoryRouting 三态分流。Logical（多仓）不要求 issue.repo_id，
    // 后续经 manifest/selection 解析（聚合视野）；Legacy（单仓）保持原无条件要求 repo_id，
    // 向后兼容；FailClosed 由下游解析按稳定错误码 repository_routing_* 报错（阶段A已实现）。
    let routing = RepositoryRouting::load_for_issue(&app_paths, &project_id, &issue_id)
        .map_err(product_store_api_error)?;
    let repository_id = match routing {
        RepositoryRouting::Legacy { .. } => issue.repo_id.clone().ok_or_else(|| {
            product_store_api_error(ProductStoreError::NotFound {
                kind: "issue_repository",
                id: issue.id.clone(),
            })
        })?,
        RepositoryRouting::Logical { .. } | RepositoryRouting::FailClosed { .. } => {
            // Logical 不要求 repo_id（manifest/selection 权威）；FailClosed 在后续解析时报稳定错误码。
            issue.repo_id.clone().unwrap_or_default()
        }
    };
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
            coding_attempts.push(coding_attempt_dto(&coding_store, attempt)?);
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
            let latest_attempt = match group_attempt.as_ref() {
                Some(attempt) => Some(coding_attempt_dto(&coding_store, attempt)?),
                None => None,
            };
            work_items.push(lifecycle_work_item_runtime_dto(
                &lifecycle,
                LifecycleWorkItemRuntimeDtoInput {
                    repository_id: &repository_id,
                    plan_id: &plan.id,
                    runtime: &runtime,
                    human_projection,
                    latest_attempt,
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
            let latest_attempt = match attempts.last() {
                Some(attempt) => Some(coding_attempt_dto(&coding_store, attempt)?),
                None => None,
            };
            for attempt in &attempts {
                coding_attempts.push(coding_attempt_dto(&coding_store, attempt)?);
            }
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

    // REQ-TGT-05：按 target_repository_id 分组的聚合视图（向后兼容，保留扁平 work_items）。
    // 数据源为 Issue 下已持久化的 LifecycleWorkItemRecord（含 target_repository_id）；
    // alias 优先取 planning context 的 RepositoryContextSet.alias，缺失时回落到
    // resolve_logical_repository 的物理投影名。Plan 3 范围：schema-v2 规划投影 item
    // 尚未落库（Plan 4 贯通 publish 链路），分组视图当前反映已持久化记录。
    // list_work_items 是分组视图的主数据源：失败必须传播，禁止静默跳过（避免与扁平
    // work_items 不一致的空分组）。alias 增强（planning context / 物理投影名）保持
    // best-effort：规划上下文缺失时跳过 alias 增强是合理的。
    let persisted_work_items = lifecycle
        .list_work_items(&project_id, &issue_id)
        .map_err(product_store_api_error)?;
    let mut member_index: BTreeMap<LogicalRepositoryId, String> = BTreeMap::new();
    if let Ok(resolution) =
        PlanningContextSetResolver::new(app_paths.clone()).resolve(&project_id, &issue_id)
    {
        for member in &resolution.set {
            member_index.insert(member.member_id, member.alias.clone());
        }
    }
    let repository_store = RepositoryStore::new(app_paths.clone());
    for record in &persisted_work_items {
        if let Some(target) = record.target_repository_id {
            if member_index.contains_key(&target) {
                continue;
            }
            if let Ok((_, _, repository)) =
                repository_store.resolve_logical_repository(&project_id, target)
            {
                member_index.insert(target, repository.name.clone());
            }
        }
    }
    let groups = group_work_items_by_target(&persisted_work_items, &member_index)
        .map_err(product_store_api_error)?;
    let work_item_repository_groups = groups
        .into_iter()
        .map(|group| {
            work_item_repository_group_dto(group, |record| {
                let attempts = coding_store
                    .list_attempts_for_work_item(&project_id, &issue_id, &record.id)
                    .map_err(product_store_api_error)?;
                let latest_attempt = match attempts.last() {
                    Some(attempt) => Some(coding_attempt_dto(&coding_store, attempt)?),
                    None => None,
                };
                let session = workspace_session_for_entity(
                    &workspace_sessions,
                    &record.id,
                    &WorkspaceType::WorkItem,
                );
                lifecycle_work_item_dto(
                    &lifecycle,
                    record,
                    latest_attempt,
                    session.map(|session| session.id.as_str()),
                )
            })
        })
        .collect::<ApiResult<Vec<_>>>()?;

    let workspace_sessions = workspace_sessions
        .iter()
        .map(workspace_session_summary_dto)
        .collect();

    let delivery_summary = issue_delivery_summary_dto(
        coding_store
            .compute_issue_delivery_summary(&project_id, &issue_id)
            .map_err(product_store_api_error)?,
    );

    Ok(Json(IssueLifecycleResponse {
        issue: product_issue_dto_with_binding(&app_paths, issue)?,
        story_specs,
        design_specs,
        work_item_plans,
        work_items,
        work_item_repository_groups,
        workspace_sessions,
        coding_attempts,
        delivery_summary,
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
    let lifecycle = LifecycleStore::new(app_paths.clone());
    // 方案 X 阶段1：按 RepositoryRouting 三态分流。Legacy（单仓）保持原 repository_id 校验 +
    // find_repository；Logical（多仓）接 PlanningContextResolver + 草稿态聚合视野
    // （aggregate_codebase=Some，involved 空由 AI 自决，create 跳过 repository_id 校验）；
    // FailClosed 报稳定错误码 repository_routing_*。
    let routing = RepositoryRouting::load_for_issue(&app_paths, &project_id, &issue_id)
        .map_err(product_store_api_error)?;
    let (repository_id, aggregate_codebase) = match routing {
        RepositoryRouting::Legacy { .. } => {
            let repository_id = issue.repo_id.clone().ok_or_else(|| {
                ApiError::validation("repository_required", "repository_id is required")
            })?;
            find_repository(&app_paths, &project_id, &repository_id)?;
            (repository_id, None)
        }
        RepositoryRouting::Logical { manifest, .. } => {
            // targets 空 → AI 自决 involved；snapshot 为权威 effective_member_ids。
            let resolved = PlanningContextResolver::new(app_paths.clone())
                .build(&project_id, &issue_id, &[])
                .map_err(product_store_api_error)?;
            let scope = AggregateStorySpecScope {
                logical_codebase_ref: manifest.logical_codebase_id,
                effective_member_ids: resolved.snapshot.effective_member_ids.clone(),
                involved_repository_ids: Vec::new(),
                focus_repository_id: None,
            };
            (String::new(), Some(scope))
        }
        RepositoryRouting::FailClosed { code, reason } => {
            return Err(routing_api_error(code, &reason));
        }
    };
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            repository_id,
            title: request.title,
            aggregate_codebase,
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
    // 方案 X 阶段1：Logical（多仓）接 PlanningContextResolver + 草稿态聚合视野
    // （aggregate_codebase=Some，involved/change_order 空由 AI 自决）；Legacy 保持现状
    // （aggregate_codebase=None）；FailClosed 报稳定错误码 repository_routing_*。
    let routing = RepositoryRouting::load_for_issue(&app_paths, &project_id, &issue_id)
        .map_err(product_store_api_error)?;
    let aggregate_codebase = match routing {
        RepositoryRouting::Legacy { .. } => None,
        RepositoryRouting::Logical { manifest, .. } => {
            let resolved = PlanningContextResolver::new(app_paths.clone())
                .build(&project_id, &issue_id, &[])
                .map_err(product_store_api_error)?;
            Some(AggregateDesignSpecScope {
                logical_codebase_ref: manifest.logical_codebase_id,
                effective_member_ids: resolved.snapshot.effective_member_ids.clone(),
                involved_repository_ids: Vec::new(),
                change_order: Vec::new(),
            })
        }
        RepositoryRouting::FailClosed { code, reason } => {
            return Err(routing_api_error(code, &reason));
        }
    };
    let design = lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: project_id.clone(),
            issue_id: issue_id.clone(),
            story_spec_ids: request.story_spec_ids,
            title: request.title,
            aggregate_codebase,
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
    let lifecycle = LifecycleStore::new(app_paths.clone());
    validate_confirmed_story_specs(&lifecycle, &project_id, &issue_id, &request.story_spec_ids)?;
    validate_confirmed_design_specs(&lifecycle, &project_id, &issue_id, &request.design_spec_ids)?;
    // 方案 X 阶段1：按 RepositoryRouting 三态分流（与 generate_story/design 一致）。
    // Legacy（单仓）保持原 repository_id 校验 + find_repository；Logical（多仓）targets =
    // confirmed design 的 involved，经 PlanningContextResolver 校验 target ∈ selection
    // （REQ-TGT-01），无单仓 repo_id；FailClosed 报稳定错误码 repository_routing_*。
    let routing = RepositoryRouting::load_for_issue(&app_paths, &project_id, &issue_id)
        .map_err(product_store_api_error)?;
    match routing {
        RepositoryRouting::Legacy { .. } => {
            let repository_id = issue.repo_id.clone().ok_or_else(|| {
                ApiError::validation("repository_required", "repository_id is required")
            })?;
            find_repository(&app_paths, &project_id, &repository_id)?;
        }
        RepositoryRouting::Logical { .. } => {
            // targets = confirmed design 的 involved（validate_confirmed_design_specs 已保证
            // design_spec_ids 非空且 Confirmed，首项必存在）。
            let designs = lifecycle
                .list_design_specs(&project_id, &issue_id)
                .map_err(product_store_api_error)?;
            let design = designs
                .iter()
                .find(|design| design.id == request.design_spec_ids[0])
                .ok_or_else(|| {
                    product_store_api_error(ProductStoreError::NotFound {
                        kind: "design_spec",
                        id: request.design_spec_ids[0].clone(),
                    })
                })?;
            let resolved = PlanningContextResolver::new(app_paths.clone())
                .build(&project_id, &issue_id, &design.involved_repository_ids)
                .map_err(product_store_api_error)?;
            // REQ-TGT-01：design involved 必须 ⊆ selection 有效成员，否则 4xx blocker。
            for target in &design.involved_repository_ids {
                if !resolved.snapshot.effective_member_ids.contains(target) {
                    return Err(ApiError::validation(
                        "target_not_in_selection",
                        format!("design involved {target:?} is not in issue codebase selection"),
                    ));
                }
            }
        }
        RepositoryRouting::FailClosed { code, reason } => {
            return Err(routing_api_error(code, &reason));
        }
    };

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

/// confirm gate 失败映射（#5 收尾）：按 [`ConfirmAggregateGateError`] 变体区分 HTTP 状态。
/// - Violation（业务违规）→ 4xx + 稳定码（error.rs 显式映射 400）；
/// - SpecNotFound → 404 `spec_not_found`；
/// - SpecLoad → 500 `confirm_gate_spec_load_failed`。
fn confirm_gate_api_error(error: ConfirmAggregateGateError) -> ApiError {
    let code = error.stable_code();
    let message = error.message();
    match error {
        ConfirmAggregateGateError::Violation { .. } => ApiError::validation(code, message),
        ConfirmAggregateGateError::SpecNotFound(_) => ApiError::validation(code, message),
        ConfirmAggregateGateError::SpecLoad(_) => ApiError::runtime(code, message, json!({})),
    }
}

pub(crate) fn confirm_workspace_entity(
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
) -> ApiResult<()> {
    match session.workspace_type {
        WorkspaceType::Story | WorkspaceType::Design => {
            // confirm gate（方案 X 3b 收紧，Blocker 2 下沉到 product 层）：多仓 Spec
            // （logical_codebase_ref Some）在 Confirmed 前校验 ① involved 非空（REQ-PLN-04）；
            // ② Design involved>1 必须 change_order（REQ-PLN-05 收紧）。单仓不校验（红线）。
            // #5 收尾：load_existing_spec 失败按 NotFound→404 / IO→500 区分（修复前统一 500）。
            if let Err(error) = lifecycle.validate_confirm_aggregate_spec(
                &session.project_id,
                &session.issue_id,
                &session.entity_id,
                &session.workspace_type,
            ) {
                return Err(confirm_gate_api_error(error));
            }
            lifecycle
                .update_spec_confirmation_status(
                    &session.project_id,
                    &session.issue_id,
                    &session.entity_id,
                    LifecycleConfirmationStatus::Confirmed,
                )
                .map_err(product_store_api_error)
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    include!("lifecycle_tests.inc.rs");
}
