# Aria Phase 1 P2 Implementation Plan

**文档信息**
- **创建日期**：2026-04-24
- **版本**：v1.1（评审后修正版）
- **修正内容**：目标文件结构补充 golden projection fixture；完成判定增加 canonical validator 与 golden JSON 对齐要求。

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建立 Document Operation、canonical artifact 校验、projection 编译、phase1 profile、OpenSpec constraint bundle、traceability binding 这条数据面基础层。

**Architecture:** P2 不碰 provider 调用和节点业务，专注于“产物如何被机器正确消费和结构化修改”。完成后，planning/execution/final closure 都只能通过 Document Operation、canonical artifact、projection、`_aria`、bundle 与 binding 读写产物，而不能自己解析 Markdown、自由拼装 JSON 或对文档做裸字符串替换。

**Tech Stack:** Rust、serde、Markdown parser、JSON/YAML parser、fixture-based tests、可选 ast-grep capability probe。

---

## 0. 评审后准入门槛

P2 是评审中 P0 缺口最集中的阶段。启动 P2 前，必须先落实 `cadence/designs/2026-04-24_技术方案_Aria一期评审后实施规格补齐_v1.2.md`：

- 第 4.5：`ArtifactProjectionRecord`、`SpecProjection`、`DesignProjection`、`PlanProjection` Rust 类型
- 第 4.6：`OpenSpecConstraintBundle` Rust 类型与 camelCase JSON 字段
- 第 5 章：Projection 编译规则、heading mapping、稳定 ID 生成、Markdown parser 裁定
- 第 6 章：OpenSpec 文件到 bundle 字段的映射、缺文件阻断、stale 判定
- 第 8 章：artifact 存储路径、版本号策略、ExternalArtifactRef 生命周期
- 第 10 章：`_aria.traceability_refs` 生成算法
- 第 15 章：fixture 树、最小输入样例与 golden JSON

特别裁定：Projection compiler、OpenSpec bundle compiler 和 fixture golden test 三者必须一起落地；不得只先写空 compiler 或只用 Markdown 原文裸测。

---

## 1. 范围与出口

P2 完成后，必须满足：

1. 17 类一期产物（16 类业务产物 + `runtime_snapshot`）有统一 validator
2. Markdown / JSON / YAML 文档修改统一走 Document Operation 层
3. `spec/design/plan` 可以编译出 projection
4. JSON artifact 支持 `_aria` 扩展与 profile validator
5. P1 固化的 `changeId` 可以完成 OpenSpec skeleton bootstrap，并编译出字段名稳定的 `OpenSpecConstraintBundle`
6. traceability binding 可以自动生成

---

## 2. 目标文件结构

**Files:**
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
- Create: `tests/fixtures/openspec/changes/sample-change/specs/sample/spec.md`
- Create: `tests/fixtures/openspec/changes/sample-change/design.md`
- Create: `tests/fixtures/openspec/changes/sample-change/tasks.md`

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

- [ ] **Step 1: 写失败测试，覆盖 Markdown 章节级 upsert**

断言：
- 可按 heading path 找到 `## 目标与范围`
- `upsert_section` 只替换目标章节，不改变其他章节顺序
- 写入后返回新的 sha256

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

- [ ] **Step 5: 运行单元测试**

Run: `cargo test --test document_ops`
Expected: PASS，文档结构操作、JSON/YAML patch、ast-grep optional probe 都通过

- [ ] **Step 6: 提交阶段性变更**

```bash
git add src/protocol/document_ops.rs src/cross_cutting/document_ops.rs src/cross_cutting/ast_grep_tool.rs tests/document_ops.rs tests/fixtures
git commit -m "feat: add document operation baseline"
```

### Task 2: 建立 canonical artifact validator 基线

**Files:**
- Create: `src/cross_cutting/artifact_validate.rs`
- Create: `src/protocol/artifacts.rs`
- Test: `tests/spec_projection.rs`

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

- [ ] **Step 2: 为 Markdown / JSON 两类 artifact 建立统一校验入口**

要求：
- Markdown artifact 返回 canonical 文本验证结果
- JSON artifact 返回结构化字段验证结果

