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
    test_provider_enabled: bool,
    is_available: F,
) -> ApiResult<ResolvedProvider<ProviderName>>
where
    F: Fn(&ProviderName) -> bool,
{
    // 缺陷 #13 层 1（2026-10-02 E2E）：登记期占位默认 `fake` 与未配置（空串）
    // 不再无条件放行——生产解析链 fail-closed（稳定错误码 + 可诊断 detail），
    // 杜绝 runner provider_for(Fake) → product_store_not_found → F-14
    // awaiting_manual_recovery 的迟发死亡。test_provider_enabled 既有豁免路径
    // 保留不动（测试面 fake registry 在场，fake/未配置默认照旧解析 Fake）。
    if repository_default_provider.trim().is_empty() {
        if test_provider_enabled {
            return Ok(ResolvedProvider {
                provider: ProviderName::Fake,
                selection: ProviderSelection::Default(ProviderName::Fake),
                status_code: "provider_available",
            });
        }
        return Err(ApiError::runtime(
            "default_provider_not_configured",
            "repository default provider is not configured; set an explicit provider at              request time or re-register the repository with a real default provider",
            json!({
                "repository_default_provider": repository_default_provider,
                "action": "pass an explicit provider or set default_provider_mode to a real provider (claude_code, codex, pi, kimi_code)"
            }),
        ));
    }
    let requested = parse_provider_name(repository_default_provider)?;
    if requested == ProviderName::Fake {
        if test_provider_enabled {
            return Ok(ResolvedProvider {
                provider: ProviderName::Fake,
                selection: ProviderSelection::Default(ProviderName::Fake),
                status_code: "provider_available",
            });
        }
        return Err(ApiError::runtime(
            "default_provider_fake_blocked",
            "repository default provider is `fake`, which is a registration placeholder              and cannot drive real coding runs",
            json!({
                "repository_default_provider": "fake",
                "action": "pass an explicit provider or re-register the repository with default_provider_mode set to a real provider (claude_code, codex, pi, kimi_code)"
            }),
        ));
    }
    if is_available(&requested) {
        return Ok(ResolvedProvider {
            provider: requested.clone(),
            selection: ProviderSelection::Default(requested),
            status_code: "provider_available",
        });
    }
    for fallback in [
        ProviderName::ClaudeCode,
        ProviderName::Codex,
        ProviderName::Pi,
        ProviderName::KimiCode,
    ] {
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
        ProviderName::Pi => is_program_on_path("pi"),
        ProviderName::KimiCode => is_program_on_path("kimi"),
    }
}

pub fn provider_type_available(provider: &ProviderType) -> bool {
    match provider {
        ProviderType::Fake => true,
        ProviderType::ClaudeCode => is_program_on_path("claude"),
        ProviderType::Codex => is_program_on_path("codex"),
        ProviderType::Pi => false,
        ProviderType::KimiCode => false,
    }
}

