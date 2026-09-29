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

/// C2 Task 5（REQ-CRO-05）reviewer 三值：effective 缺失即 None，MUST NOT 回填 author。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodingReviewerConfig {
    /// 实际生效的 reviewer；None=缺失。
    pub effective: Option<ProviderName>,
    /// 用户已选未生效的 reviewer（plan 会话侧 provisional 恢复闭环）。
    pub provisional: Option<ProviderName>,
    /// disabled 保持 disabled。
    pub enabled: bool,
}

impl CodingReviewerConfig {
    pub fn from_parts(
        effective: Option<ProviderName>,
        provisional: Option<ProviderName>,
        enabled: bool,
    ) -> Self {
        Self {
            effective,
            provisional,
            enabled,
        }
    }

    /// 缺失即未启用（空 effective）。
    pub fn missing() -> Self {
        Self::from_parts(None, None, false)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodingRoleProviderConfigSnapshot {
    pub coder: ProviderName,
    /// C2 Task 5：三值字段——旧 JSON 缺字段按空 effective（None）解释，绝不回填 author。
    #[serde(default)]
    pub code_reviewer: Option<ProviderName>,
    /// C2 Task 5：三值字段——同上。
    #[serde(default)]
    pub internal_reviewer: Option<ProviderName>,
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
        // C2 Task 5（REQ-CRO-05）：reviewer 缺失保持空 effective，MUST NOT 回填 author。
        Self {
            coder: snapshot.author.clone(),
            code_reviewer: snapshot.reviewer.clone(),
            internal_reviewer: snapshot.reviewer.clone(),
            review_rounds: snapshot.review_rounds,
            permission_modes: CodingRolePermissionModes::default(),
        }
    }
}

impl CodingRoleProviderConfigSnapshot {
    /// C2 Task 5：reviewer 角色缺失返回 None（空 effective），调用方必须显式消费三值。
    pub fn provider_for_role(&self, role: &CodingProviderRole) -> Option<&ProviderName> {
        match role {
            CodingProviderRole::Coder => Some(&self.coder),
            CodingProviderRole::CodeReviewer => self.code_reviewer.as_ref(),
            CodingProviderRole::InternalReviewer => self.internal_reviewer.as_ref(),
        }
    }

    /// reviewer 三值投影（C2 Task 5 统一契约）：effective 取 code reviewer 侧；
    /// provisional 由 plan 会话侧补充；enabled=effective 在场。
    pub fn reviewer_config(&self) -> CodingReviewerConfig {
        CodingReviewerConfig::from_parts(
            self.code_reviewer.clone(),
            None,
            self.code_reviewer.is_some(),
        )
    }

    /// internal reviewer 侧三值投影（group final review 用）。
    pub fn internal_reviewer_config(&self) -> CodingReviewerConfig {
        CodingReviewerConfig::from_parts(
            self.internal_reviewer.clone(),
            None,
            self.internal_reviewer.is_some(),
        )
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
            CodingProviderRole::CodeReviewer => self.code_reviewer = Some(provider),
            CodingProviderRole::InternalReviewer => self.internal_reviewer = Some(provider),
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

    // C2 Task 5（REQ-CRO-05）：reviewer 三值。

    #[test]
    fn legacy_role_snapshot_without_reviewer_fields_deserializes_empty_effective() {
        // 旧 role-provider-config.json 缺三值字段：按空 effective 解释，读取不失败、不回填。
        let json = serde_json::json!({
            "coder": "claude_code", "review_rounds": 1
        });
        let snapshot: CodingRoleProviderConfigSnapshot = serde_json::from_value(json).unwrap();
        assert_eq!(snapshot.code_reviewer, None);
        assert_eq!(snapshot.internal_reviewer, None);
    }

    #[test]
    fn from_legacy_snapshot_without_reviewer_keeps_reviewers_empty() {
        let snapshot = CodingRoleProviderConfigSnapshot::from(&ProviderConfigSnapshot {
            author: ProviderName::ClaudeCode,
            reviewer: None,
            review_rounds: 1,
            permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
        });
        // coder=author 是 coder 自身语义；reviewer 绝不被 author 顶替。
        assert_eq!(snapshot.coder, ProviderName::ClaudeCode);
        assert_eq!(snapshot.code_reviewer, None);
        assert_eq!(snapshot.internal_reviewer, None);
        let reviewer_config = snapshot.reviewer_config();
        assert_eq!(reviewer_config.effective, None);
        assert_eq!(reviewer_config.provisional, None);
        assert!(!reviewer_config.enabled);
    }
}
