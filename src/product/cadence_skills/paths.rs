use std::path::{Component, Path, PathBuf};

use super::CadenceSkillsError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CadenceSkillsPaths {
    repository_root: PathBuf,
    source_root: PathBuf,
    shared_skills_root: PathBuf,
    codex_skills_root: PathBuf,
    claude_skills_root: PathBuf,
}

impl CadenceSkillsPaths {
    pub fn from_home(home: &Path) -> Self {
        let repository_root = home.join(".agents/Cadence-skills");
        Self {
            source_root: repository_root.join("cadence-init/skills"),
            repository_root,
            shared_skills_root: home.join(".agents/skills"),
            codex_skills_root: home.join(".codex/skills/skills"),
            claude_skills_root: home.join(".claude/skills"),
        }
    }

    pub fn repository_root(&self) -> &Path {
        &self.repository_root
    }

    pub fn source_root(&self) -> &Path {
        &self.source_root
    }

    pub fn shared_skills_root(&self) -> &Path {
        &self.shared_skills_root
    }

    pub fn codex_skills_root(&self) -> &Path {
        &self.codex_skills_root
    }

    pub fn claude_skills_root(&self) -> &Path {
        &self.claude_skills_root
    }

    pub fn skill_source(&self, skill: &str) -> Result<PathBuf, CadenceSkillsError> {
        validate_skill_name(skill)?;
        Ok(self.source_root.join(skill))
    }
}

pub(crate) fn validate_skill_name(skill: &str) -> Result<(), CadenceSkillsError> {
    let mut components = Path::new(skill).components();
    if matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none() {
        return Ok(());
    }
    Err(CadenceSkillsError::sync_failed(
        "source",
        skill,
        skill,
        "skill name must be one normal path component",
    ))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::CadenceSkillsPaths;

    #[test]
    fn injected_home_derives_the_five_fixed_roots() {
        let paths = CadenceSkillsPaths::from_home(Path::new("/tmp/cadence-home"));

        assert_eq!(
            paths.repository_root(),
            Path::new("/tmp/cadence-home/.agents/Cadence-skills")
        );
        assert_eq!(
            paths.source_root(),
            Path::new("/tmp/cadence-home/.agents/Cadence-skills/cadence-init/skills")
        );
        assert_eq!(
            paths.shared_skills_root(),
            Path::new("/tmp/cadence-home/.agents/skills")
        );
        assert_eq!(
            paths.codex_skills_root(),
            Path::new("/tmp/cadence-home/.codex/skills/skills")
        );
        assert_eq!(
            paths.claude_skills_root(),
            Path::new("/tmp/cadence-home/.claude/skills")
        );
    }

    #[test]
    fn skill_name_accepts_only_one_normal_path_component() {
        let paths = CadenceSkillsPaths::from_home(Path::new("/tmp/cadence-home"));

        assert_eq!(
            paths.skill_source("rust-helper").unwrap(),
            PathBuf::from(
                "/tmp/cadence-home/.agents/Cadence-skills/cadence-init/skills/rust-helper"
            )
        );
        for invalid in ["", ".", "..", "../escape", "nested/skill", "/absolute"] {
            assert!(paths.skill_source(invalid).is_err(), "accepted {invalid:?}");
        }
    }
}
