use super::*;

#[test]
fn artifact_version_roundtrips_with_markdown_payload() {
    let version = ArtifactVersion {
        version: 1,
        payload: ArtifactPayload::Markdown {
            markdown: "# Artifact version\n".to_string(),
            diff: Some("diff".to_string()),
        },
        generated_by: ProviderName::ClaudeCode,
        reviewed_by: None,
        review_verdict: None,
        confirmed_by: None,
        is_current: true,
        created_at: "2026-06-01T00:00:00Z".to_string(),
        source_node_id: "node_001".to_string(),
    };
    let json = serde_json::to_value(&version).unwrap();
    assert_eq!(json["markdown"], "# Artifact version\n");
    assert_eq!(json["diff"], "diff");
    assert!(!json.as_object().unwrap().contains_key("payload"));

    let back: ArtifactVersion = serde_json::from_value(json).unwrap();
    assert_eq!(back, version);
}
