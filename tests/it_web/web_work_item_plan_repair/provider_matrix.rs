#[tokio::test]
async fn work_item_plan_repair_provider_matrix_runs_parses_routes_and_persists() {
    use cadence_aria::product::models::{PlanDefectClass, PlanDefectRoute, ProviderName};

    for provider in [ProviderName::ClaudeCode] {
        let root = tempdir().expect("provider matrix root");
        let runtime =
            PlanRepairFixtureRuntime::seed(root.path(), PlanRepairFixtureControl::default())
                .await
                .expect("seed provider matrix fixture");

        let result = runtime
            .run_provider_matrix(provider.clone())
            .await
            .expect("run provider matrix");

        assert_eq!(result.provider, provider);
        assert!(result.rendered_contract_ids_preserved);
        assert!(result.author_contract_ids.contains(&"contract.workflow".to_string()));
        assert!(result.plan_review_passed);
        assert_eq!(result.coder_defect_class, PlanDefectClass::UpstreamContractInvalid);
        assert_eq!(
            result.code_review_defect_class,
            PlanDefectClass::UpstreamContractInvalid
        );
        assert_eq!(result.code_review_route, PlanDefectRoute::PlanRepair);
        assert!(result.author_draft_artifact_persisted);
        assert!(result.plan_review_complete_event_observed);
        assert_eq!(result.coding_role_run_count, 2);
        // Each Coder/CodeReviewer invocation persists both its raw stream attempt
        // and its terminal semantic output reference for auditability.
        assert_eq!(result.coding_raw_output_ref_count, 4);
    }

    // Codex 档（通则②迁移，Main 2026-11-04 批准；实测错误面）：
    // Codex 需要**用户级 workspace trust**，gateway 在 spawn 前复验；系统按
    // 方案 A「不自动登记、只给只读指导」（provider_trust.rs 的
    // `ReadonlyProviderTrustSource` + 失败文案：reason_code=trust_entry_missing、
    // 人工登记引导）。夹具**不得写用户 home**（~/.codex/config.toml），且生产
    // 侧尚无「测试用 trust source 注入」seam（块1 与本簇撞同一堵墙，已由 Main
    // 升级为优先立项项）⇒ Codex 档在现行契约下不可跑通全链，改钉新不变量：
    // 「未登记用户级 trust ⇒ fail-closed 且给出可操作人工登记指导」。
    let codex_error = {
        let root = tempdir().expect("provider matrix root");
        let runtime =
            PlanRepairFixtureRuntime::seed(root.path(), PlanRepairFixtureControl::default())
                .await
                .expect("seed provider matrix fixture");
        match runtime.run_provider_matrix(ProviderName::Codex).await {
            Ok(_) => panic!("Codex must not pass without a registered user-level trust entry"),
            Err(error) => error.to_string(),
        }
    };
    assert!(
        codex_error.contains("provider_trust_missing")
            && codex_error.contains("trust_entry_missing"),
        "Codex launch must fail closed on the user-level workspace trust gate: {codex_error}"
    );
    assert!(
        codex_error.contains("trust_level"),
        "the trust gate must hand back an actionable manual registration shape: {codex_error}"
    );

    // Fake 档（通则②迁移，Main 2026-11-04 批准；实测错误面）：
    // 逻辑 target（attempt 带冻结快照）的 provider 启动恒经
    // `LogicalCodebaseProviderGateway`（provider_stream.rs 的
    // `logical_provider_gateway_required` 门），而
    // `ProviderRef::from_provider_name`（provider_gateway.rs:467-479）对 Fake
    // 一律 fail-closed：稳定码 `provider_unsupported_for_gateway_launch`，
    // 且明文「🔴 禁止 `_ => claude_code` 之类的静默回退」。
    // ⇒ Fake 档必须被 gateway 以该稳定码拒绝，**不得**回退成「Fake 也能跑
    // 完整链」，也**不得**被悄悄换成别的 provider。
    let fake_error = {
        let root = tempdir().expect("provider matrix root");
        let runtime =
            PlanRepairFixtureRuntime::seed(root.path(), PlanRepairFixtureControl::default())
                .await
                .expect("seed provider matrix fixture");
        match runtime.run_provider_matrix(ProviderName::Fake).await {
            Ok(_) => panic!("Fake must be rejected by the logical provider gateway"),
            Err(error) => error.to_string(),
        }
    };
    assert!(
        fake_error.contains("provider_unsupported_for_gateway_launch"),
        "Fake launch must fail closed with the stable gateway code: {fake_error}"
    );
    assert!(
        fake_error.contains("Fake"),
        "the gateway rejection must name the rejected provider: {fake_error}"
    );
}
