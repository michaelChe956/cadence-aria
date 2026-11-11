use crate::product::logical_codebase::provider_selection::{
    provider_from_wire, select_aggregate_provider, AGGREGATE_PROVIDER_CANDIDATE_ORDER,
    ProviderSelectionOutcome, REASON_CODE_PROVIDER_HEALTH_DEGRADED,
};
use crate::product::logical_codebase::{
    resolve_aggregate_provider_requirement, AggregateProviderRequirement,
    AGGREGATE_PROVIDER_NOT_REQUIRED,
};

/// Legacy `/logical-codebase/initializations` compatibility alias for the
/// default first logical codebase (v1.2 migration artifact).
pub async fn create_aggregate_initialization(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
    Json(request): Json<CreateAggregateInitializationRequest>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    let logical_codebase_id = default_logical_codebase_id(&paths, &project_id)?;
    // owner 2026-10-29：用户显式点「启动聚合初始化」= 明确表达「我要跑完整
    // 五步」——无视聚合根是否已做过 Cadence-skills 初始化（否则 skills 更新
    // 后没有任何重装途径）。跳过式检测只保留给冷启动自动推进路径。
    create_aggregate_initialization_for_lc(state, project_id, logical_codebase_id, request, true)
        .await
}

/// v1.3 canonical endpoint: aggregate initialization is resolved per logical
/// codebase (D3 spawn execute / token cancel / recovery semantics unchanged).
pub async fn create_lc_aggregate_initialization(
    State(state): State<WebAppState>,
    Path((project_id, logical_codebase_id)): Path<(String, String)>,
    Json(request): Json<CreateAggregateInitializationRequest>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    require_logical_codebase(&paths, &project_id, &logical_codebase_id)?;
    // owner 2026-10-29：手动「启动聚合初始化」始终跑完整五步（含四条
    // Cadence-skills 命令），不走「聚合根已初始化」的跳过检测——跳过式
    // 检测只属于冷启动自动推进（bootstrap/advance，full_recipe_requested
    // 仍为 false）。
    create_aggregate_initialization_for_lc(state, project_id, logical_codebase_id, request, true)
        .await
}

/// owner 2026-10-29：「重新初始化（聚合）」入口。
///
/// 与初始化入口同一操作模型、同一条信道路径，唯一差别是创建期冻结
/// 「强制跑完整五步」事实：执行期无视聚合根是否已做过 Cadence-skills
/// 初始化，四条命令照跑（Cadence-skills 更新后重装用）。
pub async fn create_lc_aggregate_reinitialization(
    State(state): State<WebAppState>,
    Path((project_id, logical_codebase_id)): Path<(String, String)>,
    Json(request): Json<CreateAggregateInitializationRequest>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    require_logical_codebase(&paths, &project_id, &logical_codebase_id)?;
    create_aggregate_initialization_for_lc(state, project_id, logical_codebase_id, request, true)
        .await
}

async fn create_aggregate_initialization_for_lc(
    state: WebAppState,
    project_id: String,
    logical_codebase_id: String,
    request: CreateAggregateInitializationRequest,
    full_recipe_requested: bool,
) -> ApiResult<Response> {
    // A（REQ-REG-10 自动推进）：创建链抽成可复用入口——bootstrap advance 的
    // 准备派发复用同一条链，不复制 trust 门/幂等/lease/spawn 语义。
    let operation = start_lc_aggregate_initialization(
        &state,
        &project_id,
        &logical_codebase_id,
        &request.idempotency_key,
        full_recipe_requested,
    )?;
    Ok((
        StatusCode::ACCEPTED,
        Json(aggregate_initialization_dto(operation)),
    )
        .into_response())
}

