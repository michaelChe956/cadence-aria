use super::dto::*;
use super::lifecycle::confirm_workspace_entity;
use super::support::*;
use super::*;
use crate::product::workspace_engine::{HttpConfirmDisposition, workspace_stage_for_status};

/// P0 1.2（REQ-WIGA-08）：HTTP 出口统一以 durable enrollment 计算会话归属；
/// 读取失败显式报错（product_store_api_error），绝不伪 client。
fn session_automation_ownership(
    state: &WebAppState,
    session: &crate::product::models::WorkspaceSessionRecord,
) -> ApiResult<crate::product::models::automation::AutomationOwnership> {
    crate::product::issue_automation_store::IssueAutomationStore::new(product_app_paths(state))
        .ownership_for_session(session)
        .map_err(product_store_api_error)
}

pub async fn workspace_session_message(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
    Json(request): Json<WorkspaceSessionMessageRequest>,
) -> ApiResult<Json<WorkspaceSessionDto>> {
    validate_workspace_message(&request)?;
    let session = LifecycleStore::new(product_app_paths(&state))
        .append_workspace_message(&session_id, request.role, request.content)
        .map_err(product_store_api_error)?;
    let automation = session_automation_ownership(&state, &session)?;
    Ok(Json(workspace_session_dto(session, automation)))
}

pub async fn workspace_session_run_next(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
    Json(request): Json<WorkspaceSessionRunNextRequest>,
) -> ApiResult<Json<WorkspaceSessionDto>> {
    let paths = product_app_paths(&state);
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .get_workspace_session(&session_id)
        .map_err(product_store_api_error)?;
    let user_prompt = workspace_user_prompt(request.user_prompt);
    lifecycle
        .append_workspace_message(&session_id, "user".to_string(), user_prompt.clone())
        .map_err(product_store_api_error)?;

    let runner = ProviderWorkspaceRunner::new(paths);
    let output = runner
        .run_next(
            WorkspaceProviderRunInput {
                session_id,
                user_prompt: provider_workspace_prompt(user_prompt),
            },
            &FakeProviderAdapter,
        )
        .map_err(|error| {
            ApiError::runtime(
                "provider_workspace_run_failed",
                "provider workspace run failed",
                json!({"details": error.details}),
            )
        })?;
    let automation = session_automation_ownership(&state, &output.session)?;
    Ok(Json(workspace_session_dto(output.session, automation)))
}

pub async fn workspace_session_takeover(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
) -> ApiResult<Json<WorkspaceSessionTakeoverDto>> {
    let lifecycle = LifecycleStore::new(product_app_paths(&state));
    // 批 2 P4（REQ-WIGA-08，C1 裁定）：精确绑定当前 enabled enrollment 的
    // plan 会话一律服务端拒绝 takeover（沿用既有码，不建 Interactive 子
    // session、不变更绑定）；非该绑定范围（无 enrollment/未绑定/已禁用/
    // story-design 等非 plan 会话）保持既有接管语义。不以 AutoIfValid 猜
    // 是否托管——只认 durable enrollment 精确绑定。
    let parent = lifecycle
        .get_workspace_session(&session_id)
        .map_err(product_store_api_error)?;
    let precisely_managed = crate::product::issue_automation_store::IssueAutomationStore::new(
        product_app_paths(&state),
    )
    .get(&parent.project_id, &parent.issue_id)
    .map_err(product_store_api_error)?
    .is_some_and(|enrollment| {
        enrollment.enabled
            && enrollment.session_id.as_deref() == Some(session_id.as_str())
            && enrollment
                .binding_history
                .as_ref()
                .is_some_and(|history| history.current.session_id == session_id)
    });
    if precisely_managed {
        return Err(ApiError::runtime(
            "workspace_session_takeover_not_allowed",
            "workspace session takeover is not allowed",
            json!({
                "reason": "session is precisely bound to an enabled automation enrollment",
                "session_id": session_id,
            }),
        ));
    }
    let child = lifecycle
        .takeover_stopped_needs_human(&session_id)
        .map_err(workspace_session_takeover_api_error)?;
    let event = lifecycle
        .get_human_gate_takeover_event(&session_id)
        .map_err(workspace_session_takeover_api_error)?
        .ok_or_else(|| {
            ApiError::runtime(
                "workspace_session_takeover_event_missing",
                "workspace session takeover event is missing",
                json!({"parent_session_id": session_id}),
            )
        })?;
    let automation = session_automation_ownership(&state, &child)?;
    Ok(Json(WorkspaceSessionTakeoverDto {
        workspace_session: workspace_session_dto(child, automation),
        parent_session_id: event.parent_session_id,
        takeover_event_id: event.id,
    }))
}

