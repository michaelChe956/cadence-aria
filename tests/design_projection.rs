use cadence_aria::cross_cutting::artifact_projection::compile_design_projection;
use cadence_aria::cross_cutting::artifact_validate::{ArtifactIndex, projection_validator};
use cadence_aria::cross_cutting::document_ops::read_document_model;
use cadence_aria::protocol::artifacts::{
    ArtifactKind, ArtifactRef, ArtifactStatus, ProjectionKind,
};
use cadence_aria::protocol::projections::ProjectionPayload;
use serde_json::Value;

#[test]
fn design_projection_compiles_decisions_components_and_risks() {
    let source =
        read_document_model("tests/fixtures/artifacts/design.md".as_ref()).expect("design");
    let source_ref = artifact_ref(
        "art_ref_design_0001",
        "art_design_001",
        ArtifactKind::Design,
        &source,
    );

    let record =
        compile_design_projection(&source, &source_ref, "N07".to_string()).expect("compile design");

    assert_eq!(record.projection_kind, ProjectionKind::DesignProjection);
    let ProjectionPayload::DesignProjection(payload) = &record.payload else {
        panic!("expected design projection payload");
    };
    assert_eq!(payload.design_decisions[0].design_decision_id, "dd-001");
    assert_eq!(
        payload.design_decisions[0].text,
        "REPL 只作为客户端，daemon 是 runtime truth。"
    );
    assert_eq!(payload.shared_components[0].component_id, "cmp-001");
    assert_eq!(payload.risk_refs[0].severity.to_string(), "high");
    assert_eq!(
        payload.risk_refs[0].related_design_decision_ids,
        vec!["dd-001".to_string()]
    );

    let golden: Value = serde_json::from_str(include_str!(
        "fixtures/artifacts/golden/design_projection.json"
    ))
    .expect("golden json");
    assert_eq!(serde_json::to_value(payload).expect("payload json"), golden);

    let validation = projection_validator(
        &record,
        &ArtifactIndex::from_active_refs(vec![source_ref]),
        Some("tests/fixtures/artifacts/golden/design_projection.json".as_ref()),
    )
    .expect("projection validation");
    assert!(validation.valid);
}

#[test]
fn design_projection_requires_design_decisions() {
    let source =
        read_document_model("tests/fixtures/artifacts/design_missing_decision.md".as_ref())
            .expect("design");
    let source_ref = artifact_ref(
        "art_ref_design_0002",
        "art_design_002",
        ArtifactKind::Design,
        &source,
    );

    let error = compile_design_projection(&source, &source_ref, "N07".to_string())
        .expect_err("missing decisions should fail");

    assert_eq!(
        error.to_string(),
        "missing required projection section 设计决策"
    );
}

#[test]
fn design_projection_accepts_decision_ids_with_dec_prefix() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let design_path = tempdir.path().join("design.md");
    std::fs::write(
        &design_path,
        "# Design\n\n\
## 设计决策\n\n\
- **[DEC-001] 前后端分离**：前端与后端作为独立服务开发。\n\
- **[DEC-002] JWT 无状态认证**：选用 JWT 避免服务端会话状态。\n\n\
## 风险\n\n\
- **[RISK-001] Token 泄露风险**：localStorage 中的 JWT 易受 XSS 攻击。\n\
",
    )
    .expect("write design");
    let source = read_document_model(&design_path).expect("design");
    let source_ref = artifact_ref(
        "art_ref_design_dec",
        "art_design_dec",
        ArtifactKind::Design,
        &source,
    );

    let record = compile_design_projection(&source, &source_ref, "N07".to_string())
        .expect("DEC decision ids should compile");

    let ProjectionPayload::DesignProjection(payload) = &record.payload else {
        panic!("expected design projection payload");
    };
    assert_eq!(payload.design_decisions[0].design_decision_id, "dec-001");
    assert_eq!(payload.design_decisions[1].design_decision_id, "dec-002");
}

#[test]
fn design_projection_synthesizes_decision_ids_for_decision_tables_without_ids() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let design_path = tempdir.path().join("design.md");
    std::fs::write(
        &design_path,
        "# Design\n\n\
## 设计决策\n\n\
| 决策项 | 选择 | 理由 |\n\
|--------|------|------|\n\
| 前端框架 | React 18 + Vite | 生态成熟，启动速度快 |\n\
| 后端框架 | Express | 轻量高效，JWT 生态完善 |\n\
",
    )
    .expect("write design");
    let source = read_document_model(&design_path).expect("design");
    let source_ref = artifact_ref(
        "art_ref_design_table",
        "art_design_table",
        ArtifactKind::Design,
        &source,
    );

    let record = compile_design_projection(&source, &source_ref, "N07".to_string())
        .expect("decision table without ids should compile");

    let ProjectionPayload::DesignProjection(payload) = &record.payload else {
        panic!("expected design projection payload");
    };
    assert_eq!(payload.design_decisions[0].design_decision_id, "dec-001");
    assert!(
        payload.design_decisions[0]
            .text
            .contains("决策项: 前端框架; 选择: React 18 + Vite")
    );
}

