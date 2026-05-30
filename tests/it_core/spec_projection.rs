use cadence_aria::cross_cutting::artifact_projection::compile_spec_projection;
use cadence_aria::cross_cutting::artifact_validate::{ArtifactIndex, projection_validator};
use cadence_aria::cross_cutting::document_ops::read_document_model;
use cadence_aria::protocol::artifacts::{
    ArtifactKind, ArtifactRef, ArtifactStatus, ProjectionKind,
};
use cadence_aria::protocol::projections::{ProjectionPayload, RequirementPriority};
use serde_json::Value;

#[test]
fn spec_projection_compiles_from_document_model_and_matches_golden_json() {
    let source = read_document_model("tests/fixtures/artifacts/spec.md".as_ref()).expect("spec");
    let source_ref = artifact_ref(
        "art_ref_spec_0001",
        "art_spec_001",
        ArtifactKind::Spec,
        &source,
    );

    let record =
        compile_spec_projection(&source, &source_ref, "N05".to_string()).expect("compile spec");

    assert_eq!(record.projection_kind, ProjectionKind::SpecProjection);
    assert_eq!(record.source_artifact_hash, source.sha256);
    assert_eq!(
        record.projection_id,
        "proj_spec_projection_art_spec_001_0001"
    );

    let ProjectionPayload::SpecProjection(payload) = &record.payload else {
        panic!("expected spec projection payload");
    };
    assert_eq!(payload.functional_requirements[0].requirement_id, "req-001");
    assert_eq!(payload.functional_requirements[1].requirement_id, "req-002");
    assert_eq!(
        payload.success_criteria[0].related_requirement_ids,
        vec!["req-001".to_string(), "req-002".to_string()]
    );

    let golden: Value = serde_json::from_str(include_str!(
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/artifacts/golden/spec_projection.json")
    ))
    .expect("golden json");
    assert_eq!(serde_json::to_value(payload).expect("payload json"), golden);

    let index = ArtifactIndex::from_active_refs(vec![source_ref]);
    let validation = projection_validator(
        &record,
        &index,
        Some("tests/fixtures/artifacts/golden/spec_projection.json".as_ref()),
    )
    .expect("projection validation");
    assert!(validation.valid);
}

#[test]
fn spec_projection_rejects_unknown_requirement_reference() {
    let source =
        read_document_model("tests/fixtures/artifacts/spec_unknown_ref.md".as_ref()).expect("spec");
    let source_ref = artifact_ref(
        "art_ref_spec_0002",
        "art_spec_002",
        ArtifactKind::Spec,
        &source,
    );

    let error = compile_spec_projection(&source, &source_ref, "N05".to_string())
        .expect_err("unknown REQ reference should fail");

    assert_eq!(
        error.to_string(),
        "unknown projection reference req-999 in success_criteria"
    );
}

#[test]
fn spec_projection_accepts_numbered_required_section_headings() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let spec_path = tempdir.path().join("spec.md");
    std::fs::write(
        &spec_path,
        "# Spec\n\n## 3. 功能需求\n\n- [REQ-001] 用户可以登录。Priority: must\n\n## 4. 成功标准\n\n- [AC-001] 登录成功后返回 JWT。Refs: REQ-001\n",
    )
    .expect("write spec");
    let source = read_document_model(&spec_path).expect("spec");
    let source_ref = artifact_ref(
        "art_ref_spec_numbered",
        "art_spec_numbered",
        ArtifactKind::Spec,
        &source,
    );

    let record = compile_spec_projection(&source, &source_ref, "N05".to_string())
        .expect("numbered headings should compile");

    let ProjectionPayload::SpecProjection(payload) = &record.payload else {
        panic!("expected spec projection payload");
    };
    assert_eq!(payload.functional_requirements[0].requirement_id, "req-001");
    assert_eq!(payload.success_criteria[0].criterion_id, "ac-001");
}