/// REQ-DLS-03：租约诊断只读端点——返回当前持有者与最近转移序列。活跃 manager
/// 优先（内存快照）；manager 已回收时退化为 durable jsonl 尾读（最近 200 行）。
pub async fn workspace_session_lease_diagnostics(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
) -> ApiResult<Json<serde_json::Value>> {
    if let Some(manager) = state.workspace_sessions.get(&session_id).await {
        return Ok(Json(manager.lease_diagnostics_snapshot()));
    }
    let lifecycle = LifecycleStore::new(product_app_paths(&state));
    let events = crate::web::workspace_session::lease_diagnostics_path(&lifecycle, &session_id)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|content| {
            let mut events: Vec<serde_json::Value> = content
                .lines()
                .filter_map(|line| serde_json::from_str(line).ok())
                .collect();
            if events.len() > 200 {
                events.drain(0..events.len() - 200);
            }
            events
        })
        .unwrap_or_default();
    Ok(Json(json!({
        "session_id": session_id,
        "holder": null,
        "epoch": null,
        "last_holder": null,
        "events": events,
    })))
}

fn workspace_session_takeover_api_error(error: ProductStoreError) -> ApiError {
    match error {
        ProductStoreError::InvalidRecord {
            kind: "human_gate_takeover",
            reason,
        } => ApiError::runtime(
            "workspace_session_takeover_not_allowed",
            "workspace session takeover is not allowed",
            json!({"reason": reason}),
        ),
        other => product_store_api_error(other),
    }
}

