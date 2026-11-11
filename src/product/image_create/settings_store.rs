use async_trait::async_trait;

use crate::cross_cutting::aria_state_paths::AriaStatePaths;
use crate::product::json_store::{read_json, write_json};

use super::models::{
    ApiKeyAction, ApiKeyUpdate, ImageCreateError, ImageCreateSettings, ImageVendor, MaskedSettings,
    MaskedVendorSettings, MaskedVendors, SettingsStoreApi, SettingsUpdate, SettingsUpdateRequest,
    VendorSettings, VendorUpdate, VendorUpdateRequest, VendorsSettings,
};

#[derive(Debug, Clone)]
pub struct SettingsStore {
    paths: AriaStatePaths,
}

impl SettingsStore {
    pub fn new(paths: AriaStatePaths) -> Self {
        Self { paths }
    }
}

pub fn mask_api_key(api_key: &str) -> String {
    let chars = api_key.chars().collect::<Vec<_>>();
    if chars.len() <= 7 {
        return "*".repeat(chars.len());
    }

    let prefix = chars[..3].iter().collect::<String>();
    let suffix = chars[chars.len() - 4..].iter().collect::<String>();
    format!("{prefix}****{suffix}")
}

pub fn load(paths: &AriaStatePaths) -> ImageCreateSettings {
    let path = paths.image_create_settings_file();
    if !path.exists() {
        return ImageCreateSettings::default();
    }

    read_json(&path).unwrap_or_default()
}

pub fn save(
    paths: &AriaStatePaths,
    settings: &ImageCreateSettings,
) -> Result<(), ImageCreateError> {
    let path = paths.image_create_settings_file();
    write_json(&path, settings).map_err(|error| ImageCreateError::Store(error.to_string()))?;
    // H1：明文 key 所在文件只允许属主读写。仅收紧本文件，
    // 不改 write_json 的全局默认行为（会话文件等不受影响）。
    restrict_to_owner(&path)
}

/// 把设置文件权限收紧为 0600。Windows 无 POSIX 权限位，安全 no-op（不报错）。
fn restrict_to_owner(path: &std::path::Path) -> Result<(), ImageCreateError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|error| {
            ImageCreateError::Store(format!("收紧设置文件权限失败 {}: {error}", path.display()))
        })
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// 逐家脱敏，并填充 recommended_models 与 available。
pub fn to_masked(settings: &ImageCreateSettings) -> MaskedSettings {
    let mask_vendor = |vendor: ImageVendor, settings: &VendorSettings| MaskedVendorSettings {
        base_url: settings.base_url.clone(),
        api_key_masked: mask_api_key(&settings.api_key),
        models: settings.models.clone(),
        default_model: settings.default_model.clone(),
        recommended_models: vendor.recommended_models(),
        available: settings.is_available(),
    };
    MaskedSettings {
        vendors: MaskedVendors {
            gpt: mask_vendor(ImageVendor::Gpt, &settings.vendors.gpt),
            minimax: mask_vendor(ImageVendor::Minimax, &settings.vendors.minimax),
        },
        defaults: settings.defaults.clone(),
    }
}

/// 逐家合并：None 一律保留旧值；base_url / models 各项 / default_model 去首尾空白；
/// models 去空项。签名不可失败。
pub fn apply_update(current: &ImageCreateSettings, update: SettingsUpdate) -> ImageCreateSettings {
    let merge = |current: &VendorSettings, update: Option<&VendorUpdate>| -> VendorSettings {
        let Some(update) = update else {
            return current.clone();
        };
        VendorSettings {
            base_url: update
                .base_url
                .as_ref()
                .map(|value| value.trim().to_string())
                .unwrap_or_else(|| current.base_url.clone()),
            api_key: match &update.api_key {
                ApiKeyUpdate::Retain => current.api_key.clone(),
                ApiKeyUpdate::Replace(value) => value.clone(),
                ApiKeyUpdate::Clear => String::new(),
            },
            models: update
                .models
                .as_ref()
                .map(|items| {
                    items
                        .iter()
                        .map(|id| id.trim().to_string())
                        .filter(|id| !id.is_empty())
                        .collect()
                })
                .unwrap_or_else(|| current.models.clone()),
            default_model: update
                .default_model
                .as_ref()
                .map(|value| value.trim().to_string())
                .unwrap_or_else(|| current.default_model.clone()),
        }
    };

    ImageCreateSettings {
        vendors: VendorsSettings {
            gpt: merge(&current.vendors.gpt, update.gpt.as_ref()),
            minimax: merge(&current.vendors.minimax, update.minimax.as_ref()),
        },
        defaults: update
            .defaults
            .clone()
            .unwrap_or_else(|| current.defaults.clone()),
    }
}

/// 逐家转换三态；Replace 的取值语义沿用现状（缺值按空串替换）。
pub fn from_request(req: SettingsUpdateRequest) -> SettingsUpdate {
    let convert = |req: Option<VendorUpdateRequest>| -> Option<VendorUpdate> {
        req.map(|req| VendorUpdate {
            base_url: req.base_url,
            api_key: match req.api_key_action {
                ApiKeyAction::Retain => ApiKeyUpdate::Retain,
                ApiKeyAction::Clear => ApiKeyUpdate::Clear,
                ApiKeyAction::Replace => ApiKeyUpdate::Replace(req.api_key.unwrap_or_default()),
            },
            models: req.models,
            default_model: req.default_model,
        })
    };

    SettingsUpdate {
        gpt: convert(req.vendors.as_ref().and_then(|vendors| vendors.gpt.clone())),
        minimax: convert(req.vendors.and_then(|vendors| vendors.minimax)),
        defaults: req.defaults,
    }
}

