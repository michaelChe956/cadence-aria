# Aria Phase 1 P3 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 建立 BYO 本地 CLI provider contract、prompt/contract registry、provider context builder，并打通 `N04-N12` 规划链。

**Architecture:** 先用 fake provider 跑通 planning chain，再接真实 CLI adapter。Claude Code / Codex 由用户在本机安装、登录和授权，Aria 只做 capability probe、compatibility matrix 与受控 spawn。P3 的重点不是模型效果，而是 Aria 怎样稳定地把 canonical artifact、projection、bundle、discipline、prompt template 组装成 provider 可执行输入，并在 CLI 缺失、未登录或能力不足时给出可诊断失败。

**Tech Stack:** Rust、fake provider adapter、BYO 本地 CLI adapter、JSON、fixture-based integration tests。

---

## 1. 范围与出口

P3 完成后，必须满足：

1. `NodeExecutionContract`、`WorkflowDisciplineSpec`、`NodePromptTemplateRef` 有注册表
2. `ProviderContextPackage -> AdapterInput` 映射成立
3. fake provider 与真实 CLI adapter baseline 共用 `AdapterInput` / `AdapterOutput`
4. Claude Code / Codex 按用户本机 CLI 接入，Aria 不内置 provider、不托管远程模型、不自动安装 CLI
5. provider capability / compatibility matrix 能写入 `ProviderRunRecord`
6. fake provider 下 `N04-N12` 规划链可跑
7. 可产出：
   - `clarification_record`
   - `spec`
   - `spec_gate_decision`
   - `design`
   - `design_review`
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
- Create: `src/runtime_units/clarification.rs`
- Create: `src/runtime_units/spec_authoring.rs`
- Create: `src/runtime_units/spec_gate_review.rs`
- Create: `src/runtime_units/design_authoring.rs`
- Create: `src/runtime_units/design_review.rs`
- Create: `src/runtime_units/design_revision.rs`
- Create: `src/runtime_units/plan_dispatch.rs`
- Create: `src/protocol/contracts.rs`
- Create: `src/runtime_units/prompt_template_registry.rs`
- Create: `tests/context_builder.rs`
- Create: `tests/cli_adapter_baseline.rs`
- Create: `tests/planning_chain_fake_provider.rs`
- Create: `tests/support/mod.rs`

---

## 3. 任务拆解

### Task 1: 实现 provider adapter 基线与 fake provider

**Files:**
- Create: `src/cross_cutting/provider_adapter.rs`
- Create: `src/cross_cutting/provider_run.rs`
- Create: `src/cross_cutting/provider_router.rs`
- Test: `tests/context_builder.rs`

- [ ] **Step 1: 定义 `AdapterInput` / `AdapterOutput` 运行时封装**

要求：
- 能记录 raw stdout/stderr
- 能记录 structured output
- 能记录 files modified
- fake provider 和 CLI adapter 后续必须共用同一组 `AdapterInput` / `AdapterOutput` 类型

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
- duration
- timeout / retry

- [ ] **Step 4: 运行单元验证**

Run: `cargo test --test context_builder`  
Expected: PASS，fake provider 可被 builder 调用

- [ ] **Step 5: 提交阶段性变更**

```bash
git add src/cross_cutting/provider_adapter.rs src/cross_cutting/provider_run.rs src/cross_cutting/provider_router.rs tests/context_builder.rs tests/support
git commit -m "feat: add provider adapter baseline and fake provider"
```

### Task 2: 实现真实 CLI adapter baseline

**Files:**
- Create: `src/cross_cutting/cli_adapter.rs`
- Create: `src/cross_cutting/provider_capabilities.rs`
- Create: `src/cross_cutting/adapter_compatibility.rs`
- Modify: `src/cross_cutting/provider_run.rs`
- Test: `tests/cli_adapter_baseline.rs`
- Test support: `tests/support/mod.rs`

- [ ] **Step 1: 写失败测试，覆盖 capability probe 与 compatibility matrix**

断言：
- fixture provider command 可被探测
- 探测结果生成 `providerCapabilityRef`
- Claude Code / Codex 都有默认 compatibility matrix entry
- provider command 不存在时返回可诊断错误，不 panic
- provider command 存在但返回 unauthorized / permission denied 时返回可诊断错误，不进入节点收口
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
- `claude_code` 的 command、prompt input mode、output parser、session flag、permission mode 字段
- `codex` 的 command、prompt input mode、output parser、resume flag、sandbox、approval policy 字段
- matrix version，并写入 provider run record

- [ ] **Step 4: 实现 CLI spawn adapter**

