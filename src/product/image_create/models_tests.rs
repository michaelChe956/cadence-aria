//! models.rs 的外置测试模块：经 models.rs 内 `#[path = "models_tests.rs"]` 引入，
//! `use super::*` 语义与原内嵌 `mod tests` 完全一致。
//!
//! 覆盖设计文档 7.1 的「设置与兼容」（S-01…S-14）与「目标解析」（T-01…T-12）。

use super::*;
use crate::cross_cutting::aria_state_paths::AriaStatePaths;
use crate::product::image_create::settings_store::{
    apply_update, from_request, load, save, to_masked,
};
use serde::{Serialize, de::DeserializeOwned};

// ============================================================
// 既有用例：自 models.rs 原 `mod tests` 逐字迁移。
// 仅删去 InputFidelity 的两行断言——该枚举已按设计删除。
// ============================================================

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
        vendor: None,
        model: None,
        selection: None,
        reference_source: None,
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

#[test]
fn generation_result_round_trips_vendor_annotation_and_tolerates_legacy_records() {
    let result = GenerationResult {
        prompt: "p".to_string(),
        params: DefaultParams::default(),
        media_type: "image/png".to_string(),
        image_id: None,
        b64: None,
        vendor: Some(ImageVendor::Minimax),
        model: Some("MiniMax-H3-Image".to_string()),
        selection: Some(ModelSelection::Auto),
        reference_source: Some(ReferenceSource::Previous),
        ts: Utc::now(),
    };
    let json = serde_json::to_string(&result).expect("serialize");
    for key in ["\"vendor\"", "\"model\"", "\"selection\"", "\"reference_source\""] {
        assert!(json.contains(key), "序列化输出应含 {key}");
    }
    assert_eq!(
        serde_json::from_str::<GenerationResult>(&json).unwrap(),
        result
    );

    // 旧会话记录（无 4 个新字段）反序列化为全 None，不报错。
    let legacy = r#"{
        "prompt": "p",
        "params": {"size": "auto", "quality": "auto", "background": "auto", "output_format": "png"},
        "media_type": "image/png",
        "image_id": "0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0",
        "ts": "2026-08-26T00:00:00Z"
    }"#;
    let record: GenerationResult = serde_json::from_str(legacy).expect("deserialize legacy");
    assert_eq!(record.vendor, None);
    assert_eq!(record.model, None);
    assert_eq!(record.selection, None);
    assert_eq!(record.reference_source, None);
}

// ============================================================
// 测试辅助
// ============================================================

fn vendor_settings(base_url: &str, api_key: &str, models: &[&str], default_model: &str) -> VendorSettings {
    VendorSettings {
        base_url: base_url.to_string(),
        api_key: api_key.to_string(),
        models: models.iter().map(|m| m.to_string()).collect(),
        default_model: default_model.to_string(),
    }
}

fn settings_with(vendors: VendorsSettings) -> ImageCreateSettings {
    ImageCreateSettings {
        vendors,
        defaults: DefaultParams::default(),
    }
}

fn both_available() -> ImageCreateSettings {
    settings_with(VendorsSettings {
        gpt: vendor_settings(
            "https://gpt.example.com/v1",
            "sk-gpt-secret123",
            &["gpt-image-2.6", "gpt-image-2.6-flare"],
            "gpt-image-2.6-flare",
        ),
        minimax: vendor_settings(
            "https://minimax.example.com/v1",
            "sk-mm-secret123",
            &["MiniMax-H3-Image"],
            "MiniMax-H3-Image",
        ),
    })
}

fn temp_paths() -> (tempfile::TempDir, AriaStatePaths) {
    let dir = tempfile::tempdir().expect("tempdir");
    let paths = AriaStatePaths::from_workspace_root(dir.path());
    (dir, paths)
}

fn write_settings_file(paths: &AriaStatePaths, json: serde_json::Value) {
    let file = paths.image_create_settings_file();
    std::fs::create_dir_all(file.parent().expect("设置文件父目录")).expect("创建父目录");
    std::fs::write(file, json.to_string()).expect("写入设置文件");
}

fn default_string_list<const N: usize>(items: [&str; N]) -> Vec<String> {
    items.iter().map(|m| m.to_string()).collect()
}