#[test]
fn spec_projection_accepts_table_entries_inside_required_section_children() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let spec_path = tempdir.path().join("spec.md");
    std::fs::write(
        &spec_path,
        "# Spec\n\n\
## 功能需求\n\n\
### 后端服务\n\n\
| ID | 需求描述 | 优先级 |\n\
|---|---|---|\n\
| **REQ-AUTH-001** | 用户可以通过用户名和密码登录。 | P0 |\n\n\
### 前端页面\n\n\
| ID | 需求描述 | 优先级 |\n\
|---|---|---|\n\
| **REQ-AUTH-002** | 前端提供登录页面和错误提示。 | P1 |\n\n\
## 成功标准\n\n\
| ID | 验收标准 | 关联需求 |\n\
|---|---|---|\n\
| **AC-001** | 登录成功后返回 JWT。 | REQ-AUTH-001 |\n\
\n\
## 非功能需求\n\n\
| ID | 需求描述 | 优先级 |\n\
|---|---|---|\n\
| NFR-001 | JWT 密钥不得硬编码在前端。 | P0 |\n\
",
    )
    .expect("write spec");
    let source = read_document_model(&spec_path).expect("spec");
    let source_ref = artifact_ref(
        "art_ref_spec_table_children",
        "art_spec_table_children",
        ArtifactKind::Spec,
        &source,
    );

    let record = compile_spec_projection(&source, &source_ref, "N05".to_string())
        .expect("table entries inside child sections should compile");

    let ProjectionPayload::SpecProjection(payload) = &record.payload else {
        panic!("expected spec projection payload");
    };
    assert_eq!(payload.functional_requirements.len(), 2);
    assert_eq!(
        payload.functional_requirements[0].requirement_id,
        "req-auth-001"
    );
    assert_eq!(
        payload.functional_requirements[0].text,
        "用户可以通过用户名和密码登录。"
    );
    assert_eq!(
        payload.functional_requirements[0].priority,
        RequirementPriority::Must
    );
    assert_eq!(
        payload.functional_requirements[1].priority,
        RequirementPriority::Should
    );
    assert_eq!(
        payload.success_criteria[0].related_requirement_ids,
        vec!["req-auth-001".to_string()]
    );
    assert_eq!(
        payload.non_functional_requirements[0].requirement_id,
        "nfr-001"
    );
}

#[test]
fn spec_projection_accepts_requirement_table_description_header() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let spec_path = tempdir.path().join("spec.md");
    std::fs::write(
        &spec_path,
        r#"# Spec

## 功能需求

| ID | 描述 |
|---|---|
| REQ-01 | 函数名为 `fibonacciSquareSum`，接收一个正整数参数 `n` |
| REQ-02 | 函数返回斐波那契数列前 n 项的平方和 |

## 成功标准

- `fibonacciSquareSum(n)` 对正整数输入返回正确的平方和值
- 基础测试全部通过
"#,
    )
    .expect("write spec");
    let source = read_document_model(&spec_path).expect("spec");
    let source_ref = artifact_ref(
        "art_ref_spec_description_header",
        "art_spec_description_header",
        ArtifactKind::Spec,
        &source,
    );

    let record = compile_spec_projection(&source, &source_ref, "N05".to_string())
        .expect("requirement tables with 描述 header should compile");

    let ProjectionPayload::SpecProjection(payload) = &record.payload else {
        panic!("expected spec projection payload");
    };
    assert_eq!(payload.functional_requirements[0].requirement_id, "req-01");
    assert_eq!(
        payload.functional_requirements[0].text,
        "函数名为 `fibonacciSquareSum`，接收一个正整数参数 `n`"
    );
    assert_eq!(payload.success_criteria[0].criterion_id, "ac-001");
}

