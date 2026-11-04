//! plan 停点分类与「继续自动化」入口可用性（**单一事实源**）。
//!
//! HTTP `GET .../automation-enrollment/control` 的 `allowed_actions` 与 WS
//! `SessionState.plan_continue_available` **同源**：两面共用本模块的
//! [`plan_control_facts`] 派生，前端不必猜「本页底部到底有没有『继续
//! 自动化』入口」——服务端说没有，前端就不给假指引。
//!
//! 停点分类（[`PlanStop`]）、stop_key 派生（[`plan_stop_key`]）与
//! `serde_stage` 从 `src/web/handlers/workspace_sc_recovery.rs` **纯移动**
//! 而来（零行为变化），只为让 control 与 WS 两面共用同一份判定。

use crate::product::app_paths::ProductAppPaths;
use crate::product::issue_automation_store::IssueAutomationStore;
use crate::product::models::automation::{
    EnrollmentBindingIdentity, IssueAutomationEnrollment, PlanGenerationPhase,
};
use crate::product::models::{SingleCandidatePhase, WorkspaceSessionStatus, WorkspaceType};

/// control 投影的「继续自动化」动作名（HTTP `allowed_actions` 与 WS 谓词
/// 共用的稳定字面量）。
pub(crate) const CONTINUE_PLAN_ACTION: &str = "continue_plan";

/// durable 事实派生的 plan 停点分类：continue-plan 按此分支，stop_key 按
/// 同一分类派生（continue 与 control 同源，防把旧说明送给新停点）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PlanStop {
    /// 会话 durable Failed（SC phase Failed；provider 失败收敛）。
    FailedProvider,
    /// 评审超限/人工停点（StoppedNeedsHuman + resumable 门快照）。
    StoppedHumanGate,
    /// 生成检查点派发前失败（intent NeedsHuman、无活 run）。
    GenerationNeedsHuman,
    /// 既有 choice/门等待（照旧走既有端点，continue 不受理）。
    WaitingExisting,
    /// 无可继续停点（活跃/终态/未停）。
    None,
}

/// 由 durable 会话记录 + 生成检查点阶段派生停点分类。
pub(crate) fn resolve_plan_stop(
    record: &crate::product::models::WorkspaceSessionRecord,
    intent_phase: Option<PlanGenerationPhase>,
) -> PlanStop {
    if record.workspace_type != WorkspaceType::WorkItemPlan {
        return PlanStop::None;
    }
    match record.status {
        WorkspaceSessionStatus::Failed => {
            if record.single_candidate_phase == Some(SingleCandidatePhase::Failed) {
                PlanStop::FailedProvider
            } else {
                PlanStop::None
            }
        }
        WorkspaceSessionStatus::StoppedNeedsHuman => {
            if record
                .human_gate_snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.resumable)
            {
                PlanStop::StoppedHumanGate
            } else {
                PlanStop::None
            }
        }
        WorkspaceSessionStatus::WaitingForHuman | WorkspaceSessionStatus::ChangeRequested => {
            PlanStop::WaitingExisting
        }
        _ => {
            if intent_phase == Some(PlanGenerationPhase::NeedsHuman) {
                PlanStop::GenerationNeedsHuman
            } else {
                PlanStop::None
            }
        }
    }
}

/// 停点对应的 durable 事实键（continue 与 control 同源）。
pub(crate) fn plan_stop_key(
    stop: &PlanStop,
    enrollment_id: &str,
    binding: &EnrollmentBindingIdentity,
) -> Option<String> {
    match stop {
        PlanStop::FailedProvider => Some(crate::web::plan_generation::plan_failed_node_fact_key(
            &binding.session_id,
        )),
        PlanStop::StoppedHumanGate => Some(crate::web::plan_generation::plan_gate_fact_key(
            &binding.session_id,
        )),
        PlanStop::GenerationNeedsHuman => Some(
            crate::web::plan_generation::plan_generation_failed_fact_key(
                enrollment_id,
                &binding.plan_id,
            ),
        ),
        PlanStop::WaitingExisting | PlanStop::None => None,
    }
}

/// control 投影的 durable 事实快照（HTTP control 与 WS 谓词共用）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlanControlFacts {
    pub(crate) managed: bool,
    pub(crate) binding: Option<EnrollmentBindingIdentity>,
    pub(crate) plan_id: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) terminated_at: Option<String>,
    pub(crate) stop_key: Option<String>,
    pub(crate) stage: String,
    pub(crate) allowed_actions: Vec<String>,
    pub(crate) context_required: bool,
}

