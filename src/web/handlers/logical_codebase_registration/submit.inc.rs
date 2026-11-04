/// Legacy `/logical-codebase/registrations` compatibility alias for the
/// default first logical codebase (v1.2 migration artifact).
pub async fn submit_logical_codebase_registration(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
    Json(request): Json<RegistrationSubmitRequest>,
) -> ApiResult<Json<RegistrationBatchDto>> {
    let paths = product_app_paths(&state);
    let logical_codebase_id = default_logical_codebase_id(&paths, &project_id)?;
    submit_registration_for_lc(state, project_id, logical_codebase_id, request)
}

/// v1.3 canonical endpoint: submission is resolved per logical codebase.
pub async fn submit_lc_registration(
    State(state): State<WebAppState>,
    Path((project_id, logical_codebase_id)): Path<(String, String)>,
    Json(request): Json<RegistrationSubmitRequest>,
) -> ApiResult<Json<RegistrationBatchDto>> {
    let paths = product_app_paths(&state);
    require_logical_codebase(&paths, &project_id, &logical_codebase_id)?;
    submit_registration_for_lc(state, project_id, logical_codebase_id, request)
}

fn submit_registration_for_lc(
    state: WebAppState,
    project_id: String,
    logical_codebase_id: String,
    request: RegistrationSubmitRequest,
) -> ApiResult<Json<RegistrationBatchDto>> {
    let paths = product_app_paths(&state);
    let snapshot =
        RegistrationPreflightSnapshotStore::for_lc(paths.clone(), logical_codebase_id.clone())
            .load_unexpired(&project_id, &request.preflight_id, Utc::now())
        .map_err(product_store_api_error)?
        .ok_or_else(|| {
            product_store_api_error(crate::product::json_store::ProductStoreError::NotFound {
                kind: "registration_preflight",
                id: request.preflight_id.clone(),
            })
        })?;
    let root = std::fs::canonicalize(&request.aggregate_root).map_err(|_| {
        ApiError::runtime(
            "aggregate_root_mismatch",
            "aggregate root does not match frozen preflight",
            serde_json::json!({}),
        )
    })?;
    if snapshot.aggregate_root != root {
        return Err(ApiError::runtime(
            "aggregate_root_mismatch",
            "aggregate root does not match frozen preflight",
            serde_json::json!({}),
        ));
    }

    // Confirmation only filters the persisted candidate list. Never rescan or
    // reconstruct candidates from caller-controlled paths: snapshot ordering,
    // identity and revision evidence stay frozen across this boundary.
    let confirmed = snapshot
        .candidates
        .iter()
        .filter(|candidate| {
            request
                .confirmed_paths
                .iter()
                .any(|path| candidate.submitted_path == std::path::Path::new(path))
        })
        .cloned()
        .collect::<Vec<_>>();
    let preflight = RegistrationPreflightResult {
        project_id: snapshot.project_id.clone(),
        aggregate_root: CanonicalAggregateRoot {
            canonical_path: snapshot.aggregate_root.clone(),
        },
        candidates: confirmed,
    };
    let include_needs_attention = preflight
        .candidates
        .iter()
        .any(|candidate| candidate.state == RegistrationCandidateState::NeedsAttention);
    let coordinator =
        LogicalCodebaseRegistrationCoordinator::for_lc(paths.clone(), logical_codebase_id.clone());
    LogicalCodebaseStore::for_lc(paths, logical_codebase_id.clone())
        .validate_registration_root(&project_id, &snapshot.aggregate_root)
        .map_err(product_store_api_error)?;
    let batch = coordinator
        .submit_confirmed_batch(ConfirmedRegistrationBatchInput::from_preflight(
            &preflight,
            include_needs_attention,
        ))
        .map_err(product_store_api_error)?;
    let completed = coordinator
        .resume_batch(&project_id, &batch.id)
        .map_err(product_store_api_error)?;
    // A（REQ-REG-10 自动推进）：登记完成即触发一次首建链自动推进——服务端
    // 后台执行，不阻塞/不改写本次提交响应；失败只 log（人工 advance 端点
    // 仍然可用）。identity → manifest/checkout 已由上面的 resume 完成，这里
    // 让 rules/policy 起的根配方长任务自动接管。
    let state_for_advance = state.clone();
    let project_for_advance = project_id.clone();
    let lc_for_advance = logical_codebase_id.clone();
    tokio::spawn(async move {
        match super::logical_codebase_bootstrap::advance_lc_bootstrap(
            &state_for_advance,
            &project_for_advance,
            &lc_for_advance,
        )
        .await
        {
            Ok(result) => {
                // 自动推进的停止解释已随 advance 落成 durable stop 记录
                //（GET 投影可见）；这里补一条服务端日志，便于事后核对是
                // 哪道门（provider 选择门/信任门）拦下的。
                tracing::info!(
                    project_id = %project_for_advance,
                    logical_codebase_id = %lc_for_advance,
                    stopped_reason = result.stopped.reason(),
                    stopped_step = result.stopped.step().map(|step| step.as_str()),
                    stopped_reason_code = result.stopped.reason_code(),
                    "bootstrap advance after registration submit stopped"
                );
            }
            Err(error) => {
                tracing::warn!(
                    project_id = %project_for_advance,
                    logical_codebase_id = %lc_for_advance,
                    code = %error.code,
                    "bootstrap advance after registration submit stopped"
                );
            }
        }
    });
    Ok(Json(RegistrationBatchDto::from_record(&completed)))
}
