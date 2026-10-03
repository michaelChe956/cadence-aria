    // Task 3（aggregate-policy-root-publication，REQ-BOOT-06）：receipt 证明的
    // producer-only 重跑预检。本文件经 coordinator_tests_profile_trust.inc.rs 在
    // mod tests 内 include，复用同模块的 gateway/审计链/注入桩。

    /// 负例矩阵的旧证据破坏开关：每个变体恰好破坏一个证明环节。
    #[derive(Debug, Clone)]
    enum ReplayEvidenceBreak {
        /// 完整证据（正例对照）。
        Intact,
        /// 四命令审计在场但不 finalize 最终 receipt。
        MissingFinalReceipt,
        /// 证据写在另一个 LC scope（本 scope 零证据）。
        WrongScope,
        /// 证据 root 是另一目录（最终 receipt 不证明本根）。
        WrongRoot,
        /// 证据完整但旧 operation 停在 Running（未 Completed）。
        OperationNotCompleted,
        /// 删除第 4 条命令 receipt。
        MissingCommand,
        /// 第 4 条命令 receipt 换成 Rejected 事实。
        RejectedCommand,
        /// 第 3 条命令 receipt 的命令身份被篡改。
        CommandIdentityDrift,
        /// 最终 receipt 的命令摘要 command_index 重复（缺末条索引）。
        RepeatedIndex,
        /// 最终 receipt 的 after_snapshot_digest 与命令 receipt 不一致。
        SummaryReferenceMismatch,
        /// 最终 receipt 的 rule_digest 与末条 snapshot 不一致。
        AgentsRuleDigestMismatch,
        /// 末条命令 receipt snapshot 中 AGENTS.md 摘要被篡改。
        AgentsAfterDigestMismatch,
        /// 证据冻结后当前 AGENTS.md 字节漂移。
        AgentsCurrentBytesDrift,
        /// 旧 snapshot 从未见过 CLAUDE.md，证据后用户新增。
        ClaudeUntrackedNewFile,
        /// 当前 CLAUDE.md 字节与旧 snapshot 摘要不符。
        ClaudeDigestMismatch,
        /// AGENTS.md 换成字节相同的 symlink（入口必须复验无 symlink）。
        #[cfg(unix)]
        AgentsSymlinkSameBytes,
        /// root 下出现 `.aria`（永不豁免）。
        AriaPresent,
    }

    struct RecipeReplayFixture {
        _temp: tempfile::TempDir,
        paths: ProductAppPaths,
        lc_id: String,
        aggregate_root: PathBuf,
        manifest: LogicalCodebaseManifest,
        member_id: String,
        checkout_record: RepositoryCheckoutRecord,
        policy_digest: String,
        coordinator: AggregateInitializationCoordinator,
        service: DeterministicAggregatePreflightService,
        audit: Arc<GatewayRunAudit>,
        streaming_adapter: Arc<CountingStreamingAdapter>,
    }

    impl RecipeReplayFixture {
        /// 新 key 的 recipe input：与旧 operation 不同的 idempotency key。
        fn new_recipe_input(&self) -> AggregateInitializationOperationInput {
            AggregateInitializationOperationInput {
                idempotency_key: "recipe-replay-0002".to_string(),
                manifest_revision: self.manifest.membership_revision,
                policy_digest: self.policy_digest.clone(),
                profile_evidence_digest: Some("sha256:profile".to_string()),
                provider_context_root: self.aggregate_root.clone(),
                provider: "claude_code".to_string(),
            }
        }

        fn lc_store(&self) -> LogicalCodebaseStore {
            LogicalCodebaseStore::for_lc(self.paths.clone(), &self.lc_id)
        }

        /// 把成员 main checkout 指到新路径（边界重核验用）。
        fn point_member_checkout_at(&self, path: &Path) {
            let mut checkout = self.checkout_record.clone();
            checkout.canonical_path = path.to_path_buf();
            self.lc_store()
                .save_checkout("project_0001", &checkout)
                .unwrap();
        }

        fn inspect_root(&self) -> Result<AggregatePreflightSnapshot, AggregateInitializationError> {
            self.service
                .inspect("project_0001", &self.manifest, &CancellationToken::new())
        }
    }

    fn replay_git(cwd: &Path, arguments: &[&str]) {
        let output = std::process::Command::new("git")
            .current_dir(cwd)
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn replay_init_member(path: &Path) {
        std::fs::create_dir_all(path).unwrap();
        replay_git(path, &["init", "--quiet", "-b", "main"]);
        replay_git(path, &["config", "user.email", "replay@test.local"]);
        replay_git(path, &["config", "user.name", "Replay Test"]);
        std::fs::write(path.join("README.md"), "# member\n").unwrap();
        replay_git(path, &["add", "."]);
        replay_git(path, &["commit", "--quiet", "-m", "initial"]);
    }

    /// 聚合根子树全量字节快照：负例"用户 bytes 不变"的强断言面。
    fn replay_root_inventory(root: &Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        let mut inventory = std::collections::BTreeMap::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let relative = path.strip_prefix(root).unwrap().to_path_buf();
                    inventory.insert(
                        relative.to_string_lossy().into_owned(),
                        std::fs::read(&path).unwrap_or_default(),
                    );
                }
            }
        }
        inventory
    }

    fn recipe_replay_fixture(break_kind: ReplayEvidenceBreak) -> RecipeReplayFixture {
        use crate::product::project_store::{CreateProjectInput, ProjectStore};

        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "recipe-replay".to_string(),
                description: None,
            })
            .unwrap();

        // 聚合根（非 Git）+ 一个真实成员 git 仓（main checkout）。
        let aggregate_root = temp.path().join("aggregate-root");
        std::fs::create_dir_all(&aggregate_root).unwrap();
        let member_root = aggregate_root.join("repo");
        replay_init_member(&member_root);

        let lc_id = LogicalCodebaseStore::new(paths.clone())
            .create(
                "project_0001",
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "recipe-replay-lc".to_string(),
                    aggregate_root: std::fs::canonicalize(&aggregate_root).unwrap(),
                },
            )
            .unwrap()
            .id;
        let lc_store = LogicalCodebaseStore::for_lc(paths.clone(), &lc_id);

        let member_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        let manifest =
            LogicalCodebaseManifest::new("project_0001", aggregate_root.clone(), vec![member_id]);
        lc_store.save_manifest("project_0001", &manifest).unwrap();
        lc_store
            .save_member(
                "project_0001",
                &CodebaseMemberRecord {
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_repo".to_string(),
                    alias: "repo".to_string(),
                    role: "member".to_string(),
                    ordinal: 1,
                    source_identity: RepositorySourceIdentity::from_git_parts(
                        &member_root,
                        member_root.join(".git"),
                        Some("ssh://git@example.test/acme/repo.git".to_string()),
                    ),
                    repo_type: Default::default(),
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_id],
                    status: Default::default(),
                    created_at: "2026-10-02T00:00:00Z".to_string(),
                    updated_at: "2026-10-02T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        let checkout_record = RepositoryCheckoutRecord {
            checkout_id,
            logical_repository_id: member_id,
            physical_repository_id: "repository_repo".to_string(),
            kind: crate::product::logical_codebase::CheckoutKind::Main,
            canonical_path: member_root.clone(),
            checkout_path_hash: "sha256:checkout".to_string(),
            git_dir_identity: "sha256:git-dir".to_string(),
            revision: Some("abc123".to_string()),
            availability: crate::product::logical_codebase::CheckoutAvailability::Available,
            observed_at: "2026-10-02T00:00:00Z".to_string(),
            created_at: "2026-10-02T00:00:00Z".to_string(),
            updated_at: "2026-10-02T00:00:00Z".to_string(),
        };
        lc_store
            .save_checkout("project_0001", &checkout_record)
            .unwrap();

        // 存量自举桩 policy：正例语义的"旧 Completed 桩 operation"配套事实。
        let policy = crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::for_lc(
            paths.clone(),
            &lc_id,
        )
        .ensure_bootstrap(&manifest)
        .unwrap();
        assert!(
            policy.is_bootstrap_placeholder(),
            "fixture policy must stay the bootstrap placeholder"
        );

        // 本 scope 旧 operation：五步全走完（OperationNotCompleted 负例停在
        // Running——四命令审计与最终 receipt 照常在场）。
        let old_operation_id = "aggregate_initialization_replay_old_0001".to_string();
        let init_store = AggregateInitializationOperationStore::for_lc(paths.clone(), &lc_id);
        init_store
            .create_idempotent(AggregateInitializationOperation::new(
                old_operation_id.clone(),
                "project_0001".to_string(),
                AggregateInitializationOperationInput {
                    idempotency_key: "replay-old-0001".to_string(),
                    manifest_revision: manifest.membership_revision,
                    policy_digest: policy.digest.clone(),
                    profile_evidence_digest: Some("sha256:profile".to_string()),
                    provider_context_root: aggregate_root.clone(),
                    provider: "claude_code".to_string(),
                },
                "2026-10-02T00:01:00Z".to_string(),
            ))
            .unwrap();
        init_store
            .mark_running(
                "project_0001",
                &old_operation_id,
                "2026-10-02T00:02:00Z".to_string(),
            )
            .unwrap();
        for step in AggregateInitializationStepKind::V1 {
            init_store
                .mark_step_running(
                    "project_0001",
                    &old_operation_id,
                    step,
                    format!("replay:{}:{}", old_operation_id, step.as_str()),
                    "2026-10-02T00:03:00Z".to_string(),
                )
                .unwrap();
            init_store
                .checkpoint_step_output(
                    "project_0001",
                    &old_operation_id,
                    step,
                    format!("aggregate-initializations/replay/{}.json", step.as_str()),
                    "2026-10-02T00:04:00Z".to_string(),
                )
                .unwrap();
            init_store
                .mark_step_completed(
                    "project_0001",
                    &old_operation_id,
                    step,
                    "2026-10-02T00:05:00Z".to_string(),
                )
                .unwrap();
        }
        if !matches!(break_kind, ReplayEvidenceBreak::OperationNotCompleted) {
            init_store
                .finish_completed(
                    "project_0001",
                    &old_operation_id,
                    "2026-10-02T00:06:00Z".to_string(),
                )
                .unwrap();
        }

        // 证据生成前根上已有 product-owned 入口：AGENTS.md + 不同正文的
        // CLAUDE.md（ClaudeUntrackedNewFile 负例的旧证据从未见过 CLAUDE）。
        let agents_bytes = b"# aggregate root rules\n".to_vec();
        let claude_bytes = b"# claude root instructions\n".to_vec();
        std::fs::write(aggregate_root.join("AGENTS.md"), &agents_bytes).unwrap();
        if !matches!(break_kind, ReplayEvidenceBreak::ClaudeUntrackedNewFile) {
            std::fs::write(aggregate_root.join("CLAUDE.md"), &claude_bytes).unwrap();
        }
        let canonical_root = std::fs::canonicalize(&aggregate_root).unwrap();

        // 证据落点：WrongScope 写到另一个 LC scope；证据根：WrongRoot 用另一
        // 目录的完整自洽证据（receipt 不证明本根）。其余都在本 scope 本根。
        let evidence_root = if matches!(break_kind, ReplayEvidenceBreak::WrongRoot) {
            let other_root = temp.path().join("other-evidence-root");
            std::fs::create_dir_all(&other_root).unwrap();
            std::fs::write(other_root.join("AGENTS.md"), &agents_bytes).unwrap();
            std::fs::write(other_root.join("CLAUDE.md"), &claude_bytes).unwrap();
            std::fs::canonicalize(&other_root).unwrap()
        } else {
            canonical_root.clone()
        };
        let receipt_store = if matches!(break_kind, ReplayEvidenceBreak::WrongScope) {
            let other_lc_root = temp.path().join("other-lc-root");
            std::fs::create_dir_all(&other_lc_root).unwrap();
            let other_lc = LogicalCodebaseStore::new(paths.clone())
                .create(
                    "project_0001",
                    crate::product::logical_codebase::LogicalCodebaseCreateInput {
                        name: "recipe-replay-other-lc".to_string(),
                        aggregate_root: std::fs::canonicalize(&other_lc_root).unwrap(),
                    },
                )
                .unwrap()
                .id;
            crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(paths.clone(), other_lc)
        } else {
            crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
                paths.clone(),
                lc_id.clone(),
            )
        };

        // 真实 auditor 审计链：四命令前后全量快照、零未知变更 → 全部 Allowed。
        let auditor = crate::product::logical_codebase::RootRecipeFilesystemAuditor::new();
        for (index, (step, command_index, command)) in
            crate::product::logical_codebase::root_recipe_command_index()
                .into_iter()
                .enumerate()
        {
            let watch = auditor
                .before_command(
                    &old_operation_id,
                    &evidence_root,
                    step,
                    command_index,
                    command,
                )
                .unwrap();
            let receipt = auditor
                .after_command(watch, format!("2026-10-02T00:10:{index:02}Z"))
                .unwrap();
            assert_eq!(
                receipt.verdict,
                crate::product::logical_codebase::RootRecipeCommandVerdict::Allowed
            );
            receipt_store
                .append_command("project_0001", receipt)
                .unwrap();
        }
        if !matches!(break_kind, ReplayEvidenceBreak::MissingFinalReceipt) {
            let rule_digest =
                crate::product::logical_codebase::root_recipe_receipt::root_rule_digest(&evidence_root)
                    .unwrap()
                    .expect("evidence root must expose a rule digest");
            receipt_store
                .finalize(
                    "project_0001",
                    &old_operation_id,
                    &policy.digest,
                    &rule_digest,
                    "2026-10-02T00:20:00Z".to_string(),
                )
                .unwrap();
        }

        // 负例破坏：durable 证据文件按变体篡改/删除（store 只在 append 时校验
        // 命令身份，durable 读侧不复检——这正是 proven 核验要拦下的漂移）。
        let receipts_root = crate::product::logical_codebase::lc_scope_root(
            &paths,
            "project_0001",
            &Some(lc_id.clone()),
        )
        .unwrap()
        .join("aggregate-recipe-receipts");
        let command_path = |index: usize| {
            receipts_root
                .join(&old_operation_id)
                .join("commands")
                .join(format!("{index:02}.json"))
        };
        let final_receipt_path = receipts_root.join(format!("{old_operation_id}.json"));
        match break_kind {
            ReplayEvidenceBreak::MissingCommand => {
                std::fs::remove_file(command_path(4)).unwrap();
            }
            ReplayEvidenceBreak::RejectedCommand => {
                let mut rejected: crate::product::logical_codebase::RootRecipeCommandReceipt =
                    crate::product::json_store::read_json(&command_path(4)).unwrap();
                std::fs::remove_file(command_path(4)).unwrap();
                rejected.verdict = crate::product::logical_codebase::RootRecipeCommandVerdict::Rejected;
                rejected.rejection_reason =
                    Some("tampered rejection must invalidate the replay proof".to_string());
                receipt_store
                    .append_command("project_0001", rejected)
                    .unwrap();
            }
            ReplayEvidenceBreak::CommandIdentityDrift => {
                let mut drifted: crate::product::logical_codebase::RootRecipeCommandReceipt =
                    crate::product::json_store::read_json(&command_path(3)).unwrap();
                drifted.command = "/rule-config --no-interrupt --drifted".to_string();
                crate::product::json_store::write_json(&command_path(3), &drifted).unwrap();
            }
            ReplayEvidenceBreak::RepeatedIndex => {
                let mut receipt: crate::product::logical_codebase::RootRecipeReceipt =
                    crate::product::json_store::read_json(&final_receipt_path).unwrap();
                receipt.commands[3].command_index = 3;
                crate::product::json_store::write_json(&final_receipt_path, &receipt).unwrap();
            }
            ReplayEvidenceBreak::SummaryReferenceMismatch => {
                let mut receipt: crate::product::logical_codebase::RootRecipeReceipt =
                    crate::product::json_store::read_json(&final_receipt_path).unwrap();
                receipt.commands[3].after_snapshot_digest = "sha256:drifted-after".to_string();
                crate::product::json_store::write_json(&final_receipt_path, &receipt).unwrap();
            }
            ReplayEvidenceBreak::AgentsRuleDigestMismatch => {
                let mut receipt: crate::product::logical_codebase::RootRecipeReceipt =
                    crate::product::json_store::read_json(&final_receipt_path).unwrap();
                receipt.rule_digest = "sha256:drifted-rule".to_string();
                crate::product::json_store::write_json(&final_receipt_path, &receipt).unwrap();
            }
            ReplayEvidenceBreak::AgentsAfterDigestMismatch => {
                let mut command: crate::product::logical_codebase::RootRecipeCommandReceipt =
                    crate::product::json_store::read_json(&command_path(4)).unwrap();
                for entry in &mut command.after_snapshot.entries {
                    if entry.path == "AGENTS.md" {
                        entry.content_digest = Some("sha256:drifted-agents".to_string());
                    }
                }
                crate::product::json_store::write_json(&command_path(4), &command).unwrap();
            }
            ReplayEvidenceBreak::AgentsCurrentBytesDrift => {
                std::fs::write(
                    aggregate_root.join("AGENTS.md"),
                    "# aggregate root rules\ndrifted by user\n",
                )
                .unwrap();
            }
            ReplayEvidenceBreak::ClaudeUntrackedNewFile => {
                std::fs::write(aggregate_root.join("CLAUDE.md"), &claude_bytes).unwrap();
            }
            ReplayEvidenceBreak::ClaudeDigestMismatch => {
                std::fs::write(
                    aggregate_root.join("CLAUDE.md"),
                    "# claude root instructions\ndrifted by user\n",
                )
                .unwrap();
            }
            #[cfg(unix)]
            ReplayEvidenceBreak::AgentsSymlinkSameBytes => {
                std::fs::remove_file(aggregate_root.join("AGENTS.md")).unwrap();
                std::fs::write(aggregate_root.join("AGENTS.md.proof"), &agents_bytes).unwrap();
                std::os::unix::fs::symlink(
                    aggregate_root.join("AGENTS.md.proof"),
                    aggregate_root.join("AGENTS.md"),
                )
                .unwrap();
            }
            ReplayEvidenceBreak::AriaPresent => {
                std::fs::create_dir_all(aggregate_root.join(".aria")).unwrap();
            }
            ReplayEvidenceBreak::Intact
            | ReplayEvidenceBreak::MissingFinalReceipt
            | ReplayEvidenceBreak::WrongScope
            | ReplayEvidenceBreak::WrongRoot
            | ReplayEvidenceBreak::OperationNotCompleted => {}
        }

        // 生产同构装配：lc-scoped operation store/preflight + gateway 录制链。
        let audit = Arc::new(GatewayRunAudit::new());
        let streaming_adapter = Arc::new(CountingStreamingAdapter::new(StreamingBehavior::Complete));
        let mut registry = ProviderRegistry::new();
        registry.register(ProviderName::ClaudeCode, streaming_adapter.clone());
        let gateway = Arc::new(LogicalCodebaseProviderGateway::with_audit(
            crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::for_lc(
                paths.clone(),
                lc_id.clone(),
            ),
            Arc::new(StaticCapabilitySource::new("1.4.0")),
            Arc::new(PassthroughTargetResolver),
            Arc::new(registry),
            Arc::new(StubSyncAdapter),
            always_available_gate(),
            audit.clone(),
            aggregate_root.clone(),
        ));
        let provider: Arc<dyn AggregateProviderTurnDriver> = Arc::new(
            GatewayBackedAggregateProviderTurnDriver::claude_code(gateway, "cap_claude_code_1_4_0"),
        );
        let skills: Arc<dyn AggregateSkillsPreparation> = Arc::new(FakeSkillsPreparation {
            calls: Arc::new(Mutex::new(Vec::new())),
        });
        let service = DeterministicAggregatePreflightService::new(paths.clone()).for_lc(lc_id.clone());
        let preflight: Arc<dyn AggregatePreflightService> =
            Arc::new(DeterministicAggregatePreflightService::new(paths.clone()).for_lc(lc_id.clone()));
        let clock: Arc<Clock> = Arc::new(|| CREATED_AT.to_string());
        let coordinator = AggregateInitializationCoordinator::new(
            paths.clone(),
            AggregateInitializationOperationStore::new(paths.clone()),
            skills,
            preflight,
            provider,
            clock,
        )
        .for_lc(lc_id.clone());

        RecipeReplayFixture {
            _temp: temp,
            paths,
            lc_id,
            aggregate_root,
            manifest,
            member_id: member_id.0.to_string(),
            checkout_record,
            policy_digest: policy.digest,
            coordinator,
            service,
            audit,
            streaming_adapter,
        }
    }

    #[tokio::test]
    async fn recipe_replay_preflight_completed_placeholder_root_advances_to_precheck() {
        let fixture = recipe_replay_fixture(ReplayEvidenceBreak::Intact);

        // 独立确定性预检：receipt 证明的根入口直接通过，不要求移动任何文件。
        let snapshot = fixture
            .inspect_root()
            .expect("scoped deterministic inspect must accept the proven root");
        assert_eq!(snapshot.members.len(), 1);
        assert_eq!(snapshot.members[0].logical_repository_id, fixture.member_id);
        assert_eq!(
            PathBuf::from(&snapshot.aggregate_root),
            std::fs::canonicalize(&fixture.aggregate_root).unwrap()
        );

        // 新 key 通过 coordinator 建立新 operation 并完整推进：预检放行后
        // 进入 PreCheck provider turn，而不是停在 202/预检等待面。
        let operation = fixture
            .coordinator
            .begin(
                "aggregate_initialization_replay_new_0001".to_string(),
                "project_0001",
                fixture.new_recipe_input(),
            )
            .unwrap();
        let operation = fixture
            .coordinator
            .execute(
                "project_0001",
                &operation.operation_id,
                CancellationToken::new(),
            )
            .await
            .expect("producer-only replay preflight must advance to PreCheck");
        assert_eq!(
            operation.status,
            AggregateInitializationOperationStatus::Completed
        );
        let preflight_step = operation
            .steps
            .get(AggregateInitializationStepKind::AggregatePreflight.index())
            .expect("aggregate_preflight step present");
        assert_eq!(
            preflight_step.status,
            AggregateInitializationStepStatus::Completed
        );

        // 录制的 provider 确实收到原 PreCheck 命令（命令文本唯一来源）。
        use crate::product::repository_store::RepositoryInitializationStepKind as RepoStep;
        let inputs = fixture.streaming_adapter.started_inputs();
        assert_eq!(inputs.len(), 3, "three provider turns must run");
        assert_eq!(inputs[0].prompt, RepoStep::PreCheck.command().unwrap());
    }

    #[tokio::test]
    async fn recipe_replay_preflight_rejects_unproven_or_changed_entries() {
        let breaks = vec![
            ReplayEvidenceBreak::MissingFinalReceipt,
            ReplayEvidenceBreak::WrongScope,
            ReplayEvidenceBreak::WrongRoot,
            ReplayEvidenceBreak::OperationNotCompleted,
            ReplayEvidenceBreak::MissingCommand,
            ReplayEvidenceBreak::RejectedCommand,
            ReplayEvidenceBreak::CommandIdentityDrift,
            ReplayEvidenceBreak::RepeatedIndex,
            ReplayEvidenceBreak::SummaryReferenceMismatch,
            ReplayEvidenceBreak::AgentsRuleDigestMismatch,
            ReplayEvidenceBreak::AgentsAfterDigestMismatch,
            ReplayEvidenceBreak::AgentsCurrentBytesDrift,
            ReplayEvidenceBreak::ClaudeUntrackedNewFile,
            ReplayEvidenceBreak::ClaudeDigestMismatch,
            #[cfg(unix)]
            ReplayEvidenceBreak::AgentsSymlinkSameBytes,
            ReplayEvidenceBreak::AriaPresent,
        ];
        for break_kind in breaks {
            let fixture = recipe_replay_fixture(break_kind.clone());
            let before = replay_root_inventory(&fixture.aggregate_root);
            let operation = fixture
                .coordinator
                .begin(
                    "aggregate_initialization_replay_new_0001".to_string(),
                    "project_0001",
                    fixture.new_recipe_input(),
                )
                .unwrap();
            let result = fixture
                .coordinator
                .execute(
                    "project_0001",
                    &operation.operation_id,
                    CancellationToken::new(),
                )
                .await;
            let Err(AggregateInitializationError::Preflight { reason, .. }) = &result else {
                panic!("{break_kind:?} must fail preflight with ownership conflict, got {result:?}");
            };
            assert!(
                reason.contains("aggregate_root_ownership_conflict"),
                "{break_kind:?}: unexpected preflight reason: {reason}"
            );
            // 无证明或证据漂移：不启动任何后续 recipe 命令，用户 bytes 不变。
            assert_eq!(
                fixture.audit.stream_launches(),
                0,
                "{break_kind:?} must not launch any provider turn"
            );
            assert_eq!(fixture.streaming_adapter.start_count(), 0);
            assert_eq!(
                before,
                replay_root_inventory(&fixture.aggregate_root),
                "{break_kind:?} must not touch user bytes"
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn recipe_replay_preflight_revalidates_members_and_root_boundaries() {
        // 完整证据下逐一破坏成员/root 边界：recipe 复用入口继续运行原检查，
        // 返回原对应错误，而不是沿用旧成员投影放行。错误面断言用统一辅助。
        fn expect_preflight_reason(fixture: &RecipeReplayFixture, expected: &str) {
            let error = fixture
                .inspect_root()
                .expect_err("boundary break must fail the replay preflight");
            let rendered = format!("{error:?}");
            assert!(
                rendered.contains(expected),
                "expected reason {expected}, got: {rendered}"
            );
        }

        // 1) 成员越界：checkout 指向 root 外的真实 git 仓。
        let fixture = recipe_replay_fixture(ReplayEvidenceBreak::Intact);
        let outside = fixture._temp.path().join("outside-member");
        replay_init_member(&outside);
        fixture.point_member_checkout_at(&outside);
        expect_preflight_reason(&fixture, "member_path_outside_root");

        // 2) symlink escape：root 内 symlink 成员指向 root 外。
        let fixture = recipe_replay_fixture(ReplayEvidenceBreak::Intact);
        let outside = fixture._temp.path().join("symlink-target");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, fixture.aggregate_root.join("escaping-member")).unwrap();
        fixture.point_member_checkout_at(&fixture.aggregate_root.join("escaping-member"));
        expect_preflight_reason(&fixture, "member_symlink_escape");

        // 3) linked worktree：成员仓挂出的 worktree 不能冒充 main checkout。
        let fixture = recipe_replay_fixture(ReplayEvidenceBreak::Intact);
        let linked = fixture.aggregate_root.join("linked-wt");
        replay_git(
            &fixture.aggregate_root.join("repo"),
            &["worktree", "add", "--detach", linked.to_str().unwrap()],
        );
        fixture.point_member_checkout_at(&linked);
        expect_preflight_reason(&fixture, "nested_worktree");

        // 4) root 变 Git：预检提前拒绝，与旧成员投影无关。
        let fixture = recipe_replay_fixture(ReplayEvidenceBreak::Intact);
        std::fs::create_dir_all(fixture.aggregate_root.join(".git")).unwrap();
        expect_preflight_reason(&fixture, "is a Git repository");

        // 5) root 重叠：另一 project 的 manifest 覆盖同一根。
        let fixture = recipe_replay_fixture(ReplayEvidenceBreak::Intact);
        LogicalCodebaseStore::new(fixture.paths.clone())
            .save_manifest(
                "project_0002",
                &LogicalCodebaseManifest::new(
                    "project_0002",
                    std::fs::canonicalize(&fixture.aggregate_root).unwrap(),
                    Vec::new(),
                ),
            )
            .unwrap();
        expect_preflight_reason(&fixture, "aggregate_root_overlap");
    }
