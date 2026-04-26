# 技术方案：Aria IO 协作协议与 Provider 契约

**文档信息**

- **设计编号**：DES-2026-04-23-ARIA-IO-PROVIDER
- **创建日期**：2026-04-26
- **版本**：v1.1
- **负责人**：Codex
- **修正内容**：明确 `ProviderContextPackage`、`AdapterInput`、`AdapterOutput` 的实现类型以规格补齐 v1.3 为准
- **上游依据**：
  - `cadence/designs/2026-04-23_技术方案_Aria一期MVP精简设计_v1.2.md`
  - `cadence/designs/aria-repl-runtime-docs/2026-04-22_技术方案_Aria全局协议_v1.0.md`
  - `cadence/designs/aria-repl-runtime-docs/2026-04-22_技术方案_Aria节点总目录_v1.0.md`
  - `cadence/designs/aria-repl-runtime-docs/cross-cutting/2026-04-22_技术方案_横切能力provider_adapter_spec_v1.0.md`
  - `cadence/analysis-docs/2026-04-22_分析报告_ClaudeCode_Codex_spawn使用方式与限制调研_v1.1.md`
- **外部参考**：
  - OpenSpec 官方站点：`https://openspec.pro/`

---

## 1. 设计目标

本文件补齐 Aria 一期方案中尚未完全明确的三类问题：

1. **每个节点的输入物和输出物如何统一表达**
   现有节点文档已经有输入契约和输出产物，但缺少一份可执行的全局 IO 矩阵，无法直接支撑 Provider Adapter、产物校验和自动路由实现。
2. **Aria、OpenSpec、Superpowers 三套产物如何协作**
   OpenSpec 的 proposal/spec/design/tasks 与 Aria 的 intake/spec/design/plan/dispatch 有重叠；Superpowers 的 brainstorming/writing-plans/executing-plans 也会生成 spec、plan 和验证记录。必须明确谁是源、谁是候选输入、谁能驱动路由。
3. **Claude Code / Codex 的使用方式如何落到节点契约**
   Aria 不应把具体 CLI flag 固化为协议真相源，但必须定义每个 Agent 节点的 provider、角色、输入包、输出 schema、写权限和失败处理。

本文件不替代上游节点文档和产物规范，而是作为它们之间的**协作层补充协议**。

---

## 2. 核心结论

Aria 一期必须明确三层真相源：

| 层级 | 作用 | 是否可驱动 Aria 路由 |
|------|------|----------------------|
| **Aria Runtime IO** | daemon 维护的 session、task、phase、handoff、checkpoint、runtime_snapshot | 是，唯一运行时真相源 |
| **Aria Canonical Artifact** | `intake_brief`、`spec`、`design`、`plan`、`dispatch_package`、各类 report | 是，但必须通过 `artifact_validate` 并被 checkpoint 固化 |
| **External Workflow Artifact** | OpenSpec、Superpowers、provider raw output 等外部或半外部产物 | 否，必须先归一化为 Aria canonical artifact |

因此：

- **Aria 是 runtime truth**：负责任务状态、路由、恢复、回流失效、集成队列。
- **OpenSpec 是 spec/change truth 的候选来源**：适合承载变更提案、需求 delta、设计说明和任务清单，但不能直接替代 Aria checkpoint。
- **Superpowers 是 provider workflow discipline**：约束 Claude Code / Codex 在 provider run 内部如何思考、计划、验证；它的输出不能直接成为 daemon 真相源。

---

## 3. IO 对象模型补充

### 3.1 CanonicalNodeInput

每个协议节点进入执行前，daemon 必须组装 `CanonicalNodeInput`。它不是新的协议节点产物，而是对全局 handoff package 的实现层扩展。

