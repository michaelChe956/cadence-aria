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
    ConflictKind, EnrollmentBindingIdentityInput, EnrollmentError, EnrollmentRebindRequest,
    EnrollmentRebindResult, EnrollmentSource, EnrollmentWriteCommand, IssueAutomationEnrollment,
    LeaseTakeoverRequest, LeaseTakeoverResult, OperationState,
};
use crate::product::work_item_plan_policy::RunPolicy;
use crate::web::error::{ApiError, ApiResult};
use crate::web::handlers::lifecycle::preflight::logical_repository_ids_for_preflight;
use crate::web::state::WebAppState;
use crate::web::types::{EnrollmentBindRequest, EnrollmentPutRequest};

use super::automation_enrollment_conflict::{
    enrollment_conflict, enrollment_payload_immutable, enrollment_revision_stale,
    termination_api_error, termination_identity_conflict,
};
use super::automation_enrollment_reenable::ensure_terminated_reenable_cleanup_closed;
use super::support::{product_app_paths, product_store_api_error};

pub async fn put_automation_enrollment(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Json(request): Json<EnrollmentPutRequest>,
) -> ApiResult<Json<IssueAutomationEnrollment>> {
    validate_request_ids(&project_id, &issue_id)?;
    validate_enrollment_scope(&state, &project_id, &issue_id, &request.command)?;
    ensure_terminated_reenable_cleanup_closed(&state, &project_id, &issue_id, &request.command)?;
    // 代际取代＋换代/悬空源重授权（编排收敛在 reenable 模块；本文件在
    // large_file_guard 登记表内禁止增长）：terminated 重开＝旧 plan 作废，
    // 一律走 `compare_and_set_reenable`：非悬空源时 options 漂移按换代放行、
    // source/target 漂移仍拒；悬空源时仅放行 source 漂移，options/target 仍逐字段相等（REQ-DRG-18）。
    let enrollment =
        super::automation_enrollment_reenable::supersede_terminated_generation_and_cas(
            &state,
            &project_id,
            &issue_id,
            request.expected_revision,
            request.command,
        )
        .await?;
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
    // Focus 5）。单仓载体已删除（REQ-ROUTE-01）：无归属 issue 在载体
    // 判定处即 fail-closed，此处只剩 LC 载体的 canonical target 校验。
    let issue = IssueStore::new(paths.clone())
        .get(project_id, issue_id)
        .map_err(product_store_api_error)?;
    let carrier = super::support::resolve_automation_carrier(&paths, project_id, &issue)?;
    let super::support::AutomationCarrierResolution::LogicalCodebase { resolution } = &carrier;
    {
        // 批 2 R1（REQ-WIGA-01、REQ-MTG-03）：LC 载体按 selection 候选
        // 构造 canonical target（0 拒/1 旧 kind/N 集合）——提交 target
        // 必须与 canonical 全等（乱序/重复已在反序列化归一），错配
        // 422 重选提示，不猜、不降级、不以 plan focus 上界替代授权集。
        let manifest = resolution.manifest.clone().ok_or_else(|| {
            invalid_scope("automation enrollment requires a logical codebase manifest routing")
        })?;
        let selection = resolution.selection.as_ref().ok_or_else(|| {
            invalid_scope("automation enrollment requires an explicit logical codebase selection")
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
        EnrollmentError::Conflict {
            current_revision,
            kind,
        } => match kind {
            // 类 1（运行中改 payload）：该先终止自动化，不是「等一下重试」。
            ConflictKind::PayloadImmutable => enrollment_payload_immutable(current_revision, kind),
            // 类 2（版本号过期）：该刷新后重试。
            ConflictKind::RevisionStale => enrollment_revision_stale(current_revision, kind),
            // 其余 7 类落回旧码（码字/状态码/message/details 逐字不变）。
            _ => enrollment_conflict(current_revision, "automation enrollment revision conflict"),
        },
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
    /// 托管判定已由「绑定在场 + 精确 plan/版本」完成;enabled 保留供诊断扩展。
    #[allow(dead_code)]
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
    let enrollment =
        IssueAutomationStore::new(paths.clone()).get(&attempt.project_id, &attempt.issue_id)?;
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
) -> Result<Option<crate::product::models::automation::AutomationOwnership>, ProductStoreError> {
    let Some(plan_id) = attempt.work_item_group_id.clone() else {
        return Ok(None);
    };
    let enrollment =
        IssueAutomationStore::new(paths.clone()).get(&attempt.project_id, &attempt.issue_id)?;
    Ok(enrollment
        .filter(|enrollment| enrollment.plan_id.as_deref() == Some(plan_id.as_str()))
        .map(
            |enrollment| crate::product::models::automation::AutomationOwnership {
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
                voided_at: enrollment.voided_at,
            },
        ))
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
        return Err(termination_identity_conflict(
            &identity,
            "termination enrollment identity mismatch",
        ));
    }
    // 提示性绑定互证：请求携带的 expected_binding 必须与当前代全等。
    if let Some(expected_binding) = request.expected_binding.as_ref()
        && let Some(history) = enrollment.binding_history.as_ref()
        && expected_binding != &history.current
    {
        return Err(termination_identity_conflict(
            &identity,
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
        .map_err(|error| termination_api_error(error, &identity))?;
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
            if let Err(error) =
                store.resolve_automation_attention_fact(&project_id, &issue_id, &binding, key)
            {
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

// continue-chain REST 入口拆分片段（同模块 include!，行数红线，纯移动）。
include!("automation_enrollment_continue_chain.inc.rs");
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

    // 以下三段为 tests mod `include!` 拆分片段（large_file_guard 行数红线，纯移动）。
    include!("automation_enrollment_tests_http.inc.rs");
    include!("automation_enrollment_tests_termination.inc.rs");
    include!("automation_enrollment_tests_reenable.inc.rs");
    include!("automation_reenable_supersede_tests.inc.rs");
}
