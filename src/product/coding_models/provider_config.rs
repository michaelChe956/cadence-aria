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
    pub tester: CodingProviderPermissionMode,
    pub analyst: CodingProviderPermissionMode,
    pub code_reviewer: CodingProviderPermissionMode,
    pub internal_reviewer: CodingProviderPermissionMode,
}

impl Default for CodingRolePermissionModes {
    fn default() -> Self {
        Self {
            coder: CodingProviderPermissionMode::Supervised,
            tester: CodingProviderPermissionMode::Auto,
            analyst: CodingProviderPermissionMode::Auto,
            code_reviewer: CodingProviderPermissionMode::Supervised,
            internal_reviewer: CodingProviderPermissionMode::Supervised,
        }
    }
}

impl fmt::Display for CodingProviderRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Coder => "Coder",
            Self::Tester => "Tester",
            Self::Analyst => "Analyst",
            Self::CodeReviewer => "Code Reviewer",
            Self::InternalReviewer => "Internal Reviewer",
        };
        formatter.write_str(label)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingRoleProviderConfigSnapshot {
    pub coder: ProviderName,
    pub tester: ProviderName,
    pub analyst: ProviderName,
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
            tester: snapshot.author.clone(),
            analyst: snapshot.author.clone(),
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
            CodingProviderRole::Tester => &self.tester,
            CodingProviderRole::Analyst => &self.analyst,
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
            CodingProviderRole::Tester => self.permission_modes.tester,
            CodingProviderRole::Analyst => self.permission_modes.analyst,
            CodingProviderRole::CodeReviewer => self.permission_modes.code_reviewer,
            CodingProviderRole::InternalReviewer => self.permission_modes.internal_reviewer,
        }
    }

    pub fn set_provider_for_role(&mut self, role: &CodingProviderRole, provider: ProviderName) {
        match role {
            CodingProviderRole::Coder => self.coder = provider,
            CodingProviderRole::Tester => self.tester = provider,
            CodingProviderRole::Analyst => self.analyst = provider,
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
            CodingProviderRole::Tester => self.permission_modes.tester = mode,
            CodingProviderRole::Analyst => self.permission_modes.analyst = mode,
            CodingProviderRole::CodeReviewer => self.permission_modes.code_reviewer = mode,
            CodingProviderRole::InternalReviewer => self.permission_modes.internal_reviewer = mode,
        }
    }
}
