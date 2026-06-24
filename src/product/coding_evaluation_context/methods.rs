use std::collections::BTreeMap;

use crate::product::coding_models::CodingProviderRole;

pub(super) fn required_methods_by_role() -> BTreeMap<String, Vec<String>> {
    BTreeMap::from([
        (
            role_key(&CodingProviderRole::Tester),
            vec![
                "systematic_debugging".to_string(),
                "verification_before_completion".to_string(),
            ],
        ),
        (
            role_key(&CodingProviderRole::Analyst),
            vec![
                "systematic_debugging".to_string(),
                "receiving_code_review".to_string(),
            ],
        ),
        (
            role_key(&CodingProviderRole::CodeReviewer),
            vec![
                "requesting_code_review".to_string(),
                "verification_before_completion".to_string(),
            ],
        ),
        (
            role_key(&CodingProviderRole::InternalReviewer),
            vec![
                "requesting_code_review".to_string(),
                "verification_before_completion".to_string(),
            ],
        ),
    ])
}

fn role_key(role: &CodingProviderRole) -> String {
    match role {
        CodingProviderRole::Coder => "coder",
        CodingProviderRole::Tester => "tester",
        CodingProviderRole::Analyst => "analyst",
        CodingProviderRole::CodeReviewer => "code_reviewer",
        CodingProviderRole::InternalReviewer => "internal_reviewer",
    }
    .to_string()
}