pub async fn workspace_session_confirm(
    State(state): State<WebAppState>,
    Path(session_id): Path<String>,
    Json(request): Json<WorkspaceSessionConfirmRequest>,
) -> ApiResult<Json<WorkspaceSessionDto>> {
    let lifecycle = LifecycleStore::new(product_app_paths(&state));
    let session = lifecycle
        .get_workspace_session(&session_id)
        .map_err(product_store_api_error)?;
    let manager = state.workspace_sessions.get(&session_id).await;

    // ── F-31 fix round（k3 P1）：在途守卫 ──────────────────────────────────
    // story/design 的 durable `running` 写点=在途驱动（生成启动于
    // provider_drive 作者生成 / lifecycle 的 start-generation 流，F-31 评审启动于
    // `begin_review_after_author_confirm` 进 CrossReview），故它等价于「有 run 在途」。
    // 此时端点既不得定稿也不得等锁：
    // ① provider run 任务在整段 drive 里持引擎锁（见 workspace_ws_handler/run 的
    //    「stream owner holds the engine mutex」），等锁会把确认请求挂到评审结束后
    //    才落定稿——用户双击/多 tab 的第二次确认变成不可诊断的静默定稿；
    // ② 落 Confirmed 会让迟到的 review 报告经 route_review_report_to_author_confirm
    //    把已定稿会话拖回 AuthorConfirm（Running→Confirmed→WaitingForHuman），
    //    与 F-30 终态口径相悖。
    if matches!(
        session.workspace_type,
        WorkspaceType::Story | WorkspaceType::Design
    ) && session.status == WorkspaceSessionStatus::Running
    {
        // stage 标注：锁空闲时取引擎 stage（残留态可诊断到 cross_review/revision），
        // run 持锁驱动中 stage 不可读，退回 durable 在途状态口径。
        let engine_handle = manager.as_ref().map(|manager| manager.engine());
        let stage = match engine_handle {
            Some(handle) => match handle.try_lock() {
                Ok(engine) => engine.session().stage.as_str(),
                Err(_) => workspace_stage_for_status(&session.status).as_str(),
            },
            None => workspace_stage_for_status(&session.status).as_str(),
        };
        return Err(ApiError::runtime(
            "workspace_session_confirm_not_allowed",
            "workspace session is in flight; confirm is not allowed before the run returns to a human gate",
            json!({
                "workspace_session_id": session_id,
                "stage": stage,
                "status": workspace_session_status_text(&session.status),
            }),
        ));
    }

    // ── F-31：引擎裁决（stage 唯一权威）──────────────────────────────────
    // 活引擎在场时先问引擎：门态放行定稿，在途 stage（引擎内存面领先 durable 的残留态）
    // 拒收，review 启用且本轮未评审则接管本轮。引擎缺席（无 run 可驱动）退回既有通路。
    let disposition = match &manager {
        Some(manager) => {
            let engine_handle = manager.engine();
            let mut engine = engine_handle.lock().await;
            engine.http_confirm_disposition(request.with_review).await
        }
        None => HttpConfirmDisposition::NotHandled,
    };
    match disposition {
        HttpConfirmDisposition::ReviewStarted => {
            // 端点 200 后发请求 tab 会乐观置 confirmed，权威状态以本帧收敛
            // （cross_review/评审在途），否则已连接 tab 停在 confirmed 终态投影。
            if let Some(manager) = &manager {
                manager.broadcast_current_session_state();
            }
            let current = lifecycle
                .get_workspace_session(&session_id)
                .map_err(product_store_api_error)?;
            let automation = session_automation_ownership(&state, &current)?;
            return Ok(Json(workspace_session_dto(current, automation)));
        }
        HttpConfirmDisposition::Rejected { stage } => {
            return Err(ApiError::runtime(
                "workspace_session_confirm_not_allowed",
                "workspace session confirm is not allowed in the current stage",
                json!({"workspace_session_id": session_id, "stage": stage}),
            ));
        }
        HttpConfirmDisposition::AlreadyConfirmed => {
            let current = lifecycle
                .get_workspace_session(&session_id)
                .map_err(product_store_api_error)?;
            let automation = session_automation_ownership(&state, &current)?;
            return Ok(Json(workspace_session_dto(current, automation)));
        }
        // change plan-compile-gate-visibility（Task 3）：WorkItemPlan 批次确认门由引擎在本
        // 裁决内落 Confirmed（plan 确认 + 子 WorkItem 会话 + stage→Completed + Completed
        // 节点）。端点不得再走下面的 store-only 兜底（那正是「假 Confirmed、半落状态」的
        // 来源），只向已连接 WS 广播权威状态并返回权威 DTO——与 ReviewStarted 臂同构。
        HttpConfirmDisposition::WorkItemPlanConfirmed => {
            if let Some(manager) = &manager {
                manager.broadcast_current_session_state();
            }
            let current = lifecycle
                .get_workspace_session(&session_id)
                .map_err(product_store_api_error)?;
            let automation = session_automation_ownership(&state, &current)?;
            return Ok(Json(workspace_session_dto(current, automation)));
        }
        HttpConfirmDisposition::WorkItemPlanRejected { message } => {
            return Err(ApiError::runtime(
                "workspace_session_confirm_not_allowed",
                message,
                json!({
                    "workspace_session_id": session_id,
                    "stage": workspace_stage_for_status(&session.status).as_str(),
                }),
            ));
        }
        HttpConfirmDisposition::Finalize | HttpConfirmDisposition::NotHandled => {}
        // F-31 纠正轮：用户显式要求送审但会话未启用 review——如实 4xx 拒绝，不静默定稿。
        HttpConfirmDisposition::ReviewUnavailable => {
            return Err(ApiError::runtime(
                "workspace_session_review_not_enabled",
                "review is not enabled for this workspace session; confirm without with_review to finalize",
                json!({
                    "workspace_session_id": session_id,
                    "stage": workspace_stage_for_status(&session.status).as_str(),
                }),
            ));
        }
    }

    // Blocker 2 修复：先 gate 后确认。confirm_workspace_entity 内部先跑 product 层
    // validate_confirm_aggregate_spec（多仓 involved/change_order 校验），gate 失败即返回
    // 4xx，此时 session 尚未被置 Confirmed（不再出现“先确认后 gate 失败遗留已 Confirmed”的不一致）。
    confirm_workspace_entity(&lifecycle, &session)?;
    lifecycle
        .update_workspace_session_status(&session_id, WorkspaceSessionStatus::Confirmed)
        .map_err(product_store_api_error)?;
    if let HttpConfirmDisposition::Finalize = disposition {
        // F-31 fix round（k3 P2）：引擎面与端点同一提交实现——Fake reviewer 快速路径的
        // 二次确认此前落 store-only，缺 confirmed_by / Completed 节点 / 终态 stage。
        if let Some(manager) = &manager {
            let engine_handle = manager.engine();
            let mut engine = engine_handle.lock().await;
            engine.commit_finalize_artifact("已确认通过").await;
        }
    }
    let confirmed = lifecycle
        .append_workspace_message(
            &session_id,
            "system".to_string(),
            format!("已由 {} 确认当前 Workspace 产物。", request.confirmed_by),
        )
        .map_err(product_store_api_error)?;
    // F-25b：durable 写完成后向已连接 WS 广播 confirmed session_state——前端
    // 乐观 setSessionStatus 只覆盖发请求的 tab，其他 tab/重连必须由服务端推送
    // 才能看到投影收敛（对照 F-09 HumanGateOpened 广播先例）。
    if let Some(manager) = &manager {
        manager.broadcast_http_confirm(&confirmed).await;
    }
    let automation = session_automation_ownership(&state, &confirmed)?;
    Ok(Json(workspace_session_dto(confirmed, automation)))
}

