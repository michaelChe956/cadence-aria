use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderErrorCode {
    ProviderCommandMissing,
    ProviderUnavailable,
    ProviderUnauthorized,
    ProviderPermissionDenied,
    ProviderIncompatibleOutput,
    ProviderTimeout,
    ProviderParseError,
    ProviderExecutionFailed,
}

impl ProviderErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderErrorCode::ProviderCommandMissing => "provider_command_missing",
            ProviderErrorCode::ProviderUnavailable => "provider_unavailable",
            ProviderErrorCode::ProviderUnauthorized => "provider_unauthorized",
            ProviderErrorCode::ProviderPermissionDenied => "provider_permission_denied",
            ProviderErrorCode::ProviderIncompatibleOutput => "provider_incompatible_output",
            ProviderErrorCode::ProviderTimeout => "provider_timeout",
            ProviderErrorCode::ProviderParseError => "provider_parse_error",
            ProviderErrorCode::ProviderExecutionFailed => "provider_execution_failed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderErrorRoute {
    Retry,
    Gate,
    ManualIntervention,
    FailureRoute,
}

pub fn route_provider_error(
    code: &ProviderErrorCode,
    retry_count: u32,
    max_retries: u32,
) -> ProviderErrorRoute {
    match code {
        ProviderErrorCode::ProviderCommandMissing
        | ProviderErrorCode::ProviderUnauthorized
        | ProviderErrorCode::ProviderPermissionDenied => ProviderErrorRoute::ManualIntervention,
        ProviderErrorCode::ProviderUnavailable | ProviderErrorCode::ProviderIncompatibleOutput => {
            ProviderErrorRoute::Gate
        }
        ProviderErrorCode::ProviderTimeout => {
            if retry_count < max_retries {
                ProviderErrorRoute::Retry
            } else {
                ProviderErrorRoute::ManualIntervention
            }
        }
        ProviderErrorCode::ProviderParseError => {
            if retry_count == 0 {
                ProviderErrorRoute::Retry
            } else {
                ProviderErrorRoute::Gate
            }
        }
        ProviderErrorCode::ProviderExecutionFailed => ProviderErrorRoute::FailureRoute,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_unavailable_routes_to_gate_without_retry_or_fallback() {
        assert_eq!(
            route_provider_error(&ProviderErrorCode::ProviderUnavailable, 0, 3),
            ProviderErrorRoute::Gate
        );
        assert_eq!(
            ProviderErrorCode::ProviderUnavailable.as_str(),
            "provider_unavailable"
        );
    }
}