// ============================================================
// 设置与兼容（S-01…S-14）
// ============================================================

#[test]
fn s01_legacy_settings_file_migrates_into_gpt_vendor_on_load() {
    let (_dir, paths) = temp_paths();
    write_settings_file(
        &paths,
        serde_json::json!({
            "base_url": "https://legacy.example.com/v1",
            "api_key": "sk-legacy-secret99",
            "defaults": {
                "size": "1536x1024",
                "quality": "high",
                "background": "opaque",
                "output_format": "webp"
            }
        }),
    );

    let settings = load(&paths);

    assert_eq!(settings.vendors.gpt.base_url, "https://legacy.example.com/v1");
    assert_eq!(settings.vendors.gpt.api_key, "sk-legacy-secret99");
    assert_eq!(settings.vendors.minimax.base_url, "");
    assert_eq!(settings.vendors.minimax.api_key, "");
    assert_eq!(settings.vendors.gpt.models, default_string_list(DEFAULT_GPT_MODELS));
    assert_eq!(
        settings.vendors.minimax.models,
        default_string_list(DEFAULT_MINIMAX_MODELS)
    );
    assert_eq!(
        settings.defaults,
        DefaultParams {
            size: ImageSize::Landscape,
            quality: ImageQuality::High,
            background: ImageBackground::Opaque,
            output_format: ImageOutputFormat::Webp,
        }
    );
}

#[test]
fn s02_missing_file_yields_builtin_default_catalogs() {
    let (_dir, paths) = temp_paths();
    let settings = load(&paths);

    assert_eq!(settings, ImageCreateSettings::default());
    assert_eq!(settings.vendors.gpt.base_url, "");
    assert_eq!(settings.vendors.gpt.api_key, "");
    assert_eq!(settings.vendors.minimax.base_url, "");
    assert_eq!(settings.vendors.minimax.api_key, "");
    let gpt: Vec<&str> = settings.vendors.gpt.models.iter().map(String::as_str).collect();
    assert_eq!(gpt, DEFAULT_GPT_MODELS);
    assert_eq!(settings.vendors.gpt.default_model, DEFAULT_GPT_ROTATION_MODEL);
    assert_eq!(
        settings.vendors.minimax.models,
        default_string_list(DEFAULT_MINIMAX_MODELS)
    );
    assert_eq!(
        settings.vendors.minimax.default_model,
        DEFAULT_MINIMAX_ROTATION_MODEL
    );
}

#[test]
fn s03_new_shape_missing_vendor_or_models_falls_back_to_builtin() {
    // gpt 家缺 models / default_model → 补内置默认。
    let settings: ImageCreateSettings = serde_json::from_value(serde_json::json!({
        "vendors": {
            "gpt": {
                "base_url": "https://gpt.example.com/v1",
                "api_key": "sk-gpt"
            },
            "minimax": {
                "base_url": "https://minimax.example.com/v1",
                "api_key": "sk-mm",
                "models": ["MiniMax-H3-Image"],
                "default_model": "MiniMax-H3-Image"
            }
        }
    }))
    .expect("反序列化新形态");
    assert_eq!(settings.vendors.gpt.models, default_string_list(DEFAULT_GPT_MODELS));
    assert_eq!(settings.vendors.gpt.default_model, DEFAULT_GPT_ROTATION_MODEL);
    assert_eq!(
        settings.vendors.minimax.models,
        default_string_list(DEFAULT_MINIMAX_MODELS)
    );

    // 缺整家 minimax → 凭据为空、清单为内置默认。
    let settings: ImageCreateSettings = serde_json::from_value(serde_json::json!({
        "vendors": {
            "gpt": {"base_url": "https://g.example.com", "api_key": "k"}
        }
    }))
    .expect("反序列化缺家形态");
    assert_eq!(settings.vendors.minimax.base_url, "");
    assert_eq!(settings.vendors.minimax.api_key, "");
    assert_eq!(
        settings.vendors.minimax.models,
        default_string_list(DEFAULT_MINIMAX_MODELS)
    );
}

