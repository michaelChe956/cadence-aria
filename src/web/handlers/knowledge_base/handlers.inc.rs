// 本片段经 `include!` 进入 `knowledge_base` handler 模块（Task 7）。
//
// 端点表（实施计划 Task 7，前缀 `/logical-codebases/{lc_id}/knowledge-base`）：
// - POST   `/initializations`                     202 + operation 投影
// - GET    `/initializations/active`              operation 投影 / 404
// - GET    `/initializations/{operation_id}`      operation 投影
// - POST   `/initializations/{operation_id}/cancel` operation 投影（无请求体）
// - GET    `/status`                              状态投影
// - GET    `/files` · GET/PUT `/file`             文件面
// - GET/POST `/change-packages` · GET `/change-packages/lint`
// - POST   `/bootstrap-inputs`

// ───────────────────────── 稳定码 ─────────────────────────

const CODE_REQUIRES_AGGREGATE_INITIALIZATION: &str =
    "knowledge_base_requires_aggregate_initialization";
const CODE_SKILLS_UNAVAILABLE: &str = "knowledge_base_skills_unavailable";
const CODE_INPUTS_INCOMPLETE: &str = "knowledge_base_inputs_incomplete";
const CODE_MANIFEST_MISSING: &str = "knowledge_base_manifest_missing";
const CODE_MANIFEST_VERSION_UNSUPPORTED: &str = "knowledge_base_manifest_version_unsupported";
const CODE_NOT_INITIALIZED: &str = "knowledge_base_not_initialized";
const CODE_CHANGE_PACKAGE_INVALID: &str = "knowledge_base_change_package_invalid";
const CODE_CHANGE_PACKAGE_PATH_INVALID: &str = "knowledge_base_change_package_path_invalid";
const CODE_OPERATION_RUNNING: &str = "knowledge_base_operation_running";
const CODE_OPERATION_NOT_FOUND: &str = "knowledge_base_operation_not_found";
const CODE_WRITE_SCOPE_VIOLATION: &str = "knowledge_base_write_scope_violation";
const CODE_OTHER_RUNNING: &str = "other_knowledge_base_running";
const CODE_PROVIDER_CAPABILITY_DENIED: &str = "knowledge_base_provider_capability_denied";
const CODE_INVALID_REQUEST: &str = "invalid_knowledge_base_request";

// ───────────────────────── 请求 DTO ─────────────────────────

/// POST `/initializations` 请求体（前端 `startKnowledgeBaseInitialization`）。
#[derive(Debug, Deserialize)]
pub struct CreateKnowledgeBaseInitializationRequest {
    /// bootstrap | update。
    pub kind: String,
    pub idempotency_key: String,
    /// 相对知识库根的变更包目录（update 必填）。
    #[serde(default)]
    pub package_path: Option<String>,
}

/// PUT `/file` 请求体（前端 `writeKnowledgeBaseFile`）。
#[derive(Debug, Deserialize)]
pub struct WriteKnowledgeBaseFileRequest {
    pub path: String,
    pub content: String,
    #[serde(default)]
    pub base_digest: Option<String>,
}

/// POST `/change-packages` 请求体（前端 `createKnowledgeBaseChangePackage`）。
#[derive(Debug, Deserialize)]
pub struct CreateKnowledgeBaseChangePackageRequest {
    pub identifier: String,
}

/// `?path=` 查询参数。
#[derive(Debug, Deserialize)]
pub struct KnowledgeBasePathQuery {
    pub path: String,
}

// ───────────────────────── 错误映射 ─────────────────────────

/// 单条变更包门禁违规 → 前端 `KnowledgeBaseChangePackageViolationDto`。
fn change_package_violation_dto(violation: &ChangePackageViolation) -> Value {
    json!({
        "code": violation.code,
        "document": violation.document,
        "field": violation.field,
        "detail": violation.detail,
    })
}

