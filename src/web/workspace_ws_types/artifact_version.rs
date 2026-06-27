use serde::{Deserialize, Serialize};

use crate::product::models::ProviderName;

use super::artifact::ArtifactPayload;
use super::review::ReviewVerdictType;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactVersion {
    pub version: u32,
    #[serde(flatten)]
    pub payload: ArtifactPayload,
    pub generated_by: ProviderName,
    pub reviewed_by: Option<ProviderName>,
    pub review_verdict: Option<ReviewVerdictType>,
    pub confirmed_by: Option<String>,
    #[serde(default = "default_true")]
    pub is_current: bool,
    pub created_at: String,
    pub source_node_id: String,
}

impl ArtifactVersion {
    pub fn markdown(&self) -> &str {
        self.payload.markdown_or_empty()
    }

    pub fn to_markdown_string(&self) -> String {
        self.payload.markdown_or_empty().to_string()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactVersionSummary {
    pub version: u32,
    pub generated_by: ProviderName,
    pub reviewed_by: Option<ProviderName>,
    pub review_verdict: Option<ReviewVerdictType>,
    pub confirmed_by: Option<String>,
    pub is_current: bool,
    pub created_at: String,
    pub source_node_id: String,
    pub markdown_size: usize,
    pub markdown_preview: String,
}

fn default_true() -> bool {
    true
}
