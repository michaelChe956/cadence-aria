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

/// C1（enrollment-recovery-surface Task 1）：enrollment 的双载体授权 target。
/// 单仓携带真实 physical `RepositoryRecord.id`；逻辑代码库必须同时携带
/// logical codebase id 与 logical repository id——任一级缺失即拒绝，绝不把
/// 单仓降级为 logical target 或从路径猜测。
///
/// 批 2 R1（REQ-WIGA-01、REQ-MTG-03）：新增成员集合变体
/// `logical_codebase_members`——一次授权 LC 的全部有效选中成员（升序
/// 去重非空）。反序列化是唯一规范化入口（原始 REST JSON 与磁盘读取同路）：
/// 乱序/重复输入归一为升序唯一集合，空集/非法 UUID 拒绝，单元素集合折叠
/// 旧 `LogicalCodebase` kind；旧 wire 原样可读、不迁移。digest、
/// selection_key、bound.target 的比较一律发生在规范化之后。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EnrollmentTarget {
    SingleRepository {
        repository_id: String,
    },
    LogicalCodebase {
        logical_codebase_id: String,
        logical_repository_id: LogicalRepositoryId,
    },
    LogicalCodebaseMembers {
        logical_codebase_id: String,
        logical_repository_ids: Vec<LogicalRepositoryId>,
    },
}

impl<'de> Deserialize<'de> for EnrollmentTarget {
    /// 统一规范化反序列化（原始 JSON 与磁盘同入口）：集合变体空集拒绝、
    /// 非 UUID 成员由 `LogicalRepositoryId` 的 transparent Uuid 反序列化
    /// 拒绝、乱序/重复归一升序唯一、单元素折叠旧 kind；旧两变体原样通过。
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(tag = "kind", rename_all = "snake_case")]
        enum EnrollmentTargetWire {
            SingleRepository {
                repository_id: String,
            },
            LogicalCodebase {
                logical_codebase_id: String,
                logical_repository_id: LogicalRepositoryId,
            },
            LogicalCodebaseMembers {
                logical_codebase_id: String,
                #[serde(default)]
                logical_repository_ids: Vec<LogicalRepositoryId>,
            },
        }
        match EnrollmentTargetWire::deserialize(deserializer)? {
            EnrollmentTargetWire::SingleRepository { repository_id } => {
                Ok(Self::SingleRepository { repository_id })
            }
            EnrollmentTargetWire::LogicalCodebase {
                logical_codebase_id,
                logical_repository_id,
            } => Ok(Self::LogicalCodebase {
                logical_codebase_id,
                logical_repository_id,
            }),
            EnrollmentTargetWire::LogicalCodebaseMembers {
                logical_codebase_id,
                logical_repository_ids,
            } => Self::canonical_logical_members(logical_codebase_id, logical_repository_ids)
                .ok_or_else(|| {
                    serde::de::Error::custom(
                        "logical_codebase_members target requires a non-empty set of logical \
                         repository ids",
                    )
                }),
        }
    }
}

impl EnrollmentTarget {
    /// 批 2 R1：从成员集合构造规范 target（GET 投影/PUT 互证共用）。
    /// 输入允许乱序/重复——内部归一为升序唯一；空集返回 `None`（调用方
    /// fail-closed）；恰一成员折叠旧 `LogicalCodebase` kind（新写不落单
    /// 元素集合形态）。
    pub fn canonical_logical_members(
        logical_codebase_id: String,
        mut logical_repository_ids: Vec<LogicalRepositoryId>,
    ) -> Option<Self> {
        logical_repository_ids.sort_unstable();
        logical_repository_ids.dedup();
        match logical_repository_ids[..] {
            [] => None,
            [only] => Some(Self::LogicalCodebase {
                logical_codebase_id,
                logical_repository_id: only,
            }),
            _ => Some(Self::LogicalCodebaseMembers {
                logical_codebase_id,
                logical_repository_ids,
            }),
        }
    }

