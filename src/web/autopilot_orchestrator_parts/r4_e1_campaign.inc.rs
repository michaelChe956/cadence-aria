// ---------------------------------------------------------------------------
// 批 2 R4/E1（REQ-WIGA-13、REQ-WIGA-09 负例）：四仓整链夹具验收。
//
// 零人工正例＝enrollment 集合授权（4 成员，plan_approval=auto）→AutoIfValid
// 自动批准（无 approve/人工门 REST）→advance 集合准入（P⊆A，选中无 WI 的
// member-d 不造空 attempt）→多仓 attempt 创建＋自动首启→跨仓依赖下游在
// 上游完成后自动开始→批 1 A2 唯一无 body 最终确认；终止两阶段（未确认
// 428 零副作用→确认后整 plan 终止）、终止后旧事实清理无幽灵通知、无新启
// 为独立负例。真实 provider 整链由 Main 按块 3 §8 执行；本文件交付关闭式
// Fake 集成（先例 p2_campaign / auto_enrolled_sc_confirms_via_engine）。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod r4_e1_four_repository_campaign {
    use super::*;
    use crate::product::coding_models::{CodingAttemptStatus, CodingExecutionStage};
    use crate::product::logical_codebase::{EnrollmentTarget, LogicalRepositoryId};
    use crate::product::models::automation::PlanApprovalPolicy;
    use crate::web::wiga_gate_fixture::*;

    fn member(id: u128) -> LogicalRepositoryId {
        LogicalRepositoryId(uuid::Uuid::from_u128(id))
    }

    fn attempt_status_for(
        fixture: &EnrolledGateFixture,
        member: &LogicalRepositoryId,
    ) -> CodingAttemptStatus {
        fixture
            .attempt_for_member(member)
            .unwrap_or_else(|| panic!("attempt for member {member:?} missing"))
            .status
    }

    /// ① 四仓零人工正例：全程唯一人手动作＝最终确认按钮（一次无 body
    /// POST）；AI 提问/门停点等故障注入不在本例（独立负例）。
    #[tokio::test]
    async fn r4_e1_four_repository_zero_human_campaign_until_single_final_confirm() {
        let mut fixture = EnrolledGateFixture::new_four_repository_collection().await;

        // enrollment 冻结：集合 target（4 成员升序）＋auto 批准策略。
        let enrollment = fixture.enrollment();
        let target = enrollment.target.as_ref().expect("frozen collection target");
        match target {
            EnrollmentTarget::LogicalCodebaseMembers {
                logical_repository_ids,
                ..
            } => {
                let mut members = logical_repository_ids.clone();
                members.sort();
                assert_eq!(
                    members,
                    vec![member(1), member(2), member(3), member(4)],
                    "collection target must cover all four seeded members"
                );
            }
            other => panic!("expected collection target, got {other:?}"),
        }
        assert_eq!(enrollment.options.plan_approval, PlanApprovalPolicy::Auto);

        // AutoIfValid 自动批准：零 approve/人工门 REST，durable直达
        // Confirmed＋Completed（引擎 policy-valid compile 语义）。
        fixture.auto_approve_plan().await;
        let durable = fixture.durable();
        assert_eq!(
            durable.status,
            crate::product::models::WorkspaceSessionStatus::Confirmed
        );
        assert_eq!(
            durable.single_candidate_phase,
            Some(crate::product::models::SingleCandidatePhase::Completed)
        );
        assert_eq!(
            durable.run_policy,
            crate::product::work_item_plan_policy::RunPolicy::AutoIfValid
        );

        // 子 WorkItem 会话精确绑定：完整授权复制（集合 target 原样）。
        let child_bindings = fixture.child_collection_binding_targets();
        assert_eq!(child_bindings.len(), 3, "three attributed work items");
        for binding_target in &child_bindings {
            assert_eq!(binding_target, target, "child copies the full collection authorization");
        }

        // reconcile 循环阶段一：advance 集合准入→3 实际桶（member-d 选中但
        // 无 WI，不造空 attempt）→就绪根（a、c）自动首启；跨仓依赖下游 b
        // 在上游 a 完成前零启动。
        fixture
            .reconcile_until(|attempts| {
                attempts.len() == 3
                    && attempts
                        .iter()
                        .filter(|attempt| attempt.start_claim.is_some())
                        .count() >= 2
            })
            .await;
        let attempts = fixture.coding_attempts();
        assert_eq!(attempts.len(), 3, "no empty attempt for member-d");
        assert!(
            fixture.attempt_for_member(&member(4)).is_none(),
            "selected member without work items must not materialize an attempt"
        );
        for expected in [member(1), member(2), member(3)] {
            let attempt = fixture
                .attempt_for_member(&expected)
                .expect("actual bucket attempt");
            let claim = attempt.start_claim.as_ref();
            if expected == member(2) {
                // wi_b 依赖 wi_a（跨仓依赖）：上游未完成前不就绪、零启动。
                assert!(
                    claim.is_none(),
                    "cross-repo dependent bucket must not start before its upstream completes"
                );
                continue;
            }
            let claim = claim.expect("independent bucket auto-started");
            assert_eq!(
                claim.origin,
                crate::product::coding_models::CodingStartOrigin::Enrolled {
                    enrollment_id: enrollment.enrollment_id.clone(),
                    policy_revision: enrollment.policy_revision,
                    binding_version: enrollment
                        .binding_history
                        .as_ref()
                        .map(|history| history.current.binding_version),
                    target: Some(target.clone()),
                },
                "coding_start must verify the collection enrollment and freeze the enrolled origin"
            );
        }

        // 阶段二：上游 a 完成→下游 b 自动开始（fan-out 挂点与 tick 同源）；
        // 期间单桶完成≠全 plan 完成，链路继续推进其余桶。
        fixture
            .reconcile_until(|attempts| {
                attempts
                    .iter()
                    .any(|attempt| attempt.start_claim.is_some()
                        && attempt
                            .target_snapshot
                            .as_ref()
                            .is_some_and(|snapshot| snapshot.logical_repository_id == member(2)))
            })
            .await;

        // 阶段三：全部实际桶到达 WaitingForHuman∧FinalConfirm；编排器只观察
        // （最终确认不被代点）。
        fixture
            .reconcile_until(|attempts| {
                !attempts.is_empty()
                    && attempts.iter().all(|attempt| {
                        attempt.status == CodingAttemptStatus::WaitingForHuman
                            && attempt.stage == CodingExecutionStage::FinalConfirm
                    })
            })
            .await;
        let worker = AutopilotOrchestrator::new(fixture.state.clone(), Default::default());
        assert_eq!(
            worker.reconcile(&fixture.state, PROJECT_ID, ISSUE_ID).await.unwrap(),
            ReconcileOutcome::AwaitingHuman,
            "orchestrator must observe the human final-confirm stop without clicking it"
        );

        // 批 1 唯一最终确认：一次无 body POST 全仓确认。
        let result = crate::web::handlers::post_plan_coding_final_confirm(
            axum::extract::State(fixture.state.clone()),
            axum::extract::Path((
                PROJECT_ID.to_string(),
                ISSUE_ID.to_string(),
                fixture.plan_id(),
            )),
        )
        .await
        .expect("single final confirm button must succeed");
        assert_eq!(result.outcome, "all_confirmed");
        assert_eq!(result.results.len(), 3);
        for item in &result.results {
            assert_eq!(item.status, "confirmed", "{item:?}");
        }
        for expected in [member(1), member(2), member(3)] {
            assert_eq!(attempt_status_for(&fixture, &expected), CodingAttemptStatus::Completed);
        }
    }

    /// ③④＋k3 波 4 残留：终止两阶段（未确认 428 零副作用→确认后整 plan
    /// 终止）；group scope 收口分支（多仓整 plan 终止主路径）；终止后旧
    /// 事实清理无幽灵通知、无新启、run 失效。
    #[tokio::test]
    async fn r4_e1_four_repository_termination_two_phase_no_ghost_no_new_starts() {
        use crate::product::models::automation::{
            AutomationAttentionFact, AutomationAttentionKind, AutomationAttentionStage,
        };

        let mut fixture = EnrolledGateFixture::new_four_repository_collection().await;
        fixture.auto_approve_plan().await;
        // 驱动到至少两桶已自动启动（真实 runner 在途/停等现场）。
        fixture
            .reconcile_until(|attempts| {
                attempts
                    .iter()
                    .filter(|attempt| attempt.start_claim.is_some())
                    .count() >= 2
            })
            .await;
        let app = crate::web::app::build_web_router(fixture.state.clone());

        // 播种当前绑定 plan 的旧事实（plan_question）：终止后必须清理，
        /// 不留幽灵通知。
        let store = crate::product::issue_automation_store::IssueAutomationStore::new(
            fixture.inner.paths.clone(),
        );
        let durable = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
        let binding = durable
            .binding_history
            .as_ref()
            .expect("versioned binding")
            .current
            .clone();
        let plan_id = durable.plan_id.clone().expect("bound plan");
        let session_id = durable.session_id.clone().expect("bound session");
        store
            .upsert_automation_attention_fact(
                PROJECT_ID,
                ISSUE_ID,
                &binding,
                AutomationAttentionFact {
                    key: format!("{plan_id}:cmd-r4-e1-seed:plan_question"),
                    kind: AutomationAttentionKind::PlanQuestion,
                    stage: AutomationAttentionStage::Plan,
                    plan_id: Some(plan_id.clone()),
                    session_id: Some(session_id.clone()),
                    attempt_id: None,
                    reason_code: "r4-e1-seed".to_string(),
                    occurred_at: "2026-10-16T00:00:00Z".to_string(),
                    choice_id: None,
                    gate_id: None,
                },
            )
            .unwrap();
        assert_eq!(
            store
                .get(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .unwrap()
                .attention_facts
                .len(),
            1
        );

        // 阶段一：未带 confirmed:true → 428 automation_plan_termination_required，
        // 零副作用（enrollment 未终止、attempt 状态不变）。
        let before: Vec<(String, CodingAttemptStatus)> = fixture
            .coding_attempts()
            .iter()
            .map(|attempt| (attempt.id.clone(), attempt.status.clone()))
            .collect();
        let unconfirmed = post_terminate(
            &app,
            &serde_json::json!({
                "command_id": "cmd-r4-e1-terminate-1",
                "expected_enrollment_id": durable.enrollment_id,
                "expected_policy_revision": durable.policy_revision,
            }),
        )
        .await;
        assert_eq!(unconfirmed.status(), axum::http::StatusCode::PRECONDITION_REQUIRED);
        let payload = crate::web::handlers::automation_enrollment_test_support::response_json(
            unconfirmed,
        )
        .await;
        assert_eq!(payload["code"], "automation_plan_termination_required");
        assert_eq!(payload["details"]["confirm_required"], true);
        assert_eq!(payload["details"]["plan_id"], plan_id.as_str());
        let unchanged = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
        assert!(unchanged.enabled);
        assert!(unchanged.terminated_at.is_none());
        let after: Vec<(String, CodingAttemptStatus)> = fixture
            .coding_attempts()
            .iter()
            .map(|attempt| (attempt.id.clone(), attempt.status.clone()))
            .collect();
        assert_eq!(before, after, "unconfirmed termination has zero side effects");

        // 阶段二：确认后整 plan 终止——group scope 专用收口（全部 group
        // attempt 干净退出、未完成桶 Aborted、文件留存）、run 失效、旧事实
        // 清理（无幽灵通知）。
        let confirmed = post_terminate(
            &app,
            &serde_json::json!({
                "command_id": "cmd-r4-e1-terminate-2",
                "expected_enrollment_id": durable.enrollment_id,
                "expected_policy_revision": durable.policy_revision,
                "confirmed": true,
            }),
        )
        .await;
        assert_eq!(confirmed.status(), axum::http::StatusCode::OK);
        let payload =
            crate::web::handlers::automation_enrollment_test_support::response_json(confirmed)
                .await;
        assert_eq!(payload["outcome"], "terminated", "{payload}");
        assert_eq!(payload["state"], "accepted");
        let terminated = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
        assert!(!terminated.enabled);
        assert!(terminated.terminated_at.is_some());
        let run = crate::product::coding_plan_run::CodingPlanRunStore::new(
            fixture.inner.paths.clone(),
        )
        .get(PROJECT_ID, ISSUE_ID, &plan_id)
        .unwrap()
        .expect("durable run");
        assert_eq!(
            run.phase,
            crate::product::coding_plan_run::CodingPlanRunPhase::Invalidated
        );
        for attempt in fixture.coding_attempts() {
            assert_eq!(
                attempt.status,
                CodingAttemptStatus::Aborted,
                "unfinished group bucket {} must be finalized Aborted (group scope branch)",
                attempt.id
            );
        }
        // 旧事实清理：终止后无幽灵通知（mapper 只读 enrollment 事实）。
        let cleared = store.get(PROJECT_ID, ISSUE_ID).unwrap().unwrap();
        assert!(
            cleared.attention_facts.is_empty(),
            "termination must clear the bound plan's stale attention facts: {:?}",
            cleared.attention_facts
        );
        assert!(
            crate::web::automation_attention_info::issue_automation_attention_items(
                &fixture.inner.paths.clone(),
                PROJECT_ID,
                ISSUE_ID,
            )
            .unwrap()
            .is_empty(),
            "no ghost automation notifications after termination"
        );

        // 终止后无新启：重复 reconcile 对 terminated 返回 NoEnrollment、
        // fan-out 驱动零启动、无新 attempt。
        let claims_before: usize = fixture
            .coding_attempts()
            .iter()
            .filter(|attempt| attempt.start_claim.is_some())
            .count();
        let attempts_before = fixture.coding_attempts().len();
        let worker = AutopilotOrchestrator::new(fixture.state.clone(), Default::default());
        for _ in 0..3 {
            assert_eq!(
                worker.reconcile(&fixture.state, PROJECT_ID, ISSUE_ID).await.unwrap(),
                ReconcileOutcome::NoEnrollment,
                "terminated enrollment must never restart the chain"
            );
        }
        crate::web::handlers::drive_ready_units_for_plan(
            &fixture.state,
            PROJECT_ID,
            ISSUE_ID,
            &plan_id,
            None,
        )
        .await
        .expect("fan-out drive after termination observes zero ready starts");
        assert_eq!(fixture.coding_attempts().len(), attempts_before);
        assert_eq!(
            fixture
                .coding_attempts()
                .iter()
                .filter(|attempt| attempt.start_claim.is_some())
                .count(),
            claims_before,
            "no new starts after termination"
        );
    }

    async fn post_terminate(
        app: &axum::Router,
        body: &serde_json::Value,
    ) -> axum::http::Response<axum::body::Body> {
        use tower::ServiceExt;

        app.clone()
            .oneshot(
                axum::http::Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/automation-enrollment/terminate"
                    ))
                    .header("content-type", "application/json")
                    .body(axum::body::Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }
}
