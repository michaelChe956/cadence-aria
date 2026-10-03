//! P1 WIGA Task 3：与人工 prepare 同源的共用数据面。
//!
//! `prepare_plan_records` 只做 provider 配置解析、源校验、routing/preflight、
//! plan/session 创建与既存身份核验（含 preflight 失败的 durable Failed 收敛）；
//! 异步 `ensure_workspace_context_message` 与 HTTP 响应投影留在调用方——
//! 人工 REST（`ids=None`）契约不变，bound 调用（Task 4，`ids=Some`）在
//! enrollment 文件锁内执行本函数、锁外补上下文消息。

use crate::product::issue_store::IssueStore;
use crate::product::lifecycle_store::{
    CreateIssueWorkItemPlanInput, CreateWorkspaceSessionInput, LifecycleStore,
};
use crate::product::logical_codebase::RepositoryRouting;
use crate::product::models::{IssueWorkItemPlan, WorkspaceSessionRecord, WorkspaceType};
use crate::product::work_item_plan_policy::{RunPolicy, WorkItemPlanFlowKind};
use crate::web::error::{ApiError, ApiResult};
use crate::web::state::WebAppState;
use crate::web::types::PrepareWorkItemPlanRequest;

use super::super::support::{
    find_repository, product_app_paths, product_store_api_error, provider_workspace_config,
    routing_api_error,
};
use super::preflight::{
    SingleCandidatePreflightDecision, logical_repository_ids_for_preflight,
    preflight_single_repository_candidate,
};
use super::{
    mark_single_candidate_prepare_failure, validate_confirmed_design_specs,
    validate_confirmed_story_specs,
};

use std::collections::BTreeSet;

/// 绑定创建的稳定目标 id（Task 4 从持久 prepare_intent_id 派生传入）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedPlanIds {
    pub plan_id: String,
    pub session_id: String,
}

pub struct PreparedPlanRecords {
    pub plan: IssueWorkItemPlan,
    pub session: WorkspaceSessionRecord,
}

