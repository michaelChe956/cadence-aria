# Aria Phase 1 P2 Implementation Plan

**文档信息**
- **创建日期**：2026-04-26
- **版本**：v1.3（研发可落地性 Review 二次修正版）
- **修正内容**：补齐 `projection_validator` / `phase1_profile_validator` 最小校验项、OpenSpec 惰性 stale 检测、artifact version 递增规则和测试文件清单对齐。

> **自动化 agent 执行提示**：agentic worker 执行本计划时使用 `superpowers:subagent-driven-development` 或 `superpowers:executing-plans`；人类研发按任务、测试命令和完成判定执行即可，不依赖这些 skill。

**Goal:** 建立 Document Operation、canonical artifact 校验、projection 编译、phase1 profile、OpenSpec constraint bundle、traceability binding 这条数据面基础层。

**Architecture:** P2 不碰 provider 调用和节点业务，专注于“产物如何被机器正确消费和结构化修改”。完成后，planning/execution/final closure 都只能通过 Document Operation、canonical artifact、projection、`_aria`、bundle 与 binding 读写产物，而不能自己解析 Markdown、自由拼装 JSON 或对文档做裸字符串替换。

**Tech Stack:** Rust、serde、Markdown parser、JSON/YAML parser、fixture-based tests、可选 ast-grep capability probe。

---

## 0. 评审后准入门槛

P2 是评审中 P0 缺口最集中的阶段。启动 P2 前，必须先落实 `cadence/designs/2026-04-26_技术方案_Aria一期评审后实施规格补齐_v1.4.md`：

- 第 4.5：`ArtifactProjectionRecord`、`SpecProjection`、`DesignProjection`、`PlanProjection` Rust 类型
- 第 4.6：`OpenSpecConstraintBundle` Rust 类型与 snake_case JSON 字段
- 第 5 章：Projection 编译规则、heading mapping、稳定 ID 生成、Markdown parser 裁定
- 第 5.6-5.7：`projection_validator` 与 `phase1_profile_validator` 的输入、输出、错误码和最小校验项
- 第 6 章：OpenSpec 文件到 bundle 字段的映射、缺文件阻断、stale 判定
- 第 8 章：artifact 存储路径、版本号策略、ExternalArtifactRef 生命周期
- 第 10 章：`_aria.traceability_refs` 生成算法
- 第 15 章：fixture 树、最小输入样例与 golden JSON

特别裁定：Projection compiler、OpenSpec bundle compiler 和 fixture golden test 三者必须一起落地；不得只先写空 compiler 或只用 Markdown 原文裸测。

---

## 1. 范围与出口

P2 完成后，必须满足：

1. 17 类一期产物（16 类业务产物 + `runtime_snapshot`）有统一三层 validator：`canonical_validator`（canonical 最小字段）→ `projection_validator`（projection schema / golden JSON）→ `phase1_profile_validator`（`_aria` / traceability / projection refs / constraint refs）
2. Markdown / JSON / YAML 文档修改统一走 Document Operation 层
3. `spec/design/plan` 可以编译出 projection
4. JSON artifact 支持 `_aria` 扩展与 profile validator
5. P1 固化的 `change_id` 可以完成 OpenSpec skeleton bootstrap，并编译出字段名稳定的 `OpenSpecConstraintBundle`
6. traceability binding 可以自动生成

---

## 2. 目标文件结构

**Files:**
- Modify: `Cargo.toml`
- Create: `src/protocol/phase1_profile.rs`
- Create: `src/protocol/projections.rs`
- Create: `src/protocol/constraints.rs`
- Create: `src/protocol/traceability.rs`
- Create: `src/protocol/document_ops.rs`
- Create: `src/cross_cutting/document_ops.rs`
- Create: `src/cross_cutting/ast_grep_tool.rs`
- Create: `src/cross_cutting/artifact_validate.rs`
- Create: `src/cross_cutting/artifact_projection.rs`
- Create: `src/cross_cutting/openspec_constraints.rs`
- Create: `src/cross_cutting/traceability.rs`
- Create: `tests/artifact_validate.rs`
- Create: `tests/artifact_schema_min_fields.rs`
- Create: `tests/spec_projection.rs`
- Create: `tests/document_ops.rs`
- Create: `tests/design_projection.rs`
- Create: `tests/plan_projection.rs`
- Create: `tests/openspec_bundle.rs`
- Create: `tests/openspec_bundle_schema.rs`
- Create: `tests/traceability_binding.rs`
- Create: `tests/superseded_artifact_refs.rs`
- Create: `tests/support/mod.rs`
- Create: `tests/fixtures/artifacts/spec.md`
- Create: `tests/fixtures/artifacts/design.md`
- Create: `tests/fixtures/artifacts/plan.md`
- Create: `tests/fixtures/artifacts/golden/spec_projection.json`
- Create: `tests/fixtures/artifacts/golden/design_projection.json`
- Create: `tests/fixtures/artifacts/golden/plan_projection.json`
- Create: `tests/fixtures/openspec/changes/sample-change/proposal.md`
- Create: `tests/fixtures/openspec/changes/sample-change/specs/main/spec.md`
- Create: `tests/fixtures/openspec/changes/sample-change/design.md`
- Create: `tests/fixtures/openspec/changes/sample-change/tasks.md`

