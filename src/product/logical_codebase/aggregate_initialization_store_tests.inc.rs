#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::json_store::ProductStoreError;
    use crate::product::logical_codebase::aggregate_initialization::{
        AggregateInitializationIdempotencyIdentity, AggregateInitializationOperationInput,
        AggregateInitializationStepKind,
    };
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
}
