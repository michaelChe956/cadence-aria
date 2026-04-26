# Aria Phase 1 P3 Implementation Plan

**文档信息**
- **创建日期**：2026-04-26
- **版本**：v1.3（研发可落地性 Review 二次修正版）
- **修正内容**：补充 `ProviderRunRecord` raw artifact 与 `AdapterOutput` 落盘关系、provider adapter baseline 的 ID 类型依赖裁定，并固化 `N06` advisory 最终决策权。

> **自动化 agent 执行提示**：agentic worker 执行本计划时使用 `superpowers:subagent-driven-development` 或 `superpowers:executing-plans`；人类研发按任务、测试命令和完成判定执行即可，不依赖这些 skill。

**Goal:** 建立 BYO 本地 CLI provider contract、prompt/contract registry、provider context builder，并打通 `N04-N12` 规划链。

**Architecture:** 先用 fake provider 跑通 planning chain，再接真实 CLI adapter。Claude Code / Codex 由用户在本机安装、登录和授权，Aria 只做 capability probe、compatibility matrix 与受控 spawn。P3 的重点不是模型效果，而是 Aria 怎样稳定地把 canonical artifact、projection、bundle、discipline、prompt template 组装成 provider 可执行输入，并在 CLI 缺失、未登录或能力不足时给出可诊断失败。

**Tech Stack:** Rust、fake provider adapter、BYO 本地 CLI adapter、JSON、fixture-based integration tests。

---

## 0. 评审后准入门槛

P3 启动前必须先落实 `cadence/designs/2026-04-26_技术方案_Aria一期评审后实施规格补齐_v1.4.md` 中与 provider 和 prompt 相关的裁定：

- 第 4.7：`NodeExecutionContract`、`WorkflowDisciplineSpec`、`NodePromptTemplateRef`、`ProviderContextPackage`、`ProviderRunRecord`
- 第 4.7.3：`AdapterInput` / `AdapterOutput` 运行时 DTO
- 第 4.7.4：`RuntimeUnit` 统一 handler 契约
- 第 9.1-9.2：prompt render order 与 `N04-N12` manifest
- 第 9.3：`N04/N05/N07` 三个完整模板
- 第 9.4：`N06/N08-N12` 可渲染模板骨架与节点差异项
- 第 9.5：Provider structured output sentinel 与 parse error / incompatible output 判定
- 第 15.5：fake provider stdout fixture

P3 可以继续先用 fake provider 跑通规划链，但 fake provider 输出必须使用正式 sentinel 格式，避免后续真实 CLI adapter 重写解析逻辑。P3 的 `ProviderRunRecord` 必须记录 capability、compatibility、adapter input/output、timeout、sandbox、approval、constraint 与 traceability 审计字段，不允许只记录 stdout/stderr 和 exit code。

---

## 1. 范围与出口

P3 完成后，必须满足：

1. `NodeExecutionContract`、`WorkflowDisciplineSpec`、`NodePromptTemplateRef` 有注册表
2. `ProviderContextPackage -> AdapterInput` 映射成立
3. fake provider 与真实 CLI adapter baseline 共用 `AdapterInput` / `AdapterOutput`
4. Claude Code / Codex 按用户本机 CLI 接入，Aria 不内置 provider、不托管远程模型、不自动安装 CLI
5. provider capability / compatibility matrix、provider error code、timeout、sandbox、approval policy 与 traceability refs 能写入 `ProviderRunRecord`
6. prompt template registry 有明确 `template_id` 清单、render order、output instruction，且 `N04-N12` 每个模板都可渲染
7. fake provider 下 `N04-N12` 规划链可跑
8. 可产出：
   - `clarification_record`
   - `spec`
   - `spec_gate_decision`
   - `design`
   - `design_review`
   - `design_revision_record`（含 revise 路径回退到 N08 再评审）
   - `readiness_check`
   - `plan`
   - `dispatch_package`

---

## 2. 目标文件结构

