//! enrollment REST 入口（P0 1.2，REQ-WIGA-01/02）：PUT/GET/绑定三端点。
//!
//! PUT 在 CAS 之前做精确作用域校验（issue 存在、确认的 story/design
//! id+version、design 引用 story、单 logical member 恰一且与 target 一致）；
//! Disable 只查存在与 revision。绑定只接受显式 POST 的 plan/session，
//! 旧 plan 不隐式认领。GET 是读投影：不存在返回 200+null 而非 404。

use std::collections::BTreeSet;

use axum::Json;
use axum::extract::{Path, State};
use chrono::DateTime;
use serde_json::json;

use crate::product::issue_automation_store::IssueAutomationStore;
use crate::product::issue_store::IssueStore;
use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::LifecycleConfirmationStatus;
use crate::product::models::WorkspaceType;
use crate::product::models::automation::{
    EnrollmentBindingIdentityInput, EnrollmentError, EnrollmentRebindRequest,
    EnrollmentRebindResult, EnrollmentSource, EnrollmentWriteCommand, IssueAutomationEnrollment,
    LeaseTakeoverRequest, LeaseTakeoverResult, OperationState,
};
use crate::product::work_item_plan_policy::RunPolicy;
use crate::web::error::{ApiError, ApiResult};
use crate::web::handlers::lifecycle::preflight::logical_repository_ids_for_preflight;
use crate::web::state::WebAppState;
use crate::web::types::{EnrollmentBindRequest, EnrollmentPutRequest};

use super::support::{product_app_paths, product_store_api_error};

pub async fn put_automation_enrollment(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Json(request): Json<EnrollmentPutRequest>,
) -> ApiResult<Json<IssueAutomationEnrollment>> {
    validate_request_ids(&project_id, &issue_id)?;
    validate_enrollment_scope(&state, &project_id, &issue_id, &request.command)?;
    let enrollment = IssueAutomationStore::new(product_app_paths(&state))
        .compare_and_set(
            &project_id,
            &issue_id,
            request.expected_revision,
            request.command,
        )
        .map_err(enrollment_api_error)?;
    if enrollment.enabled {
        // P1 WIGA Task 6：成功启用只发唤醒 hint，不绑定运行生命周期；
        // 有界 tick 与 durable reconcile 是权威推进。
        let _ = state.autopilot_wake.send(true);
    }
    Ok(Json(enrollment))
}

pub async fn get_automation_enrollment(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
) -> ApiResult<Json<Option<IssueAutomationEnrollment>>> {
    validate_request_ids(&project_id, &issue_id)?;
    ensure_issue_exists(&state, &project_id, &issue_id)?;
    IssueAutomationStore::new(product_app_paths(&state))
        .get(&project_id, &issue_id)
        .map(Json)
        .map_err(product_store_api_error)
}

