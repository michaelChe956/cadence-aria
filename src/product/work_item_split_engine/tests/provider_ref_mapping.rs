// C-2:`provider_ref_for_name` 集中映射契约测试。
//
// 契约(Task 1a 四家映射;lcg_t13 重钉):registry/availability gate 的
// `ProviderName` → gateway `ProviderRef` 对四家真实 provider(ClaudeCode/
// Codex/Pi/KimiCode)各自显式映射到同名 `ProviderRefType`,并冻结托管
// capability snapshot 引用(`cap_managed_snapshot`);Fake(及未来新增
// provider)必须显式 fail-closed(`UnsupportedCapability`,错误信息含
// `provider_unsupported_for_gateway_launch` 判别码与 provider 名),🔴
// 禁止 `_ => ProviderRef::claude_code(...)` 之类的静默回退——配置的
// provider 不允许被悄悄换成 Claude 启动。match 保持无 `_` 分支的穷尽
// 形态,未来新增 `ProviderName` 变体时编译期即强制补显式映射决策。
// (旧断言「Pi/Kimi 必须映射失败」是 Task 1a 之前的两方言形态,已被四家
// 映射语义取代——Pi/Kimi 的准入/阻断由 capability 分格门与路由级硬门
// 施加,与本映射正交。)

use crate::product::logical_codebase::ProviderGatewayError;
use crate::product::work_item_split_engine::engine::provider_ref_for_name;


/// Task 1a 四家显式映射:每个真实 provider 映射到自己的
/// `ProviderRefType`(Pi/Kimi 不是 Claude 回退),snapshot 引用冻结为
/// 托管引用。
#[test]
fn provider_ref_for_name_maps_all_four_real_providers_explicitly() {
    use crate::product::logical_codebase::ProviderRefType;
    for (provider, expected) in [
        (ProviderName::ClaudeCode, ProviderRefType::ClaudeCode),
        (ProviderName::Codex, ProviderRefType::Codex),
        (ProviderName::Pi, ProviderRefType::Pi),
        (ProviderName::KimiCode, ProviderRefType::KimiCode),
    ] {
        let mapped = provider_ref_for_name(&provider).unwrap_or_else(|error| {
            panic!("{provider:?} must map to its own gateway provider ref: {error}")
        });
        assert_eq!(
            mapped.provider_type, expected,
            "{provider:?} must map to its own provider type, not a fallback"
        );
        assert_eq!(
            mapped.capability_snapshot_ref, "cap_managed_snapshot",
            "{provider:?} must carry the managed capability snapshot ref"
        );
    }
}

/// Fake 仍显式 fail-closed(判别码 + provider 名),且四家映射绝无静默
/// Claude 回退——`from_provider_name` 的 match 无 `_` 分支,新增
/// `ProviderName` 变体时编译期强制补映射决策。
#[test]
fn provider_ref_for_name_fails_closed_for_fake_without_claude_fallback() {
    let error = provider_ref_for_name(&ProviderName::Fake)
        .err()
        .expect("Fake must not map to a gateway provider ref");
    assert!(
        matches!(&error, ProviderGatewayError::UnsupportedCapability(reason)
            if reason.contains("provider_unsupported_for_gateway_launch")),
        "expected explicit unsupported error for Fake, got {error:?}"
    );
    assert!(
        error.to_string().contains("Fake"),
        "error must name the configured provider, got {error}"
    );
}
