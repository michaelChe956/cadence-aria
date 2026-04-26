# 设计评审：Aria 一期方案与实施计划研发可落地性 Review

**文档信息**
- **创建日期**：2026-04-26
- **版本**：v2.1
- **评审类型**：v2.0 review 修复后复核
- **目标读者**：Aria 一期研发负责人、P1-P4 子计划 owner、代码评审负责人
- **评审基准**：以 `Aria一期MVP精简设计_v1.2`、`Aria一期实现总契约_v1.0` 和修订后的 `Aria一期评审后实施规格补齐_v1.3` 为准

---

## 1. 复核结论

本轮没有发现需要暂停并由用户裁定的 MVP 设计或总设计错误。

v2.0 review 提出的 P0/P1/P2 问题已经全部落到新的方案与计划文档中。研发应从 2026-04-26 修订版开始执行，不再以旧 P1/P2/P3/P4 计划作为直接实施入口。

正式研发入口：

1. `cadence/designs/2026-04-26_技术方案_Aria一期研发导读与实施拆解_v1.1.md`
2. `cadence/designs/2026-04-23_技术方案_Aria一期MVP精简设计_v1.2.md`
3. `cadence/designs/2026-04-23_技术方案_Aria一期实现总契约_v1.0.md`
4. `cadence/designs/2026-04-26_技术方案_Aria_IO协作协议与Provider契约_v1.1.md`
5. `cadence/designs/2026-04-26_技术方案_Aria一期评审后实施规格补齐_v1.3.md`
6. `cadence/plans/2026-04-26_计划文档_实施计划_Aria一期实现计划总览_v1.1.md`

P1-P4 执行时使用 2026-04-26 子计划版本。

---

## 2. 修订文档清单

| 类型 | 新文档 | 作用 |
|------|--------|------|
| 技术方案 | `cadence/designs/2026-04-26_技术方案_Aria一期评审后实施规格补齐_v1.3.md` | 补齐 `AdapterInput/AdapterOutput`、`RuntimeUnit`、provider DTO 与 handler 契约 |
| 技术方案 | `cadence/designs/2026-04-26_技术方案_Aria_IO协作协议与Provider契约_v1.1.md` | 声明 provider 相关实现类型以 v1.3 为准 |
| 导读 | `cadence/designs/2026-04-26_技术方案_Aria一期研发导读与实施拆解_v1.1.md` | 更新研发阅读顺序、计划链接与 provider DTO 裁定 |
| 总计划 | `cadence/plans/2026-04-26_计划文档_实施计划_Aria一期实现计划总览_v1.1.md` | 更新正式技术方案包、P2 横切覆盖、关键数据流图 |
| P1 计划 | `cadence/plans/2026-04-26_计划文档_实施计划_Aria一期实现计划_P1基础骨架与REPL通信_v1.1.md` | 补齐 `nodes.rs` / `artifacts.rs` 内容来源和入口函数签名 |
| P2 计划 | `cadence/plans/2026-04-26_计划文档_实施计划_Aria一期实现计划_P2产物投影与OpenSpec约束_v1.2.md` | 明确 document ops 双文件职责，拆分 validator 测试 |
| P3 计划 | `cadence/plans/2026-04-26_计划文档_实施计划_Aria一期实现计划_P3Provider驱动与规划节点_v1.2.md` | 补齐 Adapter DTO 字段、统一执行链、N06 advisory 逻辑 |
| P4 计划 | `cadence/plans/2026-04-26_计划文档_实施计划_Aria一期实现计划_P4执行集成与最终收口_v1.1.md` | 明确 traceability 由 daemon 生成，补充 recovery 测试 |

---

## 3. P0 问题复核

| 编号 | v2.0 问题 | 修复结果 | 研发执行验收 |
|------|-----------|----------|--------------|
| P0-1 | `AdapterInput` / `AdapterOutput` Rust 类型缺失 | 已在规格补齐 v1.3 §4.7.3 定义 Rust DTO；P3 v1.2 Task 1 Step 1 给出字段签名 | fake provider 与 CLI adapter 必须共用同一 DTO，`ProviderRunRecord` 引用 adapter input/output |
| P0-2 | `document_ops.rs` 双文件职责不明 | 已在 P2 v1.2 §2 和 Task 1 裁定：`protocol/document_ops.rs` 放纯类型，`cross_cutting/document_ops.rs` 放实现函数 | 研发不得新增第三个 document operation 入口，不得在 protocol 层放 IO / parser 实现 |
| P0-3 | 节点 handler 统一接口缺失 | 已在规格补齐 v1.3 §4.7.4 定义 `RuntimeUnit`、`RuntimeUnitResult`、`RuntimeUnitError`；P1 v1.1 要求 runtime units 统一实现 | daemon 状态机只能通过 `RuntimeUnit::execute(input, ctx)` 调用节点 |