/// A（REQ-REG-10 自动推进）：「跑起聚合初始化」的唯一实现——trust 硬前置门
/// → 幂等 begin → run lease → spawn 执行 + detached index build。
///
/// 同步（只做 durable 写 + spawn，不 await 长任务），供 HTTP create 面与
/// bootstrap advance 的准备派发共用。
pub(crate) fn start_lc_aggregate_initialization(
    state: &WebAppState,
    project_id: &str,
    logical_codebase_id: &str,
    idempotency_key: &str,
    full_recipe_requested: bool,
) -> ApiResult<crate::product::logical_codebase::AggregateInitializationOperation> {
    let project_paths = product_app_paths(state);
    validate_project_id(project_id)?;
    validate_idempotency_key(idempotency_key)?;
    let dependencies =
        aggregate_initialization_dependencies(state).for_lc(logical_codebase_id.to_string());
    let operation_id = deterministic_operation_id(project_id, idempotency_key);
    let manifest = load_manifest_for_profile(&project_paths, project_id, logical_codebase_id)?;
    // owner 2026-11-04（判定前移）：创建期先认「这条链到底要不要
    // provider」。聚合根已具备两个判定文件（`AGENTS.md` +
    // `.claude/rules/language.md`）⇒ 四条 Cadence-skills 命令没必要跑
    // ⇒ 整条链零 provider turn ⇒ **不需要任何 provider** ⇒ 免过
    // provider 选择门（零额度、零探针：不跑 L1 health、不触发任何
    // capability 探针），自动创建 operation ⇒ 自动开始。
    // 两个判定文件不在 ⇒ 仍需要 provider ⇒ 选择门照旧评估（不过 ⇒
    // **不创建 operation**，守 REQ-BOOT-07）。
    let provider_requirement = resolve_aggregate_provider_requirement(
        &manifest.provider_context_root,
        full_recipe_requested,
    )
    .map_err(product_store_api_error)?;
    // 冻结三件事：`input.provider`（执行/重试期一律从 durable 记录解出，
    // 重试不换人）、trust 门的准入清单、候选表。
    // 「整条链不需要 provider」时：provider 冻结为「本链无 provider」的
    // 事实（`provider_from_wire` 解不出它 ⇒ 万一真要跑 provider turn
    // 必然 fail-closed，绝不静默回落到别的 provider）、准入清单为空
    // （门自身 no-op 语义）、**不落候选表**（没有候选）。
    let (provider_wire, trust_admission, selection_table) = match provider_requirement {
        AggregateProviderRequirement::NotRequired { .. } => (
            AGGREGATE_PROVIDER_NOT_REQUIRED.to_string(),
            Vec::new(),
            None,
        ),
        AggregateProviderRequirement::Required => {
            // 片4（REQ-BOOT-07）：provider 选择器——L1 health 快照 + L2
            // 能力核验，与 gateway 准入同源；失败时**不创建 operation**，
            // 候选表/单家核验入口随失败码下发。同步桥接见
            // [`select_aggregate_provider_blocking`]。
            let selection =
                select_aggregate_provider_blocking(&project_paths, &state.provider_health, project_id)?;
            let ProviderSelectionOutcome::Selected(selection_table) = selection else {
                return Err(provider_selection_api_error(&selection));
            };
            // 未知 wire 名 fail-closed，绝不静默回落默认 provider。
            let selected = provider_from_wire(&selection_table.selected).ok_or_else(|| {
                ApiError::runtime(
                    REASON_CODE_PROVIDER_HEALTH_DEGRADED,
                    "provider selection produced an unknown provider wire name",
                    json!({}),
                )
            })?;
            (
                selection_table.selected.clone(),
                vec![selected],
                Some(selection_table),
            )
        }
    };
    let input = crate::product::logical_codebase::AggregateInitializationOperationInput {
        idempotency_key: idempotency_key.to_string(),
        manifest_revision: manifest.membership_revision,
        policy_digest: "sha256:aggregate-policy".to_string(),
        profile_evidence_digest: Some("sha256:profile".to_string()),
        provider_context_root: manifest.provider_context_root.clone(),
        provider: provider_wire,
    };
    // Task 1.8（REQ-REG-14）：trust 硬前置门——生产依赖图（`production`/
    // `for_lc` 透传）注入 gate 时在 begin 之前评估；未 Ready 只保留 gate
    // 自行落盘的 waiting/审计事实，五步 operation 不创建、worker 不启动。
    // 片4（REQ-BOOT-07）：准入清单 = 冻结的选中 provider——Pi/ClaudeCode
    // 过滤后为空（门 no-op，与接线前 Claude 行为等价）；KimiCode/Codex
    // 走既有 `aggregate_initialization_trust_waiting` 链路。
    ensure_recipe_trust_ready(
        &dependencies,
        project_id,
        &operation_id,
        logical_codebase_id,
        &manifest.provider_context_root,
        &trust_admission,
    )?;
    // REQ-BOOT-07：候选表随 operation 落账（`provider_selection`），幂等
    // 身份不含 provider ⇒ 同 key 重放返回首次落盘的选择结果。
    let operation = match selection_table {
        Some(selection_table) => dependencies
            .coordinator
            .begin_with_selection(
                operation_id,
                project_id,
                input,
                selection_table,
                full_recipe_requested,
            )
            .map_err(aggregate_initialization_api_error)?,
        None => dependencies
            .coordinator
            .begin_without_provider(
                operation_id,
                project_id,
                input,
                full_recipe_requested,
            )
            .map_err(aggregate_initialization_api_error)?,
    };
    // `begin` returns only a newly-created operation: an existing terminal
    // operation with the same key is reported as an idempotency conflict.
    let key =
        InitializationRunKey::aggregate(project_id, logical_codebase_id, &operation.operation_id);
    let lease = dependencies.runs.register(key).ok_or_else(|| {
        ApiError::runtime(
            "aggregate_initialization_in_progress",
            "aggregate initialization is already in progress",
            json!({}),
        )
    })?;
    let token = lease.cancellation_token();
    let coordinator = dependencies.coordinator.clone();
    let index = dependencies.index.clone();
    let manifest_revision = operation.input.manifest_revision;
    let project_id_for_worker = project_id.to_string();
    let operation_id_for_worker = operation.operation_id.clone();
    tokio::spawn(async move {
        // Keep the lease alive for the entire coordinator execution. Dropping
        // it before spawning would make cancellation and recovery observability
        // racy with the first provider turn.
        let _lease = lease;
        match coordinator
            .execute(&project_id_for_worker, &operation_id_for_worker, token)
            .await
        {
            Ok(_completed) => {
                // Task 4（REQ-BOOT-05）：最终政策发布与 receipt 签发已由
                // 生产 driver 在末命令收口唯一完成（发布失败强制传播为
                // ProviderTurn 失败，operation 不 Completed）——此处不再
                // warn-only 补发。Index creation is deliberately detached
                // from initialization durability. A failed index build is
                // observable in its own operation and must not roll back a
                // completed initialization.
                let project_id = project_id_for_worker.clone();
                let operation_id = operation_id_for_worker.clone();
                // Do not extend the initialization lease over the follow-up
                // index build: this is an independent, best-effort task.
                tokio::spawn(async move {
                    let build_project_id = project_id.clone();
                    let result = tokio::task::spawn_blocking(move || {
                        index.build(&build_project_id, manifest_revision)
                    })
                    .await;
                    match result {
                        Ok(Ok(_)) => {}
                        Ok(Err(error)) => {
                            tracing::warn!(
                                project_id = %project_id,
                                operation_id = %operation_id,
                                error = %error,
                                "aggregate initialization index build stopped"
                            );
                            // `AggregateIndexOperation::build` persists a Failed
                            // generation before returning. Keep this detached
                            // failure independent from the already-Completed
                            // initialization operation, while making the
                            // missing projection actionable through GET active.
                        }
                        Err(error) => tracing::warn!(
                            project_id = %project_id,
                            operation_id = %operation_id,
                            error = %error,
                            "aggregate initialization index worker panicked"
                        ),
                    }
                });
            }
            Err(error) => {
                tracing::warn!(
                    project_id = %project_id_for_worker,
                    operation_id = %operation_id_for_worker,
                    error = %error,
                    "aggregate initialization worker stopped"
                );
            }
        }
    });
    Ok(operation)
}

