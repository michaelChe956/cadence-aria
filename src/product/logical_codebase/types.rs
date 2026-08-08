use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 外部 JSON 为裸 UUID 字符串，表示逻辑成员，绝不接受 physical repository ID。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LogicalRepositoryId(pub Uuid);

/// 外部 JSON 为裸 UUID 字符串，表示一个可解析 checkout 实例。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RepositoryCheckoutId(pub Uuid);

#[cfg(test)]
mod tests {
    use super::{LogicalRepositoryId, RepositoryCheckoutId};
    use uuid::Uuid;

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
