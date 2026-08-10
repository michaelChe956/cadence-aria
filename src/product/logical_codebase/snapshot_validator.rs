use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::logical_codebase::{LogicalCodebaseStore, RepositoryRoutingErrorCode};
use crate::product::repository_store::RepositoryStore;

/// 校验 attempt 快照仍与当前逻辑代码库权威记录完全一致。
///
/// 快照存在时不得降级到物理仓库：无法验证的任何字段都统一视为不一致。
pub fn validate_snapshot_fields(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<(), RepositoryRoutingErrorCode> {
    let Some(snapshot) = attempt.target_snapshot.as_ref() else {
        return Err(RepositoryRoutingErrorCode::Inconsistent);
    };
    let authority = LogicalCodebaseStore::new(paths.clone());
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
        || checkout.revision != snapshot.revision
        || manifest.membership_revision != snapshot.membership_revision
    {
        return Err(RepositoryRoutingErrorCode::Inconsistent);
    }

    RepositoryStore::new(paths.clone())
        .resolve_logical_repository_strict(&attempt.project_id, snapshot.logical_repository_id)
        .map_err(|_| RepositoryRoutingErrorCode::Inconsistent)?;

    Ok(())
}

#[cfg(test)]
mod tests {

    use uuid::Uuid;

    use super::*;
    use crate::product::coding_models::{
        AttemptTargetSnapshot, CodingAttemptScope, CodingAttemptStatus, CodingExecutionStage,
    };
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
    fn snapshot_validator_rejects_membership_revision_mismatch() {
        // B5：snapshot 的 membership revision 与当前 manifest/selection 不一致 → Inconsistent。
        let temp = tempfile::tempdir().unwrap();
        let fixture = snapshot_with_stale_membership_revision(temp.path());

        let result = validate_snapshot_fields(&fixture.paths, &fixture.attempt);

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
}