`document_ops.rs` 职责裁定：

| 文件 | 职责 | 禁止事项 |
|------|------|----------|
| `src/protocol/document_ops.rs` | 只放纯类型：`DocumentModel`、`DocumentSection`、`DocumentBlock`、`HeadingPath`、`DocumentPatch`、`DocumentPatchResult` | 不放文件 IO、Markdown parser 调用、sha256 计算、写盘逻辑 |
| `src/cross_cutting/document_ops.rs` | 放实现函数：`read_document_model`、`upsert_section`、`extract_projection_source`、`apply_json_patch`、YAML/JSON structured writer、sha256 计算 | 不定义协议层 canonical artifact 身份，不绕过 `protocol/document_ops.rs` 的类型 |

研发不得新增第三个 document operation 入口。其他模块只能依赖上述两个文件：需要类型时引用 `protocol::document_ops`，需要实际读写时引用 `cross_cutting::document_ops`。

依赖裁定：

- P2 负责在 `Cargo.toml` 增加 Markdown / hash / YAML / JSON 结构化操作所需依赖，例如 `pulldown-cmark`、`sha2`、`serde_yaml`，以及团队选定的 JSON patch 实现或等价内部实现。
- 每次新增依赖后必须运行 `cargo check`，避免后续阶段才发现依赖或 feature 配置错误。

---

## 3. 任务拆解

### Task 1: 建立 Document Operation 基线

**Files:**
- Create: `src/protocol/document_ops.rs`
- Create: `src/cross_cutting/document_ops.rs`
- Create: `src/cross_cutting/ast_grep_tool.rs`
- Test: `tests/document_ops.rs`
- Create: `tests/fixtures/artifacts/spec.md`
- Create: `tests/fixtures/openspec/changes/sample-change/design.md`

职责边界：

- `src/protocol/document_ops.rs`：定义 `DocumentModel`、`DocumentSection`、`DocumentBlock`、`HeadingPath`、`DocumentPatch`、`DocumentPatchResult`，只做 serde 类型与值对象。
- `src/cross_cutting/document_ops.rs`：实现 Markdown / JSON / YAML 的读取、章节 upsert、projection source 抽取、structured patch、sha256 计算和错误映射。
- `src/cross_cutting/ast_grep_tool.rs`：只做可选 capability probe 与可选结构搜索封装，不进入 Markdown canonical artifact 主编辑路径。

- [ ] **Step 1: 写失败测试，覆盖 Markdown 章节级 upsert**

断言：
- 可按 heading path 找到 `## 目标与范围`
- `upsert_section` 只替换目标章节，不改变其他章节顺序
- `upsert_section` 保留目标章节外的空行、表格、代码块与中文 heading；同名 heading 通过完整 `HeadingPath` 定位，不能只按标题文本替换第一处
- 写入后返回新的 sha256
- `DocumentPatchResult` 必须返回 `changed`、`old_sha256`、`new_sha256`、`updated_heading_path`、`warnings[]`

- [ ] **Step 2: 写失败测试，覆盖 JSON / YAML 结构化 patch**

断言：
- JSON artifact 的 `_aria.traceability_refs` 可通过结构化 patch 更新
- OpenSpec YAML / JSON 配置更新后仍可反序列化
- 不允许通过字符串拼接写入非法 JSON

- [ ] **Step 3: 实现 Markdown document model**

要求：
- 解析 heading 层级、段落、列表、表格、代码块
- 提供 `read_document_model`、`upsert_section`、`extract_projection_source`
- 不在 Markdown canonical artifact 中追加隐藏 front matter