| 字段 | 必填 | 说明 |
|------|------|------|
| `sessionId` | 是 | 当前 ProjectSession |
| `taskId` | 是 | EpicTask 或 WorkTask |
| `nodeId` | 是 | 当前协议节点 ID |
| `phase` | 是 | 当前阶段 |
| `effectivePolicy` | 是 | 已解析的策略 |
| `artifactRefs` | 是 | 当前节点可引用的 Aria canonical artifacts |
| `externalRefs` | 否 | OpenSpec、Superpowers、raw provider output 等外部来源引用 |
| `worktreeRef` | 条件必填 | 执行阶段节点必须提供 |
| `riskRegistryRef` | 是 | 当前风险注册表快照 |
| `loopCounters` | 是 | 当前循环计数器 |
| `acceptanceTargets` | 条件必填 | 执行、测试、评审节点必须提供 |
| `inputValidationRefs` | 是 | 节点进入前输入校验记录 |

约束：

- `artifactRefs` 不得引用 `superseded` 产物。
- `externalRefs` 只能作为上下文来源，不能替代 `artifactRefs`。
- 若节点依赖的 canonical artifact 不存在，必须回流到负责产出该 artifact 的上游节点，不能用外部文件直接绕过。

### 3.2 ExternalArtifactRef

所有外部产物进入 Aria 时必须登记为 `ExternalArtifactRef`。

代码级裁定：`ExternalArtifactRef`、`ExternalImportStatus`、`CanonicalArtifactOrigin` 的 Rust 类型、JSON schema、fixture 以 `cadence/designs/2026-04-26_技术方案_Aria一期评审后实施规格补齐_v1.3.md` 第 4.4 章为准；本节字段表只说明职责边界，不再另起实现字段。

| 字段 | 必填 | 说明 |
|------|------|------|
| `externalRefId` | 是 | 唯一 ID，例如 `ext_openspec_add_export_001` |
| `sourceSystem` | 是 | `openspec` / `superpowers` / `provider_raw_output` / `user_repl` / `local_file` |
| `sourcePath` | 否 | 仓库内相对路径或 provider run 输出引用 |
| `sourceUri` | 否 | 外部 URI；与 `sourcePath` 至少应有一项可定位来源 |
| `sha256` | 否 | 外部产物内容 hash，用于审计和 stale 判定 |
| `importStatus` | 是 | `candidate` / `normalized` / `rejected` / `superseded` |
| `importedAt` | 是 | 首次登记时间 |
| `normalizedArtifactRef` | 否 | 归一化后生成的 Aria artifact ref |
| `rejectionReason` | 否 | `rejected` 时的拒绝原因 |

### 3.3 CanonicalArtifactOrigin

Aria 产物必须记录来源，避免复制后丢失审计链。

| 字段 | 必填 | 说明 |
|------|------|------|
| `originType` | 是 | `user_repl` / `agent_generated` / `openspec_import` / `superpowers_import` / `daemon_generated` |
| `originRefs` | 否 | 对应 `ExternalArtifactRef`、`ProviderRun` 或上游 artifact refs |
| `providerRunRef` | 否 | 若来源为 provider 候选输出，记录对应 provider run |
| `createdByNode` | 是 | 完成归一化并创建 canonical artifact 的协议节点 ID |
| `createdAt` | 是 | 创建时间 |

---

## 4. OpenSpec / Superpowers / Aria 产物映射

### 4.1 OpenSpec 到 Aria

OpenSpec 通常围绕一个 change folder 组织 proposal、specs、design、tasks。Aria 一期应支持将其作为**上游候选输入**导入。

| OpenSpec 产物 | Aria canonical 目标 | 导入节点 | 规则 |
|---------------|--------------------|----------|------|
| `proposal.md` | `intake_brief` | `N01` | 提取目标、非目标、影响范围、用户原始意图 |
| `specs/main/spec.md` | `spec` | `N05` | 一期固定 main scope；归一化为 Aria `scope`、`user_stories`、`functional_requirements`、`success_criteria`、`open_items` |
| `design.md` | `design` | `N07` | 归一化为 `architecture_summary`、前后端设计、数据模型、API contracts、risk refs |
| `tasks.md` | `plan` 候选输入 | `N11` | 可转成 work packages，但不能直接成为 `dispatch_package` |
| `tasks.md` | `dispatch_package` 候选输入 | `N12` | 必须由 Aria/Claude Code 补齐 WorkTask ID、依赖、parallel group、acceptance targets |

