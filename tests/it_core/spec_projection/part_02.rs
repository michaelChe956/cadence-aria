#[test]
fn spec_projection_accepts_required_sections_prefixed_with_requirement_ids() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let spec_path = tempdir.path().join("spec.md");
    std::fs::write(
        &spec_path,
        "# Spec\n\n\
## REQ-003 功能需求\n\n\
### 后端 API\n\
- **REQ-003.1 登录接口**：`POST /api/auth/login`\n\n\
## REQ-004 成功标准\n\n\
1. 使用预设用户可以成功登录并获得有效 JWT Token\n\
2. 携带无效 Token 访问受保护接口返回 401\n\n\
## REQ-006 非功能需求\n\n\
- **NFR-001 安全**：JWT 使用强密钥签名。\n\
",
    )
    .expect("write spec");
    let source = read_document_model(&spec_path).expect("spec");
    let source_ref = artifact_ref(
        "art_ref_spec_prefixed_sections",
        "art_spec_prefixed_sections",
        ArtifactKind::Spec,
        &source,
    );

    let record = compile_spec_projection(&source, &source_ref, "N05".to_string())
        .expect("ID-prefixed required sections should compile");

    let ProjectionPayload::SpecProjection(payload) = &record.payload else {
        panic!("expected spec projection payload");
    };
    assert_eq!(
        payload.functional_requirements[0].requirement_id,
        "req-003.1"
    );
    assert_eq!(payload.success_criteria[0].criterion_id, "ac-001");
    assert_eq!(
        payload.non_functional_requirements[0].requirement_id,
        "nfr-001"
    );
}

#[test]
fn spec_projection_accepts_success_criteria_table_standard_column() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let spec_path = tempdir.path().join("spec.md");
    std::fs::write(
        &spec_path,
        "# Spec\n\n\
## 功能需求\n\n\
**REQ-AUTH-001：用户登录 API**\n\
后端必须提供登录接口。\n\n\
## 成功标准\n\n\
| 编号 | 标准 | 验证方式 |\n\
|------|------|----------|\n\
| AC-001 | 用户输入正确凭据后，后端返回 200 OK 及有效 JWT。 | 端到端测试 |\n\
",
    )
    .expect("write spec");
    let source = read_document_model(&spec_path).expect("spec");
    let source_ref = artifact_ref(
        "art_ref_spec_standard_column",
        "art_spec_standard_column",
        ArtifactKind::Spec,
        &source,
    );

    let record = compile_spec_projection(&source, &source_ref, "N05".to_string())
        .expect("success criteria standard column should compile");

    let ProjectionPayload::SpecProjection(payload) = &record.payload else {
        panic!("expected spec projection payload");
    };
    assert_eq!(
        payload.success_criteria[0].text,
        "用户输入正确凭据后，后端返回 200 OK 及有效 JWT。"
    );
}

#[test]
fn spec_projection_accepts_success_criteria_table_standard_description_column() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let spec_path = tempdir.path().join("spec.md");
    std::fs::write(
        &spec_path,
        "# Spec\n\n\
## 功能需求\n\n\
| ID | 需求描述 | 优先级 |\n\
|---|---|---|\n\
| REQ-001 | 后端提供登录接口。 | P0 |\n\n\
## 成功标准\n\n\
| ID | 标准描述 | 验证方式 |\n\
|---|---|---|\n\
| **AC-001** | 使用正确凭据调用登录接口，成功返回 JWT Token。 | 接口单元测试 |\n",
    )
    .expect("write spec");
    let source = read_document_model(&spec_path).expect("spec");
    let source_ref = artifact_ref(
        "art_ref_spec_standard_description_column",
        "art_spec_standard_description_column",
        ArtifactKind::Spec,
        &source,
    );

    let record = compile_spec_projection(&source, &source_ref, "N05".to_string())
        .expect("success criteria standard description column should compile");

    let ProjectionPayload::SpecProjection(payload) = &record.payload else {
        panic!("expected spec projection payload");
    };
    assert_eq!(
        payload.success_criteria[0].text,
        "使用正确凭据调用登录接口，成功返回 JWT Token。"
    );
}

#[test]
fn spec_projection_accepts_fr_and_sc_id_prefixes_from_real_provider_specs() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let spec_path = tempdir.path().join("spec.md");
    std::fs::write(
        &spec_path,
        "# Spec\n\n\
## 功能需求\n\n\
### FR-01：前端页面\n\
- **FR-01.1**：提供登录页面，包含用户名输入框、密码输入框、登录按钮。\n\
- **FR-01.2**：登录成功后跳转至受保护页面。\n\n\
### FR-02：后端 API\n\
- **FR-02.1**：`POST /api/auth/login` 接收用户名和密码。\n\
- **FR-02.2**：验证通过后生成并返回 JWT Token。\n\n\
## 成功标准\n\n\
| 编号 | 标准 | 验证方式 |\n\
|------|------|----------|\n\
| SC-01 | 用户可通过前端页面完成登录流程并获取 JWT Token。 | 端到端测试 |\n\
| SC-02 | Token 无效时访问受保护路由返回 401。 | API 测试 |\n\
",
    )
    .expect("write spec");
    let source = read_document_model(&spec_path).expect("spec");
    let source_ref = artifact_ref(
        "art_ref_spec_fr_sc",
        "art_spec_fr_sc",
        ArtifactKind::Spec,
        &source,
    );

    let record = compile_spec_projection(&source, &source_ref, "N05".to_string())
        .expect("FR/SC IDs should compile");

    let ProjectionPayload::SpecProjection(payload) = &record.payload else {
        panic!("expected spec projection payload");
    };
    assert_eq!(payload.functional_requirements[0].requirement_id, "fr-01.1");
    assert_eq!(payload.functional_requirements[3].requirement_id, "fr-02.2");
    assert_eq!(payload.success_criteria[0].criterion_id, "sc-01");
    assert_eq!(
        payload.success_criteria[0].text,
        "用户可通过前端页面完成登录流程并获取 JWT Token。"
    );
}

fn artifact_ref(
    artifact_ref_id: &str,
    artifact_id: &str,
    artifact_kind: ArtifactKind,
    source: &cadence_aria::protocol::document_ops::DocumentModel,
) -> ArtifactRef {
    ArtifactRef {
        artifact_ref_id: artifact_ref_id.to_string(),
        artifact_id: artifact_id.to_string(),
        artifact_kind,
        version: 1,
        path: source.source_path.clone(),
        sha256: source.sha256.clone(),
        status: ArtifactStatus::Active,
    }
}