/// A（REQ-REG-10 自动推进）：把「根配方 operation 跑起来」暴露为可复用入口。
///
/// 幂等且不重推：同 `idempotency_key` 派生同 operation id。已在跑 →
/// `AlreadyRunning`（不再启动第二个 provider turn）；已存在但没有任何活跃
/// run（Running/Failed/Cancelled/Completed）→ 一律等人工，**绝不自动重试**
/// 失败配方；不存在 → 走既有创建链（trust 门 + 幂等 begin + lease + spawn）。
pub(crate) fn ensure_lc_initialization_started(
    state: &WebAppState,
    project_id: &str,
    logical_codebase_id: &str,
    idempotency_key: &str,
) -> crate::product::logical_codebase::BootstrapPrepareOutcome {
    use crate::product::logical_codebase::BootstrapPrepareOutcome;
    use crate::product::logical_codebase::aggregate_initialization::AggregateInitializationOperationStatus;

    let operation_id = deterministic_operation_id(project_id, idempotency_key);
    let dependencies =
        aggregate_initialization_dependencies(state).for_lc(logical_codebase_id.to_string());
    let key = InitializationRunKey::aggregate(project_id, logical_codebase_id, &operation_id);
    if dependencies.runs.is_active(&key) {
        // 幂等：同一根配方长任务已在跑，绝不启动第二个 provider turn。
        return BootstrapPrepareOutcome::AlreadyRunning;
    }
    let store = AggregateInitializationOperationStore::for_lc(
        product_app_paths(state),
        logical_codebase_id.to_string(),
    );
    // 同一 LC 上已有任意**其它**根配方在跑（例如用户刚手动点了「启动聚合
    // 初始化」，其 operation id 与自动推进派生的 id 不同）→ 绝不并行启动
    // 第二份；自身 operation 的中断判定留给下方状态分支。
    let other_recipe_running = match store.list(project_id) {
        Ok(operations) => operations.iter().any(|operation| {
            operation.operation_id != operation_id
                && operation.status == AggregateInitializationOperationStatus::Running
        }),
        Err(error) => {
            tracing::warn!(
                project_id = %project_id,
                logical_codebase_id = %logical_codebase_id,
                error = %error,
                "bootstrap advance could not list root recipe operations"
            );
            return BootstrapPrepareOutcome::WaitingForHuman {
                reason_code: "aggregate_initialization_state_unreadable".to_string(),
                detail: Some(format!("listing root recipe operations failed: {error}")),
            };
        }
    };
    if other_recipe_running {
        return BootstrapPrepareOutcome::AlreadyRunning;
    }
    match store.get(project_id, &operation_id) {
        Ok(operation) => {
            return match operation.status {
                // Created 但没有活跃 run：走既有幂等 begin（复用同一记录）
                // + 新 lease，让 execute 从 checkpoint 续跑。
                AggregateInitializationOperationStatus::Created => start_lc_initialization_outcome(
                    state,
                    project_id,
                    logical_codebase_id,
                    idempotency_key,
                ),
                AggregateInitializationOperationStatus::Running => {
                    BootstrapPrepareOutcome::WaitingForHuman {
                        reason_code: "root_recipe_interrupted".to_string(),
                        detail: Some(format!(
                            "operation {operation_id} is durable-Running but no live run \
                             is registered; an explicit continue/retry must resume it"
                        )),
                    }
                }
                AggregateInitializationOperationStatus::Completed => {
                    BootstrapPrepareOutcome::WaitingForHuman {
                        reason_code: "root_recipe_already_completed".to_string(),
                        detail: Some(format!(
                            "operation {operation_id} already completed its root recipe"
                        )),
                    }
                }
                AggregateInitializationOperationStatus::Failed => {
                    BootstrapPrepareOutcome::WaitingForHuman {
                        reason_code: "root_recipe_failed".to_string(),
                        detail: Some(format!(
                            "operation {operation_id} ended Failed{}",
                            operation
                                .error
                                .as_ref()
                                .map(|error| format!(": {} ({})", error.stage, error.reason_code))
                                .unwrap_or_default()
                        )),
                    }
                }
                AggregateInitializationOperationStatus::Cancelled => {
                    BootstrapPrepareOutcome::WaitingForHuman {
                        reason_code: "root_recipe_cancelled".to_string(),
                        detail: Some(format!(
                            "operation {operation_id} was cancelled before readiness froze"
                        )),
                    }
                }
            };
        }
        Err(crate::product::json_store::ProductStoreError::NotFound { .. }) => {}
        Err(error) => {
            tracing::warn!(
                project_id = %project_id,
                logical_codebase_id = %logical_codebase_id,
                error = %error,
                "bootstrap advance could not read the root recipe operation"
            );
            return BootstrapPrepareOutcome::WaitingForHuman {
                reason_code: "aggregate_initialization_state_unreadable".to_string(),
                detail: Some(format!("reading the root recipe operation failed: {error}")),
            };
        }
    }
    start_lc_initialization_outcome(state, project_id, logical_codebase_id, idempotency_key)
}