#[test]
fn spec_projection_synthesizes_functional_requirements_from_real_provider_numbered_list() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let spec_path = tempdir.path().join("spec.md");
    std::fs::write(
        &spec_path,
        r#"# Spec: fibonacciSquareSum 实现

## 功能需求

1. **函数签名**：提供名为 `fibonacciSquareSum(n)` 的函数，接收单一整数参数 `n`。
2. **核心算法**：利用公式 `F(1)² + ... + F(n)² = F(n) × F(n+1)` 计算结果。

## 成功标准

1. 对已知输入，函数输出与数学恒等式一致。
2. 测试文件运行后全部通过，无失败断言。
"#,
    )
    .expect("write spec");
    let source = read_document_model(&spec_path).expect("spec");
    let source_ref = artifact_ref(
        "art_ref_spec_real_numbered_list",
        "art_spec_real_numbered_list",
        ArtifactKind::Spec,
        &source,
    );

    let record = compile_spec_projection(&source, &source_ref, "N05".to_string())
        .expect("real provider numbered lists should compile");

    let ProjectionPayload::SpecProjection(payload) = &record.payload else {
        panic!("expected spec projection payload");
    };
    assert_eq!(payload.functional_requirements[0].requirement_id, "req-001");
    assert_eq!(
        payload.functional_requirements[0].text,
        "**函数签名**：提供名为 `fibonacciSquareSum(n)` 的函数，接收单一整数参数 `n`。"
    );
    assert_eq!(payload.success_criteria[0].criterion_id, "ac-001");
}

#[test]
fn spec_projection_accepts_heading_entries_inside_required_section_children() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let spec_path = tempdir.path().join("spec.md");
    std::fs::write(
        &spec_path,
        "# Spec\n\n\
## 用户故事\n\n\
### US-001 用户登录\n\
作为系统用户，我希望通过用户名和密码登录。\n\n\
## 功能需求\n\n\
### REQ-001 后端登录接口\n\
后端必须提供 `/login` 接口。\n\n\
### REQ-002 前端登录页面\n\
前端必须提供登录页面。\n\n\
## 成功标准\n\n\
### AC-001 成功登录\n\
给定有效的用户名和密码，系统应返回有效的 JWT Token。\n\n\
**Refs:** REQ-001\n\n\
## 非功能需求\n\n\
### NF-001 安全性\n\
JWT Secret 必须通过环境变量或配置文件注入。\n\
",
    )
    .expect("write spec");
    let source = read_document_model(&spec_path).expect("spec");
    let source_ref = artifact_ref(
        "art_ref_spec_heading_children",
        "art_spec_heading_children",
        ArtifactKind::Spec,
        &source,
    );

    let record = compile_spec_projection(&source, &source_ref, "N05".to_string())
        .expect("heading entries inside child sections should compile");

    let ProjectionPayload::SpecProjection(payload) = &record.payload else {
        panic!("expected spec projection payload");
    };
    assert_eq!(payload.user_stories[0].story_id, "us-001");
    assert_eq!(payload.functional_requirements[0].requirement_id, "req-001");
    assert_eq!(payload.functional_requirements[0].text, "后端登录接口");
    assert_eq!(
        payload.success_criteria[0].related_requirement_ids,
        vec!["req-001".to_string()]
    );
    assert_eq!(
        payload.non_functional_requirements[0].requirement_id,
        "nf-001"
    );
}

#[test]
fn spec_projection_accepts_bilingual_sections_bold_entries_and_checklist_criteria() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let spec_path = tempdir.path().join("spec.md");
    std::fs::write(
        &spec_path,
        "# Spec\n\n\
## 3. 功能需求（Functional Requirements）\n\n\
**REQ-01 - 登录页面**\n\
- 前端提供一个登录页面，包含用户名输入框、密码输入框和登录按钮。\n\n\
**REQ-02 - 登录 API**\n\
- 后端提供一个 `POST /api/login` 接口。\n\n\
## 4. 成功标准（Success Criteria）\n\n\
- [ ] 用户可以通过前端页面成功登录，并收到 JWT Token。\n\
- [ ] 使用错误凭据登录时，前端显示错误提示，后端返回 401。\n\n\
## 6. 非功能需求（Non-Functional Requirements）\n\n\
**NFR-01 - 安全**\n\
- JWT Secret 长度至少 256 位。\n\
",
    )
    .expect("write spec");
    let source = read_document_model(&spec_path).expect("spec");
    let source_ref = artifact_ref(
        "art_ref_spec_bilingual",
        "art_spec_bilingual",
        ArtifactKind::Spec,
        &source,
    );

    let record = compile_spec_projection(&source, &source_ref, "N05".to_string())
        .expect("bilingual heading and bold entries should compile");

    let ProjectionPayload::SpecProjection(payload) = &record.payload else {
        panic!("expected spec projection payload");
    };
    assert_eq!(payload.functional_requirements[0].requirement_id, "req-01");
    assert_eq!(payload.functional_requirements[0].text, "登录页面");
    assert_eq!(payload.success_criteria[0].criterion_id, "ac-001");
    assert_eq!(
        payload.success_criteria[0].text,
        "用户可以通过前端页面成功登录，并收到 JWT Token。"
    );
    assert_eq!(
        payload.non_functional_requirements[0].requirement_id,
        "nfr-01"
    );
}