pub async fn workspace_session_timeline_node_detail(
    State(state): State<WebAppState>,
    Path((session_id, node_id)): Path<(String, String)>,
) -> ApiResult<Json<NodeDetail>> {
    let detail = LifecycleStore::new(product_app_paths(&state))
        .load_node_detail(&session_id, &node_id)
        .map_err(node_detail_store_api_error)?;
    Ok(Json(detail))
}

pub async fn workspace_session_timeline_node_prompt(
    State(state): State<WebAppState>,
    Path((session_id, node_id)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let detail = LifecycleStore::new(product_app_paths(&state))
        .load_node_detail(&session_id, &node_id)
        .map_err(node_detail_store_api_error)?;
    let prompt = detail.prompt.ok_or_else(|| {
        ApiError::runtime(
            "node_detail_prompt_not_found",
            "node detail prompt not found",
            json!({}),
        )
    })?;
    Ok(Json(json!({"node_id": node_id, "prompt": prompt})))
}

pub async fn workspace_session_timeline_event_output(
    State(state): State<WebAppState>,
    Path((session_id, node_id, event_id)): Path<(String, String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let detail = LifecycleStore::new(product_app_paths(&state))
        .load_node_detail(&session_id, &node_id)
        .map_err(node_detail_store_api_error)?;
    let output = detail
        .execution_events
        .iter()
        .find(|event| event.get("event_id").and_then(|value| value.as_str()) == Some(&event_id))
        .and_then(|event| event.get("output").and_then(|value| value.as_str()))
        .ok_or_else(|| {
            ApiError::runtime(
                "event_output_not_found",
                "timeline event output not found",
                json!({}),
            )
        })?;
    Ok(Json(
        json!({"node_id": node_id, "event_id": event_id, "output": output}),
    ))
}

pub async fn workspace_session_artifact_version(
    State(state): State<WebAppState>,
    Path((session_id, version)): Path<(String, u32)>,
) -> ApiResult<Json<serde_json::Value>> {
    let version = LifecycleStore::new(product_app_paths(&state))
        .list_artifact_versions(&session_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .find(|artifact| artifact.version == version)
        .ok_or_else(|| {
            ApiError::runtime(
                "artifact_version_not_found",
                "artifact version not found",
                json!({}),
            )
        })?;
    let artifact = serde_json::to_value(&version.payload).map_err(|error| {
        ApiError::runtime(
            "artifact_version_serialize_failed",
            format!("artifact version serialize failed: {error}"),
            json!({ "version": version.version }),
        )
    })?;
    let markdown = version.markdown().to_string();
    let generated_by = provider_name_text(&version.generated_by).to_string();
    let reviewed_by = version
        .reviewed_by
        .as_ref()
        .map(provider_name_text)
        .map(str::to_string);
    let review_verdict = version
        .review_verdict
        .as_ref()
        .map(review_verdict_text)
        .map(str::to_string);
    Ok(Json(json!({
        "version": version.version,
        "markdown": markdown,
        "artifact": artifact,
        "generated_by": generated_by,
        "reviewed_by": reviewed_by,
        "review_verdict": review_verdict,
        "confirmed_by": version.confirmed_by,
        "is_current": version.is_current,
        "created_at": version.created_at,
        "source_node_id": version.source_node_id,
    })))
}

pub(crate) fn validate_workspace_message(
    request: &WorkspaceSessionMessageRequest,
) -> ApiResult<()> {
    if !matches!(
        request.role.as_str(),
        "user" | "assistant" | "system" | "provider" | "reviewer"
    ) || request.content.trim().is_empty()
    {
        return Err(ApiError::validation(
            "invalid_workspace_message",
            "workspace message role/content is invalid",
        ));
    }
    Ok(())
}

pub(crate) fn workspace_user_prompt(user_prompt: Option<String>) -> String {
    user_prompt
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "请基于当前 Issue 上下文生成或修订 Workspace 产物。".to_string())
}

pub(crate) fn provider_workspace_prompt(prompt: String) -> String {
    let structured = json!({
        "markdown": format!("# Provider Workspace\n\n{prompt}"),
        "review_result": "review completed",
        "revision_result": "revision completed"
    });
    format!(
        "{prompt}\n\n{}",
        structured_output_sentinel("fake0001", &structured)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::lifecycle_store::{AggregateDesignSpecScope, CreateDesignSpecInput};
    use crate::product::logical_codebase::LogicalRepositoryId;
    use crate::product::models::{WorkspaceSessionStatus, WorkspaceType};
    use tempfile::TempDir;

    #[tokio::test]
    async fn workspace_session_confirm_gate_failure_leaves_session_not_confirmed() {
        let tmp = TempDir::new().expect("temp dir");
        let root = tmp.path().to_path_buf();
        let state = WebAppState::new(root.clone(), WebRuntime::new_fake(root.clone()));
        let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.join(".aria")));
        let member_a = LogicalRepositoryId(uuid::Uuid::new_v4());
        let member_b = LogicalRepositoryId(uuid::Uuid::new_v4());
        // 多仓 Design（involved=2，无 change_order）→ confirm gate 必须 4xx 拦截。
        let design = lifecycle
            .create_design_spec(CreateDesignSpecInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                story_spec_ids: vec!["story_spec_0001".to_string()],
                title: "multi repo design".to_string(),
                aggregate_codebase: Some(AggregateDesignSpecScope {
                    logical_codebase_ref: uuid::Uuid::new_v4(),
                    effective_member_ids: vec![member_a, member_b],
                    involved_repository_ids: vec![member_a, member_b],
                    change_order: Vec::new(),
                }),
            })
            .expect("create design spec");
        let session = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: design.id.clone(),
                workspace_type: WorkspaceType::Design,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: Some(ProviderName::Codex),

                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .expect("create workspace session");
        let session_id = session.id.clone();

        let result = workspace_session_confirm(
            State(state),
            Path(session_id.clone()),
            Json(WorkspaceSessionConfirmRequest {
                confirmed_by: "user".to_string(),
                with_review: false,
            }),
        )
        .await;

        let error = result.expect_err("multi-repo Design without change_order must be rejected");
        assert_eq!(error.code, "change_order_required_for_logical_codebase");
        let response = error.into_response();
        assert!(
            response.status().is_client_error(),
            "gate failure must be 4xx, got {:?}",
            response.status()
        );

        let stored = lifecycle
            .get_workspace_session(&session_id)
            .expect("session must remain readable");
        assert_ne!(
            stored.status,
            WorkspaceSessionStatus::Confirmed,
            "HTTP confirm gate failure must NOT set the session to Confirmed"
        );
    }

    #[tokio::test]
    async fn workspace_session_artifact_version_includes_serialized_artifact_payload() {
        let tmp = TempDir::new().expect("temp dir");
        let root = tmp.path().to_path_buf();
        let state = WebAppState::new(root.clone(), WebRuntime::new_fake(root.clone()));
        let lifecycle = LifecycleStore::new(ProductAppPaths::new(root.join(".aria")));
        let session = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "story_spec_0001".to_string(),
                workspace_type: WorkspaceType::Story,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: Some(ProviderName::Codex),

                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .expect("create workspace session");
        lifecycle
            .save_artifact_versions(
                &session.id,
                &[ArtifactVersion {
                    version: 1,
                    payload: crate::web::workspace_ws_types::ArtifactPayload::Markdown {
                        markdown: "# Artifact\n".to_string(),
                        diff: None,
                    },
                    generated_by: ProviderName::ClaudeCode,
                    reviewed_by: Some(ProviderName::Codex),
                    review_verdict: Some(ReviewVerdictType::Pass),
                    confirmed_by: Some("user".to_string()),
                    is_current: true,
                    created_at: "2026-06-26T00:00:00Z".to_string(),
                    source_node_id: "timeline_node_001".to_string(),
                }],
            )
            .expect("save artifact versions");

        let Json(value) = workspace_session_artifact_version(State(state), Path((session.id, 1)))
            .await
            .expect("artifact version response");

        assert_eq!(value["version"], 1);
        assert_eq!(value["markdown"], "# Artifact\n");
        assert_eq!(value["artifact"]["markdown"], "# Artifact\n");
        assert!(value["artifact"]["diff"].is_null());
        assert_eq!(value["generated_by"], "claude_code");
        assert_eq!(value["reviewed_by"], "codex");
        assert_eq!(value["review_verdict"], "pass");
        assert_eq!(value["confirmed_by"], "user");
        assert_eq!(value["source_node_id"], "timeline_node_001");
    }

    /// 批 2 P4（REQ-WIGA-08，C1 裁定）：精确绑定 enabled enrollment 的会话
    /// takeover 一律服务端拒（沿用 `workspace_session_takeover_not_allowed`，
    /// 409、零子 session）；非托管（无 enrollment）原样放行。
    #[tokio::test]
    async fn takeover_rejected_for_precisely_bound_enabled_enrollment() {
        use crate::product::issue_automation_store::IssueAutomationStore;
        use crate::product::lifecycle_store::WorkItemPlanSessionOptions;
        use crate::product::models::IssueWorkItemPlanOptions;
        use crate::product::models::automation::{
            EnrollmentOptions, EnrollmentSource, EnrollmentWriteCommand, PlanApprovalPolicy,
        };
        use crate::product::work_item_plan_policy::{
            HumanGateSnapshot, HumanReason, RunPolicy, WorkItemPlanFlowKind,
        };
        use tower::ServiceExt;

        let inner = crate::web::handlers::automation_enrollment_test_support::seed_fixture(1, true);
        let root_path = inner._root.path().to_path_buf();
        let state = WebAppState::new(
            root_path.clone(),
            crate::web::runtime::WebRuntime::new_fake(root_path.clone()),
        );
        let paths = ProductAppPaths::new(root_path.join(".aria"));
        let lifecycle = LifecycleStore::new(paths.clone());
        let store = IssueAutomationStore::new(paths.clone());
        const PROJECT: &str = "project_0001";
        const ISSUE: &str = "issue_0001";

        store
            .compare_and_set(
                PROJECT,
                ISSUE,
                None,
                EnrollmentWriteCommand::Enable {
                    command_id: None,
                    selection_key: "takeover-reject".to_string(),
                    source: EnrollmentSource {
                        stories: Vec::new(),
                        designs: Vec::new(),
                    },
                    options: EnrollmentOptions {
                        author_provider: ProviderName::Fake,
                        reviewer_provider: ProviderName::Fake,
                        review_rounds: 0,
                        superpowers_enabled: false,
                        openspec_enabled: false,
                        plan_approval: PlanApprovalPolicy::Auto,
                        plan_options: IssueWorkItemPlanOptions {
                            include_integration_tests: false,
                            include_e2e_tests: false,
                            force_frontend_backend_split: false,
                            require_execution_plan_confirm: false,
                        },
                    },
                    target: crate::product::logical_codebase::EnrollmentTarget::SingleRepository {
                        repository_id: "repo-1".to_string(),
                    },
                },
            )
            .expect("enable enrollment");

        let stopped_record = |entity: &str| {
            let mut record = lifecycle
                .create_workspace_session(CreateWorkspaceSessionInput {
                    project_id: PROJECT.to_string(),
                    issue_id: ISSUE.to_string(),
                    entity_id: entity.to_string(),
                    workspace_type: WorkspaceType::WorkItemPlan,
                    author_provider: ProviderName::Fake,
                    reviewer_provider: None,
                    review_rounds: 0,
                    superpowers_enabled: false,
                    openspec_enabled: false,
                    work_item_plan_options: Some(WorkItemPlanSessionOptions {
                        flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                        run_policy: RunPolicy::AutoIfValid,
                        rollout_snapshot: false,
                    }),
                })
                .expect("create session");
            record.status = WorkspaceSessionStatus::StoppedNeedsHuman;
            record.single_candidate_phase =
                Some(crate::product::models::SingleCandidatePhase::Approval);
            record.human_gate_snapshot = Some(HumanGateSnapshot {
                candidate_recovery: None,
                findings: Vec::new(),
                repeated_fingerprints: Vec::new(),
                attempts_used: 0,
                manual_repairs_remaining: 1,
                trigger: HumanReason::NativeHumanRequired,
                resumable: true,
                accepted_feedback_turns: Some(0),
            });
            crate::product::json_store::write_json(
                &lifecycle
                    .app_paths()
                    .issue_lifecycle_root(PROJECT, ISSUE)
                    .join("workspace-sessions")
                    .join(format!("{}.json", record.id)),
                &record,
            )
            .expect("persist stopped record");
            record
        };

        let bound = stopped_record("plan_bound_1");
        store
            .bind_plan(PROJECT, ISSUE, 1, "plan_bound_1", &bound.id)
            .expect("bind managed plan");
        let unmanaged = stopped_record("plan_unmanaged_1");

        let app = crate::web::app::build_web_router(state.clone());
        let post_takeover = |session_id: String| {
            let app = app.clone();
            async move {
                let response = app
                    .oneshot(
                        axum::http::Request::builder()
                            .method(axum::http::Method::POST)
                            .uri(format!("/api/workspace-sessions/{session_id}/takeover"))
                            .body(axum::body::Body::empty())
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                let status = response.status();
                let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
                    .await
                    .unwrap();
                let payload: serde_json::Value =
                    serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
                (status, payload)
            }
        };

        // 精确绑定 enabled enrollment → 既有码 409 拒绝、零子 session。
        let (status, payload) = post_takeover(bound.id.clone()).await;
        assert_eq!(status, axum::http::StatusCode::CONFLICT, "{payload}");
        assert_eq!(
            payload["code"], "workspace_session_takeover_not_allowed",
            "{payload}"
        );
        assert!(
            lifecycle
                .get_human_gate_takeover_event(&bound.id)
                .expect("takeover event read")
                .is_none(),
            "rejected takeover must not create a child session"
        );

        // 非托管（无 enrollment 绑定该会话）→ 原样放行接管。
        let (status, payload) = post_takeover(unmanaged.id.clone()).await;
        assert_eq!(status, axum::http::StatusCode::OK, "{payload}");
        assert!(
            lifecycle
                .get_human_gate_takeover_event(&unmanaged.id)
                .expect("takeover event read")
                .is_some(),
            "unmanaged takeover must keep legacy behavior"
        );
    }

    /// A4（SC 失败修复包）REST 公开视图三相：pending（缺发布回执）不可读；坏回执
    /// 显式 HTTP 错误（不吞成 404 not found）；真实回执落盘后可读。
    #[tokio::test]
    async fn sc_repair_package_rest_view_hides_pending_and_surfaces_receipt_failures() {
        use crate::product::lifecycle_store::WorkItemPlanSessionOptions;
        use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
        use crate::web::workspace_ws_types::ArtifactPayload;
        use crate::web::workspace_ws_types::ScArtifactPublishedReceipt;
        use crate::web::workspace_ws_types::ScEvidenceRef;
        use crate::web::workspace_ws_types::ScFailureRepairPackage;
        use crate::web::workspace_ws_types::ScFailureSourceEvidence;
        use crate::web::workspace_ws_types::ScPackageCompleteness;
        use sha2::{Digest, Sha256};

        let tmp = TempDir::new().expect("temp dir");
        let root = tmp.path().to_path_buf();
        let state = WebAppState::new(root.clone(), WebRuntime::new_fake(root.clone()));
        let paths = ProductAppPaths::new(root.join(".aria"));
        let lifecycle = LifecycleStore::new(paths.clone());
        let session = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "plan_0001".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: Some(ProviderName::Codex),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: Some(WorkItemPlanSessionOptions {
                    flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                    ..Default::default()
                }),
            })
            .expect("create workspace session");

        let available = || ScEvidenceRef::Available {
            locator: "evidence".to_string(),
            sha256: "0".repeat(64),
            bytes: 0,
        };
        let package = ScFailureRepairPackage {
            schema_version: 1,
            package_id: "pkg_rest_sc_1".to_string(),
            chain_id: "chain_rest_sc_1".to_string(),
            session_id: session.id.clone(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            plan_id: "plan_0001".to_string(),
            terminal_class: "compile".to_string(),
            terminal_reason: "terminal compile failure".to_string(),
            completeness: ScPackageCompleteness::Complete,
            rounds: Vec::new(),
            last_provider_output: available(),
            last_failure_source: ScFailureSourceEvidence {
                source_round_id: "round_1".to_string(),
                source: available(),
            },
            teaching_prompts: Vec::new(),
            evidence_log_ref: "sc-failure-evidence/chain_rest_sc_1".to_string(),
            evidence_digest: "a".repeat(64),
            evidence_through_entry_key: "entry_1".to_string(),
            missing_evidence: Vec::new(),
        };
        let mut hasher = Sha256::new();
        hasher.update(serde_json::to_vec(&package).expect("package is serializable"));
        let package_digest: String = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        lifecycle
            .save_artifact_versions(
                &session.id,
                &[
                    ArtifactVersion {
                        version: 1,
                        payload: ArtifactPayload::Markdown {
                            markdown: "# 普通产物\n".to_string(),
                            diff: None,
                        },
                        generated_by: ProviderName::ClaudeCode,
                        reviewed_by: None,
                        review_verdict: None,
                        confirmed_by: None,
                        is_current: true,
                        created_at: "2026-11-13T00:00:00Z".to_string(),
                        source_node_id: "node_sc_1".to_string(),
                    },
                    ArtifactVersion {
                        version: 2,
                        payload: ArtifactPayload::ScFailureRepairPackage {
                            sc_failure_repair_package: Box::new(package.clone()),
                        },
                        generated_by: ProviderName::ClaudeCode,
                        reviewed_by: None,
                        review_verdict: None,
                        confirmed_by: None,
                        is_current: false,
                        created_at: "2026-11-13T00:00:00Z".to_string(),
                        source_node_id: "node_sc_1".to_string(),
                    },
                ],
            )
            .expect("save artifact versions");
        lifecycle
            .save_node_detail(
                &session.id,
                "node_sc_1",
                &crate::product::models::NodeDetail {
                    node_id: "node_sc_1".to_string(),
                    session_id: session.id.clone(),
                    node_type: crate::web::workspace_ws_types::TimelineNodeType::AuthorRun,
                    status: crate::web::workspace_ws_types::TimelineNodeStatus::Completed,
                    agent_role: None,
                    provider: None,
                    prompt: None,
                    messages: Vec::new(),
                    streaming_content: String::new(),
                    execution_events: Vec::new(),
                    permission_events: Vec::new(),
                    verdict: None,
                    artifact_ref: Some(crate::product::models::ArtifactRef {
                        artifact_id: "artifact_version_002".to_string(),
                        version: 2,
                    }),
                    is_revision: false,
                    revision_feedback: None,
                    base_artifact_ref: None,
                    started_at: "2026-11-13T00:00:00Z".to_string(),
                    ended_at: Some("2026-11-13T00:00:01Z".to_string()),
                },
            )
            .expect("save node detail");

        let receipt_path = paths
            .issue_lifecycle_root("project_0001", "issue_0001")
            .join("workspace-timelines")
            .join(&session.id)
            .join("sc-failure-evidence")
            .join(&package.chain_id)
            .join("published")
            .join(format!("{}.json", package.package_id));
        let receipt = ScArtifactPublishedReceipt {
            schema_version: 1,
            package_id: package.package_id.clone(),
            chain_id: package.chain_id.clone(),
            node_id: "node_sc_1".to_string(),
            package_digest,
            version: 2,
        };

        // ① pending（缺发布回执）→ A4 版本在 REST 公开视图不可读。
        let pending = workspace_session_artifact_version(
            State(state.clone()),
            Path((session.id.clone(), 2)),
        )
        .await
        .expect_err("pending SC repair package must not be readable");
        assert_eq!(pending.code, "artifact_version_not_found");

        // ② 坏回执 → 显式 HTTP 错误（不吞成 404 not found）。
        std::fs::create_dir_all(receipt_path.parent().expect("receipt dir"))
            .expect("create receipt dir");
        std::fs::write(&receipt_path, b"{corrupt").expect("write corrupt receipt");
        let corrupt = workspace_session_artifact_version(
            State(state.clone()),
            Path((session.id.clone(), 2)),
        )
        .await
        .expect_err("corrupt receipt must surface an explicit error");
        assert_ne!(corrupt.code, "artifact_version_not_found");

        // ③ 真实回执 → A4 版本可读。
        std::fs::write(
            &receipt_path,
            serde_json::to_vec(&receipt).expect("receipt is serializable"),
        )
        .expect("write receipt");
        let Json(value) =
            workspace_session_artifact_version(State(state), Path((session.id.clone(), 2)))
                .await
                .expect("published SC repair package must be readable");
        assert_eq!(value["version"], 2);
    }
}
