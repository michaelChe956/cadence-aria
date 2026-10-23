//! 批 2 N1（REQ-WIGA-12，块 3 设计 §7.1）：N0 三类 automation attention
//! 事实（plan_question/automation_failed/review_limit）→ A1
//! `CodingAttentionItemDto` 的**只读** mapper。
//!
//! - 目标：plan 用 `workspace_session` 目标；coding（attempt_id 身份）用原
//!   `coding_attempt` 目标；无任何身份段的事实不投影（不伪造目标）。
//! - 目标存在性：事实指向的目标实体（会话/attempt/plan）已被级联删除时
//!   **不产出**该待办——事实随实体删除即失效，留下只会让人点进死路
//!   （已删会话 → 前端 404 卡死）。存在性用产品既有 store 读取判定
//!   （`LifecycleStore`/`CodingAttemptStore` 的 get/list），记录缺失
//!   （NotFound）→ 跳过；真实 IO/JSON 错误 fail-closed 上抛。
//! - focus 不扩展：`choice_id`/`gate_id` 原样透传。
//! - 只读：不写/不清事实（写者固定 P3/P4/R2/T3），不调用 GET
//!   lifecycle/progress（无递归调用面）；terminated enrollment 的
//!   termination 清理失败事实照常投影（重清成功即消失）。

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::issue_automation_store::IssueAutomationStore;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::automation::AutomationAttentionFact;
use crate::web::coding_attention_info::{
    CodingAttentionFocusDto, CodingAttentionItemDto, CodingAttentionTargetDto,
};

/// plan_question（choice/人工门/对话停点）固定标题。
pub const AUTOMATION_ATTENTION_PLAN_QUESTION_TITLE: &str = "自动化计划等待人工答复";
/// automation_failed（provider/advance/start/清理失败）固定标题。
pub const AUTOMATION_ATTENTION_AUTOMATION_FAILED_TITLE: &str = "自动化执行失败，需要处理";
/// review_limit（plan 评审超限停点）固定标题。
pub const AUTOMATION_ATTENTION_REVIEW_LIMIT_TITLE: &str = "计划评审超限，需要人工说明";

/// 枚举 issue 的 enrollment 待处理事实 → A1 通知 DTO（未 watched 也计数/
/// 展示——目录是 durable 事实驱动，与订阅无关）。读取错误 fail-closed 上抛；
/// 无 enrollment/无事实 → 空表。
pub fn issue_automation_attention_items(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
) -> Result<Vec<CodingAttentionItemDto>, ProductStoreError> {
    let Some(enrollment) = IssueAutomationStore::new(paths.clone()).get(project_id, issue_id)?
    else {
        return Ok(Vec::new());
    };
    let mut items: Vec<CodingAttentionItemDto> = Vec::new();
    for fact in &enrollment.attention_facts {
        if let Some(item) = automation_attention_item(paths, project_id, issue_id, fact)? {
            items.push(item);
        }
    }
    items.sort_by(|left, right| {
        left.occurred_at
            .cmp(&right.occurred_at)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.key.cmp(&right.key))
    });
    Ok(items)
}

