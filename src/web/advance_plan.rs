//! P2 Task 1（tasks.md §3.1）：单一 `AdvancePlan` 共用薄入口。
//!
//! 人工 WS 与自动编排共用 `WorkspaceEngine::handle_advance` 的唯一业务实现
//! （journal/per-target 判据不在此重复）；本服务只做 origin 归属与自动路径
//! 的 enrollment/身份/单 target fail-closed 预检，不启动 provider、不调
//! StartCoding（`advance_completed` 仅状态）。人工 origin 不按 enrollment
//! 擅自升级；Enrolled origin 必须与 durable enrollment 的 id/revision/plan/
//! session 精确一致且 Confirmed+Completed SC 才放行 engine。

use crate::product::advance_store::{AdvanceInput, AdvanceOutcome};
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::issue_automation_store::IssueAutomationStore;
use crate::product::json_store::validate_relative_id;
use crate::product::models::automation::{EnrollmentBindingIdentity, IssueAutomationEnrollment};
use crate::product::models::{SingleCandidatePhase, WorkspaceSessionStatus, WorkspaceType};
use crate::product::work_item_revision_store::WorkItemRevisionStore;
use crate::web::state::WebAppState;
/// advance 请求归属：Manual 走人工 WS 语义；Enrolled 是自动编排的精确
/// enrollment 身份（id + policy_revision 快照），两者不得互相冒充。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdvancePlanOrigin {
    Manual,
    Enrolled {
        enrollment_id: String,
        policy_revision: u64,
    },
}

/// C1 Task 3：prepare/advance/start/reconcile 共用的当前 binding 读取/校验。
///
/// 从 durable enrollment 校验版本化绑定：缺失/禁用/内部投影不一致
///（enrollment id、plan/session 与 `binding_history.current` 漂移）一律
/// fail-closed；旧式 enrollment（无 binding）自动路径按 off/Manual 解释，
/// 绝不从 `logical_repository_id` 猜测 target。Manual 人工路径不经过本
/// helper，既有行为零回归。
pub(crate) fn load_current_enrollment_binding(
    enrollment: &IssueAutomationEnrollment,
) -> Result<EnrollmentBindingIdentity, String> {
    if !enrollment.enabled {
        return Err("advance enrollment is disabled".to_string());
    }
    let history = enrollment.binding_history.as_ref().ok_or_else(|| {
        "enrollment has no versioned binding; legacy enrollments cannot drive automatic \
         chains (re-enable with an explicit target or rebind)"
            .to_string()
    })?;
    let current = &history.current;
    if current.enrollment_id != enrollment.enrollment_id {
        return Err(format!(
            "binding enrollment id drift: binding {}, enrollment {}",
            current.enrollment_id, enrollment.enrollment_id
        ));
    }
    if current.plan_id != enrollment.plan_id.clone().unwrap_or_default()
        || current.session_id != enrollment.session_id.clone().unwrap_or_default()
    {
        return Err(format!(
            "binding plan/session drift: binding ({}, {}), enrollment ({:?}, {:?})",
            current.plan_id, current.session_id, enrollment.plan_id, enrollment.session_id
        ));
    }
    Ok(current.clone())
}

