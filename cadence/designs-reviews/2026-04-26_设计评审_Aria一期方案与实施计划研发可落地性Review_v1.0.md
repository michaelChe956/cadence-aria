# 设计评审：Aria 一期方案与实施计划研发可落地性 Review

**文档信息**
- **创建日期**：2026-04-26
- **版本**：v1.0
- **评审对象**：Aria 一期 MVP 设计、实现总契约、IO/Provider 契约、实施规格补齐、P1-P4 实施计划
- **目标读者**：技术负责人、daemon / REPL / provider / document operation / OpenSpec / execution 研发负责人
- **评审目标**：判断研发人员能否根据现有设计方案与实施计划清楚落地实现，并指出必须先修正的冲突和缺口

---

## 1. 总体结论

本次评审结论：**有条件通过，可以作为一期研发基线，但进入对应阶段前必须先修正本文列出的 P0/P1 问题。**

未发现必须推翻 `Aria一期MVP精简设计_v1.2` 或 `Aria一期实现总契约_v1.0` 的设计错误。当前主要问题不在 MVP 目标或总契约方向，而在实施计划与补齐规格之间存在少量字段级不一致，以及部分测试验收还不够硬。

研发可以按以下真相源顺序执行：

1. `cadence/designs/2026-04-23_技术方案_Aria一期MVP精简设计_v1.2.md`
2. `cadence/designs/2026-04-23_技术方案_Aria一期实现总契约_v1.0.md`
3. `cadence/designs/2026-04-23_技术方案_Aria_IO协作协议与Provider契约_v1.0.md`
4. `cadence/designs/2026-04-24_技术方案_Aria一期评审后实施规格补齐_v1.2.md`
5. `cadence/plans/2026-04-23_计划文档_实施计划_Aria一期实现计划总览_v1.0.md`
6. P1 / P2 v1.1 / P3 v1.1 / P4 子计划

其中 P2、P3 应执行 `v1.1` 版本；`v1.0` 文件已标记 superseded，仅作为历史参考。

---

## 2. 研发是否能看懂如何实现

整体上可以。原因是文档已经形成了较完整的分层：

| 层级 | 是否清楚 | 评审意见 |
|------|----------|----------|
| MVP 边界 | 清楚 | 单机、单仓库、REPL + daemon、BYO CLI、worktree 隔离、OpenSpec 强约束都已明确。 |
| 实现总契约 | 清楚 | 对象模型、wire protocol、provider context、OpenSpec bundle、traceability、阶段收口规则具备实现粒度。 |
| 补齐规格 | 清楚 | 已补 Rust 类型、Projection/OpenSpec 编译、daemon 发现、artifact 路径、prompt、worktree/N23、fixture。 |
| P1-P4 计划 | 基本清楚 | 文件级任务、测试命令、阶段出口明确；但少量字段和测试缺口需要修。 |
| 研发导读 | 清楚 | 能帮助不同负责人快速定位自己要读的章节和交付物。 |

当前文档已经能让研发回答：

- P1 要先实现 REPL wire、daemon session、`new_task -> intake_brief -> EpicTask -> effectivePolicy/changeId`。
- P2 要实现 Document Operation、canonical validator、Projection、`_aria`、OpenSpec bundle、traceability。
- P3 要实现 Provider contract、fake provider、CLI adapter baseline、prompt registry、`N04-N12`。
- P4 要实现 worktree、execution、integration、final review、`N26` gated followup。

---

## 3. 必须修正的问题

### P0-1：P3 计划要求 `ProviderRunRecord` 增加 `riskRegistryRef`，与总契约和补齐规格不一致

**位置**
- `cadence/plans/2026-04-24_计划文档_实施计划_Aria一期实现计划_P3Provider驱动与规划节点_v1.1.md` 第 404-409 行要求 `ProviderRunRecord` 审计字段保留 `riskRegistryRef`。
- `cadence/designs/2026-04-23_技术方案_Aria一期实现总契约_v1.0.md` 第 577-605 行列出的 `ProviderRunRecord` 最小字段没有该字段。
- `cadence/designs/2026-04-24_技术方案_Aria一期评审后实施规格补齐_v1.2.md` 第 775-803 行 Rust 类型也没有该字段。

**影响**

这是明确的字段契约冲突。研发如果按 P3 计划实现，会改变 `ProviderRunRecord` schema；如果按总契约实现，又会无法满足 P3 Task 6。

**建议裁定**

默认不要修改 MVP 和总契约。应修改 P3 计划：

- 删除 “`ProviderRunRecord` 的审计字段中保留 `riskRegistryRef`”。
- Risk Registry 继续通过 `CanonicalNodeInput.riskRegistryRef`、`RuntimeSnapshot.riskRegistry`、`ArtifactTraceabilityBinding.relatedRiskIds` 建立关联。
- 若确实要求 ProviderRunRecord 直接持有 `riskRegistryRef`，必须先升版总契约和补齐规格，再让研发实现。

### P1-1：P1 wire schema 测试未明确覆盖总契约要求的完整 event registry

**位置**
- 总契约第 772-786 行定义了一期必须实现的事件类型和最小 payload。
- P1 计划第 148-185 行只要求覆盖 envelope 基础字段，没有明确逐项锁定事件类型与 payload schema。

**影响**

REPL / daemon / provider 后续可能各自发明事件 payload，P3/P4 再接 provider、projection、traceability 事件时会出现兼容问题。

**建议修正**

在 P1 Task 2 增加 `event_type` registry 测试：

