use std::fmt;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DefaultParams {
    pub size: ImageSize,
    pub quality: ImageQuality,
    pub background: ImageBackground,
    pub output_format: ImageOutputFormat,
}

/// 图片供应方（「家」）。决定请求字段集与响应解析。
/// 一个模型 id 属于哪一家，只由它被配置在哪一家的清单里决定，绝不从 id 字符串推断。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ImageVendor {
    Gpt,
    Minimax,
}

/// 内置推荐清单：仅作为默认值，用户可在设置中增删。
pub const DEFAULT_GPT_MODELS: [&str; 8] = [
    "gpt-image-2.6",
    "gpt-image-2.6-flare",
    "gpt-image-2.6-flare-2k",
    "gpt-image-2.6-flare-4k",
    "gpt-image-2.6-sunburst",
    "gpt-image-2.6-sunburst-2k",
    "gpt-image-2.6-sunburst-4k",
    "gpt-image-2.5",
];
pub const DEFAULT_GPT_ROTATION_MODEL: &str = "gpt-image-2.6-flare";
pub const DEFAULT_MINIMAX_MODELS: [&str; 1] = ["MiniMax-H3-Image"];
pub const DEFAULT_MINIMAX_ROTATION_MODEL: &str = "MiniMax-H3-Image";

pub const MAX_MODELS_PER_VENDOR: usize = 32;
pub const MAX_MODEL_ID_CHARS: usize = 128;
/// 生成请求 `model` 字段的保留值：自动轮转。不得作为模型 id。
pub const AUTO_MODEL: &str = "auto";

impl ImageVendor {
    pub const ALL: [ImageVendor; 2] = [ImageVendor::Gpt, ImageVendor::Minimax];

    /// 另一家。
    pub fn other(self) -> ImageVendor {
        match self {
            ImageVendor::Gpt => ImageVendor::Minimax,
            ImageVendor::Minimax => ImageVendor::Gpt,
        }
    }

    /// 面向用户的名称："GPT" / "MiniMax"。
    pub fn label(self) -> &'static str {
        match self {
            ImageVendor::Gpt => "GPT",
            ImageVendor::Minimax => "MiniMax",
        }
    }

    /// 该家的内置推荐清单。
    pub fn recommended_models(self) -> Vec<String> {
        match self {
            ImageVendor::Gpt => DEFAULT_GPT_MODELS.iter().map(|m| m.to_string()).collect(),
            ImageVendor::Minimax => DEFAULT_MINIMAX_MODELS
                .iter()
                .map(|m| m.to_string())
                .collect(),
        }
    }

    /// 该家的内置默认轮转模型。
    pub fn recommended_rotation_model(self) -> &'static str {
        match self {
            ImageVendor::Gpt => DEFAULT_GPT_ROTATION_MODEL,
            ImageVendor::Minimax => DEFAULT_MINIMAX_ROTATION_MODEL,
        }
    }
}

/// 单家配置。Debug 手写：api_key 输出为 "<redacted>"。
#[derive(Clone, PartialEq, Eq, Serialize)]
pub struct VendorSettings {
    pub base_url: String,
    pub api_key: String,
    /// 用户可配置的模型 id 清单（不透明字符串）。
    pub models: Vec<String>,
    /// 自动轮转轮到本家时使用的模型。
    pub default_model: String,
}

impl VendorSettings {
    /// 凭据为空、清单与轮转模型为该家内置默认值。
    pub fn recommended(vendor: ImageVendor) -> Self {
        Self {
            base_url: String::new(),
            api_key: String::new(),
            models: vendor.recommended_models(),
            default_model: vendor.recommended_rotation_model().to_string(),
        }
    }

    /// base_url 与 api_key 去空白后均非空。
    pub fn has_credentials(&self) -> bool {
        !self.base_url.trim().is_empty() && !self.api_key.trim().is_empty()
    }

    /// has_credentials 且 models 非空。
    pub fn is_available(&self) -> bool {
        self.has_credentials() && !self.models.is_empty()
    }

    /// default_model 在清单内则取它，否则取清单首项；清单为空返回 None。
    pub fn rotation_model(&self) -> Option<&str> {
        self.models
            .iter()
            .find(|model| model.as_str() == self.default_model)
            .map(String::as_str)
            .or_else(|| self.models.first().map(String::as_str))
    }
}

