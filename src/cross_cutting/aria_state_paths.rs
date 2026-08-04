use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AriaStatePaths {
    workspace_root: PathBuf,
    aria_root: PathBuf,
}

impl AriaStatePaths {
    pub fn from_workspace_root(workspace_root: impl AsRef<Path>) -> Self {
        let workspace_root = workspace_root.as_ref().to_path_buf();
        let aria_root = if workspace_root
            .file_name()
            .is_some_and(|name| name == ".aria")
        {
            workspace_root.clone()
        } else {
            workspace_root.join(".aria")
        };
        Self {
            workspace_root,
            aria_root,
        }
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn aria_root(&self) -> &Path {
        &self.aria_root
    }

    pub fn provider_health_file(&self) -> PathBuf {
        self.aria_root.join("state/provider-health.json")
    }

    pub fn image_create_settings_file(&self) -> PathBuf {
        self.aria_root.join("image_create.json")
    }

    pub fn image_create_sessions_dir(&self) -> PathBuf {
        self.aria_root.join("image_create/sessions")
    }

    pub fn image_create_session_file(&self, id: &str) -> PathBuf {
        self.image_create_sessions_dir().join(format!("{id}.json"))
    }

    pub fn image_create_session_scratch_dir(&self, id: &str) -> PathBuf {
        self.aria_root.join("image_create/scratch").join(id)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::AriaStatePaths;

    #[test]
    fn aria_state_paths_resolves_product_home_root_once() {
        let home_root = PathBuf::from("/tmp/aria-product-home");

        let paths = AriaStatePaths::from_workspace_root(&home_root);

        assert_eq!(paths.workspace_root(), home_root.as_path());
        assert_eq!(paths.aria_root(), home_root.join(".aria"));
    }

    #[test]
    fn aria_state_paths_resolves_development_workspace_root_once() {
        let workspace_root = PathBuf::from("/tmp/cadence-aria-worktree");

        let paths = AriaStatePaths::from_workspace_root(&workspace_root);

        assert_eq!(paths.aria_root(), workspace_root.join(".aria"));
        assert_eq!(
            paths.provider_health_file(),
            workspace_root.join(".aria/state/provider-health.json")
        );
    }

    #[test]
    fn aria_state_paths_does_not_duplicate_existing_aria_root() {
        let aria_root = PathBuf::from("/tmp/cadence-aria-worktree/.aria");

        let paths = AriaStatePaths::from_workspace_root(&aria_root);

        assert_eq!(paths.aria_root(), aria_root);
        assert_eq!(
            paths.provider_health_file(),
            PathBuf::from("/tmp/cadence-aria-worktree/.aria/state/provider-health.json")
        );
    }

    #[test]
    fn aria_state_paths_resolves_image_create_storage_locations() {
        let workspace_root = PathBuf::from("/tmp/cadence-aria-worktree");
        let paths = AriaStatePaths::from_workspace_root(&workspace_root);

        assert_eq!(
            paths.image_create_settings_file(),
            workspace_root.join(".aria/image_create.json")
        );
        assert_eq!(
            paths.image_create_sessions_dir(),
            workspace_root.join(".aria/image_create/sessions")
        );
        assert_eq!(
            paths.image_create_session_file("session-1"),
            workspace_root.join(".aria/image_create/sessions/session-1.json")
        );
        assert_eq!(
            paths.image_create_session_scratch_dir("session-1"),
            workspace_root.join(".aria/image_create/scratch/session-1")
        );
    }
}
