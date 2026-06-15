use cadence_aria::product::models::ProviderName;
use cadence_aria::protocol::contracts::ProviderType;
use cadence_aria::web::provider_availability::{
    ProviderSelection, resolve_default_coding_provider, resolve_default_runtime_provider_type,
    resolve_explicit_provider_name,
};

#[test]
fn default_coding_provider_falls_back_from_unavailable_codex_to_claude_code() {
    let selected = resolve_default_coding_provider("codex", |provider| {
        matches!(provider, ProviderName::ClaudeCode)
    })
    .expect("fallback provider");

    assert_eq!(selected.provider, ProviderName::ClaudeCode);
    assert_eq!(
        selected.selection,
        ProviderSelection::Fallback {
            requested: ProviderName::Codex,
            fallback: ProviderName::ClaudeCode
        }
    );
    assert_eq!(selected.status_code, "provider_fallback");
}

#[test]
fn explicit_unavailable_provider_is_rejected_without_fallback() {
    let error = resolve_explicit_provider_name("codex", |_| false)
        .expect_err("explicit unavailable provider should block");

    assert_eq!(error.code, "provider_unavailable");
    assert!(error.message.contains("codex"));
}

#[test]
fn default_runtime_provider_uses_claude_code_when_codex_is_unavailable() {
    let selected = resolve_default_runtime_provider_type(|provider| {
        matches!(provider, ProviderType::ClaudeCode)
    })
    .expect("runtime fallback provider");

    assert_eq!(selected.provider, ProviderType::ClaudeCode);
    assert_eq!(
        selected.selection,
        ProviderSelection::Fallback {
            requested: ProviderType::Codex,
            fallback: ProviderType::ClaudeCode
        }
    );
    assert_eq!(selected.status_code, "provider_fallback");
}

#[test]
fn default_runtime_provider_blocks_when_no_real_provider_is_available() {
    let error = resolve_default_runtime_provider_type(|_| false)
        .expect_err("missing real providers should block workflow");

    assert_eq!(error.code, "real_workflow_blocked");
    assert!(error.message.contains("provider"));
}