impl fmt::Debug for VendorSettings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VendorSettings")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .field("models", &self.models)
            .field("default_model", &self.default_model)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VendorsSettings {
    pub gpt: VendorSettings,
    pub minimax: VendorSettings,
}

impl VendorsSettings {
    pub fn get(&self, vendor: ImageVendor) -> &VendorSettings {
        match vendor {
            ImageVendor::Gpt => &self.gpt,
            ImageVendor::Minimax => &self.minimax,
        }
    }

    pub fn get_mut(&mut self, vendor: ImageVendor) -> &mut VendorSettings {
        match vendor {
            ImageVendor::Gpt => &mut self.gpt,
            ImageVendor::Minimax => &mut self.minimax,
        }
    }

    /// 逐字符相等查找模型 id 所属的家。
    pub fn vendor_of(&self, model: &str) -> Option<ImageVendor> {
        ImageVendor::ALL.into_iter().find(|vendor| {
            self.get(*vendor)
                .models
                .iter()
                .any(|candidate| candidate.as_str() == model)
        })
    }
}

/// 持久化于 `.aria/image_create.json`。
/// 序列化只输出新形态；反序列化经 ImageCreateSettingsWire 兼容旧形态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "ImageCreateSettingsWire")]
pub struct ImageCreateSettings {
    pub vendors: VendorsSettings,
    pub defaults: DefaultParams,
}

impl Default for ImageCreateSettings {
    fn default() -> Self {
        Self {
            vendors: VendorsSettings {
                gpt: VendorSettings::recommended(ImageVendor::Gpt),
                minimax: VendorSettings::recommended(ImageVendor::Minimax),
            },
            defaults: DefaultParams::default(),
        }
    }
}

/// 仅用于反序列化：同时接受新形态与旧形态（顶层 base_url / api_key）。
/// 不 derive Debug：这三个 wire 结构内含 api_key 明文，Debug 输出会泄漏（见 D12.7）。
#[derive(Default, Deserialize)]
struct ImageCreateSettingsWire {
    #[serde(default)]
    vendors: Option<VendorsWire>,
    /// 旧形态顶层字段：仅当 vendors 缺失时归入 GPT 家。
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    defaults: DefaultParams,
}

#[derive(Default, Deserialize)]
struct VendorsWire {
    #[serde(default)]
    gpt: VendorWire,
    #[serde(default)]
    minimax: VendorWire,
}

#[derive(Default, Deserialize)]
struct VendorWire {
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    api_key: String,
    /// None = 文件未写该键，补内置默认清单；Some(vec![]) = 用户清空，保持为空。
    #[serde(default)]
    models: Option<Vec<String>>,
    /// None = 文件未写该键：清单非空时补该家内置轮转模型，清单为空时为空串。
    #[serde(default)]
    default_model: Option<String>,
}

impl VendorWire {
    /// 按 D3 规则合并：缺失项补该家内置默认值。
    fn into_settings(self, vendor: ImageVendor) -> VendorSettings {
        let models = match self.models {
            // Some(vec![]) = 用户主动清空，保持为空。
            Some(models) => models,
            None => vendor.recommended_models(),
        };
        let default_model = match self.default_model {
            Some(value) => value,
            None => {
                if models.is_empty() {
                    String::new()
                } else {
                    vendor.recommended_rotation_model().to_string()
                }
            }
        };
        VendorSettings {
            base_url: self.base_url,
            api_key: self.api_key,
            models,
            default_model,
        }
    }
}

impl From<ImageCreateSettingsWire> for ImageCreateSettings {
    fn from(wire: ImageCreateSettingsWire) -> Self {
        let vendors = match wire.vendors {
            Some(vendors) => VendorsSettings {
                gpt: vendors.gpt.into_settings(ImageVendor::Gpt),
                minimax: vendors.minimax.into_settings(ImageVendor::Minimax),
            },
            // 旧形态：顶层凭据此前只用于调 GPT 模型，归入 GPT 家语义无损。
            None => VendorsSettings {
                gpt: VendorSettings {
                    base_url: wire.base_url.unwrap_or_default(),
                    api_key: wire.api_key.unwrap_or_default(),
                    models: ImageVendor::Gpt.recommended_models(),
                    default_model: DEFAULT_GPT_ROTATION_MODEL.to_string(),
                },
                minimax: VendorSettings::recommended(ImageVendor::Minimax),
            },
        };
        Self {
            vendors,
            defaults: wire.defaults,
        }
    }
}

