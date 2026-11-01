use std::process::Command;

use chrono::Utc;
use thiserror::Error;

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_models::AttemptTargetSnapshot;
use crate::product::json_store::ProductStoreError;
use crate::product::logical_codebase::{
    AggregatePolicyArtifactStore, LogicalCodebaseStore, LogicalRepositoryId, MemberStatus,
};
use crate::product::project_store::ProjectStore;
use crate::product::repository_store::RepositoryStore;

const CAPTURE_SOURCE: &str = "coding_attempt_create";

/// 尝试创建时目标仓身份快照的 fail-closed 错误。
#[derive(Debug, Error)]
pub enum TargetSnapshotError {
    #[error("attempt target was not found")]
    NotFound,
    #[error("attempt target member is inactive")]
    Inactive,
    #[error("attempt target resolution failed: {0}")]
    ResolveFailed(#[from] ProductStoreError),
    #[error("git rev-parse HEAD failed")]
    GitRevParseFailed,
    #[error("aggregate policy artifact is missing")]
    PolicyMissing,
}

/// 读取 logical-codebase 权威记录和目标 checkout 的当前 HEAD，生成一个不可变的
/// coding attempt 目标快照。任何无法解析的身份、政策或 Git HEAD 都 fail-closed。
///
/// v1.3：`lc_id = Some` 时按 `logical-codebases/{lc_id}/` 子树权威记录解析
/// （R9 编码/交付链切换点）；`None`（单仓/旧数据）保持 legacy project 级路径。
pub fn build_attempt_target_snapshot(
    paths: &ProductAppPaths,
    project_id: &str,
    logical_id: LogicalRepositoryId,
    lc_id: Option<&str>,
) -> Result<AttemptTargetSnapshot, TargetSnapshotError> {
    let authority = match lc_id {
        Some(lc_id) => LogicalCodebaseStore::for_lc(paths.clone(), lc_id),
        None => LogicalCodebaseStore::new(paths.clone()),
    };
    if authority
        .load_member(project_id, logical_id)?
        .is_some_and(|member| member.status != MemberStatus::Active)
    {
        return Err(TargetSnapshotError::Inactive);
    }

    let (member, checkout, repository) = match lc_id {
        Some(lc_id) => RepositoryStore::new(paths.clone())
            .resolve_logical_repository_for_issue_codebase(project_id, Some(lc_id), logical_id)
            .map_err(map_resolve_error)?,
        None => {
            let project = ProjectStore::new(paths.clone())
                .get(project_id)
                .map_err(map_resolve_error)?;
            RepositoryStore::for_project(paths.clone(), &project)
                .resolve_logical_repository_for_issue_codebase(project_id, None, logical_id)
                .map_err(map_resolve_error)?
        }
    };
    let manifest = authority
        .load_manifest(project_id)?
        .ok_or(TargetSnapshotError::NotFound)?;
    let policy_store = match lc_id {
        Some(lc_id) => AggregatePolicyArtifactStore::for_lc(paths.clone(), lc_id),
        None => AggregatePolicyArtifactStore::new(paths.clone()),
    };
    let policy = policy_store
        .get(project_id)?
        .ok_or(TargetSnapshotError::PolicyMissing)?;
    let revision = git_head(&checkout.canonical_path)?;

    Ok(AttemptTargetSnapshot {
        logical_repository_id: member.logical_repository_id,
        checkout_id: checkout.checkout_id,
        physical_repository_id: repository.id,
        canonical_path: checkout.canonical_path,
        git_dir_identity: checkout.git_dir_identity,
        revision: Some(revision),
        policy_digest: policy.digest,
        membership_revision: manifest.membership_revision,
        captured_at: Utc::now().to_rfc3339(),
        capture_source: CAPTURE_SOURCE.to_string(),
    })
}

fn map_resolve_error(error: ProductStoreError) -> TargetSnapshotError {
    match error {
        ProductStoreError::NotFound { .. } => TargetSnapshotError::NotFound,
        error => TargetSnapshotError::ResolveFailed(error),
    }
}

fn git_head(path: &std::path::Path) -> Result<String, TargetSnapshotError> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(path)
        .output()
        .map_err(|_| TargetSnapshotError::GitRevParseFailed)?;
    if !output.status.success() {
        return Err(TargetSnapshotError::GitRevParseFailed);
    }

