# Aria Phase 1 P2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建立 canonical artifact 校验、projection 编译、phase1 profile、OpenSpec constraint bundle、traceability binding 这条数据面基础层。

**Architecture:** P2 不碰 provider 调用和节点业务，专注于“产物如何被机器正确消费”。完成后，planning/execution/final closure 都只能消费 canonical artifact、projection、`_aria`、bundle 与 binding，而不能自己解析 Markdown 或自由拼装 JSON。

**Tech Stack:** Rust、serde、Markdown parser、JSON、fixture-based tests。

---

## 1. 范围与出口

P2 完成后，必须满足：

1. canonical artifact 有统一 validator
2. `spec/design/plan` 可以编译出 projection
3. JSON artifact 支持 `_aria` 扩展与 profile validator
4. OpenSpec change 可以编译出 `OpenSpecConstraintBundle`
5. traceability binding 可以自动生成

---

## 2. 目标文件结构

**Files:**
- Create: `src/protocol/phase1_profile.rs`
- Create: `src/protocol/projections.rs`
- Create: `src/protocol/constraints.rs`
- Create: `src/protocol/traceability.rs`
- Create: `src/cross_cutting/artifact_validate.rs`
- Create: `src/cross_cutting/artifact_projection.rs`
- Create: `src/cross_cutting/openspec_constraints.rs`
- Create: `src/cross_cutting/traceability.rs`
- Create: `tests/unit/projections/spec_projection.rs`
- Create: `tests/unit/projections/design_projection.rs`
- Create: `tests/unit/projections/plan_projection.rs`
- Create: `tests/unit/constraints/openspec_bundle.rs`
- Create: `tests/unit/traceability/binding.rs`
- Create: `tests/fixtures/artifacts/spec.md`
- Create: `tests/fixtures/artifacts/design.md`
- Create: `tests/fixtures/artifacts/plan.md`
- Create: `tests/fixtures/openspec/changes/sample-change/proposal.md`
- Create: `tests/fixtures/openspec/changes/sample-change/specs/sample/spec.md`
- Create: `tests/fixtures/openspec/changes/sample-change/design.md`
- Create: `tests/fixtures/openspec/changes/sample-change/tasks.md`

---

## 3. 任务拆解

### Task 1: 建立 canonical artifact validator 基线

**Files:**
- Create: `src/cross_cutting/artifact_validate.rs`
- Create: `src/protocol/artifacts.rs`
- Test: `tests/unit/projections/spec_projection.rs`

- [ ] **Step 1: 建立 artifact 类型注册表**

至少覆盖：
- `spec`
- `design`
- `plan`
- `dispatch_package`
- `coding_report`
- `testing_report`
- `code_review_report`
- `integration_report`
- `final_review`

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

### Task 2: 实现 `SpecProjection` / `DesignProjection` / `PlanProjection`

**Files:**
- Create: `src/protocol/projections.rs`
- Create: `src/cross_cutting/artifact_projection.rs`
- Test: `tests/unit/projections/spec_projection.rs`
- Test: `tests/unit/projections/design_projection.rs`
- Test: `tests/unit/projections/plan_projection.rs`
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
- `payload`

- [ ] **Step 3: 实现 3 个 compiler**

要求：
- `spec -> SpecProjection`
- `design -> DesignProjection`
- `plan -> PlanProjection`

- [ ] **Step 4: 运行单元测试**

Run: `cargo test --test spec_projection --test design_projection --test plan_projection`  
Expected: PASS，三个 projection 都可稳定编译

- [ ] **Step 5: 提交阶段性变更**

```bash
git add src/protocol/projections.rs src/cross_cutting/artifact_projection.rs tests/fixtures tests/unit/projections
git commit -m "feat: add artifact projection compilers"
```

### Task 3: 实现 phase1 profile 与 JSON `_aria` 校验

**Files:**
- Create: `src/protocol/phase1_profile.rs`
- Modify: `src/protocol/projections.rs`
- Test: `tests/unit/traceability/binding.rs`

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

Run: `cargo test --test binding`  
Expected: PASS，profile 类型可被 traceability 逻辑消费

- [ ] **Step 5: 提交阶段性变更**

```bash
git add src/protocol/phase1_profile.rs tests/unit/traceability/binding.rs
git commit -m "feat: add phase1 profile and aria extension models"
```

### Task 4: 实现 OpenSpec bootstrap 与 constraint bundle compiler

**Files:**
- Create: `src/protocol/constraints.rs`
- Create: `src/cross_cutting/openspec_constraints.rs`
- Test: `tests/unit/constraints/openspec_bundle.rs`
- Create: `tests/fixtures/openspec/changes/sample-change/proposal.md`
- Create: `tests/fixtures/openspec/changes/sample-change/specs/sample/spec.md`
- Create: `tests/fixtures/openspec/changes/sample-change/design.md`
- Create: `tests/fixtures/openspec/changes/sample-change/tasks.md`

- [ ] **Step 1: 写失败测试，覆盖 bundle 编译与 stale 判定**

断言：
- `changeId` 绑定
- source manifest
- `bundleStatus`
- hash 变化后 `stale`

- [ ] **Step 2: 实现 OpenSpec file manifest**

必须记录：
- path
- kind
- sha256

- [ ] **Step 3: 实现 bundle compiler**

必须生成：
- `proposalConstraints`
- `requirementConstraints`
- `designConstraints`
- `taskConstraints`
- `traceabilityRequirements`
- `coverageModel`

- [ ] **Step 4: 加入缺文件回流判定**

要求：
- `proposal.md` 缺失时阻断 `N05`
- `spec.md` 缺失时阻断 `N07`
- `design.md` 缺失时阻断 `N11`
- `tasks.md` 缺失时阻断 `N12/N16`

- [ ] **Step 5: 运行验证**

Run: `cargo test --test openspec_bundle`  
Expected: PASS，bundle 编译与 stale 路径通过

- [ ] **Step 6: 提交阶段性变更**

```bash
git add src/protocol/constraints.rs src/cross_cutting/openspec_constraints.rs tests/unit/constraints tests/fixtures/openspec
git commit -m "feat: add openspec bundle compiler and stale detection"
```

### Task 5: 实现 traceability binding 与 coverage checker

**Files:**
- Create: `src/protocol/traceability.rs`
- Create: `src/cross_cutting/traceability.rs`
- Test: `tests/unit/traceability/binding.rs`

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

Run: `cargo test --test binding`  
Expected: PASS，binding 与 coverage checker 可稳定输出

- [ ] **Step 5: 提交阶段性变更**

```bash
git add src/protocol/traceability.rs src/cross_cutting/traceability.rs tests/unit/traceability/binding.rs
git commit -m "feat: add traceability binding and coverage checker"
```

---

## 4. P2 完成判定

- [ ] `cargo test --test spec_projection --test design_projection --test plan_projection` 通过
- [ ] `cargo test --test openspec_bundle` 通过
- [ ] `cargo test --test binding` 通过
- [ ] `spec/design/plan` 可稳定编译 projection
- [ ] JSON artifact 的 `_aria` 结构已定稿
- [ ] OpenSpec bundle 与 traceability binding 可落盘