#[test]
fn s04_explicitly_empty_models_list_stays_empty() {
    let settings: ImageCreateSettings = serde_json::from_value(serde_json::json!({
        "vendors": {
            "gpt": {
                "base_url": "https://g.example.com",
                "api_key": "k",
                "models": []
            },
            "minimax": {"models": []}
        }
    }))
    .expect("反序列化空清单形态");
    assert!(settings.vendors.gpt.models.is_empty());
    assert_eq!(settings.vendors.gpt.default_model, "");
    assert!(settings.vendors.minimax.models.is_empty());
    assert_eq!(settings.vendors.minimax.default_model, "");
}

#[test]
fn s05_vendors_key_wins_over_legacy_top_level_fields() {
    let settings: ImageCreateSettings = serde_json::from_value(serde_json::json!({
        "vendors": {
            "gpt": {"base_url": "https://new.example.com", "api_key": "sk-new"}
        },
        "base_url": "https://old.example.com",
        "api_key": "sk-old"
    }))
    .expect("反序列化混合形态");
    assert_eq!(settings.vendors.gpt.base_url, "https://new.example.com");
    assert_eq!(settings.vendors.gpt.api_key, "sk-new");
}

#[test]
fn s06_serialization_outputs_vendor_shape_only() {
    let json = serde_json::to_value(both_available()).expect("serialize");
    assert!(json.get("vendors").is_some());
    assert!(json.get("base_url").is_none());
    assert!(json.get("api_key").is_none());
    assert_eq!(json["vendors"]["gpt"]["base_url"], "https://gpt.example.com/v1");
    assert_eq!(json["vendors"]["minimax"]["api_key"], "sk-mm-secret123");
}

#[test]
fn s07_legacy_load_then_save_then_load_round_trips() {
    let (_dir, paths) = temp_paths();
    write_settings_file(
        &paths,
        serde_json::json!({
            "base_url": "https://legacy.example.com/v1",
            "api_key": "sk-legacy-secret99",
            "defaults": {
                "size": "auto", "quality": "auto", "background": "auto", "output_format": "png"
            }
        }),
    );

    let first = load(&paths);
    save(&paths, &first).expect("保存迁移后的设置");
    let second = load(&paths);
    assert_eq!(first, second);

    // 保存后的文件只写新形态：不再有顶层 base_url / api_key。
    let raw = std::fs::read_to_string(paths.image_create_settings_file()).unwrap();
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert!(value.get("vendors").is_some());
    assert!(value.get("base_url").is_none());
    assert!(value.get("api_key").is_none());
}

#[test]
fn s08_updating_only_minimax_key_leaves_gpt_untouched() {
    let current = both_available();
    let updated = apply_update(
        &current,
        SettingsUpdate {
            gpt: None,
            minimax: Some(VendorUpdate {
                base_url: None,
                api_key: ApiKeyUpdate::Replace("sk-mm-rotated99".to_string()),
                models: None,
                default_model: None,
            }),
            defaults: None,
        },
    );

    assert_eq!(updated.vendors.gpt.base_url, current.vendors.gpt.base_url);
    assert_eq!(updated.vendors.gpt.api_key, current.vendors.gpt.api_key);
    assert_eq!(updated.vendors.gpt.models, current.vendors.gpt.models);
    assert_eq!(updated.vendors.gpt.default_model, current.vendors.gpt.default_model);
    assert_eq!(updated.vendors.minimax.api_key, "sk-mm-rotated99");
}

