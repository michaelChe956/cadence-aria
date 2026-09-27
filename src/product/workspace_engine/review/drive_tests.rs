use super::*;

#[test]
fn only_pi_is_excluded_from_review_repair() {
    assert!(provider_allows_review_repair(&ProviderName::KimiCode));
    assert!(!provider_allows_review_repair(&ProviderName::Pi));
    assert!(provider_allows_review_repair(&ProviderName::Codex));
}

/// 诊断直通（claude×轻 握手谜团第 2 轮）：gateway 错误映射不得丢弃 adapter
/// error 的 stderr——Adapter 变体的 stderr 尾部（有界）需并入新错误的
/// details（驱动 result/WS error 消息的上游），stderr 字段同源保留。
#[test]
fn gateway_adapter_error_mapping_carries_bounded_stderr_tail() {
    let stderr_head = "NOISE_HEAD_MARKER".to_string() + &"b".repeat(700);
    let stderr = format!("{stderr_head}\nSENTINEL_GW_STDERR_TAIL");
    let inner = ProviderAdapterError::parse_error(
        "claude policy session: handshake failed: claude policy handshake cancelled",
        String::new(),
        stderr,
    );
    let mapped = map_gateway_error_to_adapter(ProviderGatewayError::Adapter(inner));
    assert!(
        mapped.details.contains("provider_gateway_adapter"),
        "gateway 映射应保留既有 Display 前缀文案，got: {}",
        mapped.details
    );
    assert!(
        mapped.details.contains("SENTINEL_GW_STDERR_TAIL"),
        "gateway 映射应携带 adapter stderr 尾部，got: {}",
        mapped.details
    );
    assert!(
        !mapped.details.contains("NOISE_HEAD_MARKER"),
        "stderr 尾部应有界（丢头保尾），got: {}",
        mapped.details
    );
    assert!(
        mapped.stderr.contains("SENTINEL_GW_STDERR_TAIL"),
        "stderr 字段应同源保留，got: {}",
        mapped.stderr
    );
}
