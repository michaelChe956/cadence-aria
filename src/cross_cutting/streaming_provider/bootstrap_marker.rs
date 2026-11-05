//! RootRecipe 相位 `BootstrapExecutorMarker` 的 provider 无关 canonical
//! digest(REQ-BOOT-15;owner 2026-11-05 裁定方案 B):三家的 LC 投影共用
//! 同一实现——固定字段序 + 长度分隔 + 各家冻结 schema 前缀,序列化 marker
//! 公开承载面的完整凭据 operation/receipt 上下文。claude 侧既有
//! `lc-claude-bootstrap-marker-v1` 摘要逐字不变(实现上收,不改变任何
//! 既有 digest 值);pi/codex 使用各自独立 schema 前缀,与三家其余 LC
//! 摘要的 per-provider 前缀约定同构。

use sha2::{Digest as ShaDigest, Sha256};

use crate::product::logical_codebase::policy::SessionPolicyAction;
use crate::product::logical_codebase::provider_admission_preflight::BootstrapExecutorMarker;

/// RootRecipe 相位 marker 的确定性 canonical digest:固定字段序(schema、
/// provider、写权限 action、operation id、自举 step、凭据 canonical root、
/// marker canonical root、receipt context)+ 长度分隔的 SHA-256,输出
/// `sha256:` + 64 位小写 hex。`schema` 前缀由各家投影冻结(claude 既有
/// `lc-claude-bootstrap-marker-v1` 逐字不变;pi/codex 各自独立前缀),
/// `provider` 名进第一字段域。凭据的 project/lc 身份与 input digest 由
/// prepare 侧 `validate_root_recipe_request` 对 durable Running operation
/// 重核验承载,不在此重复序列化;禁止 Debug 文本与递归哈希。
pub(crate) fn bootstrap_marker_canonical_digest(
    schema: &str,
    provider: &str,
    marker: &BootstrapExecutorMarker,
) -> String {
    let credential = marker.credential();
    let credential_root = credential.canonical_root().to_string_lossy();
    let marker_root = marker.canonical_root().to_string_lossy();
    marker_digest(
        schema,
        &[
            provider,
            marker_action_text(marker.action()),
            credential.operation_id(),
            credential.step().as_str(),
            &credential_root,
            &marker_root,
            marker.receipt_context(),
        ],
    )
}

/// 长度分隔的 canonical 字段(`len:value`,杜绝分隔符/注入歧义;与各家
/// projection 的 `lc_digest` 同构)。
fn canonical_field(canonical: &mut String, value: &str) {
    canonical.push_str(&value.len().to_string());
    canonical.push(':');
    canonical.push_str(value);
}

/// 投影 digest:固定字段序 + 长度分隔 + schema 前缀的 SHA-256(与各家
/// projection 的 `lc_digest` 同一算法;`sha256:` 前缀 + 64 位小写 hex)。
fn marker_digest(schema: &str, parts: &[&str]) -> String {
    let mut canonical = String::new();
    canonical_field(&mut canonical, schema);
    for part in parts {
        canonical.push('|');
        canonical_field(&mut canonical, part);
    }
    let mut hasher = Sha256::new();
    hasher.update(canonical.as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

/// `SessionPolicyAction` 的稳定文本(serde snake_case 序列化同形;与各家
/// projection 的 `action_text` 同一映射)。
fn marker_action_text(action: SessionPolicyAction) -> &'static str {
    match action {
        SessionPolicyAction::PlanningReadOnly => "planning_read_only",
        SessionPolicyAction::CodingTargetWrite => "coding_target_write",
        SessionPolicyAction::ReviewReadOnly => "review_read_only",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::logical_codebase::aggregate_initialization::AggregateInitializationStepKind;
    use crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential;

    fn test_marker() -> BootstrapExecutorMarker {
        let canonical_root = std::path::PathBuf::from("/lc/shared-marker-root");
        let credential = BootstrapPhaseCredential::for_test(
            "project_0001",
            "logical_codebase_0001",
            "aggregate_initialization_0001",
            AggregateInitializationStepKind::PreCheck,
            "sha256:test-input-digest",
            canonical_root.clone(),
        );
        BootstrapExecutorMarker::new(
            credential,
            SessionPolicyAction::CodingTargetWrite,
            canonical_root,
            "root-recipe:pre_check:command-1",
        )
        .expect("complete bootstrap executor marker")
    }

    /// 共享实现的字段序/摘要形状冻结:同一材料逐字节稳定、schema/provider
    /// 域漂移即变(claude/pi/codex 各自的相位门语义测试在各自 projection
    /// 测试面锁定)。
    #[test]
    fn shared_marker_digest_is_stable_and_drift_sensitive() {
        let marker = test_marker();
        let baseline =
            bootstrap_marker_canonical_digest("lc-test-bootstrap-marker-v1", "pi", &marker);
        assert!(baseline.starts_with("sha256:"));
        assert_eq!(baseline.len(), 71);
        assert_eq!(
            baseline,
            bootstrap_marker_canonical_digest("lc-test-bootstrap-marker-v1", "pi", &marker)
        );
        assert_ne!(
            baseline,
            bootstrap_marker_canonical_digest("lc-test-bootstrap-marker-v2", "pi", &marker)
        );
        assert_ne!(
            baseline,
            bootstrap_marker_canonical_digest("lc-test-bootstrap-marker-v1", "codex", &marker)
        );
    }
}
