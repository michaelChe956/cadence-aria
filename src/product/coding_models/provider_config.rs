use std::fmt;

use serde::{Deserialize, Serialize};

use crate::product::models::ProviderName;
use crate::web::workspace_ws_types::ProviderConfigSnapshot;

use super::execution::CodingProviderRole;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodingProviderPermissionMode {
    Auto,
    Supervised,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingRolePermissionModes {
    pub coder: CodingProviderPermissionMode,
    pub code_reviewer: CodingProviderPermissionMode,
    pub internal_reviewer: CodingProviderPermissionMode,
}

impl Default for CodingRolePermissionModes {
    fn default() -> Self {
        Self {
            coder: CodingProviderPermissionMode::Auto,
            code_reviewer: CodingProviderPermissionMode::Auto,
            internal_reviewer: CodingProviderPermissionMode::Auto,
        }
    }
}

impl fmt::Display for CodingProviderRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Coder => "Coder",
            Self::CodeReviewer => "Code Reviewer",
            Self::InternalReviewer => "Internal Reviewer",
        };
        formatter.write_str(label)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingRoleProviderConfigSnapshot {
    pub coder: ProviderName,
    pub code_reviewer: ProviderName,
    pub internal_reviewer: ProviderName,
    pub review_rounds: u32,
    #[serde(default)]
    pub permission_modes: CodingRolePermissionModes,
}

impl From<ProviderConfigSnapshot> for CodingRoleProviderConfigSnapshot {
    fn from(snapshot: ProviderConfigSnapshot) -> Self {
        Self::from(&snapshot)
    }
}

impl From<&ProviderConfigSnapshot> for CodingRoleProviderConfigSnapshot {
    fn from(snapshot: &ProviderConfigSnapshot) -> Self {
        let reviewer = snapshot
            .reviewer
            .clone()
            .unwrap_or_else(|| snapshot.author.clone());
        Self {
            coder: snapshot.author.clone(),
            code_reviewer: reviewer.clone(),
            internal_reviewer: reviewer,
            review_rounds: snapshot.review_rounds,
            permission_modes: CodingRolePermissionModes::default(),
        }
    }
}

impl CodingRoleProviderConfigSnapshot {
    pub fn provider_for_role(&self, role: &CodingProviderRole) -> &ProviderName {
        match role {
            CodingProviderRole::Coder => &self.coder,
            CodingProviderRole::CodeReviewer => &self.code_reviewer,
            CodingProviderRole::InternalReviewer => &self.internal_reviewer,
        }
    }

    pub fn permission_mode_for_role(
        &self,
        role: &CodingProviderRole,
    ) -> CodingProviderPermissionMode {
        match role {
            CodingProviderRole::Coder => self.permission_modes.coder,
            CodingProviderRole::CodeReviewer => self.permission_modes.code_reviewer,
            CodingProviderRole::InternalReviewer => self.permission_modes.internal_reviewer,
        }
    }

    pub fn set_provider_for_role(&mut self, role: &CodingProviderRole, provider: ProviderName) {
        match role {
            CodingProviderRole::Coder => self.coder = provider,
            CodingProviderRole::CodeReviewer => self.code_reviewer = provider,
            CodingProviderRole::InternalReviewer => self.internal_reviewer = provider,
        }
    }

    pub fn set_permission_mode_for_role(
        &mut self,
        role: &CodingProviderRole,
        mode: CodingProviderPermissionMode,
    ) {
        match role {
            CodingProviderRole::Coder => self.permission_modes.coder = mode,
            CodingProviderRole::CodeReviewer => self.permission_modes.code_reviewer = mode,
            CodingProviderRole::InternalReviewer => self.permission_modes.internal_reviewer = mode,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coding_role_permission_modes_default_is_auto() {
        let modes = CodingRolePermissionModes::default();
        assert_eq!(modes.coder, CodingProviderPermissionMode::Auto);
        assert_eq!(modes.code_reviewer, CodingProviderPermissionMode::Auto);
        assert_eq!(modes.internal_reviewer, CodingProviderPermissionMode::Auto);
    }

    #[test]
    fn explicit_supervised_value_preserved() {
        let json = serde_json::json!({
            "coder": "supervised", "code_reviewer": "supervised", "internal_reviewer": "supervised"
        });
        let modes: CodingRolePermissionModes = serde_json::from_value(json).unwrap();
        assert_eq!(modes.coder, CodingProviderPermissionMode::Supervised);
        assert_eq!(
            modes.code_reviewer,
            CodingProviderPermissionMode::Supervised
        );
    }

    #[test]
    fn old_coding_snapshot_without_permission_modes_deserializes_to_auto() {
        // CodingRoleProviderConfigSnapshot.permission_modes uses #[serde(default)]; absent fields use Auto.
        let json = serde_json::json!({
            "coder": "claude_code", "code_reviewer": "codex",
            "internal_reviewer": "claude_code", "review_rounds": 1
        });
        let snapshot: CodingRoleProviderConfigSnapshot = serde_json::from_value(json).unwrap();
        assert_eq!(
            snapshot.permission_modes.coder,
            CodingProviderPermissionMode::Auto
        );
    }
}