#[test]
fn spec_projection_accepts_bold_id_bullet_entries() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let spec_path = tempdir.path().join("spec.md");
    std::fs::write(
        &spec_path,
        "# Spec\n\n\
## 功能需求\n\n\
### 后端服务\n\n\
- **REQ-001** 注册接口 `POST /api/auth/register` 创建内存用户。\n\
- **REQ-002** 登录接口校验成功后签发 JWT。\n\n\
## 成功标准\n\n\
- **AC-001** 用户可通过前端注册页完成账号注册。\n\
- **AC-002** 已注册用户可通过前端登录页完成认证。\n\n\
## 非功能需求\n\n\
- **NF-001 安全性** JWT Secret 应通过环境变量或配置文件注入。\n\
",
    )
    .expect("write spec");
    let source = read_document_model(&spec_path).expect("spec");
    let source_ref = artifact_ref(
        "art_ref_spec_bold_bullets",
        "art_spec_bold_bullets",
        ArtifactKind::Spec,
        &source,
    );

    let record = compile_spec_projection(&source, &source_ref, "N05".to_string())
        .expect("bold ID bullet entries should compile");

    let ProjectionPayload::SpecProjection(payload) = &record.payload else {
        panic!("expected spec projection payload");
    };
    assert_eq!(payload.functional_requirements[0].requirement_id, "req-001");
    assert_eq!(
        payload.functional_requirements[0].text,
        "注册接口 `POST /api/auth/register` 创建内存用户。"
    );
    assert_eq!(payload.success_criteria[1].criterion_id, "ac-002");
    assert_eq!(
        payload.non_functional_requirements[0].requirement_id,
        "nf-001"
    );
}

#[test]
fn spec_projection_ignores_bracket_category_after_requirement_id() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let spec_path = tempdir.path().join("spec.md");
    std::fs::write(
        &spec_path,
        "# Spec\n\n\
## 功能需求\n\n\
- **REQ-001** [前端]: 提供登录页面，包含用户名和密码输入框。\n\
- **REQ-002** [后端]: 提供 `POST /api/login` 接口。\n\n\
## 成功标准\n\n\
- **AC-001** 用户可完成登录并获得 JWT。\n",
    )
    .expect("write spec");
    let source = read_document_model(&spec_path).expect("spec");
    let source_ref = artifact_ref(
        "art_ref_spec_bracket_category",
        "art_spec_bracket_category",
        ArtifactKind::Spec,
        &source,
    );

    let record = compile_spec_projection(&source, &source_ref, "N05".to_string())
        .expect("bracket category labels after IDs should compile");

    let ProjectionPayload::SpecProjection(payload) = &record.payload else {
        panic!("expected spec projection payload");
    };
    assert_eq!(payload.functional_requirements[0].requirement_id, "req-001");
    assert_eq!(payload.functional_requirements[1].requirement_id, "req-002");
    assert!(payload.functional_requirements[0].text.contains("前端"));
    assert_eq!(payload.success_criteria[0].criterion_id, "ac-001");
}