/// 整份校验，错误一律为 InvalidConfig：逐家 base_url + 模型清单规则 1–5（D1），
/// 由 `ImageCreateSettings::validate` 统一承担。
pub fn validate_settings(settings: &ImageCreateSettings) -> Result<(), ImageCreateError> {
    settings.validate()
}

#[async_trait]
impl SettingsStoreApi for SettingsStore {
    async fn load(&self) -> ImageCreateSettings {
        load(&self.paths)
    }

    async fn save(&self, settings: &ImageCreateSettings) -> Result<(), ImageCreateError> {
        save(&self.paths, settings)
    }

    async fn to_masked(&self, settings: &ImageCreateSettings) -> MaskedSettings {
        to_masked(settings)
    }

    async fn validate_settings(
        &self,
        settings: &ImageCreateSettings,
    ) -> Result<(), ImageCreateError> {
        validate_settings(settings)
    }

    async fn apply_update(
        &self,
        current: &ImageCreateSettings,
        update: SettingsUpdate,
    ) -> ImageCreateSettings {
        apply_update(current, update)
    }

    async fn from_request(&self, req: SettingsUpdateRequest) -> SettingsUpdate {
        from_request(req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::image_create::models::{
        DefaultParams, ImageBackground, ImageOutputFormat, ImageQuality, ImageSize,
        VendorsUpdateRequest,
    };
    use tempfile::tempdir;

    fn gpt_vendor() -> VendorSettings {
        VendorSettings {
            base_url: "https://api.example.com/v1".to_string(),
            api_key: "sk-original1234".to_string(),
            models: vec!["gpt-image-2.6".to_string()],
            default_model: "gpt-image-2.6".to_string(),
        }
    }

    fn settings() -> ImageCreateSettings {
        ImageCreateSettings {
            vendors: VendorsSettings {
                gpt: gpt_vendor(),
                minimax: VendorSettings::recommended(ImageVendor::Minimax),
            },
            defaults: DefaultParams {
                size: ImageSize::Landscape,
                quality: ImageQuality::High,
                background: ImageBackground::Opaque,
                output_format: ImageOutputFormat::Webp,
            },
        }
    }

    #[test]
    fn masks_api_keys_without_exposing_the_middle() {
        assert_eq!(mask_api_key("sk-abcd1234"), "sk-****1234");
        assert_eq!(mask_api_key("short"), "*****");
        assert_eq!(mask_api_key(""), "");
    }

    #[test]
    fn saves_and_loads_settings_round_trip_and_defaults_when_missing() {
        let dir = tempdir().expect("tempdir");
        let paths = AriaStatePaths::from_workspace_root(dir.path());
        assert_eq!(load(&paths), ImageCreateSettings::default());

        let expected = settings();
        save(&paths, &expected).expect("save settings");

        assert_eq!(load(&paths), expected);
    }

    #[test]
    fn saved_settings_file_is_owner_only() {
        // H1：明文 key 所在的设置文件必须收紧为 0600（同机其他用户不可读）。
        // Unix 下断言权限位；Windows 无 POSIX 权限位，跳过断言（实现侧为安全 no-op）。
        let dir = tempdir().expect("tempdir");
        let paths = AriaStatePaths::from_workspace_root(dir.path());

        save(&paths, &settings()).expect("save settings");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(paths.image_create_settings_file())
                .expect("settings file metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "设置文件含明文 key，权限必须为 0600");
        }
    }

    #[test]
    fn converts_request_api_key_actions_per_vendor() {
        let replace = from_request(SettingsUpdateRequest {
            vendors: Some(VendorsUpdateRequest {
                gpt: Some(VendorUpdateRequest {
                    base_url: None,
                    api_key_action: ApiKeyAction::Replace,
                    api_key: Some("sk-replacement".into()),
                    models: None,
                    default_model: None,
                }),
                minimax: None,
            }),
            defaults: None,
        });
        assert_eq!(
            replace.gpt.map(|update| update.api_key),
            Some(ApiKeyUpdate::Replace("sk-replacement".into()))
        );
        // VendorUpdate 已不 derive Debug（H4），断言改用无 Debug 约束的形式。
        assert!(replace.minimax.is_none());

        // 现状语义：replace 缺值按空串替换。
        let missing_replace_value = from_request(SettingsUpdateRequest {
            vendors: Some(VendorsUpdateRequest {
                gpt: None,
                minimax: Some(VendorUpdateRequest {
                    base_url: None,
                    api_key_action: ApiKeyAction::Replace,
                    api_key: None,
                    models: None,
                    default_model: None,
                }),
            }),
            defaults: None,
        });
        assert_eq!(
            missing_replace_value.minimax.map(|update| update.api_key),
            Some(ApiKeyUpdate::Replace(String::new()))
        );
    }
}