impl PlanControlFacts {
    /// 无自动化归属的投影（无 enrollment / 未启用 / 绑定不可用 / 绑定会话
    /// 与投影会话错绑）——一律无「继续自动化」入口。
    fn unmanaged() -> Self {
        Self {
            managed: false,
            binding: None,
            plan_id: None,
            session_id: None,
            terminated_at: None,
            stop_key: None,
            stage: "unmanaged".to_string(),
            allowed_actions: Vec::new(),
            context_required: false,
        }
    }

    /// 本会话此刻是否真有「继续自动化」入口（与 `allowed_actions` 含
    /// `continue_plan` 同义）。
    pub(crate) fn continue_available(&self) -> bool {
        self.allowed_actions
            .iter()
            .any(|action| action == CONTINUE_PLAN_ACTION)
    }
}

/// 从 durable enrollment + 会话记录派生 control 事实（**纯函数，无 IO**）。
///
/// enrollment 缺失 / 未启用 / 绑定不可用 / 绑定会话与投影会话错绑 → unmanaged
/// 投影（无自动化入口，绝不给假指引）。
pub(crate) fn plan_control_facts(
    enrollment: Option<&IssueAutomationEnrollment>,
    record: &crate::product::models::WorkspaceSessionRecord,
    intent_phase: Option<PlanGenerationPhase>,
) -> PlanControlFacts {
    let Some(enrollment) = enrollment else {
        return PlanControlFacts::unmanaged();
    };
    let terminated_at = enrollment.terminated_at.clone();
    let binding = if enrollment.enabled {
        crate::web::advance_plan::load_current_enrollment_binding(enrollment).ok()
    } else {
        None
    };
    let Some(binding) = binding else {
        return PlanControlFacts::unmanaged();
    };
    // 绑定会话与被投影会话必须同一（错绑 → 不给假指引）。
    if binding.session_id != record.id {
        return PlanControlFacts::unmanaged();
    }
    let stop = resolve_plan_stop(record, intent_phase);
    let stop_key = plan_stop_key(&stop, &enrollment.enrollment_id, &binding);
    let (stage, allowed_actions, context_required) = if terminated_at.is_some() {
        ("terminated".to_string(), Vec::new(), false)
    } else {
        match &stop {
            PlanStop::FailedProvider => (
                "failed".to_string(),
                vec![CONTINUE_PLAN_ACTION.to_string()],
                false,
            ),
            PlanStop::StoppedHumanGate => (
                "stopped_needs_human".to_string(),
                vec![CONTINUE_PLAN_ACTION.to_string()],
                true,
            ),
            PlanStop::GenerationNeedsHuman => (
                "generation_needs_human".to_string(),
                vec![CONTINUE_PLAN_ACTION.to_string()],
                false,
            ),
            PlanStop::WaitingExisting => (
                "waiting_for_human".to_string(),
                vec![
                    "choice_response".to_string(),
                    "human_gate_feedback".to_string(),
                    "human_gate_close".to_string(),
                ],
                false,
            ),
            PlanStop::None => (serde_stage(&record.status), Vec::new(), false),
        }
    };
    PlanControlFacts {
        managed: true,
        binding: Some(binding),
        plan_id: enrollment.plan_id.clone(),
        session_id: enrollment.session_id.clone(),
        terminated_at,
        stop_key,
        stage,
        allowed_actions,
        context_required,
    }
}

/// 读 plan 生成检查点阶段（读不到 → `None`；与 HTTP control 同口径、同路径）。
pub(crate) fn generation_intent_phase(
    paths: &ProductAppPaths,
    enrollment: &IssueAutomationEnrollment,
) -> Option<PlanGenerationPhase> {
    let intent_path = paths
        .issue_root(&enrollment.project_id, &enrollment.issue_id)
        .join("automation-generation-intent.json");
    if intent_path.metadata().is_err() {
        return None;
    }
    match crate::product::issue_automation_store::read_plan_generation_intent(&intent_path) {
        Ok(crate::product::issue_automation_store::PlanGenerationIntentRead::Current(intent)) => {
            Some(intent.phase)
        }
        Ok(crate::product::issue_automation_store::PlanGenerationIntentRead::LegacyUnbound {
            ..
        }) => None,
        Err(error) => {
            eprintln!(
                "[aria-plan-continue] generation intent unreadable enrollment={}: {error}",
                enrollment.enrollment_id
            );
            None
        }
    }
}

/// 「继续自动化」入口的服务端权威事实（WS `plan_continue_available`）。
///
/// - `Some(true)`：此刻确有该入口（与 control 的 `allowed_actions` 含
///   `continue_plan` 同源）。
/// - `Some(false)`：此刻确定没有该入口。
/// - `None`：服务端**未能判定**（读取失败 / 非 work_item_plan 会话）——前端按
///   「无真实续跑入口」处理，不得给假指引。
///
/// 会话记录每次现读（durable 事实优先于 manager 出生快照）：停点判定依赖
/// `status` / `single_candidate_phase` / `human_gate_snapshot` 的**当前**值。
pub(crate) fn plan_continue_available(paths: &ProductAppPaths, session_id: &str) -> Option<bool> {
    let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(paths.clone());
    let record = match lifecycle.get_workspace_session(session_id) {
        Ok(record) => record,
        Err(error) => {
            eprintln!(
                "[aria-plan-continue] session record read failed session={session_id}: {error}"
            );
            return None;
        }
    };
    plan_continue_available_for_record(paths, &record)
}

