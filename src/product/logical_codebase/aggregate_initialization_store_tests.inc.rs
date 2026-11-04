#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::audit_snapshot_budget::{
        AuditSnapshotBudgetFailure, BudgetGroup, MemberBudgetSummary,
    };
    use crate::product::json_store::ProductStoreError;
    use crate::product::logical_codebase::aggregate_initialization::{
        AggregateInitializationErrorRecord, AggregateInitializationIdempotencyIdentity,
        AggregateInitializationOperationInput, AggregateInitializationStepKind,
        AggregateProviderCandidate, AggregateProviderSelection,
    };
    use crate::product::logical_codebase::aggregate_initialization_coordinator::{
        AggregateInitializationError, TurnDiagnostic,
    };
    use crate::product::logical_codebase::provider_trust::ProviderTrustWaiting;
    use crate::product::models::provider::ProviderName;
    use crate::product::repository_store::{
        RepositoryInitializationOperationStore, RepositoryInitializationStepKind,
    };

    const CREATED_AT: &str = "2026-08-09T00:00:00Z";
    const RUNNING_AT: &str = "2026-08-09T00:00:01Z";
    const STEP_AT: &str = "2026-08-09T00:00:10Z";

    #[test]
    fn root_recipe_command_index_freezes_four_commands_in_order() {
        let index = super::root_recipe_command_index();
        let flattened: Vec<(String, usize, &str)> = index
            .into_iter()
            .map(|(step, command_index, command)| {
                (step.as_str().to_string(), command_index, command)
            })
            .collect();
        assert_eq!(
            flattened,
            vec![
                (
                    "pre_check".to_string(),
                    1,
                    "/pre-check --no-interrupt --upgrade 用大陆镜像"
                ),
                (
                    "rule_and_mcp_config".to_string(),
                    2,
                    "/rule-config --no-interrupt"
                ),
                (
                    "rule_and_mcp_config".to_string(),
                    3,
                    "/mcp-configuration --no-interrupt"
                ),
                (
                    "openspec_and_examples".to_string(),
                    4,
                    "/project-rules-examples --no-interrupt"
                ),
            ]
        );
    }

    struct AggregateInitFixture {
        _temp: tempfile::TempDir,
        paths: ProductAppPaths,
        store: AggregateInitializationOperationStore,
        provider_turns: std::sync::Mutex<u32>,
        operation_id: String,
    }

    impl AggregateInitFixture {
        fn store(&self) -> &AggregateInitializationOperationStore {
            &self.store
        }

        fn now(&self) -> String {
            STEP_AT.to_string()
        }

        /// A counter that the coordinator would increment on every provider
        /// turn. The store's recovery path must never touch a provider, so
        /// `turn_count()` staying at 0 after `recover_interrupted` proves no
        /// auto-restart happened.
        fn provider(&self) -> ProviderTurnProbe<'_> {
            ProviderTurnProbe {
                turns: &self.provider_turns,
            }
        }

        /// The operation-owned staging directory, mirroring the private
        /// `AggregateInitializationOperationStore::staging_path` layout so the
        /// test can seed and assert against the exact path the store cleans.
        fn staging_root(&self) -> PathBuf {
            self.paths
                .aggregate_initializations_root("project_0001")
                .join(&self.operation_id)
                .join("staging")
        }

        /// Write a partial staging artifact as an interrupted provider turn
        /// would, then leave the operation persisted as `Running` at `step`.
        fn persist_running_at(&self, step: AggregateInitializationStepKind) {
            let operation = self
                .store()
                .create_idempotent(self.new_operation("0001"))
                .unwrap();
            self.store()
                .mark_running("project_0001", &operation.operation_id, RUNNING_AT.into())
                .unwrap();
            for preceding in AggregateInitializationStepKind::V1 {
                if preceding == step {
                    break;
                }
                self.run_step(&operation.operation_id, preceding);
            }
            self.store()
                .mark_step_running(
                    "project_0001",
                    &operation.operation_id,
                    step,
                    format!(
                        "aggregate-init:project_0001:{}:{}:input",
                        operation.operation_id,
                        step.as_str()
                    ),
                    STEP_AT.to_string(),
                )
                .unwrap();
        }

        /// Seed a partial staging file inside the operation-owned staging
        /// directory, simulating an interrupted provider turn that wrote
        /// partial output before being killed.
        fn write_staging(&self, relative: &str) {
            let path = self.staging_root().join(relative);
            std::fs::create_dir_all(path.parent().expect("staging relative path has parent"))
                .unwrap();
            std::fs::write(&path, b"partial").unwrap();
        }

        fn new_operation(&self, idempotency_key: &str) -> AggregateInitializationOperation {
            AggregateInitializationOperation::new(
                format!("aggregate_initialization_{idempotency_key}"),
                "project_0001".to_string(),
                self.input(idempotency_key),
                CREATED_AT.to_string(),
            )
        }

        fn input(&self, idempotency_key: &str) -> AggregateInitializationOperationInput {
            AggregateInitializationOperationInput {
                idempotency_key: idempotency_key.to_string(),
                manifest_revision: 1,
                policy_digest: "sha256:policy".to_string(),
                profile_evidence_digest: Some("sha256:profile".to_string()),
                provider_context_root: self._temp.path().join("aggregate-root"),
                provider: "claude_code".to_string(),
            }
        }

        fn run_step(
            &self,
            operation_id: &str,
            step: AggregateInitializationStepKind,
        ) -> AggregateInitializationOperation {
            self.store
                .mark_step_running(
                    "project_0001",
                    operation_id,
                    step,
                    format!(
                        "aggregate-init:project_0001:{operation_id}:{}:input",
                        step.as_str()
                    ),
                    STEP_AT.to_string(),
                )
                .unwrap();
            self.store
                .checkpoint_step_output(
                    "project_0001",
                    operation_id,
                    step,
                    format!(
                        "aggregate-initializations/{operation_id}/{}.json",
                        step.as_str()
                    ),
                    STEP_AT.to_string(),
                )
                .unwrap();
            self.store
                .mark_step_completed("project_0001", operation_id, step, STEP_AT.to_string())
                .unwrap()
        }

        /// Try to read the aggregate operation record through the legacy
        /// single-repository operation store; it must reject the aggregate
        /// layout rather than silently accept it.
        fn repository_operation_store_rejects_aggregate_layout(
            &self,
            operation: &AggregateInitializationOperation,
        ) -> bool {
            let legacy = RepositoryInitializationOperationStore::new(self.paths.clone());
            matches!(
                legacy.get("project_0001", &operation.operation_id),
                Err(ProductStoreError::NotFound { .. })
                    | Err(ProductStoreError::IdentityMismatch { .. })
            )
        }
    }

    /// Lightweight probe over the fixture's provider-turn counter. The store's
    /// recovery path never invokes a provider; `turn_count()` staying at 0 is
    /// the proof that `recover_interrupted` does not auto-restart.
    struct ProviderTurnProbe<'a> {
        turns: &'a std::sync::Mutex<u32>,
    }

    impl ProviderTurnProbe<'_> {
        fn turn_count(&self) -> u32 {
            *self
                .turns
                .lock()
                .expect("provider turn probe mutex poisoned")
        }
    }

    fn aggregate_init_fixture() -> AggregateInitFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let store = AggregateInitializationOperationStore::new(paths.clone());
        AggregateInitFixture {
            _temp: temp,
            paths,
            store,
            provider_turns: std::sync::Mutex::new(0),
            operation_id: "aggregate_initialization_0001".to_string(),
        }
    }

    #[test]
    fn aggregate_layout_is_exactly_five_steps_and_rejects_jump_or_reorder() {
        let fixture = aggregate_init_fixture();
        let operation = fixture
            .store()
            .create_idempotent(fixture.new_operation("request-a"))
            .unwrap();
        assert_eq!(
            operation
                .steps
                .iter()
                .map(|step| step.step_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "machine_skills",
                "aggregate_preflight",
                "pre_check",
                "rule_and_mcp_config",
                "openspec_and_examples",
            ]
        );
        fixture
            .store()
            .mark_running("project_0001", &operation.operation_id, RUNNING_AT.into())
            .unwrap();
        // No preceding step has run, so jumping to RuleAndMcpConfig must be rejected.
        assert!(
            fixture
                .store()
                .mark_step_running(
                    "project_0001",
                    &operation.operation_id,
                    AggregateInitializationStepKind::RuleAndMcpConfig,
                    "key".to_string(),
                    STEP_AT.into(),
                )
                .is_err()
        );
    }

    #[test]
    fn aggregate_operation_is_separate_from_six_step_repository_operation_and_idempotent() {
        let fixture = aggregate_init_fixture();
        let first = fixture
            .store()
            .create_idempotent(fixture.new_operation("request-a"))
            .unwrap();
        let retry = fixture
            .store()
            .create_idempotent(fixture.new_operation("request-a"))
            .unwrap();

        assert_eq!(first.operation_id, retry.operation_id);
        assert_eq!(first.steps.len(), 5);
        assert_eq!(first.operation_kind, "aggregate_initialization");
        assert!(fixture.repository_operation_store_rejects_aggregate_layout(&first));

        // Task 1.1 迁移边界:LC root recipe 的四条命令只能映射进既有五步/
        // 三个 provider turn,不得新增 step(REQ-BOOT-03「不新增第六步」),
        // recipe provider 仍是记录在 operation input 里的固定 claude_code。
        assert_eq!(
            first
                .steps
                .iter()
                .map(|step| step.step_id)
                .collect::<Vec<_>>(),
            AggregateInitializationStepKind::V1.to_vec(),
            "aggregate step layout must stay exactly V1; no sixth step may appear"
        );
        let provider_turns: Vec<AggregateInitializationStepKind> =
            AggregateInitializationStepKind::V1
                .iter()
                .copied()
                .filter(|kind| kind.is_provider_turn())
                .collect();
        assert_eq!(
            provider_turns,
            vec![
                AggregateInitializationStepKind::PreCheck,
                AggregateInitializationStepKind::RuleAndMcpConfig,
                AggregateInitializationStepKind::OpenspecAndExamples,
            ],
            "four root recipe commands must map onto the existing three provider turns"
        );
        assert_eq!(first.input.provider, "claude_code");
    }

    /// Task 1.1 迁移边界回归锁:LC root-cwd 契约落地前后,单仓六步初始化
    /// 契约快照零语义变化(BOOT-01 后半句「传统单仓登记 SHALL 保持现有逐仓
    /// 四命令与 git_finalize 契约不变」、D1–D4 单仓不变声明)。六步布局与
    /// 顺序、四条无中断命令的字节内容、command_index 双向映射全部冻结;
    /// 聚合侧也不占用单仓读取面。任何把聚合 recipe 命令、step 或 cwd 语义
    /// 混进单仓契约的改动都会在此先红。
    #[test]
    fn legacy_repository_initialization_contract_remains_unchanged_after_lc_root_contract() {
        // 六步布局与顺序冻结:CadenceSkills → … → GitFinalize。
        assert_eq!(
            RepositoryInitializationStepKind::ALL,
            [
                RepositoryInitializationStepKind::CadenceSkills,
                RepositoryInitializationStepKind::PreCheck,
                RepositoryInitializationStepKind::RuleConfig,
                RepositoryInitializationStepKind::McpConfiguration,
                RepositoryInitializationStepKind::ProjectRulesExamples,
                RepositoryInitializationStepKind::GitFinalize,
            ]
        );

        // 四条无中断命令字节级冻结;两个确定性步骤(CadenceSkills/GitFinalize)
        // 没有命令——GitFinalize 仍是单仓专属终结点,聚合 recipe 不得复用。
        assert_eq!(
            RepositoryInitializationStepKind::CadenceSkills.command(),
            None
        );
        assert_eq!(
            RepositoryInitializationStepKind::GitFinalize.command(),
            None
        );
        assert_eq!(
            RepositoryInitializationStepKind::PreCheck.command(),
            Some("/pre-check --no-interrupt --upgrade 用大陆镜像")
        );
        assert_eq!(
            RepositoryInitializationStepKind::RuleConfig.command(),
            Some("/rule-config --no-interrupt")
        );
        assert_eq!(
            RepositoryInitializationStepKind::McpConfiguration.command(),
            Some("/mcp-configuration --no-interrupt")
        );
        assert_eq!(
            RepositoryInitializationStepKind::ProjectRulesExamples.command(),
            Some("/project-rules-examples --no-interrupt")
        );

        // command_index 双向映射冻结:1..=4 依序映射四命令步骤,0 与越界无映射。
        assert_eq!(
            RepositoryInitializationStepKind::from_command_index(0),
            None
        );
        assert_eq!(
            (1..=4)
                .map(RepositoryInitializationStepKind::from_command_index)
                .collect::<Vec<_>>(),
            vec![
                Some(RepositoryInitializationStepKind::PreCheck),
                Some(RepositoryInitializationStepKind::RuleConfig),
                Some(RepositoryInitializationStepKind::McpConfiguration),
                Some(RepositoryInitializationStepKind::ProjectRulesExamples),
            ]
        );
        assert_eq!(
            RepositoryInitializationStepKind::from_command_index(5),
            None
        );

        // 聚合侧不占用单仓读取面:legacy operation store 仍拒绝聚合布局。
        let fixture = aggregate_init_fixture();
        let operation = fixture
            .store()
            .create_idempotent(fixture.new_operation("legacy-lock"))
            .unwrap();
        assert!(fixture.repository_operation_store_rejects_aggregate_layout(&operation));
    }

    #[test]
    fn create_idempotent_returns_conflict_when_idempotency_identity_drifts() {
        let fixture = aggregate_init_fixture();
        let first = fixture
            .store()
            .create_idempotent(fixture.new_operation("request-a"))
            .unwrap();

        // Same operation id but a different idempotency key -> conflict.
        let mut drifted = fixture.new_operation("request-a");
        drifted.input.idempotency_key = "request-b".to_string();
        let error = fixture.store().create_idempotent(drifted).unwrap_err();
        assert!(matches!(error, ProductStoreError::Conflict { .. }));

        // Same idempotency key but drifted manifest revision -> conflict.
        let mut drifted_manifest = fixture.new_operation("request-a");
        drifted_manifest.input.manifest_revision = first.input.manifest_revision + 1;
        assert!(matches!(
            fixture.store().create_idempotent(drifted_manifest),
            Err(ProductStoreError::Conflict { .. })
        ));
    }

    #[test]
    fn five_steps_must_run_in_order_with_checkpoint_before_completion() {
        let fixture = aggregate_init_fixture();
        let operation = fixture
            .store()
            .create_idempotent(fixture.new_operation("request-a"))
            .unwrap();
        fixture
            .store()
            .mark_running("project_0001", &operation.operation_id, RUNNING_AT.into())
            .unwrap();

        // Cannot complete a step without first checkpointing its output.
        let started = fixture
            .store()
            .mark_step_running(
                "project_0001",
                &operation.operation_id,
                AggregateInitializationStepKind::MachineSkills,
                "digest".to_string(),
                STEP_AT.into(),
            )
            .unwrap();
        assert!(matches!(
            fixture.store().mark_step_completed(
                "project_0001",
                &operation.operation_id,
                AggregateInitializationStepKind::MachineSkills,
                STEP_AT.into(),
            ),
            Err(ProductStoreError::IdentityMismatch { .. })
        ));
        assert_eq!(
            started.current_step,
            Some(AggregateInitializationStepKind::MachineSkills)
        );

        // Cannot jump ahead to a later step while an earlier one is pending.
        assert!(matches!(
            fixture.store().mark_step_running(
                "project_0001",
                &operation.operation_id,
                AggregateInitializationStepKind::OpenspecAndExamples,
                "digest".to_string(),
                STEP_AT.into(),
            ),
            Err(ProductStoreError::IdentityMismatch { .. })
        ));

        // Running all five in order with checkpoints succeeds.
        let mut last = started;
        for step in AggregateInitializationStepKind::V1 {
            last = fixture.run_step(&operation.operation_id, step);
        }
        assert!(last.current_step.is_none());
        assert!(
            last.steps
                .iter()
                .all(|step| step.output_artifact_ref.is_some())
        );
    }

    #[test]
    fn cancel_marks_running_step_failed_and_records_cancellation() {
        let fixture = aggregate_init_fixture();
        let operation = fixture
            .store()
            .create_idempotent(fixture.new_operation("request-a"))
            .unwrap();
        fixture
            .store()
            .mark_running("project_0001", &operation.operation_id, RUNNING_AT.into())
            .unwrap();
        // Run the two preceding deterministic steps, then start PreCheck.
        fixture.run_step(
            &operation.operation_id,
            AggregateInitializationStepKind::MachineSkills,
        );
        fixture.run_step(
            &operation.operation_id,
            AggregateInitializationStepKind::AggregatePreflight,
        );
        fixture
            .store()
            .mark_step_running(
                "project_0001",
                &operation.operation_id,
                AggregateInitializationStepKind::PreCheck,
                "digest".to_string(),
                STEP_AT.into(),
            )
            .unwrap();

        let cancelled = fixture
            .store()
            .cancel(
                "project_0001",
                &operation.operation_id,
                AggregateCancellationRecord {
                    reason_code: "user_cancelled".to_string(),
                    cancelled_at: STEP_AT.to_string(),
                    detail: None,
                },
                STEP_AT.into(),
            )
            .unwrap();
        assert_eq!(
            cancelled.status,
            AggregateInitializationOperationStatus::Cancelled
        );
        assert_eq!(
            cancelled.failed_step,
            Some(AggregateInitializationStepKind::PreCheck)
        );
        assert!(cancelled.cancellation.is_some());
        assert!(cancelled.completed_at.is_some());
    }

    #[test]
    fn recover_interrupted_fails_running_step_and_records_interrupt_error() {
        let fixture = aggregate_init_fixture();
        let operation = fixture
            .store()
            .create_idempotent(fixture.new_operation("request-a"))
            .unwrap();
        fixture
            .store()
            .mark_running("project_0001", &operation.operation_id, RUNNING_AT.into())
            .unwrap();
        fixture.run_step(
            &operation.operation_id,
            AggregateInitializationStepKind::MachineSkills,
        );
        fixture
            .store()
            .mark_step_running(
                "project_0001",
                &operation.operation_id,
                AggregateInitializationStepKind::AggregatePreflight,
                "digest".to_string(),
                STEP_AT.into(),
            )
            .unwrap();

        let recovered = fixture
            .store()
            .recover_interrupted("project_0001", &operation.operation_id, STEP_AT.into())
            .unwrap();
        assert_eq!(
            recovered.status,
            AggregateInitializationOperationStatus::Failed
        );
        assert_eq!(
            recovered.failed_step,
            Some(AggregateInitializationStepKind::AggregatePreflight)
        );
        assert_eq!(
            recovered.error.as_ref().unwrap().reason_code,
            "aggregate_initialization_interrupted"
        );
    }

    #[test]
    fn idempotency_identity_groups_relevant_fields() {
        let fixture = aggregate_init_fixture();
        let operation = fixture
            .store()
            .create_idempotent(fixture.new_operation("request-a"))
            .unwrap();
        let identity = operation.idempotency_identity();
        assert_eq!(
            identity,
            AggregateInitializationIdempotencyIdentity {
                project_id: "project_0001".to_string(),
                idempotency_key: "request-a".to_string(),
                manifest_revision: 1,
                policy_digest: "sha256:policy".to_string(),
                profile_evidence_digest: Some("sha256:profile".to_string()),
            }
        );
    }

    #[test]
    fn interrupted_provider_step_cleans_staging_and_marks_failed_without_auto_restart() {
        let fixture = aggregate_init_fixture();
        fixture.persist_running_at(AggregateInitializationStepKind::RuleAndMcpConfig);
        fixture.write_staging("rule-and-mcp/partial.json");
        assert!(fixture.staging_root().exists());

        let operation = fixture
            .store()
            .recover_interrupted(
                "project_0001",
                "aggregate_initialization_0001",
                fixture.now(),
            )
            .unwrap();

        assert_eq!(
            operation.status,
            AggregateInitializationOperationStatus::Failed
        );
        assert_eq!(
            operation.failed_step,
            Some(AggregateInitializationStepKind::RuleAndMcpConfig)
        );
        assert!(!fixture.staging_root().exists());
        assert_eq!(fixture.provider().turn_count(), 0);
    }

    // ── 片1（块2 片1·契据形状与错误词表）：记录形状 + error_record 映射 ──

    fn error_record(error: AggregateInitializationError) -> AggregateInitializationErrorRecord {
        error.error_record()
    }

    fn trust_waiting() -> ProviderTrustWaiting {
        ProviderTrustWaiting {
            provider: ProviderName::ClaudeCode,
            canonical_root: std::path::PathBuf::from("/tmp/aggregate-root"),
            trust_key: "trust_key".to_string(),
            reason_code: "provider_trust_missing".to_string(),
            message: "trust missing".to_string(),
            retry_action: "complete the provider authorization and retry".to_string(),
            manual_guidance: "manual guidance".to_string(),
            command_markdown: None,
            recorded_at: "2026-11-03T00:00:00Z".to_string(),
        }
    }

    fn candidate(
        provider: &str,
        display_name: &str,
        cli_available: bool,
        capability_state: &str,
        skipped_reason: Option<&str>,
    ) -> AggregateProviderCandidate {
        AggregateProviderCandidate {
            provider: provider.to_string(),
            display_name: display_name.to_string(),
            cli_available,
            cli_reason_code: if cli_available {
                None
            } else {
                Some("command_missing".to_string())
            },
            capability_state: capability_state.to_string(),
            skipped_reason: skipped_reason.map(str::to_string),
            action_hint: "run the capability probe for this provider".to_string(),
            warning: None,
        }
    }

    #[test]
    fn legacy_operation_json_without_provider_selection_deserializes_with_none() {
        let fixture = aggregate_init_fixture();
        let operation = fixture.new_operation("request-a");
        let mut value = serde_json::to_value(&operation).unwrap();
        value.as_object_mut().unwrap().remove("provider_selection");
        let legacy = serde_json::to_string(&value).unwrap();

        let parsed: AggregateInitializationOperation = serde_json::from_str(&legacy).unwrap();
        assert_eq!(parsed.provider_selection, None);
        assert_eq!(parsed.operation_id, operation.operation_id);
    }

    #[test]
    fn legacy_error_record_json_without_cause_and_provider_deserializes_with_none() {
        let legacy = r#"{
            "stage": "pre_check",
            "reason_code": "root_recipe_probe_failed",
            "stderr_summary": "provider probe failed",
            "retryable": true,
            "action": "retry_provider_turn"
        }"#;
        let record: AggregateInitializationErrorRecord = serde_json::from_str(legacy).unwrap();
        assert_eq!(record.cause, None);
        assert_eq!(record.provider, None);
        assert_eq!(record.reason_code, "root_recipe_probe_failed");
    }

    #[test]
    fn provider_selection_roundtrips_four_candidates_in_owner_order() {
        let fixture = aggregate_init_fixture();
        let selection = AggregateProviderSelection {
            selected: "pi".to_string(),
            candidates: vec![
                candidate("pi", "Pi", true, "confirmed", None),
                candidate(
                    "kimi_code",
                    "Kimi Code",
                    true,
                    "unverified",
                    Some("needs_verification"),
                ),
                candidate(
                    "claude_code",
                    "Claude Code",
                    false,
                    "not_installed",
                    Some("cli_unavailable"),
                ),
                candidate("codex", "Codex", true, "denied", Some("denied")),
            ],
            rule_version: "v1".to_string(),
            decided_at: "2026-11-03T00:00:00Z".to_string(),
        };
        let operation = fixture
            .new_operation("request-a")
            .with_provider_selection(selection.clone());

        let encoded = serde_json::to_string(&operation).unwrap();
        assert!(encoded.contains("\"provider_selection\""));
        assert!(encoded.contains("\"rule_version\":\"v1\""));
        assert!(encoded.contains("\"capability_state\""));
        assert!(encoded.contains("\"cli_reason_code\""));
        assert!(encoded.contains("\"skipped_reason\""));
        assert!(encoded.contains("\"action_hint\""));

        let parsed: AggregateInitializationOperation = serde_json::from_str(&encoded).unwrap();
        let parsed_selection = parsed
            .provider_selection
            .expect("provider_selection is persisted");
        assert_eq!(parsed_selection, selection);
        assert_eq!(
            parsed_selection
                .candidates
                .iter()
                .map(|candidate| candidate.provider.clone())
                .collect::<Vec<_>>(),
            vec![
                "pi".to_string(),
                "kimi_code".to_string(),
                "claude_code".to_string(),
                "codex".to_string(),
            ]
        );
    }

    #[test]
    fn error_record_keeps_existing_reason_codes_and_actions() {
        let store_action = "The aggregate initialization state could not be persisted; query the operation after recovery.";

        let not_found = error_record(AggregateInitializationError::NotFound {
            id: "op".to_string(),
        });
        assert_eq!(
            not_found.reason_code,
            "aggregate_initialization_store_failed"
        );
        assert_eq!(not_found.action, store_action);

        let store = error_record(AggregateInitializationError::Store(ProductStoreError::Io(
            "boom".to_string(),
        )));
        assert_eq!(store.reason_code, "aggregate_initialization_store_failed");
        assert_eq!(store.action, store_action);

        let rejected = error_record(AggregateInitializationError::StateRejected {
            id: "op".to_string(),
            detail: "jump".to_string(),
            retryable: true,
        });
        assert_eq!(
            rejected.reason_code,
            "aggregate_initialization_state_rejected"
        );
        assert_eq!(
            rejected.action,
            "The aggregate initialization state machine rejected the transition; review the operation and resubmit."
        );

        let skills = error_record(AggregateInitializationError::SkillsPreparation {
            reason: "skills clone failed".to_string(),
            retryable: true,
        });
        assert_eq!(skills.reason_code, "aggregate_machine_skills_failed");
        assert_eq!(skills.action, "skills clone failed");

        let preflight = error_record(AggregateInitializationError::Preflight {
            reason: "preflight failed".to_string(),
            retryable: true,
        });
        assert_eq!(preflight.reason_code, "aggregate_preflight_failed");
        assert_eq!(preflight.action, "preflight failed");

        let publication = error_record(AggregateInitializationError::PolicyPublication {
            reason: "publication failed".to_string(),
            retryable: true,
        });
        assert_eq!(
            publication.reason_code,
            "aggregate_policy_publication_failed"
        );
        assert_eq!(publication.action, "publication failed");

        let provider_turn = error_record(AggregateInitializationError::ProviderTurn {
            step: AggregateInitializationStepKind::PreCheck,
            reason: "provider turn failed".to_string(),
            retryable: true,
        });
        assert_eq!(provider_turn.reason_code, "aggregate_pre_check_failed");
        assert_eq!(provider_turn.action, "provider turn failed");

        let trust = error_record(AggregateInitializationError::TrustWaiting {
            waiting: Box::new(trust_waiting()),
        });
        assert_eq!(trust.reason_code, "aggregate_initialization_trust_waiting");
        assert_eq!(
            trust.action,
            "complete the provider authorization and retry"
        );

        let cancelled = error_record(AggregateInitializationError::Cancelled);
        assert_eq!(cancelled.reason_code, "aggregate_initialization_cancelled");
        assert_eq!(
            cancelled.action,
            "The aggregate initialization was cancelled."
        );
    }

    #[test]
    fn environment_precondition_reason_code_is_the_code_itself() {
        let record = error_record(AggregateInitializationError::EnvironmentPrecondition {
            code: "codegraph_missing".to_string(),
            reason: "codegraph CLI not found".to_string(),
            retryable: false,
        });
        assert_eq!(record.reason_code, "codegraph_missing");
        assert_eq!(record.action, "codegraph CLI not found");
        assert_eq!(record.cause, None);
    }

    #[test]
    fn audit_snapshot_budget_store_error_has_its_own_reason_code() {
        let failure = AuditSnapshotBudgetFailure {
            max_bytes: 100,
            cumulative_bytes: 120,
            over_line_path: "pms/target/lib.rs".to_string(),
            max_entries: 10,
            cumulative_entries: 12,
            top_groups: vec![BudgetGroup {
                key: "pms/target".to_string(),
                bytes: 90,
            }],
            member_summaries: vec![MemberBudgetSummary {
                member: "pms".to_string(),
                bytes: 90,
                entries: 3,
            }],
        };
        let record = error_record(AggregateInitializationError::Store(
            ProductStoreError::AuditSnapshotBudgetExceeded(failure),
        ));
        assert_eq!(record.reason_code, "aggregate_audit_budget_exceeded");
        assert_ne!(record.reason_code, "aggregate_initialization_store_failed");
        assert_eq!(
            record.cause,
            Some("aggregate_audit_budget_exceeded".to_string())
        );
        // 结构化载荷必须随记录落进 stderr_summary：码表说「大项排行与成员仓
        // 见下方诊断」，不落就等于下方什么都没有（controller 2026-11-03 批准的
        // Plan §2.2 偏离）。
        let stderr_summary = record
            .stderr_summary
            .as_deref()
            .expect("budget failure carries its diagnostic");
        assert!(stderr_summary.contains("top_groups:"), "{stderr_summary}");
        assert!(
            stderr_summary.contains("member_summaries:"),
            "{stderr_summary}"
        );
        assert!(
            stderr_summary.contains("pms/target=90 bytes"),
            "{stderr_summary}"
        );
        assert!(
            stderr_summary.contains("pms=90 bytes/3 entries"),
            "{stderr_summary}"
        );
        assert!(
            stderr_summary.contains("audit_snapshot_budget_exceeded"),
            "{stderr_summary}"
        );
    }

    #[test]
    fn provider_turn_diagnosed_shares_reason_code_with_provider_turn() {
        let reason =
            "provider_capability_launch_not_confirmed: PlanningReadOnly launch cell is unknown"
                .to_string();
        let plain = error_record(AggregateInitializationError::ProviderTurn {
            step: AggregateInitializationStepKind::PreCheck,
            reason: reason.clone(),
            retryable: true,
        });
        let diagnosed = error_record(AggregateInitializationError::ProviderTurnDiagnosed {
            step: AggregateInitializationStepKind::PreCheck,
            reason: reason.clone(),
            retryable: true,
            provider: Some("pi".to_string()),
            diagnostic: TurnDiagnostic {
                cause: Some("capability_unverified".to_string()),
                output: Some("provider stderr".to_string()),
            },
        });

        assert_eq!(
            plain.reason_code,
            "aggregate_provider_capability_unverified"
        );
        assert_eq!(diagnosed.reason_code, plain.reason_code);
        assert_eq!(diagnosed.action, plain.action);
        assert_eq!(plain.cause, None);
        assert_eq!(plain.provider, None);
        assert_eq!(plain.stderr_summary, None);
        assert_eq!(diagnosed.cause, Some("capability_unverified".to_string()));
        assert_eq!(diagnosed.provider, Some("pi".to_string()));
        assert_eq!(
            diagnosed.stderr_summary,
            Some("provider stderr".to_string())
        );
    }

    #[test]
    fn provider_turn_denial_and_stale_evidence_get_their_own_reason_codes() {
        let denied = error_record(AggregateInitializationError::ProviderTurn {
            step: AggregateInitializationStepKind::PreCheck,
            reason: "provider_capability_launch_not_confirmed: PlanningReadOnly launch cell is denied; reason: no resume channel".to_string(),
            retryable: false,
        });
        assert_eq!(denied.reason_code, "aggregate_provider_capability_denied");

        let stale = error_record(AggregateInitializationError::ProviderTurn {
            step: AggregateInitializationStepKind::PreCheck,
            reason: "capability snapshot mismatch".to_string(),
            retryable: false,
        });
        assert_eq!(
            stale.reason_code,
            "aggregate_provider_capability_evidence_stale"
        );
    }

    #[test]
    fn skills_preparation_diagnosed_keeps_reason_code_and_adds_cause() {
        let plain = error_record(AggregateInitializationError::SkillsPreparation {
            reason: "skills preparation failed".to_string(),
            retryable: true,
        });
        assert_eq!(plain.reason_code, "aggregate_machine_skills_failed");
        assert_eq!(plain.cause, None);

        let diagnosed = error_record(AggregateInitializationError::SkillsPreparationDiagnosed {
            reason: "skills preparation failed".to_string(),
            retryable: true,
            cause: "skills_unavailable_clone".to_string(),
        });
        assert_eq!(diagnosed.reason_code, "aggregate_machine_skills_failed");
        assert_eq!(diagnosed.action, plain.action);
        assert_eq!(
            diagnosed.cause,
            Some("skills_unavailable_clone".to_string())
        );
    }
}
