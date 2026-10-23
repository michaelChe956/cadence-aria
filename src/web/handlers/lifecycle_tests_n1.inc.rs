// 批 2 N1（REQ-WIGA-12）：N0 三类 automation attention 事实并入
// GET /api/issues/{issue}/lifecycle 的 coding_attention_items——mapper 只读
// 直读 enrollment（不递归调 GET、不写事实）、未 watched 也计数/展示、
// 原 7 类事项不变、terminate 清空旧事实后无幽灵通知。经 lifecycle.rs 的
// tests mod include!（large_file_guard 1200 行红线拆分，n7 先例）；
// axum/构建/种子辅助复用同 mod 内 lifecycle_tests.inc.rs 已有导入。

use crate::product::coding_attempt_store::CreateChoiceGateInput;
use crate::product::issue_automation_store::IssueAutomationStore;
use crate::product::models::automation::{
    AutomationAttentionFact, AutomationAttentionKind, AutomationAttentionStage, EnrollmentOptions,
    EnrollmentSource, EnrollmentWriteCommand, PlanApprovalPolicy,
};

const N1_LIFECYCLE_PLAN: &str = "plan_n1_lifecycle";
const N1_LIFECYCLE_SESSION: &str = "workspace_session_n1_lifecycle";

/// auto 授权 + 精确绑定 plan/session 的最小 enrollment 现场（映射面不依赖
/// plan 记录本身）。
fn n1_bound_enrollment(
    paths: &ProductAppPaths,
) -> crate::product::models::automation::EnrollmentBindingIdentity {
    let store = IssueAutomationStore::new(paths.clone());
    store
        .compare_and_set(
            PROJECT_ID,
            ISSUE_ID,
            None,
            EnrollmentWriteCommand::Enable {
                command_id: None,
                selection_key: "n1-lifecycle".to_string(),
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
                    plan_options: crate::product::models::IssueWorkItemPlanOptions {
                        include_integration_tests: false,
                        include_e2e_tests: false,
                        force_frontend_backend_split: false,
                        require_execution_plan_confirm: false,
                    },
                },
                target: crate::product::logical_codebase::EnrollmentTarget::SingleRepository {
                    repository_id: REPOSITORY_ID.to_string(),
                },
            },
        )
        .expect("enable enrollment");
    // C1 恢复等待项投影按 enrollment 绑定读取 session 记录——出生记录
    // 需真实落盘（lifecycle GET 对缺失读取 fail-closed 404）。
    crate::product::lifecycle_store::LifecycleStore::new(paths.clone())
        .create_workspace_session_with_id(
            crate::product::lifecycle_store::CreateWorkspaceSessionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: N1_LIFECYCLE_PLAN.to_string(),
                workspace_type: crate::product::models::WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Fake,
                reviewer_provider: None,
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            },
            N1_LIFECYCLE_SESSION.to_string(),
        )
        .expect("persist bound session record");
    store
        .bind_plan(
            PROJECT_ID,
            ISSUE_ID,
            1,
            N1_LIFECYCLE_PLAN,
            N1_LIFECYCLE_SESSION,
        )
        .expect("bind plan");
    let enrollment = store
        .get(PROJECT_ID, ISSUE_ID)
        .expect("read enrollment")
        .expect("enrollment present");
    crate::web::advance_plan::load_current_enrollment_binding(&enrollment).expect("current binding")
}

fn n1_fact(
    key: &str,
    kind: AutomationAttentionKind,
    reason_code: &str,
    choice_id: Option<&str>,
) -> AutomationAttentionFact {
    AutomationAttentionFact {
        key: key.to_string(),
        kind,
        stage: AutomationAttentionStage::Plan,
        plan_id: Some(N1_LIFECYCLE_PLAN.to_string()),
        session_id: Some(N1_LIFECYCLE_SESSION.to_string()),
        attempt_id: None,
        reason_code: reason_code.to_string(),
        occurred_at: "2026-10-16T00:00:00Z".to_string(),
        choice_id: choice_id.map(str::to_string),
        gate_id: None,
    }
}

async fn n1_lifecycle_json(app: &axum::Router) -> serde_json::Value {
    let response = get_issue_lifecycle(app).await;
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(
        status,
        StatusCode::OK,
        "lifecycle GET failed: {}",
        String::from_utf8_lossy(&body)
    );
    serde_json::from_slice(&body).expect("lifecycle json")
}