#[test]
fn s09_api_key_tri_state_applies_per_vendor_independently() {
    let current = both_available();
    let tri = |gpt: ApiKeyAction, minimax: ApiKeyAction| -> ImageCreateSettings {
        let update = from_request(SettingsUpdateRequest {
            vendors: Some(VendorsUpdateRequest {
                gpt: Some(VendorUpdateRequest {
                    base_url: None,
                    api_key_action: gpt,
                    api_key: Some("ignored".to_string()),
                    models: None,
                    default_model: None,
                }),
                minimax: Some(VendorUpdateRequest {
                    base_url: None,
                    api_key_action: minimax,
                    api_key: Some("sk-mm-next".to_string()),
                    models: None,
                    default_model: None,
                }),
            }),
            defaults: None,
        });
        apply_update(&current, update)
    };

    // retain：两家都不变。
    let retained = tri(ApiKeyAction::Retain, ApiKeyAction::Retain);
    assert_eq!(retained.vendors.gpt.api_key, "sk-gpt-secret123");
    assert_eq!(retained.vendors.minimax.api_key, "sk-mm-secret123");

    // replace：只换 minimax，GPT 不受影响。
    let replaced = tri(ApiKeyAction::Retain, ApiKeyAction::Replace);
    assert_eq!(replaced.vendors.gpt.api_key, "sk-gpt-secret123");
    assert_eq!(replaced.vendors.minimax.api_key, "sk-mm-next");

    // clear：只清 gpt，MiniMax 不受影响。
    let cleared = tri(ApiKeyAction::Clear, ApiKeyAction::Retain);
    assert_eq!(cleared.vendors.gpt.api_key, "");
    assert_eq!(cleared.vendors.minimax.api_key, "sk-mm-secret123");

    // 某家整体缺省 = 该家全部保留。
    let untouched = apply_update(
        &current,
        from_request(SettingsUpdateRequest {
            vendors: Some(VendorsUpdateRequest {
                gpt: None,
                minimax: Some(VendorUpdateRequest {
                    base_url: None,
                    api_key_action: ApiKeyAction::Retain,
                    api_key: None,
                    models: None,
                    default_model: None,
                }),
            }),
            defaults: None,
        }),
    );
    assert_eq!(untouched.vendors.gpt, current.vendors.gpt);
}

#[test]
fn s10_model_catalog_validation_rejects_each_rule_violation() {
    let over_long = "x".repeat(MAX_MODEL_ID_CHARS + 1);
    let cases: Vec<(&str, Vec<String>)> = vec![
        ("空串", vec![String::new()]),
        ("超长", vec![over_long]),
        ("非法字符", vec!["bad model!".to_string()]),
        ("保留字 auto", vec!["auto".to_string()]),
        ("保留字 AUTO", vec!["AUTO".to_string()]),
        ("家内重复", vec!["m-a".to_string(), "m-a".to_string()]),
    ];
    for (name, models) in cases {
        let default_model = models.first().cloned().unwrap_or_default();
        let settings = settings_with(VendorsSettings {
            gpt: vendor_settings("https://g.example.com", "k", &models.iter().map(String::as_str).collect::<Vec<_>>(), &default_model),
            minimax: vendor_settings(
                "https://minimax.example.com/v1",
                "k",
                &["MiniMax-H3-Image"],
                "MiniMax-H3-Image",
            ),
        });
        assert!(
            matches!(settings.validate(), Err(ImageCreateError::InvalidConfig(_))),
            "用例「{name}」应被整体拒绝"
        );
    }

    // 跨家重复。
    let cross = settings_with(VendorsSettings {
        gpt: vendor_settings("https://g.example.com", "k", &["shared-model"], "shared-model"),
        minimax: vendor_settings("https://minimax.example.com/v1", "k", &["shared-model"], "shared-model"),
    });
    let error = cross.validate().unwrap_err().to_string();
    assert!(error.contains("shared-model"));
    assert!(error.contains("GPT") && error.contains("MiniMax"));

    // 超过 32 个；恰好 32 个通过。
    let oversize: Vec<String> = (0..=MAX_MODELS_PER_VENDOR).map(|i| format!("m-{i:02}")).collect();
    let over = settings_with(VendorsSettings {
        gpt: vendor_settings("https://g.example.com", "k", &oversize.iter().map(String::as_str).collect::<Vec<_>>(), "m-00"),
        minimax: vendor_settings("https://minimax.example.com/v1", "k", &["MiniMax-H3-Image"], "MiniMax-H3-Image"),
    });
    assert!(matches!(over.validate(), Err(ImageCreateError::InvalidConfig(_))));
    let exact: Vec<String> = (0..MAX_MODELS_PER_VENDOR).map(|i| format!("m-{i:02}")).collect();
    let boundary = settings_with(VendorsSettings {
        gpt: vendor_settings("https://g.example.com", "k", &exact.iter().map(String::as_str).collect::<Vec<_>>(), "m-00"),
        minimax: vendor_settings("https://minimax.example.com/v1", "k", &["MiniMax-H3-Image"], "MiniMax-H3-Image"),
    });
    assert!(boundary.validate().is_ok());
}