硬规则：

- OpenSpec 文件不能直接驱动 `N13` 注册 WorkTask。
- OpenSpec `tasks.md` 必须先经过 `N11 plan_authoring` 和 `N12 dispatch_authoring` 的 canonical 化。
- 若 OpenSpec `specs/main/spec.md` 与当前 Aria `spec` 冲突，必须创建新版本并将旧 artifact 标记为 `superseded`，或挂 `approval_gate` 等待用户裁决。
- OpenSpec Markdown 只能作为候选来源或约束来源，读取、章节更新、bootstrap 与导出必须经过 Document Operation 层，不能由节点或 provider 直接拼接全文。

### 4.2 Aria 到 OpenSpec

Aria 也可以把 canonical artifact 导出到 OpenSpec，用于团队审阅和长期变更追踪。

| Aria 产物 | OpenSpec 目标 | 触发时机 | 规则 |
|----------|---------------|----------|------|
| `intake_brief` | `proposal.md` | `N01` 完成后可选 | 仅导出用户目标和范围，不导出 runtime 状态 |
| `spec` | `specs/main/spec.md` | `N06` pass 后 | 一期只导出到 main scope，且只导出已通过 gate 的 spec |
| `design` | `design.md` | `N08` pass 或 conditional_pass 后 | 若设计仍需修订，不导出为稳定设计 |
| `plan` | `tasks.md` 草案 | `N11` 完成后可选 | 只能作为人工审阅草案 |
| `dispatch_package` | 不直接导出 | 不适用 | `dispatch_package` 是 Aria 运行交接包，不是 OpenSpec 需求任务文件 |

Document Operation 规则：

1. Aria 自己负责创建、更新、归一化和校验 canonical artifact 与 OpenSpec 文档；provider 只能提供候选内容。
2. Markdown 文档操作必须基于 Markdown AST / heading model；JSON artifact 与 `_aria` 必须基于 serde 类型或结构化 patch；YAML / JSON 配置必须经 parser 读写。
3. ast-grep 可以作为可选 tool adapter，用于代码结构搜索、lint、codemod 或支持语言的结构化查询；它不是 Markdown canonical artifact 的主编辑引擎。

### 4.3 Superpowers 到 Aria

Superpowers 是 Agent 工作方法，不是 Aria 协议层。Aria 调用 Claude Code / Codex 时，可以要求 provider 使用 Superpowers 的思路或技能，但其输出必须被 Aria 接管。

| Superpowers 输出 | Aria canonical 目标 | 允许方式 | 不允许方式 |
|------------------|--------------------|----------|------------|
| brainstorming spec | `clarification_record` / `spec` / `design` 候选输入 | 作为 provider run 输出或 external ref 导入 | 直接覆盖 Aria `spec` |
| writing-plans plan | `plan` 候选输入 | 由 `N11` 归一化为 canonical `plan` | 直接作为 `dispatch_package` |
| executing-plans 执行记录 | `coding_report` / `testing_report` 候选输入 | 由 `N16-N19` 归一化并校验 | 作为 `N20 ready_for_integration` 的正式输入之一；ready / block / rework 决策仍由 daemon 生成 |
| requesting-code-review 结果 | `code_review_report` 候选输入 | 由 `N18` 归一化 | 跳过 Aria code review |
| verification-before-completion 结果 | `testing_report` / `integration_report` 候选输入 | 由 `N17` 或 `N24` 归一化 | 作为唯一完成判定 |

Aria-managed Superpowers 模式必须遵守：

1. provider 可以使用 Superpowers 作为内部工作流，但最终必须输出 Aria 要求的 schema。
2. Superpowers 默认要求的提交、分支收口、路径写入等动作，在 Aria 内部执行时不得自动生效；是否提交由 Aria 的集成节点决定。
3. 如果 Superpowers 生成了 repo 文件，这些文件先登记为 `ExternalArtifactRef(sourceSystem = superpowers)`，再由 Aria 归一化。
4. Provider 不得因为 Superpowers 计划中有 `git commit` 步骤就直接提交主线；一期 candidate commit 由 `N20` 生成，integration branch 上的 cherry-pick / rollback 由 `N23 integration_execute` 控制。