    let revision = String::from_utf8(output.stdout)
        .map_err(|_| TargetSnapshotError::GitRevParseFailed)?
        .trim()
        .to_string();
    if revision.is_empty() {
        return Err(TargetSnapshotError::GitRevParseFailed);
    }
    Ok(revision)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::process::Command;

    use super::{TargetSnapshotError, build_attempt_target_snapshot};
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::{
        AggregatePolicyArtifactStore, LogicalCodebaseFeature, LogicalCodebaseStore,
        LogicalRepositoryId, MemberStatus,
    };
    use crate::product::project_store::{CreateProjectInput, ProjectStore};
    use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};
    use tempfile::TempDir;
    use uuid::Uuid;

    struct TargetSnapshotFixture {
        _root: TempDir,
        paths: ProductAppPaths,
        project_id: String,
        logical_id: LogicalRepositoryId,
        checkout_id: crate::product::logical_codebase::RepositoryCheckoutId,
        physical_repository_id: String,
        canonical_path: std::path::PathBuf,
        git_dir_identity: String,
        revision: String,
        policy_digest: String,
        membership_revision: u64,
    }

    fn target_snapshot_fixture() -> TargetSnapshotFixture {
        let root = tempfile::tempdir().expect("temporary product root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let project = ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "snapshot project".to_string(),
                description: None,
            })
            .expect("create project");
        let canonical_path = root.path().join("api");
        fs::create_dir_all(&canonical_path).expect("create repository root");
        run_git(&canonical_path, &["init", "--quiet"]);
        run_git(
            &canonical_path,
            &["config", "user.email", "snapshot@example.test"],
        );
        run_git(&canonical_path, &["config", "user.name", "Target Snapshot"]);
        fs::write(canonical_path.join("README.md"), "# api\n").expect("write initial file");
        run_git(&canonical_path, &["add", "README.md"]);
        run_git(
            &canonical_path,
            &["commit", "--quiet", "-m", "initial commit"],
        );
        let revision = git_stdout(&canonical_path, &["rev-parse", "HEAD"]);

        let repository = RepositoryStore::with_logical_codebase_feature(
            paths.clone(),
            LogicalCodebaseFeature::enabled(),
        )
        .create(CreateRepositoryInput {
            project_id: project.id.clone(),
            name: "api".to_string(),
            path: canonical_path.clone(),
            default_policy_preset: None,
            default_provider_mode: None,
            idempotency_key: "target-snapshot-fixture".to_string(),
        })
        .expect("register logical repository");
        let logical_id = repository
            .logical_repository_id
            .expect("logical repository ID");
        let checkout_id = repository.primary_checkout_id.expect("checkout ID");
        let manifest = LogicalCodebaseStore::new(paths.clone())
            .load_manifest(&project.id)
            .expect("load manifest")
            .expect("manifest");
        let policy = AggregatePolicyArtifactStore::new(paths.clone())
            .ensure_bootstrap(&manifest)
            .expect("ensure aggregate policy");
        let checkout = LogicalCodebaseStore::new(paths.clone())
            .load_checkout(&project.id, checkout_id)
            .expect("load checkout")
            .expect("checkout");

        TargetSnapshotFixture {
            _root: root,
            paths,
            project_id: project.id,
            logical_id,
            checkout_id,
            physical_repository_id: repository.id,
            canonical_path,
            git_dir_identity: checkout.git_dir_identity,
            revision,
            policy_digest: policy.digest,
            membership_revision: manifest.membership_revision,
        }
    }

    fn run_git(cwd: &std::path::Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .status()
            .expect("start git");
        assert!(status.success(), "git {} failed", args.join(" "));
    }

    fn git_stdout(cwd: &std::path::Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .expect("start git");
        assert!(output.status.success(), "git {} failed", args.join(" "));
        String::from_utf8(output.stdout)
            .expect("UTF-8 git output")
            .trim()
            .to_string()
    }

    #[test]
    fn build_attempt_target_snapshot_assembles_three_layer_identity() {
        let fixture = target_snapshot_fixture();

        let snapshot = build_attempt_target_snapshot(
            &fixture.paths,
            &fixture.project_id,
            fixture.logical_id,
            None,
        )
        .expect("build target snapshot");

        assert_eq!(snapshot.logical_repository_id, fixture.logical_id);
        assert_eq!(snapshot.checkout_id, fixture.checkout_id);
        assert_eq!(
            snapshot.physical_repository_id,
            fixture.physical_repository_id
        );
        assert_eq!(snapshot.canonical_path, fixture.canonical_path);
        assert_eq!(snapshot.git_dir_identity, fixture.git_dir_identity);
        assert_eq!(
            snapshot.revision.as_deref(),
            Some(fixture.revision.as_str())
        );
        assert_eq!(snapshot.policy_digest, fixture.policy_digest);
        assert_eq!(snapshot.membership_revision, fixture.membership_revision);
        assert!(chrono::DateTime::parse_from_rfc3339(&snapshot.captured_at).is_ok());
        assert_eq!(snapshot.capture_source, "coding_attempt_create");
    }

    #[test]
    fn build_attempt_target_snapshot_fails_when_member_inactive() {
        let fixture = target_snapshot_fixture();
        let authority = LogicalCodebaseStore::new(fixture.paths.clone());
        let mut member = authority
            .load_member(&fixture.project_id, fixture.logical_id)
            .expect("load member")
            .expect("member");
        member.status = MemberStatus::Removed;
        authority
            .save_member(&fixture.project_id, &member)
            .expect("save inactive member");

        assert!(matches!(
            build_attempt_target_snapshot(
                &fixture.paths,
                &fixture.project_id,
                fixture.logical_id,
                None
            ),
            Err(TargetSnapshotError::Inactive)
        ));
    }

    #[test]
    fn build_attempt_target_snapshot_fails_when_policy_missing() {
        let fixture = target_snapshot_fixture();
        fs::remove_file(
            fixture
                .paths
                .aggregate_policy_artifact_path(&fixture.project_id),
        )
        .expect("remove aggregate policy");

        assert!(matches!(
            build_attempt_target_snapshot(
                &fixture.paths,
                &fixture.project_id,
                fixture.logical_id,
                None
            ),
            Err(TargetSnapshotError::PolicyMissing)
        ));
    }

    /// R9：非 legacy 新 LC fixture——权威记录全部落在
    /// `logical-codebases/{lc_id}/` 子树，repos.json 无投影。
    struct NewLcFixture {
        _root: TempDir,
        paths: ProductAppPaths,
        project_id: String,
        lc_id: String,
        logical_id: LogicalRepositoryId,
        checkout_id: crate::product::logical_codebase::RepositoryCheckoutId,
        physical_repository_id: String,
        canonical_path: std::path::PathBuf,
        git_dir_identity: String,
        revision: String,
        policy_digest: String,
        membership_revision: u64,
    }

    fn new_lc_target_snapshot_fixture() -> NewLcFixture {
        let root = tempfile::tempdir().expect("temporary product root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let project = ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "new lc snapshot project".to_string(),
                description: None,
            })
            .expect("create project");
        let record = LogicalCodebaseStore::new(paths.clone())
            .create(
                &project.id,
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "new-lc".to_string(),
                    aggregate_root: root.path().join("aggregate-root"),
                },
            )
            .expect("create logical codebase record");
        let lc_id = record.id;

        let canonical_path = root.path().join("api");
        fs::create_dir_all(&canonical_path).expect("create repository root");
        run_git(&canonical_path, &["init", "--quiet"]);
        run_git(
            &canonical_path,
            &["config", "user.email", "lc@example.test"],
        );
        run_git(&canonical_path, &["config", "user.name", "New LC"]);
        fs::write(canonical_path.join("README.md"), "# api\n").expect("write file");
        run_git(&canonical_path, &["add", "README.md"]);
        run_git(
            &canonical_path,
            &["commit", "--quiet", "-m", "initial commit"],
        );
        let revision = git_stdout(&canonical_path, &["rev-parse", "HEAD"]);

        let authority = LogicalCodebaseStore::for_lc(paths.clone(), lc_id.clone());
        let logical_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = crate::product::logical_codebase::RepositoryCheckoutId(Uuid::new_v4());
        let physical_repository_id = format!("repository_{}", Uuid::new_v4().simple());
        let manifest = crate::product::logical_codebase::LogicalCodebaseManifest::new(
            &project.id,
            root.path().join("aggregate-root"),
            vec![logical_id],
        );
        authority
            .save_manifest(&project.id, &manifest)
            .expect("save lc manifest");
        let now = "2026-08-18T00:00:00Z".to_string();
        let source_identity =
            crate::product::logical_codebase::RepositorySourceIdentity::from_git_parts(
                &canonical_path,
                canonical_path.join(".git"),
                None,
            );
        let git_dir_identity = source_identity.git_dir_identity().to_string();
        authority
            .save_member(
                &project.id,
                &crate::product::logical_codebase::CodebaseMemberRecord {
                    logical_repository_id: logical_id,
                    physical_repository_id: physical_repository_id.clone(),
                    alias: "api".to_string(),
                    role: "repository".to_string(),
                    ordinal: 0,
                    source_identity: source_identity.clone(),
                    repo_type: crate::product::logical_codebase::RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_id],
                    status: MemberStatus::Active,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                },
            )
            .expect("save lc member");
        authority
            .save_checkout(
                &project.id,
                &crate::product::logical_codebase::RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: logical_id,
                    physical_repository_id: physical_repository_id.clone(),
                    kind: crate::product::logical_codebase::CheckoutKind::Main,
                    canonical_path: canonical_path.clone(),
                    checkout_path_hash: "sha256:checkout".to_string(),
                    git_dir_identity: git_dir_identity.clone(),
                    revision: None,
                    availability: crate::product::logical_codebase::CheckoutAvailability::Available,
                    observed_at: now.clone(),
                    created_at: now.clone(),
                    updated_at: now,
                },
            )
            .expect("save lc checkout");
        crate::product::logical_codebase::IdentityRegistryStore::new(paths.clone())
            .upsert_active(
                &project.id,
                crate::product::logical_codebase::IdentityRegistryEntry::active(
                    source_identity,
                    logical_id,
                    physical_repository_id.clone(),
                    checkout_id,
                    "new-lc-snapshot-fixture".to_string(),
                ),
            )
            .expect("register identity");
        let policy = AggregatePolicyArtifactStore::for_lc(paths.clone(), lc_id.clone())
            .ensure_bootstrap(&manifest)
            .expect("ensure lc aggregate policy");
        // 新 LC 登记不写 repos.json 投影。
        assert!(!paths.project_root(&project.id).join("repos.json").exists());

        NewLcFixture {
            _root: root,
            paths,
            project_id: project.id,
            lc_id,
            logical_id,
            checkout_id,
            physical_repository_id,
            canonical_path,
            git_dir_identity,
            revision,
            policy_digest: policy.digest,
            membership_revision: manifest.membership_revision,
        }
    }

    #[test]
    fn build_attempt_target_snapshot_resolves_new_lc_subtree_by_lc_id() {
        let fixture = new_lc_target_snapshot_fixture();

        let snapshot = build_attempt_target_snapshot(
            &fixture.paths,
            &fixture.project_id,
            fixture.logical_id,
            Some(&fixture.lc_id),
        )
        .expect("build target snapshot from lc subtree");

        assert_eq!(snapshot.logical_repository_id, fixture.logical_id);
        assert_eq!(snapshot.checkout_id, fixture.checkout_id);
        assert_eq!(
            snapshot.physical_repository_id,
            fixture.physical_repository_id
        );
        assert_eq!(snapshot.canonical_path, fixture.canonical_path);
        assert_eq!(snapshot.git_dir_identity, fixture.git_dir_identity);
        assert_eq!(
            snapshot.revision.as_deref(),
            Some(fixture.revision.as_str())
        );
        assert_eq!(snapshot.policy_digest, fixture.policy_digest);
        assert_eq!(snapshot.membership_revision, fixture.membership_revision);
    }

    #[test]
    fn build_attempt_target_snapshot_fails_closed_when_new_lc_policy_missing() {
        let fixture = new_lc_target_snapshot_fixture();
        fs::remove_file(
            fixture
                .paths
                .logical_codebase_record_root(&fixture.project_id, &fixture.lc_id)
                .join("aggregate-policy.json"),
        )
        .ok();

        assert!(matches!(
            build_attempt_target_snapshot(
                &fixture.paths,
                &fixture.project_id,
                fixture.logical_id,
                Some(&fixture.lc_id)
            ),
            Err(TargetSnapshotError::PolicyMissing)
        ));
    }
}
