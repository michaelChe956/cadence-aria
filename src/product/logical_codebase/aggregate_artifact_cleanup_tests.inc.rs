mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalCodebaseCreateInput,
        LogicalCodebaseManifest, LogicalCodebaseRestoreOutcome, LogicalRepositoryId,
        RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity,
    };
    use crate::product::project_store::ProjectStore;
    use uuid::Uuid;

    /// 一个已登记的聚合根 + 一个成员仓：工作区里同时存在 aria 产物
    /// （政策正文 + 指针）与 Cadence-skills 产物（AGENTS.md 等）。
    struct Fixture {
        _temp: tempfile::TempDir,
        paths: ProductAppPaths,
        project_id: String,
        lc_id: String,
        aggregate_root: PathBuf,
        policy_dir: PathBuf,
        pointer_file: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let paths = ProductAppPaths::new(temp.path());
            let project_id = "project_0001".to_string();
            let aggregate_root = temp.path().join("workspace/aggregate");
            let member_repo = temp.path().join("workspace/api");
            std::fs::create_dir_all(&aggregate_root).unwrap();
            std::fs::create_dir_all(&member_repo).unwrap();

            let record = LogicalCodebaseStore::new(paths.clone())
                .create(
                    &project_id,
                    LogicalCodebaseCreateInput {
                        name: "Platform".to_string(),
                        aggregate_root: aggregate_root.clone(),
                    },
                )
                .unwrap();
            let member_id = LogicalRepositoryId(Uuid::new_v4());
            let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
            let manifest = LogicalCodebaseManifest::new(
                &project_id,
                aggregate_root.clone(),
                vec![member_id],
            );
            let lc_store = LogicalCodebaseStore::for_lc(paths.clone(), &record.id);
            lc_store.save_manifest(&project_id, &manifest).unwrap();
            lc_store
                .save_member(&project_id, &member_fixture(member_id, checkout_id))
                .unwrap();
            lc_store
                .save_checkout(
                    &project_id,
                    &checkout_fixture(member_id, checkout_id, &member_repo),
                )
                .unwrap();

            // aria 产物：政策版本正文 + 成员仓指针。
            let policy_dir = aggregate_root
                .join("policy")
                .join(&project_id)
                .join(manifest.logical_codebase_id.to_string());
            std::fs::create_dir_all(&policy_dir).unwrap();
            std::fs::write(policy_dir.join("1"), "# policy body\n").unwrap();
            let pointer_file = member_repo.join(POINTER_FILE_NAME);
            std::fs::write(&pointer_file, "# pointer\n").unwrap();

            // Cadence-skills 产物：删除不得清理。
            std::fs::write(aggregate_root.join("AGENTS.md"), "# agents\n").unwrap();
            std::fs::write(aggregate_root.join("CLAUDE.md"), "# claude\n").unwrap();
            std::fs::create_dir_all(aggregate_root.join(".claude/rules")).unwrap();
            std::fs::write(
                aggregate_root.join(".claude/rules/language.md"),
                "# rule\n",
            )
            .unwrap();

            Self {
                _temp: temp,
                paths,
                project_id,
                lc_id: record.id,
                aggregate_root,
                policy_dir,
                pointer_file,
            }
        }

        fn store(&self) -> LogicalCodebaseStore {
            LogicalCodebaseStore::new(self.paths.clone())
        }

        /// 同根下另一个项目的政策目录：删除本项目不得波及它。
        fn write_other_project_policy(&self, other_project_id: &str) -> PathBuf {
            let dir = self
                .aggregate_root
                .join("policy")
                .join(other_project_id)
                .join(Uuid::new_v4().to_string());
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("1"), "# other policy\n").unwrap();
            dir
        }
    }

    fn member_fixture(
        logical_repository_id: LogicalRepositoryId,
        checkout_id: RepositoryCheckoutId,
    ) -> CodebaseMemberRecord {
        let now = "2026-10-29T00:00:00Z".to_string();
        CodebaseMemberRecord {
            logical_repository_id,
            physical_repository_id: "repository_0001".to_string(),
            alias: "api".to_string(),
            role: "service".to_string(),
            ordinal: 1,
            source_identity: RepositorySourceIdentity::from_git_parts(
                Path::new("/workspace/api"),
                PathBuf::from("/workspace/api/.git"),
                Some("ssh://git@example.test/acme/api.git".to_string()),
            ),
            repo_type: Default::default(),
            tech_stack: Vec::new(),
            owner: None,
            tags: Vec::new(),
            default_ref: None,
            checkout_ids: vec![checkout_id],
            status: Default::default(),
            created_at: now.clone(),
            updated_at: now,
        }
    }

    fn checkout_fixture(
        logical_repository_id: LogicalRepositoryId,
        checkout_id: RepositoryCheckoutId,
        canonical_path: &Path,
    ) -> RepositoryCheckoutRecord {
        let now = "2026-10-29T00:00:00Z".to_string();
        RepositoryCheckoutRecord {
            checkout_id,
            logical_repository_id,
            physical_repository_id: "repository_0001".to_string(),
            kind: CheckoutKind::Main,
            canonical_path: canonical_path.to_path_buf(),
            checkout_path_hash: "sha256:checkout".to_string(),
            git_dir_identity: "sha256:git-dir".to_string(),
            revision: Some("abc123".to_string()),
            availability: CheckoutAvailability::Available,
            observed_at: now.clone(),
            created_at: now.clone(),
            updated_at: now,
        }
    }

    #[test]
    fn project_delete_removes_policy_body_and_member_pointer_only() {
        // 删项目：政策正文目录与成员仓指针消失；Cadence-skills 产物必须保留。
        let fixture = Fixture::new();
        let other_project_policy = fixture.write_other_project_policy("project_0002");

        ProjectStore::new(fixture.paths.clone())
            .delete(&fixture.project_id)
            .unwrap();

        assert!(
            !fixture.policy_dir.exists(),
            "policy body directory must be removed"
        );
        assert!(
            !fixture.pointer_file.exists(),
            "member pointer file must be removed"
        );
        assert!(
            fixture.aggregate_root.join("AGENTS.md").exists(),
            "AGENTS.md belongs to the project; cleanup must not touch it"
        );
        assert!(
            fixture.aggregate_root.join("CLAUDE.md").exists(),
            "CLAUDE.md belongs to the project; cleanup must not touch it"
        );
        assert!(
            fixture
                .aggregate_root
                .join(".claude/rules/language.md")
                .exists(),
            ".claude/rules/* belongs to the project; cleanup must not touch it"
        );
        assert!(
            other_project_policy.exists(),
            "another project's policy directory must survive"
        );
    }

    #[test]
    fn project_delete_cleanup_is_idempotent_and_skips_missing_artifacts() {
        // 幂等：重复清理不报错；缺失即跳过。
        let fixture = Fixture::new();
        let store = ProjectStore::new(fixture.paths.clone());

        store.delete(&fixture.project_id).unwrap();
        assert!(!fixture.policy_dir.exists());
        assert!(!fixture.pointer_file.exists());

        // 第二次删除：项目已不存在 → NotFound（既有语义不变）。
        assert!(matches!(
            store.delete(&fixture.project_id),
            Err(crate::product::json_store::ProductStoreError::NotFound { .. })
        ));

        // 清理本身幂等：再跑一次不报错、不动作。
        let report = cleanup_project_artifacts(&fixture.paths, &fixture.project_id);
        assert!(report.is_noop(), "unexpected cleanup actions: {report:?}");
    }

    #[test]
    fn soft_delete_removes_member_pointer_but_keeps_policy_body_for_restore() {
        // 软删与 restore 的分工：政策正文是 restore 的前提（restore 后 gateway
        // 仍要读到已发布正文），软删不得删它；成员仓指针可由下次指针发布重新
        // 生成，不参与 restore 的任何不变量，软删即清。
        let fixture = Fixture::new();

        fixture
            .store()
            .delete_soft(&fixture.project_id, &fixture.lc_id)
            .unwrap();

        assert!(
            !fixture.pointer_file.exists(),
            "member pointer file must be removed on soft delete"
        );
        assert!(
            fixture.policy_dir.exists(),
            "policy body must survive a soft delete so restore keeps a published policy"
        );

        match fixture
            .store()
            .restore(&fixture.project_id, &fixture.lc_id)
            .unwrap()
        {
            LogicalCodebaseRestoreOutcome::Restored(_) => {}
            other => panic!("expected Restored, got {other:?}"),
        }
        assert!(
            fixture.policy_dir.exists(),
            "restored logical codebase must still own its published policy body"
        );
    }

    #[test]
    fn cleanup_skips_symlinked_policy_components_instead_of_escaping_the_root() {
        // 路径安全：中间组件是符号链接时跳过，绝不删到 canonical root 之外。
        let fixture = Fixture::new();
        let outside = fixture._temp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("keep.txt"), "keep\n").unwrap();
        std::fs::remove_dir_all(fixture.aggregate_root.join("policy")).unwrap();
        std::os::unix::fs::symlink(&outside, fixture.aggregate_root.join("policy")).unwrap();

        let report = cleanup_project_artifacts(&fixture.paths, &fixture.project_id);

        assert!(
            outside.join("keep.txt").exists(),
            "cleanup must not follow a symlinked policy component"
        );
        assert!(
            !report.removed_policy_dirs.is_empty() || !report.skipped.is_empty(),
            "cleanup must record what it did or skipped: {report:?}"
        );
        assert!(
            !fixture.pointer_file.exists(),
            "pointer cleanup is independent of the policy component shape"
        );
    }

    #[test]
    fn cleanup_of_tombstoned_logical_codebase_also_removes_its_artifacts() {
        // 墓碑 LC 的产物同样要清：项目删除后 app 数据全没，工作区残留必须收掉。
        let fixture = Fixture::new();
        fixture
            .store()
            .delete_soft(&fixture.project_id, &fixture.lc_id)
            .unwrap();

        cleanup_project_artifacts(&fixture.paths, &fixture.project_id);

        assert!(!fixture.policy_dir.exists());
        assert!(!fixture.pointer_file.exists());
    }
}