- [ ] **Step 4: 实现 structured patch 与 ast-grep capability probe**

要求：
- `apply_json_patch` 基于 serde JSON value 操作
- YAML / JSON 配置文件通过 serde parser 读写
- `ast_grep_tool` 只做 capability probe 和可选命令封装；未安装 ast-grep 时返回 `missing_optional_tool`，不阻断 P2 主流程
- 明确 ast-grep 不用于 Markdown canonical artifact 主编辑路径
- 文件不存在时不得用 `upsert_section` 隐式创建任意 Markdown；必须调用 `create_document(path, template_kind)` 创建受控模板。P2 至少实现 `openspec_proposal`、`openspec_spec`、`openspec_design`、`openspec_tasks` 四类模板。

fixture 要求：

- `tests/fixtures/document_ops/section_upsert_input.md`
- `tests/fixtures/document_ops/section_upsert_expected.md`
- `tests/fixtures/document_ops/create_document_openspec_spec_expected.md`

golden 测试必须逐字节比对输出，避免研发实现成简单字符串拼接。

- [ ] **Step 5: 运行单元测试**

Run: `cargo test --test document_ops`
Expected: PASS，文档结构操作、JSON/YAML patch、ast-grep optional probe 都通过

- [ ] **Step 6: 建议提交点**

```bash
git add Cargo.toml src/protocol/document_ops.rs src/cross_cutting/document_ops.rs src/cross_cutting/ast_grep_tool.rs tests/document_ops.rs tests/fixtures
git commit -m "feat: add document operation baseline"
```

### Task 2: 建立三层 validator 基线（canonical / projection / phase1_profile）

**Files:**
- Create: `src/cross_cutting/artifact_validate.rs`
- Modify: `src/protocol/artifacts.rs`
- Test: `tests/artifact_validate.rs`
- Test: `tests/artifact_schema_min_fields.rs`

- [ ] **Step 1: 建立 artifact 类型注册表**

至少覆盖：
- `intake_brief`
- `clarification_record`
- `spec`
- `spec_gate_decision`
- `design`
- `design_review`
- `design_revision_record`
- `readiness_check`
- `plan`
- `dispatch_package`
- `coding_report`
- `testing_report`
- `code_review_report`
- `integration_report`
- `final_review`
- `final_summary`
- `runtime_snapshot`

- [ ] **Step 2: 为 Markdown / JSON 两类 artifact 建立 `canonical_validator` 统一校验入口**

要求：
- `canonical_validator` 只校验 canonical schema 最小字段（如 artifact type、必填字段存在性、字段类型正确性）
- Markdown artifact 返回 canonical 文本验证结果
- JSON artifact 返回结构化字段验证结果
- projection schema 和 `_aria` 校验不在此层处理，分别由 `projection_validator` 和 `phase1_profile_validator` 负责
- 每一类 artifact 都必须有最小正例和缺核心字段负例；不能只做 artifact kind registry

- [ ] **Step 3: 加入 `canonical_validator` 失败路径测试**

至少覆盖：
- 缺必填字段
- artifact type 不匹配
- JSON schema 不合法
- `canonical_validator` 不校验 projection 字段（防止 implementation profile 字段混入 canonical schema）
- `artifact_schema_min_fields` 覆盖 17 类一期产物的 canonical 最小字段正/负例

- [ ] **Step 4: 运行验证**

Run: `cargo test --test artifact_validate --test artifact_schema_min_fields`  
Expected: PASS，validator 可被测试引用

- [ ] **Step 5: 建议提交点**

```bash
git add src/protocol/artifacts.rs src/cross_cutting/artifact_validate.rs tests/artifact_validate.rs tests/artifact_schema_min_fields.rs tests/support
git commit -m "feat: add three-layer validator baseline (canonical/projection/phase1_profile)"
```

### Task 3: 实现 `SpecProjection` / `DesignProjection` / `PlanProjection`

**Files:**
- Create: `src/protocol/projections.rs`
- Create: `src/cross_cutting/artifact_projection.rs`
- Test: `tests/spec_projection.rs`
- Test: `tests/design_projection.rs`
- Test: `tests/plan_projection.rs`
- Create: `tests/fixtures/artifacts/spec.md`
- Create: `tests/fixtures/artifacts/design.md`
- Create: `tests/fixtures/artifacts/plan.md`