#[test]
fn s11_default_model_must_belong_to_catalog() {
    // 清单非空但轮转模型不在清单内。
    let settings = settings_with(VendorsSettings {
        gpt: vendor_settings("https://g.example.com", "k", &["m-a", "m-b"], "m-missing"),
        minimax: vendor_settings("https://minimax.example.com/v1", "k", &[], ""),
    });
    assert!(matches!(settings.validate(), Err(ImageCreateError::InvalidConfig(_))));

    // 清单为空但轮转模型非空串。
    let settings = settings_with(VendorsSettings {
        gpt: vendor_settings("https://g.example.com", "k", &["m-a"], "m-a"),
        minimax: vendor_settings("https://minimax.example.com/v1", "k", &[], "m-x"),
    });
    assert!(matches!(settings.validate(), Err(ImageCreateError::InvalidConfig(_))));

    assert!(both_available().validate().is_ok());
}

#[test]
fn s12_non_loopback_http_base_url_rejects_whole_settings() {
    // 一家 http 非回环、另一家合法 → 整体拒绝，不落盘。
    let settings = settings_with(VendorsSettings {
        gpt: vendor_settings("http://insecure.example.com/v1", "k", &["m-a"], "m-a"),
        minimax: vendor_settings(
            "https://minimax.example.com/v1",
            "k",
            &["MiniMax-H3-Image"],
            "MiniMax-H3-Image",
        ),
    });
    assert!(matches!(settings.validate(), Err(ImageCreateError::InvalidConfig(_))));

    // 空串 base_url 表示该家未配置，放行。
    let empty = settings_with(VendorsSettings {
        gpt: vendor_settings("", "", &["m-a"], "m-a"),
        minimax: vendor_settings(
            "https://minimax.example.com/v1",
            "k",
            &["MiniMax-H3-Image"],
            "MiniMax-H3-Image",
        ),
    });
    assert!(empty.validate().is_ok());
}

#[test]
fn s13_masked_view_hides_keys_and_reports_availability() {
    let settings = both_available();
    let masked = to_masked(&settings);
    let json = serde_json::to_string(&masked).expect("serialize masked");
    assert!(!json.contains("sk-gpt-secret123"));
    assert!(!json.contains("sk-mm-secret123"));
    assert_eq!(masked.vendors.gpt.api_key_masked, "sk-****t123");
    assert!(masked.vendors.gpt.available);
    assert!(masked.vendors.minimax.available);
    assert_eq!(masked.defaults, settings.defaults);
    assert_eq!(masked.vendors.gpt.recommended_models, default_string_list(DEFAULT_GPT_MODELS));
    assert_eq!(
        masked.vendors.minimax.recommended_models,
        default_string_list(DEFAULT_MINIMAX_MODELS)
    );

    // 缺 key → 该家不可用，另一家不受影响。
    let mut partial = both_available();
    partial.vendors.minimax.api_key = String::new();
    let masked = to_masked(&partial);
    assert!(!masked.vendors.minimax.available);
    assert!(masked.vendors.gpt.available);

    // 空清单 → 不可用。
    let mut no_models = both_available();
    no_models.vendors.gpt.models = Vec::new();
    assert!(!to_masked(&no_models).vendors.gpt.available);
}

#[test]
fn s14_debug_output_never_exposes_api_keys() {
    let settings = both_available();
    assert!(!format!("{:?}", settings).contains("sk-gpt-secret123"));
    assert!(!format!("{:?}", settings).contains("sk-mm-secret123"));
    assert!(!format!("{:?}", settings.vendors).contains("sk-gpt-secret123"));
    assert!(!format!("{:?}", settings.vendors.gpt).contains("sk-gpt-secret123"));
    assert!(format!("{:?}", settings.vendors.gpt).contains("<redacted>"));
}

