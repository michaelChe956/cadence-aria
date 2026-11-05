use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::product::models::RepositoryRecord;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryRegistrationInput {
    pub project_id: String,
    pub name: String,
    pub path: PathBuf,
    pub default_policy_preset: Option<String>,
    pub default_provider_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryInitializationCommandSummary {
    pub command_index: usize,
    pub command: String,
    pub status: String,
    pub output_summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CadenceSkillsPreparationSummary {
    pub source_mode: String,
    pub source_root: PathBuf,
    pub skills_root: PathBuf,
    pub git_updated: bool,
    pub link_sync_status: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryInitializationSummary {
    pub provider: String,
    pub source: PathBuf,
    pub source_mode: String,
    pub skills_root: PathBuf,
    pub git_updated: bool,
    pub link_sync_status: String,
    pub commands: Vec<RepositoryInitializationCommandSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryRegistrationSuccess {
    pub repository: RepositoryRecord,
    pub cadence_skills: CadenceSkillsPreparationSummary,
    pub initialization: RepositoryInitializationSummary,
    pub warnings: Vec<String>,
    pub changed_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_finalize_warning: Option<String>,
    pub completed_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryRegistrationError {
    #[serde(flatten)]
    details: Box<RepositoryRegistrationErrorDetails>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryRegistrationErrorDetails {
    pub stage: String,
    pub provider: Option<String>,
    pub command_index: Option<usize>,
    pub command: Option<String>,
    pub reason_code: String,
    pub stderr_summary: Option<String>,
    pub changed_paths: Option<Vec<String>>,
    pub retryable: bool,
    pub action: String,
}

impl std::ops::Deref for RepositoryRegistrationError {
    type Target = RepositoryRegistrationErrorDetails;

    fn deref(&self) -> &Self::Target {
        &self.details
    }
}

impl std::ops::DerefMut for RepositoryRegistrationError {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.details
    }
}

impl std::fmt::Display for RepositoryRegistrationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} at {}: {:?}; action: {}",
            self.reason_code, self.stage, self.stderr_summary, self.action
        )
    }
}

impl std::error::Error for RepositoryRegistrationError {}

impl From<RepositoryRegistrationErrorDetails> for RepositoryRegistrationError {
    fn from(details: RepositoryRegistrationErrorDetails) -> Self {
        Self {
            details: Box::new(details),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryInitializationOperationStatus {
    Created,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryInitializationStepKind {
    CadenceSkills,
    PreCheck,
    RuleConfig,
    McpConfiguration,
    ProjectRulesExamples,
    GitFinalize,
}

impl RepositoryInitializationStepKind {
    pub const ALL: [Self; 6] = [
        Self::CadenceSkills,
        Self::PreCheck,
        Self::RuleConfig,
        Self::McpConfiguration,
        Self::ProjectRulesExamples,
        Self::GitFinalize,
    ];

    pub fn command(self) -> Option<&'static str> {
        match self {
            Self::CadenceSkills | Self::GitFinalize => None,
            Self::PreCheck => Some("/pre-check --no-interrupt --upgrade 用大陆镜像"),
            Self::RuleConfig => Some("/rule-config --no-interrupt"),
            Self::McpConfiguration => Some("/mcp-configuration --no-interrupt"),
            Self::ProjectRulesExamples => Some("/project-rules-examples --no-interrupt"),
        }
    }

    /// provider 的 skill 调用前缀（REQ-BOOT-16）：**逐字固定**，禁止运行时
    /// 拼装其它形式。
    ///
    /// - ClaudeCode → `/`
    /// - Pi → `/skill:`
    /// - KimiCode → `/skill:`
    /// - Codex → `$cadence-init:`
    pub fn skill_command_prefix(
        provider: crate::product::models::ProviderName,
    ) -> Option<&'static str> {
        use crate::product::models::ProviderName;
        match provider {
            ProviderName::ClaudeCode => Some("/"),
            ProviderName::Pi => Some("/skill:"),
            ProviderName::KimiCode => Some("/skill:"),
            ProviderName::Codex => Some("$cadence-init:"),
            ProviderName::Fake => None,
        }
    }

    /// provider-aware 命令生成（**唯一运行时命令来源**，REQ-BOOT-16）。
    ///
    /// 完整形态 = 前缀 + 命令名 + `" "` + 原 args（args 逐字不变）。
    /// `command()` 保留 Claude 形态的裸命令文本（命令名 + args），供本函数
    /// 取用命令名/args 段。
    pub fn command_for(self, provider: crate::product::models::ProviderName) -> Option<String> {
        let prefix = Self::skill_command_prefix(provider)?;
        let command = self.command()?;
        Some(format!("{prefix}{}", command.trim_start_matches('/')))
    }

    pub fn from_command_index(command_index: usize) -> Option<Self> {
        [
            Self::PreCheck,
            Self::RuleConfig,
            Self::McpConfiguration,
            Self::ProjectRulesExamples,
        ]
        .get(command_index.checked_sub(1)?)
        .copied()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryInitializationStepStatus {
    Pending,
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryInitializationStepRecord {
    pub step_id: RepositoryInitializationStepKind,
    pub status: RepositoryInitializationStepStatus,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryInitializationOperationInput {
    pub name: String,
    pub git_root: PathBuf,
    pub default_policy_preset: Option<String>,
    pub default_provider_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryInitializationOperation {
    pub operation_id: String,
    pub project_id: String,
    pub input: RepositoryInitializationOperationInput,
    pub status: RepositoryInitializationOperationStatus,
    pub steps: Vec<RepositoryInitializationStepRecord>,
    pub failed_step: Option<RepositoryInitializationStepKind>,
    pub result: Option<RepositoryRegistrationSuccess>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_finalize_checkpoint: Option<RepositoryRegistrationSuccess>,
    pub error: Option<RepositoryRegistrationError>,
    /// C5 Task 6（REQ-INIT-C5-RESUME）additive durable linkage：resume 产生的
    /// 后继 operation 指回原 Failed operation；旧 JSON 缺字段视为无 parent。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_operation_id: Option<String>,
    /// 触发本次 resume 的用户显式 command_id（与确定性 id 派生同源）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_command_id: Option<String>,
    /// 原 Failed operation 被哪个后继接续（投影按 parent→后继链消隐）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

impl RepositoryInitializationOperation {
    pub fn new(
        operation_id: String,
        project_id: String,
        input: RepositoryInitializationOperationInput,
        created_at: String,
    ) -> Self {
        Self {
            operation_id,
            project_id,
            input,
            status: RepositoryInitializationOperationStatus::Created,
            steps: RepositoryInitializationStepKind::ALL
                .iter()
                .copied()
                .map(|step_id| RepositoryInitializationStepRecord {
                    step_id,
                    status: RepositoryInitializationStepStatus::Pending,
                    started_at: None,
                    completed_at: None,
                })
                .collect(),
            failed_step: None,
            result: None,
            git_finalize_checkpoint: None,
            error: None,
            parent_operation_id: None,
            resume_command_id: None,
            superseded_by: None,
            updated_at: created_at.clone(),
            created_at,
            completed_at: None,
        }
    }
}

pub trait RepositoryInitializationProgress: Send + Sync {
    fn step_started(
        &self,
        step: RepositoryInitializationStepKind,
    ) -> Result<(), Box<RepositoryRegistrationError>>;

    fn step_completed(
        &self,
        step: RepositoryInitializationStepKind,
    ) -> Result<(), Box<RepositoryRegistrationError>>;
}

impl RepositoryRegistrationError {
    pub(crate) fn into_details(self) -> RepositoryRegistrationErrorDetails {
        *self.details
    }

    pub(crate) fn new(
        stage: impl Into<String>,
        reason_code: impl Into<String>,
        stderr_summary: Option<String>,
        retryable: bool,
        action: impl Into<String>,
    ) -> Self {
        Self {
            details: Box::new(RepositoryRegistrationErrorDetails {
                stage: stage.into(),
                provider: None,
                command_index: None,
                command: None,
                reason_code: reason_code.into(),
                stderr_summary,
                changed_paths: None,
                retryable,
                action: action.into(),
            }),
        }
    }

    pub(crate) fn for_command(
        stage: impl Into<String>,
        reason_code: impl Into<String>,
        command_index: usize,
        command: &str,
        stderr_summary: Option<String>,
        retryable: bool,
        action: impl Into<String>,
    ) -> Self {
        let mut error = Self::new(stage, reason_code, stderr_summary, retryable, action);
        error.provider = Some("claude_code".to_string());
        error.command_index = Some(command_index);
        error.command = Some(command.to_string());
        error
    }
}

#[cfg(test)]
mod tests {
    use super::RepositoryInitializationStepKind as Step;
    use crate::product::models::ProviderName;

    /// REQ-BOOT-16：四条命令 × 四家 provider = 16 个形态**逐字断言**。
    ///
    /// 完整形态 = 前缀 + 命令名 + `" "` + 原 args（args 逐字不变）。
    #[test]
    fn command_for_generates_provider_skill_syntax_verbatim() {
        let cases: Vec<(ProviderName, Step, &str)> = vec![
            (
                ProviderName::ClaudeCode,
                Step::PreCheck,
                "/pre-check --no-interrupt --upgrade 用大陆镜像",
            ),
            (
                ProviderName::Pi,
                Step::PreCheck,
                "/skill:pre-check --no-interrupt --upgrade 用大陆镜像",
            ),
            (
                ProviderName::KimiCode,
                Step::PreCheck,
                "/skill:pre-check --no-interrupt --upgrade 用大陆镜像",
            ),
            (
                ProviderName::Codex,
                Step::PreCheck,
                "$cadence-init:pre-check --no-interrupt --upgrade 用大陆镜像",
            ),
            (
                ProviderName::ClaudeCode,
                Step::RuleConfig,
                "/rule-config --no-interrupt",
            ),
            (
                ProviderName::Pi,
                Step::RuleConfig,
                "/skill:rule-config --no-interrupt",
            ),
            (
                ProviderName::KimiCode,
                Step::RuleConfig,
                "/skill:rule-config --no-interrupt",
            ),
            (
                ProviderName::Codex,
                Step::RuleConfig,
                "$cadence-init:rule-config --no-interrupt",
            ),
            (
                ProviderName::ClaudeCode,
                Step::McpConfiguration,
                "/mcp-configuration --no-interrupt",
            ),
            (
                ProviderName::Pi,
                Step::McpConfiguration,
                "/skill:mcp-configuration --no-interrupt",
            ),
            (
                ProviderName::KimiCode,
                Step::McpConfiguration,
                "/skill:mcp-configuration --no-interrupt",
            ),
            (
                ProviderName::Codex,
                Step::McpConfiguration,
                "$cadence-init:mcp-configuration --no-interrupt",
            ),
            (
                ProviderName::ClaudeCode,
                Step::ProjectRulesExamples,
                "/project-rules-examples --no-interrupt",
            ),
            (
                ProviderName::Pi,
                Step::ProjectRulesExamples,
                "/skill:project-rules-examples --no-interrupt",
            ),
            (
                ProviderName::KimiCode,
                Step::ProjectRulesExamples,
                "/skill:project-rules-examples --no-interrupt",
            ),
            (
                ProviderName::Codex,
                Step::ProjectRulesExamples,
                "$cadence-init:project-rules-examples --no-interrupt",
            ),
        ];
        for (provider, step, expected) in cases {
            assert_eq!(
                step.command_for(provider.clone()).as_deref(),
                Some(expected),
                "{provider:?} / {step:?} 形态必须逐字一致"
            );
        }
    }

    /// 同一 (provider, step) 两次调用结果相同（纯函数、确定性）。
    #[test]
    fn command_for_is_deterministic() {
        for provider in [
            ProviderName::ClaudeCode,
            ProviderName::Pi,
            ProviderName::KimiCode,
            ProviderName::Codex,
        ] {
            for step in [
                Step::PreCheck,
                Step::RuleConfig,
                Step::McpConfiguration,
                Step::ProjectRulesExamples,
            ] {
                assert_eq!(
                    step.command_for(provider.clone()),
                    step.command_for(provider.clone()),
                    "{provider:?} / {step:?} 必须确定性"
                );
            }
        }
    }

    /// 四家之间**只有前缀不同**（命令名与 args 部分相同）。
    #[test]
    fn command_for_differs_only_by_prefix_across_providers() {
        for step in [
            Step::PreCheck,
            Step::RuleConfig,
            Step::McpConfiguration,
            Step::ProjectRulesExamples,
        ] {
            let claude = step
                .command_for(ProviderName::ClaudeCode)
                .expect("claude command");
            let pi = step.command_for(ProviderName::Pi).expect("pi command");
            let kimi = step
                .command_for(ProviderName::KimiCode)
                .expect("kimi command");
            let codex = step
                .command_for(ProviderName::Codex)
                .expect("codex command");
            let claude_tail = claude.strip_prefix('/').expect("claude tail");
            assert_eq!(pi.strip_prefix("/skill:").expect("pi tail"), claude_tail);
            assert_eq!(
                kimi.strip_prefix("/skill:").expect("kimi tail"),
                claude_tail
            );
            assert_eq!(
                codex.strip_prefix("$cadence-init:").expect("codex tail"),
                claude_tail
            );
        }
    }

    /// Fake 无 skill 调用通道 ⇒ 无命令（不冒充探测）。
    #[test]
    fn command_for_is_none_for_fake_provider() {
        assert!(Step::PreCheck.command_for(ProviderName::Fake).is_none());
        assert!(Step::skill_command_prefix(ProviderName::Fake).is_none());
    }
}
