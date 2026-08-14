#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::product::logical_codebase::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalCodebaseManifest,
        LogicalRepositoryId, MemberStatus, RepositoryCheckoutId, RepositoryCheckoutRecord,
        RepositorySourceIdentity, RepositoryType,
    };
    use std::path::{Path, PathBuf};
    use std::process::Command as StdCommand;
    use tempfile::TempDir;

    const PROJECT_ID: &str = "project_0001";

    fn git(repo: &Path, args: &[&str]) {
        let output = StdCommand::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("run git fixture command");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_allow_failure(repo: &Path, args: &[&str]) -> (bool, String) {
        let output = StdCommand::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("run git fixture command");
        (
            output.status.success(),
            String::from_utf8_lossy(&output.stdout).to_string(),
        )
    }

    struct MemberRepo {
        logical_id: LogicalRepositoryId,
        checkout_id: RepositoryCheckoutId,
        repo_path: PathBuf,
        bare_remote: Option<PathBuf>,
    }

    impl MemberRepo {
        fn member_record(&self) -> CodebaseMemberRecord {
            let now = "2026-08-14T00:00:00Z".to_string();
            CodebaseMemberRecord {
                logical_repository_id: self.logical_id,
                physical_repository_id: format!("repo_{}", self.logical_id.0),
                alias: format!("member_{}", self.logical_id.0.simple()),
                role: "service".to_string(),
                ordinal: 1,
                source_identity: RepositorySourceIdentity::from_git_parts(
                    &self.repo_path,
                    self.repo_path.join(".git"),
                    Some(format!(
                        "ssh://git@example.test/acme/{}.git",
                        self.logical_id.0
                    )),
                ),
                repo_type: RepositoryType::Unknown,
                tech_stack: Vec::new(),
                owner: None,
                tags: Vec::new(),
                default_ref: None,
                checkout_ids: vec![self.checkout_id],
                status: MemberStatus::Active,
                created_at: now.clone(),
                updated_at: now,
            }
        }

        fn checkout_record(&self) -> RepositoryCheckoutRecord {
            let now = "2026-08-14T00:00:00Z".to_string();
            RepositoryCheckoutRecord {
                checkout_id: self.checkout_id,
                logical_repository_id: self.logical_id,
                physical_repository_id: format!("repo_{}", self.logical_id.0),
                kind: CheckoutKind::Main,
                canonical_path: self.repo_path.clone(),
                checkout_path_hash: format!("sha256:{}", self.logical_id.0),
                git_dir_identity: format!("sha256:git-{}", self.logical_id.0),
                revision: None,
                availability: CheckoutAvailability::Available,
                observed_at: now.clone(),
                created_at: now.clone(),
                updated_at: now,
            }
        }
    }

    struct Fixture {
        tmp: TempDir,
        coordinator: PointerPublishCoordinator,
        logical_codebase_id: String,
        members: Vec<MemberRepo>,
    }

    fn setup_member(tmp: &Path, name: &str, with_origin: bool) -> MemberRepo {
        let logical_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        let repo_path = tmp.join(name);
        std::fs::create_dir_all(&repo_path).unwrap();
        git(&repo_path, &["init"]);
        git(&repo_path, &["config", "user.email", "test@example.com"]);
        git(&repo_path, &["config", "user.name", "Test User"]);
        std::fs::write(repo_path.join("README.md"), "base\n").unwrap();
        git(&repo_path, &["add", "README.md"]);
        git(&repo_path, &["commit", "-m", "base"]);

        let bare_remote = if with_origin {
            let remote_path = tmp.join(format!("{name}-origin.git"));
            std::fs::create_dir_all(&remote_path).unwrap();
            git(&remote_path, &["init", "--bare"]);
            git(
                &repo_path,
                &["remote", "add", "origin", remote_path.to_str().unwrap()],
            );
            git(&repo_path, &["push", "-u", "origin", "master"]);
            git(&repo_path, &["branch", "-m", "main"]);
            git(&repo_path, &["push", "-u", "origin", "main"]);
            Some(remote_path)
        } else {
            None
        };

        MemberRepo {
            logical_id,
            checkout_id,
            repo_path,
            bare_remote,
        }
    }

    fn setup(member_specs: &[(&str, bool)]) -> Fixture {
        let tmp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        let members: Vec<MemberRepo> = member_specs
            .iter()
            .map(|(name, with_origin)| setup_member(tmp.path(), name, *with_origin))
            .collect();

        let aggregate_root = tmp.path().join("aggregate-root");
        std::fs::create_dir_all(&aggregate_root).unwrap();
        let manifest = LogicalCodebaseManifest::new(
            PROJECT_ID,
            aggregate_root,
            members.iter().map(|m| m.logical_id).collect(),
        );
        let logical_codebase_id = manifest.logical_codebase_id.to_string();
        let store = LogicalCodebaseStore::new(paths.clone());
        store.save_manifest(PROJECT_ID, &manifest).unwrap();
        for member in &members {
            store
                .save_member(PROJECT_ID, &member.member_record())
                .unwrap();
            store
                .save_checkout(PROJECT_ID, &member.checkout_record())
                .unwrap();
        }

        Fixture {
            tmp,
            coordinator: PointerPublishCoordinator::new(paths),
            logical_codebase_id,
            members,
        }
    }

    fn remote_has_branch(bare: &Path, branch: &str) -> bool {
        let (success, stdout) = git_allow_failure(
            bare,
            &["show-ref", "--verify", &format!("refs/heads/{branch}")],
        );
        success && !stdout.trim().is_empty()
    }

    fn entry<'a>(
        publication: &'a PointerPublication,
        member_repo_id: &str,
    ) -> &'a PointerPublicationEntry {
        publication
            .entries
            .iter()
            .find(|entry| entry.member_repo_id == member_repo_id)
            .expect("entry")
    }

    #[tokio::test]
    async fn publish_all_full_batch_pushes_all_members_and_writes_review_requests() {
        let fixture = setup(&[("api", true), ("worker", true)]);
        let publication = fixture
            .coordinator
            .publish_all(
                PROJECT_ID,
                &fixture.logical_codebase_id,
                PointerPublicationBatchKind::Full,
            )
            .await
            .expect("publish");

        assert_eq!(publication.status, PointerPublicationStatus::CompletedAll);
        assert_eq!(publication.entries.len(), 2);
        for member in &fixture.members {
            let member_repo_id = member.logical_id.0.to_string();
            let entry = entry(&publication, &member_repo_id);
            assert_eq!(entry.state, PointerPublicationEntryState::ReviewCreated);
            assert!(entry.branch_name.is_some());
            assert!(entry.commit_sha.is_some());

            let bare = member.bare_remote.as_ref().expect("bare");
            let branch = entry.branch_name.as_deref().unwrap();
            assert!(
                remote_has_branch(bare, branch),
                "remote branch {branch} must exist"
            );

            // 指针文件在主 checkout 未被污染（写入发生在临时 worktree）
            assert!(!member.repo_path.join(POINTER_FILE_NAME).exists());

            // ReviewRequest 落盘在 pointer-publications 分区
            let requests = fixture
                .coordinator
                .git_ops
                .list_pointer_review_requests(PROJECT_ID, &publication.id)
                .unwrap();
            assert_eq!(requests.len(), 2);
            let request = requests
                .iter()
                .find(|request| request.id == format!("rr-{}-{}", publication.id, member_repo_id))
                .expect("review request");
            assert_eq!(
                request.owner_kind,
                ReviewRequestOwnerKind::PointerPublication
            );
            assert_eq!(
                request.attempt_id,
                format!("pointer-pub-{}", publication.id)
            );
            assert!(!request.revoked);
        }
    }

    #[tokio::test]
    async fn publish_all_single_member_push_failure_yields_completed_partial() {
        let fixture = setup(&[("no-remote", false), ("with-remote", true)]);
        let publication = fixture
            .coordinator
            .publish_all(
                PROJECT_ID,
                &fixture.logical_codebase_id,
                PointerPublicationBatchKind::Full,
            )
            .await
            .expect("publish");

        assert_eq!(
            publication.status,
            PointerPublicationStatus::CompletedPartial
        );
        let failed_id = fixture.members[0].logical_id.0.to_string();
        let failed = entry(&publication, &failed_id);
        assert_eq!(failed.state, PointerPublicationEntryState::Failed);
        assert!(failed.push_error.is_some());

        let ok_id = fixture.members[1].logical_id.0.to_string();
        assert_eq!(
            entry(&publication, &ok_id).state,
            PointerPublicationEntryState::ReviewCreated
        );
    }

    #[tokio::test]
    async fn incremental_publish_only_creates_entries_for_new_members() {
        let fixture = setup(&[("api", true)]);
        fixture
            .coordinator
            .publish_all(
                PROJECT_ID,
                &fixture.logical_codebase_id,
                PointerPublicationBatchKind::Full,
            )
            .await
            .expect("full publish");

        // 新增第二个成员
        let new_member = setup_member(fixture.tmp.path(), "worker", true);
        let store =
            LogicalCodebaseStore::new(ProductAppPaths::new(fixture.tmp.path().join(".aria")));
        store
            .save_member(PROJECT_ID, &new_member.member_record())
            .unwrap();
        store
            .save_checkout(PROJECT_ID, &new_member.checkout_record())
            .unwrap();
        let mut manifest = store.load_manifest(PROJECT_ID).unwrap().unwrap();
        manifest.member_ids.push(new_member.logical_id);
        manifest.membership_revision += 1;
        store.save_manifest(PROJECT_ID, &manifest).unwrap();

        let publication = fixture
            .coordinator
            .publish_all(
                PROJECT_ID,
                &fixture.logical_codebase_id,
                PointerPublicationBatchKind::Incremental,
            )
            .await
            .expect("incremental publish");

        assert_eq!(publication.status, PointerPublicationStatus::CompletedAll);
        assert_eq!(publication.entries.len(), 1);
        let new_id = new_member.logical_id.0.to_string();
        assert_eq!(
            entry(&publication, &new_id).state,
            PointerPublicationEntryState::ReviewCreated
        );
        assert!(remote_has_branch(
            new_member.bare_remote.as_ref().unwrap(),
            entry(&publication, &new_id).branch_name.as_deref().unwrap()
        ));
    }

    #[tokio::test]
    async fn conflict_entry_then_retry_unresolved_blocks_until_fixed() {
        let fixture = setup(&[("api", true)]);
        // 预置冲突指针块（不同 logical_codebase_id）
        std::fs::write(
            fixture.members[0].repo_path.join(POINTER_FILE_NAME),
            "<!-- aria-logical-codebase-pointer:start\n  logical_codebase_id: other\n  repo_id: other\n  canonical_policy_locator: /other\n  声明：未加载集中政策前禁止写；本块仅用于发现，不作为政策正文\n  pointer_version: 1\naria-logical-codebase-pointer:end -->\n",
        )
        .unwrap();

        let publication = fixture
            .coordinator
            .publish_all(
                PROJECT_ID,
                &fixture.logical_codebase_id,
                PointerPublicationBatchKind::Full,
            )
            .await
            .expect("publish");
        assert_eq!(
            publication.status,
            PointerPublicationStatus::CompletedPartial
        );
        let member_repo_id = fixture.members[0].logical_id.0.to_string();
        assert_eq!(
            entry(&publication, &member_repo_id).state,
            PointerPublicationEntryState::Conflict
        );

        // 冲突未解决 → 409
        let error = fixture
            .coordinator
            .retry_member_repo(PROJECT_ID, &publication.id, &member_repo_id)
            .await
            .expect_err("conflict must block retry");
        assert!(matches!(error, PointerPublishError::ConflictUnresolved(_)));

        // 人工解决：删除冲突块
        std::fs::remove_file(fixture.members[0].repo_path.join(POINTER_FILE_NAME)).unwrap();
        let retried = fixture
            .coordinator
            .retry_member_repo(PROJECT_ID, &publication.id, &member_repo_id)
            .await
            .expect("retry after resolve");
        assert_eq!(retried.status, PointerPublicationStatus::CompletedAll);
        assert_eq!(
            entry(&retried, &member_repo_id).state,
            PointerPublicationEntryState::ReviewCreated
        );
    }

    #[tokio::test]
    async fn revoke_deletes_remote_branches_marks_requests_and_is_idempotent() {
        let fixture = setup(&[("api", true), ("worker", true)]);
        let publication = fixture
            .coordinator
            .publish_all(
                PROJECT_ID,
                &fixture.logical_codebase_id,
                PointerPublicationBatchKind::Full,
            )
            .await
            .expect("publish");

        let revoked = fixture
            .coordinator
            .revoke(PROJECT_ID, &publication.id)
            .await
            .expect("revoke");
        assert_eq!(revoked.status, PointerPublicationStatus::Revoked);
        for member in &fixture.members {
            let member_repo_id = member.logical_id.0.to_string();
            let entry = entry(&revoked, &member_repo_id);
            assert_eq!(entry.state, PointerPublicationEntryState::Revoked);
            let branch = entry.branch_name.as_deref().unwrap();
            assert!(
                !remote_has_branch(member.bare_remote.as_ref().unwrap(), branch),
                "remote branch {branch} must be deleted"
            );
        }

        let requests = fixture
            .coordinator
            .git_ops
            .list_pointer_review_requests(PROJECT_ID, &publication.id)
            .unwrap();
        assert!(requests.iter().all(|request| request.revoked));

        // 重复 revoke 幂等
        let again = fixture
            .coordinator
            .revoke(PROJECT_ID, &publication.id)
            .await
            .expect("repeat revoke");
        assert_eq!(again.status, PointerPublicationStatus::Revoked);
    }

    #[tokio::test]
    async fn revoke_delete_failure_returns_revoke_failed_and_keeps_entries() {
        let fixture = setup(&[("api", true)]);
        let publication = fixture
            .coordinator
            .publish_all(
                PROJECT_ID,
                &fixture.logical_codebase_id,
                PointerPublicationBatchKind::Full,
            )
            .await
            .expect("publish");

        // 移除 origin，使删除远端分支失败（origin 不存在 ≠ 远端 ref 不存在）
        git(
            &fixture.members[0].repo_path,
            &["remote", "remove", "origin"],
        );

        let error = fixture
            .coordinator
            .revoke(PROJECT_ID, &publication.id)
            .await
            .expect_err("revoke must fail");
        assert!(matches!(error, PointerPublishError::RevokeFailed(_)));

        let after = fixture
            .coordinator
            .publications
            .load_publication(PROJECT_ID, &publication.id)
            .unwrap();
        assert_eq!(after.status, PointerPublicationStatus::CompletedAll);
        let member_repo_id = fixture.members[0].logical_id.0.to_string();
        assert_eq!(
            entry(&after, &member_repo_id).state,
            PointerPublicationEntryState::ReviewCreated
        );
    }
}
