# 设计评审：Aria 一期方案与实施计划研发可落地性 Review

**文档信息**
- **创建日期**：2026-04-26
- **版本**：v3.0
- **评审目标**：确认设计方案与实施计划是否足以让研发人员看懂并清楚落地
- **目标读者**：Aria 一期技术负责人、P1-P4 owner、daemon / REPL / provider / OpenSpec / execution 研发负责人
- **评审基准**：以 `Aria一期MVP精简设计_v1.2` 与 `Aria一期实现总契约_v1.0` 为准；若派生文档与基准冲突，以基准裁定

---

## 1. 总体结论

本轮没有发现需要暂停并由用户裁定的 **MVP 设计 v1.2** 或 **实现总契约 v1.0** 自身错误。

但当前文档包还不能标记为“全套一期研发可直接开工无歧义”。P1-P3 基本具备落地条件，P4 与部分派生规格仍存在会让研发写偏的缺口：

1. `Aria_IO协作协议与Provider契约_v1.1` 中 `ExternalArtifactRef` / `CanonicalArtifactOrigin` 字段表仍保留旧口径，与规格补齐 v1.3 的 Rust 类型不一致。
2. `RuntimeUnit.node_id()` 允许返回“实现侧稳定 ID”的表述，与 MVP “实现单元不是新的协议节点 ID”冲突。
3. P4 计划没有明确为 `N16-N20`、`N24`、`N25-N27` 补齐 `NodeExecutionContract` / `WorkflowDisciplineSpec` / prompt template registry。
4. P4 计划只要求 `N22` 检查 candidate commit，但没有把 candidate commit 的生成职责落到 `N20 ready` 前。
5. P4 计划的 `N26 patch_followup_dispatch` 没有说明 OpenSpec `tasks.md` 更新、bundle stale/recompile 与约束覆盖校验。

建议裁定：**P1 在修正本文 P1-4 后启动；P2/P3 可以按现有计划推进；P4 启动前必须修正本文 P1-1、P1-2、P1-3。P2-1 应在进入 P2 前同步修订，避免字段实现分叉。**

---

## 2. 评审对象

### 2.1 背景与基准

| 文档 | 结论 |
|------|------|
| `cadence/designs/2026-04-22_技术方案_Aria终端REPL与多Agent编排Runtime设计_v1.0.md` | 作为文档集入口，未发现阻塞问题 |
| `cadence/designs/2026-04-23_技术方案_Aria一期MVP精简设计_v1.2.md` | MVP 边界清晰，可作为范围基准 |
| `cadence/designs/2026-04-23_技术方案_Aria一期实现总契约_v1.0.md` | 对象模型、统一执行链、OpenSpec 约束和收口规则清晰，可作为实现基准 |

### 2.2 派生方案与计划

| 文档 | 结论 |
|------|------|
| `Aria_IO协作协议与Provider契约_v1.1` | Provider DTO 已指向 v1.3，但 ExternalArtifactRef / Origin 字段仍需消歧 |
| `Aria一期研发导读与实施拆解_v1.1` | 阅读路径和模块拆解清晰 |
| `Aria一期评审后实施规格补齐_v1.3` | 大部分代码级规格可执行，但 `RuntimeUnit.node_id()` 表述需修 |
| `P1基础骨架与REPL通信_v1.1` | 基本可执行，仅有 Markdown checklist 缩进类小问题 |
| `P2产物投影与OpenSpec约束_v1.2` | 可执行，validator / document ops / projection 边界清晰 |
| `P3Provider驱动与规划节点_v1.2` | 可执行，规划链与 provider DTO 已收敛 |
| `P4执行集成与最终收口_v1.1` | 需要补齐执行/收口节点 contract registry、candidate commit 生成点、N26 OpenSpec 处理 |
| `实施计划总览_v1.1` | 总体顺序清晰，但应把 P4 修正项纳入准入门槛 |

---

## 3. 主要问题

### P1-1：P4 未补齐执行与最终收口节点的 provider contract / prompt registry

