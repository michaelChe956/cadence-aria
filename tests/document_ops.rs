use cadence_aria::cross_cutting::ast_grep_tool::probe_ast_grep;
use cadence_aria::cross_cutting::document_ops::{
    apply_json_patch, create_document, read_document_model, render_document_model, upsert_section,
    DocumentOpError, DocumentTemplateKind, JsonPatch, JsonPatchOperation,
};
use cadence_aria::protocol::document_ops::{DocumentBlock, HeadingPath};
use serde_json::json;
use tempfile::tempdir;

#[test]
fn markdown_section_upsert_uses_full_heading_path_and_preserves_surrounding_markdown() {
    let workspace = tempdir().expect("temp workspace");
    let spec_path = workspace.path().join("spec.md");
    std::fs::write(
        &spec_path,
        include_str!("fixtures/document_ops/section_upsert_input.md"),
    )
    .expect("write fixture");

    let mut model = read_document_model(&spec_path).expect("read document model");
    assert!(model.sections.iter().any(|section| {
        section.heading_path
            == vec![
                "示例 Spec".to_string(),
                "模块 B".to_string(),
                "目标与范围".to_string(),
            ]
    }));

    let result = upsert_section(
        &mut model,
        &HeadingPath(vec![
            "示例 Spec".to_string(),
            "模块 B".to_string(),
            "目标与范围".to_string(),
        ]),
        vec![
            DocumentBlock::Paragraph("模块 B 的新目标说明。".to_string()),
            DocumentBlock::BulletList(vec![
                "[REQ-002] 通过 Document Operation 更新目标章节。".to_string(),
                "[REQ-003] 保留其他章节顺序和原始 Markdown 结构。".to_string(),
            ]),
        ],
    )
    .expect("upsert target section");

    assert!(result.changed);
    assert_ne!(result.old_sha256, result.new_sha256);
    assert_eq!(
        result.updated_heading_path,
        HeadingPath(vec![
            "示例 Spec".to_string(),
            "模块 B".to_string(),
            "目标与范围".to_string()
        ])
    );
    assert!(result.warnings.is_empty());

    let rendered = render_document_model(&model);
    assert_eq!(
        rendered,
        include_str!("fixtures/document_ops/section_upsert_expected.md")
    );

    let projection_source = cadence_aria::cross_cutting::document_ops::extract_projection_source(
        &model,
        &HeadingPath(vec![
            "示例 Spec".to_string(),
            "模块 B".to_string(),
            "目标与范围".to_string(),
        ]),
    )
    .expect("extract projection source");
    assert!(projection_source.contains("[REQ-002]"));
    assert!(projection_source.contains("模块 B 的新目标说明。"));
}

#[test]
fn structured_json_patch_updates_aria_traceability_and_keeps_json_valid() {
    let mut artifact = json!({
        "artifact_kind": "coding_report",
        "_aria": {
            "traceability_refs": ["req-001"]
        }
    });

    let patch = JsonPatch::new(vec![
        JsonPatchOperation::Replace {
            path: "/_aria/traceability_refs".to_string(),
            value: json!(["req-001", "ac-001"]),
        },
        JsonPatchOperation::Add {
            path: "/_aria/projection_refs".to_string(),
            value: json!(["proj_spec_001"]),
        },
    ]);

    apply_json_patch(&mut artifact, &patch).expect("apply structured patch");

    let encoded = serde_json::to_string(&artifact).expect("json serializes");
    let reparsed: serde_json::Value = serde_json::from_str(&encoded).expect("json reparses");
    assert_eq!(
        reparsed["_aria"]["traceability_refs"],
        json!(["req-001", "ac-001"])
    );
    assert_eq!(
        reparsed["_aria"]["projection_refs"],
        json!(["proj_spec_001"])
    );
}

#[test]
fn structured_patch_can_update_yaml_backed_openspec_config_without_string_concat() {
    let yaml = r#"
status: draft
constraints:
  - req-001
"#;
    let mut value: serde_json::Value = serde_yaml::from_str(yaml).expect("yaml to value");

    apply_json_patch(
        &mut value,
        &JsonPatch::new(vec![JsonPatchOperation::Add {
            path: "/bundle_status".to_string(),
            value: json!("ready"),
        }]),
    )
    .expect("apply patch to yaml value");

    let yaml_text = serde_yaml::to_string(&value).expect("serialize yaml");
    let reparsed: serde_json::Value = serde_yaml::from_str(&yaml_text).expect("reparse yaml");
    assert_eq!(reparsed["bundle_status"], json!("ready"));
    assert_eq!(reparsed["constraints"], json!(["req-001"]));
}

#[test]
fn create_document_uses_controlled_openspec_template_and_does_not_overwrite_existing_file() {
    let workspace = tempdir().expect("temp workspace");
    let spec_path = workspace.path().join("spec.md");

    let model =
        create_document(&spec_path, DocumentTemplateKind::OpenspecSpec).expect("create spec");
    assert!(model.sections.iter().any(|section| {
        section.heading_path == vec!["变更规格".to_string(), "需求".to_string()]
    }));

    let content = std::fs::read_to_string(&spec_path).expect("read created spec");
    assert_eq!(
        content,
        include_str!("fixtures/document_ops/create_document_openspec_spec_expected.md")
    );

    let error =
        create_document(&spec_path, DocumentTemplateKind::OpenspecSpec).expect_err("no overwrite");
    assert!(matches!(error, DocumentOpError::IoError(_)));
}

#[test]
fn missing_ast_grep_is_reported_as_optional_tool_absence() {
    match probe_ast_grep() {
        Ok(capability) => assert!(!capability.version.trim().is_empty()),
        Err(DocumentOpError::MissingOptionalTool(tool)) => assert_eq!(tool, "ast-grep"),
        Err(other) => panic!("unexpected ast-grep probe error: {other:?}"),
    }
}
