use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// 外部 JSON 为裸 UUID 字符串，表示逻辑成员，绝不接受 physical repository ID。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LogicalRepositoryId(pub Uuid);

/// 外部 JSON 为裸 UUID 字符串，表示一个可解析 checkout 实例。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RepositoryCheckoutId(pub Uuid);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositorySourceIdentity {
    pub scheme: String,
    pub key_digest: String,
    pub canonical_git_dir: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_origin: Option<String>,
    pub first_seen_path_hash: String,
}

impl RepositorySourceIdentity {
    pub fn from_git_parts(
        checkout_path: &Path,
        canonical_git_dir: PathBuf,
        canonical_origin: Option<String>,
    ) -> Self {
        let canonical_origin = canonical_origin.map(|value| value.trim().to_string());
        let scheme = if canonical_origin.is_some() {
            "git_dir_and_origin_v1"
        } else {
            "git_dir_only_v1"
        };
        let origin = canonical_origin.as_deref().unwrap_or("");
        let key_digest = sha256_text(&format!(
            "{}\0{}",
            canonical_git_dir.to_string_lossy(),
            origin
        ));
        Self {
            scheme: scheme.to_string(),
            key_digest,
            canonical_git_dir,
            canonical_origin,
            first_seen_path_hash: crate::product::id::repo_hash_for_path(
                checkout_path.to_string_lossy().as_ref(),
            ),
        }
    }

    pub fn git_dir_identity(&self) -> String {
        sha256_text(self.canonical_git_dir.to_string_lossy().as_ref())
    }
}

fn sha256_text(value: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MemberStatus {
    #[default]
    Active,
    Removed,
    Tombstoned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckoutKind {
    Main,
    IssueWorktree,
    Imported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CheckoutAvailability {
    Available,
    Missing,
    #[default]
    Unresolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryType {
    Backend,
    Frontend,
    #[serde(alias = "lib")]
    Library,
    Mixed,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodebaseMemberRecord {
    pub logical_repository_id: LogicalRepositoryId,
    pub physical_repository_id: String,
    pub alias: String,
    pub role: String,
    pub ordinal: u32,
    pub source_identity: RepositorySourceIdentity,
    #[serde(default)]
    pub repo_type: RepositoryType,
    #[serde(default)]
    pub tech_stack: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_ref: Option<String>,
    #[serde(default)]
    pub checkout_ids: Vec<RepositoryCheckoutId>,
    #[serde(default)]
    pub status: MemberStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryCheckoutRecord {
    pub checkout_id: RepositoryCheckoutId,
    pub logical_repository_id: LogicalRepositoryId,
    pub physical_repository_id: String,
    pub kind: CheckoutKind,
    pub canonical_path: PathBuf,
    pub checkout_path_hash: String,
    pub git_dir_identity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    #[serde(default)]
    pub availability: CheckoutAvailability,
    pub observed_at: String,
    pub created_at: String,
    pub updated_at: String,
}

#[cfg(test)]
mod tests {
    use super::{
        CheckoutAvailability, LogicalRepositoryId, MemberStatus, RepositoryCheckoutId,
        RepositorySourceIdentity, RepositoryType,
    };
    use uuid::Uuid;

    #[test]
    fn member_defaults_and_library_alias_are_serde_compatible() {
        let repo_type: RepositoryType = serde_json::from_str("\"lib\"").unwrap();
        assert_eq!(repo_type, RepositoryType::Library);
        assert_eq!(serde_json::to_string(&repo_type).unwrap(), "\"library\"");

        let availability: CheckoutAvailability = serde_json::from_str("\"unresolved\"").unwrap();
        assert_eq!(availability, CheckoutAvailability::Unresolved);
        assert_eq!(RepositoryType::default(), RepositoryType::Unknown);
        assert_eq!(MemberStatus::default(), MemberStatus::Active);
    }

    #[test]
    fn source_identity_uses_git_dir_and_origin_not_checkout_path_hash() {
        let first = RepositorySourceIdentity::from_git_parts(
            std::path::Path::new("/workspace/api"),
            std::path::PathBuf::from("/workspace/api/.git"),
            Some("ssh://git@example.test/acme/api.git".to_string()),
        );
        let second = RepositorySourceIdentity::from_git_parts(
            std::path::Path::new("/workspace/api-renamed"),
            std::path::PathBuf::from("/workspace/api/.git"),
            Some("ssh://git@example.test/acme/api.git".to_string()),
        );
        let collision = RepositorySourceIdentity::from_git_parts(
            std::path::Path::new("/workspace/api"),
            std::path::PathBuf::from("/workspace/api/.git"),
            Some("ssh://git@example.test/acme/other.git".to_string()),
        );

        assert_eq!(first.key_digest, second.key_digest);
        assert_ne!(first.first_seen_path_hash, second.first_seen_path_hash);
        assert_ne!(first.key_digest, collision.key_digest);
        assert_eq!(first.scheme, "git_dir_and_origin_v1");
    }

    #[test]
    fn identity_newtypes_serialize_as_bare_uuid_strings() {
        let logical =
            LogicalRepositoryId(Uuid::parse_str("018f0f8e-2c2d-7a10-8a11-111111111111").unwrap());
        let checkout =
            RepositoryCheckoutId(Uuid::parse_str("018f0f8e-2c2d-7a10-8a11-222222222222").unwrap());

        assert_eq!(
            serde_json::to_string(&logical).unwrap(),
            "\"018f0f8e-2c2d-7a10-8a11-111111111111\""
        );
        assert_eq!(
            serde_json::to_string(&checkout).unwrap(),
            "\"018f0f8e-2c2d-7a10-8a11-222222222222\""
        );
        assert!(serde_json::from_str::<LogicalRepositoryId>("\"repository_0001\"").is_err());
    }
}
