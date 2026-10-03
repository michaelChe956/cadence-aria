//! C2 oracle C-2 补齐：验证处理／重跑原计划命令／受限政策读取的 REST 薄接线。
//!
//! 全部复用既有引擎应用服务（`enter_verification_triage`／
//! `decide_verification_triage`／`rerun_planned_command`）与 Task 2 attempt
//! 命令账本（policy reauthorization），不新建应用逻辑；按 attempt 地址
//! 作答（与 C2 Task 12 gate-responses REST 同一模式），供驾驶舱与
//! CodingWorkspacePage GatePanel 数据源（triage 记录 GET、
//! verification_command_evidence 派生）消费。

use axum::Json;
use axum::extract::{Path, State};

use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::web::error::{ApiError, ApiResult};
use crate::web::handlers::coding::scope::CodingAttemptRoutePath;
use crate::web::handlers::coding::{
    coding_workspace_engine_with_dummy_events, product_app_paths, resolve_coding_attempt,
};
use crate::web::handlers::support::{coding_workspace_api_error, product_store_api_error};
use crate::web::state::WebAppState;

/// C2 oracle C-2a：验证处理记录 GET（GatePanel `verificationTriage` 数据源；
/// 含 attempt 版本供页面操作上下文）。
#[derive(Debug, serde::Serialize)]
pub struct VerificationTriageRecordsDto {
    pub attempt_id: String,
    pub attempt_version: u64,
    pub records: Vec<crate::product::coding_attempt_store::VerificationTriageRecord>,
}

/// C2 oracle C-2a：转入验证处理请求体（映射引擎 `VerificationTriageEntryRequest`）。
#[derive(Debug, serde::Deserialize)]
pub struct VerificationTriageEnterRestRequest {
    pub finding_id: String,
    pub check_id: String,
    #[serde(default)]
    pub original_command: Option<String>,
    #[serde(default)]
    pub alternative_command: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub outcome: Option<String>,
    #[serde(default)]
    pub test_execution_count: Option<u64>,
    #[serde(default)]
    pub environment: Option<String>,
}

