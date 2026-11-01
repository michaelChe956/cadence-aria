use std::collections::BTreeSet;
use std::fmt;

/// 稳定域/unstable 域的哈希盐前缀（与 legacy 无盐域三方隔离）。
const STABLE_DOMAIN_SALT: &str = "v2-stable";
const UNSTABLE_DOMAIN_SALT: &str = "v2-unstable";
/// 受限提取的稳定 ID 前缀（工作项/契约/验收标准）。
const STABLE_ID_PREFIXES: [&str; 3] = ["WI-", "CT-", "AC-"];
/// 机械通道的确定性定位符前缀（REQ-TOP-04：机械 finding 以确定性投影构
/// 造，不落 reviewer 措辞域）——plan 层选项缺口与机械报告 finding code。
const MECHANICAL_LOCATOR_PREFIXES: [&str; 3] = ["plan_options.", "plan_preflight.", "mechanical."];

use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

use super::{FindingClass, ReviewFindingCategory};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidFindingFingerprint;

impl fmt::Display for InvalidFindingFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .write_str("finding fingerprint must be a 64-character lowercase SHA-256 hex string")
    }
}

impl std::error::Error for InvalidFindingFingerprint {}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct FindingFingerprint(pub String);

impl FindingFingerprint {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidFindingFingerprint> {
        let value = value.into();
        if is_lowercase_sha256_hex(&value) {
            Ok(Self(value))
        } else {
            Err(InvalidFindingFingerprint)
        }
    }

    /// Computes a finding identity using the structured schema when a category
    /// is available, and the legacy class/message/field schema otherwise.
    ///
    /// Convenience wrapper over [`Self::identity_for_finding`] for callers that
    /// only need the fingerprint（回归锚点与既有测试口径）。
    pub fn for_finding(
        category: Option<ReviewFindingCategory>,
        class: FindingClass,
        message: &str,
        contract_field: Option<&str>,
    ) -> Self {
        Self::identity_for_finding(category, class, message, contract_field).0
    }

    /// 结构化 identity（F-52，REQ-TOP-04 场景 3-5）：返回 (fingerprint,
    /// identity_unstable)。
    ///
    /// - **稳定域**（category=Some 且 contract_field 含 `WI-\d+|CT-\d+|AC-\d+`
    ///   稳定 ID）：哈希 `["v2-stable", category, ID集, 尾段]`——受限提取 ID
    ///   集合与剥 ID/下标/分隔后的规范化尾段，措辞、class、路径书写顺序均不
    ///   参与，同题异措辞 collapse 为同一身份（`WI-001.output_contracts
    ///   [CT-001].capabilities` ≡ `output_contracts[WI-001].capabilities/
    ///   CT-001`）。
    /// - **unstable 域**（category=Some 但无任何稳定 ID）：哈希
    ///   `["v2-unstable", category, message, field]`——与稳定域盐隔离永不互撞，
    ///   同文重复仍同指纹（可检测），但策略层禁自动裁决（fail-safe 人工）。
    /// - **legacy**（category=None）：保持既有 class+message+field 口径不变
    ///   （durable 旧指纹可比），不标记 unstable。
    pub fn identity_for_finding(
        category: Option<ReviewFindingCategory>,
        class: FindingClass,
        message: &str,
        contract_field: Option<&str>,
    ) -> (Self, bool) {
        match category {
            Some(category) => {
                let field = contract_field.unwrap_or_default();
                // 机械定位符（plan_options./plan_preflight./mechanical. 前缀）
                // 是确定性投影：整体作为身份成分（无尾段），不落措辞域。
                if MECHANICAL_LOCATOR_PREFIXES
                    .iter()
                    .any(|prefix| field.starts_with(prefix))
                {
                    return (
                        Self::hash_scalars([
                            STABLE_DOMAIN_SALT.to_string(),
                            normalize_text(category.as_str()),
                            normalize_text(field),
                            String::new(),
                        ]),
                        false,
                    );
                }
                let ids = stable_ids(field);
                if ids.is_empty() {
                    (
                        Self::hash_scalars([
                            UNSTABLE_DOMAIN_SALT.to_string(),
                            normalize_text(category.as_str()),
                            normalize_text(message),
                            normalize_text(field),
                        ]),
                        true,
                    )
                } else {
                    (
                        Self::hash_scalars([
                            STABLE_DOMAIN_SALT.to_string(),
                            normalize_text(category.as_str()),
                            ids,
                            normalize_tail(field),
                        ]),
                        false,
                    )
                }
            }
            None => (
                Self::hash_scalars([
                    normalize_text(class.as_str()),
                    normalize_text(message),
                    normalize_text(contract_field.unwrap_or_default()),
                ]),
                false,
            ),
        }
    }