#[test]
fn spec_projection_accepts_localized_table_id_headers() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let spec_path = tempdir.path().join("spec.md");
    std::fs::write(
        &spec_path,
        "# Spec\n\n\
## 功能需求\n\n\
| 需求 ID | 需求描述 | 优先级 |\n\
|---|---|---|\n\
| REQ-AUTH-001 | 后端必须提供 `/api/login` 端点。 | Must |\n\n\
## 成功标准\n\n\
| 验收标准 ID | 验收标准描述 | 关联需求 |\n\
|---|---|---|\n\
| AC-AUTH-001 | 使用正确凭据调用登录 API，应返回 HTTP 200。 | REQ-AUTH-001 |\n\n\
## 非功能需求\n\n\
| 需求 ID | 需求描述 | 类别 |\n\
|---|---|---|\n\
| NFR-SEC-001 | JWT Secret Key 必须从环境变量获取。 | 安全 |\n\
",
    )
    .expect("write spec");
    let source = read_document_model(&spec_path).expect("spec");
    let source_ref = artifact_ref(
        "art_ref_spec_localized_table",
        "art_spec_localized_table",
        ArtifactKind::Spec,
        &source,
    );

    let record = compile_spec_projection(&source, &source_ref, "N05".to_string())
        .expect("localized table ID headers should compile");

    let ProjectionPayload::SpecProjection(payload) = &record.payload else {
        panic!("expected spec projection payload");
    };
    assert_eq!(
        payload.functional_requirements[0].requirement_id,
        "req-auth-001"
    );
    assert_eq!(payload.success_criteria[0].criterion_id, "ac-auth-001");
    assert_eq!(
        payload.non_functional_requirements[0].requirement_id,
        "nfr-sec-001"
    );
}

#[test]
fn spec_projection_accepts_open_item_table_question_column() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let spec_path = tempdir.path().join("spec.md");
    std::fs::write(
        &spec_path,
        "# Spec\n\n\
# 功能需求\n\n\
| ID | 需求 | 说明 | 优先级 |\n\
|----|------|------|--------|\n\
| REQ-001 | 前端登录页面 | 提供用户名、密码输入框及登录按钮。 | must |\n\n\
# 成功标准\n\n\
- 用户可以成功登录并获取 JWT Token。\n\n\
# 待确认项\n\n\
| 编号 | 问题 | 当前假设 |\n\
|------|------|----------|\n\
| OQ-001 | 前端技术栈选型？ | 纯 HTML + CSS + JavaScript |\n\
",
    )
    .expect("write spec");
    let source = read_document_model(&spec_path).expect("spec");
    let source_ref = artifact_ref(
        "art_ref_spec_open_question",
        "art_spec_open_question",
        ArtifactKind::Spec,
        &source,
    );

    let record = compile_spec_projection(&source, &source_ref, "N05".to_string())
        .expect("open item question column should compile");

    let ProjectionPayload::SpecProjection(payload) = &record.payload else {
        panic!("expected spec projection payload");
    };
    assert_eq!(payload.open_items[0].item_id, "oq-001");
    assert_eq!(payload.open_items[0].text, "前端技术栈选型？");
}

#[test]
fn spec_projection_accepts_real_provider_localized_story_and_question_tables() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let spec_path = tempdir.path().join("spec.md");
    std::fs::write(
        &spec_path,
        "# 用户登录功能 Spec\n\n\
## 2. 用户故事\n\n\
| ID | 用户故事 |\n\
|---|---|\n\
| US-001 | 作为玩家，我希望通过用户名和密码登录游戏。 |\n\n\
## 3. 功能需求\n\n\
| ID | 需求描述 | 优先级 |\n\
|---|---|---|\n\
| REQ-001 | 前端登录页面提供用户名输入框、密码输入框和登录按钮。 | P0 |\n\n\
## 4. 成功标准\n\n\
1. 玩家使用预设凭证可以成功登录，并获得有效的 JWT Token。\n\n\
## 5. 待确认项（Open Questions）\n\n\
| ID | 问题描述 |\n\
|---|---|\n\
| Q-001 | 前端技术栈选型：React、Vue、纯 HTML/JS 还是其他？ |\n",
    )
    .expect("write spec");
    let source = read_document_model(&spec_path).expect("spec");
    let source_ref = artifact_ref(
        "art_ref_spec_real_provider_tables",
        "art_spec_real_provider_tables",
        ArtifactKind::Spec,
        &source,
    );

    let record = compile_spec_projection(&source, &source_ref, "N05".to_string())
        .expect("localized story and question tables should compile");

    let ProjectionPayload::SpecProjection(payload) = &record.payload else {
        panic!("expected spec projection payload");
    };
    assert_eq!(payload.user_stories[0].story_id, "us-001");
    assert_eq!(
        payload.user_stories[0].title,
        "作为玩家，我希望通过用户名和密码登录游戏。"
    );
    assert_eq!(payload.open_items[0].item_id, "q-001");
    assert_eq!(
        payload.open_items[0].text,
        "前端技术栈选型：React、Vue、纯 HTML/JS 还是其他？"
    );
}