/// C2 oracle C-2a：决定验证处理请求体（三类结论均需用户明确批准）。
#[derive(Debug, serde::Deserialize)]
pub struct VerificationTriageDecisionRestRequest {
    pub conclusion: crate::product::coding_attempt_store::VerificationTriageConclusion,
    pub decided_by: String,
    pub reason: String,
    #[serde(default)]
    pub exemption_scope: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct VerificationTriageDecisionRoutePath {
    #[serde(default)]
    pub project_id: Option<String>,
    #[serde(default)]
    pub issue_id: Option<String>,
    pub attempt_id: String,
    pub triage_id: String,
}

/// C2 oracle C-2b：计划命令与实际命令并列证据列表（GatePanel
/// `commandEvidence` 数据源；对当前 plan revision 的每个验证 check 派生，
/// 复用 Task 9 `verification_command_evidence`，缺失字段保持 None 不推断）。
#[derive(Debug, serde::Serialize)]
pub struct VerificationCommandEvidenceListDto {
    pub attempt_id: String,
    pub rework_count: u64,
    pub evidence: Vec<crate::product::coding_attempt_store::VerificationCommandEvidence>,
}

/// C2 oracle C-2b：重跑原计划命令请求体（稳定 command_id＋gate/check 身份＋
/// expected 版本绑定 attempt.rework_count；引擎内部走 Task 2 命令账本幂等）。
#[derive(Debug, serde::Deserialize)]
pub struct RerunPlannedCommandRestRequest {
    pub command_id: String,
    pub gate_id: String,
    pub check_id: String,
    pub expected_version: u64,
}

/// C2 oracle C-2b：重跑结果（`replayed=true` 表示命中账本重放首次 durable 结果）。
#[derive(Debug, serde::Serialize)]
pub struct RerunPlannedCommandRestResult {
    pub command_id: String,
    pub attempt_id: String,
    pub instruction_id: String,
    pub replayed: bool,
}

/// C2 oracle C-2c：页面受限政策读取结果（包装 mediator `PolicyTextResult`，
/// 附 attempt 版本供重新授权 expected_version）。
#[derive(Debug, serde::Serialize)]
pub struct CodingPolicyTextDto {
    pub attempt_id: String,
    pub attempt_version: u64,
    pub policy: crate::product::logical_codebase::repository_routing::PolicyTextResult,
}

/// C2 oracle C-2c：重新授权请求体（映射 mediator `PolicyReauthorizationRequest`）。
#[derive(Debug, serde::Deserialize)]
pub struct PolicyReauthorizationRestRequest {
    pub command_id: String,
    pub attempt_id: String,
    pub role: String,
    pub policy_digest: String,
    pub expected_version: u64,
}

/// C2 oracle C-2c：重新授权结果（Accepted／Replayed→200，NeedsHuman→202，
/// Rejected→409；state 语义与 gate-responses REST 一致）。
#[derive(Debug, serde::Serialize)]
pub struct PolicyReauthorizationRestResult {
    pub command_id: String,
    pub state: crate::product::models::automation::OperationState,
    pub attempt_id: String,
    pub reason: Option<String>,
    pub expires_at: Option<String>,
}

/// GET /coding-attempts/{attempt_id}/verification-triage——验证处理记录
/// GET（页面数据源；只读）。
pub(crate) async fn get_verification_triage_records(
    State(state): State<WebAppState>,
    Path(path): Path<CodingAttemptRoutePath>,
) -> ApiResult<Json<VerificationTriageRecordsDto>> {
    let app_paths = product_app_paths(&state);
    let store = CodingAttemptStore::new(app_paths);
    let attempt = resolve_coding_attempt(
        &store,
        path.project_id.as_deref(),
        path.issue_id.as_deref(),
        &path.attempt_id,
    )?;
    let records = store
        .list_verification_triage_records(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .map_err(product_store_api_error)?;
    Ok(Json(VerificationTriageRecordsDto {
        attempt_id: attempt.id,
        attempt_version: attempt.version,
        records,
    }))
}

/// POST /coding-attempts/{attempt_id}/verification-triage——转入验证处理
/// （REQ-CVT-03 受控入口；同键未决重入幂等返回既有记录）。
pub(crate) async fn post_verification_triage_enter(
    State(state): State<WebAppState>,
    Path(path): Path<CodingAttemptRoutePath>,
    Json(request): Json<VerificationTriageEnterRestRequest>,
) -> ApiResult<Json<crate::product::coding_attempt_store::VerificationTriageRecord>> {
    let app_paths = product_app_paths(&state);
    let store = CodingAttemptStore::new(app_paths);
    let attempt = resolve_coding_attempt(
        &store,
        path.project_id.as_deref(),
        path.issue_id.as_deref(),
        &path.attempt_id,
    )?;
    if request.finding_id.trim().is_empty() || request.check_id.trim().is_empty() {
        return Err(ApiError::validation(
            "verification_triage_invalid_identity",
            "finding_id and check_id must not be blank",
        ));
    }
    let engine = coding_workspace_engine_with_dummy_events(store);
    let record = engine
        .enter_verification_triage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            crate::product::coding_workspace_engine::VerificationTriageEntryRequest {
                finding_id: request.finding_id,
                check_id: request.check_id,
                original_command: request.original_command,
                alternative_command: request.alternative_command,
                cwd: request.cwd,
                outcome: request.outcome,
                test_execution_count: request.test_execution_count,
                environment: request.environment,
            },
        )
        .await
        .map_err(verification_surface_engine_error)?;
    Ok(Json(record))
}

/// POST /coding-attempts/{attempt_id}/verification-triage/{triage_id}/decision
/// ——决定验证处理（三类结论均需用户明确批准；幂等重放返回首次结果）。
/// durable-first 与 WS gate response 同款：mutation lease 下落决定，白名单
/// 续跑判定后按需唤回 runner（观察通道缺失不阻塞业务事实）。
pub(crate) async fn post_verification_triage_decision(
    State(state): State<WebAppState>,
    Path(path): Path<VerificationTriageDecisionRoutePath>,
    Json(request): Json<VerificationTriageDecisionRestRequest>,
) -> ApiResult<Json<crate::product::coding_attempt_store::VerificationTriageRecord>> {
    use crate::web::coding_ws_handler::{
        should_resume_runner_after_gate_response, spawn_coding_runner,
    };
    use crate::web::state::CodingAttemptRunKey;

    let app_paths = product_app_paths(&state);
    let coding_store = CodingAttemptStore::new(app_paths);
    let attempt = resolve_coding_attempt(
        &coding_store,
        path.project_id.as_deref(),
        path.issue_id.as_deref(),
        &path.attempt_id,
    )?;
    if path.triage_id.trim().is_empty() {
        return Err(ApiError::validation(
            "verification_triage_invalid_identity",
            "triage id must not be blank",
        ));
    }
    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    let mutation_lease = state.coding_runs.lock_attempt_mutation(&attempt_key).await;
    let engine = coding_workspace_engine_with_dummy_events(coding_store.clone());
    let decision = engine
        .decide_verification_triage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            crate::product::coding_workspace_engine::VerificationTriageDecisionRequest {
                triage_id: path.triage_id.clone(),
                conclusion: request.conclusion,
                decided_by: request.decided_by,
                reason: request.reason,
                exemption_scope: request.exemption_scope,
            },
        )
        .await;
    drop(mutation_lease);
    let record = decision.map_err(verification_surface_engine_error)?;
    // WS 同款续跑判定：批准"等价证据／限域例外"后引擎按 manual_continue
    // 语义落门续跑；attempt 回到 Running 时唤回 runner。
    if should_resume_runner_after_gate_response("manual_continue", &attempt)
        && let Ok(updated) =
            coding_store.get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
        && updated.status == crate::product::coding_models::CodingAttemptStatus::Running
    {
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(64);
        drop(_event_rx);
        let _ = spawn_coding_runner(state.clone(), coding_store.clone(), event_tx, updated);
    }
    Ok(Json(record))
}