- 覆盖 `task.created`、`task.phase_changed`、`artifact.materialized`、`projection.compiled`、`constraint_bundle.compiled`、`traceability.updated`、`gate.opened`、`gate.resolved`、`provider_run.started`、`provider_run.completed`、`provider_run.failed`。
- P1 可先做 schema 级测试，不要求 provider runtime 已实现。

### P1-2：P4 worktree 并发锁验收不够硬

**位置**
- 补齐规格第 12.3 章要求 `allowed_write_scope` 不重叠才能并行，重叠时必须串行。
- P4 计划 Task 1 只要求 worktree lease 和授权范围写入，没有单独测试重叠写范围的锁行为。

**影响**

MVP 明确支持多任务并行且依赖 worktree 隔离。若缺少重叠写范围检测，容易出现两个 WorkTask 同时修改同一模块，最终把冲突推迟到 N23。

**建议修正**

在 P4 增加 `worktree_locking.rs` 或并入 `execution_chain_fake_provider.rs`：

- 非重叠 `allowed_write_scope` 可并行。
- 重叠 `allowed_write_scope` 必须串行或阻塞。
- lease 状态变化必须写 runtime snapshot。

### P1-3：资料路径存在输入歧义

**位置**

用户给出的路径 `cadence/designs/2026-04-24_技术方案_Aria一期评审后实施规格补齐_v1.2.md2` 不存在；仓库实际文件是：

`cadence/designs/2026-04-24_技术方案_Aria一期评审后实施规格补齐_v1.2.md`

**影响**

对人工研发影响不大，但对自动化 agent / 脚本执行会直接找不到文件。

**建议修正**

在总览 plan 或研发导读中只保留实际 `.md` 路径；任务入口清单不要出现 `.md2`。

---

## 4. 建议改进项

### P2 validator 命名建议更精确

P2 完成判定中写到 “canonical validator 同时覆盖 canonical schema 最小字段和 Projection schema”。从总契约语义看，Projection 属于 phase1 profile，不应混入 canonical validator 概念。

建议改成：

- `canonical_validator`：只校验 canonical 最小字段。
- `projection_validator`：校验 `SpecProjection/DesignProjection/PlanProjection` schema 和 golden JSON。
- `phase1_profile_validator`：校验 `_aria`、traceability、projection refs、constraint refs。

这样能防止研发把 implementation profile 字段误写进 canonical schema。

### P4 Git 集成计划建议补充 dry-run / preflight

P4 已明确 candidate commit、integration branch、cherry-pick、rollback。建议在 `N22 integration_prepare` 增加 preflight 测试：

- integration branch 不存在时创建。
- candidate commit 不存在时报明确错误。
- `preMergeSha` 记录失败时不得进入 N23。
- cherry-pick conflict 必须 abort 并路由 N19。

这不是阻塞项，但能明显降低集成阶段返工成本。

### P1/P2/P3/P4 的 “提交阶段性变更” 可保留，但执行时需按团队流程处理

计划中包含 `git add` / `git commit` 步骤。对自动化 agent 是清晰的；对多人研发需要补充一句：是否提交、提交粒度、PR 拆分以团队分支策略为准，不影响技术契约。

---

## 5. 研发落地顺序

建议研发按下面的执行口径落地，不要按节点平铺开发：

| 阶段 | 先做什么 | 必须先锁定的测试 | 不允许发生的事 |
|------|----------|------------------|----------------|
| P1 | Rust 工程、REPL envelope、daemon handshake、N00-N03 | `repl_wire`、`repl_daemon_handshake`、`task_init_and_intake`、`runtime_snapshot_schema` | 没有正式 wire protocol 就写 REPL 临时 UI。 |
| P2 | Document Operation、Projection、OpenSpec bundle、Traceability | `document_ops`、`spec/design/plan_projection`、`openspec_bundle_schema`、`traceability_binding` | 直接解析 Markdown 原文做 routing / coverage。 |
| P3 | Provider DTO、fake provider、CLI adapter baseline、prompt registry、N04-N12 | `context_builder`、`cli_adapter_baseline`、`provider_error_routes`、`planning_chain_fake_provider` | fake provider 绕过 sentinel、schema、canonical validator。 |
| P4 | Worktree、execution reports、integration、final closure、N26 gate | `execution_chain_fake_provider`、`integration_retry_limit`、`final_followup_routes` | 直接把 worktree diff 合主分支，或 followup 自动进入 N26。 |

---

## 6. 研发开工前检查清单

- [ ] 确认执行 P2 v1.1、P3 v1.1，P2/P3 v1.0 只作历史参考。
- [ ] 修正 P3 Risk Registry 字段冲突：不要在未升版契约前给 `ProviderRunRecord` 增加 `riskRegistryRef`。
- [ ] P1 `repl_wire` 测试补齐完整 event registry 与 payload schema。
- [ ] P4 增加 worktree `allowed_write_scope` 重叠锁测试。
- [ ] 确认补齐规格路径统一为 `.md`，不要使用 `.md2`。
- [ ] 每个阶段启动前先对照 `2026-04-24_技术方案_Aria一期评审后实施规格补齐_v1.2.md` 的阶段准入门槛。

---

## 7. 最终判断

现有方案已经具备研发可落地性：MVP 边界、对象模型、数据流、Provider 调用、OpenSpec 约束、阶段收口和测试路径都足够清楚。

但不能让研发直接无条件按所有 plan 原文执行。必须先处理本文 P0/P1 问题，尤其是 P3 `ProviderRunRecord.riskRegistryRef` 字段冲突。处理后，研发可以按 P1 -> P2 -> P3 -> P4 顺序推进，并以每个阶段的测试命令作为完成判定。
