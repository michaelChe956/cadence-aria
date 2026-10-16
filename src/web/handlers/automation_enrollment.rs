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

// ─── 批 2 T3：统一 plan 终止服务（REQ-WIGA-09/11、REQ-WIGA-08/12）───

/// attempt 的托管身份（统一终止路由用）：enrollment 显式绑定该 attempt
/// 的权威 plan（`work_item_group_id`）即托管——enabled 或 terminated 均算
/// （terminated 仍 managed）；manual/无绑定/异 plan 一律 `None`。
pub(crate) struct AutomationPlanTerminationIdentity {
    pub enrollment_id: String,
    pub policy_revision: u64,
    pub plan_id: Option<String>,
    pub session_id: Option<String>,
    pub binding_version: Option<u64>,
    pub terminated_at: Option<String>,
    pub enabled: bool,
}

/// 统一终止路由判定（REST abort / gate-responses / WS Abort/门动作 Abort
/// 共用）：读当前 enrollment，按 attempt 的权威 plan＋冻结绑定识别托管。
/// 只读；损坏 JSON fail-closed 上抛（不当未托管吞错）。
pub(crate) fn automation_identity_for_attempt(
    paths: &crate::product::app_paths::ProductAppPaths,
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
) -> Result<Option<AutomationPlanTerminationIdentity>, ProductStoreError> {
    let Some(plan_id) = attempt.work_item_group_id.clone() else {
        return Ok(None);
    };
    let enrollment = IssueAutomationStore::new(paths.clone())
        .get(&attempt.project_id, &attempt.issue_id)?;
    Ok(enrollment
        .filter(|enrollment| {
            // k3 波5 F1 修复：托管判据＝精确绑定 plan 且 enabled 或已终止
        //（terminated 仍 managed）；仅 Disable（enabled=false 且未终止）
        // 不是托管——单 attempt abort/门动作保持原语义，与其余投影的
        // owner=client 同判据（N0 守卫同口径）。
            enrollment.plan_id.as_deref() == Some(plan_id.as_str())
                && (enrollment.enabled || enrollment.terminated_at.is_some())
        })
        .map(|enrollment| AutomationPlanTerminationIdentity {
            enrollment_id: enrollment.enrollment_id.clone(),
            policy_revision: enrollment.policy_revision,
            plan_id: enrollment.plan_id.clone(),
            session_id: enrollment.session_id.clone(),
            binding_version: enrollment
                .binding_history
                .as_ref()
                .map(|history| history.current.binding_version),
            terminated_at: enrollment.terminated_at.clone(),
            enabled: enrollment.enabled,
        }))
}

/// k3 波5 F1（P2）：coding WS 会话快照的管理投影——精确绑定 plan 即返回
/// ownership（与 REST `ownership_for_ids` 同判据：owner=Server iff
 /// enabled||terminated，仅 Disable 为 client；未绑定返回 None 保持旧帧）。
pub(crate) fn automation_ownership_for_attempt(
    paths: &crate::product::app_paths::ProductAppPaths,
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
) -> Result<
    Option<crate::product::models::automation::AutomationOwnership>,
    ProductStoreError,
> {
    let Some(plan_id) = attempt.work_item_group_id.clone() else {
        return Ok(None);
    };
    let enrollment = IssueAutomationStore::new(paths.clone())
        .get(&attempt.project_id, &attempt.issue_id)?;
    Ok(enrollment
        .filter(|enrollment| enrollment.plan_id.as_deref() == Some(plan_id.as_str()))
        .map(|enrollment| crate::product::models::automation::AutomationOwnership {
            owner: if enrollment.enabled || enrollment.terminated_at.is_some() {
                crate::product::models::automation::AutomationOwner::Server
            } else {
                crate::product::models::automation::AutomationOwner::Client
            },
            enrollment_id: Some(enrollment.enrollment_id),
            policy_revision: Some(enrollment.policy_revision),
            enabled: enrollment.enabled,
            plan_id: enrollment.plan_id,
            terminated_at: enrollment.terminated_at,
        }))
}

fn termination_identity_details(identity: &AutomationPlanTerminationIdentity) -> serde_json::Value {
    json!({
        "confirm_required": true,
        "plan_id": identity.plan_id,
        "session_id": identity.session_id,
        "enrollment_id": identity.enrollment_id,
        "policy_revision": identity.policy_revision,
        "binding_version": identity.binding_version,
        "terminated_at": identity.terminated_at,
    })
}

/// 统一二次确认要求（REST 面）：未确认零副作用，只返回确认要求与
/// plan/enrollment/binding 身份（客户端打开同一二次确认面）。
pub(crate) fn automation_plan_termination_required_error(
    identity: &AutomationPlanTerminationIdentity,
) -> ApiError {
    ApiError::validation_with_details(
        "automation_plan_termination_required",
        "automation plan termination requires unified confirmation",
        termination_identity_details(identity),
    )
}

