# 设计评审：Aria 一期方案与实施计划修复后二次 Review

**文档信息**
- **创建日期**：2026-04-26
- **版本**：v1.0
- **评审对象**：上一轮 review 后修复的 P1 / P2 / P3 / P4 实施计划，以及相关设计真相源
- **目标读者**：技术负责人、daemon / REPL / provider / document operation / OpenSpec / execution 研发负责人
- **评审目标**：确认上一轮问题是否修复，并指出剩余会影响研发落地的冲突

---

## 1. 总体结论

本次复查结论：**上一轮主要问题已基本修复，但仍有 1 个 P1 级文档一致性问题需要处理。**

已确认修复项：

| 上轮问题 | 当前状态 | 说明 |
|----------|----------|------|
| P3 `ProviderRunRecord.riskRegistryRef` 字段冲突 | 已修复 | P3 v1.1 已明确 `ProviderRunRecord` 不直接持有 `riskRegistryRef`，Risk Registry 通过 `CanonicalNodeInput`、`RuntimeSnapshot`、`ArtifactTraceabilityBinding` 关联。 |
| P1 event registry 测试缺口 | 已修复 | P1 已补充完整 event type registry 与最小 payload schema 测试。 |
| P4 worktree 并发锁测试缺口 | 已修复 | P4 已新增 `tests/worktree_locking.rs`，覆盖非重叠可并行、重叠串行或阻塞、lease 状态写 snapshot。 |
| P4 N22 preflight 建议 | 已补充 | P4 已补充 integration branch、candidate commit、`preMergeSha`、cherry-pick conflict 的 preflight 要求。 |
| P2 validator 命名建议 | P2 计划已修复 | P2 v1.1 已拆成 `canonical_validator`、`projection_validator`、`phase1_profile_validator`。 |

剩余问题集中在：**补齐规格文档第 16 章的 P2 执行清单仍保留旧口径，与当前 P2 计划和 MVP validator 分层不一致。**

---

## 2. 剩余问题

### P1-1：补齐规格 P2 执行清单仍写 “canonical validator 同时覆盖 Projection schema”

**位置**

- `cadence/designs/2026-04-24_技术方案_Aria一期评审后实施规格补齐_v1.2.md` 第 1955 行：
  - “canonical validator 同时覆盖 canonical schema 和 Projection schema”
- `cadence/designs/2026-04-23_技术方案_Aria一期MVP精简设计_v1.2.md` 第 320-327 行：
  - `canonical validator` 只校验上游最小字段与最小结构。
  - `phase1 profile validator` 校验 projection / `_aria` / traceability completeness。
- `cadence/designs/2026-04-23_技术方案_Aria一期实现总契约_v1.0.md` 第 1022-1026 行：
  - 阶段收口是 canonical artifact 校验、phase1 profile 校验、OpenSpec 约束覆盖校验。
- `cadence/plans/2026-04-24_计划文档_实施计划_Aria一期实现计划_P2产物投影与OpenSpec约束_v1.1.md` 第 38 行、第 458 行：
  - 已明确拆成 `canonical_validator`、`projection_validator`、`phase1_profile_validator`。

**影响**

这是当前唯一实质性剩余冲突。研发如果按补齐规格第 16 章执行，可能会把 Projection schema 校验塞进 `canonical_validator`，导致 implementation profile 污染 canonical schema；如果按 P2 v1.1 执行，又会和补齐规格清单不一致。

该问题不会推翻 MVP 或总契约，但会影响 P2 代码组织和测试命名，建议在进入 P2 前修正。

**建议修正**

将补齐规格第 1955 行改成：

```markdown
- [ ] P2：第 4.5-4.6、5、6、10、15 章已实现并有 golden fixture；`canonical_validator` 只覆盖 canonical schema 最小字段；`projection_validator` 校验 `SpecProjection/DesignProjection/PlanProjection` schema 和 golden JSON；`phase1_profile_validator` 校验 `_aria`、traceability、projection refs、constraint refs；`openspec_bundle_schema` 测试锁定顶层字段名。
```

如果不想新增 `projection_validator` 作为独立模块，也可以改成：

```markdown
- [ ] P2：第 4.5-4.6、5、6、10、15 章已实现并有 golden fixture；`canonical_validator` 只覆盖 canonical schema 最小字段；Projection schema 归入 `phase1_profile_validator` 校验；`openspec_bundle_schema` 测试锁定顶层字段名。
```

二选一即可。考虑当前 P2 v1.1 已明确三层 validator，推荐采用第一种写法。

---

## 3. 非阻塞风险

### R1：superseded 的 P2/P3 v1.0 文件仍保留旧内容

`cadence/plans/2026-04-23_计划文档_实施计划_Aria一期实现计划_P2产物投影与OpenSpec约束_v1.0.md` 与 `cadence/plans/2026-04-23_计划文档_实施计划_Aria一期实现计划_P3Provider驱动与规划节点_v1.0.md` 已标记废弃，并指向 v1.1。

这不构成阻塞，因为总览 plan 和研发导读都指向 v1.1。但如果后续自动化 agent 只做全文检索，仍可能扫到旧的 `ProviderRunRecord.riskRegistryRef` 表述。

建议：

- 人工研发：忽略 v1.0，执行 v1.1。
- 自动化脚本：排除文件头标记为 superseded 的 plan。
- 若希望进一步降低误读，可以将 v1.0 移入历史目录或在文件名中增加 `_superseded`，但这不是本轮必须项。

---

## 4. 当前可执行判断

除 P1-1 外，修复后的计划已经与 MVP 设计、实现总契约、IO/Provider 契约和补齐规格主干保持一致。

研发开工建议：

1. 先修补齐规格第 16 章 P2 清单的 validator 表述。
2. 修完后，可以按 P1 -> P2 -> P3 -> P4 推进。
3. 执行时以 P2 v1.1、P3 v1.1 为准，P2/P3 v1.0 仅保留历史参考。

---

## 5. 最终判断

**还有问题，但范围很小。**

当前唯一需要继续修的是补齐规格第 16 章 P2 执行清单的 validator 口径。修正后，本轮不再发现新的 P0/P1 阻塞项。