**Files:**
- Create: `src/cross_cutting/provider_adapter.rs`
- Create: `src/cross_cutting/cli_adapter.rs`
- Create: `src/cross_cutting/provider_run.rs`
- Create: `src/cross_cutting/provider_router.rs`
- Create: `src/cross_cutting/provider_context_builder.rs`
- Create: `src/cross_cutting/provider_capabilities.rs`
- Create: `src/cross_cutting/adapter_compatibility.rs`
- Create: `src/protocol/provider_errors.rs`
- Create: `src/runtime_units/clarification.rs`
- Create: `src/runtime_units/spec_authoring.rs`
- Create: `src/runtime_units/spec_gate_review.rs`
- Create: `src/runtime_units/design_authoring.rs`
- Create: `src/runtime_units/design_review.rs`
- Create: `src/runtime_units/design_revision.rs`
- Create: `src/runtime_units/plan_dispatch.rs`
- Create: `src/protocol/contracts.rs`
- Create: `src/protocol/prompt_manifest.rs`
- Create: `src/runtime_units/prompt_template_registry.rs`
- Create: `tests/provider_adapter_baseline.rs`
- Create: `tests/context_builder.rs`
- Create: `tests/cli_adapter_baseline.rs`
- Create: `tests/provider_error_routes.rs`
- Create: `tests/planning_chain_fake_provider.rs`
- Create: `tests/risk_registry_minimal.rs`
- Create: `tests/support/mod.rs`

---

## 3. 任务拆解

### Task 1: 实现 provider adapter 基线与 fake provider

**Files:**
- Create: `src/protocol/contracts.rs`
- Create: `src/cross_cutting/provider_adapter.rs`
- Create: `src/cross_cutting/provider_run.rs`
- Create: `src/cross_cutting/provider_router.rs`
- Test: `tests/provider_adapter_baseline.rs`

- [ ] **Step 1: 定义 `AdapterInput` / `AdapterOutput` 运行时封装**

要求：
- 字段必须与 `评审后实施规格补齐_v1.4` 第 4.7.3 章一致：

```rust
pub struct AdapterInput {
    pub provider_type: ProviderType,
    pub role: AdapterRole,
    pub worktree_path: Option<String>,
    pub prompt: String,
    pub context_files: Vec<String>,
    pub output_schema: String,
    pub timeout: u64,
    pub max_retries: u32,
}

pub struct AdapterOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub structured_output: Option<Value>,
    pub files_modified: Vec<String>,
    pub duration_ms: u64,
    pub timeout_status: TimeoutStatus,
}
```

- `AdapterInput` 由 `ProviderContextPackage` 映射得到，禁止 fake provider 或 CLI adapter 各自发明额外强字段
- `AdapterOutput` 必须能记录 raw stdout/stderr、structured output、files modified、duration、timeout status
- fake provider 和 CLI adapter 后续必须共用同一组 `AdapterInput` / `AdapterOutput` 类型
- DTO 唯一落位为 `src/protocol/contracts.rs`；`provider_adapter.rs`、`cli_adapter.rs`、fake provider 和 context builder 只能引用该类型，不得各自定义同名结构。

- [ ] **Step 2: 实现 fake provider adapter**

用途：
- 为 planning chain 提供稳定、可预测输出
- 不依赖真实 Claude / Codex CLI

- [ ] **Step 3: 实现 provider run record 落盘**

必须记录：
- provider run id
- node id
- runtime role / adapter role
- context package ref
- provider capability ref
- adapter compatibility ref
- adapter input ref
- adapter output ref
- stdout ref / stderr ref / structured output ref
- raw artifact refs
- files modified
- duration
- timeout / retry
- provider error code / error details

`ProviderRunRecord` 与 `AdapterOutput` 关系：
- `AdapterOutput.stdout` / `stderr` / `structured_output` 是调用完成后的内联解析输入
- adapter 或 daemon 必须将 stdout / stderr / structured output 物化到 `.aria/runtime/tasks/<task_id>/provider-runs/<provider_run_id>/`
- `stdout_ref`、`stderr_ref`、`structured_output_ref` 指向物化后的 artifact ref
- `raw_artifact_refs` 是上述 refs 加 provider 其他中间文件 refs 的有序列表，不包含 worktree 源码改动路径
- `files_modified` 来自 worktree diff 检测，独立于 `raw_artifact_refs`

- [ ] **Step 4: 运行单元验证**

