use std::collections::BTreeSet;

use super::{FindingClass, FindingFingerprint, ReviewFindingCategory};

#[test]
fn fingerprint_is_stable_for_identical_input() {
    let first = FindingFingerprint::for_finding(
        Some(ReviewFindingCategory::ContractGap),
        FindingClass::Repairable,
        "Missing field",
        Some("acceptance_criteria"),
    );
    let second = FindingFingerprint::for_finding(
        Some(ReviewFindingCategory::ContractGap),
        FindingClass::Repairable,
        "Missing field",
        Some("acceptance_criteria"),
    );

    assert_eq!(first, second);
}

#[test]
fn fingerprint_uses_category_and_contract_field_when_category_is_present() {
    // F-52 v2 稳定域：contract_field 含稳定 ID 时，指纹由 category+ID 集+
    // 尾段决定——措辞与 class 重分类不改变身份。
    let original = FindingFingerprint::for_finding(
        Some(ReviewFindingCategory::ContractGap),
        FindingClass::Repairable,
        "Missing acceptance criterion",
        Some("WI-001.acceptance_criteria[AC-001].required_evidence"),
    );

    assert_eq!(
        original,
        FindingFingerprint::for_finding(
            Some(ReviewFindingCategory::ContractGap),
            FindingClass::HumanRequired,
            "The completion criteria omit a required scenario",
            Some("WI-001.acceptance_criteria[AC-001].required_evidence"),
        )
    );
    assert_ne!(
        original,
        FindingFingerprint::for_finding(
            Some(ReviewFindingCategory::ScopeConflict),
            FindingClass::Repairable,
            "Missing acceptance criterion",
            Some("WI-001.acceptance_criteria[AC-001].required_evidence"),
        )
    );
    assert_ne!(
        original,
        FindingFingerprint::for_finding(
            Some(ReviewFindingCategory::ContractGap),
            FindingClass::Repairable,
            "Missing acceptance criterion",
            Some("WI-001.output_contracts[CT-001].capabilities"),
        )
    );
}

#[test]
fn legacy_fingerprint_normalizes_unicode_case_and_whitespace() {
    let composed = FindingFingerprint::for_finding(
        None,
        FindingClass::Repairable,
        "  RÉSUMÉ\n\t  missing  ",
        Some("  Évidence\tField "),
    );
    let decomposed = FindingFingerprint::for_finding(
        None,
        FindingClass::Repairable,
        "re\u{301}sume\u{301} missing",
        Some("e\u{301}vidence field"),
    );

    assert_eq!(composed, decomposed);
}

#[test]
fn legacy_fingerprint_uses_length_prefixes_to_resist_concatenation_ambiguity() {
    let first = FindingFingerprint::for_finding(None, FindingClass::Repairable, "bc", Some("d"));
    let second = FindingFingerprint::for_finding(None, FindingClass::Repairable, "b", Some("cd"));

    assert_ne!(first, second);
}

#[test]
fn fingerprint_serializes_as_a_string_and_validates_deserialization() {
    let fingerprint =
        FindingFingerprint::for_finding(None, FindingClass::Repairable, "message", None);
    let serialized = serde_json::to_value(&fingerprint).unwrap();

    assert_eq!(serialized, serde_json::Value::String(fingerprint.0.clone()));
    assert_eq!(
        serde_json::from_value::<FindingFingerprint>(serialized).unwrap(),
        fingerprint
    );

    for invalid in [
        "too_short",
        "A0d52e876560346f9c9fd0777ad620c166699c1d43eb6f42b0134e7506d4b2f8",
        "g0d52e876560346f9c9fd0777ad620c166699c1d43eb6f42b0134e7506d4b2f8",
    ] {
        assert!(serde_json::from_value::<FindingFingerprint>(serde_json::json!(invalid)).is_err());
    }
}

#[test]
fn fingerprint_btree_set_roundtrips_and_deduplicates() {
    let fingerprint =
        FindingFingerprint::for_finding(None, FindingClass::Advisory, "suggestion", None);
    let set = BTreeSet::from([fingerprint.clone(), fingerprint.clone()]);

    let serialized = serde_json::to_value(&set).unwrap();
    let recovered = serde_json::from_value::<BTreeSet<FindingFingerprint>>(serialized).unwrap();

    assert_eq!(recovered, BTreeSet::from([fingerprint]));
}

/// F-52（REQ-TOP-04 场景 3-5）golden：issue_0002/workspace_session_0009 四个
/// reviewer 节点的真实 findings（durable verbatim 提取，见
/// `fixtures/f52-findings-golden.json`）锁定结构化 identity 契约——同题异措辞
/// 同身份、无稳定 ID 的措辞域标记 unstable 且与稳定域永不互撞、异题稳定
/// 身份互不 collapse。
mod f52_golden {
    use super::*;
    use crate::product::work_item_plan_policy::ClassifiedFinding;
    use crate::product::work_item_plan_policy::classify_finding;
    use crate::web::workspace_ws_types::review::{ReviewFinding, ReviewVerdictType};

    #[derive(serde::Deserialize)]
    struct GoldenEntry {
        node: String,
        verdict: ReviewVerdictType,
        finding: ReviewFinding,
    }

    fn golden() -> Vec<GoldenEntry> {
        serde_json::from_str(include_str!("fixtures/f52-findings-golden.json"))
            .expect("golden fixture must parse")
    }

    fn classified(node: &str, index: usize) -> ClassifiedFinding {
        let entry = golden()
            .into_iter()
            .filter(|entry| entry.node == node)
            .nth(index)
            .unwrap_or_else(|| panic!("{node}[{index}] 必须存在于 golden"));
        classify_finding(entry.verdict, &entry.finding)
    }