- [ ] **Step 1: 写 3 组失败测试，覆盖 projection compiler 与 `projection_validator`**

分别覆盖：
- `SpecProjection`
- `DesignProjection`
- `PlanProjection`

断言：
- 稳定 ID 生成
- 结构化 payload 生成
- source artifact hash 被记录
- `projection_validator` 校验 projection schema 和 golden JSON 对齐
- `projection_validator` 不校验 canonical 最小字段（该职责属于 `canonical_validator`）
- `projection_validator` 输入为 `ArtifactProjectionRecord`、artifact index 和可选 golden JSON fixture
- `projection_validator` 输出 `ValidationResult { ok, errors[], warnings[], projection_id, source_artifact_ref }`
- 错误码至少覆盖 `projection_missing_field`、`projection_invalid_id`、`projection_source_not_found`、`projection_source_hash_mismatch`、`projection_reference_unknown`、`projection_payload_empty`
- 通用校验必须覆盖 `projection_id` 格式、`projection_kind` 与 payload kind 匹配、`source_artifact_ref` 存在且 active、`source_artifact_version` 与当前版本一致、`source_artifact_hash` 与文件 hash 一致、`compiled_at/compiled_by_node` 必填
- `SpecProjection` 校验 `functional_requirements[]`、`success_criteria[]` 非空，且 `related_requirement_ids[]` 指向已知 requirement
- `DesignProjection` 校验 `design_decisions[]` 非空，且 design decision 关联的 requirement 可在当前 `SpecProjection` 或 bundle 中找到
- `PlanProjection` 校验 `work_packages[]` 非空、work package ID 唯一、dependency 两端均存在、`traceability_refs[]` 至少包含 requirement 或 design/task 之一
- `PlanProjection.work_packages[].work_package_id` 使用独立 `WorkPackageId`；不得用 `WorkTaskId` 类型替代。`WorkTaskId` 只在 `dispatch_package._aria.worktask_routing[]` 生成后出现。

artifact index 最小结构：

```json
{
  "task_id": "task_001",
  "artifact_refs": [],
  "latest_by_kind": {},
  "superseded_refs": [],
  "projection_refs": []
}
```

落盘路径固定为 `.aria/runtime/tasks/<task_id>/indexes/artifact_index.json`；全局 `.aria/runtime/indexes/artifact_index.json` 只能作为汇总缓存，不能替代 task 局部 index。`projection_validator` 只读取 task 局部 index。

- [ ] **Step 2: 实现 projection record 与 payload 结构**

必须包含：
- `projection_id`
- `projection_kind`
- `source_artifact_ref`
- `source_artifact_version`
- `source_artifact_hash`
- `compiled_at`
- `compiled_by_node`
- `payload`

- [ ] **Step 3: 实现 3 个 compiler**

要求：
- `spec -> SpecProjection`
- `design -> DesignProjection`
- `plan -> PlanProjection`
- compiler 必须消费 `document_ops.extract_projection_source` 输出，不直接自行解析 Markdown 原文

- [ ] **Step 4: 运行单元测试**

Run: `cargo test --test spec_projection --test design_projection --test plan_projection`  
Expected: PASS，三个 projection 都可稳定编译，且通过 `projection_validator` 校验

- [ ] **Step 5: 建议提交点**

```bash
git add src/protocol/projections.rs src/cross_cutting/artifact_projection.rs tests/fixtures tests/spec_projection.rs tests/design_projection.rs tests/plan_projection.rs tests/support
git commit -m "feat: add artifact projection compilers"
```

### Task 4: 实现 `phase1_profile_validator` 与 JSON `_aria` 校验

**Files:**
- Create: `src/protocol/phase1_profile.rs`
- Modify: `src/protocol/projections.rs`
- Test: `tests/phase1_profile.rs`
- Test: `tests/traceability_binding.rs`

测试职责裁定：`tests/phase1_profile.rs` 只覆盖 `_aria` 字段、projection refs、constraint refs 与 artifact profile 校验；`tests/traceability_binding.rs` 只覆盖第 10 章 traceability 归一化、conflict log 与 `ArtifactTraceabilityBinding` 生成。

- [ ] **Step 1: 建立 `_aria` 通用字段结构与 `phase1_profile_validator` 校验规则**