Run: `cargo test --test provider_adapter_baseline`
Expected: PASS，`AdapterInput` / `AdapterOutput`、fake provider、`ProviderRunRecord` baseline 可独立验证，不依赖后续 `provider_context_builder.rs`。`ContextPackageId`、`AdapterInputRefId`、`AdapterOutputRefId` 等 ID 类型必须来自 `评审后实施规格补齐_v1.4` 第 4.1 章的基础类型定义，不得由 adapter 层重复定义。

- [ ] **Step 5: 建议提交点**

```bash
git add src/cross_cutting/provider_adapter.rs src/cross_cutting/provider_run.rs src/cross_cutting/provider_router.rs tests/provider_adapter_baseline.rs tests/support
git commit -m "feat: add provider adapter baseline and fake provider"
```

### Task 2: 实现真实 CLI adapter baseline

**Files:**
- Create: `src/cross_cutting/cli_adapter.rs`
- Create: `src/cross_cutting/provider_capabilities.rs`
- Create: `src/cross_cutting/adapter_compatibility.rs`
- Create: `src/protocol/provider_errors.rs`
- Modify: `src/cross_cutting/provider_run.rs`
- Test: `tests/cli_adapter_baseline.rs`
- Test: `tests/provider_error_routes.rs`
- Test support: `tests/support/mod.rs`

- [ ] **Step 1: 写失败测试，覆盖 capability probe 与 compatibility matrix**

断言：
- fixture provider command 可被探测
- 探测结果生成 `provider_capability_ref`
- Claude Code / Codex 都有默认 compatibility matrix entry
- provider command 不存在时返回可诊断错误，不 panic
- provider command 存在但返回 unauthorized / permission denied 时返回可诊断错误，不进入节点收口
- provider parse error、timeout、incompatible output mode 都映射为稳定错误码
- CLI adapter baseline 测试使用 fixture command，不依赖研发机器已安装 Claude Code / Codex

- [ ] **Step 2: 实现 provider capability probe**

必须记录：
- command path
- version string 或 `unknown`
- supported output modes
- supports session / resume 的探测结果
- probe timestamp
- install source 标记为 `user_local_cli`，不得在 probe 中自动安装或下载 provider

- [ ] **Step 3: 实现 adapter compatibility matrix**

必须覆盖：
- `claude_code` 的 `probe_command`、`version_command`、`auth_check_command`、prompt input mode、output parser、session flag、permission mode、`unauthorized_patterns[]`、`permission_denied_patterns[]`、`structured_output_mode` 字段
- `codex` 的 `probe_command`、`version_command`、`auth_check_command`、prompt input mode、output parser、resume flag、sandbox、approval policy、`unauthorized_patterns[]`、`permission_denied_patterns[]`、`structured_output_mode` 字段
- matrix version，并写入 provider run record

- [ ] **Step 4: 实现 provider error registry**

必须包含：
- `provider_command_missing`
- `provider_unauthorized`
- `provider_permission_denied`
- `provider_incompatible_output`
- `provider_timeout`
- `provider_parse_error`
- `provider_execution_failed`

路由要求：
- `provider_command_missing`、`provider_unauthorized`、`provider_permission_denied` 默认进入 gate 或 manual intervention，不自动重试
- `provider_timeout` 可按 `max_retries` 重试，超过阈值后进入 manual intervention
- `provider_parse_error` 可重试一次；再次失败进入 gate，要求用户或开发者修正 provider 输出配置
- 所有 provider 错误必须写入 `ProviderRunRecord` 和 `provider_run.failed` event payload

- [ ] **Step 5: 实现 CLI spawn adapter**

要求：
- 在指定 `worktree_path` 下启动进程
- 捕获 stdout、stderr、exit code、duration
- 解析 JSON structured output；失败时写 `parse_error`
- 运行前后检测 worktree diff，写入 `files_modified`
- 按 `timeout` 执行 soft terminate / hard kill，并写 `timeout_status`
- command missing / unauthorized / insufficient permission / incompatible output mode 必须映射为稳定错误码，交给 provider router 决定 retry、gate 或 manual intervention
- `ProviderRunRecord` 必须写入 `provider_capability_ref`、`adapter_compatibility_ref`、`context_package_ref`、`adapter_input_ref`、`adapter_output_ref`、`timeout_status`、`retry_count`、`sandbox_mode`、`approval_policy`、`constraint_check_ref`、`traceability_binding_refs`