fn start_lc_initialization_outcome(
    state: &WebAppState,
    project_id: &str,
    logical_codebase_id: &str,
    idempotency_key: &str,
) -> crate::product::logical_codebase::BootstrapPrepareOutcome {
    match start_lc_aggregate_initialization(
        state,
        project_id,
        logical_codebase_id,
        idempotency_key,
        false,
    ) {
        Ok(_) => crate::product::logical_codebase::BootstrapPrepareOutcome::Started,
        Err(error) => {
            tracing::warn!(
                project_id = %project_id,
                logical_codebase_id = %logical_codebase_id,
                code = %error.code,
                message = %error.message,
                "bootstrap advance root recipe prepare was rejected"
            );
            crate::product::logical_codebase::BootstrapPrepareOutcome::WaitingForHuman {
                reason_code: error.code.clone(),
                detail: prepare_rejection_detail(&error),
            }
        }
    }
}

/// prepare 被拒的可读上下文：HTTP 面错误 message + **紧凑中文**的候选状态。
///
/// 旧实现把整个 `details`（候选表 + 单家核验入口 + copy + 规则版本）序列化
/// 进诊断串，界面直接摊开一坨机器报文 —— 用户要的是「哪几家、什么状态」，
/// 不是原始 JSON。机器字段仍留在 HTTP 响应体里给 agent 用，界面文案只留
/// 可执行的那部分。落盘前统一走脱敏截断。
fn prepare_rejection_detail(error: &crate::web::error::ApiError) -> Option<String> {
    Some(
        match compact_candidate_line(&error.details) {
            Some(line) => format!("{}；{line}", error.message),
            None => error.message.clone(),
        },
    )
}

