use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::logical_codebase::{LogicalCodebaseStore, RepositoryRoutingErrorCode};
use crate::product::project_store::ProjectStore;
use crate::product::repository_store::RepositoryStore;

/// revision 是观察性字段：生产路径（repository registration 与 identity 迁移）从不持久化
/// `RepositoryCheckoutRecord.revision`（恒为 `None`），而 `AttemptTargetSnapshot.revision` 是
/// admission 时 `git rev-parse HEAD` 的冻结值。仅当 checkout 侧 revision 已观测（`Some`）时才
/// 逐字比对，`None` 时跳过 revision 比对；其余身份字段（logical/checkout id、canonical path、
/// git_dir_identity、membership_revision 等）仍严格比对，不变。
fn revision_matches(checkout_revision: Option<&str>, snapshot_revision: Option<&str>) -> bool {
    match checkout_revision {
        Some(observed) => Some(observed) == snapshot_revision,
        None => true,
    }
}

/// 校验 attempt 快照仍与当前逻辑代码库权威记录完全一致。
///
/// 快照存在时不得降级到物理仓库：无法验证的任何字段都统一视为不一致。
/// v1.3：`lc_id = Some` 时按 `logical-codebases/{lc_id}/` 子树权威记录校验
/// （R9 编码/交付链切换点）；`None`（单仓/旧数据）保持 legacy project 级路径。
pub fn validate_snapshot_fields(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    lc_id: Option<&str>,
) -> Result<(), RepositoryRoutingErrorCode> {
    let Some(snapshot) = attempt.target_snapshot.as_ref() else {
        return Err(RepositoryRoutingErrorCode::Inconsistent);
    };
    let authority = match lc_id {
        Some(lc_id) => LogicalCodebaseStore::for_lc(paths.clone(), lc_id),
        None => LogicalCodebaseStore::new(paths.clone()),
    };
    let manifest = authority
        .load_manifest(&attempt.project_id)
        .map_err(|_| RepositoryRoutingErrorCode::Inconsistent)?
        .ok_or(RepositoryRoutingErrorCode::Inconsistent)?;
    let member = authority
        .load_member(&attempt.project_id, snapshot.logical_repository_id)
        .map_err(|_| RepositoryRoutingErrorCode::Inconsistent)?
        .ok_or(RepositoryRoutingErrorCode::Inconsistent)?;
    let checkout = authority
        .load_checkout(&attempt.project_id, snapshot.checkout_id)
        .map_err(|_| RepositoryRoutingErrorCode::Inconsistent)?
        .ok_or(RepositoryRoutingErrorCode::Inconsistent)?;

    if !manifest
        .member_ids
        .contains(&snapshot.logical_repository_id)
        || member.logical_repository_id != snapshot.logical_repository_id
        || member.physical_repository_id != snapshot.physical_repository_id
        || !member.checkout_ids.contains(&snapshot.checkout_id)
        || checkout.checkout_id != snapshot.checkout_id
        || checkout.logical_repository_id != snapshot.logical_repository_id
        || checkout.physical_repository_id != snapshot.physical_repository_id
        || checkout.canonical_path != snapshot.canonical_path
        || checkout.git_dir_identity != snapshot.git_dir_identity
        || !revision_matches(checkout.revision.as_deref(), snapshot.revision.as_deref())
        || manifest.membership_revision != snapshot.membership_revision
    {
        return Err(RepositoryRoutingErrorCode::Inconsistent);
    }

    let (resolved_member, resolved_checkout, resolved_repository) = match lc_id {
        Some(lc_id) => RepositoryStore::new(paths.clone())
            .resolve_logical_repository_for_issue_codebase(
                &attempt.project_id,
                Some(lc_id),
                snapshot.logical_repository_id,
            )
            .map_err(|_| RepositoryRoutingErrorCode::Inconsistent)?,
        None => {
            let project = ProjectStore::new(paths.clone())
                .get(&attempt.project_id)
                .map_err(|_| RepositoryRoutingErrorCode::Inconsistent)?;
            RepositoryStore::for_project(paths.clone(), &project)
                .resolve_logical_repository_for_issue_codebase(
                    &attempt.project_id,
                    None,
                    snapshot.logical_repository_id,
                )
                .map_err(|_| RepositoryRoutingErrorCode::Inconsistent)?
        }
    };
    if resolved_member.logical_repository_id != snapshot.logical_repository_id
        || resolved_member.physical_repository_id != snapshot.physical_repository_id
        || resolved_checkout.checkout_id != snapshot.checkout_id
        || resolved_checkout.physical_repository_id != snapshot.physical_repository_id
        || resolved_checkout.canonical_path != snapshot.canonical_path
        || resolved_checkout.git_dir_identity != snapshot.git_dir_identity
        || !revision_matches(
            resolved_checkout.revision.as_deref(),
            snapshot.revision.as_deref(),
        )
        || resolved_repository.id != snapshot.physical_repository_id
    {
        return Err(RepositoryRoutingErrorCode::Inconsistent);
    }

    Ok(())
}

#[cfg(test)]
mod tests {

    use uuid::Uuid;

