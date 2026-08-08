use std::path::PathBuf;

use chrono::Utc;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::logical_codebase::{
    CodebaseMemberRecord, LogicalCodebaseFeature, LogicalCodebaseStore, RepositoryType,
};
use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};

/// The caller-owned input for one attach-only logical codebase registration.
///
/// This command deliberately carries no initialization or Git-finalization
/// controls: it only attaches an already-existing checkout to product
/// authority records.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachOnlyRegistrationInput {
    pub project_id: String,
    pub alias: String,
    pub role: String,
    pub canonical_path: PathBuf,
    pub repo_type: RepositoryType,
    pub tech_stack: Vec<String>,
    pub idempotency_key: String,
}

/// Coordinates an attach-only registration without entering the single-
/// repository initialization chain.
#[derive(Debug, Clone)]
pub struct LogicalCodebaseRegistrationCoordinator {
    paths: ProductAppPaths,
    repositories: RepositoryStore,
    feature: LogicalCodebaseFeature,
}

impl LogicalCodebaseRegistrationCoordinator {
    pub fn new(
        paths: ProductAppPaths,
        repositories: RepositoryStore,
        feature: LogicalCodebaseFeature,
    ) -> Self {
        Self {
            paths,
            repositories,
            feature,
        }
    }

    pub fn attach_member(
        &self,
        input: AttachOnlyRegistrationInput,
    ) -> Result<CodebaseMemberRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.idempotency_key)?;
        if !self.feature.is_enabled() {
            return Err(ProductStoreError::Conflict {
                kind: "logical_codebase_feature_disabled",
                id: input.project_id,
            });
        }

        let repository = self.repositories.create(CreateRepositoryInput {
            project_id: input.project_id.clone(),
            name: input.alias.clone(),
            path: input.canonical_path,
            default_policy_preset: None,
            default_provider_mode: None,
            idempotency_key: input.idempotency_key,
        })?;
        validate_relative_id(&repository.id)?;
        let logical_repository_id = repository.logical_repository_id.ok_or_else(|| {
            ProductStoreError::IdentityMismatch {
                kind: "repository_projection",
                id: repository.id.clone(),
            }
        })?;

        let store = LogicalCodebaseStore::new(self.paths.clone());
        let mut member = store
            .load_member(&input.project_id, logical_repository_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "logical_codebase_member",
                id: logical_repository_id.0.to_string(),
            })?;
        if member.physical_repository_id != repository.id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "logical_codebase_member",
                id: logical_repository_id.0.to_string(),
            });
        }

        member.alias = input.alias;
        member.role = input.role;
        member.repo_type = input.repo_type;
        member.tech_stack = input.tech_stack;
        member.updated_at = Utc::now().to_rfc3339();
        store.save_member(&input.project_id, &member)?;
        Ok(member)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::LogicalCodebaseFeature;
    use crate::product::project_store::{CreateProjectInput, ProjectStore};
    use crate::product::repository_store::RepositoryStore;

    struct AttachFixture {
        _root: tempfile::TempDir,
        paths: ProductAppPaths,
        coordinator: LogicalCodebaseRegistrationCoordinator,
        git_root: PathBuf,
        head_before: String,
        branch_before: String,
        status_before: String,
    }

    #[test]
    fn attach_only_registration_creates_member_without_initialization_operation() {
        let fixture = attach_fixture();
        let member = fixture
            .coordinator
            .attach_member(AttachOnlyRegistrationInput {
                project_id: "project_0001".into(),
                alias: "api".into(),
                role: "service".into(),
                canonical_path: fixture.git_root.clone(),
                repo_type: RepositoryType::Backend,
                tech_stack: vec!["rust".into()],
                idempotency_key: "batch-1:item-api".into(),
            })
            .unwrap();

        assert_eq!(member.alias, "api");
        assert_eq!(member.role, "service");
        assert_eq!(member.repo_type, RepositoryType::Backend);
        assert_eq!(member.tech_stack, vec!["rust"]);
        assert!(
            !fixture
                .paths
                .repository_initializations_root("project_0001")
                .exists()
        );
        assert_eq!(
            git_output(&fixture.git_root, &["rev-parse", "HEAD"]),
            fixture.head_before
        );
        assert_eq!(
            git_output(&fixture.git_root, &["branch", "--show-current"]),
            fixture.branch_before
        );
        assert_eq!(
            git_output(&fixture.git_root, &["status", "--porcelain"]),
            fixture.status_before
        );
    }

    fn attach_fixture() -> AttachFixture {
        let root = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(root.path());
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "project".into(),
                description: None,
            })
            .unwrap();

        let git_root = root.path().join("api");
        fs::create_dir_all(&git_root).unwrap();
        git(&git_root, &["init", "--quiet"]);
        git(&git_root, &["config", "user.email", "aria@example.invalid"]);
        git(&git_root, &["config", "user.name", "Aria Test"]);
        fs::write(git_root.join("README.md"), "# API\n").unwrap();
        git(&git_root, &["add", "README.md"]);
        git(&git_root, &["commit", "--quiet", "-m", "initial"]);

        let head_before = git_output(&git_root, &["rev-parse", "HEAD"]);
        let branch_before = git_output(&git_root, &["branch", "--show-current"]);
        let status_before = git_output(&git_root, &["status", "--porcelain"]);
        let repositories = RepositoryStore::with_logical_codebase_feature(
            paths.clone(),
            LogicalCodebaseFeature::enabled(),
        );

        AttachFixture {
            _root: root,
            paths: paths.clone(),
            coordinator: LogicalCodebaseRegistrationCoordinator::new(
                paths,
                repositories,
                LogicalCodebaseFeature::enabled(),
            ),
            git_root,
            head_before,
            branch_before,
            status_before,
        }
    }

    fn git(cwd: &Path, arguments: &[&str]) {
        let output = Command::new("git")
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

    fn git_output(cwd: &Path, arguments: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(cwd)
            .args(arguments)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    }
}
