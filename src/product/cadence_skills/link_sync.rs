use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::{CadenceSkillsError, CadenceSkillsPaths, LinkSyncStatus};
use crate::product::cadence_skills::paths::validate_skill_name;

static TEMP_LINK_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkSyncResult {
    pub status: LinkSyncStatus,
    pub changed_links: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ManagedSkillLinkSynchronizer {
    paths: CadenceSkillsPaths,
}

impl ManagedSkillLinkSynchronizer {
    pub fn new(paths: CadenceSkillsPaths) -> Self {
        Self { paths }
    }

    pub fn synchronize(&self) -> Result<LinkSyncResult, CadenceSkillsError> {
        #[cfg(not(unix))]
        return Err(CadenceSkillsError::sync_failed(
            "platform",
            "*",
            self.paths.source_root(),
            "Cadence skill symlink synchronization supports only macOS and Linux",
        ));

        #[cfg(unix)]
        self.synchronize_unix()
    }

    #[cfg(unix)]
    fn synchronize_unix(&self) -> Result<LinkSyncResult, CadenceSkillsError> {
        let mut entries = fs::read_dir(self.paths.source_root())
            .map_err(|error| self.sync_error("source", "*", self.paths.source_root(), error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| self.sync_error("source", "*", self.paths.source_root(), error))?;
        entries.sort_by_key(|entry| entry.file_name());

        self.create_layer_root("shared", self.paths.shared_skills_root())?;
        self.create_layer_root("codex", self.paths.codex_skills_root())?;
        self.create_layer_root("claude", self.paths.claude_skills_root())?;

        let mut result = LinkSyncResult {
            status: LinkSyncStatus::Synchronized,
            changed_links: 0,
            warnings: Vec::new(),
        };
        for entry in entries {
            let Some(skill) = entry.file_name().to_str().map(str::to_owned) else {
                return Err(CadenceSkillsError::sync_failed(
                    "source",
                    "<non-utf8>",
                    entry.path(),
                    "skill name is not valid UTF-8",
                ));
            };
            validate_skill_name(&skill)?;
            let source = self.paths.skill_source(&skill)?;
            let shared = self.paths.shared_skills_root().join(&skill);
            let shared_ready = self.sync_link(
                "shared",
                &skill,
                &shared,
                &source,
                std::slice::from_ref(&source),
                &mut result,
            )?;
            if !shared_ready {
                continue;
            }
            let managed_downstream = [shared.clone(), source];
            self.sync_link(
                "codex",
                &skill,
                &self.paths.codex_skills_root().join(&skill),
                &shared,
                &managed_downstream,
                &mut result,
            )?;
            self.sync_link(
                "claude",
                &skill,
                &self.paths.claude_skills_root().join(&skill),
                &shared,
                &managed_downstream,
                &mut result,
            )?;
        }
        Ok(result)
    }

    #[cfg(unix)]
    fn create_layer_root(&self, layer: &str, root: &Path) -> Result<(), CadenceSkillsError> {
        fs::create_dir_all(root).map_err(|error| self.sync_error(layer, "*", root, error))
    }

    #[cfg(unix)]
    fn sync_link(
        &self,
        layer: &str,
        skill: &str,
        link: &Path,
        expected: &Path,
        managed_targets: &[PathBuf],
        result: &mut LinkSyncResult,
    ) -> Result<bool, CadenceSkillsError> {
        match fs::symlink_metadata(link) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                let textual_target = fs::read_link(link)
                    .map_err(|error| self.sync_error(layer, skill, link, error))?;
                let resolved_target = resolve_link_target(link, &textual_target);
                if resolved_target == lexical_normalize(expected) {
                    return Ok(true);
                }
                if self.is_managed_target(skill, &resolved_target, managed_targets) {
                    self.atomic_replace_link(layer, skill, link, expected)?;
                    result.changed_links += 1;
                    return Ok(true);
                }
                result.warnings.push(format!(
                    "cadence_skills_conflict:{layer}:{skill}:unrelated_symlink:{}",
                    link.display()
                ));
                Ok(false)
            }
            Ok(_) => {
                result.warnings.push(format!(
                    "cadence_skills_conflict:{layer}:{skill}:user_content:{}",
                    link.display()
                ));
                Ok(false)
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                self.atomic_replace_link(layer, skill, link, expected)?;
                result.changed_links += 1;
                Ok(true)
            }
            Err(error) => Err(self.sync_error(layer, skill, link, error)),
        }
    }

    #[cfg(unix)]
    fn is_managed_target(
        &self,
        skill: &str,
        resolved_target: &Path,
        managed_targets: &[PathBuf],
    ) -> bool {
        if managed_targets
            .iter()
            .any(|target| lexical_normalize(target) == resolved_target)
        {
            return true;
        }
        resolved_target.starts_with(self.paths.repository_root())
            && resolved_target.file_name().and_then(|name| name.to_str()) == Some(skill)
            && resolved_target
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                == Some("skills")
    }

    #[cfg(unix)]
    fn atomic_replace_link(
        &self,
        layer: &str,
        skill: &str,
        link: &Path,
        expected: &Path,
    ) -> Result<(), CadenceSkillsError> {
        use std::os::unix::fs::symlink;

        let parent = link.parent().ok_or_else(|| {
            CadenceSkillsError::sync_failed(layer, skill, link, "link has no parent directory")
        })?;
        for _ in 0..100 {
            let sequence = TEMP_LINK_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let temporary = parent.join(format!(
                ".cadence-skills-{}-{sequence}.tmp",
                std::process::id()
            ));
            match symlink(expected, &temporary) {
                Ok(()) => {
                    if let Err(error) = fs::rename(&temporary, link) {
                        let _ = fs::remove_file(&temporary);
                        return Err(self.sync_error(layer, skill, link, error));
                    }
                    return Ok(());
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(self.sync_error(layer, skill, link, error)),
            }
        }
        Err(CadenceSkillsError::sync_failed(
            layer,
            skill,
            link,
            "could not allocate a unique temporary symlink",
        ))
    }

    fn sync_error(
        &self,
        layer: &str,
        skill: &str,
        path: &Path,
        error: std::io::Error,
    ) -> CadenceSkillsError {
        CadenceSkillsError::sync_failed(layer, skill, path, &sanitize_reason(&error.to_string()))
    }
}

fn resolve_link_target(link: &Path, textual_target: &Path) -> PathBuf {
    if textual_target.is_absolute() {
        lexical_normalize(textual_target)
    } else {
        lexical_normalize(
            &link
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(textual_target),
        )
    }
}

fn lexical_normalize(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() && !path.is_absolute() {
                    normalized.push("..");
                }
            }
            Component::RootDir | Component::Prefix(_) | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
        }
    }
    normalized
}

