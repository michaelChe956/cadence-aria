#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::logical_codebase::issue_selection::IssueCodebaseSelection;
    use crate::product::logical_codebase::store::LogicalCodebaseManifest;

    fn manifest_fixture() -> LogicalCodebaseManifest {
        LogicalCodebaseManifest::new(
            "project_0001",
            std::path::PathBuf::from("/tmp/logical-codebase"),
            Vec::new(),
        )
    }

    #[test]
    fn none_none_routes_to_legacy() {
        // (None, None) → Legacy；不读任何文件，纯判定
        let routing = RepositoryRouting::classify(None, None);
        assert!(matches!(routing, RepositoryRouting::Legacy { .. }));
    }

    #[test]
    fn some_some_routes_to_logical() {
        let selection = IssueCodebaseSelection::all_members("project_0001", "issue_0001", None);
        let routing = RepositoryRouting::classify(Some(manifest_fixture()), Some(selection));
        assert!(matches!(routing, RepositoryRouting::Logical { .. }));
    }

    #[test]
    fn some_none_is_fail_closed_with_stable_code() {
        // 有 manifest 无 selection → 不完整逻辑状态，fail-closed，稳定错误码 TargetMissing（B3）
        let routing = RepositoryRouting::classify(Some(manifest_fixture()), None);
        match routing {
            RepositoryRouting::FailClosed { code, .. } => {
                assert_eq!(code, RepositoryRoutingErrorCode::TargetMissing)
            }
            _ => panic!("(Some, None) must fail-closed"),
        }
    }

    #[test]
    fn none_some_is_fail_closed_with_stable_code() {
        // 无 manifest 有 selection → 孤立 selection/数据损坏，fail-closed，稳定错误码 OrphanedSelection
        let selection = IssueCodebaseSelection::all_members("project_0001", "issue_0001", None);
        let routing = RepositoryRouting::classify(None, Some(selection));
        match routing {
            RepositoryRouting::FailClosed { code, .. } => {
                assert_eq!(code, RepositoryRoutingErrorCode::OrphanedSelection)
            }
            _ => panic!("(None, Some) must fail-closed"),
        }
    }

    #[test]
    fn resolve_issue_logical_codebase_id_reads_persisted_attribution() {
        let temp = tempfile::tempdir().unwrap();
        let paths = crate::product::app_paths::ProductAppPaths::new(temp.path());
        let store = crate::product::issue_store::IssueStore::new(paths.clone());
        let issue = store
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: "project_0001".to_string(),
                repo_id: Some("repository_0001".to_string()),
                logical_codebase_id: Some("logical_codebase_0001".to_string()),
                title: "logical issue".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .unwrap();
        let lc_id = resolve_issue_logical_codebase_id(&paths, "project_0001", &issue.id).unwrap();
        assert_eq!(lc_id.as_deref(), Some("logical_codebase_0001"));
    }

    #[test]
    fn load_for_issue_resolves_manifest_and_selection_from_lc_subtree() {
        let temp = tempfile::tempdir().unwrap();
        let paths = crate::product::app_paths::ProductAppPaths::new(temp.path());
        let lc_id = "logical_codebase_0001";
        // 建 issue 归属 lc_id。
        let store = crate::product::issue_store::IssueStore::new(paths.clone());
        let issue = store
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: "project_0001".to_string(),
                repo_id: Some("repository_0001".to_string()),
                logical_codebase_id: Some(lc_id.to_string()),
                title: "logical issue".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .unwrap();
        // 在 lc 子树写 manifest + selection。
        let logical = LogicalCodebaseStore::for_lc(paths.clone(), lc_id);
        logical
            .save_manifest(
                "project_0001",
                &LogicalCodebaseManifest::new(
                    "project_0001",
                    std::path::PathBuf::from("/tmp/logical-codebase"),
                    Vec::new(),
                ),
            )
            .unwrap();
        IssueCodebaseSelectionStore::for_lc(paths.clone(), lc_id)
            .save(
                &IssueCodebaseSelection::all_members("project_0001", &issue.id, None)
                    .for_logical_codebase(lc_id),
            )
            .unwrap();

        let routing = RepositoryRouting::load_for_issue(&paths, "project_0001", &issue.id).unwrap();
        match routing {
            RepositoryRouting::Logical {
                manifest,
                selection,
            } => {
                assert_eq!(manifest.project_id, "project_0001");
                assert_eq!(selection.logical_codebase_id.as_deref(), Some(lc_id));
            }
            _ => panic!("must resolve Logical from lc subtree"),
        }
    }

    // ---- Task 1：唯一 authority resolver（显式 kind + fail-closed 冲突）----

    use std::collections::BTreeMap;

    use crate::product::logical_codebase::aggregate_index::{
        AggregateIndexRecord, AggregateIndexStatus, AggregateIndexStore,
    };
    use crate::product::logical_codebase::policy::{
        AggregatePolicyArtifact, AggregatePolicyArtifactStore,
    };
    use crate::product::logical_codebase::types::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, MemberStatus,
        RepositoryCheckoutRecord, RepositoryType,
    };
    use crate::product::logical_codebase::{LogicalRepositoryId, RepositoryCheckoutId};
    use crate::product::project_store::{CreateProjectInput, ProjectStore};

    fn git(cwd: &std::path::Path, arguments: &[&str]) {
        let output = std::process::Command::new("git")
            .current_dir(cwd)
            .args(arguments)
            .output()
            .expect("git command must start");
        assert!(
            output.status.success(),
            "git {arguments:?} failed in {}: {}",
            cwd.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_git_repository_with_commit(path: &std::path::Path) {
        std::fs::create_dir_all(path).unwrap();
        git(path, &["init", "-b", "main"]);
        git(path, &["config", "user.email", "resolver@test.local"]);
        git(path, &["config", "user.name", "Resolver Test"]);
        std::fs::write(path.join("README.md"), "# member\n").unwrap();
        git(path, &["add", "."]);
        git(path, &["commit", "-m", "init"]);
    }

    /// 收集 `.aria` durable inventory（相对路径 → 文件字节），用于断言 resolver 零写入。
    fn aria_inventory(root: &std::path::Path) -> BTreeMap<String, Vec<u8>> {
        let mut inventory = BTreeMap::new();
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

    fn create_project_fixture(paths: &crate::product::app_paths::ProductAppPaths) -> String {
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "resolver-project".to_string(),
                description: None,
            })
            .unwrap()
            .id
    }

    fn active_index_record(
        aggregate_index_id: &str,
        project_id: &str,
        membership_revision: u64,
    ) -> AggregateIndexRecord {
        let mut record = AggregateIndexRecord::building(
            aggregate_index_id.to_string(),
            project_id.to_string(),
            membership_revision,
            Vec::new(),
            "2026-09-28T00:00:00Z".to_string(),
        );
        record.status = AggregateIndexStatus::Active;
        record
    }

    #[test]
    fn resolver_reads_lc_manifest_selection_policy_and_index_only_from_requested_lc_subtree() {
        let temp = tempfile::tempdir().unwrap();
        let paths = crate::product::app_paths::ProductAppPaths::new(temp.path());
        let project_id = create_project_fixture(&paths);

        std::fs::create_dir_all(temp.path().join("alpha-root")).unwrap();
        std::fs::create_dir_all(temp.path().join("beta-root")).unwrap();
        let logical = LogicalCodebaseStore::new(paths.clone());
        let lc_a = logical
            .create(
                &project_id,
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "alpha".to_string(),
                    aggregate_root: temp.path().join("alpha-root"),
                },
            )
            .unwrap();
        let lc_b = logical
            .create(
                &project_id,
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "beta".to_string(),
                    aggregate_root: temp.path().join("beta-root"),
                },
            )
            .unwrap();

        let manifest_a =
            LogicalCodebaseManifest::new(&project_id, temp.path().join("alpha-root"), Vec::new());
        let manifest_b =
            LogicalCodebaseManifest::new(&project_id, temp.path().join("beta-root"), Vec::new());
        LogicalCodebaseStore::for_lc(paths.clone(), &lc_a.id)
            .save_manifest(&project_id, &manifest_a)
            .unwrap();
        LogicalCodebaseStore::for_lc(paths.clone(), &lc_b.id)
            .save_manifest(&project_id, &manifest_b)
            .unwrap();

        let issue_store = crate::product::issue_store::IssueStore::new(paths.clone());
        let issue_a = issue_store
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: project_id.clone(),
                repo_id: None,
                logical_codebase_id: Some(lc_a.id.clone()),
                title: "alpha issue".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .unwrap();
        IssueCodebaseSelectionStore::for_lc(paths.clone(), &lc_a.id)
            .save(
                &IssueCodebaseSelection::all_members(&project_id, &issue_a.id, None)
                    .for_logical_codebase(&lc_a.id),
            )
            .unwrap();

        let policy_a = AggregatePolicyArtifact::bootstrap(
            &project_id,
            &manifest_a.logical_codebase_id.to_string(),
            "2026-09-28T00:00:00Z".to_string(),
        );
        let policy_b = AggregatePolicyArtifact::bootstrap(
            &project_id,
            &manifest_b.logical_codebase_id.to_string(),
            "2026-09-28T00:00:00Z".to_string(),
        );
        AggregatePolicyArtifactStore::for_lc(paths.clone(), &lc_a.id)
            .save(&project_id, &policy_a)
            .unwrap();
        AggregatePolicyArtifactStore::for_lc(paths.clone(), &lc_b.id)
            .save(&project_id, &policy_b)
            .unwrap();

        AggregateIndexStore::for_lc(paths.clone(), &lc_a.id)
            .create(
                &project_id,
                active_index_record(
                    "aggregate_index_alpha",
                    &project_id,
                    manifest_a.membership_revision,
                ),
            )
            .unwrap();
        AggregateIndexStore::for_lc(paths.clone(), &lc_b.id)
            .create(
                &project_id,
                active_index_record(
                    "aggregate_index_beta",
                    &project_id,
                    manifest_b.membership_revision,
                ),
            )
            .unwrap();

        let resolver = RepositoryAuthorityResolver::new(paths.clone());
        let resolution = resolver
            .resolve(RepositoryRoutingRequest {
                project_id: project_id.clone(),
                issue_id: Some(issue_a.id.clone()),
                kind: RepositoryTargetKind::LogicalCodebase,
                repository_id: None,
                logical_codebase_id: Some(lc_a.id.clone()),
                logical_repository_id: None,
                checkout_id: None,
            })
            .unwrap();

        assert_eq!(
            resolution.authority_root,
            std::fs::canonicalize(temp.path().join("alpha-root")).unwrap()
        );
        assert_eq!(
            resolution.target.kind,
            RepositoryTargetKind::LogicalCodebase
        );
        assert_eq!(
            resolution.target.logical_codebase_id.as_deref(),
            Some(lc_a.id.as_str())
        );
        let manifest = resolution
            .manifest
            .expect("manifest from requested lc subtree");
        assert_eq!(
            manifest.provider_context_root,
            temp.path().join("alpha-root")
        );
        assert_eq!(manifest.logical_codebase_id, manifest_a.logical_codebase_id);
        let selection = resolution
            .selection
            .expect("selection from requested lc subtree");
        assert_eq!(
            selection.logical_codebase_id.as_deref(),
            Some(lc_a.id.as_str())
        );
        let policy = resolution.policy.expect("policy from requested lc subtree");
        assert_eq!(policy.policy_id, policy_a.policy_id);
        assert_eq!(policy.policy_digest, policy_a.digest);
        assert_eq!(policy.policy_revision, policy_a.revision);
        assert_eq!(
            resolution.aggregate_index.aggregate_index_id.as_deref(),
            Some("aggregate_index_alpha")
        );
        assert_eq!(
            resolution.aggregate_index.membership_revision,
            Some(manifest_a.membership_revision)
        );
        assert_eq!(
            resolution.aggregate_index.status,
            Some(AggregateIndexStatus::Active)
        );
    }

    // ---- C2 Task 10：受限政策读取（REQ-ENV-C2-POLICY，#17／BYPASS-17）----

    /// 最小 LC 政策 fixture：project + alpha/beta 两个 LC record + manifest +
    /// bootstrap policy artifact + issue 归属 alpha（成员/checkout 不参与
    /// policy 解析，policy 走 `resolve_logical` 的 None-member 分支）。
    #[allow(clippy::type_complexity)]
    fn policy_reader_fixture(
        paths: &crate::product::app_paths::ProductAppPaths,
        temp: &std::path::Path,
    ) -> (
        String,
        String,
        String,
        AggregatePolicyArtifact,
        std::path::PathBuf,
    ) {
        let project_id = create_project_fixture(paths);
        let alpha_root = temp.join("alpha-policy-root");
        let beta_root = temp.join("beta-policy-root");
        std::fs::create_dir_all(&alpha_root).unwrap();
        std::fs::create_dir_all(&beta_root).unwrap();
        let logical = LogicalCodebaseStore::new(paths.clone());
        let lc = logical
            .create(
                &project_id,
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "policy-alpha".to_string(),
                    aggregate_root: alpha_root.clone(),
                },
            )
            .unwrap();
        logical
            .create(
                &project_id,
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "policy-beta".to_string(),
                    aggregate_root: beta_root.clone(),
                },
            )
            .unwrap();
        let manifest = LogicalCodebaseManifest::new(&project_id, alpha_root, Vec::new());
        LogicalCodebaseStore::for_lc(paths.clone(), &lc.id)
            .save_manifest(&project_id, &manifest)
            .unwrap();
        let policy = AggregatePolicyArtifact::bootstrap(
            &project_id,
            &manifest.logical_codebase_id.to_string(),
            "2026-09-29T00:00:00Z".to_string(),
        );
        AggregatePolicyArtifactStore::for_lc(paths.clone(), &lc.id)
            .save(&project_id, &policy)
            .unwrap();
        let issue = crate::product::issue_store::IssueStore::new(paths.clone())
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: project_id.clone(),
                repo_id: None,
                logical_codebase_id: Some(lc.id.clone()),
                title: "policy read issue".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .unwrap();
        (project_id, issue.id, lc.id, policy, beta_root)
    }

    #[test]
    fn policy_reader_returns_same_digest_text_for_frozen_reference() {
        let temp = tempfile::tempdir().unwrap();
        let paths = crate::product::app_paths::ProductAppPaths::new(temp.path());
        let (project_id, issue_id, lc_id, policy, beta_root) =
            policy_reader_fixture(&paths, temp.path());

        let frozen = RepositoryAuthorityResolver::new(paths.clone())
            .resolve_for_issue(&project_id, &issue_id)
            .unwrap()
            .expect("lc resolution")
            .policy
            .expect("policy reference");

        let result = read_policy_text_for_reference(&paths, &frozen).expect("same digest text");
        assert_eq!(result.policy_id, policy.policy_id);
        assert_eq!(result.policy_revision, policy.revision);
        assert_eq!(result.policy_digest, policy.digest);
        assert_eq!(result.text, policy.policy_text);
        // 正文 digest 必须是返回正文的 canonical SHA-256（非自报）。
        let recomputed = format!("sha256:{:x}", sha2::Sha256::digest(result.text.as_bytes()));
        assert_eq!(recomputed, result.policy_digest);

        // authority root 与引用不符（引用被串改到 beta 根）→ IdentityMismatch
        // fail-closed，不按串改 root 猜测。
        let tampered = AuthorityPolicyReference {
            artifact_root: beta_root,
            ..frozen.clone()
        };
        let mismatched = read_policy_text_for_reference(&paths, &tampered).unwrap_err();
        assert!(matches!(
            mismatched,
            PolicyReadError::IdentityMismatch { .. }
        ));

        // 引用 digest 与 artifact 正文 digest 不一致（引用被串改 digest）→
        // DigestMismatch fail-closed，不得返回正文。
        let digest_tampered = AuthorityPolicyReference {
            policy_digest: "sha256:tampered-digest".to_string(),
            ..frozen.clone()
        };
        let digest_rejected = read_policy_text_for_reference(&paths, &digest_tampered).unwrap_err();
        assert!(matches!(
            digest_rejected,
            PolicyReadError::DigestMismatch { .. }
        ));

        // 政策升级（revision 2 覆盖保存）后，旧冻结引用（revision 1）不再有
        // 权威子树持有 → Unavailable fail-closed，不得静默返回新正文。
        let revised =
            policy.with_revised_policy("升级后的政策正文", "2026-09-29T01:00:00Z".to_string());
        AggregatePolicyArtifactStore::for_lc(paths.clone(), &lc_id)
            .save(&project_id, &revised)
            .unwrap();
        let stale = read_policy_text_for_reference(&paths, &frozen).unwrap_err();
        assert!(matches!(stale, PolicyReadError::Unavailable { .. }));
    }

    #[test]
    fn policy_reader_fail_closes_on_missing_artifact_or_malformed_reference() {
        let temp = tempfile::tempdir().unwrap();
        let paths = crate::product::app_paths::ProductAppPaths::new(temp.path());
        let (project_id, _issue_id, _lc_id, _policy, _beta_root) =
            policy_reader_fixture(&paths, temp.path());

        // 引用指向不存在 artifact 的 LC → Unavailable，无路径猜测。
        let foreign = AuthorityPolicyReference {
            policy_id: format!("policy/{project_id}/logical_codebase_missing/1"),
            policy_revision: 1,
            policy_digest: "sha256:deadbeef".to_string(),
            artifact_root: temp.path().join("alpha-policy-root"),
        };
        assert!(matches!(
            read_policy_text_for_reference(&paths, &foreign),
            Err(PolicyReadError::Unavailable { .. })
        ));

        // policy_id 结构不可解析 → Unavailable。
        let malformed = AuthorityPolicyReference {
            policy_id: "not-a-policy-id".to_string(),
            ..foreign
        };
        assert!(matches!(
            read_policy_text_for_reference(&paths, &malformed),
            Err(PolicyReadError::Unavailable { .. })
        ));
    }

    #[test]
    fn resolver_rejects_kind_mismatch_duplicate_source_and_legacy_conflict_without_writes() {
        let temp = tempfile::tempdir().unwrap();
        let paths = crate::product::app_paths::ProductAppPaths::new(temp.path());
        let project_id = create_project_fixture(&paths);

        let workspace = temp.path().join("workspace");
        let repo_a = workspace.join("repo-a");
        init_git_repository_with_commit(&repo_a);
        let canonical = std::fs::canonicalize(&repo_a).unwrap();
        let source =
            crate::product::repository_store::resolve_repository_source(&canonical).unwrap();

        let logical = LogicalCodebaseStore::new(paths.clone());
        let lc = logical
            .create(
                &project_id,
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "gamma".to_string(),
                    aggregate_root: workspace.clone(),
                },
            )
            .unwrap();
        let lc_store = LogicalCodebaseStore::for_lc(paths.clone(), &lc.id);

        let member_id = LogicalRepositoryId(uuid::Uuid::new_v4());
        let duplicate_member_id = LogicalRepositoryId(uuid::Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
        let mut manifest = LogicalCodebaseManifest::new(&project_id, workspace.clone(), Vec::new());
        manifest.member_ids = vec![member_id];
        lc_store.save_manifest(&project_id, &manifest).unwrap();

        let now = "2026-09-28T00:00:00Z".to_string();
        let member = CodebaseMemberRecord {
            logical_repository_id: member_id,
            physical_repository_id: "repository_gamma_member".to_string(),
            alias: "repo-a".to_string(),
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
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        lc_store.save_member(&project_id, &member).unwrap();
        lc_store
            .save_checkout(
                &project_id,
                &RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: member_id,
                    physical_repository_id: member.physical_repository_id.clone(),
                    kind: CheckoutKind::Main,
                    canonical_path: canonical.clone(),
                    checkout_path_hash: crate::product::id::repo_hash_for_path(
                        canonical.to_string_lossy().as_ref(),
                    ),
                    git_dir_identity: source.git_dir_identity(),
                    revision: None,
                    availability: CheckoutAvailability::Available,
                    observed_at: now.clone(),
                    created_at: now.clone(),
                    updated_at: now.clone(),
                },
            )
            .unwrap();

        let issue = crate::product::issue_store::IssueStore::new(paths.clone())
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: project_id.clone(),
                repo_id: None,
                logical_codebase_id: Some(lc.id.clone()),
                title: "gamma issue".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .unwrap();

        let resolver = RepositoryAuthorityResolver::new(paths.clone());

        // (a) single-repo 身份访问 LC 绑定 issue → kind mismatch，零写入。
        let before = aria_inventory(temp.path());
        let error = resolver
            .resolve(RepositoryRoutingRequest {
                project_id: project_id.clone(),
                issue_id: Some(issue.id.clone()),
                kind: RepositoryTargetKind::SingleRepo,
                repository_id: Some("repository_single".to_string()),
                logical_codebase_id: None,
                logical_repository_id: None,
                checkout_id: None,
            })
            .unwrap_err();
        assert!(
            matches!(
                &error,
                crate::product::json_store::ProductStoreError::Conflict { kind, id }
                    if *kind == "repository_routing_kind_mismatch" && id.contains(&issue.id)
            ),
            "single-repo request for a logical issue must fail closed with kind mismatch, got: {error:?}"
        );
        assert_eq!(aria_inventory(temp.path()), before);

        // (b) 同一 git-dir 两个别名成员 → source identity mismatch，零写入。
        let duplicate_member = CodebaseMemberRecord {
            logical_repository_id: duplicate_member_id,
            physical_repository_id: "repository_gamma_duplicate".to_string(),
            alias: "repo-a-alias".to_string(),
            role: "member".to_string(),
            ordinal: 2,
            source_identity: source.clone(),
            repo_type: RepositoryType::Unknown,
            tech_stack: Vec::new(),
            owner: None,
            tags: Vec::new(),
            default_ref: None,
            checkout_ids: vec![RepositoryCheckoutId(uuid::Uuid::new_v4())],
            status: MemberStatus::Active,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        lc_store
            .save_member(&project_id, &duplicate_member)
            .unwrap();
        let before = aria_inventory(temp.path());
        let error = resolver
            .resolve(RepositoryRoutingRequest {
                project_id: project_id.clone(),
                issue_id: Some(issue.id.clone()),
                kind: RepositoryTargetKind::LogicalCodebase,
                repository_id: None,
                logical_codebase_id: Some(lc.id.clone()),
                logical_repository_id: None,
                checkout_id: None,
            })
            .unwrap_err();
        assert!(
            matches!(
                &error,
                crate::product::json_store::ProductStoreError::Conflict { kind, id }
                    if *kind == "repository_routing_source_identity_mismatch"
                        && id.contains(&source.key_digest)
            ),
            "duplicate member source identity must fail closed, got: {error:?}"
        );
        assert_eq!(aria_inventory(temp.path()), before);
        lc_store
            .save_member(&project_id, &{
                // 移除重复成员，恢复唯一成员现场供 (c) 使用。
                let mut restored = duplicate_member;
                restored.status = MemberStatus::Removed;
                restored
            })
            .unwrap();

        // (c) 旧 project-level 布局与新 LC 子树来源身份冲突 → legacy conflict，零写入。
        let legacy_root = paths.logical_codebase_root(&project_id);
        let legacy_store = LogicalCodebaseStore::for_lc(
            paths.clone(),
            crate::product::logical_codebase::store::legacy_logical_codebase_id(&project_id),
        );
        let mut legacy_manifest =
            LogicalCodebaseManifest::new(&project_id, workspace.clone(), Vec::new());
        let legacy_member_id = LogicalRepositoryId(uuid::Uuid::new_v4());
        legacy_manifest.member_ids = vec![legacy_member_id];
        legacy_store
            .save_manifest(&project_id, &legacy_manifest)
            .unwrap();
        legacy_store
            .save_member(
                &project_id,
                &CodebaseMemberRecord {
                    logical_repository_id: legacy_member_id,
                    physical_repository_id: "repository_legacy_member".to_string(),
                    alias: "repo-a-legacy".to_string(),
                    role: "member".to_string(),
                    ordinal: 1,
                    source_identity: source.clone(),
                    repo_type: RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![RepositoryCheckoutId(uuid::Uuid::new_v4())],
                    status: MemberStatus::Active,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                },
            )
            .unwrap();
        let _ = legacy_root;

        let before = aria_inventory(temp.path());
        let error = resolver
            .resolve(RepositoryRoutingRequest {
                project_id: project_id.clone(),
                issue_id: Some(issue.id.clone()),
                kind: RepositoryTargetKind::LogicalCodebase,
                repository_id: None,
                logical_codebase_id: Some(lc.id.clone()),
                logical_repository_id: Some(member_id),
                checkout_id: None,
            })
            .unwrap_err();
        assert!(
            matches!(
                &error,
                crate::product::json_store::ProductStoreError::Conflict { kind, id }
                    if *kind == "repository_routing_legacy_conflict"
                        && id.contains(&source.key_digest)
            ),
            "legacy project-level layout conflicting with the requested lc must fail closed, got: {error:?}"
        );
        assert_eq!(aria_inventory(temp.path()), before);

        // 成员仓零 Git 写副作用：HEAD/dirty 不变。
        let dirty = std::process::Command::new("git")
            .current_dir(&repo_a)
            .args(["status", "--porcelain"])
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&dirty.stdout).trim().is_empty(),
            "member repository must stay clean"
        );
    }
}