**位置**

- `cadence/designs/2026-04-23_技术方案_Aria一期实现总契约_v1.0.md` 第 842-864 行：所有 Agent 节点必须查询 `NodeExecutionContract`、解析 `WorkflowDisciplineSpec` / `NodePromptTemplateRef`、组装 `ProviderContextPackage`。
- 同文档第 881-907 行：`N16-N20`、`N24`、`N25-N27` 都在节点级驱动矩阵中。
- `cadence/plans/2026-04-26_计划文档_实施计划_Aria一期实现计划_P3Provider驱动与规划节点_v1.2.md` 第 251-289 行：P3 registry 只覆盖 `N04-N12`。
- `cadence/plans/2026-04-26_计划文档_实施计划_Aria一期实现计划_P4执行集成与最终收口_v1.1.md` 第 47-69 行：P4 文件结构没有修改 `contracts.rs`、`prompt_manifest.rs`、`prompt_template_registry.rs` 或 `provider_context_builder.rs`。

**影响**

研发按 P4 计划实现 `N16/N17/N18/N19/N24/N25/N26/N27` 时，容易绕过总契约的统一执行链，直接在 runtime unit 中拼 fake provider 调用或硬编码 prompt。这样会造成：

- P4 节点无法复用 `ProviderContextPackage -> AdapterInput -> ProviderRunRecord`。
- execution/final 节点缺少统一 `allowedWriteScope`、`forbiddenActions`、`verificationCommands`、Superpowers discipline。
- fake provider 路径和真实 CLI adapter 路径重新分叉。

**建议修正**

在 P4 计划新增一个前置任务，或扩展 Task 2 / Task 4：

| 需要补齐 | 最小范围 |
|----------|----------|
| `NodeExecutionContract` | `N16`、`N17`、`N18`、`N19`、`N20 advisory`、`N24 advisory`、`N25`、`N26`、`N27` |
| `WorkflowDisciplineSpec` | `N16` 含 `test-driven-development`、`verification-before-completion`；`N17/N24` 含 `verification-before-completion`；失败路径含 `systematic-debugging`；`N19` 含 `receiving-code-review` |
| prompt templates | 至少可渲染骨架 + 节点差异项，不允许只有 template id |
| tests | `context_builder` 增加 P4 节点覆盖；`execution_chain_fake_provider` 断言执行节点走统一执行链 |

---

### P1-2：P4 缺少 candidate commit 的生成职责

**位置**

- `cadence/designs/2026-04-26_技术方案_Aria一期评审后实施规格补齐_v1.3.md` 第 1847-1854 行：`N20 ready` 前 daemon 必须校验写范围、`git add`、生成 candidate commit 并记录 `candidateCommitSha`。
- `cadence/plans/2026-04-26_计划文档_实施计划_Aria一期实现计划_P4执行集成与最终收口_v1.1.md` 第 169-173 行：`N20` 只写 ready/block/rework 决策。
- 同计划第 212-221 行：`N22` 要求 candidate commit 已存在，缺失则报错。

**影响**

计划只定义了 candidate commit 的消费者，没有定义生产者。研发可能把 candidate commit 放到 `N22` 临时创建，或者让 `N23` 直接 cherry-pick worktree diff；这会偏离 v1.3 第 13 章的审计链。

**建议修正**

在 P4 Task 2 的 `N20 ready_for_integration` 下补充：

1. `N20` 判定 ready 前校验 worktree diff 只落在 `allowedWriteScope`。
2. daemon 对授权范围执行 `git add`。
3. daemon 创建 `aria: <worktaskId> candidate` commit。
4. 记录 `candidateCommitSha` 到 worktask / integration prepare input / runtime snapshot。
5. `N20` 测试断言没有 `candidateCommitSha` 不得进入 `ready`。

`N22` 继续负责 preflight 和 `preMergeSha`，不再承担 candidate commit 生成。

---

### P1-3：P4 的 `N26 patch_followup_dispatch` 缺少 OpenSpec 更新与 bundle 失效/重编译