/// 红测①②⑤：三类事实并入 coding_attention_items（稳定 key/workspace_session
/// 目标）；未 watched 也通知（本测试零 WS 订阅/零 attachment）；原 7 类事项
/// 不变且在前；GET 只读不写事实（重复 GET 幂等、enrollment 字节不变）。
#[tokio::test]
async fn issue_lifecycle_merges_automation_attention_items_after_coding_items() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    seed_issue_and_work_items(&lifecycle, &paths);

    // 原 7 类代表：单仓 attempt + open choice gate（kind=choice）。
    let coding_store = CodingAttemptStore::new(paths.clone());
    let attempt = coding_store
        .create_attempt(CreateCodingAttemptInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: "wi-a".to_string(),
            base_branch: "main".to_string(),
            branch_name: format!("aria/{ISSUE_ID}/n1"),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: Some(ProviderName::Fake),
                review_rounds: 1,
                permission_modes: WorkspaceRolePermissionModes::default(),
            },
            target_snapshot: None,
            max_auto_rework: 1,
        })
        .expect("create attempt");
    coding_store
        .create_choice_gate(
            &attempt,
            CreateChoiceGateInput {
                attempt_id: attempt.id.clone(),
                choice_id: "choice_n1".to_string(),
                stage: CodingExecutionStage::Coding,
                node_id: None,
                role: crate::product::coding_models::CodingProviderRole::Coder,
                provider: ProviderName::Fake,
                source: "test".to_string(),
                prompt: "验收口径需要裁定".to_string(),
                options: vec![crate::product::coding_models::CodingChoiceOption {
                    id: "option-a".to_string(),
                    label: "按全局口径".to_string(),
                    description: None,
                }],
                allow_multiple: false,
                allow_free_text: false,
                questions: Vec::new(),
            },
        )
        .expect("create choice gate");

    // 批 2 事实：plan_question（choice 停点）+ review_limit（评审超限）。
    let binding = n1_bound_enrollment(&paths);
    let store = IssueAutomationStore::new(paths.clone());
    store
        .upsert_automation_attention_fact(
            PROJECT_ID,
            ISSUE_ID,
            &binding,
            n1_fact(
                &format!("plan_choice:{N1_LIFECYCLE_SESSION}:choice_n1"),
                AutomationAttentionKind::PlanQuestion,
                "plan_choice_pending",
                Some("choice_n1"),
            ),
        )
        .expect("seed plan_question fact");
    store
        .upsert_automation_attention_fact(
            PROJECT_ID,
            ISSUE_ID,
            &binding,
            n1_fact(
                &format!("plan_gate:{N1_LIFECYCLE_SESSION}:human_confirm"),
                AutomationAttentionKind::ReviewLimit,
                "plan_human_gate_open",
                None,
            ),
        )
        .expect("seed review_limit fact");
    let enrollment_json = paths
        .issue_root(PROJECT_ID, ISSUE_ID)
        .join("automation-enrollment.json");
    let enrollment_before = std::fs::read(&enrollment_json).expect("read enrollment bytes");

    let app = build_test_router(temp.path());
    let value = n1_lifecycle_json(&app).await;
    let items = value["coding_attention_items"]
        .as_array()
        .expect("coding_attention_items array");

    // 原 7 类不变：choice 项保持原 key/目标形态。
    let choice_item = items
        .iter()
        .find(|item| item["kind"] == "choice")
        .expect("原 choice 事项仍在");
    assert_eq!(
        choice_item["key"],
        serde_json::json!(format!("choice:{}:choice_n1", attempt.id))
    );

    // 批 2 三类并入同一数组：稳定 key + workspace_session 目标 + reason 透传。
    let plan_question = items
        .iter()
        .find(|item| item["kind"] == "plan_question")
        .expect("plan_question 并入 coding_attention_items");
    assert_eq!(
        plan_question["key"],
        serde_json::json!(format!("plan_choice:{N1_LIFECYCLE_SESSION}:choice_n1"))
    );
    assert_eq!(plan_question["target"]["type"], "workspace_session");
    assert_eq!(plan_question["target"]["session_id"], N1_LIFECYCLE_SESSION);
    assert_eq!(plan_question["reason_code"], "plan_choice_pending");
    assert_eq!(
        plan_question["focus"]["choice_id"], "choice_n1",
        "focus choice_id 透传"
    );
    let review_limit = items
        .iter()
        .find(|item| item["kind"] == "review_limit")
        .expect("review_limit 并入 coding_attention_items");
    assert_eq!(review_limit["target"]["type"], "workspace_session");

    // 合并位置：既有 coding 事项之后（不重排原 7 类）。
    let position = |kind: &str, key: &str| {
        items
            .iter()
            .position(|item| item["kind"] == kind && item["key"] == serde_json::json!(key))
            .unwrap_or_else(|| panic!("missing {kind} item"))
    };
    let choice_position = position("choice", &format!("choice:{}:choice_n1", attempt.id));
    let plan_question_position = position(
        "plan_question",
        &format!("plan_choice:{N1_LIFECYCLE_SESSION}:choice_n1"),
    );
    assert!(
        choice_position < plan_question_position,
        "automation 事项合并在既有 coding 事项后"
    );

    // GET 只读：重复 GET 幂等、不写 enrollment 事实（不递归调 GET）。
    let repeated = n1_lifecycle_json(&app).await;
    assert_eq!(
        repeated["coding_attention_items"], value["coding_attention_items"],
        "重复 GET 投影稳定"
    );
    let enrollment_after = std::fs::read(&enrollment_json).expect("read enrollment bytes");
    assert_eq!(
        enrollment_before, enrollment_after,
        "GET 不得写 enrollment 事实"
    );
}