/// 前置检查失败 → HTTP 错误面（请求期产物，不产生 operation；缺失文件与
/// 逐条 violation 都放进 `details` 供前端「最近一次预检失败」卡片渲染）。
fn knowledge_base_preflight_api_error(failure: KnowledgeBasePreflightFailure) -> ApiError {
    match failure {
        KnowledgeBasePreflightFailure::RequiresAggregateInitialization => ApiError::runtime(
            CODE_REQUIRES_AGGREGATE_INITIALIZATION,
            "aggregate initialization has not completed for this logical codebase",
            json!({}),
        ),
        KnowledgeBasePreflightFailure::SkillsUnavailable { missing } => ApiError::runtime(
            CODE_SKILLS_UNAVAILABLE,
            "the selected provider's skill layer is missing knowledge-base skills",
            json!({ "missing": missing }),
        ),
        KnowledgeBasePreflightFailure::InputsIncomplete { missing } => ApiError::runtime(
            CODE_INPUTS_INCOMPLETE,
            "bootstrap inputs under user-input/ are incomplete",
            json!({ "missing": missing }),
        ),
        KnowledgeBasePreflightFailure::ManifestMissing => ApiError::runtime(
            CODE_MANIFEST_MISSING,
            "knowledge base manifest.yaml is missing",
            json!({}),
        ),
        KnowledgeBasePreflightFailure::ManifestVersionUnsupported { found } => ApiError::runtime(
            CODE_MANIFEST_VERSION_UNSUPPORTED,
            "knowledge base manifest schema_version is not \"4.0\"",
            json!({ "found": found }),
        ),
        KnowledgeBasePreflightFailure::NotInitialized => ApiError::runtime(
            CODE_NOT_INITIALIZED,
            "knowledge base initialization is not complete",
            json!({}),
        ),
        KnowledgeBasePreflightFailure::ChangePackageInvalid { violations } => ApiError::runtime(
            CODE_CHANGE_PACKAGE_INVALID,
            "change package failed the mechanical gate",
            json!({
                "violations": violations
                    .iter()
                    .map(change_package_violation_dto)
                    .collect::<Vec<_>>(),
            }),
        ),
        KnowledgeBasePreflightFailure::ProviderSelection(outcome) => {
            knowledge_base_selection_api_error(&outcome)
        }
        KnowledgeBasePreflightFailure::CapabilityDenied { provider } => ApiError::runtime(
            CODE_PROVIDER_CAPABILITY_DENIED,
            "the selected provider denied the CodingTargetWrite capability cell",
            json!({ "provider": provider }),
        ),
        KnowledgeBasePreflightFailure::OtherKnowledgeBaseRunning { operation_id } => {
            ApiError::runtime(
                CODE_OTHER_RUNNING,
                "another knowledge base operation is already running for this logical codebase",
                json!({ "operation_id": operation_id }),
            )
        }
    }
}

/// provider 选择失败：**选择器失败码与候选表原样透出**（聚合初始化同款四枚
/// 选择期失败码；候选表走与聚合 DTO 相同的字段集，前端两种失败卡同构）。
fn knowledge_base_selection_api_error(outcome: &ProviderSelectionOutcome) -> ApiError {
    let code = outcome.reason_code().unwrap_or(
        crate::product::logical_codebase::provider_selection::REASON_CODE_PROVIDER_HEALTH_DEGRADED,
    );
    let message = match outcome {
        ProviderSelectionOutcome::NoCliAvailable(_) => {
            "no candidate provider CLI is available for the knowledge base write cell"
        }
        ProviderSelectionOutcome::NeedsVerification { .. } => {
            "no candidate provider has verified capability for the knowledge base write cell"
        }
        ProviderSelectionOutcome::Denied { .. } => {
            "every candidate provider denied the knowledge base write cell"
        }
        ProviderSelectionOutcome::HealthDegraded { .. } => {
            "provider health is degraded; the write cell selection cannot run"
        }
        ProviderSelectionOutcome::Selected(_) => "provider selection did not select a provider",
    };
    let mut details = serde_json::Map::new();
    if let Some(table) = outcome.table() {
        details.insert("candidates".into(), knowledge_base_selection_dto(table));
    }
    if let ProviderSelectionOutcome::HealthDegraded { detail } = outcome {
        details.insert("detail".into(), json!(detail));
    }
    ApiError::runtime(code, message, Value::Object(details))
}

/// durable store 错误 → HTTP 错误面：缺记录 404，状态机冲突 409。
fn knowledge_base_store_api_error(
    error: crate::product::json_store::ProductStoreError,
) -> ApiError {
    match error {
        crate::product::json_store::ProductStoreError::NotFound { .. } => {
            knowledge_base_operation_not_found_error()
        }
        crate::product::json_store::ProductStoreError::IdentityMismatch { .. } => {
            ApiError::runtime(
                CODE_OPERATION_RUNNING,
                "knowledge base operation is not in the required state",
                json!({}),
            )
        }
        other => product_store_api_error(other),
    }
}