/// 脱敏视图（GET / PUT 设置接口的响应体）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaskedSettings {
    pub vendors: MaskedVendors,
    pub defaults: DefaultParams,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaskedVendors {
    pub gpt: MaskedVendorSettings,
    pub minimax: MaskedVendorSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaskedVendorSettings {
    pub base_url: String,
    pub api_key_masked: String,
    pub models: Vec<String>,
    pub default_model: String,
    /// 内置推荐清单，供前端「恢复推荐清单」。
    pub recommended_models: Vec<String>,
    /// base_url、api_key、models 均非空。
    pub available: bool,
}

/// 生成请求中的模型选择。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelChoice {
    Auto,
    Explicit(String),
}

impl ModelChoice {
    /// 缺省、空白、"auto"（不区分大小写）→ Auto；其余 → Explicit(去首尾空白后的原值)。
    pub fn parse(raw: Option<&str>) -> ModelChoice {
        let Some(raw) = raw else {
            return ModelChoice::Auto;
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case(AUTO_MODEL) {
            ModelChoice::Auto
        } else {
            ModelChoice::Explicit(trimmed.to_string())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelSelection {
    Auto,
    Explicit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReferenceSource {
    None,
    Uploaded,
    Previous,
}

/// 出站前解析出的目标：哪家、哪个模型、怎么选出来的。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedTarget {
    pub vendor: ImageVendor,
    pub model: String,
    pub selection: ModelSelection,
}

impl ImageCreateSettings {
    /// 纯函数，不做 I/O。
    /// last_vendor：会话上一条成功结果所属的家；无结果为 None；
    /// 旧记录（无 vendor 字段）由调用方传 Some(ImageVendor::Gpt)。
    pub fn resolve_target(
        &self,
        choice: &ModelChoice,
        last_vendor: Option<ImageVendor>,
    ) -> Result<ResolvedTarget, ImageCreateError> {
        match choice {
            ModelChoice::Explicit(id) => {
                let Some(vendor) = self.vendors.vendor_of(id) else {
                    return Err(ImageCreateError::InvalidParam(format!(
                        "模型 {id} 不在已配置的模型清单中，请在设置中添加或改选其他模型"
                    )));
                };
                if !self.vendors.get(vendor).has_credentials() {
                    return Err(ImageCreateError::MissingConfig(format!(
                        "{} 尚未配置 Base URL 或 API Key",
                        vendor.label()
                    )));
                }
                Ok(ResolvedTarget {
                    vendor,
                    model: id.clone(),
                    selection: ModelSelection::Explicit,
                })
            }
            ModelChoice::Auto => {
                let preferred = match last_vendor {
                    None => ImageVendor::Gpt,
                    Some(vendor) => vendor.other(),
                };
                let picked = if self.vendors.get(preferred).is_available() {
                    preferred
                } else if self.vendors.get(preferred.other()).is_available() {
                    preferred.other()
                } else {
                    return Err(ImageCreateError::MissingConfig(
                        "尚未配置任何可用的图片生成端点".to_string(),
                    ));
                };
                let Some(model) = self.vendors.get(picked).rotation_model() else {
                    return Err(ImageCreateError::MissingConfig(
                        "尚未配置任何可用的图片生成端点".to_string(),
                    ));
                };
                Ok(ResolvedTarget {
                    vendor: picked,
                    model: model.to_string(),
                    selection: ModelSelection::Auto,
                })
            }
        }
    }

    /// 保存前整份校验：D1 规则 1–5（清单与轮转模型归属）+ 逐家 base_url。
    pub fn validate(&self) -> Result<(), ImageCreateError> {
        for vendor in ImageVendor::ALL {
            let label = vendor.label();
            let settings = self.vendors.get(vendor);

            if settings.models.len() > MAX_MODELS_PER_VENDOR {
                return Err(ImageCreateError::InvalidConfig(format!(
                    "{label} 模型清单最多 {MAX_MODELS_PER_VENDOR} 个"
                )));
            }

            let mut seen = std::collections::HashSet::new();
            for id in &settings.models {
                let trimmed = id.trim();
                if trimmed.is_empty()
                    || trimmed.chars().count() > MAX_MODEL_ID_CHARS
                    || !trimmed.chars().all(|c| {
                        c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '-')
                    })
                {
                    return Err(ImageCreateError::InvalidConfig(format!(
                        "{label} 模型 id「{id}」不合法：长度需为 1–128，且只能包含字母、数字与 . _ : / -"
                    )));
                }
                if trimmed.eq_ignore_ascii_case(AUTO_MODEL) {
                    return Err(ImageCreateError::InvalidConfig(format!(
                        "{label} 模型 id 不能为 auto"
                    )));
                }
                if !seen.insert(trimmed) {
                    return Err(ImageCreateError::InvalidConfig(format!(
                        "{label} 模型清单存在重复项：{id}"
                    )));
                }
            }

            let default_in_catalog = settings
                .models
                .iter()
                .any(|model| model.as_str() == settings.default_model);
            if (!settings.models.is_empty() && !default_in_catalog)
                || (settings.models.is_empty() && !settings.default_model.is_empty())
            {
                return Err(ImageCreateError::InvalidConfig(format!(
                    "{label} 的轮转模型「{}」不在其模型清单中",
                    settings.default_model
                )));
            }

            let base_url = settings.base_url.trim();
            if !base_url.is_empty() {
                crate::cross_cutting::image_client::validate_base_url(base_url)
                    .map_err(|error| ImageCreateError::InvalidConfig(error.to_string()))?;
            }
        }

        for id in &self.vendors.gpt.models {
            // 跨家去重与家内去重同一范式：两侧均取 trim 后比较，避免手改配置用首尾空白绕过唯一性。
            if self
                .vendors
                .minimax
                .models
                .iter()
                .any(|model| model.trim() == id.trim())
            {
                return Err(ImageCreateError::InvalidConfig(format!(
                    "模型 id「{id}」同时出现在 GPT 与 MiniMax 清单中"
                )));
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyUpdate {
    Retain,
    Replace(String),
    Clear,
}

/// 单家的更新意图（内部形态）。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VendorUpdate {
    pub base_url: Option<String>,
    pub api_key: ApiKeyUpdate,
    pub models: Option<Vec<String>>,
    pub default_model: Option<String>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsUpdate {
    pub gpt: Option<VendorUpdate>,
    pub minimax: Option<VendorUpdate>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vendor: Option<ImageVendor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection: Option<ModelSelection>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference_source: Option<ReferenceSource>,
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
            vendor: result.vendor,
            model: result.model,
            selection: result.selection,
            reference_source: result.reference_source,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vendor: Option<ImageVendor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<ModelSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference_source: Option<ReferenceSource>,
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

/// PUT 设置接口的请求体。
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsUpdateRequest {
    #[serde(default)]
    pub vendors: Option<VendorsUpdateRequest>,
    #[serde(default)]
    pub defaults: Option<DefaultParams>,
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VendorsUpdateRequest {
    #[serde(default)]
    pub gpt: Option<VendorUpdateRequest>,
    #[serde(default)]
    pub minimax: Option<VendorUpdateRequest>,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VendorUpdateRequest {
    #[serde(default)]
    pub base_url: Option<String>,
    /// 缺省 = retain。
    #[serde(default)]
    pub api_key_action: ApiKeyAction,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub models: Option<Vec<String>>,
    #[serde(default)]
    pub default_model: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyAction {
    #[default]
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
    /// 取代原 validate_base_url：对合并后的整份设置校验。
    async fn validate_settings(
        &self,
        settings: &ImageCreateSettings,
    ) -> Result<(), ImageCreateError>;
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
    #[error("image config missing: {0}")]
    MissingConfig(String),
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
    /// 携带上一张图失败（出站前）。
    #[error(
        "上一张图片不可用，无法基于它继续修改（{0}）；请关闭「基于上一张图继续修改」或上传参考图"
    )]
    PreviousImageUnavailable(String),
}

pub fn validate_session_id(id: &str) -> Result<(), ImageCreateError> {
    crate::product::json_store::validate_relative_id(id)
        .map_err(|_| ImageCreateError::InvalidSessionId(id.to_string()))
}

#[cfg(test)]
#[path = "models_tests.rs"]
mod tests;