- [ ] **Step 6: 运行 CLI adapter baseline 测试**

Run: `cargo test --test cli_adapter_baseline --test provider_error_routes`
Expected: PASS，使用本地 fixture command 验证真实 spawn 路径、错误码映射和路由，不依赖机器已安装 Claude Code / Codex

- [ ] **Step 7: 建议提交点**

```bash
git add src/cross_cutting/cli_adapter.rs src/cross_cutting/provider_capabilities.rs src/cross_cutting/adapter_compatibility.rs src/cross_cutting/provider_run.rs src/protocol/provider_errors.rs tests/cli_adapter_baseline.rs tests/provider_error_routes.rs tests/support
git commit -m "feat: add cli provider adapter baseline"
```

### Task 3: 实现 contract / workflow / prompt registry

**Files:**
- Modify: `src/protocol/contracts.rs`
- Create: `src/protocol/prompt_manifest.rs`
- Create: `src/runtime_units/prompt_template_registry.rs`
- Create: `src/cross_cutting/provider_context_builder.rs`
- Test: `tests/context_builder.rs`

- [ ] **Step 1: 建立 `NodeExecutionContract` 注册表**

至少覆盖：
- `N04`
- `N05`
- `N06 advisory`
- `N07`
- `N08`
- `N09`
- `N10`
- `N11`
- `N12`

- [ ] **Step 2: 建立 `WorkflowDisciplineSpec` 注册表**

要求：
- planning 节点必须标明 `using-superpowers`
- `N04/N05/N07` 标明 `brainstorming`
- `N11` 标明 `writing-plans`

- [ ] **Step 3: 建立 prompt template registry**

要求：
- 固定 render order
- 区分 system / contract / projection / bundle / output schema / failure instruction
- 注册以下一期模板：
  - `tpl_n04_clarification_v1`
  - `tpl_n05_spec_authoring_v1`
  - `tpl_n06_spec_gate_advisory_v1`
  - `tpl_n07_design_authoring_v1`
  - `tpl_n08_design_review_v1`
  - `tpl_n09_design_revision_v1`
  - `tpl_n10_readiness_check_v1`
  - `tpl_n11_plan_authoring_v1`
  - `tpl_n12_dispatch_authoring_v1`
- 每个模板必须声明 `required_sections = [system, node_contract, canonical_inputs, projection_summary, constraint_summary, workflow_discipline, output_schema, completion_or_failure]`
- 每个模板必须声明 `output_schema_ref`，并能被 context builder 渲染进 `AdapterInput.prompt`
- `N06/N08-N12` 不允许只注册 ID，必须至少按实施规格补齐文档的通用骨架 + 节点差异项渲染出完整 prompt
- `context_builder` 测试必须覆盖 `N04-N12` 全部模板的成功渲染和缺变量失败路径

- [ ] **Step 4: 实现 `ProviderContextPackage` builder**

要求：
- 输入 canonical artifact + projection + bundle
- 输出完整 context package 和 adapter input

- [ ] **Step 5: 运行单元测试**

Run: `cargo test --test context_builder`  
Expected: PASS，builder 对各节点组包正确

- [ ] **Step 6: 建议提交点**

```bash
git add src/protocol/contracts.rs src/protocol/prompt_manifest.rs src/runtime_units/prompt_template_registry.rs src/cross_cutting/provider_context_builder.rs tests/context_builder.rs
git commit -m "feat: add execution contract registries and context builder"
```

### Task 4: 实现 `N04-N07` 规划起始链

**Files:**
- Create: `src/runtime_units/clarification.rs`
- Create: `src/runtime_units/spec_authoring.rs`
- Create: `src/runtime_units/spec_gate_review.rs`
- Create: `src/runtime_units/design_authoring.rs`
- Test: `tests/planning_chain_fake_provider.rs`

- [ ] **Step 1: 写失败测试，覆盖 `N04 -> N07`**