/// GET /coding-attempts/{attempt_id}/verification-command-evidence——并列
/// 证据派生（当前 plan revision 全部验证 check；REQ-CVT-01）。
pub(crate) async fn get_verification_command_evidence(
    State(state): State<WebAppState>,
    Path(path): Path<CodingAttemptRoutePath>,
) -> ApiResult<Json<VerificationCommandEvidenceListDto>> {
    let app_paths = product_app_paths(&state);
    let store = CodingAttemptStore::new(app_paths);
    let attempt = resolve_coding_attempt(
        &store,
        path.project_id.as_deref(),
        path.issue_id.as_deref(),
        &path.attempt_id,
    )?;
    let engine = coding_workspace_engine_with_dummy_events(store.clone());
    let checks = engine
        .verification_triage_bound_checks(&attempt)
        .map_err(verification_surface_engine_error)?;
    let mut evidence = Vec::with_capacity(checks.len());
    for check in &checks {
        evidence.push(
            store
                .verification_command_evidence(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    check,
                )
                .map_err(product_store_api_error)?,
        );
    }
    Ok(Json(VerificationCommandEvidenceListDto {
        attempt_id: attempt.id,
        rework_count: attempt.rework_count as u64,
        evidence,
    }))
}

/// POST /coding-attempts/{attempt_id}/rerun-planned-command——重跑原计划
/// 命令（REQ-CVT-02；复用引擎返修落地面与 Task 2 命令账本幂等；原门
/// send_to_coder 收口后按 WS 同款续跑判定唤回 runner）。
pub(crate) async fn post_rerun_planned_command(
    State(state): State<WebAppState>,
    Path(path): Path<CodingAttemptRoutePath>,
    Json(request): Json<RerunPlannedCommandRestRequest>,
) -> ApiResult<Json<RerunPlannedCommandRestResult>> {
    use crate::web::coding_ws_handler::{
        should_resume_runner_after_gate_response, spawn_coding_runner,
    };
    use crate::web::state::CodingAttemptRunKey;

    let app_paths = product_app_paths(&state);
    let coding_store = CodingAttemptStore::new(app_paths);
    let attempt = resolve_coding_attempt(
        &coding_store,
        path.project_id.as_deref(),
        path.issue_id.as_deref(),
        &path.attempt_id,
    )?;
    crate::product::json_store::validate_relative_id(&request.command_id).map_err(|error| {
        ApiError::validation(
            "coding_rerun_invalid_command_id",
            format!("invalid rerun command id: {error}"),
        )
    })?;
    let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);
    let mutation_lease = state.coding_runs.lock_attempt_mutation(&attempt_key).await;
    let engine = coding_workspace_engine_with_dummy_events(coding_store.clone());
    let outcome = engine
        .rerun_planned_command(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &crate::product::coding_workspace_engine::RerunPlannedCommandRequest {
                command_id: request.command_id.clone(),
                gate_id: request.gate_id.clone(),
                check_id: request.check_id.clone(),
                expected_version: request.expected_version,
            },
        )
        .await;
    drop(mutation_lease);
    let outcome = outcome.map_err(verification_surface_engine_error)?;
    // 原门经引擎以 send_to_coder 同义动作收口：WS 同款续跑判定唤回 runner。
    if should_resume_runner_after_gate_response("send_to_coder", &attempt)
        && outcome.attempt.status == crate::product::coding_models::CodingAttemptStatus::Running
    {
        let (event_tx, _event_rx) = tokio::sync::mpsc::channel(64);
        drop(_event_rx);
        let _ = spawn_coding_runner(
            state.clone(),
            coding_store.clone(),
            event_tx,
            outcome.attempt.clone(),
        );
    }
    Ok(Json(RerunPlannedCommandRestResult {
        command_id: request.command_id,
        attempt_id: outcome.attempt.id,
        instruction_id: outcome.instruction_id,
        replayed: outcome.replayed,
    }))
}

