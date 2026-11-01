fn a12_existing_create_intent_contract_chain() {
    let baseline = c1_serial_baseline_tree();

    // 1. 合法 create：编译通过 + 意图随 canonical contract 持久化。
    let ir = c1_serial_compile(C1_SERIAL_CREATE_INTENT);
    let intent = ir.items[0]
        .contract
        .intent_contract
        .as_ref()
        .expect("intent contract lowered onto canonical contract");
    assert_eq!(
        intent.intent,
        crate::product::work_item_contract::WorkItemIntent::Create
    );
    assert_eq!(intent.provider_work_item_id, "WI-002");
    assert_eq!(intent.exclusive_scopes, vec!["src/levels_new/**".to_string()]);
    let report = validate_plan_candidate_ir(&ir, &c1_serial_context(&[], Some(&baseline)))
        .expect("declared create must validate");
    assert!(
        !report.has_errors(),
        "declared create findings must be Error-free: {:#?}",
        report.findings
    );

    // 2. 未声明：基线外新建写面 + 无 Plan Intent → intent_undeclared 停等
    //    （Error 保留在机械报告里等修订回灌，不产生 work item/child）。
    let ir = c1_serial_compile("");
    assert!(
        ir.items[0].contract.intent_contract.is_none(),
        "未声明意图时 contract 不得伪造 intent"
    );
    let diagnostics = validate_work_item_intent_contract(
        &ir,
        &c1_serial_context(&[], Some(&baseline)),
    )
    .expect_err("undeclared baseline writes must fail intent validation");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "intent_undeclared"),
        "expected intent_undeclared, got {diagnostics:#?}"
    );
    let report = validate_plan_candidate_ir(&ir, &c1_serial_context(&[], Some(&baseline)))
        .expect("preflight-family findings stay in the mechanical report");
    assert!(report.has_errors());
    assert!(
        report
            .findings
            .iter()
            .any(|finding| finding.code == "intent_undeclared"
                && finding.work_item_ids.contains(&"WI-001".to_string())),
        "report must carry intent_undeclared for WI-001: {:#?}",
        report.findings
    );

    // 3. 不能执行：错 provider Work Item → intent_unexecutable 拒绝（既有
    //    finding 不清空，不扩大 scope）。
    let ir = c1_serial_compile(
        r#"
### Plan Intent
- intent: create
- provider_work_item_id: WI-404
- intent_target_kind: single_repository
- intent_target_repo: repo-c1
"#,
    );
    let diagnostics = validate_work_item_intent_contract(
        &ir,
        &c1_serial_context(&[], Some(&baseline)),
    )
    .expect_err("unknown provider must be unexecutable");
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == "intent_unexecutable" && d.message.contains("WI-404")),
        "got {diagnostics:#?}"
    );
}

// ============================================================================
// A13 —— 显式换代（REQ-WIGA-01/03、REQ-C1-CHILD-01 的换代面）。
// 复用 confirmed_enrolled_fixture（真实 enrollment + Confirmed 链）：
// v1 唯一 attempt → 显式 rebind v2 → 旧代 advance/start 回执 fail-closed、
// 历史可查、无第二 attempt/provider；Manual 人工路径 replay 同一 attempt。
// compile child 跨代新建/旧 child 只读由 part_11 定向测试持有。
// ============================================================================

async fn a13_generation_switch_chain() {
    let fixture = crate::web::wiga_gate_fixture::confirmed_enrolled_fixture().await;
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
    let v1_advance = crate::web::advance_plan::advance_plan(
        &fixture.state,
        AdvanceInput {
            command_id: format!("wiga-advance-{}-{plan_1}", enrollment.enrollment_id),
            project_id: enrollment.project_id.clone(),
            issue_id: enrollment.issue_id.clone(),
            plan_id: plan_1.clone(),
        },
        crate::web::advance_plan::AdvancePlanOrigin::Enrolled {
            enrollment_id: enrollment.enrollment_id.clone(),
            policy_revision: enrollment.policy_revision,
        },
    )
    .await
    .expect("v1 advance under current binding");
    assert!(matches!(v1_advance, AdvanceOutcome::Completed { .. }));
    assert_eq!(fixture.coding_attempts().len(), 1);
    let v1_attempt_id = fixture.attempt().id.clone();

    // 显式换代 v2：新 plan/session 属于同一 issue。
    let (plan_2, session_2) = create_plan_and_session(&fixture.inner, RunPolicy::Interactive);
    let store = IssueAutomationStore::new(ProductAppPaths::new(
        fixture.state.workspace_root.join(".aria"),
    ));
    let rebind = store
        .rebind(
            &enrollment.project_id,
            &enrollment.issue_id,
            EnrollmentRebindRequest {
                command_id: "cmd-c1-serial-a13-rebind".to_string(),
                expected_policy_revision: enrollment.policy_revision,
                expected_binding_version: binding_v1.binding_version,
                binding: EnrollmentBindingIdentityInput {
                    plan_id: plan_2.clone(),
                    session_id: session_2,
                    source: enrollment.source.clone(),
                    target: binding_v1.target.clone(),
                    author_provider: enrollment.options.author_provider.clone(),
                    reviewer_provider: enrollment.options.reviewer_provider.clone(),
                },
                reason: "c1 serial generation switch".to_string(),
            },
        )
        .expect("rebind to v2");
    let after = rebind.enrollment;
    let history = after.binding_history.as_ref().expect("binding history");
    assert_eq!(history.current.binding_version, 2);
    // 旧 binding/历史可查：previous 保留完整 v1 身份。
    assert_eq!(history.previous.len(), 1);
    assert_eq!(history.previous[0].binding_version, 1);
    assert_eq!(history.previous[0].plan_id, plan_1);

    // 旧代 advance 回执 fail-closed：无第二 attempt。
    let v1_receipt = crate::web::advance_plan::advance_plan(
        &fixture.state,
        AdvanceInput {
            command_id: format!("wiga-advance-{}-{plan_1}", enrollment.enrollment_id),
            project_id: enrollment.project_id.clone(),
            issue_id: enrollment.issue_id.clone(),
            plan_id: plan_1.clone(),
        },
        crate::web::advance_plan::AdvancePlanOrigin::Enrolled {
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

    // 旧代 start 回执 fail-closed：零 runner、不改当前代。
    let v1_start = crate::web::coding_start::start_coding_once(
        &fixture.state,
        &enrollment.project_id,
        &enrollment.issue_id,
        crate::web::coding_start::StartCodingCommand {
            attempt_id: v1_attempt_id,
            command_id: "cmd-c1-serial-a13-start-v1".to_string(),
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

    // Manual 人工路径不被 enrollment 校验拦截：同代 durable replay 返回
    // 同一 attempt（不建第二链）。
    let manual = crate::web::advance_plan::advance_plan(
        &fixture.state,
        AdvanceInput {
            command_id: format!("wiga-advance-{}-{plan_1}", enrollment.enrollment_id),
            project_id: enrollment.project_id.clone(),
            issue_id: enrollment.issue_id.clone(),
            plan_id: plan_1.clone(),
        },
        crate::web::advance_plan::AdvancePlanOrigin::Manual,
    )
    .await
    .expect("manual advance must not be blocked by enrollment binding checks");
    assert!(matches!(manual, AdvanceOutcome::Replayed { .. }));
    assert_eq!(fixture.coding_attempts().len(), 1);
    assert_eq!(fixture.coding_runner_count(), 0);
}
