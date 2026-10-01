/// P2 Task 1 共享 fixture：真实 enrollment 绑定 + compile 崩溃人工恢复 +
/// 确认链完整走完——只保证 durable Confirmed/已发布 compile，不冒称 Ready。
/// advance 的 fork 基线解析在唯一 logical target 的 physical checkout 上
/// 跑真实 git（三面同源 main→master 默认链），因此补真实 main 仓库。
pub(crate) async fn confirmed_enrolled_fixture() -> EnrolledGateFixture {
    let mut fixture = EnrolledGateFixture::new().await;
    init_real_main_checkout(&fixture.inner.paths.root().join("checkout-enroll-a"));
    normalize_checkout_revision_to_unobserved(&fixture.inner.paths);
    fixture.fail_compile_after_human_approve().await;
    fixture.recover_and_confirm_compile().await;
    fixture
}

/// C5 Task 4 共享 fixture：单仓 Confirmed enrollment——干净人工批准
///（无 failpoint，与 campaign 确认链同源）；Legacy compile 使全部 plan
/// 单位无 logical target 归属，attempt 天然无 target_snapshot。
pub(crate) async fn confirmed_single_repository_enrolled_fixture() -> EnrolledGateFixture {
    let mut fixture = EnrolledGateFixture::new_single_repository().await;
    fixture.confirm_plan_by_human().await;
    fixture
}

/// C5 Task 5 共享 fixture：单仓 Confirmed enrollment 经 Task 4 双分支
/// enrolled advance 到 durable Ready——唯一 attempt 无 target_snapshot、
/// 冻结 AutoStartOnce，尚无任何 runner/provider 启动。
pub(crate) async fn ready_single_repository_attempt_fixture() -> EnrolledGateFixture {
    let fixture = confirmed_single_repository_enrolled_fixture().await;
    let enrollment = fixture.enrollment();
    let plan_id = enrollment.plan_id.clone().expect("bound plan");
    let input = crate::product::advance_store::AdvanceInput {
        command_id: format!("wiga-advance-{}-{plan_id}", enrollment.enrollment_id),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id,
    };
    let outcome = crate::web::advance_plan::advance_plan(
        &fixture.state,
        input,
        crate::web::advance_plan::AdvancePlanOrigin::Enrolled {
            enrollment_id: enrollment.enrollment_id.clone(),
            policy_revision: enrollment.policy_revision,
        },
    )
    .await
    .expect("single-repository enrolled advance to ready");
    assert!(
        matches!(
            outcome,
            crate::product::advance_store::AdvanceOutcome::Completed { .. }
        ),
        "single-repository enrolled advance must complete: {outcome:?}"
    );
    assert!(
        fixture.attempt().target_snapshot.is_none(),
        "single-repository ready attempt must not carry a logical target snapshot"
    );
    fixture
}

/// P2 Task 4 共享 fixture：Confirmed enrollment 经 Task 1 自动 advance
/// 到 durable Ready——唯一 attempt 沿真实 journal lineage 创建、冻结
/// AutoStartOnce policy，尚无任何 runner/provider 启动。
pub(crate) async fn ready_enrolled_attempt_fixture() -> EnrolledGateFixture {
    let fixture = confirmed_enrolled_fixture().await;
    let enrollment = fixture.enrollment();
    let plan_id = enrollment.plan_id.clone().expect("bound plan");
    let input = crate::product::advance_store::AdvanceInput {
        command_id: format!("wiga-advance-{}-{plan_id}", enrollment.enrollment_id),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id,
    };
    let outcome = crate::web::advance_plan::advance_plan(
        &fixture.state,
        input,
        crate::web::advance_plan::AdvancePlanOrigin::Enrolled {
            enrollment_id: enrollment.enrollment_id.clone(),
            policy_revision: enrollment.policy_revision,
        },
    )
    .await
    .expect("enrolled advance to ready");
    assert!(
        matches!(
            outcome,
            crate::product::advance_store::AdvanceOutcome::Completed { .. }
        ),
        "enrolled advance must complete: {outcome:?}"
    );
    fixture
}

/// P2 Task 8 共享 fixture：接 Task 4 已 Ready/claimed 的 attempt，经真实
/// typed StartCoding（AutoStartOnce origin、stable command id）放行 Fake
/// runner，沿实际 unit→handoff→review→readiness 业务入口跑到
/// `WaitingForHuman + FinalConfirm`。失败即暴露真实缺失的 group 事实，
/// 不手改 attempt status、不从别的 fixture 拷贝 snapshot。
pub(crate) async fn complete_enrolled_group_waiting_for_final_confirm() -> EnrolledGateFixture {
    let fixture = ready_enrolled_attempt_fixture().await;
    let attempt = fixture.attempt();
    let outcome = crate::web::coding_start::start_coding_once(
        &fixture.state,
        PROJECT_ID,
        ISSUE_ID,
        crate::web::coding_start::StartCodingCommand {
            attempt_id: attempt.id.clone(),
            command_id: format!("wiga-start-{}", attempt.id),
            origin: fixture.auto_origin(),
        },
    )
    .await
    .expect("enrolled auto first start");
    assert!(
        matches!(
            outcome,
            crate::web::coding_start::StartCodingOutcome::Started { .. }
        ),
        "fake campaign must first-start exactly once: {outcome:?}"
    );
    tokio::time::timeout(std::time::Duration::from_secs(150), async {
        loop {
            let current = fixture.attempt();
            if current.status
                == crate::product::coding_models::CodingAttemptStatus::WaitingForHuman
                && current.stage
                    == crate::product::coding_models::CodingExecutionStage::FinalConfirm
            {
                break;
            }
            assert!(
                !matches!(
                    current.status,
                    crate::product::coding_models::CodingAttemptStatus::Failed
                        | crate::product::coding_models::CodingAttemptStatus::Aborted
                        | crate::product::coding_models::CodingAttemptStatus::AwaitingManualRecovery
                ),
                "fake campaign stopped unexpectedly at {:?}/{:?} reason={:?}",
                current.status,
                current.stage,
                current.manual_recovery_reason
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("fake campaign reaches human FinalConfirm");
    fixture
}