**位置**

- `cadence/designs/2026-04-23_技术方案_Aria一期实现总契约_v1.0.md` 第 959-964 行：`N05/N07/N11/N26` 写入 OpenSpec 文件后需要编译 `OpenSpecConstraintBundle`。
- 同文档第 1004-1008 行：回流跨越 `N05/N07/N11/N26` 中任一 OpenSpec 写节点，bundle 必须置为 `stale`。
- `cadence/plans/2026-04-26_计划文档_实施计划_Aria一期实现计划_P4执行集成与最终收口_v1.1.md` 第 304-311 行：`N26` 只说明生成新 `dispatch_package`、写 `_aria.worktask_routing[]`、增加 `patch_round_counter`、回到 `N13`。

**影响**

followup patch 会产生新的执行任务，但计划没有说明这些任务如何进入 OpenSpec `tasks.md`、如何触发 bundle stale/recompile、如何保证新 dispatch package 仍受 `taskConstraints` 约束。研发可能只生成 Aria dispatch，导致 OpenSpec 约束链断开。

**建议修正**

在 P4 Task 4 的 `N26` 步骤补充：

1. gate approve 后，由 daemon 授权 `N26` 更新 OpenSpec `tasks.md` 或明确生成一个 patch task delta。
2. 更新必须走 Document Operation，不允许 provider 直接拼接 OpenSpec Markdown。
3. 更新后将当前 bundle 标记为 `stale` 并重编译。
4. 新 `dispatch_package` 必须覆盖新的 `taskConstraints`，并通过 OpenSpec 约束覆盖校验。
5. `final_followup_routes` 测试增加：approve gate -> N26 -> OpenSpec bundle version 递增或 bundle ref 更新 -> 新 dispatch package 绑定新 task id。

---

### P1-4：`RuntimeUnit.node_id()` 允许实现侧 ID，容易违反 MVP 的协议节点边界

**位置**

- `cadence/designs/2026-04-23_技术方案_Aria一期MVP精简设计_v1.2.md` 第 126-130 行：实现单元不是新的协议节点 ID，协议节点定义、顺序、路由保持不变。
- 同文档第 141-143 行：`Mxx` 只是代码/运行时层面的折叠，不替代协议层节点。
- `cadence/designs/2026-04-26_技术方案_Aria一期评审后实施规格补齐_v1.3.md` 第 950-963 行：`RuntimeUnit.node_id()` 返回 `NodeId`，实现规则允许返回 `N10` 或“实现侧稳定 ID”。

**影响**

`NodeId` 在 runtime snapshot、event、handoff、traceability 中都是协议 ID。如果 `node_id()` 返回实现侧 ID，研发可能把 `M10` 或其他 runtime unit id 写进 checkpoint / event，造成协议漂移。

**建议修正**

将 v1.3 第 963 行改为：

- `node_id()` 必须返回当前主协议节点 ID，例如 `M10 plan_dispatch_impl` 返回 `N10`。
- `covered_protocol_nodes()` 必须列出折叠覆盖的协议节点，例如 `["N10", "N11", "N12"]`。
- 如需实现侧稳定 ID，新增 `runtime_unit_id() -> String` 或 `RuntimeUnitMetadata.runtime_unit_id`，不得复用 `NodeId`。

P1 的 `runtime_snapshot_schema` 和 P3/P4 的 chain tests 应增加断言：event/snapshot 中的 `nodeId` 只能是 `N00-N28` 或 `X01-X09`，不能是 `Mxx`。

---

### P2-1：IO 协作协议的 ExternalArtifactRef / CanonicalArtifactOrigin 字段表仍是旧口径

**位置**

