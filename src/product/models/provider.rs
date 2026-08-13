use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderName {
    ClaudeCode,
    Codex,
    Pi,
    KimiCode,
    Fake,
}

#[cfg(test)]
mod tests {
    use super::ProviderName;

    #[test]
    fn provider_name_pi_serializes_to_snake_case() {
        assert_eq!(serde_json::to_string(&ProviderName::Pi).unwrap(), "\"pi\"");
        let back: ProviderName = serde_json::from_str("\"pi\"").unwrap();
        assert_eq!(back, ProviderName::Pi);
    }

    #[test]
    fn provider_name_kimi_code_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_string(&ProviderName::KimiCode).unwrap(),
            "\"kimi_code\""
        );
        let back: ProviderName = serde_json::from_str("\"kimi_code\"").unwrap();
        assert_eq!(back, ProviderName::KimiCode);
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderConversationRole {
    Author,
    Reviewer,
    Coder,
    Analyst,
    CodeReviewer,
    InternalReviewer,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ProviderConversationRef {
    pub role: ProviderConversationRole,
    pub provider: ProviderName,
    pub provider_session_id: String,
    pub updated_at: String,
    pub last_node_id: Option<String>,
}
