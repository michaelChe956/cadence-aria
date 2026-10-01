#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::logical_codebase::aggregate_index::AggregateIndexRecord;
    use crate::product::logical_codebase::aggregate_initialization::{
        AggregateInitializationOperation, AggregateInitializationOperationInput,
        AggregateInitializationStepKind,
    };
    use crate::product::logical_codebase::aggregate_initialization_store::AggregateInitializationOperationStore;
    use crate::product::logical_codebase::store::LogicalCodebaseStore;
    use crate::product::logical_codebase::types::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, MemberStatus,
        RepositoryCheckoutRecord, RepositoryType,
    };
    use crate::product::logical_codebase::{
        IdentityMigrationJournal, IdentityMigrationJournalStore, IdentityMigrationPhase,
        LogicalCodebaseCreateInput, LogicalCodebaseManifest, LogicalRepositoryId,
        RepositoryCheckoutId,
    };
    use crate::product::project_store::{CreateProjectInput, ProjectStore};

    fn git(cwd: &std::path::Path, arguments: &[&str]) {
        let output = std::process::Command::new("git")
            .current_dir(cwd)
            .args(arguments)
            .output()
            .expect("git must start");
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_git_repository_with_commit(path: &std::path::Path) {
        std::fs::create_dir_all(path.join(".claude/rules")).unwrap();
        git(path, &["init", "-b", "main"]);
        git(path, &["config", "user.email", "bootstrap@test.local"]);
        git(path, &["config", "user.name", "Bootstrap Test"]);
        std::fs::write(path.join("README.md"), "# member\n").unwrap();
        // 真实成员规则材料（Task 10 起 rules_policy 步只读检查成员规则）。
        std::fs::write(path.join(".claude/rules/language.md"), "# rule\n").unwrap();
        git(path, &["add", "."]);
        git(path, &["commit", "-m", "init"]);
    }

    fn aria_inventory(root: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        let mut inventory = std::collections::BTreeMap::new();
        let mut stack = vec![root.join(".aria")];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
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

    fn create_project_and_lc(paths: &ProductAppPaths, aggregate_root: &std::path::Path) -> String {
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "bootstrap-project".to_string(),
                description: None,
            })
            .unwrap();
        std::fs::create_dir_all(aggregate_root).unwrap();
        LogicalCodebaseStore::new(paths.clone())
            .create(
                "project_0001",
                LogicalCodebaseCreateInput {
                    name: "bootstrap-lc".to_string(),
                    aggregate_root: aggregate_root.to_path_buf(),
                },
            )
            .unwrap()
            .id
    }

    #[test]
    fn bootstrap_projection_reports_missing_active_index_without_manual_seed() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let lc_id = create_project_and_lc(&paths, &temp.path().join("aggregate-root"));

        let before = aria_inventory(temp.path());
        let projection = LogicalCodebaseBootstrapProjector::new(paths.clone())
            .project("project_0001", &lc_id)
            .expect("empty LC must still project");
        let after = aria_inventory(temp.path());

        // 投影零写入：`.aria` durable inventory 字节不变。
        assert_eq!(before, after);

        // 五个步骤全部可投影（无手工 seed）。
        assert_eq!(projection.steps.len(), 5);
        let statuses: Vec<_> = projection
            .steps
            .iter()
            .map(|step| (step.step, step.status))
            .collect();
        for (step, status) in &statuses {
            assert_ne!(
                *status,
                LogicalCodebaseBootstrapStepStatus::Completed,
                "{} must not be completed on an empty LC",
                step.as_str()
            );
        }
        // aggregate step 为 NotStarted（无任何 index 记录）。
        let aggregate = statuses
            .iter()
            .find(|(step, _)| *step == LogicalCodebaseBootstrapStep::AggregateIndexActive)
            .expect("aggregate step present");
        assert_eq!(aggregate.1, LogicalCodebaseBootstrapStepStatus::NotStarted);
        assert!(!projection.planning_ready);
        assert_eq!(projection.policy, None);
        assert_eq!(projection.membership_revision, None);
        // 空投影的等待通知不产生（NotStarted 非等待事实）。
        assert!(projection.notices.is_empty());
    }

    #[test]
    fn bootstrap_projection_maps_existing_registration_and_initialization_checkpoints() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let aggregate_root = temp.path().join("aggregate-root");
        let lc_id = create_project_and_lc(&paths, &aggregate_root);

        // 真实成员仓 + manifest/member/checkout（identity 与 manifest 事实）。
        let repo = aggregate_root.join("repo");
        init_git_repository_with_commit(&repo);
        let canonical = std::fs::canonicalize(&repo).unwrap();
        let source =
            crate::product::repository_store::resolve_repository_source(&canonical).unwrap();
        let member_id = LogicalRepositoryId(uuid::Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
        let lc_store = LogicalCodebaseStore::for_lc(paths.clone(), &lc_id);
        let mut manifest =
            LogicalCodebaseManifest::new("project_0001", aggregate_root.clone(), Vec::new());
        manifest.member_ids = vec![member_id];
        lc_store.save_manifest("project_0001", &manifest).unwrap();
        lc_store
            .save_member(
                "project_0001",
                &CodebaseMemberRecord {
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_member".to_string(),
                    alias: "repo".to_string(),
                    role: "member".to_string(),
                    ordinal: 1,
                    source_identity: source.clone(),
                    repo_type: RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_id],
                    status: MemberStatus::Active,
                    created_at: "2026-09-29T00:00:00Z".to_string(),
                    updated_at: "2026-09-29T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        lc_store
            .save_checkout(
                "project_0001",
                &RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_member".to_string(),
                    kind: CheckoutKind::Main,
                    canonical_path: canonical.clone(),
                    checkout_path_hash: crate::product::id::repo_hash_for_path(
                        canonical.to_string_lossy().as_ref(),
                    ),
                    git_dir_identity: source.git_dir_identity(),
                    revision: None,
                    availability: CheckoutAvailability::Available,
                    observed_at: "2026-09-29T00:00:00Z".to_string(),
                    created_at: "2026-09-29T00:00:00Z".to_string(),
                    updated_at: "2026-09-29T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        let policy = crate::product::logical_codebase::policy::AggregatePolicyArtifact::bootstrap(
            "project_0001",
            &manifest.logical_codebase_id.to_string(),
            "2026-09-29T00:00:00Z".to_string(),
        );
        crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::for_lc(
            paths.clone(),
            &lc_id,
        )
        .save("project_0001", &policy)
        .unwrap();

        // 既有 aggregate initialization 五步全完成（durable checkpoint 来源）。
        let init_store = AggregateInitializationOperationStore::for_lc(paths.clone(), &lc_id);
        let operation = AggregateInitializationOperation::new(
            "aggregate_initialization_bootstrap_0001".to_string(),
            "project_0001".to_string(),
            AggregateInitializationOperationInput {
                idempotency_key: "bootstrap-0001".to_string(),
                manifest_revision: manifest.membership_revision,
                policy_digest: "sha256:policy".to_string(),
                profile_evidence_digest: Some("sha256:profile".to_string()),
                provider_context_root: aggregate_root.clone(),
                provider: "claude_code".to_string(),
            },
            "2026-09-29T00:00:00Z".to_string(),
        );
        init_store.create_idempotent(operation).unwrap();
        init_store
            .mark_running(
                "project_0001",
                "aggregate_initialization_bootstrap_0001",
                "2026-09-29T00:01:00Z".to_string(),
            )
            .unwrap();
        for step in AggregateInitializationStepKind::V1 {
            init_store
                .mark_step_running(
                    "project_0001",
                    "aggregate_initialization_bootstrap_0001",
                    step,
                    format!("aggregate-init:project_0001:op:{}:input", step.as_str()),
                    "2026-09-29T00:02:00Z".to_string(),
                )
                .unwrap();
            init_store
                .checkpoint_step_output(
                    "project_0001",
                    "aggregate_initialization_bootstrap_0001",
                    step,
                    format!("aggregate-initializations/op/{}.json", step.as_str()),
                    "2026-09-29T00:03:00Z".to_string(),
                )
                .unwrap();
            init_store
                .mark_step_completed(
                    "project_0001",
                    "aggregate_initialization_bootstrap_0001",
                    step,
                    "2026-09-29T00:04:00Z".to_string(),
                )
                .unwrap();
        }
        init_store
            .finish_completed(
                "project_0001",
                "aggregate_initialization_bootstrap_0001",
                "2026-09-29T00:05:00Z".to_string(),
            )
            .unwrap();

        // Task 1.6（REQ-BOOT-03）：readiness 三源材料——根规则 + 最终 receipt
        //（真实 auditor 四命令审计后 finalize）。
        std::fs::write(aggregate_root.join("AGENTS.md"), "# aggregate root rules\n").unwrap();
        let rule_digest = crate::product::logical_codebase::root_recipe_receipt::root_rule_digest(
            &std::fs::canonicalize(&aggregate_root).unwrap(),
        )
        .unwrap()
        .expect("root rule digest");
        let receipt = finalize_root_receipt(
            &paths,
            &lc_id,
            "aggregate_initialization_bootstrap_0001",
            &aggregate_root,
            &policy.digest,
            &rule_digest,
        );
        assert_eq!(receipt.policy_digest, policy.digest);

        let projection = LogicalCodebaseBootstrapProjector::new(paths.clone())
            .project("project_0001", &lc_id)
            .unwrap();

        let identity = projection
            .steps
            .iter()
            .find(|step| step.step == LogicalCodebaseBootstrapStep::Identity)
            .unwrap();
        assert_eq!(
            identity.status,
            LogicalCodebaseBootstrapStepStatus::Completed
        );

        let manifest_step = projection
            .steps
            .iter()
            .find(|step| step.step == LogicalCodebaseBootstrapStep::ManifestCheckout)
            .unwrap();
        assert_eq!(
            manifest_step.status,
            LogicalCodebaseBootstrapStepStatus::Completed
        );
        // object id 来自 durable manifest 记录，而非新文件。
        assert_eq!(
            manifest_step.object_id,
            manifest.logical_codebase_id.to_string()
        );

        let rules = projection
            .steps
            .iter()
            .find(|step| step.step == LogicalCodebaseBootstrapStep::RulesPolicy)
            .unwrap();
        assert_eq!(rules.status, LogicalCodebaseBootstrapStepStatus::Completed);
        assert!(rules.object_id.starts_with("policy/project_0001/"));
        // Task 1.6：完成 checkpoint 锚定最终 receipt（冻结 digest 与 finalize
        // 时间），receipt 是本步的 durable 输出物。
        let rules_checkpoint = rules.checkpoint.as_ref().unwrap();
        assert_eq!(
            rules_checkpoint.input_digest.as_deref(),
            Some(policy.digest.as_str())
        );
        assert_eq!(
            rules_checkpoint.output_artifact_ref.as_deref(),
            Some("aggregate-recipe-receipts/aggregate_initialization_bootstrap_0001.json")
        );
        assert_eq!(
            rules_checkpoint.completed_at.as_deref(),
            Some(receipt.finalized_at.as_str())
        );

        let member_index = projection
            .steps
            .iter()
            .find(|step| step.step == LogicalCodebaseBootstrapStep::MemberIndex)
            .unwrap();
        assert_eq!(
            member_index.status,
            LogicalCodebaseBootstrapStepStatus::Completed
        );
        // object/checkpoint 来自既有 operation（AggregatePreflight 步的 digest/ref）。
        assert_eq!(
            member_index.object_id,
            "aggregate_initialization_bootstrap_0001"
        );
        let checkpoint = member_index.checkpoint.as_ref().unwrap();
        assert_eq!(
            checkpoint.output_artifact_ref.as_deref(),
            Some("aggregate-initializations/op/aggregate_preflight.json")
        );
        assert!(checkpoint.input_digest.is_some());

        // 无 active index：aggregate step 仍未完成，planning_ready=false。
        let aggregate = projection
            .steps
            .iter()
            .find(|step| step.step == LogicalCodebaseBootstrapStep::AggregateIndexActive)
            .unwrap();
        assert_eq!(
            aggregate.status,
            LogicalCodebaseBootstrapStepStatus::NotStarted
        );
        assert!(!projection.planning_ready);

        // Task 1.6（REQ-REG-10）：AggregateIndexActive 独立完成后 readiness
        // 闭环——五步全部 Completed，planning_ready=true。
        let index_store = AggregateIndexStore::for_lc(paths.clone(), &lc_id);
        index_store
            .create(
                "project_0001",
                AggregateIndexRecord::building(
                    "aggregate_index_bootstrap_0001".to_string(),
                    "project_0001".to_string(),
                    manifest.membership_revision,
                    Vec::new(),
                    "2026-09-29T00:06:00Z".to_string(),
                ),
            )
            .unwrap();
        index_store
            .mark_status(
                "project_0001",
                "aggregate_index_bootstrap_0001",
                AggregateIndexStatus::Active,
                None,
            )
            .unwrap();
        let ready = LogicalCodebaseBootstrapProjector::new(paths.clone())
            .project("project_0001", &lc_id)
            .unwrap();
        assert!(ready.planning_ready);
        for step in &ready.steps {
            assert_eq!(
                step.status,
                LogicalCodebaseBootstrapStepStatus::Completed,
                "step {} must complete for the readiness loop",
                step.step.as_str()
            );
        }
    }

    #[test]
    fn aggregate_initialization_v1_steps_are_not_renamed_to_c4_steps() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let lc_id = create_project_and_lc(&paths, &temp.path().join("aggregate-root"));

        let init_store = AggregateInitializationOperationStore::for_lc(paths.clone(), &lc_id);
        let operation = AggregateInitializationOperation::new(
            "aggregate_initialization_names_0001".to_string(),
            "project_0001".to_string(),
            AggregateInitializationOperationInput {
                idempotency_key: "names-0001".to_string(),
                manifest_revision: 1,
                policy_digest: "sha256:policy".to_string(),
                profile_evidence_digest: None,
                provider_context_root: temp.path().join("aggregate-root"),
                provider: "claude_code".to_string(),
            },
            "2026-09-29T00:00:00Z".to_string(),
        );
        init_store.create_idempotent(operation).unwrap();
        let persisted = init_store
            .get("project_0001", "aggregate_initialization_names_0001")
            .unwrap();
        // 既有五步协议保持原名（V1 顺序），未被 C4 步骤冒充。
        let persisted_kinds: Vec<_> = persisted.steps.iter().map(|step| step.step_id).collect();
        assert_eq!(
            persisted_kinds,
            AggregateInitializationStepKind::V1.to_vec()
        );

        let projection = LogicalCodebaseBootstrapProjector::new(paths.clone())
            .project("project_0001", &lc_id)
            .unwrap();
        // bootstrap 投影是独立的 C4 五步，与 V1 协议不同名。
        let bootstrap_steps: Vec<_> = projection.steps.iter().map(|step| step.step).collect();
        assert_eq!(bootstrap_steps, LogicalCodebaseBootstrapStep::V1.to_vec());
        let bootstrap_names: Vec<_> = bootstrap_steps.iter().map(|s| s.as_str()).collect();
        let v1_names: Vec<String> = persisted_kinds
            .iter()
            .map(|s| s.as_str().to_string())
            .collect();
        assert_ne!(bootstrap_names, v1_names);
    }

    // ---- Task 1.6（REQ-BOOT-03/REQ-REG-10）：recipe/readiness 双状态机闭环 ----

    struct ReadinessFixture {
        temp: tempfile::TempDir,
        paths: ProductAppPaths,
        lc_id: String,
        aggregate_root: std::path::PathBuf,
        manifest: LogicalCodebaseManifest,
        policy: crate::product::logical_codebase::policy::AggregatePolicyArtifact,
        operation_id: String,
    }

    impl ReadinessFixture {
        fn project(&self) -> LogicalCodebaseBootstrapProjection {
            LogicalCodebaseBootstrapProjector::new(self.paths.clone())
                .project("project_0001", &self.lc_id)
                .expect("readiness projection must stay read-only and total")
        }

        fn rules_step(projection: &LogicalCodebaseBootstrapProjection) -> &BootstrapStepProjection {
            projection
                .steps
                .iter()
                .find(|step| step.step == LogicalCodebaseBootstrapStep::RulesPolicy)
                .expect("rules_policy step present")
        }
    }

    /// 登记成员 + bootstrap policy + 五步全 Completed 的 recipe operation：
    /// readiness 三源谓词的全部 durable 前置（最终 receipt 除外）。
    fn readiness_fixture(operation_id: &str) -> ReadinessFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let aggregate_root = temp.path().join("aggregate-root");
        let lc_id = create_project_and_lc(&paths, &aggregate_root);

        let repo = aggregate_root.join("repo");
        init_git_repository_with_commit(&repo);
        let canonical = std::fs::canonicalize(&repo).unwrap();
        let source =
            crate::product::repository_store::resolve_repository_source(&canonical).unwrap();
        let member_id = LogicalRepositoryId(uuid::Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
        let lc_store = LogicalCodebaseStore::for_lc(paths.clone(), &lc_id);
        let mut manifest =
            LogicalCodebaseManifest::new("project_0001", aggregate_root.clone(), Vec::new());
        manifest.member_ids = vec![member_id];
        lc_store.save_manifest("project_0001", &manifest).unwrap();
        lc_store
            .save_member(
                "project_0001",
                &CodebaseMemberRecord {
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_member".to_string(),
                    alias: "repo".to_string(),
                    role: "member".to_string(),
                    ordinal: 1,
                    source_identity: source.clone(),
                    repo_type: RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_id],
                    status: MemberStatus::Active,
                    created_at: "2026-09-29T00:00:00Z".to_string(),
                    updated_at: "2026-09-29T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        lc_store
            .save_checkout(
                "project_0001",
                &RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_member".to_string(),
                    kind: CheckoutKind::Main,
                    canonical_path: canonical.clone(),
                    checkout_path_hash: crate::product::id::repo_hash_for_path(
                        canonical.to_string_lossy().as_ref(),
                    ),
                    git_dir_identity: source.git_dir_identity(),
                    revision: None,
                    availability: CheckoutAvailability::Available,
                    observed_at: "2026-09-29T00:00:00Z".to_string(),
                    created_at: "2026-09-29T00:00:00Z".to_string(),
                    updated_at: "2026-09-29T00:00:00Z".to_string(),
                },
            )
            .unwrap();

        let policy = crate::product::logical_codebase::policy::AggregatePolicyArtifact::bootstrap(
            "project_0001",
            &manifest.logical_codebase_id.to_string(),
            "2026-10-01T00:00:00Z".to_string(),
        );
        crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::for_lc(
            paths.clone(),
            &lc_id,
        )
        .save("project_0001", &policy)
        .unwrap();

        let init_store = AggregateInitializationOperationStore::for_lc(paths.clone(), &lc_id);
        init_store
            .create_idempotent(AggregateInitializationOperation::new(
                operation_id.to_string(),
                "project_0001".to_string(),
                AggregateInitializationOperationInput {
                    idempotency_key: format!("readiness-{operation_id}"),
                    manifest_revision: manifest.membership_revision,
                    policy_digest: policy.digest.clone(),
                    profile_evidence_digest: Some("sha256:profile".to_string()),
                    provider_context_root: aggregate_root.clone(),
                    provider: "claude_code".to_string(),
                },
                "2026-10-01T00:01:00Z".to_string(),
            ))
            .unwrap();
        init_store
            .mark_running(
                "project_0001",
                operation_id,
                "2026-10-01T00:02:00Z".to_string(),
            )
            .unwrap();
        for step in AggregateInitializationStepKind::V1 {
            init_store
                .mark_step_running(
                    "project_0001",
                    operation_id,
                    step,
                    format!("readiness:{operation_id}:{}", step.as_str()),
                    "2026-10-01T00:03:00Z".to_string(),
                )
                .unwrap();
            init_store
                .checkpoint_step_output(
                    "project_0001",
                    operation_id,
                    step,
                    format!("aggregate-initializations/op/{}.json", step.as_str()),
                    "2026-10-01T00:04:00Z".to_string(),
                )
                .unwrap();
            init_store
                .mark_step_completed(
                    "project_0001",
                    operation_id,
                    step,
                    "2026-10-01T00:05:00Z".to_string(),
                )
                .unwrap();
        }
        init_store
            .finish_completed(
                "project_0001",
                operation_id,
                "2026-10-01T00:06:00Z".to_string(),
            )
            .unwrap();

        ReadinessFixture {
            temp,
            paths,
            lc_id,
            aggregate_root,
            manifest,
            policy,
            operation_id: operation_id.to_string(),
        }
    }

    /// 用真实 auditor 走完四条命令审计并 `finalize` 最终 receipt（Task 1.5
    /// 生产链路的同构 fixture：命令无副作用 → 四条全部 Allowed）。
    fn finalize_root_receipt(
        paths: &ProductAppPaths,
        lc_id: &str,
        operation_id: &str,
        aggregate_root: &std::path::Path,
        policy_digest: &str,
        rule_digest: &str,
    ) -> crate::product::logical_codebase::RootRecipeReceipt {
        let store =
            crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(paths.clone(), lc_id);
        let auditor = crate::product::logical_codebase::RootRecipeFilesystemAuditor::new();
        let canonical_root = std::fs::canonicalize(aggregate_root).unwrap();
        for (index, (step, command_index, command)) in
            crate::product::logical_codebase::root_recipe_command_index()
                .into_iter()
                .enumerate()
        {
            let watch = auditor
                .before_command(operation_id, &canonical_root, step, command_index, command)
                .unwrap();
            let receipt = auditor
                .after_command(watch, format!("2026-10-01T00:10:{index:02}Z"))
                .unwrap();
            assert_eq!(
                receipt.verdict,
                crate::product::logical_codebase::RootRecipeCommandVerdict::Allowed
            );
            store.append_command("project_0001", receipt).unwrap();
        }
        store
            .finalize(
                "project_0001",
                operation_id,
                policy_digest,
                rule_digest,
                "2026-10-01T00:20:00Z".to_string(),
            )
            .unwrap()
    }

    /// 全树字节快照（含成员 `.git` 与 app-data）：readiness GET 零写入的
    /// 强断言面——不只限 `.aria`。
    fn full_tree_inventory(root: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
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

    /// Task 1.6（REQ-BOOT-03）：recipe operation Completed 但最终 root
    /// receipt 缺失时，RulesPolicy 保持可操作等待（root_receipt_missing）、
    /// `planning_ready == false`；MemberIndex 步仍独立由五步 checkpoint 判定
    /// Completed——两套五步状态机互不冒充。
    #[test]
    fn recipe_completed_without_root_receipt_keeps_planning_not_ready() {
        let fixture = readiness_fixture("aggregate_initialization_readiness_0001");
        // 根规则在场：隔离「receipt 缺失」这一唯一 readiness 缺口。
        std::fs::write(
            fixture.aggregate_root.join("AGENTS.md"),
            "# aggregate root rules\n",
        )
        .unwrap();

        let projection = fixture.project();
        let rules = ReadinessFixture::rules_step(&projection);
        assert_eq!(
            rules.status,
            LogicalCodebaseBootstrapStepStatus::WaitingForHuman
        );
        let failure = rules.failure.as_ref().expect("failure reason");
        assert_eq!(failure.reason_code, "root_receipt_missing");
        assert!(!rules.allowed_actions.is_empty());
        assert!(rules.allowed_actions.contains(&BootstrapActionKind::Retry));
        assert!(
            rules
                .allowed_actions
                .contains(&BootstrapActionKind::Revalidate)
        );

        let member_index = projection
            .steps
            .iter()
            .find(|step| step.step == LogicalCodebaseBootstrapStep::MemberIndex)
            .expect("member_index step present");
        assert_eq!(
            member_index.status,
            LogicalCodebaseBootstrapStepStatus::Completed
        );

        assert!(!projection.planning_ready);
        let notice = projection
            .notices
            .iter()
            .find(|notice| notice.step == LogicalCodebaseBootstrapStep::RulesPolicy)
            .expect("waiting rules_policy step must surface an actionable notice");
        assert_eq!(notice.reason_code, "root_receipt_missing");
        assert!(!notice.allowed_actions.is_empty());
    }

    /// Task 1.6（REQ-BOOT-03）：receipt 冻结的 canonical root/policy/rule
    /// 身份与当前事实漂移（或根规则缺失）时，RulesPolicy 逐项给出稳定
    /// reason_code 且 `planning_ready == false`；材料恢复一致后谓词可重入
    /// 地回到 Completed（不是单向锁）。
    #[test]
    fn policy_rule_digest_drift_keeps_planning_not_ready() {
        let fixture = readiness_fixture("aggregate_initialization_readiness_0002");
        let entry = fixture.aggregate_root.join("AGENTS.md");
        let root_rule_text = "# aggregate root rules\n";
        std::fs::write(&entry, root_rule_text).unwrap();
        let rule_digest = crate::product::logical_codebase::root_recipe_receipt::root_rule_digest(
            &std::fs::canonicalize(&fixture.aggregate_root).unwrap(),
        )
        .unwrap()
        .expect("root rule digest");
        let receipt = finalize_root_receipt(
            &fixture.paths,
            &fixture.lc_id,
            &fixture.operation_id,
            &fixture.aggregate_root,
            &fixture.policy.digest,
            &rule_digest,
        );
        assert_eq!(receipt.policy_digest, fixture.policy.digest);
        assert_eq!(receipt.rule_digest, rule_digest);

        // 基线：三源一致 → RulesPolicy Completed；planning_ready 仍受
        // AggregateIndexActive 独立门控（无 active index）。
        let ready = fixture.project();
        assert_eq!(
            ReadinessFixture::rules_step(&ready).status,
            LogicalCodebaseBootstrapStepStatus::Completed
        );
        assert!(
            !ready.planning_ready,
            "aggregate index gate must stay independent"
        );

        // 根规则内容漂移 → rule_digest_drift。
        std::fs::write(&entry, "# aggregate root rules (drifted)\n").unwrap();
        let drifted = fixture.project();
        let rules = ReadinessFixture::rules_step(&drifted);
        assert_eq!(
            rules.status,
            LogicalCodebaseBootstrapStepStatus::WaitingForHuman
        );
        assert_eq!(
            rules.failure.as_ref().unwrap().reason_code,
            "rule_digest_drift"
        );
        assert!(!drifted.planning_ready);

        // 根规则缺失 → root_rule_missing。
        std::fs::remove_file(&entry).unwrap();
        let missing = fixture.project();
        assert_eq!(
            ReadinessFixture::rules_step(&missing)
                .failure
                .as_ref()
                .unwrap()
                .reason_code,
            "root_rule_missing"
        );
        assert!(!missing.planning_ready);

        // 恢复一致 → Completed（可重入谓词）。
        std::fs::write(&entry, root_rule_text).unwrap();
        assert_eq!(
            ReadinessFixture::rules_step(&fixture.project()).status,
            LogicalCodebaseBootstrapStepStatus::Completed
        );

        // policy 升级（digest 前移）→ receipt 冻结摘要漂移。
        let revised = fixture.policy.with_revised_policy(
            "# Aggregate policy (revised)\n",
            "2026-10-01T00:30:00Z".to_string(),
        );
        crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::for_lc(
            fixture.paths.clone(),
            &fixture.lc_id,
        )
        .save("project_0001", &revised)
        .unwrap();
        let policy_drift = fixture.project();
        let rules = ReadinessFixture::rules_step(&policy_drift);
        assert_eq!(
            rules.status,
            LogicalCodebaseBootstrapStepStatus::WaitingForHuman
        );
        assert_eq!(
            rules.failure.as_ref().unwrap().reason_code,
            "policy_digest_drift"
        );
        assert!(!policy_drift.planning_ready);

        // authority root 漂移（manifest 指向别的 root）→ receipt 冻结的
        // canonical root 失配。
        let other_root = fixture.temp.path().join("other-root");
        std::fs::create_dir_all(&other_root).unwrap();
        let mut moved = fixture.manifest.clone();
        moved.provider_context_root = other_root;
        LogicalCodebaseStore::for_lc(fixture.paths.clone(), &fixture.lc_id)
            .save_manifest("project_0001", &moved)
            .unwrap();
        let authority_drift = fixture.project();
        let rules = ReadinessFixture::rules_step(&authority_drift);
        assert_eq!(
            rules.failure.as_ref().unwrap().reason_code,
            "root_receipt_authority_drift"
        );
        assert!(!authority_drift.planning_ready);
    }

    /// Task 1.6（REQ-REG-10）：readiness GET 投影零写入、可重复读、零副作用
    /// 通道——不启动 provider/index/checkout；等待项给出可操作 allowed
    /// actions；bootstrap 服务侧的 run 探针/重建派发计数保持不变。
    #[test]
    fn readiness_projection_is_read_only() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let fixture = readiness_fixture("aggregate_initialization_readiness_0003");
        std::fs::write(
            fixture.aggregate_root.join("AGENTS.md"),
            "# aggregate root rules\n",
        )
        .unwrap();

        let inventory_before = full_tree_inventory(fixture.temp.path());

        let probe_calls = std::sync::Arc::new(AtomicUsize::new(0));
        let rebuilds = std::sync::Arc::new(AtomicUsize::new(0));
        let _service = LogicalCodebaseBootstrapService::new(fixture.paths.clone())
            .with_member_index_run_probe({
                let probe_calls = probe_calls.clone();
                std::sync::Arc::new(move |_project: &str, _lc: &str, _operation: &str| {
                    probe_calls.fetch_add(1, Ordering::SeqCst);
                    true
                })
            })
            .with_aggregate_index_rebuild({
                let rebuilds = rebuilds.clone();
                std::sync::Arc::new(move |_project: &str, _command: &str, _revision: u64| {
                    rebuilds.fetch_add(1, Ordering::SeqCst);
                    Err(AggregateIndexError::Failed {
                        code: "unexpected_rebuild",
                        message: "readiness GET must not dispatch rebuilds".to_string(),
                    })
                })
            });

        let first = fixture.project();
        let second = fixture.project();
        assert_eq!(
            first, second,
            "repeated GET must re-read the same durable facts without side effects"
        );
        assert!(!first.planning_ready);
        let rules = ReadinessFixture::rules_step(&first);
        assert_eq!(
            rules.status,
            LogicalCodebaseBootstrapStepStatus::WaitingForHuman
        );
        assert!(!rules.allowed_actions.is_empty());
        for action in &rules.allowed_actions {
            assert!(
                ["prepare", "continue", "retry", "revalidate", "repair"].contains(&action.as_str()),
                "allowed actions must stay operable product verbs"
            );
        }

        let inventory_after = full_tree_inventory(fixture.temp.path());
        assert_eq!(
            inventory_before, inventory_after,
            "readiness GET must not write any durable fact (app data or aggregate root)"
        );
        assert_eq!(probe_calls.load(Ordering::SeqCst), 0);
        assert_eq!(rebuilds.load(Ordering::SeqCst), 0);
    }

    fn seed_failed_aggregate_index_record(
        paths: &ProductAppPaths,
        lc_id: &str,
        command_id: &str,
    ) -> String {
        let store = AggregateIndexStore::for_lc(paths.clone(), lc_id);
        let mut record = AggregateIndexRecord::building(
            "aggregate_index_g3_failed_0001".to_string(),
            "project_0001".to_string(),
            2,
            Vec::new(),
            chrono::Utc::now().to_rfc3339(),
        );
        record.command_id = Some(command_id.to_string());
        store.create("project_0001", record).unwrap();
        store
            .mark_status(
                "project_0001",
                "aggregate_index_g3_failed_0001",
                AggregateIndexStatus::Failed,
                Some("codegraph_version_mismatch: expected 1.6.0, got 1.6.1".to_string()),
            )
            .unwrap();
        "aggregate_index_g3_failed_0001".to_string()
    }

    fn g3_retry_request(lc_id: &str, command_id: &str) -> BootstrapActionRequest {
        BootstrapActionRequest {
            command_id: command_id.to_string(),
            project_id: "project_0001".to_string(),
            logical_codebase_id: lc_id.to_string(),
            step: LogicalCodebaseBootstrapStep::AggregateIndexActive,
            action: BootstrapActionKind::Retry,
            expected_revision: Some(2),
            expected_object_id: "aggregate_index_g3_failed_0001".to_string(),
        }
    }

    /// G3（终局关闸缺口）：aggregate_index_active 步失败后，bootstrap
    /// actions retry 此前仅按 command_id 查重放记录——找不到即 NotFound
    /// 恒 500，无重建触发分支（现场 cmd-gapfix-a03-retry-1/2）。修复：
    /// 注入重建派发器（复用 `build_with_command_id` 的幂等命令语义）后，
    /// retry 用新 command_id 派发重建；同 command 重放不重复派发。
    #[test]
    fn aggregate_index_retry_dispatches_rebuild_and_replays_same_command() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let lc_id = create_project_and_lc(&paths, &temp.path().join("aggregate-root"));
        seed_failed_aggregate_index_record(&paths, &lc_id, "cmd-gapfix-a03-first");

        let dispatches = std::sync::Arc::new(AtomicUsize::new(0));
        let store_for_rebuild = AggregateIndexStore::for_lc(paths.clone(), &lc_id);
        let dispatches_for_rebuild = dispatches.clone();
        let dispatcher =
            std::sync::Arc::new(move |project_id: &str, command_id: &str, revision: u64| {
                dispatches_for_rebuild.fetch_add(1, Ordering::SeqCst);
                // 复刻 build_with_command_id 成功效果：落一条携带该命令
                // 身份的 Active 记录（membership_revision 对齐请求）。
                let mut record = AggregateIndexRecord::building(
                    format!("aggregate_index_retry_{command_id}"),
                    project_id.to_string(),
                    revision,
                    Vec::new(),
                    chrono::Utc::now().to_rfc3339(),
                );
                record.status = AggregateIndexStatus::Active;
                record.command_id = Some(command_id.to_string());
                store_for_rebuild
                    .create(project_id, record.clone())
                    .map(|_| record)
            });
        let service = LogicalCodebaseBootstrapService::new(paths.clone())
            .with_aggregate_index_rebuild(dispatcher);

        // 修复前现场：NotFound 恒 500（无重建分支）。
        let request = g3_retry_request(&lc_id, "cmd-gapfix-a03-retry-1");
        let outcome = service.dispatch_action(&request).expect("retry rebuilds");
        assert_eq!(outcome, BootstrapActionOutcome::Completed);
        assert_eq!(dispatches.load(Ordering::SeqCst), 1);
        // 重建派发参数经 durable 事实复核（project/command/revision）。
        let rebuilt = AggregateIndexStore::for_lc(paths.clone(), &lc_id)
            .get(
                "project_0001",
                "aggregate_index_retry_cmd-gapfix-a03-retry-1",
            )
            .expect("rebuilt record")
            .expect("rebuilt record present");
        assert_eq!(rebuilt.status, AggregateIndexStatus::Active);
        assert_eq!(
            rebuilt.command_id.as_deref(),
            Some("cmd-gapfix-a03-retry-1")
        );
        assert_eq!(rebuilt.membership_revision, 2);

        // 同 command 重放：返回同一 Active 事实（Completed），不再派发。
        let replay = service
            .dispatch_action(&request)
            .expect("replay returns durable result");
        assert_eq!(replay, BootstrapActionOutcome::Completed);
        assert_eq!(dispatches.load(Ordering::SeqCst), 1);
    }

    /// G3 语义收口：未注入重建派发器（旧构造）时保持既有 NotFound——
    /// 不静默假成功；由 web 层生产注入收口。
    #[test]
    fn aggregate_index_retry_without_dispatcher_keeps_not_found() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let lc_id = create_project_and_lc(&paths, &temp.path().join("aggregate-root"));
        seed_failed_aggregate_index_record(&paths, &lc_id, "cmd-gapfix-a03-first");

        let service = LogicalCodebaseBootstrapService::new(paths);
        let error = service
            .dispatch_action(&g3_retry_request(&lc_id, "cmd-gapfix-a03-retry-2"))
            .expect_err("no dispatcher keeps fail-closed NotFound");
        assert!(matches!(
            error,
            ProductStoreError::NotFound {
                kind: "aggregate_index_command",
                ..
            }
        ));
    }
}