P0 结论：阻塞研发落地的问题已经解除，可以按修订版计划启动 P1。

---

## 4. P1 问题复核

| 编号 | v2.0 问题 | 修复结果 |
|------|-----------|----------|
| P1-1 | `ProviderContextPackage` 三份文档字段逐步增加 | IO 协议 v1.1、研发导读 v1.1、规格补齐 v1.3 均明确实现类型以 v1.3 §4.7 为准 |
| P1-2 | P2 Task 2/3 共用 `tests/spec_projection.rs` | P2 v1.2 将 Task 2 测试改为 `tests/artifact_validate.rs`，Task 3 保持 `tests/spec_projection.rs` |
| P1-3 | P3 规划节点对统一执行链引用不显式 | P3 v1.2 Task 4/5 明确引用实现总契约 §8.1，并要求覆盖 context builder、run record、归一化、validator、checkpoint |
| P1-4 | P4 `traceability_refs` 生成职责不精确 | P4 v1.1 明确 report handler 不手工填充，daemon 在归一化后按规格补齐 §10 自动生成 |
| P1-5 | P1 `nodes.rs` / `artifacts.rs` 内容来源不明 | P1 v1.1 Task 1 Step 3 明确 `nodes.rs` 放节点 ID/路由，`artifacts.rs` 放产物身份与引用类型 |
| P1-6 | P2 横切能力覆盖未在总览中列出 | 总览 v1.1 §5 明确 P2 完整实现 `X05 artifact_validate`，并列出 `tests/artifact_validate.rs` |

P1 结论：影响研发效率和多人协作的歧义已经消除。

---

## 5. P2 优化复核

| 编号 | v2.0 建议 | 修复结果 |
|------|-----------|----------|
| P2-1 | P1 Task 4 缺少入口函数签名 | P1 v1.1 Task 4 增加 `bootstrap`、`capture_intake`、`init_task`、`TaskInitResult` 签名 |
| P2-2 | P3 N06 advisory 调用逻辑不明确 | P3 v1.2 Task 4 Step 3 明确先校验 spec，可选 Codex advisory，最终 `spec_gate_decision` 仍由 daemon 生成 |
| P2-3 | P4 缺少 provider run + worktree recovery 测试 | P4 v1.1 Task 5 新增 `tests/recovery_provider_worktree.rs`，覆盖 gate、worktree lease、pending provider run、event replay |
| P2-4 | Task 间数据流不显式 | 总览 v1.1 §5.1 增加 P1->P2->P3->P4 关键数据流图和阶段消费关系表 |

P2 结论：优化项已转化为研发可执行的步骤或验收断言。

---

## 6. 研发执行裁定

1. 代码级字段、DTO、handler 接口以 `评审后实施规格补齐_v1.3` 为准。
2. 阶段执行以 2026-04-26 的总览计划 v1.1 与 P1-P4 子计划为准。
3. MVP 范围仍以 `Aria一期MVP精简设计_v1.2` 为准；任何新增行为不得扩大一期范围。
4. 总契约仍是对象模型、wire protocol、统一执行链、阶段收口规则的上位真相源。
5. 若研发实现中发现 v1.3 与 MVP v1.2 或总契约 v1.0 发生语义冲突，应先暂停并提交设计裁定，不得自行选择实现口径。

---

## 7. 仍需研发阶段验证的事项

这些不是文档阻塞项，但必须在实现时用测试证明：

| 阶段 | 必须验证 |
|------|----------|
| P1 | `RuntimeUnit` 统一调用、`nodes.rs` / `artifacts.rs` 类型归属、runtime snapshot 全字段序列化 |
| P2 | `document_ops` 职责边界、`tests/artifact_validate.rs`、projection golden JSON、OpenSpec bundle camelCase 字段 |
| P3 | fake provider 与 CLI adapter 共用 DTO、统一执行链全路径、N06 advisory 不直接推进 gate |
| P4 | daemon 自动生成 `_aria.traceability_refs`、provider run + worktree recovery、最终 coverage summary |

---

## 8. 一句话总结

v2.0 review 发现的问题已经全部落入 2026-04-26 修订版方案与计划。当前文档包已经具备研发落地条件；后续风险从“文档歧义”转为“实现是否严格按规格执行并用测试锁住”。