#[test]
fn design_projection_ignores_bracketed_related_requirement_lists() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let design_path = tempdir.path().join("design.md");
    std::fs::write(
        &design_path,
        "# Design\n\n\
## 设计决策\n\n\
- [DEC-001] 后端框架采用 FastAPI。\n\
  - Related: [req-001, req-002, req-003]\n\
- [DEC-002] 前端采用 Vanilla JavaScript。\n\
  - Related: [req-004]\n\n\
## 风险\n\n\
- [RISK-001] Token 泄露风险。Severity: high; Refs: DEC-001\n\
",
    )
    .expect("write design");
    let source = read_document_model(&design_path).expect("design");
    let source_ref = artifact_ref(
        "art_ref_design_related_lists",
        "art_design_related_lists",
        ArtifactKind::Design,
        &source,
    );

    let record = compile_design_projection(&source, &source_ref, "N09".to_string())
        .expect("related requirement lists should not be parsed as design decision IDs");

    let ProjectionPayload::DesignProjection(payload) = &record.payload else {
        panic!("expected design projection payload");
    };
    assert_eq!(payload.design_decisions.len(), 2);
    assert_eq!(payload.design_decisions[0].design_decision_id, "dec-001");
    assert_eq!(payload.design_decisions[1].design_decision_id, "dec-002");
}

#[test]
fn design_projection_compiles_real_provider_metadata_bullets_under_headings() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let design_path = tempdir.path().join("design.md");
    std::fs::write(
        &design_path,
        "# Design\n\n\
## 设计决策\n\n\
### dec-001 — 恒等式实现\n\n\
- **related_requirement_ids**: [fr-02]\n\
- **决策**: `fibonacciSquareSum(n)` 直接返回 `fibonacci(n) * fibonacci(n + 1)`。\n\n\
### dec-002 — 迭代 Fibonacci 内部逻辑\n\n\
- **related_requirement_ids**: [fr-03, fr-04]\n\
- **决策**: `fibonacci(k)` 使用 `prev` / `curr` 两个变量迭代更新。\n\n\
## 风险\n\n\
### R-001 — 大整数溢出\n\n\
- **severity**: low\n\
- **related_design_decision_ids**: [dec-001, dec-002]\n\
- **描述**: n 较大时可能超出 JavaScript Number 安全整数范围。\n",
    )
    .expect("write design");
    let source = read_document_model(&design_path).expect("design");
    let source_ref = artifact_ref(
        "art_ref_design_real_provider_metadata",
        "art_design_real_provider_metadata",
        ArtifactKind::Design,
        &source,
    );

    let record = compile_design_projection(&source, &source_ref, "N09".to_string())
        .expect("real provider metadata bullets should compile");

    let ProjectionPayload::DesignProjection(payload) = &record.payload else {
        panic!("expected design projection payload");
    };
    assert_eq!(payload.design_decisions.len(), 2);
    assert_eq!(
        payload.design_decisions[0].related_requirement_ids,
        vec!["fr-02".to_string()]
    );
    assert_eq!(
        payload.design_decisions[1].related_requirement_ids,
        vec!["fr-03".to_string(), "fr-04".to_string()]
    );
    assert_eq!(payload.risk_refs.len(), 1);
    assert_eq!(payload.risk_refs[0].risk_id, "risk-001");
    assert_eq!(
        payload.risk_refs[0].related_design_decision_ids,
        vec!["dec-001".to_string(), "dec-002".to_string()]
    );
}