/// 统一 prepare 数据面：manual（`ids=None`）保持现有 sequential id 与覆盖式
/// 创建；bound（`ids=Some`）先核对既存 plan/session 的冻结身份，绝不覆盖
/// 已有内容，且绑定 plan session 一律 `RunPolicy::Interactive`（REQ-WIGA-02）。
pub fn prepare_plan_records(
    state: &WebAppState,
    project_id: &str,
    issue_id: &str,
    request: PrepareWorkItemPlanRequest,
    ids: Option<PreparedPlanIds>,
) -> ApiResult<PreparedPlanRecords> {
    let workspace_config = provider_workspace_config(
        request.author_provider.as_deref(),
        request.reviewer_provider.as_deref(),
        request.review_rounds,
        request.superpowers_enabled,
        request.openspec_enabled,
        state.test_provider_enabled,
        &*state.provider_availability,
    )?;
    // rollout flag 只在创建 session 前读取一次；之后所有分支仅消费这份快照。
    let rollout_snapshot = state.work_item_plan_single_candidate;
    let app_paths = product_app_paths(state);
    let issue = IssueStore::new(app_paths.clone())
        .get(project_id, issue_id)
        .map_err(product_store_api_error)?;
    let lifecycle = LifecycleStore::new(app_paths.clone());
    validate_confirmed_story_specs(&lifecycle, project_id, issue_id, &request.story_spec_ids)?;
    validate_confirmed_design_specs(&lifecycle, project_id, issue_id, &request.design_spec_ids)?;

    // 在任何 plan/session/source/IR/run-history/transaction/provider 副作用之前确定 flow。
    // Logical 路径只读 manifest + selection；不得在此调用会写 invalidation 的 resolver。
    let routing = RepositoryRouting::load_for_issue(&app_paths, project_id, issue_id)
        .map_err(product_store_api_error)?;
    let preflight_failure_reason = match routing {
        RepositoryRouting::Legacy { .. } => {
            let repository_id = issue.repo_id.clone().ok_or_else(|| {
                ApiError::validation("repository_required", "repository_id is required")
            })?;
            let repository = find_repository(&app_paths, project_id, &repository_id)?;
            match preflight_single_repository_candidate(&[repository.id]) {
                SingleCandidatePreflightDecision::Eligible { .. } => None,
                SingleCandidatePreflightDecision::Ineligible { reason } => Some(reason),
            }
        }
        RepositoryRouting::Logical {
            manifest,
            selection,
        } => {
            // 保留 REQ-TGT-01：确认的 Design 只能引用当前 selection 中的目标。
            let selected_ids = logical_repository_ids_for_preflight(&manifest, &selection);
            let selected_ids = selected_ids.iter().collect::<BTreeSet<_>>();
            let designs = lifecycle
                .list_design_specs(project_id, issue_id)
                .map_err(product_store_api_error)?;
            let design = designs
                .iter()
                .find(|design| design.id == request.design_spec_ids[0])
                .ok_or_else(|| {
                    product_store_api_error(
                        crate::product::json_store::ProductStoreError::NotFound {
                            kind: "design_spec",
                            id: request.design_spec_ids[0].clone(),
                        },
                    )
                })?;
            for target in &design.involved_repository_ids {
                if !selected_ids.contains(&target.0.to_string()) {
                    return Err(ApiError::validation(
                        "target_not_in_selection",
                        format!("design involved {target:?} is not in issue codebase selection"),
                    ));
                }
            }
            // 缺陷 #7（2026-10-02 E2E）：聚合 Design（involved 非空）的单候选
            // 计数以其 involved 集为准（上方已校验 ⊆ selection，REQ-TGT-01）；
            // LC issue 的 selection 恒 all_members，按 selection 计数会把任何
            // 单成员 Design 的 plan 准备死锁在 preflight（issue_0001 现场
            // found 2）。无聚合视野的 Design（involved 空）保持 selection 口径。
            let repository_ids = if design.involved_repository_ids.is_empty() {
                selected_ids.into_iter().cloned().collect::<Vec<_>>()
            } else {
                design
                    .involved_repository_ids
                    .iter()
                    .map(|target| target.0.to_string())
                    .collect::<Vec<_>>()
            };
            match preflight_single_repository_candidate(&repository_ids) {
                SingleCandidatePreflightDecision::Eligible { .. } => None,
                SingleCandidatePreflightDecision::Ineligible { reason } => Some(reason),
            }
        }
        RepositoryRouting::FailClosed { code, reason } => {
            return Err(routing_api_error(code, &reason));
        }
    };
    let flow_kind = WorkItemPlanFlowKind::SingleCandidate;

    let plan_options = crate::product::models::IssueWorkItemPlanOptions {
        include_integration_tests: request.include_integration_tests.unwrap_or(true),
        include_e2e_tests: request.include_e2e_tests.unwrap_or(false),
        force_frontend_backend_split: request.force_frontend_backend_split.unwrap_or(false),
        require_execution_plan_confirm: request.require_execution_plan_confirm.unwrap_or(false),
    };
    let plan = match ids.clone() {
        // bound：既存 plan 冻结身份一致才复用，绝不覆盖（Task 3）。
        Some(ids) => lifecycle
            .ensure_issue_work_item_plan_with_identity(CreateIssueWorkItemPlanInput {
                id: Some(ids.plan_id),
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                source_story_spec_ids: request.story_spec_ids.clone(),
                source_design_spec_ids: request.design_spec_ids.clone(),
                options: plan_options,
                status: crate::product::models::IssueWorkItemPlanStatus::Draft,
                work_item_ids: Vec::new(),
                repository_profile_ref: None,
                verification_plan_ids: Vec::new(),
                dependency_graph: Vec::new(),
                created_from_provider_run: None,
                validator_findings: Vec::new(),
            })
            .map_err(product_store_api_error)?,
        None => lifecycle
            .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
                id: None,
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                source_story_spec_ids: request.story_spec_ids.clone(),
                source_design_spec_ids: request.design_spec_ids.clone(),
                options: plan_options,
                status: crate::product::models::IssueWorkItemPlanStatus::Draft,
                work_item_ids: Vec::new(),
                repository_profile_ref: None,
                verification_plan_ids: Vec::new(),
                dependency_graph: Vec::new(),
                created_from_provider_run: None,
                validator_findings: Vec::new(),
            })
            .map_err(product_store_api_error)?,
    };

    // P1 WIGA Task 10：测试注入的绑定 plan 落盘中窗（session 创建之前），
    // 仅 bound 路径可触发；manual（ids=None）不经过本窗口。
    #[cfg(test)]
    if ids.is_some()
        && crate::product::issue_automation_store::automation_crash_window::fire_once(
            crate::product::issue_automation_store::automation_crash_window::CrashWindow::AfterPlanSaved,
        )
    {
        return Err(ApiError::runtime(
            "automation_crash_window",
            "interrupted after bound plan saved",
            serde_json::json!({}),
        ));
    }

    let session = match ids {
        Some(ids) => lifecycle
            .create_workspace_session_bound(
                CreateWorkspaceSessionInput {
                    project_id: project_id.to_string(),
                    issue_id: issue_id.to_string(),
                    entity_id: plan.id.clone(),
                    workspace_type: WorkspaceType::WorkItemPlan,
                    author_provider: workspace_config.author_provider,
                    reviewer_provider: Some(workspace_config.reviewer_provider),
                    review_rounds: workspace_config.review_rounds,
                    superpowers_enabled: workspace_config.superpowers_enabled,
                    openspec_enabled: workspace_config.openspec_enabled,
                    work_item_plan_options: Some(
                        crate::product::lifecycle_store::WorkItemPlanSessionOptions {
                            flow_kind,
                            // REQ-WIGA-02：绑定 plan session 永远 Interactive。
                            run_policy: RunPolicy::Interactive,
                            rollout_snapshot,
                        },
                    ),
                },
                ids.session_id,
            )
            .map_err(product_store_api_error)?,
        None => lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                entity_id: plan.id.clone(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: workspace_config.author_provider,
                reviewer_provider: Some(workspace_config.reviewer_provider),
                review_rounds: workspace_config.review_rounds,
                superpowers_enabled: workspace_config.superpowers_enabled,
                openspec_enabled: workspace_config.openspec_enabled,
                work_item_plan_options: Some(
                    crate::product::lifecycle_store::WorkItemPlanSessionOptions {
                        flow_kind,
                        run_policy: request.run_policy.unwrap_or(RunPolicy::Interactive),
                        rollout_snapshot,
                    },
                ),
            })
            .map_err(product_store_api_error)?,
    };
    let session_id = session.id.clone();
    // L2 退役（T5/REQ-WSC-08）：确定性 preflight 失败收敛新路径 durable Failed
    // 终态（含原因）——无 legacy 回落、无 flow_kind 切换。
    if let Some(reason) = preflight_failure_reason {
        mark_single_candidate_prepare_failure(
            &lifecycle,
            &session_id,
            &format!("single-candidate preflight failed before session side effects: {reason}"),
        );
        return Err(ApiError::validation(
            "SINGLE_CANDIDATE_PREFLIGHT_FAILED",
            reason,
        ));
    }

    Ok(PreparedPlanRecords { plan, session })
}