---

## 5. 节点 IO 矩阵

下表定义一期主路径的最小 IO。各节点更详细的字段仍以上游节点文档和产物规范为准。

| 节点 | 主输入 | 可选外部输入 | 主输出 | 主执行者 | 校验与路由要点 |
|------|--------|--------------|--------|----------|----------------|
| `N00 session_bootstrap` | repo root、daemon config | 无 | `runtime_snapshot` | daemon | 建立 session 和恢复入口 |
| `N01 intake_capture` | 用户 REPL 请求 | OpenSpec `proposal.md` | `intake_brief` | REPL + daemon | 外部 proposal 只能作为候选来源 |
| `N02 epic_task_create` | `intake_brief` | 无 | `runtime_snapshot` | daemon | 创建 EpicTask |
| `N03 policy_resolve` | EpicTask、policy config | 阶段级策略覆盖 | `runtime_snapshot` | daemon | 计算 `effectivePolicy` |
| `N04 clarification` | `intake_brief`、`effectivePolicy` | Superpowers brainstorming 草稿 | `clarification_record` | Claude Code | open questions 可挂 gate |
| `N05 spec_authoring` | `intake_brief`、`clarification_record` | OpenSpec `specs/main/spec.md`、Superpowers spec | `spec` | Claude Code | 输出必须符合 Aria `spec` schema |
| `N06 spec_gate_review` | `spec`、`clarification_record` | Codex advisory review | `spec_gate_decision` | daemon + 可选 Codex | `pass` 到 `N07`，`backtrack` 到 `N04`，`hold` 挂 gate |
| `N07 design_authoring` | `spec`、`spec_gate_decision` | OpenSpec `design.md`、Superpowers design | `design` | Claude Code | 风险写入 Risk Registry |
| `N08 design_review` | `design`、`spec` | 无 | `design_review` | Codex | `revise` 到 `N09` |
| `N09 design_revision` | `design`、`design_review` | 无 | `design_revision_record`、更新后的 `design` | Claude Code | 修订后回 `N08` |
| `N10 plan_readiness_check` | `design`、`design_review`、`spec` | 无 | `readiness_check` | Claude Code | 不 ready 则回流 `N07` |
| `N11 plan_authoring` | `readiness_check`、`design`、`spec` | OpenSpec `tasks.md`、Superpowers plan | `plan` | Claude Code | 外部 tasks 只能作为 work package 候选 |
| `N12 dispatch_authoring` | `plan` | OpenSpec `tasks.md` | `dispatch_package` | Claude Code + daemon | 必须生成 WorkTask、依赖、并行组、验收目标 |
| `N13 worktask_register` | `dispatch_package` | 无 | `runtime_snapshot` | daemon | 注册 WorkTask，不接受外部 tasks 直通 |
| `N14 worktree_prepare` | WorkTask registry | 无 | `runtime_snapshot` | daemon | 创建 `WorktreeLease` |
| `N15 execution_route_resolve` | WorkTask、worktree、policy | 无 | `runtime_snapshot` | daemon | 决定进入 `N16/N17/N18` |
| `N16 coding` | WorkTask payload、design refs、worktree | Superpowers executing-plans 候选记录 | `coding_report`、文件变更 | Codex | 写入限定 worktree，不能越界 |
| `N17 testing` | `coding_report`、worktree、acceptance targets | Superpowers verification 记录 | `testing_report` | Codex | fail 到 `N19`，pass 到 `N18` |
| `N18 code_review` | 代码 diff、`testing_report`、design/spec refs | Superpowers review 记录 | `code_review_report` | Codex | revise 到 `N19`，pass 到 `N20` |
| `N19 rework` | failure report、review findings、worktree | Superpowers executing-plans 记录 | 更新后的 coding/testing/review report | Codex | 超过 rework 阈值进 `X08` |
| `N20 ready_for_integration` | `coding_report`、`testing_report`、`code_review_report` | Codex advisory | `runtime_snapshot` | daemon + 可选 Codex | daemon 决定 ready/block/rework |
| `N21 integration_enqueue` | ready WorkTask | 无 | `runtime_snapshot` | daemon | FIFO 入队 |
| `N22 integration_prepare` | queue item、base ref、worktree | 无 | `runtime_snapshot` | daemon | 冲突预检，必要时 gate |
| `N23 integration_execute` | prepared worktree、base ref | 无 | `integration_report` | daemon + git/test toolchain | 一期执行 `git cherry-pick --no-commit <candidateCommitSha>`；失败到 `N19` |
| `N24 integration_verify` | `integration_report`、集成后代码 | Codex verification advisory | `integration_report` | daemon + Codex/本地测试 | fail 可 rollback 或回 `N19` |
| `N25 final_review` | 全部关键 artifacts、integration reports | 无 | `final_review` | Claude Code | pass 到 `N27`；followup 先进入 approval gate，用户确认后才允许 `N26` |
| `N26 patch_followup_dispatch` | `final_review`、用户确认 | 重编译后的 `taskConstraints` | `dispatch_package` | Claude Code + daemon | 一期必须用户显式确认；provider 只输出候选，OpenSpec 更新由 daemon 执行 |
| `N27 final_summary` | all key artifacts、`final_review` | 无 | `final_summary` | Claude Code | 用户最终可读输出 |
| `N28 session_closeout` | `final_summary`、runtime state | 无 | `runtime_snapshot` | daemon | 释放 lease，关闭 session |