/// 把 provider 选择门的 `details.candidates` 渲染成一行紧凑中文（逐家状态）。
///
/// 无候选表时返回 `None`（诊断串只保留错误 message，不追加任何机器字段）。
fn compact_candidate_line(details: &serde_json::Value) -> Option<String> {
    let candidates = details.get("candidates")?.as_array()?;
    let parts: Vec<String> = candidates
        .iter()
        .map(|candidate| {
            let name = candidate
                .get("display_name")
                .and_then(serde_json::Value::as_str)
                .or_else(|| candidate.get("provider").and_then(serde_json::Value::as_str))
                .unwrap_or("未知 provider");
            format!("{name}（{}）", candidate_state_label(candidate))
        })
        .collect();
    if parts.is_empty() {
        return None;
    }
    Some(format!("候选：{}", parts.join("、")))
}

/// 候选的中文状态标签（与前端 `SKIPPED_REASON_LABELS` / 能力面口径一致）。
///
/// 优先用 `skipped_reason`（为什么被跳过），缺省再看 `capability_state`；
/// 认不出的形态一律按「未核验」呈现 —— 不谎报「已核验」。
fn candidate_state_label(candidate: &serde_json::Value) -> &'static str {
    match candidate
        .get("skipped_reason")
        .and_then(serde_json::Value::as_str)
    {
        Some("cli_unavailable") => "未安装",
        Some("needs_verification") => "未核验",
        Some("denied") => "实测拒绝",
        Some("evidence_stale") => "证据过期",
        _ => match candidate
            .get("capability_state")
            .and_then(serde_json::Value::as_str)
        {
            Some("confirmed") => "已核验",
            Some("not_installed") => "未安装",
            Some("denied") => "实测拒绝",
            Some("evidence_stale") => "证据过期",
            _ => "未核验",
        },
    }
}