`phase1_profile_validator` 职责：
- 校验 `_aria` 扩展字段（不校验 canonical 最小字段，该职责属于 `canonical_validator`）
- 校验 traceability binding 引用完整性
- 校验 projection refs 和 constraint refs 的关联关系
- 输入为 JSON artifact value、artifact kind、projection index、constraint bundle index、traceability binding index、provider run index
- 输出 `ValidationResult { ok, errors[], warnings[], artifact_ref, profile_version }`
- 错误码至少覆盖 `profile_missing_aria`、`profile_version_missing`、`profile_projection_ref_unknown`、`profile_constraint_ref_unknown`、`traceability_refs_missing`、`traceability_ref_unknown`、`worktask_routing_source_unknown`、`coverage_summary_missing`

必须包含：
- `profile_version`
- `constraint_check_ref`
- `traceability_refs`
- `provider_run_refs`
- `projection_refs`

通用规则：
- `_aria.profile_version` 一期固定 `phase1.v1`
- `_aria.provider_run_refs[]` 若存在必须指向已落盘 `ProviderRunRecord`
- `_aria.projection_refs[]` 必须指向存在且 source artifact 未 superseded 的 projection
- `_aria.constraint_check_ref` 必须指向本次消费的 bundle check record
- report 类 artifact 的 `_aria.traceability_refs[]` 必须由 daemon 归一化生成，provider 自报只能作为候选输入

`ConstraintCheckRecord` 最小实现：

```json
{
  "constraint_check_id": "chk_001",
  "bundle_ref": "bundle_001",
  "artifact_ref": "ref_spec_0001",
  "node_id": "N06",
  "checked_at": "2026-04-26T00:00:00Z",
  "result": "pass",
  "errors": [],
  "warnings": []
}
```

生成时机：
- 每个消费 OpenSpec bundle 的节点在收口前写入一条 check record。
- `_aria.constraint_check_ref` 只能指向同 task 下 `constraints/checks/<constraint_check_id>.json` 中存在且 `result != fail` 的记录。
- 若 bundle stale，不写 pass 记录，先触发重编译。

- [ ] **Step 2: 定义 `dispatch_package._aria.worktask_routing[]`**

必须包含：
- `worktask_id`
- `source_work_package_id`
- `execution_mode`
- `human_required_reason`
- `allowed_write_scope`
- `traceability_refs`
- `verification_commands`

- [ ] **Step 3: 定义 `final_review._aria.coverage_summary`**

至少包含：
- covered items
- uncovered items
- manual exemptions

校验要求：
- `dispatch_package._aria.worktask_routing[]` 每项的 `source_work_package_id` 必须能映射到 `PlanProjection.work_packages[]`
- report 类 JSON artifact 没有 `_aria.traceability_refs` 时必须失败
- `final_review._aria.coverage_summary` 必须覆盖 closed / uncovered / exempted 三类集合
- `final_summary` 不得引入 `final_review` 中不存在的新 coverage 结论

- [ ] **Step 4: 运行验证**

Run: `cargo test --test phase1_profile --test traceability_binding`
Expected: PASS，`phase1_profile_validator` 可正确校验 `_aria` 字段，profile 类型可被 traceability 逻辑消费

- [ ] **Step 5: 建议提交点**

```bash
git add src/protocol/phase1_profile.rs tests/phase1_profile.rs tests/traceability_binding.rs
git commit -m "feat: add phase1 profile validator and aria extension models"
```

### Task 5: 实现 OpenSpec bootstrap、bundle schema 与 constraint compiler

**Files:**
- Create: `src/protocol/constraints.rs`
- Create: `src/cross_cutting/openspec_constraints.rs`
- Test: `tests/openspec_bundle.rs`
- Test: `tests/openspec_bundle_schema.rs`
- Create: `tests/fixtures/openspec/changes/sample-change/proposal.md`
- Create: `tests/fixtures/openspec/changes/sample-change/specs/main/spec.md`
- Create: `tests/fixtures/openspec/changes/sample-change/design.md`
- Create: `tests/fixtures/openspec/changes/sample-change/tasks.md`

- [ ] **Step 1: 写失败测试，覆盖 bundle schema、bootstrap 与 stale 判定**

