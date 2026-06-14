use serde_json::json;

use crate::product::models::ProviderName;
use crate::protocol::contracts::ProviderType;
use crate::task_run::types::TaskRunError;
use crate::web::error::{ApiError, ApiResult};
use crate::web::provider_probe::is_program_on_path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProvider<T> {
    pub provider: T,
    pub selection: ProviderSelection<T>,
    pub status_code: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderSelection<T> {
    Explicit(T),
    Default(T),
    Fallback { requested: T, fallback: T },
}

pub fn resolve_explicit_provider_name<F>(
    value: &str,
    is_available: F,
) -> ApiResult<ResolvedProvider<ProviderName>>
where
    F: Fn(&ProviderName) -> bool,
{
    let provider = parse_provider_name(value)?;
    if provider == ProviderName::Fake || is_available(&provider) {
        return Ok(ResolvedProvider {
            provider: provider.clone(),
            selection: ProviderSelection::Explicit(provider),
            status_code: "provider_available",
        });
    }
    Err(provider_unavailable_api_error(&provider))
}

pub fn resolve_default_coding_provider<F>(
    repository_default_provider: &str,
    is_available: F,
) -> ApiResult<ResolvedProvider<ProviderName>>
where
    F: Fn(&ProviderName) -> bool,
{
    let requested = parse_provider_name(repository_default_provider)?;
    if requested == ProviderName::Fake || is_available(&requested) {
        return Ok(ResolvedProvider {
            provider: requested.clone(),
            selection: ProviderSelection::Default(requested),
            status_code: "provider_available",
        });
    }
    for fallback in [ProviderName::ClaudeCode, ProviderName::Codex] {
        if fallback != requested && is_available(&fallback) {
            return Ok(ResolvedProvider {
                provider: fallback.clone(),
                selection: ProviderSelection::Fallback {
                    requested,
                    fallback,
                },
                status_code: "provider_fallback",
            });
        }
    }
    Err(real_workflow_blocked_api_error())
}

pub fn resolve_runtime_provider_type<F>(
    value: &str,
    is_available: F,
) -> Result<ResolvedProvider<ProviderType>, TaskRunError>
where
    F: Fn(&ProviderType) -> bool,
{
    let provider = parse_provider_type(value)?;
    if is_available(&provider) {
        return Ok(ResolvedProvider {
            provider: provider.clone(),
            selection: ProviderSelection::Explicit(provider),
            status_code: "provider_available",
        });
    }
    Err(TaskRunError::new(
        "provider_unavailable",
        format!("requested provider is unavailable: {value}"),
    ))
}

pub fn resolve_default_runtime_provider_type<F>(
    is_available: F,
) -> Result<ResolvedProvider<ProviderType>, TaskRunError>
where
    F: Fn(&ProviderType) -> bool,
{
    let requested = ProviderType::Codex;
    if is_available(&requested) {
        return Ok(ResolvedProvider {
            provider: requested.clone(),
            selection: ProviderSelection::Default(requested),
            status_code: "provider_available",
        });
    }
    let fallback = ProviderType::ClaudeCode;
    if is_available(&fallback) {
        return Ok(ResolvedProvider {
            provider: fallback.clone(),
            selection: ProviderSelection::Fallback {
                requested,
                fallback,
            },
            status_code: "provider_fallback",
        });
    }
    Err(TaskRunError::new(
        "real_workflow_blocked",
        "real workflow is blocked because no real provider CLI is available",
    ))
}

pub fn provider_name_available(provider: &ProviderName) -> bool {
    match provider {
        ProviderName::Fake => true,
        ProviderName::ClaudeCode => is_program_on_path("claude"),
        ProviderName::Codex => is_program_on_path("codex"),
    }
}

pub fn provider_type_available(provider: &ProviderType) -> bool {
    match provider {
        ProviderType::Fake => true,
        ProviderType::ClaudeCode => is_program_on_path("claude"),
        ProviderType::Codex => is_program_on_path("codex"),
    }
}

pub fn host_real_workflow_ready() -> Result<(), TaskRunError> {
    if cfg!(windows) {
        return Err(TaskRunError::new(
            "real_workflow_blocked",
            "real workflow is blocked on Windows hosts",
        ));
    }
    for program in ["node", "npm"] {
        if !is_program_on_path(program) {
            return Err(TaskRunError::new(
                "real_workflow_blocked",
                format!("real workflow is blocked because `{program}` is not available on PATH"),
            ));
        }
    }
    Ok(())
}

pub fn provider_name_key(provider: &ProviderName) -> &'static str {
    match provider {
        ProviderName::ClaudeCode => "claude_code",
        ProviderName::Codex => "codex",
        ProviderName::Fake => "fake",
    }
}

pub fn provider_type_key(provider: &ProviderType) -> &'static str {
    match provider {
        ProviderType::ClaudeCode => "claude_code",
        ProviderType::Codex => "codex",
        ProviderType::Fake => "fake",
    }
}

fn parse_provider_name(value: &str) -> ApiResult<ProviderName> {
    match value {
        "claude_code" => Ok(ProviderName::ClaudeCode),
        "codex" => Ok(ProviderName::Codex),
        "fake" => Ok(ProviderName::Fake),
        _ => Err(ApiError::validation(
            "invalid_provider",
            "provider must be claude_code, codex, or fake",
        )),
    }
}

fn parse_provider_type(value: &str) -> Result<ProviderType, TaskRunError> {
    match value {
        "claude_code" => Ok(ProviderType::ClaudeCode),
        "codex" => Ok(ProviderType::Codex),
        other => Err(TaskRunError::new(
            "web_runtime_provider_type",
            format!("unsupported provider_type: {other}"),
        )),
    }
}

fn provider_unavailable_api_error(provider: &ProviderName) -> ApiError {
    ApiError::runtime(
        "provider_unavailable",
        format!(
            "requested provider is unavailable: {}",
            provider_name_key(provider)
        ),
        json!({
            "provider": provider_name_key(provider),
            "action": "install provider CLI or choose another available provider"
        }),
    )
}

fn real_workflow_blocked_api_error() -> ApiError {
    ApiError::runtime(
        "real_workflow_blocked",
        "real workflow is blocked because no real provider CLI is available",
        json!({
            "action": "install Claude Code or Codex CLI with npm, then retry"
        }),
    )
}
