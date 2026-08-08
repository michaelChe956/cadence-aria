use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::logical_codebase::{
    CodebaseMemberRecord, LogicalCodebaseFeature, LogicalCodebaseStore, RepositoryType,
};
use crate::product::repository_store::{CreateRepositoryInput, RepositoryStore};

/// Canonical, non-Git common parent that has passed aggregate-root admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalAggregateRoot {
    pub canonical_path: PathBuf,
}

/// A deterministic admission failure for the aggregate root.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct AggregateRootPreflightError {
    code: &'static str,
    message: String,
}

impl AggregateRootPreflightError {
    pub fn code(&self) -> &'static str {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Validates the filesystem ownership and containment invariants for an
/// aggregate root before any member discovery or registration is performed.
#[derive(Debug, Clone)]
pub struct AggregateRootPreflight {
    paths: ProductAppPaths,
}

impl AggregateRootPreflight {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn validate(
        &self,
        project_id: &str,
        root: &Path,
        candidate_paths: &[PathBuf],
    ) -> Result<CanonicalAggregateRoot, AggregateRootPreflightError> {
        validate_relative_id(project_id).map_err(|error| {
            preflight_error(
                "aggregate_root_invalid_project_id",
                format!(
                    "project ID {project_id:?} is not a safe relative identifier: {error}; use a project ID without path separators"
                ),
            )
        })?;

        let canonical_root = canonicalize_for_preflight(root, "aggregate_root_missing")?;
        if !canonical_root.is_dir() {
            return Err(preflight_error(
                "aggregate_root_missing",
                format!(
                    "aggregate root {} is not a directory; choose an existing common parent directory",
                    canonical_root.display()
                ),
            ));
        }
        if is_git_root(&canonical_root)? {
            return Err(preflight_error(
                "aggregate_root_is_git",
                format!(
                    "aggregate root {} is a Git repository; choose its non-Git common parent instead",
                    canonical_root.display()
                ),
            ));
        }

        for candidate in candidate_paths {
            self.validate_member_path(root, &canonical_root, candidate)?;
        }

        self.reject_owned_root_files(&canonical_root)?;
        self.reject_overlapping_manifest_root(project_id, &canonical_root)?;

        Ok(CanonicalAggregateRoot {
            canonical_path: canonical_root,
        })
    }

    fn validate_member_path(
        &self,
        supplied_root: &Path,
        canonical_root: &Path,
        candidate: &Path,
    ) -> Result<(), AggregateRootPreflightError> {
        let candidate_is_under_root =
            candidate.starts_with(supplied_root) || candidate.starts_with(canonical_root);
        let canonical_member = canonicalize_for_preflight(candidate, "member_path_missing")?;
        if canonical_member == canonical_root {
            return Err(preflight_error(
                "member_path_outside_root",
                format!(
                    "member path {} resolves to aggregate root {}; select a descendant member directory",
                    candidate.display(),
                    canonical_root.display()
                ),
            ));
        }
        if !candidate_is_under_root {
            return Err(preflight_error(
                "member_path_outside_root",
                format!(
                    "member path {} resolves to {} outside aggregate root {}; select a path below the aggregate root",
                    candidate.display(),
                    canonical_member.display(),
                    canonical_root.display()
                ),
            ));
        }
        if !canonical_member.starts_with(canonical_root) {
            return Err(preflight_error(
                "member_symlink_escape",
                format!(
                    "member path {} resolves to {} outside aggregate root {}; remove the escaping symlink or select an in-root member",
                    candidate.display(),
                    canonical_member.display(),
                    canonical_root.display()
                ),
            ));
        }
        if is_linked_worktree(&canonical_member)? {
            return Err(preflight_error(
                "nested_worktree",
                format!(
                    "member path {} resolves to linked worktree {}; select the main checkout instead",
                    candidate.display(),
                    canonical_member.display()
                ),
            ));
        }
        Ok(())
    }

    fn reject_owned_root_files(
        &self,
        canonical_root: &Path,
    ) -> Result<(), AggregateRootPreflightError> {
        for name in ["CLAUDE.md", "AGENTS.md", ".aria"] {
            let path = canonical_root.join(name);
            if path_exists_for_preflight(&path)? {
                return Err(preflight_error(
                    "aggregate_root_ownership_conflict",
                    format!(
                        "aggregate root {} already contains user-owned {}; move or merge it before aggregate initialization",
                        canonical_root.display(),
                        path.display()
                    ),
                ));
            }
        }
        Ok(())
    }