/// 红测④：terminate 后无幽灵门通知——T3 已清空三类旧事实的 terminated
/// enrollment 不再投影 plan_question/review_limit；termination 清理失败事实
/// 仍投影（重清成功即消失）。
#[tokio::test]
async fn issue_lifecycle_no_ghost_attention_items_after_termination() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path().join(".aria"));
    let lifecycle = LifecycleStore::new(paths.clone());
    seed_issue_and_work_items(&lifecycle, &paths);

    let binding = n1_bound_enrollment(&paths);
    let store = IssueAutomationStore::new(paths.clone());
    // T3 终止清理失败留痕（唯一应存留的事实）。
    store
        .upsert_automation_attention_fact(
            PROJECT_ID,
            ISSUE_ID,
            &binding,
            AutomationAttentionFact {
                key: format!("{N1_LIFECYCLE_PLAN}:cmd_term:termination_cleanup_failed"),
                kind: AutomationAttentionKind::AutomationFailed,
                stage: AutomationAttentionStage::Termination,
                plan_id: Some(N1_LIFECYCLE_PLAN.to_string()),
                session_id: Some(N1_LIFECYCLE_SESSION.to_string()),
                attempt_id: None,
                reason_code: "termination_cleanup_failed".to_string(),
                occurred_at: "2026-10-16T01:00:00Z".to_string(),
                choice_id: None,
                gate_id: None,
            },
        )
        .expect("seed termination fact");
    // T1 终止 CAS durable 结果（三类旧事实已被 T3 批量清空）。
    let enrollment_path = paths
        .issue_root(PROJECT_ID, ISSUE_ID)
        .join("automation-enrollment.json");
    let mut terminated = store
        .get(PROJECT_ID, ISSUE_ID)
        .unwrap()
        .expect("enrollment present");
    terminated.enabled = false;
    terminated.terminated_at = Some("2026-10-16T01:00:00Z".to_string());
    crate::product::json_store::write_json(&enrollment_path, &terminated)
        .expect("persist terminated enrollment");

    let app = build_test_router(temp.path());
    let value = n1_lifecycle_json(&app).await;
    let items = value["coding_attention_items"]
        .as_array()
        .expect("coding_attention_items array");
    let automation_kinds = ["plan_question", "automation_failed", "review_limit"];
    let automation_items: Vec<_> = items
        .iter()
        .filter(|item| automation_kinds.contains(&item["kind"].as_str().unwrap_or("")))
        .collect();
    assert_eq!(
        automation_items.len(),
        1,
        "终止后仅 termination 清理失败事项"
    );
    assert_eq!(
        automation_items[0]["key"],
        serde_json::json!(format!(
            "{N1_LIFECYCLE_PLAN}:cmd_term:termination_cleanup_failed"
        ))
    );
    assert_eq!(automation_items[0]["kind"], "automation_failed");
    assert!(
        items
            .iter()
            .all(|item| item["kind"] != "plan_question" && item["kind"] != "review_limit"),
        "terminate 后无幽灵门通知"
    );
}
