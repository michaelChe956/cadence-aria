// C-2:`provider_ref_for_name` 集中映射契约测试。
//
// 契约:registry/availability gate 的 `ProviderName` → gateway `ProviderRef` 仅在
// ClaudeCode/Codex 间分发;Pi/KimiCode/Fake(及未来 provider)必须显式
// fail-closed(`UnsupportedCapability`,错误信息含 provider 名),🔴 禁止
// `_ => ProviderRef::claude_code(...)` 之类的静默回退——配置的 provider 不允许
// 被悄悄换成 Claude 启动(回归锁定:这是修 bug,不是行为回归)。

use crate::product::logical_codebase::ProviderGatewayError;
use crate::product::work_item_split_engine::engine::provider_ref_for_name;

#[test]
fn provider_ref_for_name_maps_claude_code_and_codex_only() {
    let claude = provider_ref_for_name(&ProviderName::ClaudeCode)
        .expect("ClaudeCode must map to a gateway provider ref");
    assert_eq!(
        claude.provider_type,
        crate::product::logical_codebase::ProviderRefType::ClaudeCode
    );

    let codex = provider_ref_for_name(&ProviderName::Codex)
        .expect("Codex must map to a gateway provider ref");
    assert_eq!(
        codex.provider_type,
        crate::product::logical_codebase::ProviderRefType::Codex
    );
}

#[test]
fn provider_ref_for_name_fails_closed_for_kimi_and_pi_without_claude_fallback() {
    for provider in [ProviderName::KimiCode, ProviderName::Pi] {
        let error = provider_ref_for_name(&provider)
            .err()
            .unwrap_or_else(|| panic!("{provider:?} must not map to a gateway provider ref"));
        assert!(
            matches!(&error, ProviderGatewayError::UnsupportedCapability(reason)
                if reason.contains("provider_unsupported_for_gateway_launch")),
            "expected explicit unsupported error for {provider:?}, got {error:?}"
        );
        assert!(
            error.to_string().contains(&format!("{provider:?}")),
            "error must name the configured provider, got {error}"
        );
    }
}