/// 共用 advance 薄服务：`input` 按值传入，`origin` 决定 enrollment 预检。
pub async fn advance_plan(
    state: &WebAppState,
    input: AdvanceInput,
    origin: AdvancePlanOrigin,
) -> Result<AdvanceOutcome, String> {
    validate_relative_id(&input.command_id)
        .map_err(|error| format!("invalid advance command id: {error}"))?;

    let paths = ProductAppPaths::new(state.workspace_root.join(".aria"));
    let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(paths.clone());

    // 解析 plan 对应的 WorkItemPlan 会话（单候选一 plan 一会话；歧义即
    // fail-closed，不猜）。
    let session_record = lifecycle
        .list_workspace_sessions(&input.project_id, &input.issue_id)
        .map_err(|error| format!("list plan workspace sessions failed: {error}"))?
        .into_iter()
        .filter(|record| {
            record.workspace_type == WorkspaceType::WorkItemPlan
                && record.entity_id == input.plan_id
        })
        .max_by(|left, right| left.created_at.cmp(&right.created_at))
        .ok_or_else(|| {
            format!(
                "no workspace session bound to plan {} for advance",
                input.plan_id
            )
        })?;

    // P2 Task 2：Enrolled origin 在精确核验后冻结 AutoStartOnce（enrollment
    // id/revision + plan revision id）；Manual 恒为 Manual，不按 enrollment
    // 擅自升级。
    let mut enrolled_start_policy: Option<crate::product::coding_models::CodingStartRunPolicy> =
        None;
    if let AdvancePlanOrigin::Enrolled {
        enrollment_id,
        policy_revision,
    } = &origin
    {
        // 自动身份精确复核：enabled + id/revision/plan/session 与 durable
        // enrollment 完全一致，固定 command_id（同 enrollment+plan）保证跨
        // 重启同一 durable replay 线。漂移即显式拒绝，不隐式新 command。
        let enrollment = IssueAutomationStore::new(paths.clone())
            .get(&input.project_id, &input.issue_id)
            .map_err(|error| format!("load enrollment for advance failed: {error}"))?
            .ok_or_else(|| "advance enrollment is missing".to_string())?;
        if !enrollment.enabled {
            return Err("advance enrollment is disabled".to_string());
        }
        if enrollment.enrollment_id != *enrollment_id {
            return Err(format!(
                "advance enrollment id mismatch: expected {enrollment_id}, actual {}",
                enrollment.enrollment_id
            ));
        }
        if enrollment.policy_revision != *policy_revision {
            return Err(format!(
                "advance enrollment policy revision mismatch: expected {policy_revision}, actual {}",
                enrollment.policy_revision
            ));
        }
        let bound_plan_id = enrollment
            .plan_id
            .as_deref()
            .ok_or_else(|| "advance enrollment has no bound plan".to_string())?;
        if bound_plan_id != input.plan_id {
            return Err(format!(
                "advance enrollment plan mismatch: expected plan {bound_plan_id}, got {}",
                input.plan_id
            ));
        }
        let bound_session_id = enrollment
            .session_id
            .as_deref()
            .ok_or_else(|| "advance enrollment has no bound session".to_string())?;
        if bound_session_id != session_record.id {
            return Err(format!(
                "advance enrollment session mismatch: expected {bound_session_id}, got {}",
                session_record.id
            ));
        }
        // C1 Task 3：当前 binding 前置——版本化绑定存在且与 enrollment 投影
        // 一致；v2 换代后旧代 plan/session 在此 fail-closed。
        let binding = load_current_enrollment_binding(&enrollment)?;
        if binding.plan_id != input.plan_id {
            return Err(format!(
                "advance enrollment plan mismatch: expected plan {}, got {}",
                binding.plan_id, input.plan_id
            ));
        }
        let expected_command_id = format!("wiga-advance-{enrollment_id}-{}", input.plan_id);
        if input.command_id != expected_command_id {
            return Err(format!(
                "enrolled advance command id must be the stable per-enrollment id \
                 {expected_command_id}"
            ));
        }
        // durable Confirmed + Completed SC 是自动 advance 的最低前置（成功
        // publication/compile 由该终态与 engine 的 compile/revision 检查共同
        // 保证）；未确认现场显式拒绝。
        if session_record.status != WorkspaceSessionStatus::Confirmed
            || session_record.single_candidate_phase != Some(SingleCandidatePhase::Completed)
        {
            return Err(format!(
                "enrolled advance requires confirmed single-candidate session, got status {:?} phase {:?}",
                session_record.status, session_record.single_candidate_phase
            ));
        }
        // C5 Task 4（决策 4 前半）：Ready/AutoStartOnce 写入前重读 issue 最新
        // authority 载体，与 binding target 漂移（错仓/跨载体）即 fail-closed
        // ——先比载体，后做单仓/LC group 判定，全部只读、全部在 attempt
        // 状态写入与 provider 启动之前。
        let carrier_issue = crate::product::issue_store::IssueStore::new(paths.clone())
            .get(&input.project_id, &input.issue_id)
            .map_err(|error| format!("load issue for enrolled advance failed: {error}"))?;
        let carrier = crate::web::handlers::resolve_automation_carrier(
            &paths,
            &input.project_id,
            &carrier_issue,
        )
        .map_err(|error| {
            format!(
                "resolve automation carrier for enrolled advance failed: {} [{}]",
                error.message, error.code
            )
        })?;
        let carrier_matches_binding = match (&carrier, &binding.target) {
            (
                crate::web::handlers::AutomationCarrierResolution::SingleRepository {
                    target:
                        crate::product::logical_codebase::EnrollmentTarget::SingleRepository {
                            repository_id: current_repository,
                        },
                },
                crate::product::logical_codebase::EnrollmentTarget::SingleRepository {
                    repository_id: bound_repository,
                },
            ) => current_repository == bound_repository,
            (
                crate::web::handlers::AutomationCarrierResolution::LogicalCodebase { .. },
                crate::product::logical_codebase::EnrollmentTarget::LogicalCodebase { .. },
            ) => true,
            _ => false,
        };
        if !carrier_matches_binding {
            return Err(format!(
                "enrolled advance carrier drift: binding target {:?} no longer matches the \
                 issue's authoritative automation carrier",
                binding.target
            ));
        }
        // 自动路径恰一 target fail-closed：engine 支持 split，但 enrolled 自动
        // 授权的 target 判定按 enrollment 载体分派（C5 Task 4，REQ-MTG-03
        // 单仓唯一例外）。
        let revision_store = WorkItemRevisionStore::new(paths.clone());
        let lineage = revision_store
            .get_plan_lineage(&input.project_id, &input.issue_id, &input.plan_id)
            .map_err(|error| format!("load plan lineage failed: {error}"))?;
        let active_revision_id = lineage
            .active_revision_id
            .ok_or_else(|| "plan has no active revision for enrolled advance".to_string())?;
        let authoritative = CodingAttemptStore::new(paths.clone())
            .resolve_authoritative_group_plan_binding_for_revision(
                &input.project_id,
                &input.issue_id,
                &input.plan_id,
                &active_revision_id,
            )
            .map_err(|error| format!("resolve authoritative group plan binding failed: {error}"))?;
        let grouped = crate::product::coding_attempt_store::units_by_target(&authoritative);
        match &binding.target {
            crate::product::logical_codebase::EnrollmentTarget::SingleRepository { .. } => {
                // 单仓分支：by_target 空 ∧ unattributed 非空 ∧ 现存 attempt 无
                // target_snapshot；出现任何 logical 归属或 snapshot 即跨载体
                // 污染（多 target 红线不被「单次只有一个」突破）。
                if !grouped.by_target.is_empty() || grouped.unattributed.is_empty() {
                    return Err(format!(
                        "single-repository enrolled advance requires every plan unit \
                         unattributed, got {} logical target(s) and {} unattributed unit(s)",
                        grouped.by_target.len(),
                        grouped.unattributed.len()
                    ));
                }
                let snapshot_pollution = CodingAttemptStore::new(paths.clone())
                    .list_attempts_for_issue(&input.project_id, &input.issue_id)
                    .map_err(|error| {
                        format!("list coding attempts for enrolled advance failed: {error}")
                    })?
                    .iter()
                    .any(|attempt| attempt.target_snapshot.is_some());
                if snapshot_pollution {
                    return Err(
                        "single-repository enrolled advance found an attempt carrying a \
                         logical target snapshot; cross-carrier pollution is rejected"
                            .to_string(),
                    );
                }
            }
            crate::product::logical_codebase::EnrollmentTarget::LogicalCodebase {
                logical_repository_id: binding_repository,
                ..
            } => {
                // LC 分支：恰一 logical target 且必须等于 enrollment binding
                // target（attempt 唯一 target 与 enrollment 不符即 fail-closed）。
                if grouped.by_target.len() != 1 || !grouped.unattributed.is_empty() {
                    return Err(format!(
                        "enrolled advance requires exactly one logical target, got {} \
                         target(s) and {} unattributed unit(s)",
                        grouped.by_target.len(),
                        grouped.unattributed.len()
                    ));
                }
                let unique_target = grouped
                    .by_target
                    .keys()
                    .next()
                    .expect("by_target is validated to hold exactly one entry");
                if unique_target != binding_repository {
                    return Err(format!(
                        "enrolled advance logical target drift: enrollment targets \
                         {binding_repository:?} but the plan uniquely targets {unique_target:?}"
                    ));
                }
            }
        }
        enrolled_start_policy = Some(
            crate::product::coding_models::CodingStartRunPolicy::AutoStartOnce {
                enrollment_id: enrollment_id.clone(),
                policy_revision: *policy_revision,
                source_plan_revision: authoritative.plan_revision_id.clone(),
            },
        );
    }

    // registry 命中：取得与 socket/编排器**同一** manager（生产路径）。
    // registry miss（隔离 fixture 等场景）：按 durable 记录构造裸 engine 执行
    // 同一段 `handle_advance`——不冷启 manager，避免 `ensure_workspace_context_message`
    // 重写会话消息的附带副作用；engine 侧身份校验兜底。
    let outcome = match state.workspace_sessions.peek(&session_record.id).await {
        Some(manager) => {
            let engine_arc = manager.engine();
            let mut engine = engine_arc.lock().await;
            match enrolled_start_policy {
                Some(policy) => {
                    engine
                        .handle_advance_with_start_policy(input, policy)
                        .await?
                }
                None => engine.handle_advance(input).await?,
            }
        }
        None => {
            let full_record = lifecycle
                .get_workspace_session(&session_record.id)
                .map_err(|error| format!("load workspace session failed: {error}"))?;
            let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
            let mut engine = crate::product::workspace_engine::WorkspaceEngine::new_persistent(
                std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
                    state.workspace_root.join("advance-engine-checkpoints"),
                )),
                lifecycle,
                event_tx,
                crate::product::workspace_engine::WorkspaceSession::from_record(full_record),
            );
            match enrolled_start_policy {
                Some(policy) => {
                    engine
                        .handle_advance_with_start_policy(input, policy)
                        .await?
                }
                None => engine.handle_advance(input).await?,
            }
        }
    };

    // 回放后仍核 target_attempts 不超过一条：engine 支持 split，enrolled
    // 自动路径不允许出现多 target 绑定结果。
    if let AdvancePlanOrigin::Enrolled { .. } = origin
        && let AdvanceOutcome::Replayed { record } = &outcome
        && record.target_attempts.len() > 1
    {
        return Err(format!(
            "enrolled advance replay produced {} target attempts; expected at most one",
            record.target_attempts.len()
        ));
    }
    Ok(outcome)
}