/// P1 WIGA Task 4：后台补偿入口——enrollment 锁内唯一创建/绑定（`ensure_
/// plan_binding`），create 回调把冻结意图转成 `PrepareWorkItemPlanRequest`
/// 走共用 prepare 数据面（`ids=Some`，绝不覆盖既存内容）；锁外再补
/// `ensure_workspace_context_message`（幂等，崩溃后下次补偿重试）。
pub async fn ensure_enrolled_plan(
    state: &WebAppState,
    enrollment: &crate::product::models::automation::IssueAutomationEnrollment,
) -> ApiResult<PreparedPlanRecords> {
    // C1 Task 3：prepare 前置——自动链只接受当前版本化 binding（旧式
    // enrollment 按 off/Manual 解释，自动路径 fail-closed）。
    crate::web::advance_plan::load_current_enrollment_binding(enrollment)
        .map_err(|reason| ApiError::validation("automation_enrollment_binding_invalid", reason))?;

    let store =
        crate::product::issue_automation_store::IssueAutomationStore::new(product_app_paths(state));
    let (project_id, issue_id) = (enrollment.project_id.clone(), enrollment.issue_id.clone());
    // create 回调错误类型固定为 EnrollmentError：ApiError 原样暂存、锁外还原，
    // 不丢验证/运行错误细节。
    let mut create_error: Option<ApiError> = None;
    let bound = store
        .ensure_plan_binding(
            &project_id,
            &issue_id,
            &enrollment.enrollment_id,
            |_, intent| {
                let request = intent_request(intent);
                match prepare_plan_records(
                    state,
                    &project_id,
                    &issue_id,
                    request,
                    Some(PreparedPlanIds {
                        plan_id: intent.plan_id.clone(),
                        session_id: intent.session_id.clone(),
                    }),
                ) {
                    Ok(_) => Ok(()),
                    Err(error) => {
                        create_error = Some(error);
                        Err(
                            crate::product::models::automation::EnrollmentError::InvalidScope(
                                "bound plan creation failed".to_string(),
                            ),
                        )
                    }
                }
            },
        )
        .map_err(|error| match create_error {
            Some(api_error) => api_error,
            None => crate::web::handlers::automation_enrollment::enrollment_api_error(error),
        })?;

    // 绑定已 durable；锁外加载记录并补异步上下文消息（Unchanged 幂等路径同样补）。
    let app_paths = product_app_paths(state);
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let Some(plan_id) = bound.plan_id.clone() else {
        return Err(ApiError::runtime(
            "automation_enrollment_binding_missing",
            "enrollment has no bound plan after compensation",
            serde_json::json!({}),
        ));
    };
    let Some(session_id) = bound.session_id.clone() else {
        return Err(ApiError::runtime(
            "automation_enrollment_binding_missing",
            "enrollment has no bound session after compensation",
            serde_json::json!({}),
        ));
    };
    let plan = lifecycle
        .get_issue_work_item_plan(&project_id, &issue_id, &plan_id)
        .map_err(product_store_api_error)?;
    let session = lifecycle
        .get_workspace_session(&session_id)
        .map_err(product_store_api_error)?;
    let session = match crate::web::workspace_context::ensure_workspace_context_message(
        &app_paths, &lifecycle, session,
    )
    .await
    {
        Ok(session) => session,
        Err(error) => {
            // 绑定已 durable 成功：上下文消息是幂等只读补齐，失败不回滚绑定、
            // 也不标记 Failed（那会毒化链条）；下次补偿（Unchanged 快路径）重试。
            eprintln!("wiga bound plan context message deferred: {error}");
            lifecycle
                .get_workspace_session(&session_id)
                .map_err(product_store_api_error)?
        }
    };
    Ok(PreparedPlanRecords { plan, session })
}

