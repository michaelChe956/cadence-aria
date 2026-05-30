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

    let golden: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/artifacts/golden/design_projection.json"
    )))
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

#[test]
fn design_projection_synthesizes_decision_ids_from_numbered_child_headings() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let design_path = tempdir.path().join("design.md");
    std::fs::write(
        &design_path,
        r#"# climbStairs 模块设计

## 设计决策

### 1. 算法选择：迭代法 + O(1) 空间优化

采用双变量滚动迭代实现斐波那契递推。

### 2. 校验策略：Fail-fast 前置校验

函数入口处依次执行类型校验、整型校验、非负校验。

## 公共组件

### `climbStairs(n: number): number`

- **功能**：计算每次可爬 1 或 2 阶时，爬到第 n 阶的不同方法数

## 风险

1. **数值溢出**：当 n 较大时可能超过 Number.MAX_SAFE_INTEGER。
"#,
    )
    .expect("write design");
    let source = read_document_model(&design_path).expect("design");
    let source_ref = artifact_ref(
        "art_ref_design_numbered_child_headings",
        "art_design_numbered_child_headings",
        ArtifactKind::Design,
        &source,
    );

    let record = compile_design_projection(&source, &source_ref, "N07".to_string())
        .expect("numbered child heading decisions should compile");

    let ProjectionPayload::DesignProjection(payload) = &record.payload else {
        panic!("expected design projection payload");
    };
    assert_eq!(payload.design_decisions[0].design_decision_id, "dec-001");
    assert_eq!(payload.design_decisions[1].design_decision_id, "dec-002");
    assert_eq!(payload.shared_components[0].component_id, "cmp-001");
    assert_eq!(payload.risk_refs[0].risk_id, "risk-001");
}

#[test]
fn design_projection_accepts_short_risk_ids_from_provider_tables() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let design_path = tempdir.path().join("design.md");
    std::fs::write(
        &design_path,
        r#"# climbStairs 设计文档

## 设计决策

### D-01 算法选择：滚动变量动态规划

使用两个滚动变量维护前两个状态，单次循环。

## 风险

| ID | 级别 | 描述 | 缓解 |
|----|------|------|------|
| R-01 | 中 | 非法输入策略可能不一致 | 在 API 契约与测试断言中固化异常类型 |
"#,
    )
    .expect("write design");
    let source = read_document_model(&design_path).expect("design");
    let source_ref = artifact_ref(
        "art_ref_design_short_risk_ids",
        "art_design_short_risk_ids",
        ArtifactKind::Design,
        &source,
    );

    let record = compile_design_projection(&source, &source_ref, "N07".to_string())
        .expect("short provider risk ids should compile");

    let ProjectionPayload::DesignProjection(payload) = &record.payload else {
        panic!("expected design projection payload");
    };
    assert_eq!(payload.design_decisions[0].design_decision_id, "dec-001");
    assert_eq!(payload.risk_refs[0].risk_id, "risk-01");
}

#[test]
fn design_projection_accepts_decision_text_from_provider_decision_tables() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let design_path = tempdir.path().join("design.md");
    std::fs::write(
        &design_path,
        r#"# climbStairs 设计文档

## 设计决策

| ID | 决策 | 选项对比 | 选择理由 | 关联需求 |
|---|---|---|---|---|
| DD-001 | 算法选型采用双变量滚动迭代 Fibonacci | 递归 / DP 数组 / 双变量滚动 | O(n) 时间 + O(1) 空间 | FR-CLIMB-005 |

## 风险

| ID | 级别 | 描述 | 缓解 |
|----|------|------|------|
| R-001 | 中 | 非法输入策略可能不一致 | 在测试断言中固化异常类型 |
"#,
    )
    .expect("write design");
    let source = read_document_model(&design_path).expect("design");
    let source_ref = artifact_ref(
        "art_ref_design_provider_decision_table",
        "art_design_provider_decision_table",
        ArtifactKind::Design,
        &source,
    );

    let record = compile_design_projection(&source, &source_ref, "N07".to_string())
        .expect("provider decision tables should compile");

    let ProjectionPayload::DesignProjection(payload) = &record.payload else {
        panic!("expected design projection payload");
    };
    assert_eq!(payload.design_decisions[0].design_decision_id, "dd-001");
    assert_eq!(
        payload.design_decisions[0].text,
        "算法选型采用双变量滚动迭代 Fibonacci"
    );
}