断言：
- `change_id` 绑定
- source manifest
- `bundle_status`
- hash 变化后 `stale`
- 一期不启用文件系统 watch；stale 检测只在依赖 OpenSpec 的节点进入前通过 source manifest hash 比对触发
- provider run 结束归一化时必须再次做 constraint check；若发现 bundle 已 stale，阻断推进并要求重编译
- OpenSpec 写回与 bundle recompile 是原子操作；测试必须覆盖 recompile 失败时回滚 Markdown 写回并保留旧 bundle active
- skeleton 文件存在但关键 section 为空时不能返回 ready
- `compiled_from_projection_refs` 在纯 OpenSpec 编译时为空；由 projection 写回 OpenSpec 后重编译时记录对应 projection refs
- 序列化 JSON 顶层字段固定使用 `proposal_constraints`、`requirement_constraints`、`design_constraints`、`task_constraints`、`traceability_requirements`、`coverage_model`
- `scope_constraints`、`requirement_ids`、`task_ids`、`traceability_map` 只能作为内部 helper 输出或 payload 子字段，不能替代 bundle 顶层字段名

- [ ] **Step 2: 实现 OpenSpec skeleton bootstrap**

要求：
- 读取 P1 task runtime state 中已固化的 `change_id`
- 若 `openspec/changes/<change_id>/` 不存在，通过 `document_ops.create_document(path, template_kind)` 创建最小 skeleton；已有文件的章节更新才使用 `upsert_section`
- 一期固定 `openspec_scope = "main"`，不新增 `requested_scope` wire 字段
- 最小 skeleton 包含 `proposal.md`、`specs/main/spec.md`、`design.md`、`tasks.md`
- 若已有 change 目录下存在多个 `specs/*/spec.md`，返回 `openspec_multiple_scopes_unsupported` 并进入 gate 或 manual intervention，不自动合并
- fixture 路径固定为 `tests/fixtures/openspec/changes/sample-change/specs/main/spec.md`
- bootstrap 完成后将 task runtime state 中 `openspec_bootstrap_status` 从 `bootstrap_pending` 更新为 `bootstrapped`

- [ ] **Step 3: 实现 OpenSpec file manifest**

必须记录：
- path
- kind
- sha256

- [ ] **Step 4: 实现 bundle compiler**

必须生成：
- `proposal_constraints`
- `requirement_constraints`
- `design_constraints`
- `task_constraints`
- `traceability_requirements`
- `coverage_model`

要求：
- OpenSpec Markdown 文件读取必须走 `document_ops.read_document_model`
- OpenSpec bootstrap 必须走 `document_ops.create_document(path, template_kind)`；已有 OpenSpec 文件的章节更新必须走 `document_ops.upsert_section` / structured writer
- `OpenSpecConstraintBundle` Rust 类型、JSON schema 与 fixture golden JSON 必须共享同一套字段名
- `TaskConstraints` 必须拆分 requirement/design/acceptance 三类映射：`related_requirement_ids_by_task`、`related_design_decision_ids_by_task`、`acceptance_target_ids_by_task`
- `compiled_from_projection_refs` 按补齐规格规则填充：纯读取 OpenSpec 文件时为空；由 Aria projection 写回 OpenSpec 后触发重编译时记录对应 projection refs
- artifact version 规则按补齐规格第 8.3 章实现：同一 `ArtifactKind + TaskId + 逻辑产物槽位` 更新时沿用 `artifact_id` 并递增 version；全新逻辑产物使用新 `artifact_id` 且 version 从 1 开始

- [ ] **Step 5: 加入缺文件与内容未就绪回流判定**

要求：
- `proposal.md` 缺失时阻断 `N05`
- `spec.md` 缺失时阻断 `N07`
- `design.md` 缺失时阻断 `N11`
- `tasks.md` 缺失时阻断 `N12/N16`
- `specs/main/spec.md` 存在但无 requirement id 时阻断 `N07`
- `design.md` 存在但无 design decision id 且无 component id 时阻断 `N11`
- `tasks.md` 存在但无 task id 时阻断 `N12/N16`

- [ ] **Step 6: 运行验证**

Run: `cargo test --test openspec_bundle --test openspec_bundle_schema`  
Expected: PASS，bundle schema、bootstrap、编译与 stale 路径通过

- [ ] **Step 7: 建议提交点**

```bash
git add src/protocol/constraints.rs src/cross_cutting/openspec_constraints.rs tests/openspec_bundle.rs tests/openspec_bundle_schema.rs tests/fixtures/openspec
git commit -m "feat: add openspec bundle compiler and stale detection"
```