---

## 6. Provider IO Contract

### 6.1 ProviderContextPackage

所有 Agent 节点调用 Claude Code / Codex 前，daemon 必须组装 `ProviderContextPackage`。

| 字段 | 必填 | 说明 |
|------|------|------|
| `providerType` | 是 | `claude_code` / `codex` |
| `role` | 是 | `orchestrator` / `executor` / `reviewer` / `advisory_reviewer` |
| `nodeId` | 是 | 当前节点 |
| `worktreePath` | 条件必填 | 执行阶段必须提供 |
| `allowedWriteScope` | 是 | provider 可写路径或只读 |
| `canonicalInputs` | 是 | Aria canonical artifact 摘要和 refs |
| `externalInputs` | 否 | 已登记的 external refs 摘要 |
| `instructions` | 是 | 节点指令 |
| `outputSchemaRef` | 是 | 期望输出产物 schema |
| `completionCriteria` | 是 | 完成判定 |
| `forbiddenActions` | 是 | 禁止越权写文件、直接集成、直接提交主线等 |
| `verificationCommands` | 否 | 节点允许执行的验证命令 |

与 `Aria一期实现总契约_v1.0` 的字段映射裁定：

| 本文旧字段 | 实现总契约字段 | 裁定 |
|-----------|----------------|------|
| `role` | `runtimeRole` | daemon 内部运行角色，允许 `advisory_reviewer` |
| `role` | `adapterRole` | 传给底层 adapter 的角色，只允许 `orchestrator` / `executor` / `reviewer` |
| `role=advisory_reviewer` | `runtimeRole=advisory_reviewer` + `adapterRole=reviewer` + `advisoryOnly=true` | advisory 节点只读，最终决策仍由 daemon 生成 |

实现裁定：

1. `ProviderContextPackage` 的 Rust 类型与 JSON schema 以 `cadence/designs/2026-04-26_技术方案_Aria一期评审后实施规格补齐_v1.3.md` 第 4.7 章为准。
2. 本文表格中的 `role` 只保留为概念说明，不作为新增序列化字段；代码实现必须使用 `runtimeRole`、`adapterRole`、`advisoryOnly`。
3. 底层 provider adapter 的 DTO 以规格补齐 v1.3 第 4.7.3 章 `AdapterInput` / `AdapterOutput` 为准；fake provider 与 CLI adapter 必须共用同一 DTO。

### 6.2 Claude Code 节点契约

Claude Code 一期主要作为 orchestrator。