fn sanitize_reason(reason: &str) -> String {
    reason
        .chars()
        .filter(|character| !character.is_control())
        .take(512)
        .collect()
}

#[cfg(all(test, unix))]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    use std::os::unix::ffi::OsStringExt;
    use std::os::unix::fs::symlink;

    use tempfile::TempDir;

    use super::ManagedSkillLinkSynchronizer;
    use crate::product::cadence_skills::{CadenceSkillsPaths, LinkSyncStatus};

    fn fixture() -> (TempDir, CadenceSkillsPaths) {
        let home = TempDir::new().unwrap();
        let paths = CadenceSkillsPaths::from_home(home.path());
        fs::create_dir_all(paths.source_root()).unwrap();
        (home, paths)
    }

    fn add_skill(paths: &CadenceSkillsPaths, name: &str) {
        fs::create_dir_all(paths.source_root().join(name)).unwrap();
    }

    #[test]
    fn synchronizes_all_three_layers_and_is_idempotent() {
        let (_home, paths) = fixture();
        add_skill(&paths, "demo");
        let synchronizer = ManagedSkillLinkSynchronizer::new(paths.clone());

        let first = synchronizer.synchronize().unwrap();
        let second = synchronizer.synchronize().unwrap();

        assert_eq!(first.status, LinkSyncStatus::Synchronized);
        assert_eq!(first.changed_links, 3);
        assert!(first.warnings.is_empty());
        assert_eq!(second.changed_links, 0);
        assert!(second.warnings.is_empty());
        assert_eq!(
            fs::read_link(paths.shared_skills_root().join("demo")).unwrap(),
            paths.source_root().join("demo")
        );
        assert_eq!(
            fs::read_link(paths.codex_skills_root().join("demo")).unwrap(),
            paths.shared_skills_root().join("demo")
        );
        assert_eq!(
            fs::read_link(paths.claude_skills_root().join("demo")).unwrap(),
            paths.shared_skills_root().join("demo")
        );
    }

    #[test]
    fn replaces_managed_old_and_dangling_links() {
        let (home, paths) = fixture();
        add_skill(&paths, "demo");
        fs::create_dir_all(paths.shared_skills_root()).unwrap();
        fs::create_dir_all(paths.codex_skills_root()).unwrap();
        fs::create_dir_all(paths.claude_skills_root()).unwrap();
        symlink(
            paths.repository_root().join("legacy/skills/demo"),
            paths.shared_skills_root().join("demo"),
        )
        .unwrap();
        symlink(
            paths.source_root().join("demo"),
            paths.codex_skills_root().join("demo"),
        )
        .unwrap();
        symlink(
            home.path().join(".agents/skills/demo"),
            paths.claude_skills_root().join("demo"),
        )
        .unwrap();

        let result = ManagedSkillLinkSynchronizer::new(paths.clone())
            .synchronize()
            .unwrap();

        assert_eq!(result.changed_links, 2);
        assert_eq!(
            fs::read_link(paths.codex_skills_root().join("demo")).unwrap(),
            paths.shared_skills_root().join("demo")
        );
        assert_eq!(
            fs::read_link(paths.claude_skills_root().join("demo")).unwrap(),
            paths.shared_skills_root().join("demo")
        );
    }

    #[test]
    fn preserves_user_content_and_unrelated_links_with_warnings() {
        let (home, paths) = fixture();
        for skill in ["file", "directory", "valid-link", "broken-link"] {
            add_skill(&paths, skill);
        }
        fs::create_dir_all(paths.shared_skills_root()).unwrap();
        fs::write(paths.shared_skills_root().join("file"), "user").unwrap();
        fs::create_dir(paths.shared_skills_root().join("directory")).unwrap();
        let external = home.path().join("external");
        fs::create_dir(&external).unwrap();
        symlink(&external, paths.shared_skills_root().join("valid-link")).unwrap();
        symlink(
            home.path().join("missing-external"),
            paths.shared_skills_root().join("broken-link"),
        )
        .unwrap();

        let result = ManagedSkillLinkSynchronizer::new(paths.clone())
            .synchronize()
            .unwrap();

        assert_eq!(result.warnings.len(), 4);
        assert_eq!(
            fs::read_to_string(paths.shared_skills_root().join("file")).unwrap(),
            "user"
        );
        assert!(paths.shared_skills_root().join("directory").is_dir());
        assert_eq!(
            fs::read_link(paths.shared_skills_root().join("valid-link")).unwrap(),
            external
        );
        assert_eq!(
            fs::read_link(paths.shared_skills_root().join("broken-link")).unwrap(),
            home.path().join("missing-external")
        );
    }

    #[test]
    fn rejects_source_entries_that_are_not_valid_skill_names() {
        let (_home, paths) = fixture();
        let invalid = paths.source_root().join(OsString::from_vec(vec![0xff]));
        fs::create_dir_all(invalid).unwrap();

        let error = ManagedSkillLinkSynchronizer::new(paths)
            .synchronize()
            .unwrap_err();

        assert_eq!(error.code(), "cadence_skills_sync_failed");
    }
}
