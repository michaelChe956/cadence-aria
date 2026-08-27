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
    let original = FindingFingerprint::for_finding(
        Some(ReviewFindingCategory::ContractGap),
        FindingClass::Repairable,
        "Missing acceptance criterion",
        Some("acceptance_criteria"),
    );

    assert_eq!(
        original,
        FindingFingerprint::for_finding(
            Some(ReviewFindingCategory::ContractGap),
            FindingClass::HumanRequired,
            "The completion criteria omit a required scenario",
            Some("acceptance_criteria"),
        )
    );
    assert_ne!(
        original,
        FindingFingerprint::for_finding(
            Some(ReviewFindingCategory::ScopeConflict),
            FindingClass::Repairable,
            "Missing acceptance criterion",
            Some("acceptance_criteria"),
        )
    );
    assert_ne!(
        original,
        FindingFingerprint::for_finding(
            Some(ReviewFindingCategory::ContractGap),
            FindingClass::Repairable,
            "Missing acceptance criterion",
            Some("scope"),
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