/// 同 [`plan_continue_available`]，但复用调用方已读好的 durable 会话记录。
pub(crate) fn plan_continue_available_for_record(
    paths: &ProductAppPaths,
    record: &crate::product::models::WorkspaceSessionRecord,
) -> Option<bool> {
    if record.workspace_type != WorkspaceType::WorkItemPlan {
        // 非 plan 会话没有「继续自动化」语义，且服务端无法判定其入口是否存在
        // → 未知（None），前端不得给假指引。
        return None;
    }
    let store = IssueAutomationStore::new(paths.clone());
    let enrollment = match store.get(&record.project_id, &record.issue_id) {
        Ok(Some(enrollment)) => enrollment,
        Ok(None) => {
            // 没有 enrollment → 没有自动化归属 → 确定无「继续自动化」入口。
            return Some(false);
        }
        Err(error) => {
            eprintln!(
                "[aria-plan-continue] enrollment read failed session={}: {error}",
                record.id
            );
            return None;
        }
    };
    let intent_phase = generation_intent_phase(paths, &enrollment);
    let facts = plan_control_facts(Some(&enrollment), record, intent_phase);
    Some(facts.continue_available())
}

/// `WorkspaceSessionStatus` 的稳定 stage 投影（与 wire snake_case 同口径）。
pub(crate) fn serde_stage(status: &WorkspaceSessionStatus) -> String {
    serde_json::to_value(status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::issue_automation_store::IssueAutomationStore;
    use crate::product::models::automation::{
        EnrollmentOptions, EnrollmentSource, EnrollmentWriteCommand, PlanApprovalPolicy,
    };
    use crate::product::models::{IssueWorkItemPlanOptions, ProviderName};
    use crate::product::work_item_plan_policy::{HumanGateSnapshot, HumanReason};
    use axum::http::StatusCode;
    use tower::ServiceExt;

    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_0001";
    const SESSION_ID: &str = "workspace_session_plan_continue_1";
    const PLAN_ID: &str = "issue_work_item_plan_continue_1";

    struct Fixture {
        _root: tempfile::TempDir,
        paths: ProductAppPaths,
        state: crate::web::state::WebAppState,
        store: IssueAutomationStore,
    }

    fn enable_command() -> EnrollmentWriteCommand {
        EnrollmentWriteCommand::Enable {
            command_id: None,
            selection_key: "plan-continue-fixture".to_string(),
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
        }
    }

    fn resumable_gate_snapshot() -> HumanGateSnapshot {
        HumanGateSnapshot {
            candidate_recovery: None,
            findings: Vec::new(),
            repeated_fingerprints: Vec::new(),
            attempts_used: 0,
            manual_repairs_remaining: 1,
            trigger: HumanReason::NativeHumanRequired,
            resumable: true,
            accepted_feedback_turns: Some(0),
        }
    }

    fn fixture(
        status: WorkspaceSessionStatus,
        phase: Option<SingleCandidatePhase>,
        snapshot: Option<HumanGateSnapshot>,
    ) -> Fixture {
        let root = tempfile::TempDir::new().unwrap();
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let store = IssueAutomationStore::new(paths.clone());
        let enrollment = store
            .compare_and_set(PROJECT_ID, ISSUE_ID, None, enable_command())
            .expect("enable enrollment");
        let mut record = crate::web::workspace_session::test_session_record(SESSION_ID);
        record.project_id = PROJECT_ID.to_string();
        record.issue_id = ISSUE_ID.to_string();
        record.entity_id = PLAN_ID.to_string();
        record.workspace_type = WorkspaceType::WorkItemPlan;
        record.status = status;
        record.single_candidate_phase = phase;
        record.human_gate_snapshot = snapshot;
        crate::product::json_store::write_json(
            &paths
                .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
                .join("workspace-sessions")
                .join(format!("{SESSION_ID}.json")),
            &record,
        )
        .expect("persist durable session record");
        store
            .bind_plan(
                PROJECT_ID,
                ISSUE_ID,
                enrollment.policy_revision,
                PLAN_ID,
                SESSION_ID,
            )
            .expect("bind plan");
        let state = crate::web::state::WebAppState::new(
            root.path().to_path_buf(),
            crate::web::runtime::WebRuntime::new_fake(root.path().to_path_buf()),
        );
        Fixture {
            _root: root,
            paths,
            state,
            store,
        }
    }

    /// HTTP control 投影（真实 handler 面）。
    async fn control_projection(state: &crate::web::state::WebAppState) -> serde_json::Value {
        let app = crate::web::app::build_web_router(state.clone());
        let uri =
            format!("/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/automation-enrollment/control");
        let response = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .method(axum::http::Method::GET)
                    .uri(uri)
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "control projection must answer"
        );
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).expect("control projection json")
    }

    /// REQ-HGC-04（Plan 5-C 等价性）：WS 谓词 `plan_continue_available` 与
    /// HTTP control 的 `allowed_actions` 含 `continue_plan` **必须一致**——同一
    /// fixture 逐条比对。两面同源 ⇒ 服务端不可能自相矛盾，前端不会拿到
    /// 「服务端说有入口、control 说没有」的矛盾指引。
    #[tokio::test]
    async fn plan_continue_available_matches_control_allowed_actions() {
        let cases = [
            (
                "failed_provider",
                WorkspaceSessionStatus::Failed,
                Some(SingleCandidatePhase::Failed),
                None,
            ),
            (
                "stopped_needs_human",
                WorkspaceSessionStatus::StoppedNeedsHuman,
                Some(SingleCandidatePhase::Evaluate),
                Some(resumable_gate_snapshot()),
            ),
            (
                "waiting_for_human",
                WorkspaceSessionStatus::WaitingForHuman,
                Some(SingleCandidatePhase::Evaluate),
                None,
            ),
            (
                "confirmed",
                WorkspaceSessionStatus::Confirmed,
                Some(SingleCandidatePhase::Evaluate),
                None,
            ),
            (
                "open",
                WorkspaceSessionStatus::Open,
                Some(SingleCandidatePhase::Evaluate),
                None,
            ),
        ];
        for (label, status, phase, snapshot) in cases {
            let fixture = fixture(status, phase, snapshot);
            let control = control_projection(&fixture.state).await;
            let expected = control["allowed_actions"]
                .as_array()
                .map(|actions| {
                    actions
                        .iter()
                        .any(|action| action.as_str() == Some(CONTINUE_PLAN_ACTION))
                })
                .unwrap_or(false);
            let actual = plan_continue_available(&fixture.paths, SESSION_ID);
            assert_eq!(
                actual,
                Some(expected),
                "case {label}: plan_continue_available must match control allowed_actions \
                 (control={control})"
            );
        }
    }

    /// 关闭 enrollment（未终止）→ control 退位 unmanaged、无 `continue_plan`；
    /// WS 谓词同步给出 `Some(false)`（确定无入口），不得说「有入口」。
    #[tokio::test]
    async fn plan_continue_available_is_false_when_enrollment_disabled() {
        let fixture = fixture(
            WorkspaceSessionStatus::StoppedNeedsHuman,
            Some(SingleCandidatePhase::Evaluate),
            Some(resumable_gate_snapshot()),
        );
        let enrollment = fixture
            .store
            .get(PROJECT_ID, ISSUE_ID)
            .expect("read enrollment")
            .expect("enrollment present");
        fixture
            .store
            .compare_and_set(
                PROJECT_ID,
                ISSUE_ID,
                Some(enrollment.policy_revision),
                EnrollmentWriteCommand::Disable,
            )
            .expect("disable enrollment");

        let control = control_projection(&fixture.state).await;
        assert_eq!(
            control["managed"], false,
            "disabled enrollment is unmanaged"
        );
        assert_eq!(
            plan_continue_available(&fixture.paths, SESSION_ID),
            Some(false),
            "disabled enrollment must not advertise a continue entry"
        );
    }

    /// 非 work_item_plan 会话 → 服务端**未能判定**（None）：前端按「无真实
    /// 续跑入口」处理，不得给假指引。
    #[tokio::test]
    async fn plan_continue_available_is_none_for_non_plan_session() {
        let fixture = fixture(
            WorkspaceSessionStatus::StoppedNeedsHuman,
            Some(SingleCandidatePhase::Evaluate),
            Some(resumable_gate_snapshot()),
        );
        let record = fixture
            .paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("workspace-sessions")
            .join(format!("{SESSION_ID}.json"));
        let mut durable = crate::product::json_store::read_json::<
            crate::product::models::WorkspaceSessionRecord,
        >(&record)
        .expect("read durable record");
        durable.workspace_type = WorkspaceType::Story;
        crate::product::json_store::write_json(&record, &durable).expect("persist story record");
        assert_eq!(
            plan_continue_available(&fixture.paths, SESSION_ID),
            None,
            "non plan sessions stay unknown"
        );
    }
}
