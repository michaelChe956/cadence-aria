
// ---------------------------------------------------------------------------
// A03：LC 冷启动 → 中断 → 显式续跑 → planning_ready → 重放。
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a03_new_lc_reaches_planning_ready_without_manual_seed_or_duplicate_provider_turn() {
    let (app, factory, root_path) = a03_app().await;
    let aggregate_root = root_path.join("aggregate-root");
    let member_a = aggregate_root.join("alpha");
    let member_b = aggregate_root.join("beta");
    // 裸成员仓(fix-admission-member-rules-scope):无任何 .claude/rules 材料
    // ——路B 验收锚:bootstrap 凭据豁免成员规则门,根材料由 recipe 生成,
    // 成员文件树/Git 状态零写(不预写规则文件,不靠成员材料证明根有效)。
    for member in [&member_a, &member_b] {
        git_repo_at(member);
        git_commit_initial(member);
    }
    let head_a = git_head(&member_a);
    let head_b = git_head(&member_b);

    // 零手工 seed：全部事实经 REST 产品动作产生。
    let lc_id =
        register_lc_members(&app, &aggregate_root, &[member_a.clone(), member_b.clone()]).await;

    // 登记产物投影：identity/manifest 由登记提交同步完成（resume 产物）。
    // REQ-REG-10 起登记提交成功即后台自动推进首建链（rules_policy prepare
    // → 根配方 operation），后三步不再有可稳定观察的「全 not_started /
    // planning_ready==false」窗口——过时前提删除，其状态改由下方
    // 「注入中断 → failed 收敛」统一断言。
    let uri = bootstrap_uri(&lc_id);
    let initial = poll_until(
        &app,
        &uri,
        |projection| {
            bootstrap_step(projection, "identity")["status"] == "completed"
                && bootstrap_step(projection, "manifest_checkout")["status"] == "completed"
        },
        "initial registration facts",
    )
    .await;
    let membership_revision = initial["membership_revision"].as_u64().expect("revision");
    let authority_root = initial["authority_root"]
        .as_str()
        .expect("authority root")
        .to_string();
    let canonical_aggregate_root =
        std::fs::canonicalize(&aggregate_root).expect("canonicalize aggregate root");
    assert_eq!(
        authority_root,
        canonical_aggregate_root.to_string_lossy().to_string(),
        "authority root must freeze the manifest aggregate root"
    );

    // deterministic checkpoint：首建 run 由 REQ-REG-10 登记后自动推进驱动
    // （唯一根配方 operation，idempotency_key 服务端派生；不再手工 POST
    // /initializations——那会开出第二个 operation，其完成态会把本检查点的
    // failed 投影覆盖成 completed）。首个 provider turn 恒为 pre_check：
    // 一次性中断被该 run 消费 → operation Failed，且自动推进对失败配方
    // 绝不重试（失败步骤停住等待人工）；provider 启动计数仍为 0（中断
    // 先于成功启动）。
    let interrupted = poll_until(
        &app,
        &uri,
        |projection| bootstrap_step(projection, "member_index")["status"] == "failed",
        "member_index fails at the injected pre_check interruption",
    )
    .await;
    let member_index = bootstrap_step(&interrupted, "member_index");
    assert_eq!(
        member_index["failure"]["reason_code"], "aggregate_pre_check_failed",
        "failure must name the interrupted turn: {member_index}"
    );
    // operation 身份从投影取回：自动推进派生的 idempotency_key 对应唯一
    // 根配方 operation（member_index 步 object_id = 最新 operation id），
    // 下方显式 Retry 以 expected_object_id 精确钉住它。
    let operation_id = member_index["object_id"]
        .as_str()
        .expect("auto-advanced root recipe operation id")
        .to_string();
    assert_eq!(factory.audit().stream_launches(), 0);

    // “关闭页面”后再 GET：纯投影，不推进、不重复执行（两次 GET 语义一致，
    // provider 计数不变）。
    let reread = request_json(&app, Method::GET, &uri, json!({})).await;
    assert_eq!(reread.0, StatusCode::OK);
    assert_eq!(
        bootstrap_step(&reread.1, "member_index")["status"],
        "failed"
    );
    assert_eq!(factory.audit().stream_launches(), 0);

    // Task 4（生产同构）：根规则材料（AGENTS + .claude/rules/language.md）
    // 由 provider 在 RuleAndMcpConfig 轮生成（fixture 按轮次合同在第二个
    // 成功 turn 落盘，见 FaultOncePreCheckStreamingProvider）——中断窗口
    // 无需再手工预写 AGENTS；重试的命令审计窗口将其作为 allowlisted
    // 变更冻结，末命令收口发布真正文并冻结最终 receipt。成员仓保持裸态
    // 全程（bootstrap 凭据豁免成员规则门，不写成员规则）。

    // 从 REST 点击带 expected revision 的显式 Retry：原编排链从 checkpoint
    // 续跑（已完成 machine_skills/aggregate_preflight 不重跑，pre_check 及其
    // 后续 turn 各启动一次）。
    let action_uri = format!("{}/actions", bootstrap_uri(&lc_id));
    let command_id = "cmd-a03-member-index-retry-1";
    let (status, result) = request_json(
        &app,
        Method::POST,
        &action_uri,
        json!({
            "command_id": command_id,
            "step": "member_index",
            "action": "retry",
            "expected_revision": membership_revision,
            "expected_object_id": operation_id,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert_eq!(result["outcome"], "accepted");

    let ready = poll_until(
        &app,
        &uri,
        |projection| projection["planning_ready"] == true,
        "bootstrap reaches planning_ready after the explicit retry",
    )
    .await;
    for step in [
        "identity",
        "manifest_checkout",
        "rules_policy",
        "member_index",
        "aggregate_index_active",
    ] {
        assert_eq!(
            bootstrap_step(&ready, step)["status"],
            "completed",
            "step {step} must complete: {ready}"
        );
    }
    // 唯一 authority + 冻结 policy digest。
    assert_eq!(
        ready["authority_root"].as_str().unwrap(),
        authority_root,
        "authority root must stay frozen on the same per-LC subtree"
    );
    let policy_digest = ready["policy"]["policy_digest"]
        .as_str()
        .expect("policy digest")
        .to_string();
    assert!(policy_digest.starts_with("sha256:"));

    // 恰好三个 provider turn（pre_check 重试 + rule_and_mcp_config +
    // openspec_and_examples）；已完成 provider turn 计数不增加。
    assert_eq!(factory.audit().stream_launches(), 3);

    // 成员仓零 Git 写副作用,且全程无任何规则文件被写入(裸成员零写:
    // bootstrap 凭据豁免成员规则门,不复制/生成成员规则)。
    assert!(!member_a.join(".claude/rules/language.md").exists());
    assert!(!member_b.join(".claude/rules/language.md").exists());
    assert_eq!(git_porcelain(&member_a), "");
    assert_eq!(git_porcelain(&member_b), "");
    assert_eq!(git_head(&member_a), head_a);
    assert_eq!(git_head(&member_b), head_b);

    // 重放同一 command：replayed，同 operation/index 身份，provider 计数不变。
    let (status, replay) = request_json(
        &app,
        Method::POST,
        &action_uri,
        json!({
            "command_id": command_id,
            "step": "member_index",
            "action": "retry",
            "expected_revision": membership_revision,
            "expected_object_id": operation_id,
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay["outcome"], "replayed");
    assert_eq!(replay["projection"]["planning_ready"], true);
    assert_eq!(
        bootstrap_step(&replay["projection"], "member_index")["object_id"],
        operation_id
    );
    assert_eq!(factory.audit().stream_launches(), 3);
}