/// GET /coding-attempts/{attempt_id}/policy-text——blocked／rework 等待面
/// 受限政策读取（REQ-ENV-C2-POLICY；resolver 冻结一致性，fail-closed 落
/// 核验等待事实）。
pub(crate) async fn get_coding_policy_text(
    State(state): State<WebAppState>,
    Path(path): Path<CodingAttemptRoutePath>,
) -> ApiResult<Json<CodingPolicyTextDto>> {
    let app_paths = product_app_paths(&state);
    let store = CodingAttemptStore::new(app_paths.clone());
    let attempt = resolve_coding_attempt(
        &store,
        path.project_id.as_deref(),
        path.issue_id.as_deref(),
        &path.attempt_id,
    )?;
    let policy = crate::product::logical_codebase::read_policy_text_for_attempt(
        &app_paths,
        &attempt.project_id,
        &attempt.issue_id,
        &attempt.id,
    )
    .map_err(crate::web::handlers::evidence_error_mapping::policy_api_error)?;
    Ok(Json(CodingPolicyTextDto {
        attempt_id: attempt.id,
        attempt_version: attempt.version,
        policy,
    }))
}

/// POST /coding-attempts/{attempt_id}/policy-reauthorization——重新授权
/// （用户确认后；绑定 attempt＋role＋policy digest，仅下一次返修 run 运行态
/// 有效；Task 2 命令账本幂等）。
pub(crate) async fn post_coding_policy_reauthorization(
    State(state): State<WebAppState>,
    Path(path): Path<CodingAttemptRoutePath>,
    Json(request): Json<PolicyReauthorizationRestRequest>,
) -> ApiResult<(
    axum::http::StatusCode,
    Json<PolicyReauthorizationRestResult>,
)> {
    use crate::product::models::automation::OperationState;
    use axum::http::StatusCode;

    let app_paths = product_app_paths(&state);
    let store = CodingAttemptStore::new(app_paths.clone());
    let attempt = resolve_coding_attempt(
        &store,
        path.project_id.as_deref(),
        path.issue_id.as_deref(),
        &path.attempt_id,
    )?;
    if request.attempt_id != attempt.id {
        return Err(ApiError::validation(
            "policy_reauthorization_attempt_mismatch",
            "request attempt_id must match the addressed attempt",
        ));
    }
    let result = crate::product::logical_codebase::handle_policy_reauthorization(
        &app_paths,
        &attempt.project_id,
        &attempt.issue_id,
        &crate::product::logical_codebase::evidence_mediator::PolicyReauthorizationRequest {
            command_id: request.command_id.clone(),
            attempt_id: request.attempt_id.clone(),
            role: request.role.clone(),
            policy_digest: request.policy_digest.clone(),
            expected_version: request.expected_version,
        },
    )
    .map_err(crate::web::handlers::evidence_error_mapping::policy_api_error)?;
    let code = match result.state {
        OperationState::Accepted | OperationState::Replayed => StatusCode::OK,
        OperationState::NeedsHuman => StatusCode::ACCEPTED,
        OperationState::Rejected => StatusCode::CONFLICT,
    };
    Ok((
        code,
        Json(PolicyReauthorizationRestResult {
            command_id: result.command_id,
            state: result.state,
            attempt_id: result.attempt_id,
            reason: result.reason,
            expires_at: result.expires_at,
        }),
    ))
}

