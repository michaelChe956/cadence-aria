use std::net::IpAddr;

use async_trait::async_trait;

use crate::cross_cutting::aria_state_paths::AriaStatePaths;
use crate::product::json_store::{read_json, write_json};

use super::models::{
    ApiKeyAction, ApiKeyUpdate, ImageCreateError, ImageCreateSettings, MaskedSettings,
    SettingsStoreApi, SettingsUpdate, SettingsUpdateRequest,
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
    write_json(&paths.image_create_settings_file(), settings)
        .map_err(|error| ImageCreateError::Store(error.to_string()))
}

pub fn to_masked(settings: &ImageCreateSettings) -> MaskedSettings {
    MaskedSettings {
        base_url: settings.base_url.clone(),
        api_key_masked: mask_api_key(&settings.api_key),
        defaults: settings.defaults.clone(),
    }
}

pub fn validate_base_url(url: &str) -> Result<(), ImageCreateError> {
    let parsed = reqwest::Url::parse(url).map_err(|error| {
        ImageCreateError::InvalidConfig(format!("base_url is not a valid URL: {error}"))
    })?;

    if parsed.scheme() == "https" {
        return Ok(());
    }

    if parsed.scheme() != "http" {
        return Err(ImageCreateError::InvalidConfig(
            "base_url must use HTTPS or HTTP on a loopback host".to_string(),
        ));
    }

    let host = parsed.host_str().ok_or_else(|| {
        ImageCreateError::InvalidConfig("base_url must include a host".to_string())
    })?;
    let unbracketed_host = host
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(host);
    let is_loopback = host.eq_ignore_ascii_case("localhost")
        || unbracketed_host
            .parse::<IpAddr>()
            .is_ok_and(|address| address.is_loopback());

    if is_loopback {
        Ok(())
    } else {
        Err(ImageCreateError::InvalidConfig(
            "base_url must use HTTPS or resolve to a loopback host".to_string(),
        ))
    }
}

pub fn apply_update(current: &ImageCreateSettings, update: SettingsUpdate) -> ImageCreateSettings {
    ImageCreateSettings {
        base_url: update.base_url.unwrap_or_else(|| current.base_url.clone()),
        api_key: match update.api_key {
            ApiKeyUpdate::Retain => current.api_key.clone(),
            ApiKeyUpdate::Replace(value) => value,
            ApiKeyUpdate::Clear => String::new(),
        },
        defaults: update.defaults.unwrap_or_else(|| current.defaults.clone()),
    }
}

pub fn from_request(req: SettingsUpdateRequest) -> SettingsUpdate {
    let api_key = match req.api_key_action {
        ApiKeyAction::Retain => ApiKeyUpdate::Retain,
        ApiKeyAction::Clear => ApiKeyUpdate::Clear,
        ApiKeyAction::Replace => ApiKeyUpdate::Replace(req.api_key.unwrap_or_default()),
    };

    SettingsUpdate {
        base_url: req.base_url,
        api_key,
        defaults: req.defaults,
    }
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

    async fn validate_base_url(&self, url: &str) -> Result<(), ImageCreateError> {
        validate_base_url(url)
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
        ApiKeyAction, ApiKeyUpdate, DefaultParams, ImageBackground, ImageOutputFormat,
        ImageQuality, ImageSize,
    };
    use tempfile::tempdir;

    fn settings(api_key: &str) -> ImageCreateSettings {
        ImageCreateSettings {
            base_url: "https://api.example.com/v1".to_string(),
            api_key: api_key.to_string(),
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
    fn applies_api_key_retain_clear_and_replace_updates() {
        let current = settings("sk-original1234");

        let retained = apply_update(
            &current,
            SettingsUpdate {
                base_url: None,
                api_key: ApiKeyUpdate::Retain,
                defaults: None,
            },
        );
        assert_eq!(retained.api_key, "sk-original1234");

        let cleared = apply_update(
            &current,
            SettingsUpdate {
                base_url: None,
                api_key: ApiKeyUpdate::Clear,
                defaults: None,
            },
        );
        assert!(cleared.api_key.is_empty());

        let replaced = apply_update(
            &current,
            SettingsUpdate {
                base_url: Some("https://images.example.com".into()),
                api_key: ApiKeyUpdate::Replace("sk-new5678".into()),
                defaults: Some(DefaultParams::default()),
            },
        );
        assert_eq!(replaced.base_url, "https://images.example.com");
        assert_eq!(replaced.api_key, "sk-new5678");
        assert_eq!(replaced.defaults, DefaultParams::default());
    }

    #[test]
    fn converts_request_api_key_actions_to_domain_updates() {
        let replace = from_request(SettingsUpdateRequest {
            base_url: None,
            api_key_action: ApiKeyAction::Replace,
            api_key: Some("sk-replacement".into()),
            defaults: None,
        });
        assert_eq!(
            replace.api_key,
            ApiKeyUpdate::Replace("sk-replacement".into())
        );

        let missing_replace_value = from_request(SettingsUpdateRequest {
            base_url: None,
            api_key_action: ApiKeyAction::Replace,
            api_key: None,
            defaults: None,
        });
        assert_eq!(
            missing_replace_value.api_key,
            ApiKeyUpdate::Replace(String::new())
        );

        for (action, expected) in [
            (ApiKeyAction::Retain, ApiKeyUpdate::Retain),
            (ApiKeyAction::Clear, ApiKeyUpdate::Clear),
        ] {
            let update = from_request(SettingsUpdateRequest {
                base_url: None,
                api_key_action: action,
                api_key: Some("ignored".into()),
                defaults: None,
            });
            assert_eq!(update.api_key, expected);
        }
    }

    #[test]
    fn saves_and_loads_settings_round_trip_and_defaults_when_missing() {
        let dir = tempdir().expect("tempdir");
        let paths = AriaStatePaths::from_workspace_root(dir.path());
        assert_eq!(load(&paths), ImageCreateSettings::default());

        let expected = settings("sk-secret1234");
        save(&paths, &expected).expect("save settings");

        assert_eq!(load(&paths), expected);
    }

    #[test]
    fn creates_masked_settings_without_changing_other_fields() {
        let plain = settings("sk-abcd1234");

        let masked = to_masked(&plain);

        assert_eq!(masked.base_url, plain.base_url);
        assert_eq!(masked.api_key_masked, "sk-****1234");
        assert_eq!(masked.defaults, plain.defaults);
    }

    #[test]
    fn validates_https_and_loopback_base_urls_only() {
        for allowed in [
            "https://api.example.com/v1",
            "http://localhost:8080/v1",
            "http://127.0.0.1:8080/v1",
            "http://127.42.7.9/v1",
            "http://[::1]:8080/v1",
        ] {
            assert!(
                validate_base_url(allowed).is_ok(),
                "expected {allowed} to pass"
            );
        }

        for rejected in [
            "http://api.example.com/v1",
            "ftp://localhost/images",
            "not-a-url",
            "http://127.0.0.1.example.com/v1",
            "",
        ] {
            assert!(
                matches!(
                    validate_base_url(rejected),
                    Err(ImageCreateError::InvalidConfig(_))
                ),
                "expected {rejected} to fail"
            );
        }
    }
}