| 节点 | role | 写权限 | 期望输出 | 特别约束 |
|------|------|--------|----------|----------|
| `N04` | `orchestrator` | 只写 Aria 产物区或 stdout | `clarification_record` | 必须列出 assumptions 和 open questions |
| `N05` | `orchestrator` | 只写 Aria 产物区或 stdout | `spec` | 不得跳过 `open_items` |
| `N07` | `orchestrator` | 只写 Aria 产物区或 stdout | `design` | 风险必须可映射到 Risk Registry |
| `N09` | `orchestrator` | 只写 Aria 产物区或 stdout | `design_revision_record`、更新后 design | 必须逐项回应 `design_review.findings` |
| `N10` | `orchestrator` | 只写 Aria 产物区或 stdout | `readiness_check` | 不 ready 时必须给出回流节点 |
| `N11` | `orchestrator` | 只写 Aria 产物区或 stdout | `plan` | 可吸收 OpenSpec/Superpowers tasks，但必须 canonical 化 |
| `N12` | `orchestrator` | 只写 Aria 产物区或 stdout | `dispatch_package` | WorkTask 必须有验收目标 |
| `N25` | `orchestrator` | 只写 Aria 产物区或 stdout | `final_review` | followup 一期必须交给 gate 决策 |
| `N26` | `orchestrator` | provider 只写 Aria 产物区或 stdout；daemon 可通过 Document Operation 更新 OpenSpec `tasks.md` | `dispatch_package` / patch task delta 候选 | 仅用户确认后进入；OpenSpec 实际更新由 daemon 执行 |
| `N27` | `orchestrator` | 只写 Aria 产物区或 stdout | `final_summary` | 必须引用验证结果和剩余风险 |

`N26` 写权限裁定：

1. Claude Code provider 不得直接修改 `openspec/changes/<changeId>/tasks.md`，只能输出候选 `dispatch_package` 或 patch task delta。
2. daemon 在 gate approve 后使用 Document Operation 更新 OpenSpec `tasks.md`，并触发 bundle stale / recompile。
3. 新 `dispatch_package` 必须绑定重编译后的 `taskConstraints` 并通过 OpenSpec coverage 校验。

### 6.3 Codex 节点契约

Codex 一期主要作为 executor / reviewer。

| 节点 | role | 写权限 | 期望输出 | 特别约束 |
|------|------|--------|----------|----------|
| `N06` advisory | `advisory_reviewer` | 只读 | advisory review | 最终 gate decision 仍由 daemon 生成 |
| `N08` | `reviewer` | 只读 | `design_review` | findings 必须可操作 |
| `N16` | `executor` | 当前 WorkTask worktree 内允许写 | `coding_report`、文件变更 | 不得修改任务边界外文件 |
| `N17` | `executor` | 当前 WorkTask worktree 内允许写测试补丁；默认不改生产代码，除非 route 指明 | `testing_report` | 测试失败必须列 failure 和 next recommendation |
| `N18` | `reviewer` | 只读 | `code_review_report` | 必须基于 diff、spec、design、tests |
| `N19` | `executor` / `reviewer` | 当前 WorkTask worktree 内允许写 | 更新后的 report | 只修复失败或评审指出的问题 |
| `N20` advisory | `advisory_reviewer` | 只读 | advisory review | daemon 决定 ready/block/rework |
| `N24` advisory | `advisory_reviewer` | 只读，除非 daemon 显式授权 | verification advisory | 不直接 rollback |

### 6.4 CLI / SDK 边界

Provider Adapter 的稳定边界是 `AdapterInput` / `AdapterOutput`，不是具体 CLI 参数。字段定义以 `Aria一期评审后实施规格补齐_v1.3` 第 4.7.3 章为准。

一期允许默认配置：

| Provider | 一期默认接入 | 可配置项 | 二期升级 |
|----------|--------------|----------|----------|
| Claude Code | 用户本机 CLI + CLI spawn | command、prompt 输入方式、output format、resume/fork、permission mode | Claude Agent SDK |
| Codex | 用户本机 CLI + CLI spawn / `codex exec` | command、json stream、output schema、resume、sandbox、approval policy | Codex SDK / app-server |

硬规则：