断言：
- 每个 Agent 节点都按 `实现总契约_v1.0` 第 8.1 章统一执行链执行：组装 `CanonicalNodeInput` -> 读取 projection / bundle -> 组装 `ProviderContextPackage` -> 映射 `AdapterInput` -> 调用 provider -> 收集 `ProviderRunRecord` -> 归一化 provider 输出 -> `artifact_validate` -> 写 checkpoint
- `clarification_record` 生成
- `spec` 生成并触发 `SpecProjection`
- `spec_gate_decision` 生成
- `N06 pass` 后 daemon 通过 Document Operation 将通过 gate 的 `spec` 写入 `openspec/changes/<change_id>/specs/main/spec.md`
- spec 写回后当前 `OpenSpecConstraintBundle` 标记为 `stale` 并重编译，新 bundle 的 requirement constraints 非空
- `design` 生成并触发 `DesignProjection`

- [ ] **Step 2: 实现 `N04 clarification`**

要求：
- 按统一执行链调用 Claude fake provider，不得直接把 fake provider stdout 当作正式产物
- 产出 `clarification_record`

- [ ] **Step 3: 实现 `N05 spec_authoring` 与 `N06 spec_gate_review`**

要求：
- `N05` 归一化出 `spec`
- `N06` 先执行 `artifact_validate` 校验 `spec` 与 `SpecProjection`
- `N06` advisory 是否启用由 `NodeExecutionContract.advisory_only` / runtime role `AdvisoryReviewer` 控制；不得新增临时全局开关覆盖节点 contract
- advisory 启用时，通过 context builder 构建只读 advisory 请求并调用 Codex，advisory 输出只能作为候选输入和审计证据
- `N06` 最终仍由 daemon 按固定协议字段生成 `spec_gate_decision`，不得由 Codex 直接推进 gate 决策
- daemon 决策顺序固定为：`artifact_validate` -> `SpecProjection` 覆盖检查 -> OpenSpec `proposal_constraints` 覆盖检查 -> 生成 `spec_gate_decision`
- `N06` 不检查 `requirement_constraints` 非空；该约束必须在 `N06 pass` 写回 `specs/main/spec.md` 并重编译 bundle 后，由 `N07` 进入前检查
- advisory 可以提供 findings 或 blocking issue 作为 daemon 判定 `backtrack` 的输入，但 daemon 保留最终决策权；advisory 与 daemon 检查冲突时，以 daemon 检查结果为准
- `N06` 判定 pass 后，daemon 必须将 canonical `spec` 结构化写回 OpenSpec `specs/main/spec.md`，记录 source manifest，触发 bundle stale / recompile
- spec 写回后的重编译 bundle 必须生成非空 `requirement_constraints`；若为空，阻断 `N07` 并返回 `openspec_requirement_constraints_empty`

- [ ] **Step 4: 实现 `N07 design_authoring`**

要求：
- 产出 `design`
- 编译 `DesignProjection`
- 按统一执行链完成 provider 调用、归一化、校验、traceability 和 checkpoint
- `N07` 只生成 Aria `design`；是否写回 OpenSpec 由 `N08` review 结果裁定，避免未通过评审的设计进入稳定 `design.md`

- [ ] **Step 5: 运行集成测试**

Run: `cargo test --test planning_chain_fake_provider`  
Expected: PASS，前半段规划链跑通

- [ ] **Step 6: 建议提交点**

```bash
git add src/runtime_units/clarification.rs src/runtime_units/spec_authoring.rs src/runtime_units/spec_gate_review.rs src/runtime_units/design_authoring.rs tests/planning_chain_fake_provider.rs
git commit -m "feat: add planning chain start nodes"
```

### Task 5: 实现 `N08-N12` 规划后半链

**Files:**
- Create: `src/runtime_units/design_review.rs`
- Create: `src/runtime_units/design_revision.rs`
- Create: `src/runtime_units/plan_dispatch.rs`
- Modify: `tests/planning_chain_fake_provider.rs`

- [ ] **Step 1: 扩展失败测试，覆盖 `N08-N12`**