/// 冻结意图 → 共用 prepare 请求：provider/选项只取意图冻结值（补偿时绝不
/// 重新取环境默认）；绑定 plan session 一律 Interactive（REQ-WIGA-02）。
fn intent_request(
    intent: &crate::product::models::automation::PreparedPlanIntent,
) -> PrepareWorkItemPlanRequest {
    let options = &intent.options;
    let provider = |name: &crate::product::models::provider::ProviderName| {
        serde_json::to_value(name)
            .ok()
            .and_then(|value| value.as_str().map(str::to_string))
            .unwrap_or_default()
    };
    PrepareWorkItemPlanRequest {
        title: "Work Item Plan（自动化）".to_string(),
        story_spec_ids: intent
            .source
            .stories
            .iter()
            .map(|reference| reference.id.clone())
            .collect(),
        design_spec_ids: intent
            .source
            .designs
            .iter()
            .map(|reference| reference.id.clone())
            .collect(),
        author_provider: Some(provider(&options.author_provider)),
        reviewer_provider: Some(provider(&options.reviewer_provider)),
        review_rounds: Some(options.review_rounds),
        superpowers_enabled: Some(options.superpowers_enabled),
        openspec_enabled: Some(options.openspec_enabled),
        run_policy: Some(RunPolicy::Interactive),
        include_integration_tests: Some(options.plan_options.include_integration_tests),
        include_e2e_tests: Some(options.plan_options.include_e2e_tests),
        force_frontend_backend_split: Some(options.plan_options.force_frontend_backend_split),
        require_execution_plan_confirm: Some(options.plan_options.require_execution_plan_confirm),
    }
}