/// Legacy `/logical-codebase/initializations/{operation_id}` compatibility
/// alias for the default first logical codebase.
pub async fn get_aggregate_initialization(
    State(state): State<WebAppState>,
    Path((project_id, operation_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    let logical_codebase_id = default_logical_codebase_id(&paths, &project_id)?;
    get_aggregate_initialization_for_lc(state, project_id, logical_codebase_id, operation_id)
}

/// v1.3 canonical endpoint: operation recovery is resolved per logical
/// codebase.
pub async fn get_lc_aggregate_initialization(
    State(state): State<WebAppState>,
    Path((project_id, logical_codebase_id, operation_id)): Path<(String, String, String)>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    require_logical_codebase(&paths, &project_id, &logical_codebase_id)?;
    get_aggregate_initialization_for_lc(state, project_id, logical_codebase_id, operation_id)
}

/// Legacy `/logical-codebase/initializations/active` compatibility alias for
/// the default first logical codebase.
pub async fn get_aggregate_initialization_active(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    let logical_codebase_id = default_logical_codebase_id(&paths, &project_id)?;
    get_aggregate_initialization_active_for_lc(state, project_id, logical_codebase_id)
}

/// v1.3 canonical endpoint: returns the LC's latest aggregate initialization
/// operation regardless of terminal state (completed/failed/cancelled), so
/// the management panel can restore the real status after a page reload
/// instead of showing「尚未初始化」.
pub async fn get_lc_aggregate_initialization_active(
    State(state): State<WebAppState>,
    Path((project_id, logical_codebase_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    require_logical_codebase(&paths, &project_id, &logical_codebase_id)?;
    get_aggregate_initialization_active_for_lc(state, project_id, logical_codebase_id)
}

fn get_aggregate_initialization_active_for_lc(
    state: WebAppState,
    project_id: String,
    logical_codebase_id: String,
) -> ApiResult<Response> {
    validate_project_id(&project_id)?;
    let dependencies =
        aggregate_initialization_dependencies(&state).for_lc(logical_codebase_id.clone());
    // 纯投影:任意状态(含 completed/failed/cancelled)的最新 operation;
    // 无 operation 时按未初始化语义返回稳定 not_found code(404)。
    let operation = dependencies
        .coordinator
        .latest(&project_id)
        .map_err(aggregate_initialization_api_error)?
        .ok_or_else(|| {
            aggregate_initialization_api_error(AggregateInitializationError::NotFound {
                id: "active".to_string(),
            })
        })?;
    Ok(Json(aggregate_initialization_dto(operation)).into_response())
}

fn get_aggregate_initialization_for_lc(
    state: WebAppState,
    project_id: String,
    logical_codebase_id: String,
    operation_id: String,
) -> ApiResult<Response> {
    validate_project_id(&project_id)?;
    validate_operation_id(&operation_id)?;
    let dependencies =
        aggregate_initialization_dependencies(&state).for_lc(logical_codebase_id.clone());
    // C4 Task 6：GET 纯投影——Running 但内存 run 不活跃时不再隐式
    // `recover_interrupted`；中断事实由显式 bootstrap action（Continue/
    // Retry）落盘，GET 只读 durable 记录与 DTO。
    let operation = dependencies
        .coordinator
        .get(&project_id, &operation_id)
        .map_err(aggregate_initialization_api_error)?;
    Ok(Json(aggregate_initialization_dto(operation)).into_response())
}

/// Legacy `/logical-codebase/initializations/{operation_id}/cancel`
/// compatibility alias for the default first logical codebase.
pub async fn cancel_aggregate_initialization(
    State(state): State<WebAppState>,
    Path((project_id, operation_id)): Path<(String, String)>,
    Json(request): Json<CancelAggregateInitializationRequest>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    let logical_codebase_id = default_logical_codebase_id(&paths, &project_id)?;
    cancel_aggregate_initialization_for_lc(
        state,
        project_id,
        logical_codebase_id,
        operation_id,
        request,
    )
}

/// v1.3 canonical endpoint: cancellation is resolved per logical codebase.
pub async fn cancel_lc_aggregate_initialization(
    State(state): State<WebAppState>,
    Path((project_id, logical_codebase_id, operation_id)): Path<(String, String, String)>,
    Json(request): Json<CancelAggregateInitializationRequest>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    require_logical_codebase(&paths, &project_id, &logical_codebase_id)?;
    cancel_aggregate_initialization_for_lc(
        state,
        project_id,
        logical_codebase_id,
        operation_id,
        request,
    )
}

fn cancel_aggregate_initialization_for_lc(
    state: WebAppState,
    project_id: String,
    logical_codebase_id: String,
    operation_id: String,
    request: CancelAggregateInitializationRequest,
) -> ApiResult<Response> {
    validate_project_id(&project_id)?;
    validate_operation_id(&operation_id)?;
    let dependencies =
        aggregate_initialization_dependencies(&state).for_lc(logical_codebase_id.clone());
    let operation = dependencies
        .coordinator
        .cancel(
            &project_id,
            &operation_id,
            &request.reason,
            request.detail.clone(),
        )
        .map_err(aggregate_initialization_api_error)?;
    // Persist the cancellation first, then signal the in-memory worker. The
    // worker checks this token at every step boundary and will not advance
    // after the provider turn currently in flight completes.
    dependencies.runs.cancel(&InitializationRunKey::aggregate(
        &project_id,
        &logical_codebase_id,
        &operation_id,
    ));
    Ok((
        StatusCode::OK,
        Json(aggregate_initialization_dto(operation)),
    )
        .into_response())
}

fn aggregate_initialization_dependencies(
    state: &WebAppState,
) -> AggregateInitializationDependencies {
    state.aggregate_initialization_dependencies()
}

fn load_manifest_for_profile(
    paths: &ProductAppPaths,
    project_id: &str,
    logical_codebase_id: &str,
) -> ApiResult<crate::product::logical_codebase::store::LogicalCodebaseManifest> {
    let store = crate::product::logical_codebase::LogicalCodebaseStore::for_lc(
        paths.clone(),
        logical_codebase_id,
    );
    store
        .load_manifest(project_id)
        .map_err(product_store_api_error)?
        .ok_or_else(|| {
            ApiError::runtime(
                "logical_codebase_manifest_missing",
                "logical codebase manifest is missing; register members first",
                json!({}),
            )
        })
}

fn validate_project_id(project_id: &str) -> ApiResult<()> {
    validate_relative_id(project_id).map_err(|error| {
        ApiError::validation("invalid_project_id", format!("invalid project id: {error}"))
    })?;
    Ok(())
}

fn validate_operation_id(operation_id: &str) -> ApiResult<()> {
    validate_relative_id(operation_id).map_err(|error| {
        ApiError::validation(
            "invalid_operation_id",
            format!("invalid operation id: {error}"),
        )
    })?;
    Ok(())
}

fn validate_idempotency_key(key: &str) -> ApiResult<()> {
    if key.is_empty() {
        return Err(ApiError::validation(
            "invalid_idempotency_key",
            "idempotency_key must not be empty",
        ));
    }
    Ok(())
}

fn deterministic_operation_id(project_id: &str, idempotency_key: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(project_id.as_bytes());
    hasher.update(idempotency_key.as_bytes());
    format!("aggregate_initialization_{:x}", hasher.finalize())[..40].to_string()
}

fn aggregate_initialization_api_error(error: AggregateInitializationError) -> ApiError {
    match error {
        AggregateInitializationError::NotFound { .. } => ApiError::runtime(
            "aggregate_initialization_operation_not_found",
            "aggregate initialization operation not found",
            json!({}),
        ),
        AggregateInitializationError::TrustWaiting { waiting } => ApiError::runtime(
            "aggregate_initialization_trust_waiting",
            waiting.message.clone(),
            json!({
                "provider": crate::product::logical_codebase::provider_trust::provider_wire_label(
                    &waiting.provider,
                ),
                "reason_code": waiting.reason_code,
                "retry_action": waiting.retry_action,
                "manual_guidance": waiting.manual_guidance,
                // 指引卡简化（owner 2026-10-12 直批）：结构化命令段
                //（entry 缺失才有值），消费方直渲命令窗。
                "command_markdown": waiting.command_markdown,
                "trust_key": waiting.trust_key,
                "canonical_root": waiting.canonical_root.to_string_lossy(),
                "retryable": true,
            }),
        ),
        AggregateInitializationError::StateRejected { detail, .. } => {
            ApiError::runtime("aggregate_initialization_state_rejected", detail, json!({}))
        }
        AggregateInitializationError::Store(error) => product_store_api_error(error),
        other => ApiError::runtime(
            "aggregate_initialization_failed",
            other.to_string(),
            json!({}),
        ),
    }
}

/// Task 1.8（REQ-REG-14）：route 面 trust 硬前置门——与 coordinator 的
/// `execute_with_trust` 同一语义（registry durable facts 按
/// (project_id, lc_id) 调用点 scope，根暂不可得时保持原样、由后续
/// aggregate_preflight fail-closed 拦截）。未装配 gate 的依赖图（测试
/// 默认）保持旧行为；Waiting 以稳定 code 的可重试等待面拒绝，绝不
/// begin/execute。
fn ensure_recipe_trust_ready(
    dependencies: &AggregateInitializationDependencies,
    project_id: &str,
    operation_id: &str,
    logical_codebase_id: &str,
    provider_context_root: &std::path::Path,
    providers: &[crate::product::models::ProviderName],
) -> ApiResult<()> {
    use crate::product::logical_codebase::provider_trust::ProviderTrustPreparationResult;

    let Some(trust) = dependencies.trust() else {
        return Ok(());
    };
    let canonical_root = std::fs::canonicalize(provider_context_root)
        .unwrap_or_else(|_| provider_context_root.to_path_buf());
    match trust.ensure_ready(
        project_id,
        operation_id,
        logical_codebase_id,
        &canonical_root,
        // 片4（REQ-BOOT-07）：准入清单 = 冻结的选中 provider（Pi/
        // ClaudeCode 过滤后为空 → 门 no-op；KimiCode/Codex 进入登记）。
        // 「整条链零 provider turn」时清单为空：门照旧被调用并评估
        // （门自身 no-op 语义），只是没有需要登记 trust 的 provider。
        providers,
        // route 面是五步 recipe 入口：以 Recipe 场景驱动中立 trust 门。
        crate::product::logical_codebase::provider_trust::ProviderTrustScenario::Recipe,
    ) {
        ProviderTrustPreparationResult::Ready { .. } => Ok(()),
        ProviderTrustPreparationResult::Waiting { waiting } => Err(
            aggregate_initialization_api_error(AggregateInitializationError::TrustWaiting {
                waiting: Box::new(waiting),
            }),
        ),
    }
}

/// 片4（REQ-BOOT-07）：`start_lc_aggregate_initialization` 是同步入口
/// （HTTP create 面与 bootstrap advance 的同步 prepare 派发共用），选择器
/// 是 async——专用线程 + current-thread runtime 阻塞取值（先例：
/// aggregate_index 的 codegraph runner）。health 刷新 token 在选择器内部
/// 自建，与请求生命周期解耦；store 级错误走既有 product store 映射。
fn select_aggregate_provider_blocking(
    paths: &ProductAppPaths,
    health: &std::sync::Arc<crate::cross_cutting::provider_health::ProviderHealthService>,
    project_id: &str,
) -> ApiResult<ProviderSelectionOutcome> {
    let health = health.clone();
    let paths = paths.clone();
    let project_id = project_id.to_string();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("provider selection runtime");
        runtime.block_on(select_aggregate_provider(&paths, &health, &project_id))
    })
    .join()
    .map_err(|_| {
        ApiError::runtime(
            REASON_CODE_PROVIDER_HEALTH_DEGRADED,
            "provider selection worker panicked",
            json!({}),
        )
    })?
    .map_err(product_store_api_error)
}

/// 片4（REQ-BOOT-07 / REQ-PCR-02）：选择期失败 → HTTP 失败面。**不创建
/// operation**（选择器零写入），稳定码取选择器 `reason_code()`（四个
/// `REASON_CODE_*`，三段成品与执行期三态同源放 `details.copy`）。
///
/// - `details.candidates`：候选表（owner 顺序，逐家 `action_hint`）；
/// - `details.revalidate`：单家核验入口六字段——`NeedsVerification` 指向
///   `suggested`；`Denied` 指向被点名者的**下一家**（owner 顺序）；
/// - `details.fallback_provider`：仅 `Denied`——下一家 wire 名；被点名者
///   已是末家时为 `null` 且不下发 `revalidate`（不指向不存在的下一家）。
fn provider_selection_api_error(outcome: &ProviderSelectionOutcome) -> ApiError {
    use crate::product::logical_codebase::failure_copy::{describe, FailureInput};
    use crate::product::logical_codebase::provider_trust::provider_wire_label;

    let code = outcome
        .reason_code()
        .unwrap_or("aggregate_initialization_failed");
    // 被点名 provider：NeedsVerification = 建议核验的那家；Denied = owner
    // 顺序中第一家已安装（被真实拒绝）的候选，其下一家为 fallback。
    let (named, fallback) = match outcome {
        ProviderSelectionOutcome::NeedsVerification { suggested, .. } => {
            (Some(suggested.clone()), None)
        }
        ProviderSelectionOutcome::Denied { table } => {
            let named = table
                .candidates
                .iter()
                .find(|candidate| candidate.cli_available)
                .and_then(|candidate| provider_from_wire(&candidate.provider));
            let fallback = named.clone().and_then(|named| {
                AGGREGATE_PROVIDER_CANDIDATE_ORDER
                    .iter()
                    .skip_while(|provider| *provider != &named)
                    .nth(1)
                    .cloned()
            });
            (named, fallback)
        }
        _ => (None, None),
    };
    let copy = describe(FailureInput {
        provider: named
            .map(|provider| provider_wire_label(&provider).to_string())
            .as_deref(),
        ..FailureInput::new(code)
    });
    let display_name = |wire: &str| -> String {
        outcome
            .table()
            .and_then(|table| {
                table
                    .candidates
                    .iter()
                    .find(|candidate| candidate.provider == wire)
            })
            .map(|candidate| candidate.display_name.clone())
            .unwrap_or_else(|| wire.to_string())
    };
    let mut details = serde_json::json!({ "copy": copy });
    if let Some(table) = outcome.table() {
        details["candidates"] = serde_json::to_value(&table.candidates).unwrap_or_default();
        details["rule_version"] = serde_json::json!(table.rule_version);
        details["decided_at"] = serde_json::json!(table.decided_at);
    }
    if let ProviderSelectionOutcome::HealthDegraded { detail } = outcome {
        details["detail"] = serde_json::json!(detail);
    }
    // 核验入口只指向单家：NeedsVerification → suggested；Denied → 下一家。
    let revalidate_target = match outcome {
        ProviderSelectionOutcome::NeedsVerification { suggested, .. } => Some(suggested.clone()),
        ProviderSelectionOutcome::Denied { .. } => fallback.clone(),
        _ => None,
    };
    if let Some(target) = revalidate_target {
        let wire = provider_wire_label(&target).to_string();
        details["suggested_provider"] = serde_json::json!(wire);
        details["revalidate"] = capability_revalidate_entry(&target, &display_name(&wire));
    }
    if let ProviderSelectionOutcome::Denied { .. } = outcome {
        details["fallback_provider"] = match fallback {
            Some(next) => serde_json::json!(provider_wire_label(&next)),
            None => serde_json::Value::Null,
        };
    }
    ApiError::runtime(code, copy.title_zh.clone(), details)
}

/// 单家核验入口（REQ-PCR-02 场景 2：`expect` 说明真实探针会消耗额度；
/// MUST NOT 指向「一键核验全部」）。`ui_path` 是能力卡片（逻辑代码库
/// 管理抽屉）所在的 workbench 页。
///
/// 能力面改**机器级**（REQ-PCR-08）：核验入口不再按项目 / 逻辑代码库定位 ⇒
/// 路径是机器级路由（一个项目都没有也能核验）。
fn capability_revalidate_entry(
    target: &crate::product::models::ProviderName,
    display_name: &str,
) -> serde_json::Value {
    let wire =
        crate::product::logical_codebase::provider_trust::provider_wire_label(target).to_string();
    json!({
        "ui_path": "/workbench",
        "button_label": format!("核验 {display_name}"),
        "method": "POST",
        "path": "/api/providers/capabilities/revalidate",
        "body": json!({ "provider_type": wire }),
        "expect": "点击后立即执行一次真实能力探针（会消耗少量该 provider 的额度），通过后自动导入核验记录；全行已核验时秒回 already_confirmed。",
    })
}
