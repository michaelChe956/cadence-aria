use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::product::models::ProviderName;
use crate::protocol::contracts::ProviderType;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ImageSize {
    #[serde(rename = "1024x1024")]
    Square,
    #[serde(rename = "1536x1024")]
    Landscape,
    #[serde(rename = "1024x1536")]
    Portrait,
    #[default]
    #[serde(rename = "auto")]
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ImageQuality {
    #[serde(rename = "low")]
    Low,
    #[serde(rename = "medium")]
    Medium,
    #[serde(rename = "high")]
    High,
    #[default]
    #[serde(rename = "auto")]
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ImageBackground {
    #[serde(rename = "transparent")]
    Transparent,
    #[serde(rename = "opaque")]
    Opaque,
    #[default]
    #[serde(rename = "auto")]
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ImageOutputFormat {
    #[default]
    #[serde(rename = "png")]
    Png,
    #[serde(rename = "jpeg")]
    Jpeg,
    #[serde(rename = "webp")]
    Webp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum InputFidelity {
    #[serde(rename = "low")]
    Low,
    #[serde(rename = "high")]
    High,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DefaultParams {
    pub size: ImageSize,
    pub quality: ImageQuality,
    pub background: ImageBackground,
    pub output_format: ImageOutputFormat,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ImageCreateSettings {
    pub base_url: String,
    pub api_key: String,
    pub defaults: DefaultParams,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaskedSettings {
    pub base_url: String,
    pub api_key_masked: String,
    pub defaults: DefaultParams,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyUpdate {
    Retain,
    Replace(String),
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsUpdate {
    pub base_url: Option<String>,
    pub api_key: ApiKeyUpdate,
    pub defaults: Option<DefaultParams>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PresetTemplate {
    PptBusinessIllustration,
    BusinessFlowDiagram,
    WebPageUi,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateChoice {
    pub preset: Option<PresetTemplate>,
    pub custom: Option<String>,
}

impl From<ProviderName> for ProviderType {
    fn from(value: ProviderName) -> Self {
        match value {
            ProviderName::ClaudeCode => Self::ClaudeCode,
            ProviderName::Codex => Self::Codex,
            ProviderName::Pi => Self::Pi,
            ProviderName::KimiCode => Self::KimiCode,
            ProviderName::Fake => Self::Fake,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageCreateSession {
    pub id: String,
    pub provider_name: ProviderName,
    pub template: TemplateChoice,
    pub last_provider_session_id: Option<String>,
    pub current_prompt: Option<String>,
    pub status: SessionStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session: ImageCreateSession,
    pub messages: Vec<ChatMessage>,
    pub prompt_blocks: Vec<PromptBlock>,
    pub generation_results: Vec<GenerationResult>,
    pub events: Vec<SessionEvent>,
    pub generation: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Deleting,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionSummary {
    pub id: String,
    pub provider_name: ProviderName,
    pub template: TemplateChoice,
    pub status: SessionStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct GenerationResultDto {
    pub prompt: String,
    pub params: DefaultParams,
    pub media_type: String,
    pub image_id: Option<String>,
    pub legacy_pending: bool,
    pub ts: DateTime<Utc>,
}

impl From<GenerationResult> for GenerationResultDto {
    fn from(result: GenerationResult) -> Self {
        Self {
            prompt: result.prompt,
            params: result.params,
            media_type: result.media_type,
            image_id: result.image_id,
            legacy_pending: result.b64.is_some(),
            ts: result.ts,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SessionRecordDto {
    pub session: ImageCreateSession,
    pub messages: Vec<ChatMessage>,
    pub prompt_blocks: Vec<PromptBlock>,
    pub generation_results: Vec<GenerationResultDto>,
    pub events: Vec<SessionEvent>,
    pub generation: u64,
}

impl From<SessionRecord> for SessionRecordDto {
    fn from(record: SessionRecord) -> Self {
        Self {
            session: record.session,
            messages: record.messages,
            prompt_blocks: record.prompt_blocks,
            generation_results: record
                .generation_results
                .into_iter()
                .map(GenerationResultDto::from)
                .collect(),
            events: record.events,
            generation: record.generation,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SessionSummaryDto {
    pub id: String,
    pub provider_name: ProviderName,
    pub template: TemplateChoice,
    pub status: SessionStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<SessionSummary> for SessionSummaryDto {
    fn from(summary: SessionSummary) -> Self {
        Self {
            id: summary.id,
            provider_name: summary.provider_name,
            template: summary.template,
            status: summary.status,
            created_at: summary.created_at,
            updated_at: summary.updated_at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
    pub ts: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptBlock {
    pub content: String,
    pub version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationResult {
    pub prompt: String,
    pub params: DefaultParams,
    pub media_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub b64: Option<String>,
    pub ts: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionEvent {
    pub kind: String,
    pub message: String,
    pub ts: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunKind {
    Iteration,
    Generate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IterationEvent {
    pub kind: String,
    pub text: Option<String>,
    pub suggested_prompt: Option<String>,
    pub provider_session_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsUpdateRequest {
    pub base_url: Option<String>,
    pub api_key_action: ApiKeyAction,
    pub api_key: Option<String>,
    pub defaults: Option<DefaultParams>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyAction {
    Retain,
    Replace,
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateSessionRequest {
    pub template: TemplateChoice,
    pub provider_name: ProviderName,
}

#[derive(Debug, Clone)]
pub struct DeleteLease {
    pub id: String,
    pub token: u64,
}

#[async_trait]
pub trait SessionStoreApi: Send + Sync {
    async fn create(
        &self,
        req: CreateSessionRequest,
    ) -> Result<ImageCreateSession, ImageCreateError>;
    async fn list(&self) -> Result<Vec<SessionSummary>, ImageCreateError>;
    async fn get(&self, id: &str) -> Result<Option<SessionRecord>, ImageCreateError>;
    async fn append_message(&self, id: &str, msg: ChatMessage) -> Result<(), ImageCreateError>;
    async fn append_prompt_block(
        &self,
        id: &str,
        block: PromptBlock,
    ) -> Result<(), ImageCreateError>;
    async fn append_generation_result(
        &self,
        id: &str,
        result: GenerationResult,
    ) -> Result<(), ImageCreateError>;
    async fn append_event(&self, id: &str, event: SessionEvent) -> Result<(), ImageCreateError>;
    async fn update_session_meta(
        &self,
        id: &str,
        current_prompt: Option<String>,
        last_provider_session_id: Option<String>,
    ) -> Result<(), ImageCreateError>;
    async fn begin_delete(&self, id: &str) -> Result<Option<DeleteLease>, ImageCreateError>;
    async fn finish_delete(&self, lease: DeleteLease) -> Result<(), ImageCreateError>;
}

#[async_trait]
pub trait SettingsStoreApi: Send + Sync {
    async fn load(&self) -> ImageCreateSettings;
    async fn save(&self, settings: &ImageCreateSettings) -> Result<(), ImageCreateError>;
    async fn to_masked(&self, settings: &ImageCreateSettings) -> MaskedSettings;
    async fn validate_base_url(&self, url: &str) -> Result<(), ImageCreateError>;
    async fn apply_update(
        &self,
        current: &ImageCreateSettings,
        update: SettingsUpdate,
    ) -> ImageCreateSettings;
    #[allow(clippy::wrong_self_convention)]
    async fn from_request(&self, req: SettingsUpdateRequest) -> SettingsUpdate;
}

#[derive(Debug, thiserror::Error)]
pub enum ImageCreateError {
    #[error("session not found")]
    SessionNotFound,
    #[error("session is closing")]
    SessionClosing,
    #[error("session busy")]
    SessionBusy,
    #[error("session gone")]
    SessionGone,
    #[error("invalid session id: {0}")]
    InvalidSessionId(String),
    #[error("image config missing")]
    MissingConfig,
    #[error("invalid config: {0}")]
    InvalidConfig(String),
    #[error("invalid parameter: {0}")]
    InvalidParam(String),
    #[error("store error: {0}")]
    Store(String),
    #[error("image client error: {0}")]
    ImageClient(String),
    #[error("iteration error: {0}")]
    Iteration(String),
    #[error("reference image error: {0}")]
    RefImage(String),
}

pub fn validate_session_id(id: &str) -> Result<(), ImageCreateError> {
    crate::product::json_store::validate_relative_id(id)
        .map_err(|_| ImageCreateError::InvalidSessionId(id.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Serialize, de::DeserializeOwned};

    fn assert_json_round_trip<T>(value: T, expected: &str)
    where
        T: Serialize + DeserializeOwned + PartialEq + std::fmt::Debug,
    {
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(json, format!("\"{expected}\""));
        assert_eq!(serde_json::from_str::<T>(&json).unwrap(), value);
    }

    #[test]
    fn parameter_enums_round_trip_with_exact_wire_values() {
        assert_json_round_trip(ImageSize::Square, "1024x1024");
        assert_json_round_trip(ImageSize::Landscape, "1536x1024");
        assert_json_round_trip(ImageSize::Portrait, "1024x1536");
        assert_json_round_trip(ImageSize::Auto, "auto");

        assert_json_round_trip(ImageQuality::Low, "low");
        assert_json_round_trip(ImageQuality::Medium, "medium");
        assert_json_round_trip(ImageQuality::High, "high");
        assert_json_round_trip(ImageQuality::Auto, "auto");

        assert_json_round_trip(ImageBackground::Transparent, "transparent");
        assert_json_round_trip(ImageBackground::Opaque, "opaque");
        assert_json_round_trip(ImageBackground::Auto, "auto");

        assert_json_round_trip(ImageOutputFormat::Png, "png");
        assert_json_round_trip(ImageOutputFormat::Jpeg, "jpeg");
        assert_json_round_trip(ImageOutputFormat::Webp, "webp");

        assert_json_round_trip(InputFidelity::Low, "low");
        assert_json_round_trip(InputFidelity::High, "high");
    }

    #[test]
    fn api_key_update_is_discriminable() {
        assert!(
            serde_json::to_string(&ApiKeyUpdate::Retain)
                .unwrap()
                .contains("retain")
        );
        let c = serde_json::to_string(&ApiKeyUpdate::Clear).unwrap();
        assert!(c.contains("clear"));
        let r = serde_json::to_string(&ApiKeyUpdate::Replace("sk-new".into())).unwrap();
        assert!(r.contains("replace") && r.contains("sk-new"));
    }

    #[test]
    fn validate_session_id_rejects_traversal() {
        assert!(validate_session_id("../x").is_err());
        assert!(validate_session_id("/abs").is_err());
        assert!(validate_session_id("").is_err());
        assert!(validate_session_id("a/b").is_err());
        assert!(validate_session_id("a\\b").is_err());
        assert!(validate_session_id("valid-id-1").is_ok());
    }

    #[test]
    fn generation_result_serializes_reference_form_without_b64() {
        let result = GenerationResult {
            prompt: "p".to_string(),
            params: DefaultParams::default(),
            media_type: "image/png".to_string(),
            image_id: Some("0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0".to_string()),
            b64: None,
            ts: Utc::now(),
        };
        let json = serde_json::to_string(&result).expect("serialize");
        assert!(json.contains("image_id"));
        assert!(!json.contains("b64"));
    }

    #[test]
    fn generation_result_deserializes_legacy_inline_b64() {
        let legacy = r#"{
            "prompt": "p",
            "params": {"size": "auto", "quality": "auto", "background": "auto", "output_format": "png"},
            "media_type": "image/png",
            "b64": "aGVsbG8=",
            "ts": "2026-08-26T00:00:00Z"
        }"#;
        let result: GenerationResult = serde_json::from_str(legacy).expect("deserialize legacy");
        assert_eq!(result.b64.as_deref(), Some("aGVsbG8="));
        assert_eq!(result.image_id, None);
    }
}