/// 引擎 reason code 直达映射：验证处理／重跑的 fail-closed reason code
/// （`verification_triage_*`／`coding_rerun_*`）保持稳定码透传（HTTP 状态
/// 段在 `web/error.rs` 集中登记）；其余走既有引擎映射。
fn verification_surface_engine_error(
    error: crate::product::coding_workspace_engine::CodingWorkspaceEngineError,
) -> ApiError {
    if let crate::product::coding_workspace_engine::CodingWorkspaceEngineError::ProviderStream(
        reason,
    ) = &error
    {
        let code = reason.split(':').next().unwrap_or_default().trim();
        if code.starts_with("verification_triage_") || code.starts_with("coding_rerun_") {
            return ApiError::runtime(code.to_string(), reason.clone(), serde_json::json!({}));
        }
    }
    if let crate::product::coding_workspace_engine::CodingWorkspaceEngineError::Store(store_error) =
        error
    {
        // store 稳定码直达（如验证处理记录 NotFound → 404），不坍缩为
        // 泛化 500。
        return product_store_api_error(store_error);
    }
    coding_workspace_api_error(error)
}

#[cfg(test)]
mod tests {
    use axum::Json;
    use axum::extract::{Path, State};

    use crate::product::app_paths::ProductAppPaths;
    use crate::product::coding_attempt_store::CodingAttemptStore;
    use crate::product::coding_attempt_store::CreateCodingAttemptInput;
    use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
    use crate::product::json_store::validate_relative_id;
    use crate::product::models::automation::OperationState;
    use crate::web::handlers::coding::scope::CodingAttemptRoutePath;
    use crate::web::handlers::coding::verification_surface::*;
    use crate::web::state::WebAppState;
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_0001";