pub async fn post_automation_enrollment_binding(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Json(request): Json<EnrollmentBindRequest>,
) -> ApiResult<Json<IssueAutomationEnrollment>> {
    validate_request_ids(&project_id, &issue_id)?;
    if let Err(error) = validate_relative_id(&request.plan_id) {
        return Err(binding_target_invalid("plan_id", error));
    }
    if let Err(error) = validate_relative_id(&request.session_id) {
        return Err(binding_target_invalid("session_id", error));
    }
    ensure_issue_exists(&state, &project_id, &issue_id)?;

    let paths = product_app_paths(&state);
    let store = IssueAutomationStore::new(paths.clone());
    let enrollment = store
        .get(&project_id, &issue_id)
        .map_err(product_store_api_error)?
        .ok_or_else(enrollment_not_found)?;
    if !enrollment.enabled {
        return Err(enrollment_conflict(
            None,
            "automation enrollment is disabled",
        ));
    }

    // 绑定前复查授权 source 未漂移（精确 id+version，不猜 latest）。
    let lifecycle = LifecycleStore::new(paths.clone());
    validate_confirmed_source_refs(&lifecycle, &project_id, &issue_id, &enrollment.source)?;

    // 只绑定显式指定的 plan/session：目标存在、同 issue、类型/策略/身份一致，
    // 且 plan 不早于 enrollment 创建（旧 plan 不能隐式认领）。
    let plan = lifecycle
        .list_issue_work_item_plans(&project_id, &issue_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .find(|plan| plan.id == request.plan_id)
        .ok_or_else(|| binding_target_not_found("plan"))?;
    let session = lifecycle
        .list_workspace_sessions(&project_id, &issue_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .find(|session| session.id == request.session_id)
        .ok_or_else(|| binding_target_not_found("session"))?;
    if session.workspace_type != WorkspaceType::WorkItemPlan {
        return Err(invalid_scope(
            "binding target session must be a work item plan session",
        ));
    }
    if session.entity_id != plan.id {
        return Err(invalid_scope(
            "binding target session entity must match the plan",
        ));
    }
    // 批 2 P1（REQ-WIGA-02 修订）：绑定会话 run policy 必须与 enrollment
    // 冻结的 plan_approval 策略一致——auto 冻结放行 AutoIfValid（自动链
    // 绑定准入），manual 仍只认 Interactive（AutoIfValid 不代表人工授权；
    // 错绑/降级一律拒绝）。
    if session.run_policy != enrollment.options.plan_approval.expected_run_policy() {
        return Err(enrollment_conflict(
            None,
            "binding target session run policy must match the enrollment's frozen plan approval",
        ));
    }
    if created_before(&plan.created_at, &enrollment.created_at) {
        return Err(invalid_scope(
            "plan created before the enrollment cannot be claimed implicitly",
        ));
    }

    store
        .bind_plan(
            &project_id,
            &issue_id,
            request.expected_revision,
            &request.plan_id,
            &request.session_id,
        )
        .map(Json)
        .map_err(enrollment_api_error)
}

/// C1 Task 2（REQ-WIGA-01）：显式重绑/换代 REST。先验证 issue/source/
/// plan/session/target/provider 的精确身份（不猜 latest；run policy 须与
/// enrollment 冻结的 plan_approval 一致——批 2 P1 起 auto 冻结认 AutoIfValid），
/// 再由 store 在 enrollment 文件锁内按 expected policy/binding CAS 换代；
/// Accepted 只发唤醒 hint，Replayed 不再触发编排。
pub async fn post_automation_enrollment_rebind(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Json(request): Json<EnrollmentRebindRequest>,
) -> ApiResult<Json<EnrollmentRebindResult>> {
    validate_request_ids(&project_id, &issue_id)?;
    ensure_issue_exists(&state, &project_id, &issue_id)?;

    let paths = product_app_paths(&state);
    let store = IssueAutomationStore::new(paths.clone());
    let enrollment = store
        .get(&project_id, &issue_id)
        .map_err(product_store_api_error)?
        .ok_or_else(enrollment_not_found)?;
    if !enrollment.enabled {
        return Err(enrollment_conflict(
            None,
            "automation enrollment is disabled",
        ));
    }

    // 新代 source 必须是已确认的精确事实（id+version，不猜 latest）。
    let lifecycle = LifecycleStore::new(paths.clone());
    validate_confirmed_source_refs(&lifecycle, &project_id, &issue_id, &request.binding.source)?;

    // 新代 plan/session 精确身份：存在、同 issue、类型/实体一致且 run
    // policy 与 enrollment 冻结的 plan_approval 一致（manual 只认
    // Interactive、auto 认 AutoIfValid；换代是用户显式提交，不按
    // created_at 排除旧 plan——排除的只是隐式认领）。
    validate_rebind_binding_target(
        &lifecycle,
        &project_id,
        &issue_id,
        enrollment.options.plan_approval.expected_run_policy(),
        &request.binding,
    )?;
    // 换代换 provider 同样过完整角色链同源预检(C5 Task 3/Task 8)。
    // rebind 手上只有 enrollment 声明的 target——carrier 取声明值,不
    // 重解析 issue 权威载体;旧代 enrollment(无 target)由 store rebind
    // 语义兜底拒绝。Task 8b:沿 declared target 的现有 LC 组装 readonly
    // gateway(读取现有 LC/成员而不猜 issue);缺 checkout 材料列 reason,
    // 不建 worktree/provider。
    if let Some(declared) = enrollment.target.as_ref() {
        // 批 2 R1：集合声明同属 LC 载体——沿声明 LC 组装一次 readonly
        // gateway（五角色 verdict 复用，不按仓循环、不加健康探测）。
        let gateway = match declared.logical_codebase_id() {
            Some(logical_codebase_id) => {
                super::automation_gateway_preflight::readonly_preflight_gateway(
                    &state,
                    &project_id,
                    Some(logical_codebase_id),
                )
            }
            // 单仓声明的载体类别在判定核心内走原跳过,无需 gateway。
            None => None,
        };
        super::automation_gateway_preflight::validate_role_chain_for_declared_enrollment_target(
            gateway.as_ref(),
            &request.binding.author_provider,
            &request.binding.reviewer_provider,
            declared,
            true,
            state.test_provider_enabled,
        )?;
    }

    let result = store
        .rebind(&project_id, &issue_id, request)
        .map_err(enrollment_api_error)?;
    if result.state == OperationState::Accepted {
        let _ = state.autopilot_wake.send(true);
    }
    Ok(Json(result))
}

/// C1 Task 6（REQ-WIGA-03）：死亡租约确认接管薄入口。只调用
/// `CodingWorkspaceEngine::confirm_takeover` 应用服务（分类/判定/owner
/// CAS 全在服务内），不承载业务逻辑、不复用 workspace-session takeover
/// 的 stopped_needs_human 语义。`Accepted` 只发唤醒 hint。
pub async fn post_automation_enrollment_lease_takeover(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Json(request): Json<LeaseTakeoverRequest>,
) -> ApiResult<Json<LeaseTakeoverResult>> {
    validate_request_ids(&project_id, &issue_id)?;
    ensure_issue_exists(&state, &project_id, &issue_id)?;

    let paths = product_app_paths(&state);
    let store = crate::product::coding_attempt_store::CodingAttemptStore::new(paths);
    let (event_tx, _event_rx) =
        tokio::sync::mpsc::channel::<crate::web::coding_ws_handler::CodingWsOutMessage>(1);
    let engine = crate::product::coding_workspace_engine::CodingWorkspaceEngine::new(
        store,
        crate::product::git_workspace_service::GitWorkspaceService::new(),
        event_tx,
    );
    let result = engine
        .confirm_takeover(&project_id, &issue_id, &request)
        .map_err(lease_takeover_api_error)?;
    if result.state == OperationState::Accepted {
        let _ = state.autopilot_wake.send(true);
    }
    Ok(Json(result))
}

/// C1 Task 6：接管链错误映射——过期 binding/lease/attempt 的
/// `IdentityMismatch` → 409 稳定码；账本异 payload Conflict → 409。
fn lease_takeover_api_error(error: ProductStoreError) -> ApiError {
    match error {
        ProductStoreError::IdentityMismatch { kind, id } => ApiError::validation_with_details(
            "lease_takeover_identity_mismatch",
            format!("lease takeover rejected: {kind} {id}"),
            json!({ "kind": kind, "id": id }),
        ),
        ProductStoreError::Conflict {
            kind: "enrollment_command_ledger",
            id,
        } => ApiError::validation_with_details(
            "lease_takeover_command_conflict",
            format!("command ledger conflict: {id}"),
            json!({ "command_id": id }),
        ),
        other => product_store_api_error(other),
    }
}

/// rebind 新代绑定目标的精确身份校验（与显式 binding 同一存在性/一致性
/// 口径；run policy 须等于 enrollment 冻结 plan_approval 的期望值——批 2
/// P1 起 auto 冻结认 AutoIfValid、manual 只认 Interactive；不含 created_at
/// 旧 plan 排除——那是隐式认领的门，不是显式换代的）。
fn validate_rebind_binding_target(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    expected_policy: RunPolicy,
    binding: &EnrollmentBindingIdentityInput,
) -> ApiResult<()> {
    let plan = lifecycle
        .list_issue_work_item_plans(project_id, issue_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .find(|plan| plan.id == binding.plan_id)
        .ok_or_else(|| binding_target_not_found("plan"))?;
    let session = lifecycle
        .list_workspace_sessions(project_id, issue_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .find(|session| session.id == binding.session_id)
        .ok_or_else(|| binding_target_not_found("session"))?;
    if session.workspace_type != WorkspaceType::WorkItemPlan {
        return Err(invalid_scope(
            "rebind target session must be a work item plan session",
        ));
    }
    if session.entity_id != plan.id {
        return Err(invalid_scope(
            "rebind target session entity must match the plan",
        ));
    }
    if session.run_policy != expected_policy {
        return Err(enrollment_conflict(
            None,
            "rebind target session run policy must match the enrollment's frozen plan approval",
        ));
    }
    Ok(())
}

pub(crate) fn validate_request_ids(project_id: &str, issue_id: &str) -> ApiResult<()> {
    validate_relative_id(project_id)
        .map_err(|_| ApiError::validation("invalid_project_id", "project id must be relative"))?;
    validate_relative_id(issue_id)
        .map_err(|_| ApiError::validation("invalid_issue_id", "issue id must be relative"))?;
    Ok(())
}

pub(crate) fn ensure_issue_exists(
    state: &WebAppState,
    project_id: &str,
    issue_id: &str,
) -> ApiResult<()> {
    IssueStore::new(product_app_paths(state))
        .get(project_id, issue_id)
        .map(|_| ())
        .map_err(product_store_api_error)
}

/// Enable 前的精确作用域校验；Disable 只要求 enrollment 存在（store NotFound 兜底），
/// 不重验可能已失效的 source。
fn validate_enrollment_scope(
    state: &WebAppState,
    project_id: &str,
    issue_id: &str,
    command: &EnrollmentWriteCommand,
) -> ApiResult<()> {
    ensure_issue_exists(state, project_id, issue_id)?;
    let EnrollmentWriteCommand::Enable {
        source,
        options,
        target,
        ..
    } = command
    else {
        return Ok(());
    };

    let paths = product_app_paths(state);
    let lifecycle = LifecycleStore::new(paths.clone());
    validate_confirmed_source_refs(&lifecycle, project_id, issue_id, source)?;

    // C5 Task 2：载体判定与 GET 投影共用唯一 resolver 入口（Review
    // Focus 5）；issue 记录为权威 repo_id 来源。
    let issue = IssueStore::new(paths.clone())
        .get(project_id, issue_id)
        .map_err(product_store_api_error)?;
    let carrier = super::support::resolve_automation_carrier(&paths, project_id, &issue)?;
    match &carrier {
        super::support::AutomationCarrierResolution::SingleRepository {
            target: authoritative,
        } => {
            // 单仓载体：提交 target 必须与权威解析逐字节相等（错仓/跨载体
            // 422 重选提示，不猜、不降级）；单仓跳过 gateway 谓词
            //（Review Focus 5，A10 单仓不误拒）。
            if *target != *authoritative {
                return Err(invalid_scope(
                    "automation enrollment target must match the issue's single physical \
                     repository; re-select the automation target and retry",
                ));
            }
            Ok(())
        }
        super::support::AutomationCarrierResolution::LogicalCodebase { resolution } => {
            // 批 2 R1（REQ-WIGA-01、REQ-MTG-03）：LC 载体按 selection 候选
            // 构造 canonical target（0 拒/1 旧 kind/N 集合）——提交 target
            // 必须与 canonical 全等（乱序/重复已在反序列化归一），错配
            // 422 重选提示，不猜、不降级、不以 plan focus 上界替代授权集。
            let manifest = resolution.manifest.clone().ok_or_else(|| {
                invalid_scope("automation enrollment requires a logical codebase manifest routing")
            })?;
            let selection = resolution.selection.as_ref().ok_or_else(|| {
                invalid_scope(
                    "automation enrollment requires an explicit logical codebase selection",
                )
            })?;
            let candidate_ids = logical_repository_ids_for_preflight(&manifest, selection);
            let expected = canonical_lc_enrollment_target(
                &manifest.logical_codebase_id.to_string(),
                &candidate_ids,
            )?;
            if target != &expected {
                return Err(invalid_scope(
                    "automation enrollment target must match the issue's authorized logical \
                     member set; re-select the automation target and retry",
                ));
            }
            // C5 Task 3/Task 8：canonical target 确认后做完整角色链同源
            // 预检——与 GET automation-target 投影同一 carrier、同一判定,
            // Enable 前拒绝,一次列全全部违规角色。批 2 R1：集合同属 LC，
            // 一次 LC 预检覆盖全部成员（不按仓循环）。Task 8b:readonly
            // gateway 接线(resolver 冻结的权威 lc_id),组装失败 fail-closed。
            let gateway = super::automation_gateway_preflight::readonly_preflight_gateway(
                state,
                project_id,
                resolution.target.logical_codebase_id.as_deref(),
            );
            super::automation_gateway_preflight::validate_role_chain_for_enrollment(
                gateway.as_ref(),
                &options.author_provider,
                &options.reviewer_provider,
                &carrier,
                true,
                state.test_provider_enabled,
            )
        }
    }
}

/// 批 2 R1：GET 投影与 PUT Enable 共用的 canonical LC target 构造。
/// 候选（manifest ∩ selection，升序唯一）0 个即拒绝；恰一折叠旧
/// `LogicalCodebase` kind；多个落 `LogicalCodebaseMembers` 集合。
pub(crate) fn canonical_lc_enrollment_target(
    logical_codebase_id: &str,
    candidate_ids: &[String],
) -> ApiResult<crate::product::logical_codebase::EnrollmentTarget> {
    let members = candidate_ids
        .iter()
        .map(|candidate| {
            uuid::Uuid::parse_str(candidate)
                .map(crate::product::logical_codebase::LogicalRepositoryId)
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| {
            invalid_scope("automation enrollment logical repository id is not a valid uuid")
        })?;
    crate::product::logical_codebase::EnrollmentTarget::canonical_logical_members(
        logical_codebase_id.to_string(),
        members,
    )
    .ok_or_else(|| invalid_scope("automation target requires at least one logical repository"))
}

/// 精确 source 绑定（REQ-WIGA-01）：id 存在、已确认、current_version 与引用一致
/// （版本不存在即拒绝，不猜 latest）；design 必须引用 enrollment 的 story。
fn validate_confirmed_source_refs(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    source: &EnrollmentSource,
) -> ApiResult<()> {
    if source.stories.is_empty() || source.designs.is_empty() {
        return Err(invalid_scope(
            "automation enrollment requires confirmed story and design sources",
        ));
    }
    let stories = lifecycle
        .list_story_specs(project_id, issue_id)
        .map_err(product_store_api_error)?;
    for story_ref in &source.stories {
        let Some(story) = stories.iter().find(|story| story.id == story_ref.id) else {
            return Err(invalid_scope(format!(
                "story spec {} not found for automation enrollment",
                story_ref.id
            )));
        };
        if story.confirmation_status != LifecycleConfirmationStatus::Confirmed {
            return Err(invalid_scope(format!(
                "story spec {} must be confirmed before enabling automation",
                story_ref.id
            )));
        }
        if story.current_version != Some(story_ref.version) {
            return Err(invalid_scope(format!(
                "story spec {} version {} does not match current version {:?}; refusing to guess latest",
                story_ref.id, story_ref.version, story.current_version
            )));
        }
    }
    let designs = lifecycle
        .list_design_specs(project_id, issue_id)
        .map_err(product_store_api_error)?;
    let story_ids = source
        .stories
        .iter()
        .map(|story| story.id.as_str())
        .collect::<BTreeSet<_>>();
    for design_ref in &source.designs {
        let Some(design) = designs.iter().find(|design| design.id == design_ref.id) else {
            return Err(invalid_scope(format!(
                "design spec {} not found for automation enrollment",
                design_ref.id
            )));
        };
        if design.confirmation_status != LifecycleConfirmationStatus::Confirmed {
            return Err(invalid_scope(format!(
                "design spec {} must be confirmed before enabling automation",
                design_ref.id
            )));
        }
        if design.current_version != Some(design_ref.version) {
            return Err(invalid_scope(format!(
                "design spec {} version {} does not match current version {:?}; refusing to guess latest",
                design_ref.id, design_ref.version, design.current_version
            )));
        }
        if !design
            .story_spec_ids
            .iter()
            .any(|story_id| story_ids.contains(story_id.as_str()))
        {
            return Err(invalid_scope(format!(
                "design spec {} must reference the enrolled story specs",
                design_ref.id
            )));
        }
    }
    Ok(())
}

fn created_before(plan_created_at: &str, enrollment_created_at: &str) -> bool {
    match (
        DateTime::parse_from_rfc3339(plan_created_at),
        DateTime::parse_from_rfc3339(enrollment_created_at),
    ) {
        (Ok(plan), Ok(enrollment)) => plan < enrollment,
        _ => plan_created_at < enrollment_created_at,
    }
}

pub(crate) fn invalid_scope(reason: impl Into<String>) -> ApiError {
    ApiError::validation("automation_enrollment_invalid_scope", reason)
}

fn enrollment_conflict(current_revision: Option<u64>, message: &str) -> ApiError {
    ApiError::validation_with_details(
        "automation_enrollment_conflict",
        message,
        json!({ "current_revision": current_revision }),
    )
}

fn enrollment_not_found() -> ApiError {
    ApiError::runtime(
        "automation_enrollment_not_found",
        "automation enrollment not found",
        json!({}),
    )
}

fn binding_target_invalid(field: &str, error: ProductStoreError) -> ApiError {
    ApiError::validation(
        "automation_enrollment_invalid_scope",
        format!("binding target {field} is invalid: {error}"),
    )
}

fn binding_target_not_found(kind: &str) -> ApiError {
    ApiError::runtime(
        "automation_enrollment_not_found",
        format!("binding target {kind} not found"),
        json!({}),
    )
}

pub(crate) fn enrollment_api_error(error: EnrollmentError) -> ApiError {
    match error {
        EnrollmentError::Conflict { current_revision } => {
            enrollment_conflict(current_revision, "automation enrollment revision conflict")
        }
        EnrollmentError::InvalidScope(reason) => invalid_scope(reason),
        EnrollmentError::NotFound => enrollment_not_found(),
        EnrollmentError::Store(error) => product_store_api_error(error),
    }
}
#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use crate::product::models::LifecycleConfirmationStatus;
    use crate::product::work_item_plan_policy::RunPolicy;

    use super::super::automation_enrollment_test_support::{
        ISSUE_ID, PROJECT_ID, SINGLE_LOGICAL_ID, create_plan_and_session, enrollment_body,
        enrollment_body_with_target, enrollment_file_exists, fixture_logical_codebase_id,
        get_enrollment, post_binding, post_rebind, put_enrollment, rebind_body, response_json,
        seed_fixture,
    };

    #[tokio::test]
    async fn automation_enrollment_http_get_returns_null_when_absent() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();

        let response = get_enrollment(&app).await;
        assert_eq!(response.status(), StatusCode::OK);
        let payload = response_json(response).await;
        assert!(payload.is_null());
    }

    #[tokio::test]
    async fn automation_enrollment_http_put_rejects_unconfirmed_or_stale_source() {
        let fixture = seed_fixture(1, false);
        let app = fixture.router();

        // design 未确认 → 422，且不落 enrollment 文件。
        let response = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!enrollment_file_exists(&fixture));

        // 确认 design 后，引用不存在的版本（2 > current 1）→ 422，不猜 latest。
        fixture
            .lifecycle
            .update_spec_confirmation_status(
                PROJECT_ID,
                ISSUE_ID,
                &fixture.design_id,
                LifecycleConfirmationStatus::Confirmed,
            )
            .unwrap();
        let response = put_enrollment(&app, enrollment_body(&fixture, 1, 2)).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!enrollment_file_exists(&fixture));

        // 未知 story id 同样拒绝。
        let mut body = enrollment_body(&fixture, 1, 1);
        body["command"]["source"]["stories"][0]["id"] = serde_json::json!("story_spec_9999");
        let response = put_enrollment(&app, body).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!enrollment_file_exists(&fixture));
    }

    #[tokio::test]
    async fn automation_enrollment_http_put_is_idempotent_and_conflicts_on_divergence() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();

        let first = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(first.status(), StatusCode::OK);
        let first_body = response_json(first).await;
        assert_eq!(first_body["policy_revision"], 1);
        assert!(first_body["enabled"].as_bool().unwrap());

        // 同键同 payload 重发：200，同 enrollment_id/revision。
        let second = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(second.status(), StatusCode::OK);
        let second_body = response_json(second).await;
        assert_eq!(second_body["enrollment_id"], first_body["enrollment_id"]);
        assert_eq!(second_body["policy_revision"], 1);

        // 异 options + 旧 revision → 409，details 携带当前 revision。
        let mut divergent = enrollment_body(&fixture, 1, 1);
        divergent["command"]["options"]["review_rounds"] = serde_json::json!(2);
        divergent["expected_revision"] = serde_json::json!(99);
        let response = put_enrollment(&app, divergent).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let payload = response_json(response).await;
        assert_eq!(payload["code"], "automation_enrollment_conflict");
        assert_eq!(payload["details"]["current_revision"], 1);

        // Disable 后以旧 revision Enable → 409。
        let disable = serde_json::json!({
            "expected_revision": 1,
            "command": {"type": "disable"}
        });
        let response = put_enrollment(&app, disable).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await["policy_revision"], 2);
        let response = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);

        // GET 投影读取当前（禁用）记录。
        let response = get_enrollment(&app).await;
        assert_eq!(response.status(), StatusCode::OK);
        let payload = response_json(response).await;
        assert_eq!(payload["enabled"], false);
        assert_eq!(payload["policy_revision"], 2);
    }

    /// C5 Task 1：Enable 必带 target——缺 target 的请求在反序列化层即
    /// 422 拒绝（旧 `logical_repository_id` 字段被忽略、不能替代），
    /// 零 enrollment 写入。
    #[tokio::test]
    async fn automation_enrollment_http_put_requires_target() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();
        let mut body = enrollment_body(&fixture, 1, 1);
        assert!(
            body["command"]
                .as_object_mut()
                .unwrap()
                .remove("target")
                .is_some()
        );
        // 旧字段存在也不能替代 target。
        body["command"]["logical_repository_id"] = serde_json::json!(SINGLE_LOGICAL_ID);
        let response = put_enrollment(&app, body).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!enrollment_file_exists(&fixture));
    }

    #[tokio::test]
    async fn automation_enrollment_http_put_rejects_zero_target_and_mismatched_member_set() {
        // 0 logical member（无 manifest/selection → Legacy 路由）→ 422。
        let fixture = seed_fixture(0, true);
        let app = fixture.router();
        let response = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!enrollment_file_exists(&fixture));

        // 2 logical member 下提交旧单成员 target → 与 canonical 集合不符，
        // 422 且零写入（批 2 R1：多成员授权必须全集合匹配）。
        let fixture = seed_fixture(2, true);
        let app = fixture.router();
        let response = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let payload = response_json(response).await;
        assert_eq!(payload["code"], "automation_enrollment_invalid_scope");
        assert!(!enrollment_file_exists(&fixture));
    }

    /// 批 2 R1 红测⑤：2 成员 selection 的 PUT 集合 target——乱序/重复输入
    /// 归一后与 canonical 全等即 200 落盘（enrollment/binding 冻结同一规范
    /// 形态；错配成员子集 422 零写入）。
    #[tokio::test]
    async fn automation_enrollment_http_put_accepts_normalized_member_set() {
        let fixture = seed_fixture(2, true);
        let app = fixture.router();
        let mut body = enrollment_body(&fixture, 1, 1);
        body["command"]["target"] = serde_json::json!({
            "kind": "logical_codebase_members",
            "logical_codebase_id": fixture_logical_codebase_id(&fixture),
            // 乱序 + 重复：反序列化归一为升序唯一后与 canonical 全等。
            "logical_repository_ids": [
                "00000000-0000-0000-0000-000000000002",
                "00000000-0000-0000-0000-000000000001",
                "00000000-0000-0000-0000-000000000002"
            ]
        });
        let response = put_enrollment(&app, body).await;
        assert_eq!(response.status(), StatusCode::OK);
        let enrollment = response_json(response).await;
        assert_eq!(enrollment["target"]["kind"], "logical_codebase_members");
        assert_eq!(
            enrollment["target"]["logical_repository_ids"],
            serde_json::json!([
                "00000000-0000-0000-0000-000000000001",
                "00000000-0000-0000-0000-000000000002"
            ])
        );
        assert_eq!(
            enrollment["binding_history"]["current"]["target"]["kind"],
            "logical_codebase_members"
        );
        assert!(enrollment_file_exists(&fixture));

        // 成员子集（缺一）→ canonical 不全等即 422 零写入。
        let mut subset = enrollment_body(&fixture, 1, 1);
        subset["command"]["target"] = serde_json::json!({
            "kind": "logical_codebase_members",
            "logical_codebase_id": fixture_logical_codebase_id(&fixture),
            "logical_repository_ids": ["00000000-0000-0000-0000-000000000001"]
        });
        let response = put_enrollment(&app, subset).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(
            response_json(response).await["code"],
            "automation_enrollment_invalid_scope"
        );
    }

    #[tokio::test]
    async fn automation_enrollment_http_binding_binds_explicit_interactive_plan_session() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();

        let enable = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(enable.status(), StatusCode::OK);
        let enrollment = response_json(enable).await;
        let revision = enrollment["policy_revision"].as_u64().unwrap();

        // manual 缺省 enrollment（无 plan_approval 字段）+ AutoIfValid 会话
        // → 409（REQ-WIGA-02 修订后口径：AutoIfValid 只对 auto 冻结放行）。
        let (auto_plan, auto_session) = create_plan_and_session(&fixture, RunPolicy::AutoIfValid);
        let response = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": revision,
                "plan_id": auto_plan,
                "session_id": auto_session,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);

        // Interactive plan/session 显式绑定成功。
        let (plan_id, session_id) = create_plan_and_session(&fixture, RunPolicy::Interactive);
        let response = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": revision,
                "plan_id": plan_id,
                "session_id": session_id,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let bound = response_json(response).await;
        assert_eq!(bound["plan_id"], plan_id.as_str());
        assert_eq!(bound["session_id"], session_id.as_str());
        let bound_revision = bound["policy_revision"].as_u64().unwrap();

        // 同键重试 → 200 同 revision；重绑他者 plan → 409。
        let response = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": bound_revision,
                "plan_id": plan_id,
                "session_id": session_id,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response_json(response).await["policy_revision"],
            bound_revision
        );

        let (other_plan, other_session) = create_plan_and_session(&fixture, RunPolicy::Interactive);
        let response = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": bound_revision,
                "plan_id": other_plan,
                "session_id": other_session,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);

        // 绑定目标不存在（未知 plan/session）→ 404。
        let response = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": bound_revision,
                "plan_id": "plan_9999",
                "session_id": "session_9999",
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn automation_enrollment_http_binding_rejects_plan_created_before_enrollment() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();

        // 旧 plan（先于 enrollment 创建）不能被认领 → 422。
        let (old_plan, old_session) = create_plan_and_session(&fixture, RunPolicy::Interactive);
        let enable = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(enable.status(), StatusCode::OK);
        let revision = response_json(enable).await["policy_revision"]
            .as_u64()
            .unwrap();
        let response = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": revision,
                "plan_id": old_plan,
                "session_id": old_session,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    // ---- C1 Task 2：显式重绑/换代 REST（REQ-WIGA-01、REQ-C1-TARGET-01）----

    #[tokio::test]
    async fn automation_enrollment_rebind_requires_current_binding() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();

        // 新式 enable：显式声明 target（logical 双级齐全）→ binding v1。
        let enable = put_enrollment(&app, enrollment_body_with_target(&fixture)).await;
        assert_eq!(enable.status(), StatusCode::OK);
        let enrolled = response_json(enable).await;
        assert_eq!(enrolled["binding_history"]["current"]["binding_version"], 1);

        // v1 显式绑定 plan/session。
        let revision = enrolled["policy_revision"].as_u64().unwrap();
        let (plan_1, session_1) = create_plan_and_session(&fixture, RunPolicy::Interactive);
        let response = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": revision,
                "plan_id": plan_1,
                "session_id": session_1,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let bound_revision = response_json(response).await["policy_revision"]
            .as_u64()
            .unwrap();

        // 合法 rebind → 200 accepted：current v2、previous 逐字保留 v1、
        // enrollment 投影换新代、policy_revision 递增（旧回执必然失配）。
        let (plan_2, session_2) = create_plan_and_session(&fixture, RunPolicy::Interactive);
        let rebind = rebind_body(
            &fixture,
            "rebind_cmd_0001",
            bound_revision,
            1,
            &plan_2,
            &session_2,
        );
        let response = post_rebind(&app, rebind.clone()).await;
        assert_eq!(response.status(), StatusCode::OK);
        let payload = response_json(response).await;
        assert_eq!(payload["command_id"], "rebind_cmd_0001");
        assert_eq!(payload["state"], "accepted");
        let after = payload["enrollment"].clone();
        assert_eq!(after["binding_history"]["current"]["binding_version"], 2);
        assert_eq!(
            after["binding_history"]["current"]["plan_id"],
            plan_2.as_str()
        );
        assert_eq!(
            after["binding_history"]["previous"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            after["binding_history"]["previous"][0]["plan_id"],
            plan_1.as_str()
        );
        assert_eq!(
            after["binding_history"]["previous"][0]["session_id"],
            session_1.as_str()
        );
        assert_eq!(after["plan_id"], plan_2.as_str());
        assert!(after["policy_revision"].as_u64().unwrap() > bound_revision);

        // durable 只有一份当前版本：GET 投影与返回一致。
        assert_eq!(response_json(get_enrollment(&app).await).await, after);

        // 同 command 同 payload 重放 → replayed，durable 不变。
        let replay = post_rebind(&app, rebind.clone()).await;
        assert_eq!(replay.status(), StatusCode::OK);
        let replay_body = response_json(replay).await;
        assert_eq!(replay_body["state"], "replayed");
        assert_eq!(replay_body["enrollment"], after);
        assert_eq!(response_json(get_enrollment(&app).await).await, after);

        // 同 command 异 payload（完整换绑定目标）→ 409，durable 不变。
        let mut diverged = rebind.clone();
        diverged["binding"]["plan_id"] = serde_json::json!(plan_1);
        diverged["binding"]["session_id"] = serde_json::json!(session_1);
        let response = post_rebind(&app, diverged).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(response_json(get_enrollment(&app).await).await, after);

        // 旧 expected binding version（当前已是 v2）→ 409。
        let after_revision = after["policy_revision"].as_u64().unwrap();
        let stale_binding = rebind_body(
            &fixture,
            "rebind_cmd_0002",
            after_revision,
            1,
            &plan_1,
            &session_1,
        );
        let response = post_rebind(&app, stale_binding).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);

        // 旧 expected policy revision → 409。
        let stale_policy = rebind_body(
            &fixture,
            "rebind_cmd_0003",
            bound_revision,
            2,
            &plan_1,
            &session_1,
        );
        let response = post_rebind(&app, stale_policy).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);

        // 跨载体 target（logical → single）→ 422，durable 不变。
        let mut cross_carrier = rebind_body(
            &fixture,
            "rebind_cmd_0004",
            after_revision,
            2,
            &plan_1,
            &session_1,
        );
        cross_carrier["binding"]["target"] = serde_json::json!({
            "kind": "single_repository",
            "repository_id": "repo_physical_1",
        });
        let response = post_rebind(&app, cross_carrier).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

        // 逻辑 target 缺 logical_codebase_id 一级 → 422（serde fail-closed）。
        let mut incomplete = rebind_body(
            &fixture,
            "rebind_cmd_0005",
            after_revision,
            2,
            &plan_1,
            &session_1,
        );
        incomplete["binding"]["target"]
            .as_object_mut()
            .unwrap()
            .remove("logical_codebase_id");
        let response = post_rebind(&app, incomplete).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

        // plan/session 不属于 issue → 404。
        let unknown_target = rebind_body(
            &fixture,
            "rebind_cmd_0006",
            after_revision,
            2,
            "plan_9999",
            "session_9999",
        );
        let response = post_rebind(&app, unknown_target).await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        // 全部拒绝后 durable 仍只有一份当前版本。
        assert_eq!(response_json(get_enrollment(&app).await).await, after);
    }

    // ---- 批 2 P1：plan_approval 冻结与 AutoIfValid 绑定准入（REQ-WIGA-02 修订）----

    /// ①auto 授权冻结 plan_approval，并把 require_execution_plan_confirm 压
    /// false（不论弹窗复选框原值）；同 payload 幂等；旧 wire 缺字段按
    /// manual 冻结且手动复选框原样保留（③④）。
    #[tokio::test]
    async fn automation_enrollment_http_put_freezes_auto_plan_approval() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();

        // auto + 复选框 true → 冻结 auto 且 require_execution_plan_confirm=false。
        let mut body = enrollment_body_with_target(&fixture);
        body["command"]["options"]["plan_approval"] = serde_json::json!("auto");
        body["command"]["options"]["plan_options"]["require_execution_plan_confirm"] =
            serde_json::json!(true);
        let first = put_enrollment(&app, body.clone()).await;
        assert_eq!(first.status(), StatusCode::OK);
        let enrolled = response_json(first).await;
        assert_eq!(enrolled["options"]["plan_approval"], "auto");
        assert_eq!(
            enrolled["options"]["plan_options"]["require_execution_plan_confirm"],
            false
        );

        // 同 payload（复选框仍 true）重发 → 幂等：同 enrollment_id/revision。
        let second = put_enrollment(&app, body).await;
        assert_eq!(second.status(), StatusCode::OK);
        let replayed = response_json(second).await;
        assert_eq!(replayed["enrollment_id"], enrolled["enrollment_id"]);
        assert_eq!(replayed["policy_revision"], enrolled["policy_revision"]);
        assert_eq!(
            replayed["options"]["plan_options"]["require_execution_plan_confirm"],
            false
        );

        // 旧 wire 缺 plan_approval 字段 → 冻结 manual；复选框 true 原样保留。
        let manual_fixture = seed_fixture(1, true);
        let manual_app = manual_fixture.router();
        let mut manual = enrollment_body_with_target(&manual_fixture);
        manual["command"]["options"]["plan_options"]["require_execution_plan_confirm"] =
            serde_json::json!(true);
        let response = put_enrollment(&manual_app, manual).await;
        assert_eq!(response.status(), StatusCode::OK);
        let manual_enrolled = response_json(response).await;
        assert_eq!(manual_enrolled["options"]["plan_approval"], "manual");
        assert_eq!(
            manual_enrolled["options"]["plan_options"]["require_execution_plan_confirm"],
            true
        );
    }

    /// ②binding/rebind 只放行与当前 enabled enrollment 冻结策略一致的
    /// run policy：auto 冻结放行 AutoIfValid（自动链绑定准入）、拒
    /// Interactive（降级）；manual 冻结的 rebind 对 AutoIfValid 仍拒绝
    /// （AutoIfValid 不代表人工授权，错绑拒绝）。
    #[tokio::test]
    async fn automation_enrollment_binding_and_rebind_follow_frozen_plan_approval() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();

        let mut body = enrollment_body_with_target(&fixture);
        body["command"]["options"]["plan_approval"] = serde_json::json!("auto");
        body["command"]["options"]["plan_options"]["require_execution_plan_confirm"] =
            serde_json::json!(true);
        let enable = put_enrollment(&app, body).await;
        assert_eq!(enable.status(), StatusCode::OK);
        let enrolled = response_json(enable).await;
        let revision = enrolled["policy_revision"].as_u64().unwrap();

        // 降级绑定：auto 冻结 + Interactive 会话 → 409。
        let (interactive_plan, interactive_session) =
            create_plan_and_session(&fixture, RunPolicy::Interactive);
        let response = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": revision,
                "plan_id": interactive_plan,
                "session_id": interactive_session,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);

        // 冻结策略一致的 AutoIfValid → 显式绑定成功。
        let (plan_id, session_id) = create_plan_and_session(&fixture, RunPolicy::AutoIfValid);
        let response = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": revision,
                "plan_id": plan_id,
                "session_id": session_id,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let bound = response_json(response).await;
        assert_eq!(bound["plan_id"], plan_id.as_str());
        assert_eq!(bound["session_id"], session_id.as_str());
        let bound_revision = bound["policy_revision"].as_u64().unwrap();
        let binding_version = bound["binding_history"]["current"]["binding_version"]
            .as_u64()
            .unwrap();

        // rebind 换代：新代 AutoIfValid → accepted。
        let (next_plan, next_session) = create_plan_and_session(&fixture, RunPolicy::AutoIfValid);
        let rebind = rebind_body(
            &fixture,
            "rebind_auto_0001",
            bound_revision,
            binding_version,
            &next_plan,
            &next_session,
        );
        let response = post_rebind(&app, rebind.clone()).await;
        assert_eq!(response.status(), StatusCode::OK);
        let accepted = response_json(response).await;
        assert_eq!(accepted["state"], "accepted");
        let after_revision = accepted["enrollment"]["policy_revision"].as_u64().unwrap();

        // 同 command 同 payload 重放 → replayed（幂等回归）。
        let response = post_rebind(&app, rebind).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await["state"], "replayed");

        // rebind 降级到 Interactive 新代 → 409，durable 不变。
        let (downgrade_plan, downgrade_session) =
            create_plan_and_session(&fixture, RunPolicy::Interactive);
        let downgrade = rebind_body(
            &fixture,
            "rebind_auto_0002",
            after_revision,
            binding_version + 1,
            &downgrade_plan,
            &downgrade_session,
        );
        let response = post_rebind(&app, downgrade).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(
            response_json(get_enrollment(&app).await).await["policy_revision"],
            after_revision
        );

        // manual 冻结：rebind 对 AutoIfValid 新代仍拒绝（错绑）。
        let manual_fixture = seed_fixture(1, true);
        let manual_app = manual_fixture.router();
        let enable =
            put_enrollment(&manual_app, enrollment_body_with_target(&manual_fixture)).await;
        let manual_revision = response_json(enable).await["policy_revision"]
            .as_u64()
            .unwrap();
        let (manual_plan, manual_session) =
            create_plan_and_session(&manual_fixture, RunPolicy::Interactive);
        let response = post_binding(
            &manual_app,
            serde_json::json!({
                "expected_revision": manual_revision,
                "plan_id": manual_plan,
                "session_id": manual_session,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let manual_bound = response_json(response).await;
        let (wrong_plan, wrong_session) =
            create_plan_and_session(&manual_fixture, RunPolicy::AutoIfValid);
        let wrong = rebind_body(
            &manual_fixture,
            "rebind_manual_0001",
            manual_bound["policy_revision"].as_u64().unwrap(),
            1,
            &wrong_plan,
            &wrong_session,
        );
        let response = post_rebind(&manual_app, wrong).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    // ---- P1 WIGA Task 4：enrollment-bound 唯一创建与半提交恢复 ----

    use crate::product::issue_automation_store::IssueAutomationStore;
    use crate::product::json_store::write_json;
    use crate::product::lifecycle_store::{CreateIssueWorkItemPlanInput, LifecycleStore};
    use crate::product::models::automation::{IssueAutomationEnrollment, PreparedPlanIntent};
    use crate::product::models::{
        IssueWorkItemPlan, IssueWorkItemPlanStatus, WorkspaceSessionRecord,
    };
    use crate::web::error::ApiResult;
    use crate::web::handlers::lifecycle::plan_preparation::ensure_enrolled_plan;
    use crate::web::runtime::WebRuntime;
    use crate::web::state::WebAppState;

    impl super::super::automation_enrollment_test_support::Fixture {
        /// 中窗模拟：按当前 enrollment 派生并持久化真实意图文件，再执行共用
        /// prepare 的 plan 写入步骤（无 session、无绑定）——崩溃于 plan 已写、
        /// session 未写之间，不伪造绑定。
        fn seed_intent_and_plan_without_session(&self, expected_plan_id: &str) {
            let enrollment = IssueAutomationStore::new(self.paths.clone())
                .get(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .unwrap();
            let target = enrollment
                .target
                .clone()
                .expect("fixture enrollment declares a target");
            let intent = PreparedPlanIntent::from_enrollment(&enrollment, &target);
            assert_eq!(intent.plan_id, expected_plan_id);
            write_json(
                &self
                    .paths
                    .issue_root(PROJECT_ID, ISSUE_ID)
                    .join("automation-plan-intent.json"),
                &intent,
            )
            .unwrap();
            LifecycleStore::new(self.paths.clone())
                .ensure_issue_work_item_plan_with_identity(CreateIssueWorkItemPlanInput {
                    id: Some(intent.plan_id.clone()),
                    project_id: PROJECT_ID.to_string(),
                    issue_id: ISSUE_ID.to_string(),
                    source_story_spec_ids: intent
                        .source
                        .stories
                        .iter()
                        .map(|reference| reference.id.clone())
                        .collect(),
                    source_design_spec_ids: intent
                        .source
                        .designs
                        .iter()
                        .map(|reference| reference.id.clone())
                        .collect(),
                    options: intent.options.plan_options.clone(),
                    status: IssueWorkItemPlanStatus::Draft,
                    work_item_ids: Vec::new(),
                    repository_profile_ref: None,
                    verification_plan_ids: Vec::new(),
                    dependency_graph: Vec::new(),
                    created_from_provider_run: None,
                    validator_findings: Vec::new(),
                })
                .unwrap();
        }

        /// 以重建的 state/store 执行后台补偿，返回补偿后的 enrollment 投影。
        async fn ensure_enrolled_plan(&self) -> ApiResult<IssueAutomationEnrollment> {
            let root = self._root.path();
            let state =
                WebAppState::new(root.to_path_buf(), WebRuntime::new_fake(root.to_path_buf()));
            let enrollment = IssueAutomationStore::new(self.paths.clone())
                .get(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .unwrap();
            ensure_enrolled_plan(&state, &enrollment).await?;
            IssueAutomationStore::new(self.paths.clone())
                .get(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .ok_or_else(|| {
                    crate::web::error::ApiError::runtime(
                        "automation_enrollment_not_found",
                        "enrollment vanished after compensation",
                        serde_json::json!({}),
                    )
                })
        }

        fn plans(&self) -> Vec<IssueWorkItemPlan> {
            self.lifecycle
                .list_issue_work_item_plans(PROJECT_ID, ISSUE_ID)
                .unwrap()
        }

        fn sessions(&self) -> Vec<WorkspaceSessionRecord> {
            self.lifecycle
                .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
                .unwrap()
        }
    }

    #[tokio::test]
    async fn automation_prepare_recovers_plan_without_session_or_rebinding() {
        let fixture = seed_fixture(1, true);
        let enrolled = response_json(
            put_enrollment(&fixture.router(), enrollment_body_with_target(&fixture)).await,
        )
        .await;
        let intent_id = enrolled["prepare_intent_id"].as_str().unwrap();
        let expected_plan_id = format!("issue_work_item_plan_auto_{intent_id}");
        fixture.seed_intent_and_plan_without_session(&expected_plan_id);

        // 重建进程后的两次补偿：绑定同一 plan/session，plan 文件不被覆盖。
        let recovered = fixture.ensure_enrolled_plan().await.unwrap();
        let again = fixture.ensure_enrolled_plan().await.unwrap();
        assert_eq!(recovered.plan_id, again.plan_id);
        assert_eq!(recovered.session_id, again.session_id);
        assert_eq!(
            recovered.plan_id.as_deref(),
            Some(expected_plan_id.as_str())
        );
        assert_eq!(
            recovered.session_id.as_deref(),
            Some(format!("workspace_session_auto_{intent_id}").as_str())
        );
        assert_eq!(fixture.plans().len(), 1);
        assert_eq!(fixture.sessions().len(), 1);
        assert_eq!(fixture.sessions()[0].run_policy, RunPolicy::Interactive);
    }

    /// Disable → 异 payload 重开：旧 intent 冻结快照与新授权不匹配 → 补偿
    /// fail-closed，原 plan 内容/创建时间不变。
    #[tokio::test]
    async fn automation_prepare_fails_closed_on_divergent_reopen() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();
        put_enrollment(&app, enrollment_body_with_target(&fixture)).await;
        fixture.ensure_enrolled_plan().await.unwrap();
        let plan_before = fixture.plans().remove(0);

        let revision = IssueAutomationStore::new(fixture.paths.clone())
            .get(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .unwrap()
            .policy_revision;
        let disable = serde_json::json!({
            "expected_revision": revision,
            "command": {"type": "disable"}
        });
        let response = put_enrollment(&app, disable).await;
        assert_eq!(response.status(), StatusCode::OK);
        let mut divergent = enrollment_body_with_target(&fixture);
        divergent["command"]["options"]["review_rounds"] = serde_json::json!(2);
        divergent["expected_revision"] = serde_json::json!(revision + 1);
        let response = put_enrollment(&app, divergent).await;
        assert_eq!(response.status(), StatusCode::OK);

        let error = fixture.ensure_enrolled_plan().await.unwrap_err();
        assert_eq!(error.code, "automation_enrollment_conflict");
        let plans = fixture.plans();
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].id, plan_before.id);
        assert_eq!(plans[0].created_at, plan_before.created_at);
        assert_eq!(plans[0].options, plan_before.options);
    }
}