#[test]
fn s14_wire_structs_do_not_derive_debug() {
    // 编译期性质以源码形态固化：三个 wire 结构内含 api_key 明文，不 derive Debug。
    let source = include_str!("models.rs");
    for name in ["ImageCreateSettingsWire", "VendorsWire", "VendorWire"] {
        let at = source
            .find(&format!("struct {name}"))
            .unwrap_or_else(|| panic!("models.rs 应声明 struct {name}"));
        let derive_at = source[..at]
            .rfind("#[derive(")
            .unwrap_or_else(|| panic!("{name} 之上应有 derive 属性"));
        let derive_end = source[derive_at..]
            .find(']')
            .expect("derive 属性应闭合")
            + derive_at;
        let derive = &source[derive_at..derive_end];
        assert!(
            !derive.contains("Debug"),
            "{name} 不得 derive Debug（api_key 明文会经 Debug 泄漏）"
        );
    }
}

#[test]
fn previous_image_unavailable_error_renders_chinese_hint() {
    let error = ImageCreateError::PreviousImageUnavailable("图片文件缺失".to_string());
    assert!(error.to_string().contains("上一张图片不可用"));
    assert!(error.to_string().contains("图片文件缺失"));
}

// ============================================================
// 目标解析（T-01…T-12）
// ============================================================

#[test]
fn t01_auto_without_last_vendor_targets_gpt_rotation_model() {
    let target = both_available()
        .resolve_target(&ModelChoice::Auto, None)
        .expect("解析自动目标");
    assert_eq!(target.vendor, ImageVendor::Gpt);
    assert_eq!(target.model, "gpt-image-2.6-flare");
    assert_eq!(target.selection, ModelSelection::Auto);
}

#[test]
fn t02_auto_after_gpt_result_targets_minimax() {
    let target = both_available()
        .resolve_target(&ModelChoice::Auto, Some(ImageVendor::Gpt))
        .expect("解析自动目标");
    assert_eq!(target.vendor, ImageVendor::Minimax);
    assert_eq!(target.model, "MiniMax-H3-Image");
    assert_eq!(target.selection, ModelSelection::Auto);
}

#[test]
fn t03_auto_after_minimax_result_targets_gpt() {
    let target = both_available()
        .resolve_target(&ModelChoice::Auto, Some(ImageVendor::Minimax))
        .expect("解析自动目标");
    assert_eq!(target.vendor, ImageVendor::Gpt);
    assert_eq!(target.model, "gpt-image-2.6-flare");
    assert_eq!(target.selection, ModelSelection::Auto);
}

fn gpt_only_available() -> ImageCreateSettings {
    let mut settings = both_available();
    settings.vendors.minimax.api_key = String::new();
    settings
}

#[test]
fn t04_auto_with_only_gpt_available_stays_on_gpt() {
    let target = gpt_only_available()
        .resolve_target(&ModelChoice::Auto, Some(ImageVendor::Gpt))
        .expect("解析自动目标");
    assert_eq!(target.vendor, ImageVendor::Gpt);
    assert_eq!(target.model, "gpt-image-2.6-flare");
    assert_eq!(target.selection, ModelSelection::Auto);
}

#[test]
fn t05_auto_with_only_minimax_available_falls_to_minimax() {
    let mut settings = both_available();
    settings.vendors.gpt.api_key = String::new();
    let target = settings
        .resolve_target(&ModelChoice::Auto, None)
        .expect("解析自动目标");
    assert_eq!(target.vendor, ImageVendor::Minimax);
    assert_eq!(target.model, "MiniMax-H3-Image");
    assert_eq!(target.selection, ModelSelection::Auto);
}

#[test]
fn t06_auto_with_no_available_vendor_fails_with_missing_config() {
    let mut settings = both_available();
    settings.vendors.gpt.api_key = String::new();
    settings.vendors.minimax.api_key = String::new();
    let error = settings
        .resolve_target(&ModelChoice::Auto, None)
        .expect_err("两家均不可用应报错");
    assert!(matches!(error, ImageCreateError::MissingConfig(_)));
    assert!(error.to_string().contains("尚未配置任何可用的图片生成端点"));
}

#[test]
fn t07_explicit_gpt_model_ignores_last_vendor() {
    for last in [None, Some(ImageVendor::Minimax)] {
        let target = both_available()
            .resolve_target(&ModelChoice::Explicit("gpt-image-2.6-flare".to_string()), last)
            .expect("解析显式目标");
        assert_eq!(target.vendor, ImageVendor::Gpt);
        assert_eq!(target.model, "gpt-image-2.6-flare");
        assert_eq!(target.selection, ModelSelection::Explicit);
    }
}