- Aria 一期采用 BYO Provider CLI：用户负责在本机安装、登录和授权 Claude Code / Codex CLI；Aria 不内置 provider、不托管远程模型、不自动安装 CLI。
- 具体 CLI flag 必须放入 adapter compatibility matrix 或运行时配置，不写死为协议字段。
- adapter 启动时应探测 provider 版本和能力，并写入 provider run record。
- 若 provider CLI 不存在、未登录、权限不足或能力不满足节点要求，daemon 必须阻断当前 provider run，返回可诊断错误并按 policy 进入 retry、gate 或 manual intervention。
- provider stdout/stderr 不是系统真相源；只有归一化后的 Aria artifact、validation record、checkpoint 才是系统真相源。

一期 CLI adapter 最低验收：

| 项目 | 必须落地的行为 |
|------|----------------|
| capability probe | 启动前探测 provider command 是否存在、版本字符串、支持的输出模式，并生成 `providerCapabilityRef` |
| compatibility matrix | 为 Claude Code 与 Codex 分别记录 command、stdin/prompt 输入方式、输出解析策略、session/resume 参数、sandbox/approval 参数 |
| spawn execution | 能在指定 `worktreePath` 下启动进程，并按 `timeout` / `maxRetries` 执行 |
| output capture | stdout、stderr、exit code、duration、structured output、parse error 全部写入 `AdapterOutput` |
| diff capture | 运行前后检测 worktree 文件变更，生成 `filesModified`；不得只接受 provider 自报 |
| timeout handling | soft timeout 后尝试终止，hard timeout 后强制结束，并把结果写入 `ProviderRunRecord.timeoutStatus` |
| fake provider parity | fake provider 与 CLI adapter 使用同一 `AdapterInput` / `AdapterOutput` 类型，避免测试路径和真实路径分叉 |

真实 CLI adapter baseline 只验证 adapter 行为，不要求 Claude Code / Codex 在测试中产出高质量业务结果；业务结果仍通过 fake provider fixture 固化。

一期 provider 错误必须归一化为稳定错误码：

| 错误码 | 说明 |
|--------|------|
| `provider_command_missing` | provider command 不存在 |
| `provider_unauthorized` | CLI 未登录、token 失效或账号未授权 |
| `provider_permission_denied` | sandbox、文件系统或审批策略拒绝 |
| `provider_incompatible_output` | provider 不支持所需 output mode 或 schema |
| `provider_timeout` | provider run 超时 |
| `provider_parse_error` | structured output 无法解析 |
| `provider_execution_failed` | 其他 provider 非零退出 |

这些错误码必须进入 `ProviderRunRecord` 和 `provider_run.failed` event；不得只依赖 stderr 文本做路由。

---

## 7. 归一化与校验流程

### 7.1 外部输入导入流程

1. 用户或 daemon 指定外部来源，例如 OpenSpec change。
2. daemon 创建 `ExternalArtifactRef(importStatus = candidate)`。
3. daemon 执行来源存在性和 hash 记录。
4. 对照映射表生成 canonical artifact 草稿。
5. 写入或更新 Markdown / JSON / YAML 时必须通过 Document Operation 层完成结构化修改。
6. 执行 `artifact_validate`：
   - L1：必填字段存在
   - L2：字段类型、枚举值、引用关系正确
   - L3：语义完整性，一期可延后增强
7. 校验通过后写入 Aria artifact，并记录 `CanonicalArtifactOrigin`。
8. 写 checkpoint 和 runtime snapshot。

### 7.2 Provider 输出归一化流程

1. daemon 派发 provider run。
2. adapter 收集 stdout/stderr、结构化输出和文件变更。
3. daemon 将 raw output 存为 provider run record。
4. 节点实现单元从 provider output 中提取目标 artifact。
5. 通过 Document Operation 层执行结构化归一化和写入。
6. `artifact_validate` 校验目标 artifact。
7. 校验通过后写入 canonical artifact。
8. 生成 handoff package 和 checkpoint。

### 7.3 冲突处理