    use super::*;
    use crate::product::coding_models::{
        AttemptTargetSnapshot, CodingAttemptScope, CodingAttemptStatus, CodingExecutionStage,
    };
    use crate::product::json_store::{read_json, write_json};
    use crate::product::logical_codebase::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, IssueCodebaseSelection,
        IssueCodebaseSelectionStore, LogicalCodebaseManifest, LogicalRepositoryId, MemberStatus,
        RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
    };
    use crate::product::models::{ProviderName, RepositoryRecord};
    use crate::product::project_store::{CreateProjectInput, ProjectStore};
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    struct SnapshotFixture {
        paths: ProductAppPaths,
        attempt: CodingExecutionAttempt,
    }

    #[test]
    fn snapshot_validator_rejects_primary_checkout_repointed_to_another_valid_checkout() {
        // strict resolver must return its primary checkout and compare it with the frozen one;
        // merely resolving the logical repository successfully is not sufficient.
        let temp = tempfile::tempdir().unwrap();
        let mut fixture = snapshot_with_stale_membership_revision(temp.path());
        fixture
            .attempt
            .target_snapshot
            .as_mut()
            .unwrap()
            .membership_revision = 2;
        let authority = LogicalCodebaseStore::new(fixture.paths.clone());
        let snapshot = fixture.attempt.target_snapshot.as_ref().unwrap();
        let replacement_checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        let original_checkout = authority
            .load_checkout("project_0001", snapshot.checkout_id)
            .unwrap()
            .unwrap();
        let mut replacement_checkout = original_checkout.clone();
        replacement_checkout.checkout_id = replacement_checkout_id;
        replacement_checkout.canonical_path = original_checkout
            .canonical_path
            .with_file_name("repository_0002");
        authority
            .save_checkout("project_0001", &replacement_checkout)
            .unwrap();
        let mut member = authority
            .load_member("project_0001", snapshot.logical_repository_id)
            .unwrap()
            .unwrap();
        member.checkout_ids.push(replacement_checkout_id);
        authority.save_member("project_0001", &member).unwrap();
        let repos_path = fixture
            .paths
            .project_root("project_0001")
            .join("repos.json");
        let mut repositories: Vec<RepositoryRecord> = read_json(&repos_path).unwrap();
        repositories[0].primary_checkout_id = Some(replacement_checkout_id);
        write_json(&repos_path, &repositories).unwrap();

        let result = validate_snapshot_fields(&fixture.paths, &fixture.attempt, None);

        assert!(matches!(
            result,
            Err(RepositoryRoutingErrorCode::Inconsistent)
        ));
    }

    #[test]
    fn snapshot_validator_rejects_membership_revision_mismatch() {
        // B5：snapshot 的 membership revision 与当前 manifest/selection 不一致 → Inconsistent。
        let temp = tempfile::tempdir().unwrap();
        let fixture = snapshot_with_stale_membership_revision(temp.path());

        let result = validate_snapshot_fields(&fixture.paths, &fixture.attempt, None);

        assert!(matches!(
            result,
            Err(RepositoryRoutingErrorCode::Inconsistent)
        ));
    }

    fn snapshot_with_stale_membership_revision(root: &std::path::Path) -> SnapshotFixture {
        let paths = ProductAppPaths::new(root.join(".aria"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".to_string(),
                description: None,
            })
            .unwrap();
        let logical_repository_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        let repository_path = root.join("repository_0001");
        let source_identity = RepositorySourceIdentity::from_git_parts(
            &repository_path,
            repository_path.join(".git"),
            None,
        );
        let authority = LogicalCodebaseStore::new(paths.clone());
        let mut manifest = LogicalCodebaseManifest::new(
            "project_0001",
            root.join("aggregate-root"),
            vec![logical_repository_id],
        );
        manifest.membership_revision = 2;
        authority.save_manifest("project_0001", &manifest).unwrap();
        authority
            .save_member(
                "project_0001",
                &CodebaseMemberRecord {
                    logical_repository_id,
                    physical_repository_id: "repository_0001".to_string(),
                    alias: "repository_0001".to_string(),
                    role: "repository".to_string(),
                    ordinal: 1,
                    source_identity: source_identity.clone(),
                    repo_type: RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_id],
                    status: MemberStatus::Active,
                    created_at: "2026-08-11T00:00:00Z".to_string(),
                    updated_at: "2026-08-11T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        authority
            .save_checkout(
                "project_0001",
                &RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id,
                    physical_repository_id: "repository_0001".to_string(),
                    kind: CheckoutKind::Main,
                    canonical_path: repository_path.clone(),
                    checkout_path_hash: "sha256:checkout".to_string(),
                    git_dir_identity: source_identity.git_dir_identity(),
                    revision: Some("abcdef".to_string()),
                    availability: CheckoutAvailability::Available,
                    observed_at: "2026-08-11T00:00:00Z".to_string(),
                    created_at: "2026-08-11T00:00:00Z".to_string(),
                    updated_at: "2026-08-11T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        IssueCodebaseSelectionStore::new(paths.clone())
            .save(&IssueCodebaseSelection::explicit(
                "project_0001",
                "issue_0001",
                vec![logical_repository_id],
                Vec::new(),
                vec![logical_repository_id],
                None,
            ))
            .unwrap();
        crate::product::json_store::write_json(
            &paths.project_root("project_0001").join("repos.json"),
            &[RepositoryRecord {
                id: "repository_0001".to_string(),
                project_id: "project_0001".to_string(),
                name: "repository_0001".to_string(),
                path: repository_path.clone(),
                repo_hash: "sha256:repository".to_string(),
                runtime_root: repository_path.join(".aria/runtime"),
                default_policy_preset: "manual-write".to_string(),
                default_provider_mode: "fake".to_string(),
                created_at: "2026-08-11T00:00:00Z".to_string(),
                logical_repository_id: Some(logical_repository_id),
                primary_checkout_id: Some(checkout_id),
                identity_schema_version: 1,
                updated_at: "2026-08-11T00:00:00Z".to_string(),
            }],
        )
        .unwrap();

        SnapshotFixture {
            paths,
            attempt: CodingExecutionAttempt {
                id: "coding_attempt_0001".to_string(),
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                work_item_id: "work_item_0001".to_string(),
                attempt_no: 1,
                scope: CodingAttemptScope::WorkItem,
                status: CodingAttemptStatus::Running,
                version: 0,
                manual_recovery_reason: None,
                admission_ticket_consumed_at: None,
                admission_kind: crate::product::coding_models::CodingAdmissionKind::LegacyGroup,
                stage: CodingExecutionStage::WorktreePrepare,
                base_branch: "main".to_string(),
                branch_name: "aria/attempt".to_string(),
                worktree_path: None,
                provider_config_snapshot: ProviderConfigSnapshot {
                    author: ProviderName::Fake,
                    reviewer: None,
                    review_rounds: 0,
                    permission_modes: Default::default(),
                },
                rework_count: 0,
                max_auto_rework: 0,
                work_item_group_id: None,
                current_work_item_id: Some("work_item_0001".to_string()),
                active_unit_id: None,
                head_commit: None,
                pushed_remote: None,
                review_request_id: None,
                provider_conversations: Vec::new(),
                created_at: "2026-08-11T00:00:00Z".to_string(),
                updated_at: "2026-08-11T00:00:00Z".to_string(),
                target_snapshot: Some(AttemptTargetSnapshot {
                    logical_repository_id,
                    checkout_id,
                    physical_repository_id: "repository_0001".to_string(),
                    canonical_path: repository_path,
                    git_dir_identity: source_identity.git_dir_identity(),
                    revision: Some("abcdef".to_string()),
                    policy_digest: String::new(),
                    membership_revision: 1,
                    captured_at: "2026-08-11T00:00:00Z".to_string(),
                    capture_source: "test".to_string(),
                }),
                completed_at: None,
            },
        }
    }

    /// 构造仅 revision 字段可配置、其余身份一致的 fixture（membership_revision 一致），
    /// 用于隔离验证 revision 比对语义。TempDir 由调用方持有。
    fn snapshot_fixture_with_revisions(
        root: &std::path::Path,
        checkout_revision: Option<&str>,
        snapshot_revision: Option<&str>,
    ) -> SnapshotFixture {
        let mut fixture = snapshot_with_stale_membership_revision(root);
        // 把 membership_revision 对齐到 manifest（2），只测 revision 行为。
        let snapshot = fixture.attempt.target_snapshot.as_mut().unwrap();
        snapshot.membership_revision = 2;
        snapshot.revision = snapshot_revision.map(str::to_string);

        let authority = LogicalCodebaseStore::new(fixture.paths.clone());
        let snapshot = fixture.attempt.target_snapshot.as_ref().unwrap();
        let mut checkout = authority
            .load_checkout("project_0001", snapshot.checkout_id)
            .unwrap()
            .unwrap();
        checkout.revision = checkout_revision.map(str::to_string);
        authority.save_checkout("project_0001", &checkout).unwrap();

        fixture
    }

    #[test]
    fn snapshot_validator_skips_revision_when_checkout_revision_unobserved() {
        // 生产形态：checkout.revision 恒为 None（registration/迁移不持久化），snapshot.revision
        // 为 admission 时冻结的 git HEAD。身份一致时不得因 revision 未观测而拒绝。
        let temp = tempfile::tempdir().unwrap();
        let fixture = snapshot_fixture_with_revisions(temp.path(), None, Some("observed-head"));

        assert_eq!(
            validate_snapshot_fields(&fixture.paths, &fixture.attempt, None),
            Ok(())
        );
    }

    #[test]
    fn snapshot_validator_rejects_observed_revision_mismatch() {
        // checkout.revision 已观测（Some）时仍严格拒绝漂移。
        let temp = tempfile::tempdir().unwrap();
        let fixture =
            snapshot_fixture_with_revisions(temp.path(), Some("old-head"), Some("new-head"));

        assert!(matches!(
            validate_snapshot_fields(&fixture.paths, &fixture.attempt, None),
            Err(RepositoryRoutingErrorCode::Inconsistent)
        ));
    }
}