要求：
- 在指定 `worktreePath` 下启动进程
- 捕获 stdout、stderr、exit code、duration
- 解析 JSON structured output；失败时写 `parseError`
- 运行前后检测 worktree diff，写入 `filesModified`
- 按 `timeout` 执行 soft terminate / hard kill，并写 `timeoutStatus`
- command missing / unauthorized / insufficient permission / incompatible output mode 必须映射为稳定错误码，交给 provider router 决定 retry、gate 或 manual intervention

- [ ] **Step 5: 运行 CLI adapter baseline 测试**

Run: `cargo test --test cli_adapter_baseline`
Expected: PASS，使用本地 fixture command 验证真实 spawn 路径，不依赖机器已安装 Claude Code / Codex

- [ ] **Step 6: 提交阶段性变更**

```bash
git add src/cross_cutting/cli_adapter.rs src/cross_cutting/provider_capabilities.rs src/cross_cutting/adapter_compatibility.rs src/cross_cutting/provider_run.rs tests/cli_adapter_baseline.rs tests/support
git commit -m "feat: add cli provider adapter baseline"
```

### Task 3: 实现 contract / workflow / prompt registry

**Files:**
- Create: `src/protocol/contracts.rs`
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

- [ ] **Step 4: 实现 `ProviderContextPackage` builder**

要求：
- 输入 canonical artifact + projection + bundle
- 输出完整 context package 和 adapter input

- [ ] **Step 5: 运行单元测试**

Run: `cargo test --test context_builder`  
Expected: PASS，builder 对各节点组包正确

- [ ] **Step 6: 提交阶段性变更**

```bash
git add src/protocol/contracts.rs src/runtime_units/prompt_template_registry.rs src/cross_cutting/provider_context_builder.rs tests/context_builder.rs
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
- `clarification_record` 生成
- `spec` 生成并触发 `SpecProjection`
- `spec_gate_decision` 生成
- `design` 生成并触发 `DesignProjection`

- [ ] **Step 2: 实现 `N04 clarification`**

要求：
- 调用 Claude fake provider
- 产出 `clarification_record`

- [ ] **Step 3: 实现 `N05 spec_authoring` 与 `N06 spec_gate_review`**

要求：
- `N05` 归一化出 `spec`
- `N06` 由 daemon 生成 `spec_gate_decision`

- [ ] **Step 4: 实现 `N07 design_authoring`**

要求：
- 产出 `design`
- 编译 `DesignProjection`

- [ ] **Step 5: 运行集成测试**

Run: `cargo test --test planning_chain_fake_provider`  
Expected: PASS，前半段规划链跑通

- [ ] **Step 6: 提交阶段性变更**

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

断言：
- `design_review`
- `readiness_check`
- `plan`
- `PlanProjection`
- `dispatch_package`
- `dispatch_package._aria.worktask_routing[]`

- [ ] **Step 2: 实现 `N08 design_review` / `N09 design_revision`**

要求：
- 支持 review findings
- 支持 revision route

- [ ] **Step 3: 实现 `N10 readiness_check` / `N11 plan_authoring`**

要求：
- 产出 `readiness_check`
- 产出 `plan`
- 编译 `PlanProjection`

- [ ] **Step 4: 实现 `N12 dispatch_authoring`**

要求：
- 产出 `dispatch_package`
- 填充 `_aria.worktask_routing[]`

- [ ] **Step 5: 运行集成测试**

Run: `cargo test --test planning_chain_fake_provider`  
Expected: PASS，`N04-N12` 全链路通过

- [ ] **Step 6: 提交阶段性变更**

```bash
git add src/runtime_units/design_review.rs src/runtime_units/design_revision.rs src/runtime_units/plan_dispatch.rs tests/planning_chain_fake_provider.rs
git commit -m "feat: add planning chain review readiness and dispatch nodes"
```

---

## 4. P3 完成判定

- [ ] `cargo test --test context_builder` 通过
- [ ] `cargo test --test cli_adapter_baseline` 通过
- [ ] `cargo test --test planning_chain_fake_provider` 通过
- [ ] `ProviderContextPackage -> AdapterInput` 映射稳定
- [ ] fake provider 与 CLI adapter 共用 `AdapterInput` / `AdapterOutput`
- [ ] provider capability / compatibility matrix 写入 `ProviderRunRecord`
- [ ] 真实 Claude Code / Codex 作为用户本机 BYO CLI 接入；CLI 缺失、未登录或权限不足时可诊断失败，不自动安装、不静默降级
- [ ] `N04-N12` 可在 fake provider 下跑通
- [ ] `dispatch_package._aria.worktask_routing[]` 稳定生成