/// WS 面同语义：`CodingProtocolError.code=automation_plan_termination_required`，
/// message 为 JSON 身份载荷（wire 契约：前端按 code 打开统一二次确认面，
/// 确认后经统一终止 REST 提交，不在 WS 侧产生任何终止副作用）。
pub(crate) fn automation_plan_termination_required_message(
    identity: &AutomationPlanTerminationIdentity,
) -> String {
    serde_json::to_string(&termination_identity_details(identity))
        .unwrap_or_else(|_| "{}".to_string())
}

/// 统一终止 REST（块 3 §4）：①未带 `confirmed:true` 零副作用返回
/// `automation_plan_termination_required`＋身份；②确认后 T1 store CAS
/// `terminate_plan`（幂等可重入，同 command 同 payload 重放首次 durable
/// 结果）→ T2 `terminate_plan_engine_side`（失效 run/取消 session run/
/// CAS Terminated/逐 attempt 干净退出＋专用收口）→ ③批量清当前绑定 plan
/// 三类旧事实，清理失败写 `automation_failed(stage=termination)`（重清
/// 成功即消失）；cleanup_required 仍不可继续，同 command 重放只重做收口。
pub async fn post_automation_enrollment_terminate(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Json(request): Json<crate::web::types::AutomationEnrollmentTerminateRequest>,
) -> ApiResult<Json<crate::web::types::AutomationEnrollmentTerminateResponse>> {
    use crate::web::types::{
        AutomationEnrollmentTerminateAttemptResult, AutomationEnrollmentTerminateResponse,
    };

    validate_request_ids(&project_id, &issue_id)?;
    ensure_issue_exists(&state, &project_id, &issue_id)?;
    let paths = product_app_paths(&state);
    let store = IssueAutomationStore::new(paths.clone());
    let enrollment = store
        .get(&project_id, &issue_id)
        .map_err(product_store_api_error)?
        .ok_or_else(enrollment_not_found)?;
    let identity = AutomationPlanTerminationIdentity {
        enrollment_id: enrollment.enrollment_id.clone(),
        policy_revision: enrollment.policy_revision,
        plan_id: enrollment.plan_id.clone(),
        session_id: enrollment.session_id.clone(),
        binding_version: enrollment
            .binding_history
            .as_ref()
            .map(|history| history.current.binding_version),
        terminated_at: enrollment.terminated_at.clone(),
        enabled: enrollment.enabled,
    };
    if !request.confirmed {
        return Err(automation_plan_termination_required_error(&identity));
    }
    if request.expected_enrollment_id != enrollment.enrollment_id {
        return Err(enrollment_conflict(
            Some(enrollment.policy_revision),
            "termination enrollment identity mismatch",
        ));
    }
    // 提示性绑定互证：请求携带的 expected_binding 必须与当前代全等。
    if let Some(expected_binding) = request.expected_binding.as_ref()
        && let Some(history) = enrollment.binding_history.as_ref()
        && expected_binding != &history.current
    {
        return Err(enrollment_conflict(
            Some(enrollment.policy_revision),
            "termination binding identity mismatch",
        ));
    }

    // ①T1 store CAS：terminated_at＋enabled=false 同一原子写（命令账本
    // 幂等/冲突；terminated 异命令 → Conflict，未完成清理同 command 重放）。
    let store_request = crate::product::models::automation::EnrollmentTerminationRequest {
        command_id: request.command_id.clone(),
        expected_enrollment_id: request.expected_enrollment_id.clone(),
        expected_policy_revision: request.expected_policy_revision,
        reason: request
            .reason
            .clone()
            .unwrap_or_else(|| "automation plan termination confirmed by user".to_string()),
    };
    let result = store
        .terminate_plan(&project_id, &issue_id, store_request)
        .map_err(enrollment_api_error)?;
    let terminated_enrollment = result.enrollment.clone();

    // ②T2 引擎侧收口（专用取消契约；仅调用不并写其文件）。
    let mut cleanup_required = false;
    let mut results: Vec<AutomationEnrollmentTerminateAttemptResult> = Vec::new();
    if let Some(plan_id) = terminated_enrollment.plan_id.clone() {
        let engine_outcome = crate::web::workspace_session::terminate_plan_engine_side(
            paths.clone(),
            state.workspace_sessions.clone(),
            state.coding_runs.clone(),
            &project_id,
            &issue_id,
            &plan_id,
            terminated_enrollment.session_id.as_deref(),
        )
        .await;
        if engine_outcome.session_error.is_some() {
            cleanup_required = true;
            results.push(AutomationEnrollmentTerminateAttemptResult {
                attempt_id: plan_id.clone(),
                status: "cleanup_failed".to_string(),
                reason_code: Some("session_termination_failed".to_string()),
                message: engine_outcome.session_error.clone(),
            });
        }
        results.extend(engine_outcome.attempts.into_iter().map(|attempt| {
            if attempt.status == "cleanup_failed" {
                cleanup_required = true;
            }
            AutomationEnrollmentTerminateAttemptResult {
                attempt_id: attempt.attempt_id,
                status: attempt.status.to_string(),
                reason_code: attempt.reason_code,
                message: attempt.message,
            }
        }));
    }

    // ③批量清当前绑定 plan 三类旧事实（plan_question/automation_failed/
    // review_limit）；清理失败写 automation_failed(stage=termination)，
    // 重清成功即消失（outcome 回到 terminated）。
    let fresh = store
        .get(&project_id, &issue_id)
        .map_err(product_store_api_error)?
        .unwrap_or(terminated_enrollment.clone());
    if let Some(binding) = fresh
        .binding_history
        .as_ref()
        .map(|history| history.current.clone())
    {
        let stale_keys: Vec<String> = fresh
            .attention_facts
            .iter()
            .filter(|fact| {
                fact.plan_id.as_deref() == Some(binding.plan_id.as_str())
                    || (fact.plan_id.is_none()
                        && fact.session_id.as_deref() == Some(binding.session_id.as_str()))
            })
            .map(|fact| fact.key.clone())
            .collect();
        let mut clear_errors: Vec<String> = Vec::new();
        for key in &stale_keys {
            if let Err(error) = store.resolve_automation_attention_fact(
                &project_id,
                &issue_id,
                &binding,
                key,
            ) {
                clear_errors.push(format!("{key}: {error}"));
            }
        }
        if cleanup_required || !clear_errors.is_empty() {
            let termination_fact = crate::product::models::automation::AutomationAttentionFact {
                key: format!(
                    "{}:{}:termination_cleanup_failed",
                    binding.plan_id, request.command_id
                ),
                kind: crate::product::models::automation::AutomationAttentionKind::AutomationFailed,
                stage: crate::product::models::automation::AutomationAttentionStage::Termination,
                plan_id: Some(binding.plan_id.clone()),
                session_id: fresh.session_id.clone(),
                attempt_id: None,
                reason_code: "termination_cleanup_failed".to_string(),
                occurred_at: chrono::Utc::now().to_rfc3339(),
                choice_id: None,
                gate_id: None,
            };
            if let Err(error) = store.upsert_automation_attention_fact(
                &project_id,
                &issue_id,
                &binding,
                termination_fact,
            ) {
                clear_errors.push(format!("termination fact upsert failed: {error}"));
            }
        }
        if !clear_errors.is_empty() {
            cleanup_required = true;
            results.push(AutomationEnrollmentTerminateAttemptResult {
                attempt_id: fresh
                    .plan_id
                    .clone()
                    .unwrap_or_else(|| "attention-facts".to_string()),
                status: "cleanup_failed".to_string(),
                reason_code: Some("attention_fact_cleanup_failed".to_string()),
                message: Some(clear_errors.join("; ")),
            });
        }
    }

    Ok(Json(AutomationEnrollmentTerminateResponse {
        command_id: result.command_id,
        state: result.state,
        enrollment_id: result.enrollment.enrollment_id,
        plan_id: result.enrollment.plan_id,
        terminated_at: result.terminated_at,
        outcome: if cleanup_required {
            "cleanup_required".to_string()
        } else {
            "terminated".to_string()
        },
        results,
    }))
}