断言（happy path + revise path）：
- 每个 Agent 节点都按 `实现总契约_v1.0` 第 8.1 章统一执行链执行，不允许绕过 context builder、ProviderRunRecord、归一化、validator 或 checkpoint
- `design_review` 生成，`review_decision` 枚举值为 `pass/revise/conditional_pass`
- `design_review.review_decision=pass` 时路由到 `N10`
- `design_review.review_decision=pass/conditional_pass` 时 daemon 将稳定 `design` 写入 `openspec/changes/<change_id>/design.md`，并触发 bundle stale / recompile
- `design_review.review_decision=revise` 时必须进入 `N09`
- `N09 design_revision` 必须生成 `design_revision_record` 和更新后的 `design` ref
- `N09` 完成后必须回到 `N08` 再评审
- 若修订跨越中间产物，必须按回流失效规则标记相关 artifact 为 `superseded`
- `readiness_check`
- `plan`
- `PlanProjection`
- `N11` 完成后 daemon 将 plan 中的任务约束写入 `openspec/changes/<change_id>/tasks.md`，并触发 bundle stale / recompile；重编译后的 `task_constraints.task_ids[]` 必须非空
- `dispatch_package`
- `dispatch_package._aria.worktask_routing[]`，且 `execution_mode` 使用统一枚举 `agent_only/human_assisted/human_required`，每项必须包含 `source_work_package_id`
- `plan_dispatch` 的 `RuntimeUnitResult.protocol_steps[]` 必须分别包含 `N10`、`N11`、`N12`，并分别写 snapshot / checkpoint

- [ ] **Step 2: 实现 `N08 design_review` / `N09 design_revision`**

要求：
- `N08` 产出 `design_review`，`review_decision` 取值严格为 `pass/revise/conditional_pass`
- `N08` 可调用 Codex reviewer，但 review 输出必须先归一化为 `design_review` 并通过 validator
- `review_decision=revise` 时触发 `N09`
- `N09` 产出 `design_revision_record`，逐项回应 `design_review.findings`
- `N09` 同时产出更新后的 `design`，并编译新的 `DesignProjection`
- `N09` 完成后回到 `N08` 再评审
- 修订导致的旧 `design` / `DesignProjection` 标记为 `superseded`，由 daemon 写入 `superseded_artifact_refs`

- [ ] **Step 3: 实现 `N10 readiness_check` / `N11 plan_authoring`**

要求：
- 产出 `readiness_check`
- 产出 `plan`
- 编译 `PlanProjection`
- `N10/N11` 必须消费 `SpecProjection`、`DesignProjection` 与 `OpenSpecConstraintBundle`，不得直接解析 Markdown 原文做 readiness 或 plan routing
- `N11` 负责把计划任务约束写入 OpenSpec `tasks.md`；`N12` 不直接导出 `dispatch_package` 到 OpenSpec
- 写入 `tasks.md` 后必须重编译 bundle，并用新 `task_constraints` 约束 `N12`

- [ ] **Step 4: 实现 `N12 dispatch_authoring`**

要求：
- 产出 `dispatch_package`
- 填充 `_aria.worktask_routing[]`
- `_aria.worktask_routing[]` 每项必须包含 `source_work_package_id`，用于执行报告 traceability 回查 `PlanProjection.work_packages[]`

- [ ] **Step 5: 运行集成测试**

Run: `cargo test --test planning_chain_fake_provider`  
Expected: PASS，`N04-N12` 全链路通过

- [ ] **Step 6: 建议提交点**

```bash
git add src/runtime_units/design_review.rs src/runtime_units/design_revision.rs src/runtime_units/plan_dispatch.rs tests/planning_chain_fake_provider.rs
git commit -m "feat: add planning chain review readiness and dispatch nodes"
```

### Task 6: 实现 Risk Registry 条目写入与引用验证

**Files:**
- Modify: `src/cross_cutting/provider_run.rs`
- Modify: `src/protocol/artifacts.rs`
- Test: `tests/risk_registry_minimal.rs`

- [ ] **Step 1: 写失败测试，覆盖 Risk Registry 条目能力**

  断言：
  - P1/P2 已为 EpicTask 初始化空 Risk Registry sidecar snapshot 与 `risk_registry_ref`
  - risk_id 可被分配并唯一
  - risk entry 可被创建，包含 `risk_id`、`description`、`severity`、`status`
  - risk registry 可落盘到 task 级 `risk-registry/` sidecar，并同步进入 runtime snapshot
  - daemon 重启后可恢复 risk registry
  - 产物引用中的 risk_id 可被正确解析

