use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CadenceSkillsSourceMode {
    OnlineClone,
    OnlineUpdate,
    Offline,
}

impl CadenceSkillsSourceMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OnlineClone => "online_clone",
            Self::OnlineUpdate => "online_update",
            Self::Offline => "offline",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkSyncStatus {
    Synchronized,
}

impl LinkSyncStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Synchronized => "synchronized",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CadenceSkillsPreparationResult {
    pub source_mode: CadenceSkillsSourceMode,
    pub source_root: PathBuf,
    pub skills_root: PathBuf,
    pub git_updated: bool,
    pub link_sync_status: LinkSyncStatus,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CadenceSkillsError {
    #[error("cadence_skills_unavailable at {stage}: {reason}; action: {action}")]
    Unavailable {
        stage: String,
        reason: String,
        action: String,
    },
    #[error("cadence_skills_update_failed at {stage}: {reason}; action: {action}")]
    UpdateFailed {
        stage: String,
        reason: String,
        action: String,
    },
    #[error("cadence_skills_sync_failed at layer {layer} for skill {skill} ({path}): {reason}")]
    SyncFailed {
        layer: String,
        skill: String,
        path: String,
        reason: String,
    },
}

impl CadenceSkillsError {
    pub fn unavailable(stage: &str, reason: &str, action: &str) -> Self {
        Self::Unavailable {
            stage: stage.to_string(),
            reason: reason.to_string(),
            action: action.to_string(),
        }
    }

    pub fn update_failed(stage: &str, reason: &str, action: &str) -> Self {
        Self::UpdateFailed {
            stage: stage.to_string(),
            reason: reason.to_string(),
            action: action.to_string(),
        }
    }

    pub fn sync_failed(
        layer: &str,
        skill: &str,
        path: impl AsRef<std::path::Path>,
        reason: &str,
    ) -> Self {
        Self::SyncFailed {
            layer: layer.to_string(),
            skill: skill.to_string(),
            path: path.as_ref().to_string_lossy().into_owned(),
            reason: reason.to_string(),
        }
    }

    pub const fn code(&self) -> &'static str {
        match self {
            Self::Unavailable { .. } => "cadence_skills_unavailable",
            Self::UpdateFailed { .. } => "cadence_skills_update_failed",
            Self::SyncFailed { .. } => "cadence_skills_sync_failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CadenceSkillsError, CadenceSkillsSourceMode, LinkSyncStatus};

    #[test]
    fn public_result_enums_have_stable_contract_values() {
        assert_eq!(
            CadenceSkillsSourceMode::OnlineClone.as_str(),
            "online_clone"
        );
        assert_eq!(
            CadenceSkillsSourceMode::OnlineUpdate.as_str(),
            "online_update"
        );
        assert_eq!(CadenceSkillsSourceMode::Offline.as_str(), "offline");
        assert_eq!(LinkSyncStatus::Synchronized.as_str(), "synchronized");
    }

    #[test]
    fn blocking_errors_expose_stable_codes() {
        assert_eq!(
            CadenceSkillsError::unavailable("clone", "network", "install offline").code(),
            "cadence_skills_unavailable"
        );
        assert_eq!(
            CadenceSkillsError::update_failed("fetch", "network", "retry").code(),
            "cadence_skills_update_failed"
        );
        assert_eq!(
            CadenceSkillsError::sync_failed("shared", "demo", "/tmp/demo", "denied").code(),
            "cadence_skills_sync_failed"
        );
    }
}