- [ ] **Step 3: 加入失败路径测试**

至少覆盖：
- 缺必填字段
- artifact type 不匹配
- JSON schema 不合法

- [ ] **Step 4: 运行验证**

Run: `cargo test --test spec_projection`  
Expected: PASS，validator 可被测试引用

- [ ] **Step 5: 提交阶段性变更**

```bash
git add src/protocol/artifacts.rs src/cross_cutting/artifact_validate.rs tests
git commit -m "feat: add canonical artifact validator baseline"
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

- [ ] **Step 1: 写 3 组失败测试**

分别覆盖：
- `SpecProjection`
- `DesignProjection`
- `PlanProjection`

断言：
- 稳定 ID 生成
- 结构化 payload 生成
- source artifact hash 被记录

- [ ] **Step 2: 实现 projection record 与 payload 结构**

必须包含：
- `projectionId`
- `projectionKind`
- `sourceArtifactRef`
- `sourceArtifactVersion`
- `sourceArtifactHash`
- `compiledAt`
- `compiledByNode`
- `payload`

- [ ] **Step 3: 实现 3 个 compiler**

要求：
- `spec -> SpecProjection`
- `design -> DesignProjection`
- `plan -> PlanProjection`
- compiler 必须消费 `document_ops.extract_projection_source` 输出，不直接自行解析 Markdown 原文

- [ ] **Step 4: 运行单元测试**

Run: `cargo test --test spec_projection --test design_projection --test plan_projection`  
Expected: PASS，三个 projection 都可稳定编译

- [ ] **Step 5: 提交阶段性变更**

```bash
git add src/protocol/projections.rs src/cross_cutting/artifact_projection.rs tests/fixtures tests/spec_projection.rs tests/design_projection.rs tests/plan_projection.rs tests/support
git commit -m "feat: add artifact projection compilers"
```

### Task 4: 实现 phase1 profile 与 JSON `_aria` 校验

**Files:**
- Create: `src/protocol/phase1_profile.rs`
- Modify: `src/protocol/projections.rs`
- Test: `tests/traceability_binding.rs`

- [ ] **Step 1: 建立 `_aria` 通用字段结构**

必须包含：
- `profile_version`
- `constraint_check_ref`
- `traceability_refs`
- `provider_run_refs`
- `projection_refs`

- [ ] **Step 2: 定义 `dispatch_package._aria.worktask_routing[]`**

必须包含：
- `worktask_id`
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

- [ ] **Step 4: 运行验证**

Run: `cargo test --test traceability_binding`
Expected: PASS，profile 类型可被 traceability 逻辑消费

- [ ] **Step 5: 提交阶段性变更**

```bash
git add src/protocol/phase1_profile.rs tests/traceability_binding.rs
git commit -m "feat: add phase1 profile and aria extension models"
```

### Task 5: 实现 OpenSpec bootstrap、bundle schema 与 constraint compiler

**Files:**
- Create: `src/protocol/constraints.rs`
- Create: `src/cross_cutting/openspec_constraints.rs`
- Test: `tests/openspec_bundle.rs`
- Test: `tests/openspec_bundle_schema.rs`
- Create: `tests/fixtures/openspec/changes/sample-change/proposal.md`
- Create: `tests/fixtures/openspec/changes/sample-change/specs/sample/spec.md`
- Create: `tests/fixtures/openspec/changes/sample-change/design.md`
- Create: `tests/fixtures/openspec/changes/sample-change/tasks.md`

- [ ] **Step 1: 写失败测试，覆盖 bundle schema、bootstrap 与 stale 判定**

断言：
- `changeId` 绑定
- source manifest
- `bundleStatus`
- hash 变化后 `stale`
- 序列化 JSON 顶层字段固定使用 `proposalConstraints`、`requirementConstraints`、`designConstraints`、`taskConstraints`、`traceabilityRequirements`、`coverageModel`
- `scope_constraints`、`requirement_ids`、`task_ids`、`traceability_map` 只能作为内部 helper 输出或 payload 子字段，不能替代 bundle 顶层字段名

- [ ] **Step 2: 实现 OpenSpec skeleton bootstrap**

要求：
- 读取 P1 task runtime state 中已固化的 `changeId`
- 若 `openspec/changes/<changeId>/` 不存在，通过 `document_ops.upsert_section` / structured writer 创建最小 skeleton
- 最小 skeleton 包含 `proposal.md`、`specs/<task-scope>/spec.md`、`design.md`、`tasks.md`
- bootstrap 完成后将 task runtime state 中 `openspec_bootstrap_status` 从 `bootstrap_pending` 更新为 `bootstrapped`

- [ ] **Step 3: 实现 OpenSpec file manifest**

必须记录：
- path
- kind
- sha256

- [ ] **Step 4: 实现 bundle compiler**

必须生成：
- `proposalConstraints`
- `requirementConstraints`
- `designConstraints`
- `taskConstraints`
- `traceabilityRequirements`
- `coverageModel`

要求：
- OpenSpec Markdown 文件读取必须走 `document_ops.read_document_model`
- OpenSpec 写入或 bootstrap 必须走 `document_ops.upsert_section` / structured writer
- `OpenSpecConstraintBundle` Rust 类型、JSON schema 与 fixture golden JSON 必须共享同一套字段名

- [ ] **Step 5: 加入缺文件回流判定**

要求：
- `proposal.md` 缺失时阻断 `N05`
- `spec.md` 缺失时阻断 `N07`
- `design.md` 缺失时阻断 `N11`
- `tasks.md` 缺失时阻断 `N12/N16`

- [ ] **Step 6: 运行验证**

Run: `cargo test --test openspec_bundle --test openspec_bundle_schema`  
Expected: PASS，bundle schema、bootstrap、编译与 stale 路径通过

- [ ] **Step 7: 提交阶段性变更**

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

- [ ] **Step 5: 提交阶段性变更**

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
  - 回流后旧 artifact 被写入 `supersededArtifactRefs`
  - 节点进入前不得引用 superseded 产物作为输入
  - `ArtifactRef` 被标记为 `superseded` 后不可再被 canonical artifact validator 接受为有效输入

- [ ] **Step 2: 在 artifact validator 中接入 superseded 判定**

  要求：
  - validator 校验输入 artifact 时检查其是否存在于 `supersededArtifactRefs`
  - 若引用 superseded artifact，返回 `invalid_input_superseded` 错误
  - 回流操作必须同时更新 task runtime state 的 `supersededArtifactRefs`

- [ ] **Step 3: 运行验证**

  Run: `cargo test --test superseded_artifact_refs`
  Expected: PASS，回流后旧产物不可作为输入

- [ ] **Step 4: 提交阶段性变更**

  ```bash
  git add src/cross_cutting/artifact_validate.rs tests/superseded_artifact_refs.rs
  git commit -m "feat: add superseded artifact ref validation"
  ```

---

## 4. P2 完成判定

- [ ] `cargo test --test spec_projection --test design_projection --test plan_projection` 通过，且 compiler 输出与 `tests/fixtures/artifacts/golden/*.json` 逐项对齐
- [ ] `cargo test --test document_ops` 通过
- [ ] `cargo test --test openspec_bundle --test openspec_bundle_schema` 通过
- [ ] `cargo test --test traceability_binding` 通过
- [ ] `cargo test --test superseded_artifact_refs` 通过，回流后旧产物不可再作为输入
- [ ] `spec/design/plan` 可稳定编译 projection，projection payload 与 golden JSON 对齐
- [ ] JSON artifact 的 `_aria` 结构已定稿
- [ ] OpenSpec skeleton bootstrap、bundle schema 与 traceability binding 可落盘
- [ ] 17 类一期产物（16 类业务产物 + `runtime_snapshot`）全部进入 validator 注册表
- [ ] canonical validator 同时覆盖 canonical schema 最小字段和 Projection schema，fixture 同时通过两种校验
- [ ] 协议不漂移检查：P2 实现字段、projection payload、OpenSpec bundle schema、fixture golden JSON 与 `实现总契约_v1.0`、`评审后实施规格补齐_v1.2` 一致，顶层序列化字段固定使用 camelCase