    fn reject_overlapping_manifest_root(
        &self,
        project_id: &str,
        canonical_root: &Path,
    ) -> Result<(), AggregateRootPreflightError> {
        for manifest in LogicalCodebaseStore::new(self.paths.clone())
            .list_manifests()
            .map_err(|error| {
                preflight_error(
                    "aggregate_root_overlap",
                    format!(
                        "could not inspect existing logical-codebase manifests while validating {}: {error}; resolve the state-store error before retrying",
                        canonical_root.display()
                    ),
                )
            })?
        {
            if manifest.project_id == project_id {
                continue;
            }
            let existing_root = canonicalize_for_preflight(
                &manifest.provider_context_root,
                "aggregate_root_overlap",
            )?;
            if paths_overlap(canonical_root, &existing_root) {
                return Err(preflight_error(
                    "aggregate_root_overlap",
                    format!(
                        "aggregate root {} overlaps logical codebase root {} owned by project {}; choose a disjoint common parent",
                        canonical_root.display(),
                        existing_root.display(),
                        manifest.project_id
                    ),
                ));
            }
        }
        Ok(())
    }
}

fn canonicalize_for_preflight(
    path: &Path,
    missing_code: &'static str,
) -> Result<PathBuf, AggregateRootPreflightError> {
    fs::canonicalize(path).map_err(|error| {
        preflight_error(
            missing_code,
            format!(
                "path {} cannot be canonicalized: {error}; choose an existing accessible path",
                path.display()
            ),
        )
    })
}

fn is_git_root(path: &Path) -> Result<bool, AggregateRootPreflightError> {
    let git_path = path.join(".git");
    let metadata = match fs::symlink_metadata(&git_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(preflight_error(
                "aggregate_root_is_git",
                format!(
                    "could not inspect Git metadata {}: {error}; fix access and retry",
                    git_path.display()
                ),
            ));
        }
    };
    Ok(metadata.file_type().is_dir()
        || metadata.file_type().is_file()
        || metadata.file_type().is_symlink())
}

fn is_linked_worktree(path: &Path) -> Result<bool, AggregateRootPreflightError> {
    let git_file = path.join(".git");
    let metadata = match fs::symlink_metadata(&git_file) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(preflight_error(
                "nested_worktree",
                format!(
                    "could not inspect worktree metadata {}: {error}; fix access and retry",
                    git_file.display()
                ),
            ));
        }
    };
    if !metadata.file_type().is_file() {
        return Ok(false);
    }

    let contents = fs::read_to_string(&git_file).map_err(|error| {
        preflight_error(
            "nested_worktree",
            format!(
                "could not read worktree metadata {}: {error}; select a main checkout instead",
                git_file.display()
            ),
        )
    })?;
    let Some(gitdir) = contents.strip_prefix("gitdir:") else {
        return Ok(false);
    };
    let gitdir = gitdir.trim();
    if gitdir.is_empty() {
        return Err(preflight_error(
            "nested_worktree",
            format!(
                "worktree metadata {} has an empty gitdir target; select a main checkout instead",
                git_file.display()
            ),
        ));
    }

    let gitdir_path = Path::new(gitdir);
    let gitdir_path = if gitdir_path.is_absolute() {
        gitdir_path.to_path_buf()
    } else {
        path.join(gitdir_path)
    };
    let canonical_gitdir = fs::canonicalize(&gitdir_path).map_err(|error| {
            preflight_error(
                "nested_worktree",
                format!(
                    "worktree metadata {} points to inaccessible gitdir {}: {error}; select a main checkout instead",
                    git_file.display(),
                    gitdir_path.display()
                ),
            )
        })?;
    if canonical_gitdir
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "worktrees")
    {
        return Ok(true);
    }
    Ok(false)
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn path_exists_for_preflight(path: &Path) -> Result<bool, AggregateRootPreflightError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(preflight_error(
            "aggregate_root_ownership_conflict",
            format!(
                "could not inspect existing root asset {}: {error}; fix access and retry",
                path.display()
            ),
        )),
    }
}

