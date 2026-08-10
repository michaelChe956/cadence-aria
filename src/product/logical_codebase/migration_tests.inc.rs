#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::json_store::write_json;
    use crate::product::logical_codebase::LogicalCodebaseStore;
    use crate::product::models::RepositoryRecord;

    #[test]
    fn journal_preserves_uuid_mapping_and_phase_for_crash_replay() {
        let journal = IdentityMigrationJournal::new("project_0001", "sha256:legacy-repos");
        assert_eq!(journal.phase, IdentityMigrationPhase::Scanning);
        assert_eq!(journal.migration_id, "identity-migration:project_0001:v1");
        assert_eq!(journal.target_schema_version, 1);
        assert_eq!(journal.read_mode, None);
        assert_eq!(
            serde_json::to_value(&journal).unwrap()["journal_version"],
            1
        );
    }

    #[test]
    fn journal_store_round_trips_the_project_scoped_journal() {
        let directory = tempfile::tempdir().expect("temporary product root");
        let store = IdentityMigrationJournalStore::new(ProductAppPaths::new(directory.path()));
        let journal = IdentityMigrationJournal::new("project_0001", "sha256:legacy-repos");

        assert_eq!(store.load("project_0001").expect("load missing"), None);
        store
            .save("project_0001", &journal)
            .expect("save project journal");

        assert_eq!(
            store.load("project_0001").expect("load journal"),
            Some(journal)
        );
    }

    #[test]
    fn active_legacy_attempt_blocks_switch_but_terminal_attempt_gets_observed_snapshot() {
        let fixture = migration_fixture_with_one_git_repository();
        fixture.write_active_legacy_attempt_without_target_snapshot();
        let executor = IdentityMigrationExecutor::new(fixture.paths.clone());

        let error = executor.ensure_identity_schema("project_0001").unwrap_err();
        assert!(
            error.to_string().contains("target_snapshot_missing"),
            "unexpected migration error: {error}"
        );

        fixture.mark_attempt_completed();
        executor.ensure_identity_schema("project_0001").unwrap();
        let attempt = fixture.read_attempt();
        assert_eq!(
            attempt.target_snapshot.unwrap().capture_source,
            "migration_observed"
        );
        assert_eq!(
            fixture.journal().read_mode.as_deref(),
            Some("logical_authoritative")
        );
    }

    #[test]
    fn verifier_ignores_non_attempt_json_beneath_coding_attempt_directory() {
        let fixture = migration_fixture_with_one_git_repository();
        fixture.write_active_legacy_attempt_without_target_snapshot();
        fixture.mark_attempt_completed();
        IdentityMigrationExecutor::new(fixture.paths.clone())
            .ensure_identity_schema("project_0001")
            .expect("migrate terminal legacy attempt");
        fixture.write_non_attempt_json_beneath_coding_attempt_directory();
        fixture.set_journal_read_mode("dual");

        IdentityMigrationVerifier::new(fixture.paths.clone())
            .verify("project_0001")
            .expect("attempt verifier must ignore child records");
    }

    #[test]
    fn authority_write_crash_replays_the_same_mapping_without_duplicate_members() {
        let fixture = migration_fixture_with_one_git_repository();
        let failing = IdentityMigrationExecutor::with_fault_injector(
            fixture.paths.clone(),
            Arc::new(FailAfterAuthorityWrite::new()),
        );
        assert!(failing.ensure_through_authority("project_0001").is_err());

        let first = IdentityMigrationJournalStore::new(fixture.paths.clone())
            .load("project_0001")
            .unwrap()
            .unwrap();
        let first_mapping = first.mappings[0].clone();
        assert!(first_mapping.authority_written);

        IdentityMigrationExecutor::new(fixture.paths.clone())
            .ensure_through_authority("project_0001")
            .unwrap();
        let second = IdentityMigrationJournalStore::new(fixture.paths.clone())
            .load("project_0001")
            .unwrap()
            .unwrap();
        let members = LogicalCodebaseStore::new(fixture.paths.clone())
            .list_members("project_0001")
            .unwrap();

        assert_eq!(
            second.mappings[0].logical_repository_id,
            first_mapping.logical_repository_id
        );
        assert_eq!(
            second.mappings[0].primary_checkout_id,
            first_mapping.primary_checkout_id
        );
        assert_eq!(members.len(), 1);
    }

    struct MigrationFixture {
        _root: tempfile::TempDir,
        paths: ProductAppPaths,
    }

    fn migration_fixture_with_one_git_repository() -> MigrationFixture {
        let root = tempfile::tempdir().expect("temporary product root");
        let repository_path = root.path().join("repository");
        run_git_command(&["init", "--quiet", repository_path.to_str().unwrap()]);
        run_git_command_in(
            &repository_path,
            &[
                "remote",
                "add",
                "origin",
                "ssh://git@example.test/acme/api.git",
            ],
        );

        let paths = ProductAppPaths::new(root.path());
        let record = RepositoryRecord {
            id: "repository_0001".to_string(),
            project_id: "project_0001".to_string(),
            name: "api".to_string(),
            path: repository_path,
            repo_hash: "legacy-hash".to_string(),
            runtime_root: PathBuf::from("/unused/.aria/runtime"),
            default_policy_preset: "manual-write".to_string(),
            default_provider_mode: "fake".to_string(),
            created_at: "2026-08-08T00:00:00Z".to_string(),
            updated_at: "2026-08-08T00:00:00Z".to_string(),
            logical_repository_id: None,
            primary_checkout_id: None,
            identity_schema_version: 0,
        };
        write_json(
            &paths.project_root("project_0001").join("repos.json"),
            &vec![record],
        )
        .expect("write legacy repositories");
        write_json(
            &paths
                .issue_root("project_0001", "issue_0001")
                .join("issue.json"),
            &serde_json::json!({
                "id": "issue_0001",
                "project_id": "project_0001",
                "repo_id": "repository_0001",
                "author": null,
                "title": "legacy issue",
                "description": null,
                "change_id": "legacy",
                "phase": "clarification",
                "status": "draft",
                "active_binding_id": null,
                "created_at": "2026-08-08T00:00:00Z",
                "updated_at": "2026-08-08T00:00:00Z"
            }),
        )
        .expect("write legacy issue");
        MigrationFixture { _root: root, paths }
    }

    impl MigrationFixture {
        fn attempt_path(&self) -> PathBuf {
            self.paths
                .issue_root("project_0001", "issue_0001")
                .join("coding-attempts")
                .join("coding_attempt_0001.json")
        }

        fn write_active_legacy_attempt_without_target_snapshot(&self) {
            write_json(
                &self
                    .paths
                    .issue_root("project_0001", "issue_0001")
                    .join("work-items")
                    .join("work_item_0001.json"),
                &serde_json::json!({
                    "id": "work_item_0001",
                    "project_id": "project_0001",
                    "issue_id": "issue_0001",
                    "repository_id": "repository_0001",
                    "story_spec_ids": [],
                    "design_spec_ids": [],
                    "title": "legacy work item",
                    "plan_status": "not_started",
                    "execution_status": "pending",
                    "worktree_path": null,
                    "created_at": "2026-08-08T00:00:00Z",
                    "updated_at": "2026-08-08T00:00:00Z"
                }),
            )
            .expect("write legacy work item");
            write_json(
                &self.attempt_path(),
                &serde_json::json!({
                    "id": "coding_attempt_0001",
                    "project_id": "project_0001",
                    "issue_id": "issue_0001",
                    "work_item_id": "work_item_0001",
                    "attempt_no": 1,
                    "status": "running",
                    "stage": "coding",
                    "base_branch": "main",
                    "branch_name": "aria/legacy",
                    "worktree_path": null,
                    "provider_config_snapshot": {
                        "author": "fake",
                        "reviewer": null,
                        "review_rounds": 0,
                        "permission_modes": {"author": "auto", "reviewer": "auto"}
                    },
                    "rework_count": 0,
                    "max_auto_rework": 0,
                    "head_commit": null,
                    "pushed_remote": null,
                    "review_request_id": null,
                    "created_at": "2026-08-08T00:00:00Z",
                    "updated_at": "2026-08-08T00:00:00Z",
                    "completed_at": null
                }),
            )
            .expect("write legacy attempt");
        }

        fn mark_attempt_completed(&self) {
            let mut value: serde_json::Value =
                read_json(&self.attempt_path()).expect("read attempt JSON");
            value["status"] = serde_json::Value::String("completed".to_string());
            value["completed_at"] = serde_json::Value::String("2026-08-08T00:01:00Z".to_string());
            write_json(&self.attempt_path(), &value).expect("write completed attempt");
        }

        fn read_attempt(&self) -> crate::product::coding_models::CodingExecutionAttempt {
            read_json(&self.attempt_path()).expect("read migrated attempt")
        }

        fn write_non_attempt_json_beneath_coding_attempt_directory(&self) {
            write_json(
                &self
                    .paths
                    .issue_root("project_0001", "issue_0001")
                    .join("coding-attempts")
                    .join("coding_attempt_0001")
                    .join("units")
                    .join("coding_unit_0001.json"),
                &serde_json::json!({"kind": "coding_execution_unit"}),
            )
            .expect("write non-attempt child record");
        }

        fn set_journal_read_mode(&self, read_mode: &str) {
            let mut journal = self.journal();
            journal.read_mode = Some(read_mode.to_string());
            IdentityMigrationJournalStore::new(self.paths.clone())
                .save("project_0001", &journal)
                .expect("save journal read mode");
        }

        fn journal(&self) -> IdentityMigrationJournal {
            IdentityMigrationJournalStore::new(self.paths.clone())
                .load("project_0001")
                .expect("load journal")
                .expect("journal")
        }
    }

    #[test]
    fn migration_executor_write_issue_selection_tolerates_new_format() {
        let fixture = migration_fixture_with_one_git_repository();
        let executor = IdentityMigrationExecutor::new(fixture.paths.clone());
        executor
            .ensure_through_authority("project_0001")
            .expect("migrate through authority");
        let mapping = fixture.journal().mappings.remove(0);
        let selections = crate::product::logical_codebase::IssueCodebaseSelectionStore::new(
            fixture.paths.clone(),
        );
        selections
            .save(
                &crate::product::logical_codebase::IssueCodebaseSelection::explicit(
                    "project_0001",
                    "issue_0001",
                    vec![mapping.logical_repository_id],
                    Vec::new(),
                    vec![mapping.logical_repository_id],
                    None,
                ),
            )
            .expect("seed authoritative selection");

        executor
            .write_issue_selection(
                &fixture.paths.issue_root("project_0001", "issue_0001"),
                &mapping,
            )
            .expect("new format is already migrated");
    }

    #[test]
    fn migration_persists_bootstrap_policy_artifact_after_authority_write() {
        let fixture = migration_fixture_with_one_git_repository();
        IdentityMigrationExecutor::new(fixture.paths.clone())
            .ensure_through_authority("project_0001")
            .expect("migrate through authority");

        // WP0 bootstrap 闭环:authority 写出后应存在 revision 1 的 bootstrap 政策
        // artifact,供首次真实 provider launch 校验。
        let policy_store = AggregatePolicyArtifactStore::new(fixture.paths.clone());
        let artifact = policy_store
            .get("project_0001")
            .expect("read bootstrap policy artifact")
            .expect("bootstrap policy artifact persisted by migration");
        assert_eq!(artifact.revision, 1);
        assert_eq!(artifact.project_id, "project_0001");

        let manifest = LogicalCodebaseStore::new(fixture.paths.clone())
            .load_manifest("project_0001")
            .expect("load manifest")
            .expect("manifest");
        assert_eq!(artifact.logical_codebase_id, manifest.logical_codebase_id.to_string());

        // 幂等:重跑 migration 不产生副作用,也不升级 revision。
        IdentityMigrationExecutor::new(fixture.paths.clone())
            .ensure_through_authority("project_0001")
            .expect("re-run migration is idempotent");
        let replayed = policy_store
            .get("project_0001")
            .expect("read bootstrap policy artifact after replay")
            .expect("bootstrap policy artifact persisted after replay");
        assert_eq!(replayed, artifact);
    }

    fn run_git_command(arguments: &[&str]) {
        let status = Command::new("git")
            .args(arguments)
            .status()
            .expect("start git");
        assert!(status.success(), "git {arguments:?}");
    }

    fn run_git_command_in(repository: &Path, arguments: &[&str]) {
        let status = Command::new("git")
            .current_dir(repository)
            .args(arguments)
            .status()
            .expect("start git");
        assert!(
            status.success(),
            "git -C {} {arguments:?}",
            repository.display()
        );
    }

    struct FailAfterAuthorityWrite {
        has_failed: AtomicBool,
    }

    impl FailAfterAuthorityWrite {
        fn new() -> Self {
            Self {
                has_failed: AtomicBool::new(false),
            }
        }
    }

    impl MigrationFaultInjector for FailAfterAuthorityWrite {
        fn after_authority_write(
            &self,
            _project_id: &str,
            _mapping: &RepositoryIdentityMapping,
        ) -> Result<(), ProductStoreError> {
            if !self.has_failed.swap(true, Ordering::SeqCst) {
                return Err(ProductStoreError::Io(
                    "injected crash after authority write".to_string(),
                ));
            }
            Ok(())
        }
    }
}