    async fn fixture() -> (
        tempfile::TempDir,
        WebAppState,
        CodingAttemptStore,
        crate::product::coding_models::CodingExecutionAttempt,
    ) {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = temp.path().to_path_buf();
        let state = WebAppState::new(
            root.clone(),
            crate::web::runtime::WebRuntime::new_fake(root.clone()),
        );
        let paths = ProductAppPaths::new(root.join(".aria"));
        IssueStore::new(paths.clone())
            .create(CreateProductIssueInput {
                project_id: PROJECT_ID.to_string(),
                repo_id: Some("repository_0001".to_string()),
                logical_codebase_id: None,
                title: "verification surface issue".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .expect("issue");
        let store = CodingAttemptStore::new(paths.clone());
        let attempt = store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: "work_item_0001".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/c2-verification-surface".to_string(),
                worktree_path: None,
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: crate::product::models::ProviderName::Fake,
                    reviewer: None,
                    review_rounds: 0,
                    permission_modes: Default::default(),
                },
                target_snapshot: None,
                max_auto_rework: 0,
            })
            .expect("attempt");
        (temp, state, store, attempt)
    }

    fn route(
        attempt: &crate::product::coding_models::CodingExecutionAttempt,
    ) -> Path<CodingAttemptRoutePath> {
        Path(CodingAttemptRoutePath {
            project_id: Some(attempt.project_id.clone()),
            issue_id: Some(attempt.issue_id.clone()),
            attempt_id: attempt.id.clone(),
        })
    }

    fn enter_request() -> Json<VerificationTriageEnterRestRequest> {
        Json(VerificationTriageEnterRestRequest {
            finding_id: "code_review_report_0001#0".to_string(),
            check_id: "check_run_tests".to_string(),
            original_command: None,
            alternative_command: Some("pnpm -C web exec vitest run src/lib.test.ts".to_string()),
            cwd: Some("/repo".to_string()),
            outcome: Some("3 passed".to_string()),
            test_execution_count: Some(3),
            environment: Some("linux".to_string()),
        })
    }

    /// 记录 GET：attempt 无记录时返回空列表＋attempt 版本（页面数据源契约）。
    #[tokio::test]
    async fn c2_verification_triage_records_get_returns_empty_list_with_version() {
        let (_tmp, state, _store, attempt) = fixture().await;
        let response = get_verification_triage_records(State(state), route(&attempt))
            .await
            .expect("records get succeeds");
        assert_eq!(response.attempt_id, attempt.id);
        assert_eq!(response.attempt_version, attempt.version);
        assert!(response.records.is_empty());
    }

    /// 转入（负例）：无可转入门时引擎 fail-closed reason code 直达
    /// （证明接线到引擎应用服务而非本地复制判定）。
    #[tokio::test]
    async fn c2_verification_triage_enter_surfaces_engine_fail_closed_reason() {
        let (_tmp, state, _store, attempt) = fixture().await;
        let error = post_verification_triage_enter(State(state), route(&attempt), enter_request())
            .await
            .expect_err("enter without eligible gate must fail closed");
        assert_eq!(error.code, "verification_triage_no_eligible_gate");
    }

    /// 决定（负例）：不存在的 triage 记录 → store NotFound 映射直达。
    #[tokio::test]
    async fn c2_verification_triage_decision_surfaces_missing_record() {
        let (_tmp, state, _store, attempt) = fixture().await;
        let path = Path(VerificationTriageDecisionRoutePath {
            project_id: Some(attempt.project_id.clone()),
            issue_id: Some(attempt.issue_id.clone()),
            attempt_id: attempt.id.clone(),
            triage_id: "verification_triage_9999".to_string(),
        });
        let request = Json(VerificationTriageDecisionRestRequest {
            conclusion:
                crate::product::coding_attempt_store::VerificationTriageConclusion::AcceptEquivalentEvidence,
            decided_by: "human".to_string(),
            reason: "等价证据".to_string(),
            exemption_scope: Vec::new(),
        });
        let error = post_verification_triage_decision(State(state), path, request)
            .await
            .expect_err("decision on missing record must surface not found");
        assert!(
            error.code.contains("not_found"),
            "unexpected code: {}",
            error.code
        );
    }

    /// 并列证据（负例）：无活动 coding unit／plan 绑定时引擎 fail-closed
    /// reason code 直达（不本地推断）。
    #[tokio::test]
    async fn c2_verification_command_evidence_surfaces_engine_fail_closed() {
        let (_tmp, state, _store, attempt) = fixture().await;
        let error = get_verification_command_evidence(State(state), route(&attempt))
            .await
            .expect_err("evidence without plan binding must fail closed");
        assert_eq!(
            error.code, "verification_triage_plan_revision_unresolvable",
            "unexpected code: {}",
            error.code
        );
    }

    /// 重跑（版本负例）：expected 版本与 attempt.rework_count 不符 → 409
    /// 语义错误直达（不触发返修、不启动 provider），attempt 不被改动。
    #[tokio::test]
    async fn c2_rerun_planned_command_stale_version_fails_closed() {
        let (_tmp, state, store, attempt) = fixture().await;
        let request = Json(RerunPlannedCommandRestRequest {
            command_id: "cmd-c2-rerun-0001".to_string(),
            gate_id: "coding_blocked_gate_0001".to_string(),
            check_id: "check_run_tests".to_string(),
            expected_version: 99,
        });
        let error = post_rerun_planned_command(State(state), route(&attempt), request)
            .await
            .expect_err("stale rerun version must fail closed");
        assert!(
            error.code.contains("coding_rerun_version_conflict"),
            "unexpected code: {}",
            error.code
        );
        let after = store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("attempt after");
        assert_eq!(after.rework_count, attempt.rework_count);
        assert_eq!(after.status, attempt.status);
    }

    /// 政策读取（负例）：attempt 不在 blocked／rework 等待面 → mediator
    /// NotAvailable 直达（证明接线到 read_policy_text_for_attempt）。
    #[tokio::test]
    async fn c2_policy_text_read_surfaces_not_available_for_running_attempt() {
        let (_tmp, state, _store, attempt) = fixture().await;
        let error = get_coding_policy_text(State(state), route(&attempt))
            .await
            .expect_err("policy read off waiting surface must fail closed");
        assert!(
            error.code.contains("policy_not_available"),
            "unexpected code: {}",
            error.code
        );
    }

    /// 重新授权（版本负例）：expected 版本不符 → Rejected 落 Task 2 命令
    /// 账本（409 语义），不签发、attempt 不变。
    #[tokio::test]
    async fn c2_policy_reauthorization_stale_version_rejected_into_ledger() {
        let (_tmp, state, store, attempt) = fixture().await;
        let request = Json(PolicyReauthorizationRestRequest {
            command_id: "cmd-c2-policy-reauth-0001".to_string(),
            attempt_id: attempt.id.clone(),
            role: "coder".to_string(),
            policy_digest: "0".repeat(64),
            expected_version: 99,
        });
        let (status, body) =
            post_coding_policy_reauthorization(State(state), route(&attempt), request)
                .await
                .expect("stale version handled as rejected body");
        assert_eq!(status, axum::http::StatusCode::CONFLICT);
        assert_eq!(body.state, OperationState::Rejected);
        let ledger = store
            .find_attempt_command_result(
                &attempt.project_id,
                &attempt.issue_id,
                &attempt.id,
                "cmd-c2-policy-reauth-0001",
            )
            .expect("ledger read")
            .expect("rejected result recorded");
        assert_eq!(ledger.state, OperationState::Rejected);
        validate_relative_id("cmd-c2-policy-reauth-0001").expect("command id stays relative");
    }
}