fn preflight_error(code: &'static str, message: impl Into<String>) -> AggregateRootPreflightError {
    AggregateRootPreflightError {
        code,
        message: message.into(),
    }
}

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
    #[cfg(unix)]
    use std::os::unix::fs::symlink;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::{
        LogicalCodebaseFeature, LogicalCodebaseManifest, LogicalCodebaseStore,
    };
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

    #[test]
    #[cfg(unix)]
    fn aggregate_root_preflight_rejects_git_root_symlink_escape_and_ownership_conflicts() {
        let fixture = aggregate_root_fixture();
        fixture.init_git_at_root();
        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.member),
                )
                .unwrap_err()
                .code(),
            "aggregate_root_is_git"
        );

        fixture.remove_root_git();
        fixture.create_symlink_member_to_outside_root();
        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.symlink_member),
                )
                .unwrap_err()
                .code(),
            "member_symlink_escape"
        );

        fixture.create_file("CLAUDE.md", "user instructions");
        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.member),
                )
                .unwrap_err()
                .code(),
            "aggregate_root_ownership_conflict"
        );
    }

    #[test]
    #[cfg(unix)]
    fn aggregate_root_preflight_rejects_members_outside_root_nested_worktrees_and_overlap() {
        let fixture = aggregate_root_fixture();
        let outside_member = fixture.root.parent().unwrap().join("outside-member");
        fs::create_dir(&outside_member).unwrap();
        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&outside_member),
                )
                .unwrap_err()
                .code(),
            "member_path_outside_root"
        );
        assert!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.root),
                )
                .is_err_and(|error| error.code() == "member_path_outside_root")
        );

        fixture.init_git_at_member();
        fixture.create_nested_worktree_at_member();
        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.nested_worktree_member),
                )
                .unwrap_err()
                .code(),
            "nested_worktree"
        );

        fixture.remove_nested_worktree();
        fixture.save_manifest_for("project_0002", fixture.root.parent().unwrap());
        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.member),
                )
                .unwrap_err()
                .code(),
            "aggregate_root_overlap"
        );
    }

    #[test]
    #[cfg(unix)]
    fn aggregate_root_preflight_accepts_non_git_root_with_direct_git_members() {
        let fixture = aggregate_root_fixture();
        fixture.init_git_at_member();

        assert_eq!(
            fixture
                .preflight()
                .validate(
                    "project_0001",
                    &fixture.root,
                    std::slice::from_ref(&fixture.member),
                )
                .unwrap()
                .canonical_path,
            fs::canonicalize(&fixture.root).unwrap()
        );
    }

    struct AggregateRootFixture {
        _temp: tempfile::TempDir,
        paths: ProductAppPaths,
        root: PathBuf,
        member: PathBuf,
        symlink_member: PathBuf,
        nested_worktree_member: PathBuf,
    }

    impl AggregateRootFixture {
        fn preflight(&self) -> AggregateRootPreflight {
            AggregateRootPreflight::new(self.paths.clone())
        }

        fn create_file(&self, relative_path: &str, contents: &str) {
            fs::write(self.root.join(relative_path), contents).unwrap();
        }

        fn init_git_at_root(&self) {
            git(&self.root, &["init", "--quiet"]);
        }

        fn remove_root_git(&self) {
            fs::remove_dir_all(self.root.join(".git")).unwrap();
        }

        fn init_git_at_member(&self) {
            git(&self.member, &["init", "--quiet"]);
            git(
                &self.member,
                &["config", "user.email", "aria@example.invalid"],
            );
            git(&self.member, &["config", "user.name", "Aria Test"]);
            fs::write(self.member.join("README.md"), "# Member\n").unwrap();
            git(&self.member, &["add", "README.md"]);
            git(&self.member, &["commit", "--quiet", "-m", "initial"]);
        }

        fn create_symlink_member_to_outside_root(&self) {
            let outside_member = self.root.parent().unwrap().join("outside-symlink-target");
            fs::create_dir(&outside_member).unwrap();
            symlink(&outside_member, &self.symlink_member).unwrap();
        }

        fn create_nested_worktree_at_member(&self) {
            git(
                &self.member,
                &[
                    "worktree",
                    "add",
                    "--detach",
                    self.nested_worktree_member.to_str().unwrap(),
                ],
            );
        }

        fn remove_nested_worktree(&self) {
            git(
                &self.member,
                &[
                    "worktree",
                    "remove",
                    "--force",
                    self.nested_worktree_member.to_str().unwrap(),
                ],
            );
        }

        fn save_manifest_for(&self, project_id: &str, root: &Path) {
            let manifest = LogicalCodebaseManifest::new(
                project_id,
                fs::canonicalize(root).unwrap(),
                Vec::new(),
            );
            LogicalCodebaseStore::new(self.paths.clone())
                .save_manifest(project_id, &manifest)
                .unwrap();
        }
    }

    fn aggregate_root_fixture() -> AggregateRootFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path().join("aria-state"));
        let root = temp.path().join("aggregate-root");
        let member = root.join("member");
        let symlink_member = root.join("symlink-member");
        let nested_worktree_member = member.join("nested-worktree");
        fs::create_dir_all(&member).unwrap();
        for name in ["project", "project-other"] {
            ProjectStore::new(paths.clone())
                .create(CreateProjectInput {
                    name: name.to_string(),
                    description: None,
                })
                .unwrap();
        }

        AggregateRootFixture {
            _temp: temp,
            paths,
            root,
            member,
            symlink_member,
            nested_worktree_member,
        }
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