- `cadence/designs/2026-04-26_技术方案_Aria_IO协作协议与Provider契约_v1.1.md` 第 81-105 行：`ExternalArtifactRef` 包含 `sourceVersion`、`artifactKind`、`mappedCanonicalType`、`validationRefs`，`importStatus` 为 `candidate/imported/rejected/superseded`；`CanonicalArtifactOrigin` 包含 `normalizedByNode`、`normalizationSummary`。
- `cadence/designs/2026-04-26_技术方案_Aria一期评审后实施规格补齐_v1.3.md` 第 367-422 行：Rust 类型使用 `sourceUri`、`sha256`、`importedAt`、`normalizedArtifactRef`、`rejectionReason`，`importStatus` 为 `candidate/normalized/rejected/superseded`；Origin 使用 `providerRunRef`、`createdByNode`、`createdAt`。

**影响**

P1/P2 研发可能按 IO 文档旧字段实现 `src/protocol/artifacts.rs`，再与 v1.3、P1/P2 plan 冲突。尤其 `imported` vs `normalized` 会直接导致状态机和 validator 枚举不一致。

**建议修正**

在 IO 协作协议第 3.2 / 3.3 加一条裁定：

> 本节字段表仅保留概念说明。`ExternalArtifactRef`、`ExternalImportStatus`、`CanonicalArtifactOrigin` 的 Rust 类型、JSON schema、fixture 以 `Aria一期评审后实施规格补齐_v1.3` 第 4.4 章为准。

同时把表格字段同步为 v1.3 字段，避免研发检索时误读旧字段。

---

## 4. 非阻塞优化

### R1：P1 Task 5 的 Markdown checklist 缩进会影响阅读

`cadence/plans/2026-04-26_计划文档_实施计划_Aria一期实现计划_P1基础骨架与REPL通信_v1.1.md` 第 439-472 行中，Step 3-5 看起来被缩进到 Step 2 下。技术含义不变，但研发按 checkbox 执行时容易误认为这些是 Step 2 的子任务。

建议去掉这些 Step 前的两个空格，让 Task 5 的 Step 1-6 平级展示。

### R2：计划中的 git commit 步骤应继续保留“团队策略优先”说明

P1-P4 都包含阶段性 `git add` / `git commit` 命令，并已说明对多人团队以分支策略为准。这不构成阻塞。建议总览中也补一句：自动化 agent 可按计划提交，人工团队可按 PR 粒度合并提交。

---

## 5. 研发可执行裁定

| 阶段 | 当前判断 | 必须处理 |
|------|----------|----------|
| P1 | 修 P1-4 后启动 | `RuntimeUnit.node_id()` 必须先消歧；修 checklist 缩进更利于执行 |
| P2 | 可以启动 | 修 IO 文档旧字段，避免 `ExternalArtifactRef` / Origin 实现分叉 |
| P3 | 可以启动 | 无新增阻塞项 |
| P4 | 暂不建议按现文档启动 | 先补 contract/prompt registry、candidate commit 生成、N26 OpenSpec bundle 流程 |

---

## 6. 建议修订清单

按优先级处理：

1. 修改规格补齐 v1.3 第 4.7.4：`RuntimeUnit.node_id()` 只能返回协议节点 ID；需要实现侧 ID 时新增字段。
2. 修改 P4 plan：新增 P4 节点的 contract / workflow / prompt registry 扩展任务和 `context_builder` 测试。
3. 修改 P4 plan：把 candidate commit 生成落到 `N20 ready` 前，并增加测试断言。
4. 修改 P4 plan：补齐 `N26` 对 OpenSpec `tasks.md`、bundle stale/recompile、约束覆盖校验的要求。
5. 修改 IO 协作协议 v1.1：同步 `ExternalArtifactRef` / `CanonicalArtifactOrigin` 字段到规格补齐 v1.3，或明确旧表仅为概念说明。
6. 修改 P1 plan：整理 Task 5 checklist 缩进。

---

## 7. 一句话总结

当前文档包已经比上一轮更接近可研发落地：P1-P3 基本清楚，MVP 和总契约没有发现需要推翻的问题。但全套一期还差 P4 的三个关键落地点和两个字段口径消歧；修完后，研发才能仅凭方案和计划稳定实现而不需要再自行脑补执行链。