/// 单条事实 → DTO；无 attempt/session/plan 身份段时返回 `None`（不伪造
/// 目标，project/issue 顶层不足以路由进 workspace_session）；目标实体已被
/// 级联删除时同样返回 `None`（点进去是死路，不产出待办）。
fn automation_attention_item(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    fact: &AutomationAttentionFact,
) -> Result<Option<CodingAttentionItemDto>, ProductStoreError> {
    use crate::product::models::automation::AutomationAttentionKind;
    let (kind, title) = match fact.kind {
        AutomationAttentionKind::PlanQuestion => {
            ("plan_question", AUTOMATION_ATTENTION_PLAN_QUESTION_TITLE)
        }
        AutomationAttentionKind::AutomationFailed => (
            "automation_failed",
            AUTOMATION_ATTENTION_AUTOMATION_FAILED_TITLE,
        ),
        AutomationAttentionKind::ReviewLimit => {
            ("review_limit", AUTOMATION_ATTENTION_REVIEW_LIMIT_TITLE)
        }
    };
    let target = if let Some(attempt_id) = fact.attempt_id.as_deref() {
        if !coding_attempt_exists(paths, project_id, issue_id, attempt_id)? {
            return Ok(None);
        }
        CodingAttentionTargetDto::coding_attempt(attempt_id)
    } else if let Some(session_id) = fact.session_id.as_deref() {
        if !workspace_session_exists(paths, session_id)? {
            return Ok(None);
        }
        CodingAttentionTargetDto {
            target_type: "workspace_session".to_string(),
            attempt_id: None,
            plan_id: None,
            session_id: Some(session_id.to_string()),
        }
    } else {
        let Some(plan_id) = fact.plan_id.as_deref() else {
            return Ok(None);
        };
        if !coding_plan_exists(paths, project_id, issue_id, plan_id)? {
            return Ok(None);
        }
        CodingAttentionTargetDto::coding_plan(plan_id)
    };
    let focus = match (fact.gate_id.as_deref(), fact.choice_id.as_deref()) {
        (None, None) => None,
        (gate_id, choice_id) => Some(CodingAttentionFocusDto {
            gate_id: gate_id.map(str::to_string),
            choice_id: choice_id.map(str::to_string),
        }),
    };
    Ok(Some(CodingAttentionItemDto {
        key: fact.key.clone(),
        kind: kind.to_string(),
        project_id: project_id.to_string(),
        issue_id: issue_id.to_string(),
        plan_id: fact.plan_id.clone(),
        attempt_id: fact.attempt_id.clone(),
        logical_work_item_id: None,
        repository_name: None,
        reason_code: Some(fact.reason_code.clone()),
        title: title.to_string(),
        // reason_code 只读不翻译（写者自有码原样展示）。
        summary: fact.reason_code.clone(),
        occurred_at: fact.occurred_at.clone(),
        target,
        focus,
    }))
}

/// 会话记录是否仍存在（`LifecycleStore` get：记录缺失=NotFound → 已删）。
fn workspace_session_exists(
    paths: &ProductAppPaths,
    session_id: &str,
) -> Result<bool, ProductStoreError> {
    match LifecycleStore::new(paths.clone()).get_workspace_session(session_id) {
        Ok(_) => Ok(true),
        // 会话已被删除（实体级联删除清盘）→ 目标不存在。
        Err(ProductStoreError::NotFound { .. }) => Ok(false),
        Err(other) => Err(other),
    }
}

/// attempt 记录是否仍存在（`CodingAttemptStore` get：记录缺失=NotFound → 已删）。
fn coding_attempt_exists(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
) -> Result<bool, ProductStoreError> {
    match CodingAttemptStore::new(paths.clone()).get_attempt(project_id, issue_id, attempt_id) {
        Ok(_) => Ok(true),
        // attempt 已被删除（实体级联删除清盘）→ 目标不存在。
        Err(ProductStoreError::NotFound { .. }) => Ok(false),
        Err(other) => Err(other),
    }
}

