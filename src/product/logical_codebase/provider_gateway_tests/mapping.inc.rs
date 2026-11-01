// Task 1a(lcg_t01):四家显式 provider 映射与 Fake/未知不回退
// (REQ-LCG-01;计划冻结接口「Task 1a(映射/承载)」)。
//
// 断言对照计划 Task 1 Step 1:`assert_eq!(mapped_ref.provider_type,
// expected_provider);`——四家真实 provider 经 `ProviderRef::
// from_provider_name` 唯一映射到显式 `ProviderRefType`,wire dialect
// 序列化冻结;Fake 与未知/未来值 fail-closed,绝不回退 Claude、其它
// provider 或 legacy direct。

use crate::product::logical_codebase::policy::ProviderWireDialect;
use crate::product::logical_codebase::provider_gateway::{
    PROVIDER_UNSUPPORTED_FOR_GATEWAY_LAUNCH, ProviderRef, ProviderRefType,
};
use crate::product::models::ProviderName;

/// 四家真实 provider 唯一映射:provider_type 显式对应,wire dialect
/// 序列化等于冻结字符串(claude-stream-json/codex-app-server-rpc/
/// pi-rpc/kimi-acp)。
#[test]
fn lcg_t01_provider_ref_maps_four_real_providers() {
    let cases = [
        (
            ProviderName::ClaudeCode,
            ProviderRefType::ClaudeCode,
            ProviderWireDialect::ClaudeCodeStreamJson,
            "\"claude-stream-json\"",
        ),
        (
            ProviderName::Codex,
            ProviderRefType::Codex,
            ProviderWireDialect::CodexAppServerRpc,
            "\"codex-app-server-rpc\"",
        ),
        (
            ProviderName::Pi,
            ProviderRefType::Pi,
            ProviderWireDialect::PiRpc,
            "\"pi-rpc\"",
        ),
        (
            ProviderName::KimiCode,
            ProviderRefType::KimiCode,
            ProviderWireDialect::KimiAcp,
            "\"kimi-acp\"",
        ),
    ];
    for (provider, expected_provider, expected_wire, expected_wire_json) in cases {
        let mapped_ref = ProviderRef::from_provider_name(&provider, "cap_managed_snapshot")
            .expect("四家真实 provider 必须显式映射,不得拒绝");
        assert_eq!(mapped_ref.provider_type, expected_provider);
        assert_eq!(
            serde_json::to_string(&expected_wire).expect("wire dialect 可序列化"),
            expected_wire_json
        );
    }
}

/// Fake(及未来新增的未知 ProviderName 值)不得回退:fail-closed 错误携带
/// 稳定判别码 `PROVIDER_UNSUPPORTED_FOR_GATEWAY_LAUNCH` 与被拒 provider 名,
/// 绝不静默换 Claude、其它 provider 或 legacy direct 启动。
#[test]
fn lcg_t01_fake_and_unknown_never_fallback() {
    let error = ProviderRef::from_provider_name(&ProviderName::Fake, "cap_managed_snapshot")
        .expect_err("Fake 不得进入 gateway 启动映射");
    let message = error.to_string();
    assert!(
        message.contains(PROVIDER_UNSUPPORTED_FOR_GATEWAY_LAUNCH),
        "错误必须携带稳定判别码 {PROVIDER_UNSUPPORTED_FOR_GATEWAY_LAUNCH}: {message}"
    );
    assert!(
        message.contains("Fake"),
        "错误必须点名被拒 provider,不得静默换 provider: {message}"
    );
}