| 冲突类型 | 处理规则 |
|----------|----------|
| OpenSpec spec 与 Aria spec 冲突 | 挂 `approval_gate`，用户选择采用哪一版或要求合并 |
| Superpowers plan 与 Aria plan 冲突 | `N10/N11` 阶段重新做 readiness/plan，不直接覆盖 |
| Provider 输出 schema 不合法 | 触发 retry；超过阈值后 gate 或 manual intervention |
| Provider 修改越界文件 | `N16/N19` 进入 manual intervention，或按 policy 回滚 worktree |
| 回流后外部引用仍指向旧 artifact | daemon 标记旧 Aria artifact `superseded`，外部引用仅保留审计用途 |

---

## 8. 最小命令面建议

本文件只定义语义，不强制最终 REPL 命令名。为实现一期闭环，最小必需命令以 `Aria一期实现总契约` 的 command registry 为准；以下命令语义中，`import/export openspec` 是 P2 之后的扩展建议，不作为 P1 通信验收阻塞项。

| 命令语义 | 作用 |
|----------|------|
| `new <request>` | 创建原生 Aria EpicTask |
| `import openspec <changeId>` | 导入 OpenSpec change 为候选输入；P2 之后可实现 |
| `status` | 查看 session、task、gate、queue |
| `artifacts` | 查看 canonical artifacts 与 external refs |
| `approve <gateId>` | 通过 gate |
| `reject <gateId>` | 拒绝 gate 并进入回流或人工介入 |
| `reply <gateId> <text>` | 为 clarification 或 manual intervention 提供补充 |
| `export openspec <taskId>` | 将已通过 gate 的 Aria spec/design/plan 导出为 OpenSpec 候选文档；P2 之后可实现 |

---

## 9. 与现有文档的关系

本文件对现有文档形成如下补充：

| 现有文档 | 补充方式 |
|----------|----------|
| 节点总目录 | 不改节点 ID，只补全跨节点 IO 矩阵 |
| 节点文档 | 节点仍保留自己的输入契约；本文件提供全局映射和外部来源规则 |
| 产物规范 | 不改 canonical 字段；本文件定义外部产物如何归一化到 canonical 字段 |
| Provider Adapter | 不替代 `AdapterInput/AdapterOutput`；本文件定义每个节点如何使用 adapter |
| MVP 精简设计 | 不改变 21 个实现单元；本文件让实现单元能按协议边界组装输入输出 |

---

## 10. 一期验收标准

一期实现本协议后，至少应满足：

1. 任意节点进入前都能生成 `CanonicalNodeInput`。
2. 任意 Agent 节点都能生成 `ProviderContextPackage`。
3. OpenSpec proposal/spec/design/tasks 可以作为候选输入导入，但不会直接驱动 Aria 路由。
4. Superpowers 产物可以被登记和归一化，但不会直接覆盖 Aria canonical artifacts。
5. Claude Code / Codex 的 raw output 都会被保存为 provider run record，而不是直接成为系统真相源。
6. `N16-N19` 中 provider 的写权限受 worktree 和 WorkTask 边界限制。
7. `N23` 仍是集成、提交、合入、回滚的默认控制点，provider 内部计划不得绕过。
8. 所有归一化后的产物都能通过 L1/L2 校验并写入 checkpoint。

---

## 11. 后续实施建议

后续 implementation plan 应拆成四块：

1. **协议模型**
   实现 `CanonicalNodeInput`、`ExternalArtifactRef`、`CanonicalArtifactOrigin`、`ProviderContextPackage`。
2. **映射注册表**
   实现 OpenSpec、Superpowers、Aria artifact 的映射表和校验入口。
3. **Provider Contract Registry**
   为每个 Agent 节点定义 provider、role、write scope、output schema、failure route。
4. **REPL 命令与可观测性**
   支持 external refs、artifacts、provider runs、gates、queue 的查询和导入导出命令。

---

## 12. 一句话总结

Aria 不应该让 OpenSpec、Superpowers、Claude Code 或 Codex 直接决定运行时状态；它们都可以提供高质量输入，但必须先被 Aria 归一化、校验、固化为 canonical artifact 和 checkpoint，才能进入下一节点。