#[test]
fn spec_projection_truncates_inline_metadata_before_following_paragraph_body() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let spec_path = tempdir.path().join("spec.md");
    std::fs::write(
        &spec_path,
        "# Spec\n\n\
## 功能需求\n\n\
**[REQ-001] 前端登录页面**\n\
系统必须提供登录页面。\n\n\
**[REQ-002] 后端登录 API**\n\
系统必须提供登录接口。\n\n\
## 成功标准\n\n\
**[AC-001] 正常登录流程** Refs: REQ-001, REQ-002\n\
给定有效的预设用户名和密码，系统应返回有效的 JWT 令牌。\n",
    )
    .expect("write spec");
    let source = read_document_model(&spec_path).expect("spec");
    let source_ref = artifact_ref(
        "art_ref_spec_inline_metadata_body",
        "art_spec_inline_metadata_body",
        ArtifactKind::Spec,
        &source,
    );

    let record = compile_spec_projection(&source, &source_ref, "N05".to_string())
        .expect("inline metadata should stop before following body line");

    let ProjectionPayload::SpecProjection(payload) = &record.payload else {
        panic!("expected spec projection payload");
    };
    assert_eq!(
        payload.success_criteria[0].related_requirement_ids,
        vec!["req-001".to_string(), "req-002".to_string()]
    );
}

#[test]
fn spec_projection_ignores_non_requirement_words_in_success_refs() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let spec_path = tempdir.path().join("spec.md");
    std::fs::write(
        &spec_path,
        "# Spec\n\n\
## 功能需求\n\n\
- **[REQ-001] 登录接口**\n\n\
## 成功标准\n\n\
- **[AC-001]** 代码通过所有单元测试。Refs: 全局\n\
- **[AC-002]** 登录成功返回 JWT。Refs: REQ-001\n\
",
    )
    .expect("write spec");
    let source = read_document_model(&spec_path).expect("spec");
    let source_ref = artifact_ref(
        "art_ref_spec_global_refs",
        "art_spec_global_refs",
        ArtifactKind::Spec,
        &source,
    );

    let record = compile_spec_projection(&source, &source_ref, "N05".to_string())
        .expect("non requirement refs should be ignored");

    let ProjectionPayload::SpecProjection(payload) = &record.payload else {
        panic!("expected spec projection payload");
    };
    assert!(
        payload.success_criteria[0]
            .related_requirement_ids
            .is_empty()
    );
    assert_eq!(
        payload.success_criteria[1].related_requirement_ids,
        vec!["req-001".to_string()]
    );
}

#[test]
fn spec_projection_accepts_chinese_priority_values() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let spec_path = tempdir.path().join("spec.md");
    std::fs::write(
        &spec_path,
        "# Spec\n\n\
## 功能需求\n\n\
| ID | 需求描述 | 优先级 |\n\
|----|---------|--------|\n\
| REQ-001 | 后端提供登录接口。 | 高 |\n\
| REQ-002 | 后端配置 CORS。 | 中 |\n\
| REQ-003 | 页面展示辅助提示。 | 低 |\n\n\
## 成功标准\n\n\
- **[AC-001]** 登录成功返回 JWT。Refs: REQ-001\n\
",
    )
    .expect("write spec");
    let source = read_document_model(&spec_path).expect("spec");
    let source_ref = artifact_ref(
        "art_ref_spec_chinese_priority",
        "art_spec_chinese_priority",
        ArtifactKind::Spec,
        &source,
    );

    let record = compile_spec_projection(&source, &source_ref, "N05".to_string())
        .expect("Chinese priorities should compile");

    let ProjectionPayload::SpecProjection(payload) = &record.payload else {
        panic!("expected spec projection payload");
    };
    assert_eq!(
        payload.functional_requirements[0].priority,
        RequirementPriority::Must
    );
    assert_eq!(
        payload.functional_requirements[1].priority,
        RequirementPriority::Should
    );
    assert_eq!(
        payload.functional_requirements[2].priority,
        RequirementPriority::Could
    );
}

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