#[test]
fn design_projection_accepts_real_climb_stairs_provider_design_shape() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let design_path = tempdir.path().join("design.md");
    std::fs::write(
        &design_path,
        r#"# 候选设计:climbStairs(n) 算法实现

## 设计决策

### DD-01 算法选型:迭代式动态规划(双滚动变量)

- 选择:使用两个滚动变量 `prev1`、`prev2` 自底向上递推 `f(i) = f(i-1) + f(i-2)`,初值 `f(1)=1, f(2)=2`。
- 理由:
  - 满足 FR-01 的递推关系定义。

### DD-02 边界约定:climbStairs(0) 返回 1

- 决策:遵循 spec FR-02 与 clarification_record 默认假设,`climbStairs(0)` 返回 1(空路径计 1 种)。

## 公共组件

| 组件 | 路径 | 复用性 | 是否新增 |
|------|------|--------|----------|
| climbStairs | src/climbStairs.js | 业务方可 require 复用 | 是 |

## 数据模型

| 名称 | 类型 | 含义 | 约束 |
|------|------|------|------|
| n | number(integer) | 入参,目标阶数 | 0 ≤ n ≤ 10000(FR-10) |

## API 契约

### climbStairs(n)

- 签名:`climbStairs(n: number): number`

## 风险

| 编号 | 风险 | 影响 | 缓解 |
|------|------|------|------|
| R-01 | 模块格式假设错误（CJS vs ESM 不匹配仓库实际配置） | 测试 require/import 失败 | daemon 落盘前读取 `package.json` 的 `type` 字段最终化 |
| R-02 | Node.js 版本过低，`node:test` 不可用 | 测试无法运行 | 在测试文件顶部注释最低 Node 版本要求 |

## 待确认项

| 编号 | 内容 | 当前默认假设 | 处置建议 |
|------|------|--------------|----------|
| OQ-01 | climbStairs(0) 返回值是否最终固化为 1 | 返回 1(对齐 DD-02) | 由 daemon 在约束折叠阶段固化 |
"#,
    )
    .expect("write design");
    let source = read_document_model(&design_path).expect("design");
    let source_ref = artifact_ref(
        "art_ref_design_real_climb_stairs_shape",
        "art_design_real_climb_stairs_shape",
        ArtifactKind::Design,
        &source,
    );

    let record = compile_design_projection(&source, &source_ref, "N07".to_string())
        .expect("real climbStairs provider design should compile");

    let ProjectionPayload::DesignProjection(payload) = &record.payload else {
        panic!("expected design projection payload");
    };
    assert_eq!(payload.design_decisions[0].design_decision_id, "dd-01");
    assert!(
        payload
            .api_entries
            .iter()
            .any(|api| api.name == "climbStairs(n)")
    );
    assert_eq!(payload.risk_refs[0].risk_id, "risk-01");
    assert_eq!(payload.open_items[0].item_id, "oq-01");
}

#[test]
fn design_projection_extracts_structured_api_and_component_tables_from_revised_design() {
    let tempdir = tempfile::tempdir().expect("tempdir");
    let design_path = tempdir.path().join("design.md");
    std::fs::write(
        &design_path,
        r#"# Design：climbStairs(n) 函数与单元测试候选设计 v3.0

## 2. 设计决策

| 决策 ID | 决策点 | 选择 | 依据 |
|---------|--------|------|------|
| DD-01 | 算法 | 迭代 DP，两个滚动变量 prev1 / prev2 | req-003 |
| DD-11 | 错误正则匹配 | 测试断言使用 `/climbStairs:/` 匹配 `error.message` | req-007 |

## 3. 公共组件

本设计仅交付两个新增组件，不修改既有公共组件：

| 组件 | 路径 | 责任 |
|------|------|------|
| climbStairs 模块 | `src/climbStairs.js` | 唯一导出 `climbStairs` 纯函数；无副作用 |
| climbStairs 测试套件 | `tests/climbStairs.test.js` | 覆盖合法/非法用例；不依赖外部网络 |

显式不引入：
- 不新增 helpers/utils 模块
- 不修改 `package.json.dependencies` / `devDependencies`

## 5. API 契约

### 5.1 结构化 API 契约（供 design_projection 派生）

| 字段 | 值 |
|------|----|
| name | `climbStairs` |
| module_path | `src/climbStairs.js` |
| export_kind | `named` |
| input.name | `n` |
| input.type | `number` |
| input.constraints | `Number.isInteger(n) === true` 且 `n >= 1` |
| output.type | `number` |
| output.semantics | 斐波那契递推 `dp[i] = dp[i-1] + dp[i-2]` |
| throws.type | `Error` |

### 5.3 错误消息契约

- DD-11 明确不使用 `/^Error: climbStairs:/`，因为 Node 内置 `assert.throws` 对 RegExp 默认匹配 `error.message`。

## 6. 风险

| 风险 ID | 风险描述 | 严重度 | 触发条件 | 缓解措施 |
|---------|----------|--------|----------|----------|
| R-01 | 仓库实际使用 ESM 而默认 CJS | 低 | package.json 缺失 | 实现节点必须把实际选择写 task_run_log |
"#,
    )
    .expect("write design");
    let source = read_document_model(&design_path).expect("design");
    let source_ref = artifact_ref(
        "art_ref_design_revised_structured_tables",
        "art_design_revised_structured_tables",
        ArtifactKind::Design,
        &source,
    );

    let record = compile_design_projection(&source, &source_ref, "N09".to_string())
        .expect("revised structured design should compile");

    let ProjectionPayload::DesignProjection(payload) = &record.payload else {
        panic!("expected design projection payload");
    };
    assert_eq!(payload.shared_components.len(), 2);
    assert_eq!(payload.shared_components[0].name, "climbStairs 模块");
    assert_eq!(
        payload.shared_components[0].responsibility,
        "唯一导出 `climbStairs` 纯函数；无副作用"
    );
    assert_eq!(payload.api_entries.len(), 1);
    assert_eq!(payload.api_entries[0].api_id, "api-001");
    assert_eq!(payload.api_entries[0].name, "climbStairs");
    assert!(payload.api_entries[0].input.contains("input.name: n"));
    assert!(payload.api_entries[0].input.contains("input.type: number"));
    assert!(
        payload.api_entries[0]
            .output
            .contains("output.type: number")
    );
    assert_eq!(payload.risk_refs[0].risk_id, "risk-01");
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