/// 批 2 T3（块 3 §4）：continue-chain——advance/coding_start 停点的第三种
/// 显式继续。advance 沿 R1 冻结 policy 复用引擎 retry-initialization；
/// coding_start 仅重放原 Claimed/RunnerRegistered claim（越 barrier 不清
/// claim，走既有 RecoverCoding/RestartCoding 恢复并核 auto）。成功 resolve
/// 原事实＋wake（auto 保留）；NeedsHuman/失败保留原事实。
pub async fn post_automation_enrollment_continue_chain(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Json(request): Json<crate::web::types::AutomationContinueChainRequest>,
) -> ApiResult<Json<crate::web::types::AutomationContinueChainResponse>> {
    use crate::product::models::automation::{AutomationAttentionKind, AutomationAttentionStage};
    use crate::web::types::AutomationContinueChainResponse;

    validate_request_ids(&project_id, &issue_id)?;
    for id in [&request.command_id, &request.expected_stop_key] {
        validate_relative_id(id).map_err(|error| {
            ApiError::validation(
                "automation_continue_chain_invalid_id",
                format!("invalid continue-chain id: {error}"),
            )
        })?;
    }
    ensure_issue_exists(&state, &project_id, &issue_id)?;
    let paths = product_app_paths(&state);
    let store = IssueAutomationStore::new(paths.clone());
    let enrollment = store
        .get(&project_id, &issue_id)
        .map_err(product_store_api_error)?
        .ok_or_else(enrollment_not_found)?;
    if enrollment.terminated_at.is_some() {
        return Err(enrollment_conflict(
            Some(enrollment.policy_revision),
            "terminated enrollment cannot continue",
        ));
    }
    if !enrollment.enabled {
        return Err(enrollment_conflict(
            Some(enrollment.policy_revision),
            "automation enrollment is disabled",
        ));
    }
    let binding = enrollment
        .binding_history
        .as_ref()
        .map(|history| history.current.clone())
        .ok_or_else(|| {
            invalid_scope("enrollment has no versioned binding; cannot continue the chain")
        })?;
    if binding != request.expected_binding {
        return Err(enrollment_conflict(
            Some(enrollment.policy_revision),
            "continue-chain binding expired",
        ));
    }
    let Some(fact) = enrollment
        .attention_facts
        .iter()
        .find(|fact| fact.key == request.expected_stop_key)
        .cloned()
    else {
        return Err(ApiError::runtime(
            "automation_continue_chain_stop_key_not_found",
            "expected_stop_key does not match an open attention fact",
            json!({ "stop_key": request.expected_stop_key }),
        ));
    };
    if fact.kind != AutomationAttentionKind::AutomationFailed
        || !matches!(
            fact.stage,
            AutomationAttentionStage::Advance | AutomationAttentionStage::CodingStart
        )
        || fact.plan_id.as_deref() != Some(binding.plan_id.as_str())
    {
        return Err(ApiError::runtime(
            "automation_continue_chain_stop_key_not_replayable",
            "stop key is not an advance/coding_start drive failure for the bound plan",
            json!({ "stop_key": request.expected_stop_key, "stage": fact.stage }),
        ));
    }

    let resolve_and_wake = |store: IssueAutomationStore| {
        let outcome = store.resolve_automation_attention_fact(
            &project_id,
            &issue_id,
            &binding,
            &request.expected_stop_key,
        );
        let _ = state.autopilot_wake.send(true);
        outcome
    };

    match fact.stage {
        AutomationAttentionStage::Advance => {
            let advance_store = crate::product::advance_store::AdvanceStore::new(paths.clone());
            let Some(record) = advance_store
                .get_advance_for_plan(&project_id, &issue_id, &binding.plan_id)
                .map_err(product_store_api_error)?
            else {
                return Err(ApiError::runtime(
                    "automation_continue_chain_stop_key_not_replayable",
                    "no advance record for the bound plan",
                    json!({}),
                ));
            };
            if record.status != crate::product::advance_store::AdvanceStatus::Failed {
                // 停点事实已过期（advance 已非 Failed）：幂等解除并唤醒。
                resolve_and_wake(store).map_err(enrollment_api_error)?;
                return Ok(Json(AutomationContinueChainResponse {
                    command_id: request.command_id,
                    state: crate::product::models::automation::OperationState::Replayed,
                    stop_key: request.expected_stop_key,
                    stage: "advance".to_string(),
                    resolved: true,
                    detail: None,
                }));
            }
            let journal = advance_store
                .get_advance_initialization(&record)
                .map_err(product_store_api_error)?
                .ok_or_else(|| {
                    ApiError::runtime(
                        "automation_continue_chain_stop_key_not_replayable",
                        "failed advance has no initialization journal",
                        json!({}),
                    )
                })?;
            let retry_request = crate::product::models::automation::RetryInitializationRequest {
                command_id: request.command_id.clone(),
                expected_binding: binding.clone(),
                expected_attempt_id: journal.attempt_id.clone(),
                expected_checkpoint: journal.phase,
                confirm_unknown_side_effect: request.confirm_unknown_side_effect,
            };
            // 与 retry-initialization REST 同一引擎链（借用 R1 冻结 policy：
            // record.start_run_policy 首次冻结，不猜 Manual、不覆盖）。
            let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(paths.clone());
            let session_record = lifecycle
                .list_workspace_sessions(&project_id, &issue_id)
                .map_err(product_store_api_error)?
                .into_iter()
                .find(|record| {
                    record.workspace_type == crate::product::models::WorkspaceType::WorkItemPlan
                        && record.entity_id == binding.plan_id
                })
                .ok_or_else(|| {
                    ApiError::runtime(
                        "automation_continue_chain_session_not_found",
                        "no workspace session bound to the plan",
                        json!({}),
                    )
                })?;
            let admission =
                crate::product::workspace_engine::AdvancePlanAdmission::acquire(
                    &paths,
                    &project_id,
                    &issue_id,
                    &binding.plan_id,
                )
                .map_err(|error| {
                    ApiError::runtime(
                        "automation_continue_chain_admission_failed",
                        format!("continue-chain admission rejected: {error:?}"),
                        json!({}),
                    )
                })?;
            let retry = match state.workspace_sessions.peek(&session_record.id).await {
                Some(manager) => {
                    let engine_arc = manager.engine();
                    let mut engine = engine_arc.lock().await;
                    engine
                        .retry_initialization_with_lease(&retry_request, &admission)
                        .await
                }
                None => {
                    let full_record = lifecycle
                        .get_workspace_session(&session_record.id)
                        .map_err(product_store_api_error)?;
                    let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
                    let mut engine = crate::product::workspace_engine::WorkspaceEngine::new_persistent(
                        std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
                            state.workspace_root.join("advance-engine-checkpoints"),
                        )),
                        lifecycle,
                        event_tx,
                        crate::product::workspace_engine::WorkspaceSession::from_record(full_record),
                    );
                    engine
                        .retry_initialization_with_lease(&retry_request, &admission)
                        .await
                }
            }
            .map_err(|error| {
                ApiError::runtime(
                    "automation_continue_chain_rejected",
                    error,
                    json!({}),
                )
            })?;
            let succeeded = matches!(
                retry.state,
                crate::product::models::automation::OperationState::Accepted
                    | crate::product::models::automation::OperationState::Replayed
            ) && retry
                .outcome
                .as_ref()
                .is_some_and(|outcome| {
                    matches!(
                        outcome,
                        crate::product::advance_store::AdvanceOutcome::Completed { .. }
                            | crate::product::advance_store::AdvanceOutcome::Replayed { .. }
                    )
                });
            if succeeded {
                resolve_and_wake(store).map_err(enrollment_api_error)?;
            }
            Ok(Json(AutomationContinueChainResponse {
                command_id: retry.command_id,
                state: retry.state,
                stop_key: request.expected_stop_key,
                stage: "advance".to_string(),
                resolved: succeeded,
                detail: if succeeded {
                    None
                } else {
                    Some("advance retry stopped at needs_human; fact kept".to_string())
                },
            }))
        }
        AutomationAttentionStage::CodingStart => {
            let Some(attempt_id) = fact.attempt_id.clone() else {
                return Err(ApiError::runtime(
                    "automation_continue_chain_stop_key_not_replayable",
                    "coding_start fact carries no attempt id",
                    json!({}),
                ));
            };
            let coding_store =
                crate::product::coding_attempt_store::CodingAttemptStore::new(paths.clone());
            let attempt = coding_store
                .get_attempt(&project_id, &issue_id, &attempt_id)
                .map_err(product_store_api_error)?;
            let Some(claim) = attempt.start_claim.clone() else {
                return Err(ApiError::runtime(
                    "automation_continue_chain_stop_key_not_replayable",
                    "attempt has no start claim; use the existing recovery paths",
                    json!({}),
                ));
            };
            if !matches!(
                claim.phase,
                crate::product::coding_models::CodingStartPhase::Claimed
                    | crate::product::coding_models::CodingStartPhase::RunnerRegistered
            ) {
                // 越屏障（ProviderMayHaveStarted/NeedsHuman）不清 claim——
                // 走既有 RecoverCoding/RestartCoding 恢复（服务端核 auto）。
                return Err(ApiError::runtime(
                    "automation_continue_chain_stop_key_not_replayable",
                    "start claim passed the replay barrier; use the existing recovery paths",
                    json!({ "claim_phase": format!("{:?}", claim.phase) }),
                ));
            }
            let outcome = crate::web::coding_start::start_coding_once(
                &state,
                &project_id,
                &issue_id,
                crate::web::coding_start::StartCodingCommand {
                    attempt_id: attempt.id.clone(),
                    command_id: claim.command_id.clone(),
                    origin: claim.origin.clone(),
                },
            )
            .await;
            match outcome {
                Ok(
                    crate::web::coding_start::StartCodingOutcome::Started { .. }
                    | crate::web::coding_start::StartCodingOutcome::AlreadyStarted { .. },
                ) => {
                    resolve_and_wake(store).map_err(enrollment_api_error)?;
                    Ok(Json(AutomationContinueChainResponse {
                        command_id: request.command_id,
                        state: crate::product::models::automation::OperationState::Accepted,
                        stop_key: request.expected_stop_key,
                        stage: "coding_start".to_string(),
                        resolved: true,
                        detail: None,
                    }))
                }
                Ok(crate::web::coding_start::StartCodingOutcome::NeedsHuman {
                    attempt_id,
                    reason,
                }) => Ok(Json(AutomationContinueChainResponse {
                    command_id: request.command_id,
                    state: crate::product::models::automation::OperationState::NeedsHuman,
                    stop_key: request.expected_stop_key,
                    stage: "coding_start".to_string(),
                    resolved: false,
                    detail: Some(format!("{attempt_id}: {reason}")),
                })),
                Err(error) => Err(ApiError::runtime(
                    "automation_continue_chain_start_failed",
                    format!("coding start replay failed: {}", error.message()),
                    json!({ "code": error.code() }),
                )),
            }
        }
        AutomationAttentionStage::Plan | AutomationAttentionStage::Termination => {
            // 上方 stage 过滤已排除； unreachable 分支保持穷尽。
            unreachable!("stage filtered above")
        }
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

    // ─── 批 2 T3：统一终止 REST（REQ-WIGA-09/11、REQ-WIGA-08/12）───

    async fn post_terminate(
        app: &axum::Router,
        body: &serde_json::Value,
    ) -> axum::http::Response<axum::body::Body> {
        use axum::body::Body;
        use tower::ServiceExt;

        app.clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/automation-enrollment/terminate"
                    ))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    /// T3 红测①：未带 confirmed:true → 428 `automation_plan_termination_required`
    ///（details 含 confirm_required/plan/enrollment 身份），零副作用。
    #[tokio::test]
    async fn terminate_rest_requires_confirmation_with_zero_side_effects() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();
        let enable = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(enable.status(), StatusCode::OK);
        let enrollment = response_json(enable).await;
        let revision = enrollment["policy_revision"].as_u64().unwrap();
        let (plan_id, session_id) = create_plan_and_session(&fixture, RunPolicy::Interactive);
        let bound = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": revision,
                "plan_id": plan_id,
                "session_id": session_id,
            }),
        )
        .await;
        assert_eq!(bound.status(), StatusCode::OK);

        let response = post_terminate(
            &app,
            &serde_json::json!({
                "command_id": "cmd-t3-terminate-1",
                "expected_enrollment_id": enrollment["enrollment_id"],
                "expected_policy_revision": revision + 1,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::PRECONDITION_REQUIRED);
        let payload = response_json(response).await;
        assert_eq!(payload["code"], "automation_plan_termination_required");
        assert_eq!(payload["details"]["confirm_required"], true);
        assert_eq!(payload["details"]["plan_id"], plan_id.as_str());
        assert_eq!(
            payload["details"]["enrollment_id"],
            enrollment["enrollment_id"]
        );

        // 零副作用：enrollment 未终止未禁用，session 仍 Open。
        let current = IssueAutomationStore::new(fixture.paths.clone())
            .get(PROJECT_ID, ISSUE_ID)
            .unwrap()
            .unwrap();
        assert!(current.enabled);
        assert!(current.terminated_at.is_none());
        assert_eq!(
            crate::product::lifecycle_store::LifecycleStore::new(fixture.paths.clone())
                .get_workspace_session(&session_id)
                .unwrap()
                .status,
            crate::product::models::WorkspaceSessionStatus::Open,
        );
    }

    /// T3 红测①②⑤⑥⑧：确认后整 plan 终止（session Terminated、三类旧事实
    /// 清空）、同 command 重放幂等（terminated_at 冻结、不复活）、terminated
    /// 仍 managed（同源管理投影）。
    #[tokio::test]
    async fn terminate_rest_confirmed_terminates_and_replays_idempotently() {
        use crate::product::models::automation::{
            AutomationAttentionFact, AutomationAttentionKind, AutomationAttentionStage,
        };
        use crate::product::models::WorkspaceSessionStatus;

        let fixture = seed_fixture(1, true);
        let app = fixture.router();
        let enable = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        let enrollment = response_json(enable).await;
        let revision = enrollment["policy_revision"].as_u64().unwrap();
        let (plan_id, session_id) = create_plan_and_session(&fixture, RunPolicy::Interactive);
        let bound = response_json(
            post_binding(
                &app,
                serde_json::json!({
                    "expected_revision": revision,
                    "plan_id": plan_id,
                    "session_id": session_id,
                }),
            )
            .await,
        )
        .await;
        let bound_revision = bound["policy_revision"].as_u64().unwrap();

        // 播种当前绑定 plan 的三类旧事实之一（plan_question）。
        let store = IssueAutomationStore::new(fixture.paths.clone());
        let durable = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
        let binding = durable.binding_history.as_ref().unwrap().current.clone();
        store
            .upsert_automation_attention_fact(
                PROJECT_ID,
                ISSUE_ID,
                &binding,
                AutomationAttentionFact {
                    key: format!("{plan_id}:cmd-seed:plan_question"),
                    kind: AutomationAttentionKind::PlanQuestion,
                    stage: AutomationAttentionStage::Plan,
                    plan_id: Some(plan_id.clone()),
                    session_id: Some(session_id.clone()),
                    attempt_id: None,
                    reason_code: "test".to_string(),
                    occurred_at: "2026-10-16T00:00:00Z".to_string(),
                    choice_id: None,
                    gate_id: None,
                },
            )
            .unwrap();

        let body = serde_json::json!({
            "command_id": "cmd-t3-terminate-2",
            "expected_enrollment_id": enrollment["enrollment_id"],
            "expected_policy_revision": bound_revision,
            "confirmed": true,
        });
        let response = post_terminate(&app, &body).await;
        assert_eq!(response.status(), StatusCode::OK);
        let first = response_json(response).await;
        assert_eq!(first["outcome"], "terminated");
        assert_eq!(first["state"], "accepted");
        assert_eq!(first["plan_id"], plan_id.as_str());
        let terminated_at = first["terminated_at"].as_str().unwrap().to_string();

        // session Terminated；enrollment 不可恢复终止；三类旧事实清空。
        assert_eq!(
            crate::product::lifecycle_store::LifecycleStore::new(fixture.paths.clone())
                .get_workspace_session(&session_id)
                .unwrap()
                .status,
            WorkspaceSessionStatus::Terminated,
        );
        let durable = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
        assert!(!durable.enabled);
        assert_eq!(durable.terminated_at.as_deref(), Some(terminated_at.as_str()));
        assert!(
            durable.attention_facts.is_empty(),
            "termination must clear the bound plan's stale attention facts"
        );

        // terminated 仍 managed（同源管理投影；REST/WS 同一口径）。
        let ownership = store
            .ownership_for_ids(PROJECT_ID, ISSUE_ID, &session_id)
            .unwrap();
        assert_eq!(
            ownership.owner,
            crate::product::models::automation::AutomationOwner::Server
        );
        assert_eq!(ownership.terminated_at.as_deref(), Some(terminated_at.as_str()));

        // 同 command 同 payload 重放：Replayed、terminated_at 冻结、幂等。
        let replay = post_terminate(&app, &body).await;
        assert_eq!(replay.status(), StatusCode::OK);
        let replayed = response_json(replay).await;
        assert_eq!(replayed["state"], "replayed");
        assert_eq!(replayed["terminated_at"], terminated_at.as_str());

        // 重启后不复活：新进程替身 reconcile → NoEnrollment（terminated）。
        let state = crate::web::state::WebAppState::new(
            fixture._root.path().to_path_buf(),
            crate::web::runtime::WebRuntime::new_fake(fixture._root.path().to_path_buf()),
        );
        let worker = crate::web::autopilot_orchestrator::AutopilotOrchestrator::new(
            state,
            Default::default(),
        );
        let outcome = worker
            .reconcile(
                &crate::web::state::WebAppState::new(
                    fixture._root.path().to_path_buf(),
                    crate::web::runtime::WebRuntime::new_fake(fixture._root.path().to_path_buf()),
                ),
                PROJECT_ID,
                ISSUE_ID,
            )
            .await
            .unwrap();
        assert_eq!(outcome, crate::web::autopilot_orchestrator::ReconcileOutcome::NoEnrollment);
    }

    /// T3 红测②（C2 旧 abort REST 路由）：自动绑定 plan 上的旧 abort 入口
    /// 统一返回终止确认要求（零副作用）；manual/无绑定不走 428。
    #[tokio::test]
    async fn abort_rest_routes_managed_attempt_to_unified_termination() {
        use tower::ServiceExt;

        use crate::product::coding_attempt_store::{
            CodingAttemptStore, CreateCodingAttemptInput,
        };
        use crate::web::workspace_ws_types::ProviderConfigSnapshot;

        let fixture = seed_fixture(1, true);
        let app = fixture.router();
        let enable = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        let enrollment = response_json(enable).await;
        let revision = enrollment["policy_revision"].as_u64().unwrap();
        let (plan_id, session_id) = create_plan_and_session(&fixture, RunPolicy::Interactive);
        let bound = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": revision,
                "plan_id": plan_id,
                "session_id": session_id,
            }),
        )
        .await;
        assert_eq!(bound.status(), StatusCode::OK);

        let coding_store = CodingAttemptStore::new(fixture.paths.clone());
        let attempt = coding_store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: "work_item_0001".to_string(),
                base_branch: "HEAD".to_string(),
                branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
                worktree_path: None,
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: crate::product::models::provider::ProviderName::Fake,
                    reviewer: None,
                    review_rounds: 0,
                    permission_modes: Default::default(),
                },
                target_snapshot: None,
                max_auto_rework: 1,
            })
            .unwrap();
        // 独立 manual 对照 attempt（不同 work_item，无 group 绑定）。
        let manual_attempt = coding_store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: "work_item_0002".to_string(),
                base_branch: "HEAD".to_string(),
                branch_name: "aria/work-items/work_item_0002/attempt-1".to_string(),
                worktree_path: None,
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: crate::product::models::provider::ProviderName::Fake,
                    reviewer: None,
                    review_rounds: 0,
                    permission_modes: Default::default(),
                },
                target_snapshot: None,
                max_auto_rework: 1,
            })
            .unwrap();
        let mut managed = attempt.clone();
        managed.work_item_group_id = Some(plan_id.clone());
        coding_store.write_coding_attempt_for_test(&managed).unwrap();

        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/coding-attempts/{}/abort",
                        managed.id
                    ))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PRECONDITION_REQUIRED);
        let payload = response_json(response).await;
        assert_eq!(payload["code"], "automation_plan_termination_required");
        assert_eq!(payload["details"]["plan_id"], plan_id.as_str());
        // 零副作用：attempt 状态未被单 attempt abort 改写。
        assert_eq!(
            coding_store
                .get_attempt(PROJECT_ID, ISSUE_ID, &managed.id)
                .unwrap()
                .status,
            crate::product::coding_models::CodingAttemptStatus::Created,
        );

        // manual 对照：无绑定 attempt 不返回 428（原语义错误面或成功）。
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/coding-attempts/{}/abort",
                        manual_attempt.id
                    ))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let manual_payload = response_json(response).await;
        assert_ne!(
            manual_payload["code"],
            "automation_plan_termination_required",
            "manual attempt abort must keep the original semantics"
        );
    }

    /// k3 波5 F1（P2）：Disable 态（enabled=false 且未终止）不是托管——
    /// coding WS 管理投影 owner=client（与 REST ownership 同判据）、abort
    /// REST 不路由统一终止；Terminate 后 owner 仍 server（terminated 仍
    /// managed）。
    #[tokio::test]
    async fn disabled_enrollment_projects_client_owner_and_keeps_manual_abort() {
        use crate::product::coding_attempt_store::{
            CodingAttemptStore, CreateCodingAttemptInput,
        };
        use crate::product::models::automation::AutomationOwner;
        use crate::web::coding_ws_handler::build_coding_session_state;
        use crate::web::workspace_ws_types::ProviderConfigSnapshot;
        use tower::ServiceExt;

        let fixture = seed_fixture(1, true);
        let app = fixture.router();
        let enable = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        let enrollment = response_json(enable).await;
        let revision = enrollment["policy_revision"].as_u64().unwrap();
        let (plan_id, session_id) = create_plan_and_session(&fixture, RunPolicy::Interactive);
        let bound = response_json(
            post_binding(
                &app,
                serde_json::json!({
                    "expected_revision": revision,
                    "plan_id": plan_id,
                    "session_id": session_id,
                }),
            )
            .await,
        )
        .await;
        let bound_revision = bound["policy_revision"].as_u64().unwrap();

        let coding_store = CodingAttemptStore::new(fixture.paths.clone());
        let attempt = coding_store
            .create_attempt(CreateCodingAttemptInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                work_item_id: "work_item_0001".to_string(),
                base_branch: "HEAD".to_string(),
                branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
                worktree_path: None,
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: crate::product::models::provider::ProviderName::Fake,
                    reviewer: None,
                    review_rounds: 0,
                    permission_modes: Default::default(),
                },
                target_snapshot: None,
                max_auto_rework: 1,
            })
            .unwrap();
        let mut group = attempt.clone();
        group.work_item_group_id = Some(plan_id.clone());
        coding_store.write_coding_attempt_for_test(&group).unwrap();
        // 快照装配的 plan-repair reconcile 需要最小 lineage。
        crate::product::work_item_revision_store::WorkItemRevisionStore::new(
            fixture.paths.clone(),
        )
        .put_plan_lineage(
            &crate::product::models::work_item_revision::WorkItemPlanLineage {
                id: plan_id.clone(),
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                story_spec_refs: Vec::new(),
                design_spec_refs: Vec::new(),
                active_revision_id: None,
                active_amendment_id: None,
                created_at: "2026-10-16T00:00:00Z".to_string(),
                updated_at: "2026-10-16T00:00:00Z".to_string(),
            },
        )
        .unwrap();

        let ws_automation = |coding_store: &CodingAttemptStore| {
            let frame = build_coding_session_state(coding_store, group.clone()).unwrap();
            match frame {
                crate::web::coding_ws_handler::CodingWsOutMessage::CodingSessionState {
                    automation,
                    ..
                } => automation,
                _ => panic!("expected coding session state frame"),
            }
        };

        // Disable 态：owner=client（其余投影同判据），abort REST 不路由终止。
        let disable = serde_json::json!({
            "expected_revision": bound_revision,
            "command": {"type": "disable"}
        });
        let response = put_enrollment(&app, disable).await;
        assert_eq!(response.status(), StatusCode::OK);
        let disabled = ws_automation(&coding_store).expect("bound projection present");
        assert_eq!(disabled.owner, AutomationOwner::Client);
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/coding-attempts/{}/abort",
                        group.id
                    ))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let payload = response_json(response).await;
        assert_ne!(
            payload["code"],
            "automation_plan_termination_required",
            "disabled enrollment must not route single-attempt abort to termination"
        );

        // 重新 Enable 后终止：terminated 仍 managed（owner=server）。
        let enable = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        let enable_status = enable.status();
        let enable_body = response_json(enable).await;
        assert_eq!(enable_status, StatusCode::OK, "re-enable body: {enable_body:?}");
        let enabled_revision = enable_body["policy_revision"].as_u64().unwrap();
        let terminate = post_terminate(
            &app,
            &serde_json::json!({
                "command_id": "cmd-t3-f1-terminate",
                "expected_enrollment_id": enrollment["enrollment_id"],
                "expected_policy_revision": enabled_revision,
                "confirmed": true,
            }),
        )
        .await;
        assert_eq!(terminate.status(), StatusCode::OK);
        let terminated = ws_automation(&coding_store).expect("bound projection present");
        assert_eq!(terminated.owner, AutomationOwner::Server);
        assert!(terminated.terminated_at.is_some());
    }
}