    /// 场景 3（同题异措辞同身份）：node_007 与 node_012 对同一 CT-001 供需
    /// 缺口用了两种 contract_field 措辞（`WI-001.output_contracts[CT-001]
    /// .capabilities` vs `output_contracts[WI-001].capabilities/CT-001`），
    /// 受限提取 {WI-001, CT-001} + 尾段后必须同指纹（F-52 现场旧实现
    /// 7798ad88… ≠ ddb9d6bb… 双向失稳的反例锚点）。
    #[test]
    fn golden_same_issue_different_wording_shares_identity() {
        let node_007 = classified("timeline_node_007", 0);
        let node_012 = classified("timeline_node_012", 0);
        assert!(!node_007.identity_unstable);
        assert!(!node_012.identity_unstable);
        assert_eq!(
            node_007.fingerprint, node_012.fingerprint,
            "同题异措辞必须 collapse 到同一结构化身份"
        );
    }

    /// 场景 4（无稳定 ID fail-safe）：node_027 四条 must_fix 的
    /// contract_field 均为 `output_contracts[0].capabilities`（无 WI/CT/AC
    /// 稳定 ID）——全部标记 unstable、彼此不 collapse（同文可检测、异文互
    /// 异），且不与任何稳定域指纹相等（盐隔离）。
    #[test]
    fn golden_unstable_findings_stay_distinct_and_never_collide_with_stable() {
        let mut unstable_fingerprints = Vec::new();
        for index in 0..4 {
            let finding = classified("timeline_node_027", index);
            assert!(
                finding.identity_unstable,
                "027[{index}] 无稳定 ID，必须标记 unstable"
            );
            unstable_fingerprints.push(finding.fingerprint);
        }
        let distinct = unstable_fingerprints
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        assert_eq!(distinct.len(), 4, "四条不同措辞必须互不 collapse");

        let stable = [
            classified("timeline_node_007", 0).fingerprint,
            classified("timeline_node_012", 0).fingerprint,
            classified("timeline_node_029", 0).fingerprint,
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();
        for fingerprint in &unstable_fingerprints {
            assert!(
                !stable.contains(fingerprint),
                "unstable 域与稳定域盐隔离，永不互撞：{:?}",
                fingerprint
            );
        }
    }

    /// 场景 5（异题不撞）：node_029 三条稳定身份（{WI-003,AC-023}、
    /// {CT-003}、4×WI done_when_refs）互异。
    #[test]
    fn golden_distinct_stable_identities_do_not_collide() {
        let identities = [
            classified("timeline_node_029", 0),
            classified("timeline_node_029", 1),
            classified("timeline_node_029", 2),
        ];
        for finding in &identities {
            assert!(!finding.identity_unstable);
        }
        let fingerprints: BTreeSet<_> = identities.iter().map(|f| f.fingerprint.clone()).collect();
        assert_eq!(
            fingerprints.len(),
            3,
            "异题稳定身份互不 collapse：{fingerprints:?}"
        );
    }

    /// legacy（category=None）分支保持 class+message+field 口径不变（回归
    /// 锚点：durable 旧指纹在 legacy 流继续可比）。
    #[test]
    fn golden_legacy_branch_keeps_legacy_hash_shape() {
        let (legacy, unstable) = FindingFingerprint::identity_for_finding(
            None,
            FindingClass::Repairable,
            "Missing field",
            Some("acceptance_criteria"),
        );
        assert!(!unstable, "legacy 分支不标记 unstable");
        assert_eq!(
            legacy,
            FindingFingerprint::for_finding(
                None,
                FindingClass::Repairable,
                "Missing field",
                Some("acceptance_criteria")
            ),
            "legacy 分支哈希形状必须与既有实现逐位一致"
        );
    }

    /// 身份 API 直测：无稳定 ID → unstable（同文同指纹、异文互异）；
    /// 有稳定 ID → 措辞/大小写/分隔变化不改变指纹。
    #[test]
    fn identity_for_finding_splits_stable_and_unstable_domains() {
        let (unstable_a, is_unstable) = FindingFingerprint::identity_for_finding(
            Some(ReviewFindingCategory::ContractGap),
            FindingClass::Repairable,
            "message A",
            Some("output_contracts[0].capabilities"),
        );
        assert!(is_unstable);
        let (same_wording, _) = FindingFingerprint::identity_for_finding(
            Some(ReviewFindingCategory::ContractGap),
            FindingClass::Repairable,
            "message A",
            Some("output_contracts[0].capabilities"),
        );
        assert_eq!(unstable_a, same_wording, "同文重复仍同指纹（可检测）");
        let (other_wording, _) = FindingFingerprint::identity_for_finding(
            Some(ReviewFindingCategory::ContractGap),
            FindingClass::Repairable,
            "message B",
            Some("output_contracts[0].capabilities"),
        );
        assert_ne!(unstable_a, other_wording);

        let (stable, is_unstable) = FindingFingerprint::identity_for_finding(
            Some(ReviewFindingCategory::ContractGap),
            FindingClass::Repairable,
            "message A",
            Some("WI-001.output_contracts[CT-001].capabilities"),
        );
        assert!(!is_unstable);
        let (restyled, _) = FindingFingerprint::identity_for_finding(
            Some(ReviewFindingCategory::ContractGap),
            FindingClass::HumanRequired,
            "完全不同的措辞与 class",
            Some("output_contracts[WI-001].capabilities/CT-001"),
        );
        assert_eq!(
            stable, restyled,
            "稳定域哈希只看 category+ID 集+尾段，措辞与 class 不参与"
        );
    }
}
