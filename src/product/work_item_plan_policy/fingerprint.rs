use std::fmt;

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
    /// Structured findings intentionally exclude prose and class: reviewer
    /// wording and a later policy reclassification must not make the same
    /// field-level issue look like a new finding.
    pub fn for_finding(
        category: Option<ReviewFindingCategory>,
        class: FindingClass,
        message: &str,
        contract_field: Option<&str>,
    ) -> Self {
        match category {
            Some(category) => Self::hash_scalars([
                normalize_text(category.as_str()),
                normalize_text(contract_field.unwrap_or_default()),
            ]),
            None => Self::hash_scalars([
                normalize_text(class.as_str()),
                normalize_text(message),
                normalize_text(contract_field.unwrap_or_default()),
            ]),
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