/// plan 记录是否仍存在（`LifecycleStore` list：按 issue 枚举命中即存在）。
fn coding_plan_exists(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    plan_id: &str,
) -> Result<bool, ProductStoreError> {
    let plans =
        LifecycleStore::new(paths.clone()).list_issue_work_item_plans(project_id, issue_id)?;
    Ok(plans.iter().any(|plan| plan.id == plan_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::coding_attempt_store::{CodingAttemptStore, CreateGroupCodingAttemptInput};
    use crate::product::coding_models::CodingStartRunPolicy;
    use crate::product::lifecycle_store::{
        CreateIssueWorkItemPlanInput, CreateWorkspaceSessionInput, LifecycleStore,
    };
    use crate::product::logical_codebase::EnrollmentTarget;
    use crate::product::models::IssueWorkItemPlanOptions;
    use crate::product::models::ProviderName;
    use crate::product::models::automation::{
        AutomationAttentionKind, AutomationAttentionStage, EnrollmentBindingIdentity,
        EnrollmentOptions, EnrollmentSource, EnrollmentWriteCommand, PlanApprovalPolicy,
    };
    use crate::product::models::{
        IssueWorkItemPlanStatus, WorkspaceRolePermissionModes, WorkspaceType,
    };
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    const N1_PROJECT: &str = "project_n1";
    const N1_ISSUE: &str = "issue_n1";
    const N1_PLAN: &str = "plan_n1";
    /// 会话 id 必须带 `workspace_session_` 前缀（会话记录文件名约定）。
    const N1_SESSION: &str = "workspace_session_n1";

    struct MapperFixture {
        _root: tempfile::TempDir,
        paths: ProductAppPaths,
        store: IssueAutomationStore,
        binding: EnrollmentBindingIdentity,
        attempt_id: String,
    }

    fn mapper_fixture() -> MapperFixture {
        let root = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let store = IssueAutomationStore::new(paths.clone());
        store
            .compare_and_set(
                N1_PROJECT,
                N1_ISSUE,
                None,
                EnrollmentWriteCommand::Enable {
                    selection_key: "n1-mapper".to_string(),
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
                    target: EnrollmentTarget::SingleRepository {
                        repository_id: "repo_n1".to_string(),
                    },
                },
            )
            .expect("enable enrollment");
        store
            .bind_plan(N1_PROJECT, N1_ISSUE, 1, N1_PLAN, N1_SESSION)
            .expect("bind plan");
        let enrollment = store
            .get(N1_PROJECT, N1_ISSUE)
            .expect("read enrollment")
            .expect("enrollment present");
        let binding = crate::web::advance_plan::load_current_enrollment_binding(&enrollment)
            .expect("current binding");
        // 目标存在性校验需要真实实体记录：mapper 只在事实指向的目标仍存在
        // 时投影待办（目标已被级联删除 → 点进去是死路 → 不产出）。
        let lifecycle = LifecycleStore::new(paths.clone());
        lifecycle
            .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
                id: Some(N1_PLAN.to_string()),
                project_id: N1_PROJECT.to_string(),
                issue_id: N1_ISSUE.to_string(),
                source_story_spec_ids: Vec::new(),
                source_design_spec_ids: Vec::new(),
                options: IssueWorkItemPlanOptions {
                    include_integration_tests: false,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                status: IssueWorkItemPlanStatus::Confirmed,
                work_item_ids: Vec::new(),
                repository_profile_ref: None,
                verification_plan_ids: Vec::new(),
                dependency_graph: Vec::new(),
                created_from_provider_run: None,
                validator_findings: Vec::new(),
            })
            .expect("create plan");
        lifecycle
            .create_workspace_session_bound(
                CreateWorkspaceSessionInput {
                    project_id: N1_PROJECT.to_string(),
                    issue_id: N1_ISSUE.to_string(),
                    entity_id: N1_PLAN.to_string(),
                    workspace_type: WorkspaceType::WorkItemPlan,
                    author_provider: ProviderName::Fake,
                    reviewer_provider: Some(ProviderName::Fake),
                    review_rounds: 0,
                    superpowers_enabled: false,
                    openspec_enabled: false,
                    work_item_plan_options: None,
                },
                N1_SESSION.to_string(),
            )
            .expect("create session");
        let attempt = CodingAttemptStore::new(paths.clone())
            .create_group_attempt(CreateGroupCodingAttemptInput {
                project_id: N1_PROJECT.to_string(),
                issue_id: N1_ISSUE.to_string(),
                plan_id: N1_PLAN.to_string(),
                current_work_item_id: "wi_n1".to_string(),
                base_branch: "main".to_string(),
                branch_name: "aria/issues/issue_n1".to_string(),
                worktree_path: None,
                provider_config_snapshot: provider_snapshot(),
                target_snapshot: None,
                max_auto_rework: 2,
                start_run_policy: CodingStartRunPolicy::Manual,
            })
            .expect("create attempt");
        MapperFixture {
            _root: root,
            paths,
            store,
            binding,
            attempt_id: attempt.id,
        }
    }

    fn provider_snapshot() -> ProviderConfigSnapshot {
        ProviderConfigSnapshot {
            author: ProviderName::Fake,
            reviewer: Some(ProviderName::Fake),
            review_rounds: 0,
            permission_modes: WorkspaceRolePermissionModes::default(),
        }
    }

    /// 会话记录文件路径（模拟实体级联删除清盘）。
    fn session_path(paths: &ProductAppPaths, session_id: &str) -> std::path::PathBuf {
        paths
            .issue_lifecycle_root(N1_PROJECT, N1_ISSUE)
            .join("workspace-sessions")
            .join(format!("{session_id}.json"))
    }

    /// plan 记录文件路径（模拟实体级联删除清盘）。
    fn plan_path(paths: &ProductAppPaths, plan_id: &str) -> std::path::PathBuf {
        paths
            .issue_lifecycle_root(N1_PROJECT, N1_ISSUE)
            .join("issue-work-item-plans")
            .join(format!("{plan_id}.json"))
    }

    /// attempt 记录文件路径（模拟实体级联删除清盘）。
    fn attempt_path(paths: &ProductAppPaths, attempt_id: &str) -> std::path::PathBuf {
        paths
            .issue_lifecycle_root(N1_PROJECT, N1_ISSUE)
            .join("coding-attempts")
            .join(format!("{attempt_id}.json"))
    }

    fn fact(
        key: &str,
        kind: AutomationAttentionKind,
        stage: AutomationAttentionStage,
        attempt_id: Option<&str>,
        reason_code: &str,
        choice_id: Option<&str>,
        gate_id: Option<&str>,
    ) -> crate::product::models::automation::AutomationAttentionFact {
        crate::product::models::automation::AutomationAttentionFact {
            key: key.to_string(),
            kind,
            stage,
            plan_id: Some(N1_PLAN.to_string()),
            session_id: Some(N1_SESSION.to_string()),
            attempt_id: attempt_id.map(str::to_string),
            reason_code: reason_code.to_string(),
            occurred_at: "2026-10-16T00:00:00Z".to_string(),
            choice_id: choice_id.map(str::to_string),
            gate_id: gate_id.map(str::to_string),
        }
    }

    fn seeded_facts(fixture: &MapperFixture) -> Vec<String> {
        fixture
            .store
            .get(N1_PROJECT, N1_ISSUE)
            .unwrap()
            .expect("enrollment present")
            .attention_facts
            .into_iter()
            .map(|fact| fact.key)
            .collect()
    }

    /// 红测①：三类 kind → A1 DTO（kind 字符串/稳定 key/正确 session 目标/
    /// coding attempt 原目标/focus 与 reason 只读透传）。
    #[test]
    fn automation_attention_items_map_three_kinds_with_stable_keys_and_targets() {
        let fixture = mapper_fixture();
        let choice_fact = fact(
            &format!("plan_choice:{N1_SESSION}:choice_n1"),
            AutomationAttentionKind::PlanQuestion,
            AutomationAttentionStage::Plan,
            None,
            "plan_choice_pending",
            Some("choice_n1"),
            None,
        );
        let gate_fact = fact(
            &format!("plan_gate:{N1_SESSION}:human_confirm"),
            AutomationAttentionKind::ReviewLimit,
            AutomationAttentionStage::Plan,
            None,
            "plan_human_gate_open",
            None,
            None,
        );
        let coding_fact = fact(
            &format!("{N1_PLAN}:cmd_n1:coding_start_failed"),
            AutomationAttentionKind::AutomationFailed,
            AutomationAttentionStage::CodingStart,
            Some(fixture.attempt_id.as_str()),
            "coding_start_failed",
            None,
            None,
        );
        for seeded in [&choice_fact, &gate_fact, &coding_fact] {
            fixture
                .store
                .upsert_automation_attention_fact(
                    N1_PROJECT,
                    N1_ISSUE,
                    &fixture.binding,
                    seeded.clone(),
                )
                .expect("seed fact");
        }

        let items = issue_automation_attention_items(&fixture.paths, N1_PROJECT, N1_ISSUE)
            .expect("map attention items");
        assert_eq!(items.len(), 3, "三类事实各一条：{items:?}");

        let choice_item = items
            .iter()
            .find(|item| item.kind == "plan_question")
            .expect("plan_question item");
        assert_eq!(choice_item.key, choice_fact.key, "稳定 key 原样");
        assert_eq!(choice_item.title, AUTOMATION_ATTENTION_PLAN_QUESTION_TITLE);
        assert_eq!(choice_item.target.target_type, "workspace_session");
        assert_eq!(choice_item.target.session_id.as_deref(), Some(N1_SESSION));
        assert!(choice_item.target.attempt_id.is_none());
        assert_eq!(
            choice_item
                .focus
                .as_ref()
                .and_then(|focus| focus.choice_id.clone()),
            Some("choice_n1".to_string()),
            "focus choice_id 透传"
        );
        assert_eq!(
            choice_item.reason_code.as_deref(),
            Some("plan_choice_pending")
        );
        assert_eq!(
            choice_item.summary, "plan_choice_pending",
            "reason 只读不翻译"
        );
        assert_eq!(choice_item.occurred_at, "2026-10-16T00:00:00Z");
        assert_eq!(choice_item.plan_id.as_deref(), Some(N1_PLAN));

        let gate_item = items
            .iter()
            .find(|item| item.kind == "review_limit")
            .expect("review_limit item");
        assert_eq!(gate_item.target.target_type, "workspace_session");
        assert_eq!(gate_item.target.session_id.as_deref(), Some(N1_SESSION));
        assert_eq!(gate_item.title, AUTOMATION_ATTENTION_REVIEW_LIMIT_TITLE);

        let coding_item = items
            .iter()
            .find(|item| item.kind == "automation_failed")
            .expect("automation_failed item");
        assert_eq!(
            coding_item.target.target_type, "coding_attempt",
            "coding 用原目标"
        );
        assert_eq!(
            coding_item.target.attempt_id.as_deref(),
            Some(fixture.attempt_id.as_str())
        );
        assert_eq!(
            coding_item.title,
            AUTOMATION_ATTENTION_AUTOMATION_FAILED_TITLE
        );
    }

    /// 红测④/⑤：terminated enrollment——T3 清空三类旧事实后无幽灵事项；
    /// termination 清理失败事实仍投影（重清成功即消失）；mapper 只读
    ///（enrollment 文件字节不变、不写事实）。
    #[test]
    fn automation_attention_items_terminated_no_ghost_and_read_only() {
        let fixture = mapper_fixture();
        let cleanup_fact = crate::product::models::automation::AutomationAttentionFact {
            key: format!("{N1_PLAN}:cmd_term:termination_cleanup_failed"),
            kind: AutomationAttentionKind::AutomationFailed,
            stage: AutomationAttentionStage::Termination,
            plan_id: Some(N1_PLAN.to_string()),
            session_id: Some(N1_SESSION.to_string()),
            attempt_id: None,
            reason_code: "termination_cleanup_failed".to_string(),
            occurred_at: "2026-10-16T01:00:00Z".to_string(),
            choice_id: None,
            gate_id: None,
        };
        fixture
            .store
            .upsert_automation_attention_fact(
                N1_PROJECT,
                N1_ISSUE,
                &fixture.binding,
                cleanup_fact.clone(),
            )
            .expect("seed termination fact");
        // 模拟 T1 终止 CAS durable 结果（terminated_at＋enabled=false）。
        let enrollment_path = fixture
            .paths
            .issue_root(N1_PROJECT, N1_ISSUE)
            .join("automation-enrollment.json");
        let mut terminated = fixture
            .store
            .get(N1_PROJECT, N1_ISSUE)
            .unwrap()
            .expect("enrollment present");
        terminated.enabled = false;
        terminated.terminated_at = Some("2026-10-16T01:00:00Z".to_string());
        crate::product::json_store::write_json(&enrollment_path, &terminated)
            .expect("persist terminated enrollment");
        let before = std::fs::read(&enrollment_path).expect("read enrollment bytes");

        let items = issue_automation_attention_items(&fixture.paths, N1_PROJECT, N1_ISSUE)
            .expect("map attention items");
        assert_eq!(
            items.len(),
            1,
            "清空三类旧事实后无幽灵门通知，仅 termination 清理失败事实：{items:?}"
        );
        assert_eq!(items[0].key, cleanup_fact.key);
        assert_eq!(
            items[0].reason_code.as_deref(),
            Some("termination_cleanup_failed")
        );

        // GET 只读：重复映射不写事实（文件字节不变）。
        let again = issue_automation_attention_items(&fixture.paths, N1_PROJECT, N1_ISSUE)
            .expect("map attention items again");
        assert_eq!(again, items);
        let after = std::fs::read(&enrollment_path).expect("read enrollment bytes");
        assert_eq!(before, after, "mapper 不得写 enrollment 事实");
        assert_eq!(seeded_facts(&fixture), vec![cleanup_fact.key]);
    }

    /// 红测①边界：无 enrollment → 空表；无身份段事实不投影（不伪造目标）。
    #[test]
    fn automation_attention_items_empty_without_enrollment_or_identity() {
        let root = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        assert!(
            issue_automation_attention_items(&paths, "project_none", "issue_none")
                .expect("no enrollment")
                .is_empty()
        );

        let fixture = mapper_fixture();
        let identityless = crate::product::models::automation::AutomationAttentionFact {
            key: "identityless".to_string(),
            kind: AutomationAttentionKind::PlanQuestion,
            stage: AutomationAttentionStage::Plan,
            plan_id: None,
            session_id: None,
            attempt_id: None,
            reason_code: "plan_choice_pending".to_string(),
            occurred_at: "2026-10-16T00:00:00Z".to_string(),
            choice_id: Some("choice_x".to_string()),
            gate_id: None,
        };
        fixture
            .store
            .upsert_automation_attention_fact(N1_PROJECT, N1_ISSUE, &fixture.binding, identityless)
            .expect("seed identityless fact");
        assert!(
            issue_automation_attention_items(&fixture.paths, N1_PROJECT, N1_ISSUE)
                .expect("map attention items")
                .is_empty(),
            "无身份段事实不投影"
        );
    }

    /// plan 目标事实（无 attempt/session 身份段 → `coding_plan` 目标）。
    fn plan_target_fact(
        key: &str,
        reason_code: &str,
    ) -> crate::product::models::automation::AutomationAttentionFact {
        crate::product::models::automation::AutomationAttentionFact {
            key: key.to_string(),
            kind: AutomationAttentionKind::AutomationFailed,
            stage: AutomationAttentionStage::Plan,
            plan_id: Some(N1_PLAN.to_string()),
            session_id: None,
            attempt_id: None,
            reason_code: reason_code.to_string(),
            occurred_at: "2026-10-16T02:00:00Z".to_string(),
            choice_id: None,
            gate_id: None,
        }
    }

    /// F2 红测①：事实指向的会话已被删除（plan 级联删除清盘）→ 待办点进去
    /// 是死路 → 不产出；会话仍在 → 照常产出（不误杀）。
    #[test]
    fn automation_attention_items_skip_deleted_session_target() {
        let fixture = mapper_fixture();
        let deleted_session_fact = fact(
            &format!("{N1_PLAN}:cmd_n1:session_deleted"),
            AutomationAttentionKind::AutomationFailed,
            AutomationAttentionStage::CodingStart,
            None,
            "coding_start_failed",
            None,
            None,
        );
        fixture
            .store
            .upsert_automation_attention_fact(
                N1_PROJECT,
                N1_ISSUE,
                &fixture.binding,
                deleted_session_fact.clone(),
            )
            .expect("seed fact");

        // 会话仍在 → 产出（不误杀）。
        let items = issue_automation_attention_items(&fixture.paths, N1_PROJECT, N1_ISSUE)
            .expect("map attention items");
        assert_eq!(items.len(), 1, "会话存在时待办照常产出：{items:?}");
        assert_eq!(items[0].target.target_type, "workspace_session");
        assert_eq!(items[0].target.session_id.as_deref(), Some(N1_SESSION));

        // 模拟实体级联删除清盘会话文件。
        std::fs::remove_file(session_path(&fixture.paths, N1_SESSION))
            .expect("remove session file");
        let items = issue_automation_attention_items(&fixture.paths, N1_PROJECT, N1_ISSUE)
            .expect("map attention items");
        assert!(
            items.is_empty(),
            "已删会话的待办不得产出（点进去是死路）：{items:?}"
        );
    }

    /// F2 红测②：事实指向的 attempt 记录不存在 → 不产出该待办。
    #[test]
    fn automation_attention_items_skip_deleted_attempt_target() {
        let fixture = mapper_fixture();
        let deleted_attempt_fact = fact(
            &format!("{N1_PLAN}:cmd_n1:attempt_deleted"),
            AutomationAttentionKind::AutomationFailed,
            AutomationAttentionStage::CodingStart,
            Some(fixture.attempt_id.as_str()),
            "coding_start_failed",
            None,
            None,
        );
        fixture
            .store
            .upsert_automation_attention_fact(
                N1_PROJECT,
                N1_ISSUE,
                &fixture.binding,
                deleted_attempt_fact.clone(),
            )
            .expect("seed fact");

        // attempt 仍在 → 产出（不误杀）。
        let items = issue_automation_attention_items(&fixture.paths, N1_PROJECT, N1_ISSUE)
            .expect("map attention items");
        assert_eq!(items.len(), 1, "attempt 存在时待办照常产出：{items:?}");
        assert_eq!(items[0].target.target_type, "coding_attempt");

        std::fs::remove_file(attempt_path(&fixture.paths, &fixture.attempt_id))
            .expect("remove attempt file");
        let items = issue_automation_attention_items(&fixture.paths, N1_PROJECT, N1_ISSUE)
            .expect("map attention items");
        assert!(
            items.is_empty(),
            "已删 attempt 的待办不得产出（点进去是死路）：{items:?}"
        );
    }

    /// F2 红测③：plan 目标事实——plan 记录不存在 → 不产出；plan 仍在 → 产出。
    #[test]
    fn automation_attention_items_skip_deleted_plan_target() {
        let fixture = mapper_fixture();
        let deleted_plan_fact = plan_target_fact(
            &format!("{N1_PLAN}:cmd_n1:plan_deleted"),
            "plan_advance_failed",
        );
        fixture
            .store
            .upsert_automation_attention_fact(
                N1_PROJECT,
                N1_ISSUE,
                &fixture.binding,
                deleted_plan_fact.clone(),
            )
            .expect("seed fact");

        // plan 仍在 → 产出（不误杀）。
        let items = issue_automation_attention_items(&fixture.paths, N1_PROJECT, N1_ISSUE)
            .expect("map attention items");
        assert_eq!(items.len(), 1, "plan 存在时待办照常产出：{items:?}");
        assert_eq!(items[0].target.target_type, "coding_plan");
        assert_eq!(items[0].target.plan_id.as_deref(), Some(N1_PLAN));

        std::fs::remove_file(plan_path(&fixture.paths, N1_PLAN)).expect("remove plan file");
        let items = issue_automation_attention_items(&fixture.paths, N1_PROJECT, N1_ISSUE)
            .expect("map attention items");
        assert!(
            items.is_empty(),
            "已删 plan 的待办不得产出（点进去是死路）：{items:?}"
        );
    }
}