/// 执行层错误 → HTTP 错误面。
fn knowledge_base_error_api_error(error: KnowledgeBaseError) -> ApiError {
    match error {
        KnowledgeBaseError::Store(store_error) => knowledge_base_store_api_error(store_error),
        KnowledgeBaseError::Preflight(failure) => knowledge_base_preflight_api_error(failure),
        KnowledgeBaseError::TrustWaiting(waiting) => ApiError::runtime(
            "aggregate_initialization_trust_waiting",
            waiting.message.clone(),
            json!({ "provider": format!("{:?}", waiting.provider) }),
        ),
        KnowledgeBaseError::InvalidInput(detail) => ApiError::validation(
            CODE_INVALID_REQUEST,
            format!("invalid knowledge base operation input: {detail}"),
        ),
        KnowledgeBaseError::NotRunning {
            operation_id,
            status,
        } => ApiError::runtime(
            CODE_OPERATION_RUNNING,
            "knowledge base operation is not running",
            json!({ "operation_id": operation_id, "status": status }),
        ),
        KnowledgeBaseError::Cancelled => ApiError::runtime(
            CODE_OPERATION_RUNNING,
            "knowledge base operation was cancelled",
            json!({}),
        ),
        KnowledgeBaseError::TurnFailed {
            reason_code,
            summary,
            retryable,
        } => ApiError::runtime(reason_code, summary, json!({ "retryable": retryable })),
        KnowledgeBaseError::WriteScopeViolation { paths } => ApiError::runtime(
            CODE_WRITE_SCOPE_VIOLATION,
            "provider wrote outside the knowledge base write scope",
            json!({ "paths": paths }),
        ),
        KnowledgeBaseError::Io(detail) => ApiError::runtime(
            "knowledge_base_io",
            format!("knowledge base io failure: {detail}"),
            json!({}),
        ),
    }
}

/// 文件面错误：稳定码直接取 `KnowledgeBaseFileError` 的 Display 形态
/// （Task 5 已固定），HTTP 状态由 `web/error.rs` 登记。
fn knowledge_base_file_api_error(error: KnowledgeBaseFileError) -> ApiError {
    let message = match error {
        KnowledgeBaseFileError::PathEscapes => "path escapes the knowledge base root",
        KnowledgeBaseFileError::NotFound => "file not found inside the knowledge base",
        KnowledgeBaseFileError::NotEditable => "path is outside the editable whitelist",
        KnowledgeBaseFileError::TooLarge => "file exceeds the 1 MiB size limit",
        KnowledgeBaseFileError::Conflict => "base digest does not match the current content",
        KnowledgeBaseFileError::OperationRunning => {
            "a knowledge base operation is running; file writes are suspended"
        }
    };
    ApiError::runtime(error.to_string(), message, json!({}))
}

fn knowledge_base_operation_not_found_error() -> ApiError {
    ApiError::runtime(
        CODE_OPERATION_NOT_FOUND,
        "knowledge base operation not found",
        json!({}),
    )
}

fn knowledge_base_invalid_request(message: impl Into<String>) -> ApiError {
    ApiError::validation(CODE_INVALID_REQUEST, message)
}

// ───────────────────────── operation 投影 ─────────────────────────

/// 选择表投影（与聚合初始化 DTO 同一字段集；候选行的可空字段显式 null）。
fn knowledge_base_selection_dto(selection: &AggregateProviderSelection) -> Value {
    json!({
        "selected": selection.selected,
        "candidates": selection
            .candidates
            .iter()
            .map(|candidate| {
                json!({
                    "provider": candidate.provider,
                    "display_name": candidate.display_name,
                    "cli_available": candidate.cli_available,
                    "cli_reason_code": candidate.cli_reason_code,
                    "capability_state": candidate.capability_state,
                    "skipped_reason": candidate.skipped_reason,
                    "action_hint": candidate.action_hint,
                    "warning": candidate.warning,
                })
            })
            .collect::<Vec<_>>(),
        "rule_version": selection.rule_version,
        "decided_at": selection.decided_at,
    })
}

