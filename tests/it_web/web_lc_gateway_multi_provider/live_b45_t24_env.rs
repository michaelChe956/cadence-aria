//! T24 现场环境入口(证据根 + cancel provider 集合解析)。
//!
//! 抽离理由:`live_b45_t24` 已按祖父条款登记 2400 行上限(棘轮只许删不许
//! 涨),类1′ 引入的 run-id 证据根计算点与 cancel 集合 env 解析整体落到本
//! 文件;纯移动、行为不变。

use std::path::PathBuf;

use cadence_aria::product::models::ProviderName;

use super::evidence_root::{B45_T24_RUN_ID_ENV, provider_evidence_root};

/// T24 证据根(计划 Files:cadence/reports/lc-gateway-multi-provider/pi/<facet>/;
/// 设 `B45_T24_RUN_ID` 时 `pi/<run-id>/<facet>/`,按轮隔离)。
pub(crate) fn b45_t24_evidence_root(facet: &str) -> PathBuf {
    provider_evidence_root(&ProviderName::Pi, B45_T24_RUN_ID_ENV).join(facet)
}

/// 取消格默认三家(env 未设即此集合)。
fn b45_t24_cancel_default_providers() -> Vec<ProviderName> {
    vec![
        ProviderName::ClaudeCode,
        ProviderName::Codex,
        ProviderName::KimiCode,
    ]
}

/// `B45_T24_CANCEL_PROVIDERS` 解析(逗号分隔 claude/codex/kimi;大小写与
/// 重复记号按集合收敛)。
///
/// 语义(集合精确口径的另一半):未设→默认三家;设了但**空集合**或未知
/// 记号→Err(必须红,不静默回退三家)。
pub(crate) fn b45_t24_parse_cancel_providers(
    raw: Option<&str>,
) -> Result<Vec<ProviderName>, String> {
    let Some(raw) = raw else {
        return Ok(b45_t24_cancel_default_providers());
    };
    let mut providers = Vec::new();
    for token in raw.split(',') {
        let token = token.trim().to_lowercase();
        if token.is_empty() {
            continue;
        }
        let provider = match token.as_str() {
            "claude" => ProviderName::ClaudeCode,
            "codex" => ProviderName::Codex,
            "kimi" => ProviderName::KimiCode,
            other => {
                return Err(format!(
                    "未知 provider 记号 {other:?}(只认 claude/codex/kimi)"
                ));
            }
        };
        if !providers.contains(&provider) {
            providers.push(provider);
        }
    }
    if providers.is_empty() {
        return Err(format!(
            "空集合(B45_T24_CANCEL_PROVIDERS={raw:?});空集合必须红"
        ));
    }
    Ok(providers)
}

/// env 解析入口(非法 env 直接 panic=红,不降级)。
pub(crate) fn b45_t24_cancel_providers_from_env() -> Vec<ProviderName> {
    b45_t24_parse_cancel_providers(std::env::var("B45_T24_CANCEL_PROVIDERS").ok().as_deref())
        .unwrap_or_else(|error| panic!("B45_T24_CANCEL_PROVIDERS 非法:{error}"))
}