pub fn host_real_workflow_ready() -> Result<(), TaskRunError> {
    if cfg!(windows) {
        return Err(TaskRunError::new(
            "host_real_workflow_blocked",
            "real workflow is blocked on Windows hosts",
        ));
    }
    for program in ["node", "npm"] {
        if !is_program_on_path(program) {
            return Err(TaskRunError::new(
                "host_real_workflow_blocked",
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
        ProviderName::Pi => "pi",
        ProviderName::KimiCode => "kimi_code",
        ProviderName::Fake => "fake",
    }
}

pub fn provider_type_key(provider: &ProviderType) -> &'static str {
    match provider {
        ProviderType::ClaudeCode => "claude_code",
        ProviderType::Codex => "codex",
        ProviderType::Pi => "pi",
        ProviderType::KimiCode => "kimi_code",
        ProviderType::Fake => "fake",
    }
}

fn parse_provider_name(value: &str) -> ApiResult<ProviderName> {
    match value {
        "claude_code" => Ok(ProviderName::ClaudeCode),
        "codex" => Ok(ProviderName::Codex),
        "pi" => Ok(ProviderName::Pi),
        "kimi_code" => Ok(ProviderName::KimiCode),
        "fake" => Ok(ProviderName::Fake),
        _ => Err(ApiError::validation(
            "invalid_provider",
            "provider must be claude_code, codex, pi, kimi_code, or fake",
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
            "action": "install Claude Code, Codex, Pi, or Kimi Code CLI, then retry"
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::{parse_provider_name, parse_provider_type, provider_name_key};
    use crate::product::models::ProviderName;

    #[test]
    fn parse_provider_name_accepts_kimi_code() {
        assert_eq!(
            parse_provider_name("kimi_code").unwrap(),
            ProviderName::KimiCode
        );
    }

    #[test]
    fn provider_name_key_kimi_code() {
        assert_eq!(provider_name_key(&ProviderName::KimiCode), "kimi_code");
    }

    #[test]
    fn parse_provider_name_accepts_pi() {
        assert_eq!(parse_provider_name("pi").unwrap(), ProviderName::Pi);
    }

    #[test]
    fn provider_name_key_pi() {
        assert_eq!(provider_name_key(&ProviderName::Pi), "pi");
    }

    #[test]
    fn parse_provider_type_still_rejects_pi() {
        let err = parse_provider_type("pi").unwrap_err();
        assert!(err.message.contains("pi") || format!("{err:?}").contains("pi"));
    }

    mod default_provider_policy {
        use super::super::resolve_default_coding_provider;
        use crate::product::models::ProviderName;

        /// 生产 availability 形态：gate 对 Fake 恒放行（既有豁免
        /// `provider_availability_gate_always_allows_fake_for_tests`），真实
        /// provider 按探针——仅 claude 可用。
        fn production_availability(provider: &ProviderName) -> bool {
            matches!(provider, ProviderName::ClaudeCode | ProviderName::Fake)
        }

        /// 缺陷 #13 层 1（2026-10-02 E2E）：登记期占位默认 `fake` 曾被
        /// `resolve_default_coding_provider` 显式放行（可用性门旁路），生产
        /// registry 无 Fake → runner provider_for(Fake) → product_store_not_found
        /// → F-14 awaiting_manual_recovery。生产解析链必须 fail-closed。
        #[test]
        fn resolve_default_fail_closes_fake_placeholder_in_production() {
            let error = resolve_default_coding_provider("fake", false, production_availability)
                .err()
                .expect("生产 fake 占位默认必须 fail-closed");
            assert_eq!(
                error.code, "default_provider_fake_blocked",
                "稳定错误码 + detail：{error:?}"
            );
            assert!(error.message.contains("fake"));
        }

        #[test]
        fn resolve_default_fail_closes_unset_default_in_production() {
            for unset in ["", "  "] {
                let error = resolve_default_coding_provider(unset, false, production_availability)
                    .err()
                    .expect("未配置默认必须 fail-closed");
                assert_eq!(
                    error.code, "default_provider_not_configured",
                    "稳定错误码 + detail：{error:?}"
                );
            }
        }

        /// test_provider_enabled 既有豁免路径保留不动：测试面 fake/未配置默认
        /// 仍解析 Fake（fake registry 在场）。
        #[test]
        fn resolve_default_keeps_test_surface_fake_exemption() {
            for default in ["fake", ""] {
                let resolved =
                    resolve_default_coding_provider(default, true, production_availability)
                        .unwrap_or_else(|error| panic!("测试豁免面必须保留：{error:?}"));
                assert_eq!(resolved.provider, ProviderName::Fake);
                assert_eq!(resolved.status_code, "provider_available");
            }
        }

        /// 真实默认与回退链语义不变（红线）。
        #[test]
        fn resolve_default_real_provider_semantics_unchanged() {
            let direct = resolve_default_coding_provider(
                "claude_code",
                false,
                production_availability,
            )
            .expect("可用真实默认直接命中");
            assert_eq!(direct.provider, ProviderName::ClaudeCode);
            assert_eq!(direct.status_code, "provider_available");

            let fallback =
                resolve_default_coding_provider("codex", false, production_availability)
                    .expect("不可用真实默认走回退链");
            assert_eq!(fallback.provider, ProviderName::ClaudeCode);
            assert_eq!(fallback.status_code, "provider_fallback");
        }
    }
}
