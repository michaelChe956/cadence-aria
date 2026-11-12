    // 片4（REQ-BOOT-07）provider 走 request 的行为用例：选择冻结落账、
    // 重试不换人、未知 provider 名 fail-closed。断言只依赖 coordinator 的
    // 公开行为与 durable 记录。

    /// 记录每轮 turn 实际使用的 provider（`request.provider`），可按需让
    /// 首个 pre_check turn 失败一次以制造可续跑的 Failed 现场。
    struct ProviderRecordingDriver {
        providers: Mutex<Vec<crate::product::models::ProviderName>>,
        fail_first_pre_check: std::sync::atomic::AtomicBool,
    }

    impl ProviderRecordingDriver {
        fn providers(&self) -> Vec<crate::product::models::ProviderName> {
            self.providers.lock().unwrap().clone()
        }

        /// 单发开关：下一个 pre_check turn 失败一次后自动复位。
        fn trip_first_pre_check(&self) {
            self.fail_first_pre_check
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl AggregateProviderTurnDriver for ProviderRecordingDriver {
        async fn run_turn(
            &self,
            request: AggregateProviderTurnRequest<'_>,
        ) -> Result<
            crate::product::logical_codebase::aggregate_initialization_coordinator::AggregateProviderTurnOutcome,
            AggregateInitializationError,
        > {
            let AggregateProviderTurnRequest {
                step, provider, ..
            } = request;
            self.providers.lock().unwrap().push(provider.clone());
            if self.fail_first_pre_check.swap(false, std::sync::atomic::Ordering::SeqCst)
                && step == AggregateInitializationStepKind::PreCheck
            {
                return Err(AggregateInitializationError::ProviderTurn {
                    step,
                    reason: "first pre-check turn failed for retry observation".to_string(),
                    retryable: true,
                });
            }
            Ok(crate::product::logical_codebase::aggregate_initialization_coordinator::AggregateProviderTurnOutcome {
                summary: format!("{} summary", step.as_str()),
                report: crate::product::logical_codebase::aggregate_initialization_coordinator::RecipeCommandReport::Absent,
            })
        }
    }

    use crate::product::logical_codebase::aggregate_initialization::AggregateProviderCandidate;

    /// 按 owner 顺序构造候选表（display_name/action_hint 只需稳定可读）。
    /// 候选行派生自 `AGGREGATE_PROVIDER_CANDIDATE_ORDER`：顺序/家数与生产
    /// 常量自洽，候选序列变更时本夹具无需再动。
    fn frozen_selection_table(
        selected: &str,
    ) -> crate::product::logical_codebase::AggregateProviderSelection {
        use crate::product::logical_codebase::provider_selection::AGGREGATE_PROVIDER_CANDIDATE_ORDER;
        use crate::product::logical_codebase::provider_trust::provider_wire_label;
        use crate::product::models::ProviderName;
        let display_name = |provider: &ProviderName| match provider {
            ProviderName::Pi => "Pi",
            ProviderName::ClaudeCode => "Claude Code",
            ProviderName::Codex => "Codex",
            _ => "Provider",
        };
        let candidates = AGGREGATE_PROVIDER_CANDIDATE_ORDER
            .iter()
            .map(|provider| {
                let wire = provider_wire_label(provider);
                AggregateProviderCandidate {
                    provider: wire.to_string(),
                    display_name: display_name(provider).to_string(),
                    cli_available: true,
                    cli_reason_code: None,
                    capability_state: if wire == selected {
                        "confirmed".to_string()
                    } else {
                        "needs_verification".to_string()
                    },
                    skipped_reason: (wire != selected).then(|| "needs_verification".to_string()),
                    action_hint: format!("hint:{wire}"),
                    warning: None,
                }
            })
            .collect();
        crate::product::logical_codebase::AggregateProviderSelection {
            selected: selected.to_string(),
            candidates,
            rule_version: "v1".to_string(),
            decided_at: CREATED_AT.to_string(),
        }
    }

    /// provider 参数化 fixture：manifest + 记录型 driver + 按 wire 冻结的
    /// operation（`with_selection=false` 走既有 `begin` 旧入口）。
    struct FrozenProviderFixture {
        _temp: tempfile::TempDir,
        driver: Arc<ProviderRecordingDriver>,
        coordinator: AggregateInitializationCoordinator,
    }

    fn frozen_provider_fixture(provider_wire: &str, with_selection: bool) -> FrozenProviderFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let mut manifest = LogicalCodebaseManifest::new(
            "project_0001",
            temp.path().join("aggregate-root"),
            Vec::new(),
        );
        manifest.created_at = CREATED_AT.to_string();
        manifest.updated_at = CREATED_AT.to_string();
        LogicalCodebaseStore::new(paths.clone())
            .save_manifest("project_0001", &manifest)
            .unwrap();
        let skills: Arc<dyn AggregateSkillsPreparation> = Arc::new(FakeSkillsPreparation {
            calls: Arc::new(Mutex::new(Vec::new())),
        });
        let preflight: Arc<dyn AggregatePreflightService> = Arc::new(FakePreflightService::new(
            Arc::new(Mutex::new(Vec::new())),
            manifest
                .provider_context_root
                .to_string_lossy()
                .into_owned(),
        ));
        let driver = Arc::new(ProviderRecordingDriver {
            providers: Mutex::new(Vec::new()),
            fail_first_pre_check: std::sync::atomic::AtomicBool::new(false),
        });
        let clock: Arc<Clock> = Arc::new(|| CREATED_AT.to_string());
        let coordinator = AggregateInitializationCoordinator::new(
            paths.clone(),
            AggregateInitializationOperationStore::new(paths.clone()),
            skills,
            preflight,
            driver.clone(),
            clock,
        );
        let input = AggregateInitializationOperationInput {
            idempotency_key: "0001".to_string(),
            manifest_revision: manifest.membership_revision,
            policy_digest: "sha256:policy".to_string(),
            profile_evidence_digest: Some("sha256:profile".to_string()),
            provider_context_root: manifest.provider_context_root.clone(),
            provider: provider_wire.to_string(),
        };
        if with_selection {
            coordinator
                .begin_with_selection(
                    "aggregate_initialization_0001".to_string(),
                    "project_0001",
                    input,
                    frozen_selection_table(provider_wire),
                    false,
                )
                .unwrap();
        } else {
            coordinator
                .begin(
                    "aggregate_initialization_0001".to_string(),
                    "project_0001",
                    input,
                )
                .unwrap();
        }
        FrozenProviderFixture {
            _temp: temp,
            driver,
            coordinator,
        }
    }

    #[test]
    fn begin_without_selection_freezes_none_and_with_selection_freezes_table() {
        // 落账回归：既有 `begin` 旧入口产出 provider_selection == None（现有
        // 调用点零迁移、零变化）；`begin_with_selection` 产出非 None 且候选表
        // 按 owner 顺序原样冻结。
        let legacy = frozen_provider_fixture("claude_code", false);
        let operation = legacy
            .coordinator
            .get("project_0001", "aggregate_initialization_0001")
            .unwrap();
        assert_eq!(operation.provider_selection, None);

        let frozen = frozen_provider_fixture("pi", true);
        let operation = frozen
            .coordinator
            .get("project_0001", "aggregate_initialization_0001")
            .unwrap();
        let selection = operation.provider_selection.expect("selection is frozen");
        assert_eq!(selection.selected, "pi");
        assert_eq!(
            selection
                .candidates
                .iter()
                .map(|candidate| candidate.provider.as_str())
                .collect::<Vec<_>>(),
            vec!["pi", "claude_code", "codex"],
            "candidates stay in owner order, never reordered by outcome"
        );
        assert!(selection
            .candidates
            .iter()
            .all(|candidate| !candidate.action_hint.is_empty()));
    }

    #[tokio::test]
    async fn frozen_provider_survives_execute_and_retry_without_reselection() {
        // REQ-BOOT-07「重试不换人」：provider 只从 durable 记录解出，
        // execute/execute_remaining 路径不重跑选择器——即便机器状态变化
        // （health 变成只有 claude_code 可用），每轮 turn 仍用冻结的 pi。
        let fixture = frozen_provider_fixture("pi", true);
        fixture.driver.trip_first_pre_check();
        let first = fixture
            .coordinator
            .execute(
                "project_0001",
                "aggregate_initialization_0001",
                CancellationToken::new(),
            )
            .await;
        assert!(
            first.is_err(),
            "first execute stops at the failed pre-check turn"
        );
        let operation = fixture
            .coordinator
            .get("project_0001", "aggregate_initialization_0001")
            .unwrap();
        assert_eq!(
            operation.status,
            AggregateInitializationOperationStatus::Failed
        );
        assert_eq!(
            operation.provider_selection.as_ref().unwrap().selected,
            "pi"
        );
        fixture
            .coordinator
            .execute_remaining(
                "project_0001",
                "aggregate_initialization_0001",
                CancellationToken::new(),
            )
            .await
            .unwrap();
        let providers = fixture.driver.providers();
        assert_eq!(
            providers.len(),
            4,
            "pre_check reruns once, then two more turns"
        );
        assert!(
            providers
                .iter()
                .all(|provider| *provider == crate::product::models::ProviderName::Pi),
            "every turn keeps using the frozen provider pi"
        );
    }

    #[tokio::test]
    async fn unknown_frozen_provider_fails_closed_without_fallback() {
        // REQ-BOOT-07 fail-closed：durable 记录里的 provider 名不是四家真实
        // provider 之一时，provider turn 拒绝启动（不回落默认 provider），
        // operation 判 Failed 且记录携带被拒的 provider 名。
        let fixture = frozen_provider_fixture("bogus", false);
        let result = fixture
            .coordinator
            .execute(
                "project_0001",
                "aggregate_initialization_0001",
                CancellationToken::new(),
            )
            .await;
        assert!(result.is_err());
        let operation = fixture
            .coordinator
            .get("project_0001", "aggregate_initialization_0001")
            .unwrap();
        assert_eq!(
            operation.status,
            AggregateInitializationOperationStatus::Failed
        );
        assert!(
            fixture.driver.providers().is_empty(),
            "no provider turn may start for an unknown frozen provider"
        );
        let record = operation.error.as_ref().expect("durable failure record");
        assert_eq!(record.provider.as_deref(), Some("bogus"));
        assert_eq!(record.reason_code, "aggregate_pre_check_failed");
        // 三个 provider turn 从未启动，全部保持 Pending。
        assert!(operation
            .steps
            .iter()
            .skip(2)
            .all(|step| step.status == AggregateInitializationStepStatus::Pending));
    }

/// 7.7（启动期 `ProviderAdapterError` → cause / diagnostic）：adapter 在
/// 启动面以带敏感内容的 stdout/stderr 拒绝时，durable 失败记录的
/// `cause` 必须是 adapter 的 code 子码（`ProviderErrorCode::as_str()`，
/// 不是通用串），`stderr_summary` 携带已脱敏的 stdout/stderr（凭据
/// `[REDACTED]`；home 前缀降级需带 home 的 `RedactionContext`，默认
/// context 与 B2S2 既有分支同限），且保留原文其余内容；常规
/// （事件流 `Failed`，非启动期）失败路径由既有 `StreamingBehavior::Fail`
/// 用例钉住，不受本改动影响。
#[tokio::test]
async fn start_failing_adapter_records_code_subcode_and_sanitized_output() {
    let secret = "sk-live-abcdefghijklmnopqrst";
    let fixture = gateway_aggregate_fixture_with(
        StreamingBehavior::StartFail {
            stdout: "planning under ./workspace/project".to_string(),
            stderr: format!("api_token={secret} unauthorized"),
        },
        None,
    );
    let input = AggregateInitializationOperationInput {
        idempotency_key: "start-fail-0001".to_string(),
        ..fixture.recipe_input()
    };
    fixture
        .coordinator()
        .begin(
            "aggregate_initialization_start_fail".to_string(),
            "project_0001",
            input,
        )
        .unwrap();
    let result = fixture
        .coordinator()
        .execute(
            "project_0001",
            "aggregate_initialization_start_fail",
            CancellationToken::new(),
        )
        .await;
    assert!(result.is_err(), "a start-time adapter failure must fail the turn");
    let operation = fixture
        .coordinator()
        .get("project_0001", "aggregate_initialization_start_fail")
        .unwrap();
    let record = operation.error.as_ref().expect("durable failure record");
    assert_eq!(record.reason_code, "aggregate_pre_check_failed");
    assert_eq!(
        record.cause.as_deref(),
        Some("provider_unauthorized"),
        "cause must be the adapter code subcode, not a generic string"
    );
    let output = record.stderr_summary.as_deref().expect("diagnostic output");
    assert!(
        output.contains("planning under"),
        "the stdout tail must survive sanitization: {output}"
    );
    assert!(
        !output.contains(secret),
        "credentials must be redacted: {output}"
    );
    assert!(
        output.contains("[REDACTED]"),
        "the credential assignment must be visibly redacted: {output}"
    );
    assert!(
        output.contains("unauthorized"),
        "the stderr tail must survive sanitization: {output}"
    );
}