#[test]
fn t08_explicit_unknown_model_is_invalid_param() {
    let error = both_available()
        .resolve_target(&ModelChoice::Explicit("no-such-model".to_string()), None)
        .expect_err("未知模型应报参数错误");
    assert!(matches!(error, ImageCreateError::InvalidParam(_)));
    assert!(error.to_string().contains("no-such-model"));
}

#[test]
fn t09_explicit_minimax_model_without_credentials_is_missing_config() {
    // MiniMax 无凭据：配置缺失，信息含「MiniMax」，不落到 GPT。
    let error = gpt_only_available()
        .resolve_target(&ModelChoice::Explicit("MiniMax-H3-Image".to_string()), None)
        .expect_err("显式指定未配置的家应报配置缺失");
    assert!(matches!(error, ImageCreateError::MissingConfig(_)));
    assert!(error.to_string().contains("MiniMax"));

    // 有凭据时同一 id 解析到 MiniMax 本家。
    let target = both_available()
        .resolve_target(&ModelChoice::Explicit("MiniMax-H3-Image".to_string()), None)
        .expect("解析显式目标");
    assert_eq!(target.vendor, ImageVendor::Minimax);
}

#[test]
fn t10_auto_falls_back_to_first_catalog_entry_when_default_model_absent() {
    let mut settings = both_available();
    settings.vendors.gpt.models = vec!["m-second".to_string(), "gpt-image-2.6-flare".to_string()];

    // default_model 在清单内 → 取它。
    settings.vendors.gpt.default_model = "gpt-image-2.6-flare".to_string();
    let target = settings
        .resolve_target(&ModelChoice::Auto, None)
        .expect("解析自动目标");
    assert_eq!(target.model, "gpt-image-2.6-flare");

    // default_model 不在清单内（手改文件导致）→ 取清单首项。
    settings.vendors.gpt.default_model = "not-in-list".to_string();
    let target = settings
        .resolve_target(&ModelChoice::Auto, None)
        .expect("解析自动目标");
    assert_eq!(target.model, "m-second");
}

#[test]
fn t11_explicit_model_match_is_case_sensitive() {
    // 逐字符相等：仅大小写不同即视为不在清单。
    let error = both_available()
        .resolve_target(&ModelChoice::Explicit("GPT-IMAGE-2.6-FLARE".to_string()), None)
        .expect_err("大小写不同应报参数错误");
    assert!(matches!(error, ImageCreateError::InvalidParam(_)));
}

#[test]
fn t12_model_choice_parse_treats_blank_and_auto_as_auto() {
    assert_eq!(ModelChoice::parse(None), ModelChoice::Auto);
    assert_eq!(ModelChoice::parse(Some("")), ModelChoice::Auto);
    assert_eq!(ModelChoice::parse(Some("   ")), ModelChoice::Auto);
    assert_eq!(ModelChoice::parse(Some("auto")), ModelChoice::Auto);
    assert_eq!(ModelChoice::parse(Some("AUTO")), ModelChoice::Auto);
    assert_eq!(
        ModelChoice::parse(Some(" x ")),
        ModelChoice::Explicit("x".to_string())
    );
    assert_eq!(
        ModelChoice::parse(Some("MiniMax-H3-Image")),
        ModelChoice::Explicit("MiniMax-H3-Image".to_string())
    );
}

#[test]
fn image_vendor_serde_uses_lowercase_names() {
    assert_eq!(serde_json::to_string(&ImageVendor::Gpt).unwrap(), "\"gpt\"");
    assert_eq!(
        serde_json::to_string(&ImageVendor::Minimax).unwrap(),
        "\"minimax\""
    );
    assert_eq!(
        serde_json::from_str::<ImageVendor>("\"minimax\"").unwrap(),
        ImageVendor::Minimax
    );
    assert_eq!(ImageVendor::Gpt.other(), ImageVendor::Minimax);
    assert_eq!(ImageVendor::Minimax.other(), ImageVendor::Gpt);
    assert_eq!(ImageVendor::Gpt.label(), "GPT");
    assert_eq!(ImageVendor::Minimax.label(), "MiniMax");
}
