use crate::cross_cutting::adapter_compatibility::{AdapterCompatibilityEntry, CommandSpec};
use crate::cross_cutting::cli_adapter::run_command_capture;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::protocol::contracts::ProviderType;
use crate::protocol::enums::ProviderCapabilityId;
use chrono::Utc;
use serde::{Deserialize, Serialize};

/// provider capability 的可审计三态证据。
///
/// 区分「探测确认支持」「探测确认不支持」与「未探测/未知」,避免只以
/// `supports_resume: bool` 单一维度判定能力——单布尔无法表达「未知」,
/// 会把「未探测」与「确认不支持」混为一谈。三态可持久化/审计,供 envelope
/// 与 fingerprint 复验 provider 真实能力。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum ProviderCapabilityEvidence {
    /// 探测确认该能力可用。
    Confirmed,
    /// 探测确认该能力不可用。
    Denied { reason: String },
    /// 未探测或探测结果不可靠,不能据此放行受保护操作。
    Unknown,
}

impl ProviderCapabilityEvidence {
    pub fn confirmed() -> Self {
        Self::Confirmed
    }

    pub fn denied(reason: impl Into<String>) -> Self {
        Self::Denied {
            reason: reason.into(),
        }
    }

    pub fn unknown() -> Self {
        Self::Unknown
    }
}

/// provider adapter 的已知 dialect 字符串,与
/// `product::logical_codebase::policy::ProviderDialect` 对应。gateway 在复验时
/// 比对该字符串,确保 envelope 冻结的 dialect 与实际 adapter 一致。
pub const ADAPTER_DIALECT_CLAUDE_CODE_CLI_V1: &str = "claude_code_cli_v1";
pub const ADAPTER_DIALECT_CODEX_CLI_V1: &str = "codex_cli_v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCapability {
    pub provider_capability_ref: ProviderCapabilityId,
    pub provider_type: ProviderType,
    pub command_path: String,
    pub version: String,
    pub supported_output_modes: Vec<String>,
    pub supports_session: bool,
    pub supports_resume: bool,
    /// adapter dialect 字符串(见 `ADAPTER_DIALECT_*` 常量)。envelope 与
    /// fingerprint 据此复验 adapter 与 provider 类型一致。
    pub adapter_dialect: String,
    /// `supports_resume` 的可审计三态证据。仅当 `Confirmed` 时才允许 resume;
    /// `Unknown`/`Denied` 时 gateway 对 resume fail-closed。
    pub resume_evidence: ProviderCapabilityEvidence,
    pub probed_at: String,
    pub install_source: String,
}

pub struct ProviderCapabilityProbe {
    compatibility: AdapterCompatibilityEntry,
}

impl ProviderCapabilityProbe {
    pub fn new(compatibility: AdapterCompatibilityEntry) -> Self {
        Self { compatibility }
    }

    pub fn probe(&self) -> Result<ProviderCapability, ProviderAdapterError> {
        let probe_output =
            run_command_capture(&self.compatibility.probe_command, None, None, None)?;
        ensure_probe_success(probe_output, &self.compatibility)?;
        let version =
            match run_command_capture(&self.compatibility.version_command, None, None, None) {
                Ok(output) if output.exit_code == Some(0) => first_nonempty_line(&output.stdout)
                    .unwrap_or("unknown")
                    .to_string(),
                Err(_) => "unknown".to_string(),
                Ok(_) => "unknown".to_string(),
            };
        let auth_output =
            run_command_capture(&self.compatibility.auth_check_command, None, None, None)
                .map_err(|error| classify_probe_error(error, &self.compatibility))?;
        ensure_probe_success(auth_output, &self.compatibility)?;

        Ok(ProviderCapability {
            provider_capability_ref: format!(
                "cap_{}_{}",
                provider_type_key(&self.compatibility.provider_type),
                stable_command_suffix(&self.compatibility.probe_command)
            ),
            provider_type: self.compatibility.provider_type.clone(),
            command_path: self
                .compatibility
                .provider_command
                .to_string_lossy()
                .to_string(),
            version,
            supported_output_modes: vec!["sentinel_json".to_string()],
            supports_session: self.compatibility.supports_session,
            supports_resume: self.compatibility.supports_resume,
            adapter_dialect: adapter_dialect_for(&self.compatibility.provider_type).to_string(),
            resume_evidence: if self.compatibility.supports_resume {
                ProviderCapabilityEvidence::confirmed()
            } else {
                ProviderCapabilityEvidence::denied("compatibility table marks resume unsupported")
            },
            probed_at: Utc::now().to_rfc3339(),
            install_source: "user_local_cli".to_string(),
        })
    }
}

fn ensure_probe_success(
    output: crate::cross_cutting::cli_adapter::CapturedCommandOutput,
    compatibility: &AdapterCompatibilityEntry,
) -> Result<(), ProviderAdapterError> {
    if output.exit_code == Some(0) {
        return Ok(());
    }
    Err(classify_probe_error(
        ProviderAdapterError::execution_failed(
            output.exit_code,
            output.stdout,
            output.stderr,
            output.duration_ms,
        ),
        compatibility,
    ))
}

fn classify_probe_error(
    error: ProviderAdapterError,
    compatibility: &AdapterCompatibilityEntry,
) -> ProviderAdapterError {
    let combined = format!("{} {}", error.stderr, error.details).to_lowercase();
    if compatibility
        .unauthorized_patterns
        .iter()
        .any(|pattern| combined.contains(&pattern.to_lowercase()))
    {
        return ProviderAdapterError::unauthorized(error.details, error.stdout, error.stderr);
    }
    if compatibility
        .permission_denied_patterns
        .iter()
        .any(|pattern| combined.contains(&pattern.to_lowercase()))
    {
        return ProviderAdapterError::permission_denied(error.details, error.stdout, error.stderr);
    }
    error
}

fn first_nonempty_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|line| !line.is_empty())
}

fn stable_command_suffix(command: &CommandSpec) -> String {
    command
        .program
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .rev()
        .take(8)
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}

fn provider_type_key(provider_type: &ProviderType) -> &'static str {
    match provider_type {
        ProviderType::ClaudeCode => "claude_code",
        ProviderType::Codex => "codex",
        ProviderType::Pi => unreachable!("provider capability probe has no pi compatibility entry"),
        ProviderType::KimiCode => "kimi_code",
        ProviderType::Fake => "fake",
    }
}

/// 返回 provider 类型对应的 adapter dialect 字符串。Fake/Pi 不经此 probe 路径,
/// 返回占位 dialect 仅供序列化占位。
fn adapter_dialect_for(provider_type: &ProviderType) -> &'static str {
    match provider_type {
        ProviderType::ClaudeCode => ADAPTER_DIALECT_CLAUDE_CODE_CLI_V1,
        ProviderType::Codex => ADAPTER_DIALECT_CODEX_CLI_V1,
        ProviderType::KimiCode | ProviderType::Pi | ProviderType::Fake => "unknown",
    }
}