/// C1 Task 7（REQ-ADV-C1-RETRY）：Failed advance 显式 retry REST 薄入口。
/// 解析 plan 会话后调 `WorkspaceEngine::retry_initialization`（身份链/
/// 副作用门/续做全在 engine 服务内）；响应体固定为
/// `RetryInitializationResult`（`NeedsHuman` 由 `state` 承载）。
pub async fn post_work_item_plan_advance_retry_initialization(
    axum::extract::State(state): axum::extract::State<WebAppState>,
    axum::extract::Path((project_id, issue_id, plan_id)): axum::extract::Path<(
        String,
        String,
        String,
    )>,
    axum::Json(request): axum::Json<crate::product::models::automation::RetryInitializationRequest>,
) -> Result<
    axum::response::Json<crate::product::models::automation::RetryInitializationResult>,
    crate::web::error::ApiError,
> {
    use crate::web::error::ApiError;

    for id in [&project_id, &issue_id, &plan_id, &request.command_id] {
        validate_relative_id(id).map_err(|error| {
            ApiError::validation(
                "retry_initialization_invalid_id",
                format!("invalid id: {error}"),
            )
        })?;
    }
    let paths = ProductAppPaths::new(state.workspace_root.join(".aria"));
    let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(paths.clone());
    let session_record = lifecycle
        .list_workspace_sessions(&project_id, &issue_id)
        .map_err(|error| {
            ApiError::runtime(
                "retry_initialization_session_lookup_failed",
                format!("list plan workspace sessions failed: {error}"),
                serde_json::json!({}),
            )
        })?
        .into_iter()
        .find(|record| {
            record.workspace_type == WorkspaceType::WorkItemPlan
                && record.entity_id == plan_id
                && record.project_id == project_id
                && record.issue_id == issue_id
        })
        .ok_or_else(|| {
            ApiError::runtime(
                "retry_initialization_session_not_found",
                format!("no workspace session bound to plan {plan_id}"),
                serde_json::json!({}),
            )
        })?;

    let retry = async {
        match state.workspace_sessions.peek(&session_record.id).await {
            Some(manager) => {
                let engine_arc = manager.engine();
                let mut engine = engine_arc.lock().await;
                engine.retry_initialization(&request).await
            }
            None => {
                let full_record = lifecycle
                    .get_workspace_session(&session_record.id)
                    .map_err(|error| format!("load workspace session failed: {error}"))?;
                let (event_tx, _event_rx) = tokio::sync::mpsc::channel(8);
                let mut engine = crate::product::workspace_engine::WorkspaceEngine::new_persistent(
                    std::sync::Arc::new(crate::product::checkpoint_store::CheckpointStore::new(
                        state.workspace_root.join("advance-engine-checkpoints"),
                    )),
                    lifecycle,
                    event_tx,
                    crate::product::workspace_engine::WorkspaceSession::from_record(full_record),
                );
                engine.retry_initialization(&request).await
            }
        }
    }
    .await
    .map_err(|error| {
        ApiError::runtime(
            "retry_initialization_rejected",
            error,
            serde_json::json!({}),
        )
    })?;
    Ok(axum::response::Json(retry))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::advance_store::AdvanceOutcome;
    use crate::web::wiga_gate_fixture::{
        ISSUE_ID, PROJECT_ID, confirmed_enrolled_fixture,
        confirmed_single_repository_enrolled_fixture,
    };

    /// P2 Task 1：Enrolled origin 经真实 Confirmed fixture advance 到稳定
    /// attempt——同 command_id 二次调用是 durable replay（同 attempt），不启
    /// 动任何 coding runner。
    #[tokio::test]
    async fn enrolled_advance_replays_one_ready_attempt_without_starting_runner() {
        let fixture = confirmed_enrolled_fixture().await;
        let enrollment = fixture.enrollment();
        assert_eq!(enrollment.project_id, PROJECT_ID);
        assert_eq!(enrollment.issue_id, ISSUE_ID);
        let input = AdvanceInput {
            command_id: format!(
                "wiga-advance-{}-{}",
                enrollment.enrollment_id,
                enrollment.plan_id.as_deref().unwrap()
            ),
            project_id: enrollment.project_id.clone(),
            issue_id: enrollment.issue_id.clone(),
            plan_id: enrollment.plan_id.clone().unwrap(),
        };
        let origin = AdvancePlanOrigin::Enrolled {
            enrollment_id: enrollment.enrollment_id.clone(),
            policy_revision: enrollment.policy_revision,
        };
        let first = advance_plan(&fixture.state, input.clone(), origin.clone())
            .await
            .unwrap();
        let second = advance_plan(&fixture.state, input, origin).await.unwrap();
        let first_id = match &first {
            AdvanceOutcome::Completed { attempt_id, .. } => attempt_id.as_str(),
            AdvanceOutcome::Replayed { record } => record.attempt_id.as_deref().unwrap(),
            AdvanceOutcome::Rejected { code, .. } => panic!("unexpected reject: {code}"),
        };
        let second_id = match &second {
            AdvanceOutcome::Completed { attempt_id, .. } => attempt_id.as_str(),
            AdvanceOutcome::Replayed { record } => record.attempt_id.as_deref().unwrap(),
            AdvanceOutcome::Rejected { code, .. } => panic!("unexpected reject: {code}"),
        };
        assert_eq!(first_id, second_id);
        assert_eq!(fixture.coding_attempts().len(), 1);
        assert_eq!(fixture.coding_runner_count(), 0);

        // P2 Task 2：enrolled advance 在新 journal attempt 上冻结
        // AutoStartOnce（enrollment id/revision + plan revision id，非可变
        // latest ref）；disable 后 replay 不洗白冻结 policy。
        let expected_revision = match &first {
            AdvanceOutcome::Completed { record, .. } => record.plan_revision_id.clone(),
            AdvanceOutcome::Replayed { record } => record.plan_revision_id.clone(),
            AdvanceOutcome::Rejected { .. } => unreachable!("asserted non-rejected above"),
        };
        let frozen = fixture
            .coding_attempts()
            .into_iter()
            .find(|attempt| attempt.id == first_id)
            .unwrap();
        assert_eq!(
            frozen.start_run_policy,
            crate::product::coding_models::CodingStartRunPolicy::AutoStartOnce {
                enrollment_id: enrollment.enrollment_id.clone(),
                policy_revision: enrollment.policy_revision,
                source_plan_revision: expected_revision,
            }
        );
        // Disable→replay：journal 已存在，policy 保持冻结不被洗白。
        let store = IssueAutomationStore::new(ProductAppPaths::new(
            fixture.state.workspace_root.join(".aria"),
        ));
        store
            .compare_and_set(
                &enrollment.project_id,
                &enrollment.issue_id,
                Some(enrollment.policy_revision),
                crate::product::models::automation::EnrollmentWriteCommand::Disable,
            )
            .expect("disable enrollment");
        let replayed = advance_plan(
            &fixture.state,
            AdvanceInput {
                command_id: format!(
                    "wiga-advance-{}-{}",
                    enrollment.enrollment_id,
                    enrollment.plan_id.as_deref().unwrap()
                ),
                project_id: enrollment.project_id.clone(),
                issue_id: enrollment.issue_id.clone(),
                plan_id: enrollment.plan_id.clone().unwrap(),
            },
            AdvancePlanOrigin::Enrolled {
                enrollment_id: enrollment.enrollment_id.clone(),
                policy_revision: enrollment.policy_revision,
            },
        )
        .await;
        // Disable 后 Enrolled origin 在服务层显式拒绝（不隐式 replay）。
        assert!(replayed.is_err());
        let still_frozen = fixture
            .coding_attempts()
            .into_iter()
            .find(|attempt| attempt.id == first_id)
            .unwrap();
        assert_eq!(still_frozen.start_run_policy, frozen.start_run_policy);
    }

    /// C1 Task 3：binding v1→v2 换代后，自动链每次重读当前代；v1 的
    /// generation/advance/start 回执全部 fail-closed，无第二 attempt/provider；
    /// 当前 binding 对应的唯一 plan/session 才可继续；Manual 不被 enrollment
    /// 校验拦截（REQ-WIGA-03/04、REQ-C1-TARGET-01）。
    #[tokio::test]
    async fn enrolled_advance_reloads_current_binding_and_rejects_old_generation() {
        use crate::product::models::automation::{
            EnrollmentBindingIdentityInput, EnrollmentRebindRequest,
        };
        use crate::product::work_item_plan_policy::RunPolicy;
        use crate::web::handlers::automation_enrollment_test_support::create_plan_and_session;

        let fixture = confirmed_enrolled_fixture().await;
        let enrollment = fixture.enrollment();
        let plan_1 = enrollment.plan_id.clone().expect("bound plan");
        let binding_v1 = enrollment
            .binding_history
            .as_ref()
            .expect("versioned binding v1")
            .current
            .clone();
        assert_eq!(binding_v1.binding_version, 1);

        // v1 advance 成功：journal 冻结 v1 身份（唯一 attempt）。
        let v1_advance = advance_plan(
            &fixture.state,
            AdvanceInput {
                command_id: format!("wiga-advance-{}-{plan_1}", enrollment.enrollment_id),
                project_id: enrollment.project_id.clone(),
                issue_id: enrollment.issue_id.clone(),
                plan_id: plan_1.clone(),
            },
            AdvancePlanOrigin::Enrolled {
                enrollment_id: enrollment.enrollment_id.clone(),
                policy_revision: enrollment.policy_revision,
            },
        )
        .await
        .expect("v1 advance under current binding");
        assert!(matches!(v1_advance, AdvanceOutcome::Completed { .. }));
        assert_eq!(fixture.coding_attempts().len(), 1);

        // 显式 rebind 制造 v2：新 plan/session 属于同一 issue。
        let (plan_2, _session_2) = create_plan_and_session(&fixture.inner, RunPolicy::Interactive);
        let store = IssueAutomationStore::new(ProductAppPaths::new(
            fixture.state.workspace_root.join(".aria"),
        ));
        let rebind = store
            .rebind(
                &enrollment.project_id,
                &enrollment.issue_id,
                EnrollmentRebindRequest {
                    command_id: "rebind-c1-task3".to_string(),
                    expected_policy_revision: enrollment.policy_revision,
                    expected_binding_version: binding_v1.binding_version,
                    binding: EnrollmentBindingIdentityInput {
                        plan_id: plan_2.clone(),
                        session_id: _session_2,
                        source: enrollment.source.clone(),
                        target: binding_v1.target.clone(),
                        author_provider: enrollment.options.author_provider.clone(),
                        reviewer_provider: enrollment.options.reviewer_provider.clone(),
                    },
                    reason: "c1 task3 generation switch".to_string(),
                },
            )
            .expect("rebind to v2");
        let after = rebind.enrollment;
        assert_eq!(
            after
                .binding_history
                .as_ref()
                .unwrap()
                .current
                .binding_version,
            2
        );

        // v1 advance 回执（旧 revision + 旧 plan）→ 身份 fail-closed。
        let v1_receipt = advance_plan(
            &fixture.state,
            AdvanceInput {
                command_id: format!("wiga-advance-{}-{plan_1}", enrollment.enrollment_id),
                project_id: enrollment.project_id.clone(),
                issue_id: enrollment.issue_id.clone(),
                plan_id: plan_1.clone(),
            },
            AdvancePlanOrigin::Enrolled {
                enrollment_id: enrollment.enrollment_id.clone(),
                policy_revision: enrollment.policy_revision,
            },
        )
        .await
        .unwrap_err();
        assert!(
            v1_receipt.contains("mismatch"),
            "v1 advance receipt must fail closed on identity, got: {v1_receipt}"
        );
        assert_eq!(fixture.coding_attempts().len(), 1);

        // v1 start 回执（旧 origin 身份）→ frozen/current 互证 fail-closed，
        // 零 runner。
        let v1_attempt = fixture.attempt();
        let v1_start = crate::web::coding_start::start_coding_once(
            &fixture.state,
            &enrollment.project_id,
            &enrollment.issue_id,
            crate::web::coding_start::StartCodingCommand {
                attempt_id: v1_attempt.id.clone(),
                command_id: "wiga-start-v1-receipt".to_string(),
                origin: crate::product::coding_models::CodingStartOrigin::Enrolled {
                    enrollment_id: enrollment.enrollment_id.clone(),
                    policy_revision: enrollment.policy_revision,
                    binding_version: Some(binding_v1.binding_version),
                    target: Some(binding_v1.target.clone()),
                },
            },
        )
        .await;
        assert!(v1_start.is_err(), "v1 start receipt must fail closed");
        assert_eq!(fixture.coding_runner_count(), 0);
        assert_eq!(fixture.coding_attempts().len(), 1);

        // v2 当前代：唯一可继续的 plan/session 是 binding current（plan_2）；
        // 身份通过、停在既有门（session_2 未 Confirmed）。
        let v2_receipt = advance_plan(
            &fixture.state,
            AdvanceInput {
                command_id: format!("wiga-advance-{}-{plan_2}", enrollment.enrollment_id),
                project_id: enrollment.project_id.clone(),
                issue_id: enrollment.issue_id.clone(),
                plan_id: plan_2.clone(),
            },
            AdvancePlanOrigin::Enrolled {
                enrollment_id: enrollment.enrollment_id.clone(),
                policy_revision: after.policy_revision,
            },
        )
        .await
        .unwrap_err();
        assert!(
            v2_receipt.contains("confirmed single-candidate"),
            "v2 advance must pass identity and stop at the existing gate, got: {v2_receipt}"
        );
        assert_eq!(fixture.coding_attempts().len(), 1);

        // 旧式 enrollment（无版本化 binding）自动路径 fail-closed：剥离
        // binding 后同一 v2 身份也被拒。
        let mut legacy = after.clone();
        legacy.binding_history = None;
        // 直接以 durable 文件模拟旧数据：写入剥离 binding 的 enrollment。
        crate::product::json_store::write_json(
            &ProductAppPaths::new(fixture.state.workspace_root.join(".aria"))
                .issue_root(&enrollment.project_id, &enrollment.issue_id)
                .join("automation-enrollment.json"),
            &legacy,
        )
        .unwrap();
        let legacy_error = advance_plan(
            &fixture.state,
            AdvanceInput {
                command_id: format!("wiga-advance-{}-{plan_2}", enrollment.enrollment_id),
                project_id: enrollment.project_id.clone(),
                issue_id: enrollment.issue_id.clone(),
                plan_id: plan_2.clone(),
            },
            AdvancePlanOrigin::Enrolled {
                enrollment_id: enrollment.enrollment_id.clone(),
                policy_revision: after.policy_revision,
            },
        )
        .await
        .unwrap_err();
        assert!(
            legacy_error.contains("versioned binding"),
            "legacy enrollment without binding must fail closed, got: {legacy_error}"
        );

        // Manual 不被 enrollment 校验拦截：v1 plan 的 journal durable replay
        // 返回同一 attempt，不创建第二 attempt。
        let manual = advance_plan(
            &fixture.state,
            AdvanceInput {
                command_id: format!("wiga-advance-{}-{plan_1}", enrollment.enrollment_id),
                project_id: enrollment.project_id.clone(),
                issue_id: enrollment.issue_id.clone(),
                plan_id: plan_1.clone(),
            },
            AdvancePlanOrigin::Manual,
        )
        .await
        .expect("manual advance must not be blocked by enrollment binding checks");
        assert!(matches!(manual, AdvanceOutcome::Replayed { .. }));
        assert_eq!(fixture.coding_attempts().len(), 1);
        assert_eq!(fixture.coding_runner_count(), 0);
    }

    /// P2 Task 2：手工（Manual）advance 的新 journal attempt 显式 Manual；
    /// 已冻结的 AutoStartOnce 不被后续手工命令偷换。
    #[tokio::test]
    async fn manual_advance_freezes_manual_policy_on_fresh_journal() {
        let fixture = confirmed_enrolled_fixture().await;
        let enrollment = fixture.enrollment();
        let manual = advance_plan(
            &fixture.state,
            AdvanceInput {
                command_id: "manual-advance-fresh-1".to_string(),
                project_id: enrollment.project_id.clone(),
                issue_id: enrollment.issue_id.clone(),
                plan_id: enrollment.plan_id.clone().unwrap(),
            },
            AdvancePlanOrigin::Manual,
        )
        .await
        .unwrap();
        let attempt_id = match &manual {
            AdvanceOutcome::Completed { attempt_id, .. } => attempt_id.clone(),
            AdvanceOutcome::Replayed { record } => {
                record.attempt_id.clone().expect("replayed attempt id")
            }
            AdvanceOutcome::Rejected { code, .. } => panic!("unexpected reject: {code}"),
        };
        let attempt = fixture
            .coding_attempts()
            .into_iter()
            .find(|attempt| attempt.id == attempt_id)
            .unwrap();
        assert_eq!(
            attempt.start_run_policy,
            crate::product::coding_models::CodingStartRunPolicy::Manual
        );
    }

    /// 自动身份漂移（policy_revision/enrollment id/command id）与禁用 enrollment
    /// 都必须在 engine 前显式拒绝：零 coding attempt、零 runner。
    #[tokio::test]
    async fn enrolled_advance_rejects_identity_drift_and_disabled_enrollment() {
        let fixture = confirmed_enrolled_fixture().await;
        let enrollment = fixture.enrollment();
        let base_input = AdvanceInput {
            command_id: format!(
                "wiga-advance-{}-{}",
                enrollment.enrollment_id,
                enrollment.plan_id.as_deref().unwrap()
            ),
            project_id: enrollment.project_id.clone(),
            issue_id: enrollment.issue_id.clone(),
            plan_id: enrollment.plan_id.clone().unwrap(),
        };
        let drifted_revision = AdvancePlanOrigin::Enrolled {
            enrollment_id: enrollment.enrollment_id.clone(),
            policy_revision: enrollment.policy_revision + 1,
        };
        let error = advance_plan(&fixture.state, base_input.clone(), drifted_revision)
            .await
            .unwrap_err();
        assert!(error.contains("policy revision mismatch"), "{error}");
        assert_eq!(fixture.coding_attempts().len(), 0);

        let wrong_command = AdvanceInput {
            command_id: "wiga-advance-adhoc-command".to_string(),
            ..base_input.clone()
        };
        let error = advance_plan(
            &fixture.state,
            wrong_command,
            AdvancePlanOrigin::Enrolled {
                enrollment_id: enrollment.enrollment_id.clone(),
                policy_revision: enrollment.policy_revision,
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("stable per-enrollment id"), "{error}");
        assert_eq!(fixture.coding_attempts().len(), 0);

        // Disable 后旧 origin 不得再 advance（D2：禁用先胜未消费即失效）。
        let store = IssueAutomationStore::new(crate::product::app_paths::ProductAppPaths::new(
            fixture.state.workspace_root.join(".aria"),
        ));
        store
            .compare_and_set(
                &enrollment.project_id,
                &enrollment.issue_id,
                Some(enrollment.policy_revision),
                crate::product::models::automation::EnrollmentWriteCommand::Disable,
            )
            .expect("disable enrollment");
        let error = advance_plan(
            &fixture.state,
            base_input,
            AdvancePlanOrigin::Enrolled {
                enrollment_id: enrollment.enrollment_id.clone(),
                policy_revision: enrollment.policy_revision,
            },
        )
        .await
        .unwrap_err();
        assert!(error.contains("disabled"), "{error}");
        assert_eq!(fixture.coding_attempts().len(), 0);
        assert_eq!(fixture.coding_runner_count(), 0);
    }

    /// Manual origin 走同一 engine 实现：enrolled advance 已存在的 plan 级
    /// record 对人工命令按 durable replay 返回同一 attempt（人工不升级、
    /// 不隐式新 command 偷跑第二个 attempt）。
    #[tokio::test]
    async fn manual_advance_replays_same_plan_attempt_after_enrolled_advance() {
        let fixture = confirmed_enrolled_fixture().await;
        let enrollment = fixture.enrollment();
        let enrolled_input = AdvanceInput {
            command_id: format!(
                "wiga-advance-{}-{}",
                enrollment.enrollment_id,
                enrollment.plan_id.as_deref().unwrap()
            ),
            project_id: enrollment.project_id.clone(),
            issue_id: enrollment.issue_id.clone(),
            plan_id: enrollment.plan_id.clone().unwrap(),
        };
        let enrolled_origin = AdvancePlanOrigin::Enrolled {
            enrollment_id: enrollment.enrollment_id.clone(),
            policy_revision: enrollment.policy_revision,
        };
        let first = advance_plan(&fixture.state, enrolled_input.clone(), enrolled_origin)
            .await
            .unwrap();
        let first_id = match &first {
            AdvanceOutcome::Completed { attempt_id, .. } => attempt_id.clone(),
            AdvanceOutcome::Replayed { record } => {
                record.attempt_id.clone().expect("replayed attempt id")
            }
            AdvanceOutcome::Rejected { code, .. } => panic!("unexpected reject: {code}"),
        };

        let manual = advance_plan(
            &fixture.state,
            AdvanceInput {
                command_id: "manual-advance-command-1".to_string(),
                ..enrolled_input
            },
            AdvancePlanOrigin::Manual,
        )
        .await
        .unwrap();
        match &manual {
            AdvanceOutcome::Completed { attempt_id, .. } => assert_eq!(*attempt_id, first_id),
            AdvanceOutcome::Replayed { record } => {
                assert_eq!(record.attempt_id.as_deref(), Some(first_id.as_str()))
            }
            AdvanceOutcome::Rejected { code, reason, .. } => {
                panic!("unexpected manual reject: {code} {reason}")
            }
        }
        assert_eq!(fixture.coding_attempts().len(), 1);
        assert_eq!(fixture.coding_runner_count(), 0);
    }

    /// C5 Task 4 红测：单仓 enrollment（binding.target 为 SingleRepository）的
    /// Confirmed plan——全部工作项无 logical target 归属、attempt 无
    /// target_snapshot——经 enrolled advance 沿单仓路径把唯一 attempt 置
    /// Ready 并冻结 AutoStartOnce；不要求 logical target、不启动任何
    /// runner/provider、不创建 LC selection/snapshot（REQ-ADV-05）。
    #[tokio::test]
    async fn enrolled_advance_single_repository_plan_reaches_ready() {
        let fixture = confirmed_single_repository_enrolled_fixture().await;
        let enrollment = fixture.enrollment();
        assert!(
            matches!(
                enrollment.target.as_ref(),
                Some(crate::product::logical_codebase::EnrollmentTarget::SingleRepository { .. })
            ),
            "fixture must enroll a single-repository target, got {:?}",
            enrollment.target
        );
        let input = AdvanceInput {
            command_id: format!(
                "wiga-advance-{}-{}",
                enrollment.enrollment_id,
                enrollment.plan_id.as_deref().unwrap()
            ),
            project_id: enrollment.project_id.clone(),
            issue_id: enrollment.issue_id.clone(),
            plan_id: enrollment.plan_id.clone().unwrap(),
        };
        let origin = AdvancePlanOrigin::Enrolled {
            enrollment_id: enrollment.enrollment_id.clone(),
            policy_revision: enrollment.policy_revision,
        };
        let first = advance_plan(&fixture.state, input.clone(), origin.clone())
            .await
            .expect("single-repository enrolled advance must reach ready");
        let (first_id, expected_revision) = match &first {
            AdvanceOutcome::Completed {
                attempt_id, record, ..
            } => (attempt_id.clone(), record.plan_revision_id.clone()),
            AdvanceOutcome::Replayed { record } => (
                record.attempt_id.clone().expect("replayed attempt id"),
                record.plan_revision_id.clone(),
            ),
            AdvanceOutcome::Rejected { code, reason, .. } => {
                panic!("unexpected reject: {code} {reason}")
            }
        };

        // 同 command 幂等 replay：同一 attempt，不产生第二身份。
        let second = advance_plan(&fixture.state, input, origin).await.unwrap();
        let second_id = match &second {
            AdvanceOutcome::Completed { attempt_id, .. } => attempt_id.clone(),
            AdvanceOutcome::Replayed { record } => {
                record.attempt_id.clone().expect("replayed attempt id")
            }
            AdvanceOutcome::Rejected { code, reason, .. } => {
                panic!("unexpected replay reject: {code} {reason}")
            }
        };
        assert_eq!(first_id, second_id);
        assert_eq!(fixture.coding_attempts().len(), 1);
        assert_eq!(fixture.coding_runner_count(), 0);

        let attempt = fixture.attempt();
        // 单仓链红线：attempt 绝不携带 logical target snapshot。
        assert!(
            attempt.target_snapshot.is_none(),
            "single-repository attempt must not carry a logical target snapshot"
        );
        assert_eq!(
            attempt.start_run_policy,
            crate::product::coding_models::CodingStartRunPolicy::AutoStartOnce {
                enrollment_id: enrollment.enrollment_id.clone(),
                policy_revision: enrollment.policy_revision,
                source_plan_revision: expected_revision,
            }
        );
    }

    /// C5 Task 4：LC enrollment 的 plan 唯一 logical target 与 enrollment
    /// target 不同 → fail-closed 拒绝（补齐的核对），零 attempt、零 provider。
    #[tokio::test]
    async fn enrolled_advance_rejects_plan_target_drift_from_enrollment() {
        let fixture = confirmed_enrolled_fixture().await;
        let enrollment = fixture.enrollment();
        let plan_id = enrollment.plan_id.clone().expect("bound plan");
        // 制造 durable 漂移：编译产物的 accepted draft target 改指另一 logical 仓
        //（binding target 仍为原 logical repository）。
        let drifted = crate::product::logical_codebase::LogicalRepositoryId(uuid::Uuid::new_v4());
        let plan_store = crate::product::work_item_plan_store::WorkItemPlanStore::new(
            ProductAppPaths::new(fixture.state.workspace_root.join(".aria")),
        );
        for draft in plan_store
            .list_draft_records(&enrollment.project_id, &enrollment.issue_id, &plan_id)
            .expect("list compiled drafts")
        {
            let mut patched = draft.clone();
            patched.candidate.target_repository_id = Some(drifted);
            plan_store.put_draft_record(&patched).expect("patch draft");
        }

        let error = advance_plan(
            &fixture.state,
            AdvanceInput {
                command_id: format!("wiga-advance-{}-{plan_id}", enrollment.enrollment_id),
                project_id: enrollment.project_id.clone(),
                issue_id: enrollment.issue_id.clone(),
                plan_id: plan_id.clone(),
            },
            AdvancePlanOrigin::Enrolled {
                enrollment_id: enrollment.enrollment_id.clone(),
                policy_revision: enrollment.policy_revision,
            },
        )
        .await
        .unwrap_err();
        assert!(
            error.contains("logical target drift") || error.contains("exactly one logical target"),
            "unexpected rejection reason: {error}"
        );
        assert_eq!(fixture.coding_attempts().len(), 0);
        assert_eq!(fixture.coding_runner_count(), 0);
    }

    /// C5 Task 4（Review Focus 3）：单仓 enrollment 的 plan 出现任一 logical
    /// target 归属、或现存 attempt 携带 target_snapshot → 跨载体污染拒绝
    ///（多 target 红线不被「单次只有一个」突破），零 attempt 写入、零启动。
    #[tokio::test]
    async fn enrolled_advance_rejects_cross_carrier_pollution_under_single_repository() {
        let fixture = confirmed_single_repository_enrolled_fixture().await;
        let enrollment = fixture.enrollment();
        let plan_id = enrollment.plan_id.clone().expect("bound plan");
        let polluted = crate::product::logical_codebase::LogicalRepositoryId(uuid::Uuid::new_v4());
        let plan_store = crate::product::work_item_plan_store::WorkItemPlanStore::new(
            ProductAppPaths::new(fixture.state.workspace_root.join(".aria")),
        );
        for draft in plan_store
            .list_draft_records(&enrollment.project_id, &enrollment.issue_id, &plan_id)
            .expect("list compiled drafts")
        {
            let mut patched = draft.clone();
            patched.candidate.target_repository_id = Some(polluted);
            plan_store.put_draft_record(&patched).expect("patch draft");
        }

        let origin = AdvancePlanOrigin::Enrolled {
            enrollment_id: enrollment.enrollment_id.clone(),
            policy_revision: enrollment.policy_revision,
        };
        let input = AdvanceInput {
            command_id: format!("wiga-advance-{}-{plan_id}", enrollment.enrollment_id),
            project_id: enrollment.project_id.clone(),
            issue_id: enrollment.issue_id.clone(),
            plan_id: plan_id.clone(),
        };
        let error = advance_plan(&fixture.state, input.clone(), origin.clone())
            .await
            .unwrap_err();
        assert!(
            error.contains("single-repository"),
            "cross-carrier pollution must be rejected by the single-repository branch: {error}"
        );
        assert_eq!(fixture.coding_attempts().len(), 0);
        assert_eq!(fixture.coding_runner_count(), 0);

        // 还原 draft 归属后正常 advance 到 Ready，再把 attempt 污染为携带
        // LC target snapshot：replay 路径同样 fail-closed。
        for draft in plan_store
            .list_draft_records(&enrollment.project_id, &enrollment.issue_id, &plan_id)
            .expect("list compiled drafts again")
        {
            let mut patched = draft.clone();
            patched.candidate.target_repository_id = None;
            plan_store
                .put_draft_record(&patched)
                .expect("restore draft");
        }
        advance_plan(&fixture.state, input.clone(), origin.clone())
            .await
            .expect("clean single-repository advance reaches ready");
        let attempt = fixture.attempt();
        let attempt_path = fixture
            .inner
            .paths
            .issue_root(&enrollment.project_id, &enrollment.issue_id)
            .join("coding-attempts")
            .join(format!("{}.json", attempt.id));
        let mut attempt_json: serde_json::Value =
            crate::product::json_store::read_json(&attempt_path).expect("read attempt json");
        attempt_json["target_snapshot"] = serde_json::json!({
            "logical_repository_id": uuid::Uuid::new_v4().to_string(),
            "checkout_id": uuid::Uuid::new_v4().to_string(),
            "physical_repository_id": "physical-polluted".to_string(),
            "canonical_path": "/tmp/polluted",
            "git_dir_identity": "sha256:polluted".to_string(),
            "policy_digest": "polluted".to_string(),
            "membership_revision": 1,
            "captured_at": "2026-09-30T00:00:00Z".to_string(),
            "capture_source": "test".to_string(),
        });
        crate::product::json_store::write_json(&attempt_path, &attempt_json)
            .expect("pollute attempt snapshot");

        let error = advance_plan(&fixture.state, input, origin)
            .await
            .unwrap_err();
        assert!(
            error.contains("cross-carrier pollution") || error.contains("target snapshot"),
            "snapshot pollution must fail closed: {error}"
        );
        assert_eq!(fixture.coding_runner_count(), 0);
    }
}