/// operation → 前端 `KnowledgeBaseOperationSnapshotDto`（字段逐字对齐；
/// 可空字段显式 null，不缺键）。
fn knowledge_base_operation_dto(operation: &KnowledgeBaseOperation) -> Value {
    let kind = match operation.kind {
        KnowledgeBaseOperationKind::Bootstrap => "bootstrap",
        KnowledgeBaseOperationKind::Update => "update",
    };
    let status = match operation.status {
        crate::product::logical_codebase::knowledge_base::operation::KnowledgeBaseOperationStatus::Running => "running",
        crate::product::logical_codebase::knowledge_base::operation::KnowledgeBaseOperationStatus::Completed => "completed",
        crate::product::logical_codebase::knowledge_base::operation::KnowledgeBaseOperationStatus::Failed => "failed",
        crate::product::logical_codebase::knowledge_base::operation::KnowledgeBaseOperationStatus::Cancelled => "cancelled",
    };
    let steps = operation
        .steps
        .iter()
        .map(|step| {
            let step_id = match step.step_id {
                crate::product::logical_codebase::knowledge_base::operation::KnowledgeBaseStepKind::Bootstrap => "bootstrap",
                crate::product::logical_codebase::knowledge_base::operation::KnowledgeBaseStepKind::ApplyUpdate => "apply_update",
            };
            let step_status = match step.status {
                crate::product::logical_codebase::knowledge_base::operation::KnowledgeBaseStepStatus::Pending => "pending",
                crate::product::logical_codebase::knowledge_base::operation::KnowledgeBaseStepStatus::Running => "running",
                crate::product::logical_codebase::knowledge_base::operation::KnowledgeBaseStepStatus::Completed => "completed",
                crate::product::logical_codebase::knowledge_base::operation::KnowledgeBaseStepStatus::Failed => "failed",
            };
            json!({
                "step_id": step_id,
                "status": step_status,
                "started_at": step.started_at,
                "completed_at": step.completed_at,
                "output_summary": step.output_summary,
            })
        })
        .collect::<Vec<_>>();
    json!({
        "operation_id": operation.operation_id,
        "project_id": operation.project_id,
        "logical_codebase_id": operation.logical_codebase_id,
        "kind": kind,
        "input": {
            "idempotency_key": operation.input.idempotency_key,
            "provider": operation.input.provider,
            "package_path": operation.input.package_path,
            "package_id": operation.input.package_id,
        },
        "status": status,
        "steps": steps,
        "provider_selection": operation.provider_selection.as_ref().map(knowledge_base_selection_dto),
        "error": operation
            .error
            .as_ref()
            .map(|record| serde_json::to_value(ApiError::from(record.clone())).unwrap_or(Value::Null)),
        "created_at": operation.created_at,
        "started_at": operation.started_at,
        "completed_at": operation.completed_at,
    })
}

// ───────────────────────── operation 生命周期端点 ─────────────────────────

