use crate::cross_cutting::adapter_compatibility::{AdapterCompatibilityEntry, CommandSpec};
use crate::cross_cutting::cli_adapter::run_command_capture;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::protocol::contracts::ProviderType;
use crate::protocol::enums::ProviderCapabilityId;
use chrono::Utc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCapability {
    pub provider_capability_ref: ProviderCapabilityId,
    pub provider_type: ProviderType,
    pub command_path: String,
    pub version: String,
    pub supported_output_modes: Vec<String>,
    pub supports_session: bool,
    pub supports_resume: bool,
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
        ProviderType::Fake => "fake",
    }
}