    /// 逻辑载体变体（单成员/集合）的 LC 身份；单仓返回 `None`。
    pub fn logical_codebase_id(&self) -> Option<&str> {
        match self {
            Self::SingleRepository { .. } => None,
            Self::LogicalCodebase {
                logical_codebase_id,
                ..
            }
            | Self::LogicalCodebaseMembers {
                logical_codebase_id,
                ..
            } => Some(logical_codebase_id.as_str()),
        }
    }

    /// 集合变体的授权成员集（升序唯一）；单成员/单仓返回 `None`。
    pub fn logical_member_ids(&self) -> Option<&[LogicalRepositoryId]> {
        match self {
            Self::LogicalCodebaseMembers {
                logical_repository_ids,
                ..
            } => Some(logical_repository_ids.as_slice()),
            Self::SingleRepository { .. } | Self::LogicalCodebase { .. } => None,
        }
    }
}

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

    /// 批 2 R1 红测①：原始 JSON 反序列化（REST/磁盘同一入口）——乱序/重复
    /// 归一升序唯一；单元素折叠旧 kind；空集/非法 UUID 拒绝；旧 wire 原读。
    #[test]
    fn enrollment_target_members_json_normalizes_sorted_unique() {
        use super::EnrollmentTarget;

        let raw = serde_json::json!({
            "kind": "logical_codebase_members",
            "logical_codebase_id": "lc_0001",
            "logical_repository_ids": [
                "00000000-0000-0000-0000-000000000003",
                "00000000-0000-0000-0000-000000000001",
                "00000000-0000-0000-0000-000000000003",
                "00000000-0000-0000-0000-000000000002"
            ]
        });
        let target: EnrollmentTarget = serde_json::from_value(raw).expect("normalized set target");
        let expected = EnrollmentTarget::LogicalCodebaseMembers {
            logical_codebase_id: "lc_0001".to_string(),
            logical_repository_ids: vec![super::LogicalRepositoryId(uuid::Uuid::from_u128(1))],
        };
        assert_ne!(
            target, expected,
            "guard: multi member set must not collapse"
        );

        let sorted = EnrollmentTarget::LogicalCodebaseMembers {
            logical_codebase_id: "lc_0001".to_string(),
            logical_repository_ids: vec![
                super::LogicalRepositoryId(uuid::Uuid::from_u128(1)),
                super::LogicalRepositoryId(uuid::Uuid::from_u128(2)),
                super::LogicalRepositoryId(uuid::Uuid::from_u128(3)),
            ],
        };
        assert_eq!(target, sorted);
        // 归一后写回 wire：升序唯一、无重复残留。
        let wire = serde_json::to_value(&target).unwrap();
        assert_eq!(wire["kind"], "logical_codebase_members");
        assert_eq!(
            wire["logical_repository_ids"],
            serde_json::json!([
                "00000000-0000-0000-0000-000000000001",
                "00000000-0000-0000-0000-000000000002",
                "00000000-0000-0000-0000-000000000003"
            ])
        );

        // 语义等价（乱序/重复差异）输入反序列化后全等——digest/bound.target
        // 比较在规范化之后进行。
        let shuffled: EnrollmentTarget = serde_json::from_value(serde_json::json!({
            "kind": "logical_codebase_members",
            "logical_codebase_id": "lc_0001",
            "logical_repository_ids": [
                "00000000-0000-0000-0000-000000000002",
                "00000000-0000-0000-0000-000000000003",
                "00000000-0000-0000-0000-000000000001",
                "00000000-0000-0000-0000-000000000001"
            ]
        }))
        .expect("shuffled set target");
        assert_eq!(shuffled, sorted);
    }

    #[test]
    fn enrollment_target_members_json_single_element_collapses_legacy_kind() {
        use super::{EnrollmentTarget, LogicalRepositoryId};

        let raw = serde_json::json!({
            "kind": "logical_codebase_members",
            "logical_codebase_id": "lc_0001",
            "logical_repository_ids": [
                "00000000-0000-0000-0000-000000000009",
                "00000000-0000-0000-0000-000000000009"
            ]
        });
        let target: EnrollmentTarget =
            serde_json::from_value(raw).expect("single member collapses legacy kind");
        assert_eq!(
            target,
            EnrollmentTarget::LogicalCodebase {
                logical_codebase_id: "lc_0001".to_string(),
                logical_repository_id: LogicalRepositoryId(uuid::Uuid::from_u128(9)),
            }
        );
        let wire = serde_json::to_value(&target).unwrap();
        assert_eq!(wire["kind"], "logical_codebase");
        assert!(
            wire.get("logical_repository_ids").is_none(),
            "collapsed target serializes the legacy wire shape"
        );
    }

    #[test]
    fn enrollment_target_members_json_rejects_empty_and_invalid_uuid() {
        use super::EnrollmentTarget;

        let empty = serde_json::json!({
            "kind": "logical_codebase_members",
            "logical_codebase_id": "lc_0001",
            "logical_repository_ids": []
        });
        assert!(serde_json::from_value::<EnrollmentTarget>(empty).is_err());

        let missing = serde_json::json!({
            "kind": "logical_codebase_members",
            "logical_codebase_id": "lc_0001"
        });
        assert!(serde_json::from_value::<EnrollmentTarget>(missing).is_err());

        let invalid = serde_json::json!({
            "kind": "logical_codebase_members",
            "logical_codebase_id": "lc_0001",
            "logical_repository_ids": ["not-a-uuid"]
        });
        assert!(serde_json::from_value::<EnrollmentTarget>(invalid).is_err());
    }

    #[test]
    fn enrollment_target_legacy_wire_roundtrip_unchanged() {
        use super::EnrollmentTarget;

        let single: EnrollmentTarget = serde_json::from_value(serde_json::json!({
            "kind": "single_repository",
            "repository_id": "repo_physical_1"
        }))
        .unwrap();
        assert_eq!(
            single,
            EnrollmentTarget::SingleRepository {
                repository_id: "repo_physical_1".to_string()
            }
        );
        assert_eq!(single.logical_codebase_id(), None);
        assert_eq!(single.logical_member_ids(), None);

        let logical: EnrollmentTarget = serde_json::from_value(serde_json::json!({
            "kind": "logical_codebase",
            "logical_codebase_id": "lc_0001",
            "logical_repository_id": "00000000-0000-0000-0000-000000000004"
        }))
        .unwrap();
        assert_eq!(
            logical,
            EnrollmentTarget::LogicalCodebase {
                logical_codebase_id: "lc_0001".to_string(),
                logical_repository_id: super::LogicalRepositoryId(uuid::Uuid::from_u128(4)),
            }
        );
        assert_eq!(logical.logical_member_ids(), None);
        assert_eq!(logical.logical_codebase_id(), Some("lc_0001"));
    }

    #[test]
    fn enrollment_target_canonical_logical_members_builder_matches_wire_rules() {
        use super::{EnrollmentTarget, LogicalRepositoryId};

        let member = |n: u128| LogicalRepositoryId(uuid::Uuid::from_u128(n));
        assert_eq!(
            EnrollmentTarget::canonical_logical_members(
                "lc_0001".to_string(),
                vec![member(3), member(1), member(3), member(2)],
            ),
            Some(EnrollmentTarget::LogicalCodebaseMembers {
                logical_codebase_id: "lc_0001".to_string(),
                logical_repository_ids: vec![member(1), member(2), member(3)],
            })
        );
        assert_eq!(
            EnrollmentTarget::canonical_logical_members(
                "lc_0001".to_string(),
                vec![member(7), member(7)],
            ),
            Some(EnrollmentTarget::LogicalCodebase {
                logical_codebase_id: "lc_0001".to_string(),
                logical_repository_id: member(7),
            })
        );
        assert_eq!(
            EnrollmentTarget::canonical_logical_members("lc_0001".to_string(), vec![]),
            None
        );
    }
}