/// POST `/initializations`：解析 LC → provider 选择（写分格，失败码原样透出、
/// 不创建 operation）→ 前置检查（失败即返回，不创建 operation）→ trust 门 →
/// 冻结输入 + 幂等 begin → run lease + spawn 执行。202 同步返回 operation 投影。
///
/// 注：预检②/命令构造都需要 provider，故 provider 选择先行——与「选择必须
/// 在创建 operation 之前」的约束一致；选择失败码与 details 原样透出。
pub async fn create_knowledge_base_initialization(
    State(state): State<WebAppState>,
    Path((project_id, lc_id)): Path<(String, String)>,
    Json(request): Json<CreateKnowledgeBaseInitializationRequest>,
) -> ApiResult<Response> {
    validate_knowledge_base_ids(&project_id, &lc_id)?;
    if request.idempotency_key.trim().is_empty() {
        return Err(knowledge_base_invalid_request(
            "idempotency_key must not be empty",
        ));
    }
    let kind = match request.kind.as_str() {
        "bootstrap" => KnowledgeBaseOperationKind::Bootstrap,
        "update" => KnowledgeBaseOperationKind::Update,
        other => {
            return Err(knowledge_base_invalid_request(format!(
                "kind must be \"bootstrap\" or \"update\", got {other:?}"
            )));
        }
    };
    // 变更包路径（相对知识库根）双形态：给 preflight 的目录与给命令/冻结输入
    // 的相对聚合根路径必须指向**同一个包**（门禁与实际执行对得上）。
    let (package_kb_rel, package_root_rel) = match kind {
        KnowledgeBaseOperationKind::Bootstrap => (None, None),
        KnowledgeBaseOperationKind::Update => {
            let package_path = request.package_path.clone().ok_or_else(|| {
                knowledge_base_invalid_request("package_path is required for update")
            })?;
            validate_change_package_rel_path(&package_path)?;
            let root_rel = format!("cadence/knowledge-base/{package_path}");
            (Some(package_path), Some(root_rel))
        }
    };

    let paths = product_app_paths(&state);
    let record = require_logical_codebase(&paths, &project_id, &lc_id)?;
    let canonical_root = fs::canonicalize(&record.aggregate_root).map_err(|error| {
        ApiError::runtime(
            "aggregate_root_missing",
            format!(
                "canonicalize aggregate root {}: {error}",
                record.aggregate_root.display()
            ),
            json!({}),
        )
    })?;

    // ① provider 选择（写分格选择器，零写入）。
    let outcome = select_aggregate_provider_for_write(&paths, &state.provider_health, &project_id)
        .await
        .map_err(product_store_api_error)?;
    let ProviderSelectionOutcome::Selected(selection) = outcome else {
        return Err(knowledge_base_selection_api_error(&outcome));
    };
    let provider = provider_from_wire(&selection.selected).ok_or_else(|| {
        ApiError::runtime(
            crate::product::logical_codebase::provider_selection::REASON_CODE_PROVIDER_HEALTH_DEGRADED,
            "provider selection produced an unknown provider wire name",
            json!({}),
        )
    })?;
    // 命令可构造性（provider 无 skill 前缀 ⇒ 知识库面不可用）。
    let command_constructible = match kind {
        KnowledgeBaseOperationKind::Bootstrap => bootstrap_command(&provider).is_some(),
        KnowledgeBaseOperationKind::Update => {
            update_command(&provider, package_root_rel.as_deref().unwrap_or_default()).is_some()
        }
    };
    if !command_constructible {
        return Err(ApiError::runtime(
            CODE_SKILLS_UNAVAILABLE,
            "the selected provider has no knowledge-base skill command prefix",
            json!({ "missing": [] }),
        ));
    }

    // ② 前置检查（只读、零副作用；失败不创建 operation）。
    let members = knowledge_base_member_repos(&paths, &project_id, &lc_id)?;
    let preflight_result = match kind {
        KnowledgeBaseOperationKind::Bootstrap => {
            preflight_bootstrap(&paths, &lc_id, &canonical_root, &provider)
        }
        KnowledgeBaseOperationKind::Update => preflight_update(
            &paths,
            &lc_id,
            &canonical_root,
            &provider,
            &kb_root(&canonical_root).join(package_kb_rel.as_deref().unwrap_or_default()),
            &members,
        ),
    };
    preflight_result.map_err(knowledge_base_preflight_api_error)?;
    // 前置检查全部通过后再组装执行依赖（gateway 需要 LC 政策 artifact，而
    // 预检①已证明其在场；预检/选择失败期零 gateway、零 operation）。
    let dependencies = knowledge_base_dependencies(&state, &project_id, &lc_id)?;

    // ③ trust 硬前置门（与聚合面同源；未装配的依赖图 no-op）。
    let operation_id = derive_operation_id(&project_id, &lc_id, &request.idempotency_key);
    dependencies
        .coordinator
        .ensure_trust_ready(&project_id, &operation_id, &canonical_root, &provider)
        .await
        .map_err(knowledge_base_error_api_error)?;

    // ④ 冻结输入 + 幂等 begin（同 id 重放返回既有记录；单飞冲突 409）。
    let input = KnowledgeBaseOperationInput {
        idempotency_key: request.idempotency_key.clone(),
        provider: selection.selected.clone(),
        package_path: package_root_rel.clone(),
        package_id: package_kb_rel
            .as_ref()
            .and_then(|path| StdPath::new(path).file_name())
            .and_then(|name| name.to_str())
            .map(str::to_string),
    };
    let operation = dependencies
        .coordinator
        .begin(
            operation_id.clone(),
            &project_id,
            &lc_id,
            kind,
            input,
            selection.clone(),
        )
        .await
        .map_err(knowledge_base_error_api_error)?;

    // ⑤ run lease + spawn 执行（lease 存活期覆盖整个 execute，取消可命中）。
    let key = InitializationRunKey::knowledge_base(
        project_id.clone(),
        lc_id.clone(),
        operation_id.clone(),
    );
    let lease = dependencies.runs.register(key.clone()).ok_or_else(|| {
        ApiError::runtime(
            CODE_OTHER_RUNNING,
            "another knowledge base operation is already running for this logical codebase",
            json!({ "operation_id": operation_id }),
        )
    })?;
    let token = lease.cancellation_token();
    let coordinator = dependencies.coordinator.clone();
    let worker_project = project_id.clone();
    let worker_operation = operation_id.clone();
    let worker_root = canonical_root.clone();
    tokio::spawn(async move {
        let _lease = lease;
        if let Err(error) = coordinator
            .execute(&worker_project, &worker_operation, &worker_root, token)
            .await
        {
            tracing::warn!(
                project_id = %worker_project,
                operation_id = %worker_operation,
                error = %error,
                "knowledge base worker stopped"
            );
        }
    });
    Ok((
        StatusCode::ACCEPTED,
        Json(knowledge_base_operation_dto(&operation)),
    )
        .into_response())
}

