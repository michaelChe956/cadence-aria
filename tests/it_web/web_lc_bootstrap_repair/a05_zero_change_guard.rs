
// ---------------------------------------------------------------------------
// REQ-BOOT-17 负例（it_web 级回归）：零动作的 provider turn 必须让该 step
// 判失败（cause=root_recipe_turn_zero_change）——零变更 turn 绝不静默通过。
// 正例（A03/A04）的夹具按真实四命令落盘面产生变更，故本负例是「零变更 ⇒
// 失败」这条契约在 HTTP 面的唯一护栏。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a05_zero_change_pre_check_turn_fails_the_step_with_durable_cause() {
    let (app, factory, root_path) = a03_app_with(RecipeProviderOptions {
        // 不注入中断：pre_check turn 正常结束（零动作）——失败原因必须是
        // 「零变更」，不得被中断注入混淆。
        interrupt_first_pre_check: false,
        zero_change_pre_check: true,
    })
    .await;
    let aggregate_root = root_path.join("aggregate-root");
    let member_a = aggregate_root.join("alpha");
    let member_b = aggregate_root.join("beta");
    for member in [&member_a, &member_b] {
        git_repo_at(member);
        git_commit_initial(member);
    }

    let lc_id =
        register_lc_members(&app, &aggregate_root, &[member_a.clone(), member_b.clone()]).await;
    let uri = bootstrap_uri(&lc_id);

    // 零动作的 pre_check turn ⇒ 该 step 判失败（不静默通过）。
    let failed = poll_until(
        &app,
        &uri,
        |projection| bootstrap_step(projection, "member_index")["status"] == "failed",
        "the zero-change pre_check turn fails its step",
    )
    .await;
    let member_index = bootstrap_step(&failed, "member_index");
    assert_eq!(
        member_index["failure"]["reason_code"], "aggregate_pre_check_failed",
        "the zero-change turn must fail its step: {member_index}"
    );
    // 零变更 ≠ 未启动：turn 确实启动过（否则失败原因会是启动失败）。
    assert_eq!(factory.audit().stream_launches(), 1);

    // durable 失败事实含 provider / step / 零变更三项。
    let operation_id = member_index["object_id"]
        .as_str()
        .expect("root recipe operation id")
        .to_string();
    let (status, operation) = request_json(
        &app,
        Method::GET,
        &format!(
            "/api/projects/{PROJECT_ID}/logical-codebases/{lc_id}/initializations/{operation_id}"
        ),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{operation}");
    assert_eq!(operation["error"]["code"], "aggregate_pre_check_failed");
    let details = &operation["error"]["details"];
    assert_eq!(
        details["cause"], "root_recipe_turn_zero_change",
        "the durable failure must name the zero-change cause: {operation}"
    );
    assert_eq!(details["provider"], "claude_code");
    let action = details["action"].as_str().unwrap_or("");
    assert!(
        action.contains("zero_change") && action.contains("pre_check"),
        "the failure fact must state the zero change and the step: {operation}"
    );

    // 后续 step 不推进：零动作 turn 绝不放行 readiness。
    assert_eq!(failed["planning_ready"], false, "{failed}");
    assert_ne!(
        bootstrap_step(&failed, "aggregate_index_active")["status"],
        "completed",
        "a zero-change turn must not advance the later steps: {failed}"
    );
}