    fn hash_scalars<const N: usize>(scalars: [String; N]) -> Self {
        let mut hasher = Sha256::new();
        for scalar in scalars {
            write_length_prefixed(&mut hasher, &scalar);
        }
        Self(hex::encode(hasher.finalize()))
    }
}
/// 受限提取 contract_field 中的稳定 ID（`WI-\d+|CT-\d+|AC-\d+`），
/// BTreeSet 去重排序后逗号连接——`WI-001.output_contracts[CT-001]` 与
/// `output_contracts[WI-001]…/CT-001` 提取同一集合，书写顺序无关。
fn stable_ids(field: &str) -> String {
    let mut ids = BTreeSet::new();
    for prefix in STABLE_ID_PREFIXES {
        let mut cursor = 0;
        while let Some(offset) = field[cursor..].find(prefix) {
            let digits_start = cursor + offset + prefix.len();
            let digits_end = field[digits_start..]
                .find(|ch: char| !ch.is_ascii_digit())
                .map(|len| digits_start + len)
                .unwrap_or(field.len());
            if digits_end > digits_start {
                ids.insert(field[digits_start - prefix.len()..digits_end].to_string());
            }
            cursor = digits_start;
        }
    }
    ids.into_iter().collect::<Vec<_>>().join(",")
}

/// 尾段规范化：剥稳定 ID 与 `[N]` 数组下标，把 `. / [ ]` 分隔折叠为空白后
/// 走 [`normalize_text`]（小写 + NFC + 空白折叠）。
fn normalize_tail(field: &str) -> String {
    let without_ids = remove_stable_ids(field);
    let separated = without_ids.replace(['.', '/', '[', ']'], " ");
    let kept = separated
        .split_whitespace()
        .filter(|token| !token.bytes().all(|byte| byte.is_ascii_digit()))
        .collect::<Vec<_>>();
    normalize_text(&kept.join(" "))
}

fn remove_stable_ids(field: &str) -> String {
    let mut result = String::with_capacity(field.len());
    let mut cursor = 0;
    while cursor < field.len() {
        let rest = &field[cursor..];
        let mut matched = None;
        for prefix in STABLE_ID_PREFIXES {
            if !rest.starts_with(prefix) {
                continue;
            }
            let digits_start = cursor + prefix.len();
            let digits_end = field[digits_start..]
                .find(|ch: char| !ch.is_ascii_digit())
                .map(|len| digits_start + len)
                .unwrap_or(field.len());
            if digits_end > digits_start {
                matched = Some(digits_end);
            }
            break;
        }
        match matched {
            Some(end) => {
                result.push(' ');
                cursor = end;
            }
            None => {
                let ch = rest.chars().next().expect("cursor 停在字符边界");
                result.push(ch);
                cursor += ch.len_utf8();
            }
        }
    }
    result
}

impl<'de> Deserialize<'de> for FindingFingerprint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

pub(crate) fn is_lowercase_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn write_length_prefixed(hasher: &mut Sha256, value: &str) {
    hasher.update(value.len().to_string().as_bytes());
    hasher.update(b":");
    hasher.update(value.as_bytes());
}

fn normalize_text(value: &str) -> String {
    let lowercase = value
        .chars()
        .flat_map(char::to_lowercase)
        .collect::<String>();
    let normalized = lowercase.nfc().collect::<String>();
    normalized.split_whitespace().collect::<Vec<_>>().join(" ")
}