/// GET `/initializations/active`：LC 最新一条 operation（任意状态）；无记录 →
/// 404 `knowledge_base_operation_not_found`（前端解析为 null 的正常态）。
pub async fn get_active_knowledge_base_initialization(
    State(state): State<WebAppState>,
    Path((project_id, lc_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let (_, _, store) = knowledge_base_read_context(&state, &project_id, &lc_id)?;
    let operation = store
        .latest(&project_id)
        .map_err(product_store_api_error)?
        .ok_or_else(knowledge_base_operation_not_found_error)?;
    Ok(Json(knowledge_base_operation_dto(&operation)).into_response())
}

/// GET `/initializations/{operation_id}`：operation 纯投影。
pub async fn get_knowledge_base_initialization(
    State(state): State<WebAppState>,
    Path((project_id, lc_id, operation_id)): Path<(String, String, String)>,
) -> ApiResult<Response> {
    validate_knowledge_base_ids(&project_id, &lc_id)?;
    validate_relative_id(&operation_id).map_err(|error| {
        ApiError::validation(
            "invalid_operation_id",
            format!("invalid operation id: {error}"),
        )
    })?;
    let (_, _, store) = knowledge_base_read_context(&state, &project_id, &lc_id)?;
    let operation = store
        .get(&project_id, &operation_id)
        .map_err(product_store_api_error)?;
    Ok(Json(knowledge_base_operation_dto(&operation)).into_response())
}

/// POST `/initializations/{operation_id}/cancel`：durable 取消先落盘，再经
/// run registry 通知在跑 worker 的取消令牌。前端不带请求体。
pub async fn cancel_knowledge_base_initialization(
    State(state): State<WebAppState>,
    Path((project_id, lc_id, operation_id)): Path<(String, String, String)>,
) -> ApiResult<Response> {
    validate_knowledge_base_ids(&project_id, &lc_id)?;
    validate_relative_id(&operation_id).map_err(|error| {
        ApiError::validation(
            "invalid_operation_id",
            format!("invalid operation id: {error}"),
        )
    })?;
    // 取消是纯 durable 写 + run 令牌通知：不建 gateway（政策 artifact 缺失
    // 也不阻碍取消在跑/遗留的 operation）。
    let (_, _, store) = knowledge_base_read_context(&state, &project_id, &lc_id)?;
    store
        .cancel(&project_id, &operation_id, chrono::Utc::now().to_rfc3339())
        .map_err(knowledge_base_store_api_error)?;
    let runs = state.aggregate_initialization_dependencies().runs().clone();
    runs.cancel(&InitializationRunKey::knowledge_base(
        project_id.clone(),
        lc_id.clone(),
        operation_id.clone(),
    ));
    let operation = store
        .get(&project_id, &operation_id)
        .map_err(product_store_api_error)?;
    Ok((
        StatusCode::OK,
        Json(knowledge_base_operation_dto(&operation)),
    )
        .into_response())
}

// ───────────────────────── 状态投影端点 ─────────────────────────

/// GET `/status`：manifest 只读投影 + skills 可用性（Task 4 判定，不重写名单）。
pub async fn get_knowledge_base_status(
    State(state): State<WebAppState>,
    Path((project_id, lc_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let (paths, canonical_root, _) = knowledge_base_read_context(&state, &project_id, &lc_id)?;
    let snapshot = read_manifest(&kb_root(&canonical_root)).map_err(product_store_api_error)?;
    let skills_available =
        knowledge_base_skills_available(&state, &paths, &project_id, &lc_id, &canonical_root).await;
    Ok(Json(json!({
        "exists": snapshot.exists,
        "schema_version": snapshot.schema_version,
        "initialization_status": snapshot.initialization_status,
        "global_validation": snapshot.global_validation,
        "completed_stages": snapshot.completed_stages,
        "skipped_stages": snapshot
            .skipped_stages
            .iter()
            .map(|stage| json!({ "stage": stage.stage, "reason": stage.reason }))
            .collect::<Vec<_>>(),
        "skills_available": skills_available,
    }))
    .into_response())
}

/// skills 可用性：被选 provider（写分格选择器，读快照零探针；选择失败 ⇒
/// 证明不了在场，fail-closed false）→ 复用 `preflight_bootstrap` 的 skills
/// 判定。预检顺序固定 ①→②→③：② 之后的任何结果都证明七个 skill 齐备；
/// ①（聚合初始化未完成）先失败时判 false（与 Task 4 的 fail-closed 口径一致）。
async fn knowledge_base_skills_available(
    state: &WebAppState,
    paths: &ProductAppPaths,
    project_id: &str,
    lc_id: &str,
    canonical_root: &StdPath,
) -> bool {
    let selection = select_aggregate_provider_for_write(paths, &state.provider_health, project_id)
        .await
        .ok();
    let Some(ProviderSelectionOutcome::Selected(selection)) = selection else {
        return false;
    };
    let Some(provider) = provider_from_wire(&selection.selected) else {
        return false;
    };
    match preflight_bootstrap(paths, lc_id, canonical_root, &provider) {
        Ok(()) => true,
        Err(KnowledgeBasePreflightFailure::SkillsUnavailable { .. })
        | Err(KnowledgeBasePreflightFailure::RequiresAggregateInitialization) => false,
        Err(_) => true,
    }
}

// ───────────────────────── 文件面端点 ─────────────────────────

/// GET `/files`：知识库文件树（未初始化 ⇒ 空列表）。
pub async fn list_knowledge_base_file_nodes(
    State(state): State<WebAppState>,
    Path((project_id, lc_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let (_, canonical_root, _) = knowledge_base_read_context(&state, &project_id, &lc_id)?;
    let nodes = list_knowledge_base_files(&canonical_root).map_err(product_store_api_error)?;
    let nodes = nodes
        .iter()
        .map(|node| {
            json!({
                "path": node.path,
                "is_dir": node.is_dir,
                "size": node.size,
                "editable": node.editable,
            })
        })
        .collect::<Vec<_>>();
    Ok(Json(json!({ "nodes": nodes })).into_response())
}

/// GET `/file?path=`：读取单文件（内容 + sha256 摘要）。
pub async fn read_knowledge_base_file_content(
    State(state): State<WebAppState>,
    Path((project_id, lc_id)): Path<(String, String)>,
    Query(query): Query<KnowledgeBasePathQuery>,
) -> ApiResult<Response> {
    let (_, canonical_root, _) = knowledge_base_read_context(&state, &project_id, &lc_id)?;
    let content = read_knowledge_base_file(&canonical_root, &query.path)
        .map_err(knowledge_base_file_api_error)?;
    Ok(Json(json!({
        "path": content.path,
        "content": content.content,
        "digest": content.digest,
    }))
    .into_response())
}

/// PUT `/file`：写入单文件（白名单 + 乐观并发 + 运行期禁写）。
pub async fn write_knowledge_base_file_content(
    State(state): State<WebAppState>,
    Path((project_id, lc_id)): Path<(String, String)>,
    Json(request): Json<WriteKnowledgeBaseFileRequest>,
) -> ApiResult<Response> {
    let (_, canonical_root, store) = knowledge_base_read_context(&state, &project_id, &lc_id)?;
    let operation_running = store
        .list_running(&project_id)
        .map_err(product_store_api_error)?
        .iter()
        .any(|operation| matches!(operation.status, KnowledgeBaseOperationStatus::Running));
    let digest = write_knowledge_base_file(
        &canonical_root,
        &request.path,
        &request.content,
        request.base_digest.as_deref(),
        operation_running,
    )
    .map_err(knowledge_base_file_api_error)?;
    Ok(Json(json!({
        "path": request.path,
        "digest": digest,
    }))
    .into_response())
}

// ───────────────────────── 变更包端点 ─────────────────────────

/// GET `/change-packages`：扫 `user-input/updates/` 下的 `CHANGE-*` 目录并逐个
/// lint（单包 lint 异常不阻断整列表，标记为该包的违规）。
pub async fn list_knowledge_base_change_packages(
    State(state): State<WebAppState>,
    Path((project_id, lc_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let (paths, canonical_root, _) = knowledge_base_read_context(&state, &project_id, &lc_id)?;
    let kb = kb_root(&canonical_root);
    let snapshot = read_manifest(&kb).map_err(product_store_api_error)?;
    let members = knowledge_base_member_repos(&paths, &project_id, &lc_id)?;
    let updates_dir = kb.join("user-input").join("updates");
    let mut packages: Vec<(String, Value)> = Vec::new();
    let entries = match fs::read_dir(&updates_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Json(json!({ "packages": [] })).into_response());
        }
        Err(error) => {
            return Err(ApiError::runtime(
                "knowledge_base_io",
                format!("read {}: {error}", updates_dir.display()),
                json!({}),
            ));
        }
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("CHANGE-")
            || !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false)
        {
            continue;
        }
        let package_dir = updates_dir.join(&name);
        let lint = lint_change_package(&package_dir, &snapshot, &members);
        let violations = lint.violations;
        let dto = json!({
            "id": name,
            "path": format!("user-input/updates/{name}"),
            "violation_count": violations.len(),
            "violations": violations.iter().map(change_package_violation_dto).collect::<Vec<_>>(),
        });
        packages.push((dto["path"].as_str().unwrap_or_default().to_string(), dto));
    }
    packages.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(Json(json!({
        "packages": packages.into_iter().map(|(_, dto)| dto).collect::<Vec<_>>(),
    }))
    .into_response())
}

/// GET `/change-packages/lint?path=`：单包门禁判定（违规 + 字段填写状态）。
pub async fn lint_knowledge_base_change_package(
    State(state): State<WebAppState>,
    Path((project_id, lc_id)): Path<(String, String)>,
    Query(query): Query<KnowledgeBasePathQuery>,
) -> ApiResult<Response> {
    let (paths, canonical_root, _) = knowledge_base_read_context(&state, &project_id, &lc_id)?;
    let kb = kb_root(&canonical_root);
    let package_dir = resolve_change_package_dir(&kb, &query.path)?;
    let snapshot = read_manifest(&kb).map_err(product_store_api_error)?;
    let members = knowledge_base_member_repos(&paths, &project_id, &lc_id)?;
    let lint = lint_change_package(&package_dir, &snapshot, &members);
    let fields = lint
        .fields
        .iter()
        .map(|field| change_package_field_status_dto(field))
        .collect::<Vec<_>>();
    Ok(Json(json!({
        "violations": lint.violations.iter().map(change_package_violation_dto).collect::<Vec<_>>(),
        "fields": fields,
    }))
    .into_response())
}

fn change_package_field_status_dto(field: &ChangePackageFieldStatus) -> Value {
    json!({
        "document": field.document,
        "field": field.field,
        "filled": field.filled,
    })
}

/// POST `/change-packages`：从插件模板创建变更包骨架（幂等：目录已存在 ⇒
/// 返回既有目录且不改动其中文件）。
pub async fn create_knowledge_base_change_package(
    State(state): State<WebAppState>,
    Path((project_id, lc_id)): Path<(String, String)>,
    Json(request): Json<CreateKnowledgeBaseChangePackageRequest>,
) -> ApiResult<Response> {
    let (_, canonical_root, _) = knowledge_base_read_context(&state, &project_id, &lc_id)?;
    let skills_source_root = knowledge_base_skills_source_root()?;
    let package_dir =
        create_change_package_template(&canonical_root, &skills_source_root, &request.identifier)
            .map_err(|error| match error {
            crate::product::json_store::ProductStoreError::InvalidRecord { .. } => {
                ApiError::runtime(
                    CODE_CHANGE_PACKAGE_PATH_INVALID,
                    "change package identifier is invalid",
                    json!({ "identifier": request.identifier }),
                )
            }
            other => product_store_api_error(other),
        })?;
    let kb = kb_root(&canonical_root);
    let relative = package_dir
        .strip_prefix(&kb)
        .unwrap_or(&package_dir)
        .to_string_lossy()
        .replace('\\', "/");
    let id = StdPath::new(&relative)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string();
    Ok(Json(json!({
        "id": id,
        "path": relative,
    }))
    .into_response())
}

/// POST `/bootstrap-inputs`：补齐初始化输入模板（只补缺失，绝不覆盖）。
pub async fn create_knowledge_base_bootstrap_inputs(
    State(state): State<WebAppState>,
    Path((project_id, lc_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let (_, canonical_root, _) = knowledge_base_read_context(&state, &project_id, &lc_id)?;
    let skills_source_root = knowledge_base_skills_source_root()?;
    let created = create_bootstrap_input_templates(&canonical_root, &skills_source_root)
        .map_err(product_store_api_error)?;
    Ok(Json(json!({ "created": created })).into_response())
}

// ───────────────────────── 路径校验 ─────────────────────────

/// 变更包相对路径（相对知识库根）校验：非空、不以 `/` 开头、不含 `..`、
/// 反斜杠或空白（命令单参数语义），组件合法。
fn validate_change_package_rel_path(path: &str) -> ApiResult<()> {
    let invalid = || {
        ApiError::runtime(
            CODE_CHANGE_PACKAGE_PATH_INVALID,
            "change package path must be a relative path under the knowledge base root",
            json!({ "path": path }),
        )
    };
    if path.is_empty()
        || path.starts_with('/')
        || path.contains('\\')
        || path.chars().any(|ch| ch.is_whitespace() || ch.is_control())
    {
        return Err(invalid());
    }
    for component in path.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(invalid());
        }
    }
    Ok(())
}

/// lint 端点的包目录解析：相对知识库根 + 必须落在 `user-input/updates/` 下。
fn resolve_change_package_dir(kb: &StdPath, path: &str) -> ApiResult<PathBuf> {
    validate_change_package_rel_path(path)?;
    if !path.starts_with("user-input/updates/") {
        return Err(ApiError::runtime(
            CODE_CHANGE_PACKAGE_PATH_INVALID,
            "change package path must live under user-input/updates/",
            json!({ "path": path }),
        ));
    }
    Ok(kb.join(path))
}