### Task 6: 实现 traceability binding 与 coverage checker

**Files:**
- Create: `src/protocol/traceability.rs`
- Create: `src/cross_cutting/traceability.rs`
- Test: `tests/traceability_binding.rs`

- [ ] **Step 1: 写失败测试，覆盖 binding 生成**

断言：
- requirement IDs
- design decision IDs
- task IDs
- risk IDs

- [ ] **Step 2: 实现 `ArtifactTraceabilityBinding`**

要求：
- 支持 projection 输入
- 支持 JSON report `_aria.traceability_refs`
- 支持冲突记录

- [ ] **Step 3: 实现 coverage checker**

至少支持：
- closed
- uncovered
- exempted

- [ ] **Step 4: 运行验证**

Run: `cargo test --test traceability_binding`
Expected: PASS，binding 与 coverage checker 可稳定输出

- [ ] **Step 5: 建议提交点**

```bash
git add src/protocol/traceability.rs src/cross_cutting/traceability.rs tests/traceability_binding.rs
git commit -m "feat: add traceability binding and coverage checker"
```

### Task 7: 实现 superseded artifact refs 验证

**Files:**
- Modify: `src/cross_cutting/artifact_validate.rs`
- Test: `tests/superseded_artifact_refs.rs`

- [ ] **Step 1: 写失败测试，覆盖回流后旧产物失效**

  断言：
  - 回流后旧 artifact 被写入 `superseded_artifact_refs`
  - 节点进入前不得引用 superseded 产物作为输入
  - `ArtifactRef` 被标记为 `superseded` 后不可再被 canonical artifact validator 接受为有效输入

- [ ] **Step 2: 在 artifact validator 中接入 superseded 判定**

  要求：
  - validator 校验输入 artifact 时检查其是否存在于 `superseded_artifact_refs`
  - 若引用 superseded artifact，返回 `invalid_input_superseded` 错误
  - 回流操作必须同时更新 task runtime state 的 `superseded_artifact_refs`

- [ ] **Step 3: 运行验证**

  Run: `cargo test --test superseded_artifact_refs`
  Expected: PASS，回流后旧产物不可作为输入

- [ ] **Step 4: 建议提交点**

  ```bash
  git add src/cross_cutting/artifact_validate.rs tests/superseded_artifact_refs.rs
  git commit -m "feat: add superseded artifact ref validation"
  ```

---

## 4. P2 完成判定

- [ ] `cargo test --test artifact_validate` 通过，且 canonical validator 只校验 canonical 最小字段
- [ ] `cargo test --test artifact_schema_min_fields` 通过，17 类一期产物均有 canonical 最小字段正/负例
- [ ] `cargo test --test spec_projection --test design_projection --test plan_projection` 通过，且 compiler 输出与 `tests/fixtures/artifacts/golden/*.json` 逐项对齐
- [ ] `cargo test --test document_ops` 通过
- [ ] `cargo test --test openspec_bundle --test openspec_bundle_schema` 通过
- [ ] `cargo test --test traceability_binding` 通过
- [ ] `cargo test --test superseded_artifact_refs` 通过，回流后旧产物不可再作为输入
- [ ] `spec/design/plan` 可稳定编译 projection，projection payload 与 golden JSON 对齐
- [ ] JSON artifact 的 `_aria` 结构已定稿
- [ ] OpenSpec skeleton bootstrap、bundle schema 与 traceability binding 可落盘
- [ ] OpenSpec bundle readiness 测试覆盖“文件存在但关键约束为空”的阻断路径
- [ ] `TaskConstraints` 拆分 requirement/design/acceptance 映射，fixture 同时覆盖三类 ID
- [ ] `compiled_from_projection_refs` 填充规则有测试覆盖
- [ ] 17 类一期产物（16 类业务产物 + `runtime_snapshot`）全部进入 validator 注册表
- [ ] `canonical_validator` 只覆盖 canonical schema 最小字段；`projection_validator` 校验 `SpecProjection/DesignProjection/PlanProjection` schema 和 golden JSON；`phase1_profile_validator` 校验 `_aria`、traceability、projection refs、constraint refs。fixture 同时通过三种校验
- [ ] 协议不漂移检查：P2 实现字段、projection payload、OpenSpec bundle schema、fixture golden JSON 与 `实现总契约_v1.0`、`评审后实施规格补齐_v1.4` 一致，顶层序列化字段固定使用 snake_case