- [ ] **Step 2: 在 provider run 和 artifact 中接入 risk registry**

  要求：
  - `ProviderRunRecord` **不**直接持有 `risk_registry_ref`；Risk Registry 关联通过以下字段建立：
    - `CanonicalNodeInput.risk_registry_ref`：节点输入时的 Risk Registry 快照引用
    - `RuntimeSnapshot.risk_registry`：运行时快照中的风险注册表状态
    - `ArtifactTraceabilityBinding.related_risk_ids`：产物与风险的追踪绑定
  - planning 节点（如 `N08 design_review`、`N10 readiness_check`）发现风险时可写入 risk entry
  - risk registry snapshot 成为 `RuntimeSnapshot` 的合法子结构
  - 若缺少 P1/P2 初始化的 `risk_registry_ref`，`ProviderContextPackage` builder 必须失败，不得临时创建匿名 registry
  - 若后续确实要求 `ProviderRunRecord` 直接持有 `risk_registry_ref`，必须先升版总契约和补齐规格，再让研发实现

- [ ] **Step 3: 运行验证**

  Run: `cargo test --test risk_registry_minimal`
  Expected: PASS，risk_id 创建、引用、落盘、恢复都通过

- [ ] **Step 4: 建议提交点**

  ```bash
  git add src/cross_cutting/provider_run.rs src/protocol/artifacts.rs tests/risk_registry_minimal.rs
  git commit -m "feat: add risk registry minimal validation"
  ```

---

## 4. P3 完成判定

- [ ] `cargo test --test provider_adapter_baseline` 通过
- [ ] `cargo test --test context_builder` 通过
- [ ] `cargo test --test cli_adapter_baseline --test provider_error_routes` 通过
- [ ] `cargo test --test planning_chain_fake_provider` 通过，且覆盖 `N08 review_decision=revise` → `N09` → 回到 `N08` 再评审的闭环
- [ ] `cargo test --test risk_registry_minimal` 通过，risk_id 创建、引用、落盘、恢复覆盖完整
- [ ] `ProviderContextPackage -> AdapterInput` 映射稳定
- [ ] fake provider 与 CLI adapter 共用 `AdapterInput` / `AdapterOutput`
- [ ] provider capability / compatibility matrix / provider error code 写入 `ProviderRunRecord`
- [ ] prompt template registry 注册 `N04-N12` 一期模板并可稳定渲染；N08 模板差异项使用 `review_decision=pass/revise/conditional_pass`
- [ ] 真实 Claude Code / Codex 作为用户本机 BYO CLI 接入；CLI 缺失、未登录或权限不足时可诊断失败，不自动安装、不静默降级
- [ ] `N04-N12` 可在 fake provider 下跑通，且 `design_revision_record` 在 revise 路径中稳定产出
- [ ] `N06/N08/N11` 写回 OpenSpec 后都会触发 bundle stale / recompile，且新 bundle 中 requirement/design/task constraints 非空
- [ ] `dispatch_package._aria.worktask_routing[]` 稳定生成，且 `execution_mode` 使用统一枚举 `agent_only/human_assisted/human_required`，每项包含 `source_work_package_id`
- [ ] `plan_dispatch` 折叠实现单元分别写入 `N10/N11/N12` 的 `RuntimeProtocolStep`、runtime snapshot 与 checkpoint
- [ ] fake provider 输出不会绕过 canonical validator；prompt manifest 输出 schema 与上游产物枚举一致
- [ ] **协议不漂移检查**：P3 实现字段、provider contract、prompt template、`ProviderRunRecord` 审计字段、fake provider sentinel 与 `实现总契约_v1.0`、`评审后实施规格补齐_v1.4` 一致
- [ ] **`ProviderRunRecord` 字段一致性**：`ProviderRunRecord` **不**包含 `risk_registry_ref`；Risk Registry 关联通过 `CanonicalNodeInput.risk_registry_ref`、`RuntimeSnapshot.risk_registry`、`ArtifactTraceabilityBinding.related_risk_ids` 建立