#[test]
fn design_projection_accepts_real_provider_canonical_definition_tables() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let design_path = tempdir.path().join("design.md");
    std::fs::write(
        &design_path,
        "# Design\n\n\
## 设计决策\n\n\
- **DEC-001 后端技术栈**：选用 Node.js + Express。\n\n\
## 规范定义清单（Canonical Projection 来源）\n\n\
### shared_components\n\n\
| 组件标识 | 组件名 | 类型 | 职责 | 文件建议位置 |\n\
|----------|--------|------|------|-------------|\n\
| sc-001 | AuthMiddleware | backend_middleware | 验证 Authorization Header 中的 JWT，解析 req.user | src/middleware/auth.js |\n\n\
### data_entities\n\n\
| 实体标识 | 实体名 | 类型 | 字段定义 | 存储方式 |\n\
|----------|--------|------|----------|----------|\n\
| de-001 | User | memory_entity | id, username, passwordHash, createdAt | 进程内 Map |\n\n\
### api_entries\n\n\
| API 标识 | 方法 | 路径 | 请求契约 | 成功响应 | 错误响应 | 认证要求 |\n\
|----------|------|------|----------|----------|----------|----------|\n\
| api-001 | POST | /api/auth/login | username, password | accessToken, user | error | 无 |\n\n\
### shared_modules\n\n\
| 模块标识 | 模块名 | 类型 | 职责 | 文件建议位置 |\n\
|----------|--------|------|------|-------------|\n\
| sm-001 | server | backend_entry | Express 应用启动入口，挂载中间件与路由 | src/server.js |\n\n\
## 风险\n\n\
- **RISK-001 内存数据易失性**：进程重启后用户数据全部丢失。\n\
- **RISK-005 JWT Secret 缺失启动失败**：应用启动时若 JWT_SECRET 环境变量缺失或长度不足 32 位，服务应立即抛出错误并退出。\n",
    )
    .expect("write design");
    let source = read_document_model(&design_path).expect("design");
    let source_ref = artifact_ref(
        "art_ref_design_real_provider_tables",
        "art_design_real_provider_tables",
        ArtifactKind::Design,
        &source,
    );

    let record = compile_design_projection(&source, &source_ref, "N09".to_string())
        .expect("real provider canonical definition tables should compile");

    let ProjectionPayload::DesignProjection(payload) = &record.payload else {
        panic!("expected design projection payload");
    };
    assert_eq!(payload.shared_components[0].component_id, "sc-001");
    assert_eq!(payload.shared_components[0].name, "AuthMiddleware");
    assert_eq!(payload.data_entities[0].entity_id, "de-001");
    assert_eq!(payload.data_entities[0].name, "User");
    assert_eq!(payload.api_entries[0].api_id, "api-001");
    assert_eq!(payload.api_entries[0].name, "/api/auth/login");
    assert_eq!(payload.shared_modules[0].component_id, "sm-001");
    assert!(
        payload
            .risk_refs
            .iter()
            .any(|risk| risk.risk_id == "risk-005")
    );
}

#[test]
fn design_projection_synthesizes_ids_from_provider_heading_sections() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let design_path = tempdir.path().join("design.md");
    std::fs::write(
        &design_path,
        r#"# 认证模块设计文档

## 设计决策

| 决策项 | 选择 | 理由 |
|--------|------|------|
| 认证协议 | JWT (HS256) | 单服务场景下对称密钥足够 |

## 公共组件

### 1. AuthMiddleware

- **职责**：拦截请求，提取并校验 Access Token

## 数据实体

### User（内存对象）

| 字段 | 类型 | 说明 |
|------|------|------|
| username | string | 登录用户名 |

## API 契约

### POST /api/auth/login

**请求体**：username/password

**成功响应**：access_token

## 风险

1. **内存数据丢失风险**：服务重启后内存中的用户、token、撤销列表全部丢失。

## 待确认项

1. 预设用户的具体列表和初始密码？
"#,
    )
    .expect("write design");
    let source = read_document_model(&design_path).expect("design");
    let source_ref = artifact_ref(
        "art_ref_design_heading_sections",
        "art_design_heading_sections",
        ArtifactKind::Design,
        &source,
    );

    let record = compile_design_projection(&source, &source_ref, "N09".to_string())
        .expect("heading-only provider sections should compile");

    let ProjectionPayload::DesignProjection(payload) = &record.payload else {
        panic!("expected design projection payload");
    };
    assert_eq!(payload.design_decisions[0].design_decision_id, "dec-001");
    assert_eq!(payload.shared_components[0].component_id, "cmp-001");
    assert_eq!(payload.shared_components[0].name, "AuthMiddleware");
    assert_eq!(payload.data_entities[0].entity_id, "de-001");
    assert_eq!(payload.data_entities[0].name, "User（内存对象）");
    assert_eq!(payload.api_entries[0].api_id, "api-001");
    assert_eq!(payload.api_entries[0].name, "POST /api/auth/login");
    assert_eq!(payload.risk_refs[0].risk_id, "risk-001");
    assert_eq!(payload.open_items[0].item_id, "oq-001");
}

#[test]
fn design_projection_synthesizes_decision_ids_from_numbered_decision_list() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let design_path = tempdir.path().join("design.md");
    std::fs::write(
        &design_path,
        r#"# 候选设计方案

## 设计决策

1. **输出格式决策**：采用标准 design artifact 格式。
2. **公式实现决策**：使用 `F(n) * F(n+1)` 计算平方和。

## 公共组件

> 当前无需共享组件。

## 风险

1. **精度风险**：大数场景可能超过 JavaScript number 安全范围。
"#,
    )
    .expect("write design");
    let source = read_document_model(&design_path).expect("design");
    let source_ref = artifact_ref(
        "art_ref_design_numbered_decisions",
        "art_design_numbered_decisions",
        ArtifactKind::Design,
        &source,
    );

    let record = compile_design_projection(&source, &source_ref, "N07".to_string())
        .expect("numbered decision lists should compile");

    let ProjectionPayload::DesignProjection(payload) = &record.payload else {
        panic!("expected design projection payload");
    };
    assert_eq!(payload.design_decisions[0].design_decision_id, "dec-001");
    assert_eq!(payload.design_decisions[1].design_decision_id, "dec-002");
    assert_eq!(payload.risk_refs[0].risk_id, "risk-001");
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
